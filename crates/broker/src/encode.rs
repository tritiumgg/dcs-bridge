//! The record encoder: put calls writing protobuf into a preallocated buffer.
//!
//! The broker links no protobuf runtime. A record crosses from Lua as typed
//! put calls, each naming its field number, and this writes the tag and the
//! value by hand into a buffer allocated once, at construction. A put call
//! allocates nothing.
//!
//! What `commit` hands back is the tail of an `Envelope`: every field but
//! `seq`, which differs per connection and is written where the frame is.
//! Today that tail is one field, the `Any` payload, whose type URL is the
//! topic `begin` was given and whose value is the record's own fields. The
//! wrapper is written at `begin`, ahead of the record, so a put appends to a
//! body that is already in its place and `commit` copies nothing.
//!
//! Every integer is written as a plain varint, which is the wire form of
//! `int32`, `int64`, `uint32`, `uint64`, `bool` and every enum. There is no
//! zigzag put, so a schema must not declare `sint32` or `sint64` fields: a
//! negative value on one would decode as a large positive number.
//!
//! A nested message needs its length in front of its body, and the body is
//! not known until `end_message`. Rather than write the body elsewhere and
//! copy it in behind its length, `message` leaves a fixed-width gap for the
//! length and `end_message` fills it in. The gap is as wide as the largest
//! length the buffer can hold, so the varint written there is padded: it
//! carries continuation bytes a minimal encoding would not. Protobuf permits
//! that, and the tests show a stock decoder reading it. ADR 0012.

use std::fmt;

/// Field numbers above this do not fit the 29 bits a tag leaves them.
const MAX_FIELD: u32 = (1 << 29) - 1;

/// The most nested messages a record may hold open at once.
///
/// The stack of open messages is sized once, at construction, so this is a
/// bound and not a tunable. A generated emitter nests as deep as its schema
/// and no schema in this project comes near it.
pub const MAX_DEPTH: usize = 64;

/// What protobuf writes in front of a fully-qualified type name to make an
/// `Any` type URL. The topic is the name, and this is what every runtime
/// expects to find before it.
pub const TYPE_URL_PREFIX: &[u8] = b"type.googleapis.com/";

/// `Envelope.payload`, the `Any` the record goes in.
const ENVELOPE_PAYLOAD: u32 = 4;
/// `Any.type_url`.
const ANY_TYPE_URL: u32 = 1;
/// `Any.value`.
const ANY_VALUE: u32 = 2;

/// The wire type of a varint value.
const WIRE_VARINT: u64 = 0;
/// The wire type of a little-endian eight-byte value.
const WIRE_FIXED64: u64 = 1;
/// The wire type of a length-prefixed value.
const WIRE_LENGTH: u64 = 2;

/// Why a put or a commit was refused.
///
/// Every error but [`Error::NotOpen`] poisons the record: later puts return
/// the same error, `commit` returns it and discards the record, and the next
/// `begin` starts clean. Poisoning rather than skipping the one bad put keeps
/// a record from committing with a field silently missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A put or a commit with no record open.
    NotOpen,
    /// The record outgrew the buffer.
    Full,
    /// A field number outside 1 to 2^29 - 1.
    FieldNumber,
    /// More than [`MAX_DEPTH`] messages open at once.
    Depth,
    /// An `end_message` with no message open, or a `commit` with one open.
    Unbalanced,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::NotOpen => "no record is open",
            Error::Full => "the record outgrew its buffer",
            Error::FieldNumber => "field number outside 1 to 2^29 - 1",
            Error::Depth => "too many nested messages open",
            Error::Unbalanced => "message and end_message do not pair",
        })
    }
}

impl std::error::Error for Error {}

/// Where a record stands between `begin` and `commit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing open. The buffer may still hold the last committed body.
    Idle,
    /// A record is open and every put so far succeeded.
    Open,
    /// A record is open and a put failed. `commit` will refuse it.
    Poisoned(Error),
}

