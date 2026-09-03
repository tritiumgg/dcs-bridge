//! The record encoder: put calls writing protobuf into a preallocated buffer.
//!
//! The broker links no protobuf runtime. A record crosses from Lua as typed
//! put calls, each naming its field number, and this writes the tag and the
//! value by hand into a buffer allocated once, at construction. A put call
//! allocates nothing.
//!
//! What `commit` hands back is a message body: the payload's fields, one
//! after another, with no length prefix. Wrapping that body in an `Any`
//! inside an `Envelope` and framing it for the socket happen later and
//! elsewhere.
//!
//! Every integer is written as a plain varint, which is the wire form of
//! `int32`, `int64`, `uint32`, `uint64`, `bool` and every enum. There is no
//! zigzag put, so a schema must not declare `sint32` or `sint64` fields: a
//! negative value on one would decode as a large positive number.

use std::fmt;

/// Field numbers above this do not fit the 29 bits a tag leaves them.
const MAX_FIELD: u32 = (1 << 29) - 1;

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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::NotOpen => "no record is open",
            Error::Full => "the record outgrew its buffer",
            Error::FieldNumber => "field number outside 1 to 2^29 - 1",
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
/// `begin` opens a record, the puts append fields, and `commit` closes it and
/// returns the body. A record that `begin` finds still open, or that `commit`
/// refuses, is discarded and counted, because the caller that abandoned it is
/// the caller that needs to know.
#[derive(Debug)]
pub struct Encoder {
    buf: Vec<u8>,
    capacity: usize,
    state: State,
    discarded: u64,
}

impl Encoder {
    /// An encoder holding at most `bytes` per record. The one allocation.
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            buf: Vec::with_capacity(bytes),
            capacity: bytes,
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

    /// Open a record, discarding one left open. Returns whether it did.
    pub fn begin(&mut self) -> bool {
        let abandoned = self.state != State::Idle;
        if abandoned {
            self.discarded += 1;
        }
        self.buf.clear();
        self.state = State::Open;
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

    /// Close the record and return its body, or refuse and discard it.
    ///
    /// The body stays readable until the next `begin`.
    pub fn commit(&mut self) -> Result<&[u8], Error> {
        match self.state {
            State::Idle => Err(Error::NotOpen),
            State::Poisoned(error) => {
                self.discard();
                Err(error)
            }
            State::Open => {
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
        match self.state {
            State::Open => {}
            State::Idle => return Err(Error::NotOpen),
            State::Poisoned(error) => return Err(error),
        }
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
fn varint_len(mut v: u64) -> usize {
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

/// The done-when is that a stock library decodes the output, so these tests
/// decode through `prost` rather than through a decoder written beside the
/// encoder, which would share its misreadings.
#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

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
        #[prost(int32, tag = "15")]
        near: i32,
        #[prost(int32, tag = "16")]
        far: i32,
        #[prost(uint64, tag = "536870911")]
        last: u64,
    }

    fn decode(body: &[u8]) -> Scalars {
        Scalars::decode(body).expect("a stock decoder reads the body")
    }

    #[test]
    fn every_scalar_decodes() {
        let mut e = Encoder::with_capacity(256);
        e.begin();
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
            let mut e = Encoder::with_capacity(16);
            e.begin();
            e.integer(1, n).unwrap();
            let body = e.commit().unwrap();
            assert_eq!(body.len(), 1 + bytes, "{n}");
            assert_eq!(decode(body).n, n, "{n}");
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
            let mut e = Encoder::with_capacity(16);
            e.begin();
            e.double(2, x).unwrap();
            let got = decode(e.commit().unwrap()).x;
            assert_eq!(got.to_bits(), x.to_bits(), "{x}");
        }

        let mut e = Encoder::with_capacity(16);
        e.begin();
        e.double(2, f64::NAN).unwrap();
        assert!(decode(e.commit().unwrap()).x.is_nan());
    }

    #[test]
    fn defaults_decode_whether_written_or_omitted() {
        let mut e = Encoder::with_capacity(16);
        e.begin();
        assert!(e.commit().unwrap().is_empty());

        e.begin();
        e.string(3, b"").unwrap();
        e.boolean(4, false).unwrap();
        let body = e.commit().unwrap();
        // proto3 would omit both, and the explicit form must read the same.
        assert_eq!(body, [0x1a, 0x00, 0x20, 0x00]);
        assert_eq!(decode(body), Scalars::default());
    }

    #[test]
    fn a_put_needs_an_open_record() {
        let mut e = Encoder::with_capacity(16);
        assert_eq!(e.integer(1, 1), Err(Error::NotOpen));
        assert_eq!(e.commit(), Err(Error::NotOpen));
        e.begin();
        e.commit().unwrap();
        assert_eq!(e.boolean(1, true), Err(Error::NotOpen));
        assert_eq!(e.discarded(), 0);
    }

    #[test]
    fn a_bad_field_number_poisons_the_record() {
        for field in [0, MAX_FIELD + 1] {
            let mut e = Encoder::with_capacity(16);
            e.begin();
            assert_eq!(e.integer(field, 1), Err(Error::FieldNumber));
            assert_eq!(e.integer(1, 1), Err(Error::FieldNumber));
            assert_eq!(e.commit(), Err(Error::FieldNumber));
            assert_eq!(e.discarded(), 1);
        }
    }

    #[test]
    fn a_record_fills_its_buffer_exactly_and_no_further() {
        let mut e = Encoder::with_capacity(3);
        e.begin();
        e.string(1, b"x").unwrap();
        assert_eq!(e.commit().unwrap(), [0x0a, 0x01, b'x']);

        e.begin();
        e.string(1, b"xy").unwrap_err();
        assert_eq!(e.string(1, b"xy"), Err(Error::Full));
        assert_eq!(e.boolean(2, true), Err(Error::Full));
        assert_eq!(e.commit(), Err(Error::Full));
        assert_eq!(e.discarded(), 1);
        assert!(!e.begin(), "the refused record was already discarded");
    }

    #[test]
    fn begin_discards_an_open_record_and_counts_it() {
        let mut e = Encoder::with_capacity(16);
        e.begin();
        e.integer(1, 1).unwrap();
        assert!(e.begin());
        assert_eq!(e.discarded(), 1);
        assert!(e.commit().unwrap().is_empty());
        assert!(!e.begin());
    }

    #[test]
    fn puts_allocate_nothing() {
        let mut e = Encoder::with_capacity(64);
        let before = e.buf.capacity();
        for _ in 0..3 {
            e.begin();
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
        }
    }
}