/// One record at a time, encoded into a buffer sized once.
///
/// `begin` opens a record on a topic, the puts append fields, and `commit`
/// closes it and returns the envelope tail holding it. A record that `begin`
/// finds still open, or that `commit`
/// refuses, is discarded and counted, because the caller that abandoned it is
/// the caller that needs to know.
#[derive(Debug)]
pub struct Encoder {
    buf: Vec<u8>,
    capacity: usize,
    /// For each open message, innermost last, where its length gap starts.
    open: Vec<usize>,
    /// Bytes every length gap takes: enough for the largest body possible.
    len_width: usize,
    /// Where the payload's length gap starts, and where the `Any` value's
    /// does. Both are written at `begin` and filled at `commit`. They are
    /// not on the `open` stack, so no `end_message` can close them.
    payload_gap: usize,
    value_gap: usize,
    state: State,
    discarded: u64,
}

impl Encoder {
    /// An encoder holding at most `bytes` per record. The two allocations:
    /// the buffer, and the stack of open messages.
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            buf: Vec::with_capacity(bytes),
            capacity: bytes,
            open: Vec::with_capacity(MAX_DEPTH),
            len_width: varint_len(bytes as u64),
            payload_gap: 0,
            value_gap: 0,
            state: State::Idle,
            discarded: 0,
        }
    }

    /// The most bytes one record may hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Records that started and never committed: abandoned at the next
    /// `begin`, or refused at `commit`.
    pub fn discarded(&self) -> u64 {
        self.discarded
    }

    /// Open a record on `topic`, discarding one left open. Returns whether it
    /// did.
    ///
    /// The topic is the record's fully-qualified type name, and it goes into
    /// the wrapper here: the payload field, the `Any` type URL built from the
    /// topic, and the `Any` value field the puts then fill. A topic the buffer
    /// cannot hold poisons the record with [`Error::Full`], so every later
    /// put reports it and `commit` refuses it.
    pub fn begin(&mut self, topic: &[u8]) -> bool {
        let abandoned = self.state != State::Idle;
        if abandoned {
            self.discarded += 1;
        }
        self.buf.clear();
        self.open.clear();
        self.state = State::Open;

        let width = self.len_width;
        let url_len = TYPE_URL_PREFIX.len() + topic.len();
        let wrapper = self
            .put(ENVELOPE_PAYLOAD, WIRE_LENGTH, width, |buf| {
                buf.resize(buf.len() + width, 0);
            })
            .and_then(|()| {
                self.payload_gap = self.buf.len() - width;
                self.put(
                    ANY_TYPE_URL,
                    WIRE_LENGTH,
                    varint_len(url_len as u64) + url_len,
                    |buf| {
                        put_varint(buf, url_len as u64);
                        buf.extend_from_slice(TYPE_URL_PREFIX);
                        buf.extend_from_slice(topic);
                    },
                )
            })
            .and_then(|()| {
                self.put(ANY_VALUE, WIRE_LENGTH, width, |buf| {
                    buf.resize(buf.len() + width, 0);
                })
            });
        if wrapper.is_ok() {
            self.value_gap = self.buf.len() - width;
        }
        abandoned
    }

    /// Put a signed 64-bit integer as a varint.
    ///
    /// A negative value takes all ten bytes, which is what every decoder
    /// expects of a negative `int32` or `int64`.
    pub fn integer(&mut self, field: u32, n: i64) -> Result<(), Error> {
        // Sign is carried by the two's-complement bit pattern, not the type.
        let v = n as u64;
        self.put(field, WIRE_VARINT, varint_len(v), |buf| put_varint(buf, v))
    }

    /// Put a double as eight little-endian bytes.
    pub fn double(&mut self, field: u32, x: f64) -> Result<(), Error> {
        self.put(field, WIRE_FIXED64, 8, |buf| {
            buf.extend_from_slice(&x.to_le_bytes());
        })
    }

    /// Put bytes behind their length. Lua strings carry any bytes, so this
    /// takes bytes and leaves UTF-8 to the schema.
    pub fn string(&mut self, field: u32, s: &[u8]) -> Result<(), Error> {
        let len = s.len() as u64;
        self.put(field, WIRE_LENGTH, varint_len(len) + s.len(), |buf| {
            put_varint(buf, len);
            buf.extend_from_slice(s);
        })
    }

    /// Put a boolean as a one-byte varint.
    pub fn boolean(&mut self, field: u32, b: bool) -> Result<(), Error> {
        self.put(field, WIRE_VARINT, 1, |buf| buf.push(u8::from(b)))
    }

    /// Open a nested message on `field`. Every put until the matching
    /// `end_message` lands inside it, and a message may open another.
    ///
    /// A repeated message field is one pair per element, all on one field
    /// number, and put order is element order.
    pub fn message(&mut self, field: u32) -> Result<(), Error> {
        self.ready()?;
        if self.open.len() == MAX_DEPTH {
            return self.poison(Error::Depth);
        }
        let width = self.len_width;
        self.put(field, WIRE_LENGTH, width, |buf| {
            // The gap the length is written into at end_message. Within
            // capacity, so this grows the length and not the allocation.
            buf.resize(buf.len() + width, 0);
        })?;
        self.open.push(self.buf.len() - width);
        Ok(())
    }

    /// Close the innermost open message, writing its length into the gap
    /// `message` left, padded to the gap's width.
    pub fn end_message(&mut self) -> Result<(), Error> {
        self.ready()?;
        let Some(gap) = self.open.pop() else {
            return self.poison(Error::Unbalanced);
        };
        let body = self.buf.len() - (gap + self.len_width);
        put_padded_varint(&mut self.buf[gap..gap + self.len_width], body as u64);
        Ok(())
    }

    /// Close the record and return the envelope tail holding it, or refuse
    /// and discard it.
    ///
    /// A record with a message still open is refused: closing it here would
    /// commit a shape the caller did not finish.
    ///
    /// The tail stays readable until the next `begin`.
    pub fn commit(&mut self) -> Result<&[u8], Error> {
        match self.state {
            State::Idle => Err(Error::NotOpen),
            State::Poisoned(error) => {
                self.discard();
                Err(error)
            }
            State::Open if !self.open.is_empty() => {
                self.discard();
                Err(Error::Unbalanced)
            }
            State::Open => {
                let width = self.len_width;
                let end = self.buf.len();
                for gap in [self.value_gap, self.payload_gap] {
                    let body = end - (gap + width);
                    put_padded_varint(&mut self.buf[gap..gap + width], body as u64);
                }
                self.state = State::Idle;
                Ok(&self.buf)
            }
        }
    }

    /// Write one field: the tag, then `size` bytes of value from `write`.
    ///
    /// The size check comes first, so a put either lands whole or not at all
    /// and the buffer never grows past its capacity.
    fn put(
        &mut self,
        field: u32,
        wire: u64,
        size: usize,
        write: impl FnOnce(&mut Vec<u8>),
    ) -> Result<(), Error> {
        self.ready()?;
        if field == 0 || field > MAX_FIELD {
            return self.poison(Error::FieldNumber);
        }
        let tag = (u64::from(field) << 3) | wire;
        if self.capacity - self.buf.len() < varint_len(tag) + size {
            return self.poison(Error::Full);
        }
        put_varint(&mut self.buf, tag);
        write(&mut self.buf);
        Ok(())
    }

    /// Whether a put may proceed: a record is open and nothing has failed.
    fn ready(&self) -> Result<(), Error> {
        match self.state {
            State::Open => Ok(()),
            State::Idle => Err(Error::NotOpen),
            State::Poisoned(error) => Err(error),
        }
    }

    fn poison(&mut self, error: Error) -> Result<(), Error> {
        self.state = State::Poisoned(error);
        Err(error)
    }

    fn discard(&mut self) {
        self.discarded += 1;
        self.buf.clear();
        self.state = State::Idle;
    }
}

/// How many bytes `v` takes as a varint: one per seven bits, at least one.
pub(crate) fn varint_len(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

/// Append `v` as a minimal varint: seven bits per byte, low bits first, the
/// high bit of each byte saying another follows.
fn put_varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        // Truncation keeps the low seven bits, which is the point.
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Write `v` as a minimal varint at the start of `out`, returning how many
/// bytes it took. For a caller building a header on the stack rather than in
/// a `Vec`, so the frame writer and the encoder agree on what a varint is.
///
/// # Panics
///
/// If `out` is shorter than [`varint_len`] of `v`; ten bytes always suffice.
pub(crate) fn write_varint(out: &mut [u8], mut v: u64) -> usize {
    let mut n = 0;
    while v >= 0x80 {
        // Truncation keeps the low seven bits, which is the point.
        out[n] = (v as u8) | 0x80;
        v >>= 7;
        n += 1;
    }
    out[n] = v as u8;
    n + 1
}

/// Write `v` as a varint filling all of `gap`: every byte but the last has
/// the continuation bit, whether or not the bits above it are zero.
///
/// The caller sizes the gap for the largest value it can hold, so a value
/// that does not fit is a defect here rather than a runtime condition.
fn put_padded_varint(gap: &mut [u8], mut v: u64) {
    let last = gap.len() - 1;
    for (i, byte) in gap.iter_mut().enumerate() {
        let more = if i < last { 0x80 } else { 0 };
        // Truncation keeps the low seven bits, which is the point.
        *byte = ((v as u8) & 0x7f) | more;
        v >>= 7;
    }
    debug_assert_eq!(v, 0, "the length outgrew its gap");
}

/// The done-when is that a stock library decodes the output, so these tests
/// decode through `prost` rather than through a decoder written beside the
/// encoder, which would share its misreadings.
#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    /// The topic every test opens on, unless it is counting bytes.
    const TOPIC: &[u8] = b"dcs.builtin.UnitDestroyed";

    /// A one-byte topic, for the tests that count bytes against capacity.
    const SHORT: &[u8] = b"t";

    /// What the wrapper takes for [`SHORT`] under a capacity below 128, where
    /// a length gap is one byte: the payload tag and gap, the type URL tag,
    /// length and 21 bytes, and the value tag and gap.
    const SHORT_WRAPPER: usize = 2 + 23 + 2;

    /// `dcs.bridge.Envelope` without `seq`, which the connection writes. The
    /// `Any` is the library's own, so the type URL is read the way every
    /// consumer reads it.
    #[derive(Clone, PartialEq, Message)]
    struct Tail {
        #[prost(message, optional, tag = "4")]
        payload: Option<prost_types::Any>,
    }

    /// One field of each wire form, plus the field numbers at the edges of a
    /// one-byte tag and of the whole numbering space.
    #[derive(Clone, PartialEq, Message)]
    struct Scalars {
        #[prost(int64, tag = "1")]
        n: i64,
        #[prost(double, tag = "2")]
        x: f64,
        #[prost(string, tag = "3")]
        s: String,
        #[prost(bool, tag = "4")]
        b: bool,
        #[prost(int64, repeated, tag = "5")]
        seq: Vec<i64>,
        #[prost(message, optional, tag = "6")]
        inner: Option<Inner>,
        #[prost(message, repeated, tag = "7")]
        items: Vec<Inner>,
        #[prost(int32, tag = "15")]
        near: i32,
        #[prost(int32, tag = "16")]
        far: i32,
        #[prost(uint64, tag = "536870911")]
        last: u64,
    }

    /// A nested message that can hold itself, for depth.
    #[derive(Clone, PartialEq, Message)]
    struct Inner {
        #[prost(string, tag = "1")]
        s: String,
        #[prost(message, optional, boxed, tag = "2")]
        inner: Option<Box<Inner>>,
    }

    /// The `Any` out of a committed tail, with its type URL checked against
    /// the topic the record was opened on.
    fn any(tail: &[u8], topic: &[u8]) -> prost_types::Any {
        let tail = Tail::decode(tail).expect("a stock decoder reads the tail");
        let any = tail.payload.expect("the tail carries a payload");
        let mut url = TYPE_URL_PREFIX.to_vec();
        url.extend_from_slice(topic);
        assert_eq!(any.type_url.as_bytes(), url, "the type URL names the topic");
        any
    }

    /// The record's own bytes, out of the wrapper.
    fn body(tail: &[u8]) -> Vec<u8> {
        any(tail, TOPIC).value
    }

    fn decode(tail: &[u8]) -> Scalars {
        Scalars::decode(&body(tail)[..]).expect("a stock decoder reads the body")
    }

    #[test]
    fn every_scalar_decodes() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        e.integer(1, -7).unwrap();
        e.double(2, 2.5).unwrap();
        e.string(3, "über".as_bytes()).unwrap();
        e.boolean(4, true).unwrap();
        e.integer(15, 15).unwrap();
        e.integer(16, 16).unwrap();
        e.integer(MAX_FIELD, 9).unwrap();

        let got = decode(e.commit().unwrap());
        assert_eq!(got.n, -7);
        assert_eq!(got.x, 2.5);
        assert_eq!(got.s, "über");
        assert!(got.b);
        assert_eq!((got.near, got.far, got.last), (15, 16, 9));
    }

    #[test]
    fn integer_edges_decode() {
        for (n, bytes) in [
            (i64::MIN, 10),
            (-1, 10),
            (0, 1),
            (127, 1),
            (128, 2),
            (i64::MAX, 9),
        ] {
            let mut e = Encoder::with_capacity(256);
            e.begin(TOPIC);
            e.integer(1, n).unwrap();
            let tail = e.commit().unwrap();
            assert_eq!(body(tail).len(), 1 + bytes, "{n}");
            assert_eq!(decode(tail).n, n, "{n}");
        }
    }

    #[test]
    fn double_edges_decode() {
        for x in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let mut e = Encoder::with_capacity(256);
            e.begin(TOPIC);
            e.double(2, x).unwrap();
            let got = decode(e.commit().unwrap()).x;
            assert_eq!(got.to_bits(), x.to_bits(), "{x}");
        }

        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        e.double(2, f64::NAN).unwrap();
        assert!(decode(e.commit().unwrap()).x.is_nan());
    }

    #[test]
    fn defaults_decode_whether_written_or_omitted() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        assert!(body(e.commit().unwrap()).is_empty());

        e.begin(TOPIC);
        e.string(3, b"").unwrap();
        e.boolean(4, false).unwrap();
        let tail = e.commit().unwrap();
        // proto3 would omit both, and the explicit form must read the same.
        assert_eq!(body(tail), [0x1a, 0x00, 0x20, 0x00]);
        assert_eq!(decode(tail), Scalars::default());
    }

    #[test]
    fn a_put_needs_an_open_record() {
        let mut e = Encoder::with_capacity(256);
        assert_eq!(e.integer(1, 1), Err(Error::NotOpen));
        assert_eq!(e.commit(), Err(Error::NotOpen));
        e.begin(TOPIC);
        e.commit().unwrap();
        assert_eq!(e.boolean(1, true), Err(Error::NotOpen));
        assert_eq!(e.discarded(), 0);
    }

    #[test]
    fn a_bad_field_number_poisons_the_record() {
        for field in [0, MAX_FIELD + 1] {
            let mut e = Encoder::with_capacity(256);
            e.begin(TOPIC);
            assert_eq!(e.integer(field, 1), Err(Error::FieldNumber));
            assert_eq!(e.integer(1, 1), Err(Error::FieldNumber));
            assert_eq!(e.commit(), Err(Error::FieldNumber));
            assert_eq!(e.discarded(), 1);
        }
    }

    #[test]
    fn a_record_fills_its_buffer_exactly_and_no_further() {
        // The wrapper counts against the capacity: three bytes are left.
        let mut e = Encoder::with_capacity(SHORT_WRAPPER + 3);
        e.begin(SHORT);
        e.string(1, b"x").unwrap();
        let tail = e.commit().unwrap();
        assert_eq!(tail.len(), SHORT_WRAPPER + 3);
        assert_eq!(any(tail, SHORT).value, [0x0a, 0x01, b'x']);

        e.begin(SHORT);
        e.string(1, b"xy").unwrap_err();
        assert_eq!(e.string(1, b"xy"), Err(Error::Full));
        assert_eq!(e.boolean(2, true), Err(Error::Full));
        assert_eq!(e.commit(), Err(Error::Full));
        assert_eq!(e.discarded(), 1);
        assert!(!e.begin(TOPIC), "the refused record was already discarded");
    }

    #[test]
    fn begin_discards_an_open_record_and_counts_it() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        e.integer(1, 1).unwrap();
        assert!(e.begin(TOPIC));
        assert_eq!(e.discarded(), 1);
        assert!(body(e.commit().unwrap()).is_empty());
        assert!(!e.begin(TOPIC));
    }

    #[test]
    fn puts_allocate_nothing() {
        let mut e = Encoder::with_capacity(256);
        let before = e.buf.capacity();
        for _ in 0..3 {
            e.begin(TOPIC);
            e.integer(1, i64::MIN).unwrap();
            e.string(3, b"0123456789").unwrap();
            e.double(2, 1.0).unwrap();
            e.commit().unwrap();
        }
        assert_eq!(e.buf.capacity(), before);
    }

    #[test]
    fn varint_len_matches_the_bytes_written() {
        for v in [0, 1, 0x7f, 0x80, 0x3fff, 0x4000, u64::MAX] {
            let mut buf = Vec::new();
            put_varint(&mut buf, v);
            assert_eq!(buf.len(), varint_len(v), "{v:#x}");
            assert_eq!(prost::encoding::decode_varint(&mut &buf[..]).unwrap(), v);

            let mut out = [0u8; 10];
            let n = write_varint(&mut out, v);
            assert_eq!(&out[..n], &buf[..], "{v:#x} differs between writers");
        }
    }

    #[test]
    fn distinct_fields_decode_in_any_order() {
        let mut forward = Encoder::with_capacity(256);
        forward.begin(TOPIC);
        forward.integer(1, 4).unwrap();
        forward.string(3, b"x").unwrap();
        let forward = decode(forward.commit().unwrap());

        let mut backward = Encoder::with_capacity(256);
        backward.begin(TOPIC);
        backward.string(3, b"x").unwrap();
        backward.integer(1, 4).unwrap();
        assert_eq!(decode(backward.commit().unwrap()), forward);
    }

    #[test]
    fn a_repeated_field_keeps_put_order() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        for n in [3, 1, 2] {
            e.integer(5, n).unwrap();
        }
        assert_eq!(decode(e.commit().unwrap()).seq, [3, 1, 2]);
    }

    /// The done-when's second half: the length in front of a nested message
    /// is padded to the gap's width, and a stock decoder reads it anyway.
    #[test]
    fn a_nested_length_is_padded_and_decodes() {
        // The product buffer, whose largest length takes three varint bytes.
        let mut e = Encoder::with_capacity(1 << 20);
        e.begin(TOPIC);
        e.message(6).unwrap();
        e.string(1, b"a").unwrap();
        e.end_message().unwrap();
        let tail = e.commit().unwrap();

        // Tag, then a three-byte varint for a body of three bytes.
        let body = body(tail);
        assert_eq!(body, [0x32, 0x83, 0x80, 0x00, 0x0a, 0x01, b'a']);
        assert_eq!(prost::encoding::decode_varint(&mut &body[1..4]).unwrap(), 3);
        assert_eq!(decode(tail).inner.unwrap().s, "a");
    }

    /// The wrapper `begin` writes: the payload field over an `Any` whose
    /// type URL is the topic behind protobuf's prefix, with the two lengths
    /// padded the way a nested message's is. A stock decoder reads it, and
    /// reads the topic out of it with no schema for the record.
    #[test]
    fn the_wrapper_names_the_topic_behind_two_padded_lengths() {
        let mut e = Encoder::with_capacity(1 << 20);
        e.begin(SHORT);
        e.integer(1, 1).unwrap();
        let tail = e.commit().unwrap();

        let mut want = vec![0x22, 0x9d, 0x80, 0x00];
        want.extend_from_slice(b"\x0a\x15type.googleapis.com/t");
        want.extend_from_slice(&[0x12, 0x82, 0x80, 0x00, 0x08, 0x01]);
        assert_eq!(tail, want);
        assert_eq!(
            prost::encoding::decode_varint(&mut &tail[1..4]).unwrap(),
            29
        );

        let any = any(tail, SHORT);
        assert_eq!(any.type_url, "type.googleapis.com/t");
        assert_eq!(any.value, [0x08, 0x01]);
    }

    #[test]
    fn a_topic_the_buffer_cannot_hold_poisons_the_record() {
        let mut e = Encoder::with_capacity(8);
        assert!(!e.begin(TOPIC));
        assert_eq!(e.integer(1, 1), Err(Error::Full));
        assert_eq!(e.commit(), Err(Error::Full));
        assert_eq!(e.discarded(), 1);
    }

    #[test]
    fn nested_bodies_of_every_length_decode() {
        // Zero, the one-byte edge, and past it.
        for len in [0, 127, 128, 200] {
            let s = vec![b'z'; len];
            let mut e = Encoder::with_capacity(4096);
            e.begin(TOPIC);
            e.message(6).unwrap();
            if len > 0 {
                e.string(1, &s).unwrap();
            }
            e.end_message().unwrap();
            let got = decode(e.commit().unwrap()).inner.unwrap();
            assert_eq!(got.s.len(), len);
        }
    }

    #[test]
    fn repeated_messages_keep_put_order() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        for s in ["c", "a", "b"] {
            e.message(7).unwrap();
            e.string(1, s.as_bytes()).unwrap();
            e.end_message().unwrap();
        }
        let items = decode(e.commit().unwrap()).items;
        let got: Vec<&str> = items.iter().map(|i| i.s.as_str()).collect();
        assert_eq!(got, ["c", "a", "b"]);
    }

    #[test]
    fn messages_nest_and_puts_land_in_the_innermost() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        e.message(6).unwrap();
        e.string(1, b"outer").unwrap();
        e.message(2).unwrap();
        e.string(1, b"middle").unwrap();
        e.message(2).unwrap();
        e.string(1, b"inner").unwrap();
        e.end_message().unwrap();
        e.end_message().unwrap();
        e.end_message().unwrap();
        e.integer(1, 1).unwrap();

        let got = decode(e.commit().unwrap());
        assert_eq!(got.n, 1);
        let outer = got.inner.unwrap();
        let middle = outer.inner.unwrap();
        let inner = middle.inner.unwrap();
        assert_eq!(outer.s, "outer");
        assert_eq!(middle.s, "middle");
        assert_eq!(inner.s, "inner");
        assert!(inner.inner.is_none());
    }

    #[test]
    fn depth_is_capped_at_construction() {
        let mut e = Encoder::with_capacity(4096);
        let stack = e.open.capacity();
        e.begin(TOPIC);
        for _ in 0..MAX_DEPTH {
            e.message(2).unwrap();
        }
        assert_eq!(e.message(2), Err(Error::Depth));
        assert_eq!(e.commit(), Err(Error::Depth));
        assert_eq!(e.open.capacity(), stack);
    }

    #[test]
    fn unbalanced_messages_are_refused() {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        assert_eq!(e.end_message(), Err(Error::Unbalanced));
        assert_eq!(e.commit(), Err(Error::Unbalanced));

        e.begin(TOPIC);
        e.message(6).unwrap();
        assert_eq!(e.commit(), Err(Error::Unbalanced));
        assert_eq!(e.discarded(), 2);
        assert!(!e.begin(TOPIC));
    }

    #[test]
    fn a_message_that_does_not_fit_is_refused_whole() {
        let mut e = Encoder::with_capacity(SHORT_WRAPPER + 4);
        e.begin(SHORT);
        e.string(1, b"xy").unwrap();
        // Nothing is left for a tag and its length gap.
        assert_eq!(e.message(6), Err(Error::Full));
        assert!(e.open.is_empty());
    }
}
