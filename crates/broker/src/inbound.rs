//! The inbound path: what a connection sends, read on its own thread.
//!
//! Every byte from a socket is read here and nowhere else. The reader thread
//! owns the frame from its length prefix to the end of its payload, and the
//! logic thread sees only what the reader thread has already made into a
//! whole record. A fault in here is caught at the thread and drops the one
//! connection it was reading; the process, and the mission, carry on.
//!
//! The parser runs inside the DCS process, before authentication completes,
//! so it reads as little as it can and bounds every read before it makes it.
//! The length prefix is checked against the frame cap before a byte of the
//! frame is allocated for, and the payload's type URL, which is the one
//! string read out of every frame, is checked against the URL cap before
//! the decoder allocates for it. Both are configuration, read fresh for
//! every frame through [`Limits`], so a later `configure` binds from the
//! next frame on.
//! The envelope decodes through `prost`, the one crate the shipped build
//! takes: a decoder written beside the encoder would share its misreadings,
//! and this one is fuzzed and bounds its own recursion. ADR 0016.
//!
//! The broker answers some messages itself, on this thread, and they reach
//! no ring. Each answer is encoded here and handed to the writer thread,
//! which numbers it in the connection's stream. ADR 0018.
//!
//! A connection proceeds in a fixed order: handshake, then authentication,
//! then everything else. Before authentication it may send `Ping` and
//! `Auth` and nothing else; any other topic closes the connection, and so
//! does a connection that has not authenticated within the handshake
//! timeout. Authentication is one `Auth` carrying the token's
//! secret and one `AuthResult` back; after a failed one the connection is
//! closed, once the result has reached the wire. After a successful one the
//! writer thread is told, and records begin to fan out to the connection.
//! Once authenticated a connection may also send the three messages the
//! broker handles itself: `GetSchema`, answered with the schema or with why
//! there is none; `SeqAck`, consumed and counted and answered by nothing;
//! and `SetEnabled`, the kill switch, applied when the token carries
//! `reload` and refused when it does not. None of them reaches a ring.
//! Every other topic is a record for Lua, and this thread puts it on the
//! ring the registered route map names for it, with the connection's id
//! for the answer to be addressed to. A topic in no route map goes nowhere;
//! a ring with no room turns the record away; neither closes the
//! connection. ADR 0024.
//!
//! A record the broker delivers nowhere is refused out loud: the sender is
//! answered with `Rejected`, carrying the `seq` it gave the record, the
//! topic and the reason, so it can tell which record went nowhere and
//! nothing here has to remember it. A frame that does not parse is not
//! refused this way, because nothing in it can be echoed; that connection
//! is closed.
//!
//! Records for Lua are rate-limited here too, per connection and over
//! every connection together, and the two answer differently: a connection
//! over its own rate is refused the record and kept, because it is
//! misbehaving alone; one that pushes the total over is closed, because
//! that is a capacity problem no refusal fixes. What a connection is told
//! of its refusals is capped as well, so a flood buys no answers. ADR 0026.

use std::collections::HashSet;
use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use dcsbridge_topic::{self as topic, TYPE_URL_PREFIX};
use prost::Message;

use crate::config::Config;
use crate::encode::Encoder;
use crate::fanout::{Capabilities, ConnectionId, Connections};
use crate::registry::{Capability, Member};
use crate::transport::Record;

/// The live keys the reader thread decides by, as of one moment.
///
/// Taken from the configuration in force each time the reader is about to
/// read, so a value swapped in by a later `configure` binds from the next
/// frame on and the deadline of the next connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// How long a connection has to authenticate before it is closed. A
    /// connection that has not done so in this time is not going to.
    pub handshake_timeout: Duration,
    /// The most bytes a frame may claim. Read before anything is allocated
    /// for the frame, because the length is the peer's to write.
    pub max_frame_bytes: u32,
    /// The most bytes a payload's type URL may take. A real one is about
    /// fifty.
    pub max_type_url_bytes: usize,
    /// The most records for Lua one connection may send in a second; the
    /// next is refused and the connection kept.
    pub inbound_records_per_sec: u32,
    /// The most records for Lua every connection together may send in a
    /// second; the connection that sends the next is closed.
    pub inbound_records_per_sec_total: u32,
    /// The most `Rejected` one connection is sent in a second for an
    /// unknown topic, a missing capability or its own rate.
    pub rejected_max_per_sec: u32,
    /// The most `Rejected` one connection is sent in a second for a full
    /// ring, capped apart because it answers a well-behaved consumer.
    pub busy_max_per_sec: u32,
}

impl From<&Config> for Limits {
    fn from(config: &Config) -> Self {
        Limits {
            handshake_timeout: Duration::from_millis(config.handshake_timeout_ms),
            max_frame_bytes: config.max_frame_bytes,
            max_type_url_bytes: config.max_type_url_bytes as usize,
            inbound_records_per_sec: config.inbound_records_per_sec,
            inbound_records_per_sec_total: config.inbound_records_per_sec_total,
            rejected_max_per_sec: config.rejected_max_per_sec,
            busy_max_per_sec: config.busy_max_per_sec,
        }
    }
}

/// A count of what one second admitted, for a cap stated per second.
///
/// The window is fixed rather than sliding: it opens at the first event
/// after the last one closed and holds for a second, and what it admits is
/// what fits under the cap. Across a boundary that lets two seconds' worth
/// through in less than a second, which every cap here can afford. The
/// cap is the caller's, read fresh for every event, so a later `configure`
/// binds on the next event. The clock is the caller's too, so a test can
/// drive it. ADR 0026.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    since: Instant,
    count: u32,
}

impl Window {
    /// A window that opens at the first event.
    pub fn new(now: Instant) -> Self {
        Window {
            since: now,
            count: 0,
        }
    }

    /// Admit one event at `now` under `cap`: true when it fits, false
    /// when it does not. A refused event is not counted, so a cap of zero
    /// refuses everything and a window never fills past its cap.
    pub fn admit(&mut self, now: Instant, cap: u32) -> bool {
        if now.saturating_duration_since(self.since) >= Duration::from_secs(1) {
            self.since = now;
            self.count = 0;
        }
        if self.count < cap {
            self.count += 1;
            true
        } else {
            false
        }
    }
}

impl Default for Limits {
    /// The specification's defaults.
    fn default() -> Self {
        Limits::from(&Config::default())
    }
}

/// The bytes an answer takes at most beyond what it carries: a wrapper and
/// a few short fields. The `Schema` answer is this plus the set.
pub(crate) const ANSWER_BYTES: usize = 128;

/// `dcsbridge.broker.Envelope`, as much of it as the broker reads: `seq` and the
/// payload's `Any`. `epoch` and `mission_time` are the broker's to write, at
/// `begin` on the logic thread, and no consumer's to send, so they are
/// skipped as unknown fields.
#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    /// The consumer's own numbering, read and echoed and never checked.
    #[prost(uint64, tag = "1")]
    pub seq: u64,
    /// The record, behind its type URL.
    #[prost(message, optional, tag = "4")]
    pub payload: Option<Payload>,
}

/// `google.protobuf.Any`, hand-numbered so the shipped build carries no
/// second crate for one two-field message.
#[derive(Clone, PartialEq, Message)]
pub struct Payload {
    /// `type.googleapis.com/` and the payload's fully-qualified type name.
    #[prost(string, tag = "1")]
    pub type_url: String,
    /// The payload's own bytes, decoded only by whoever the topic is for.
    #[prost(bytes = "vec", tag = "2")]
    pub value: Vec<u8>,
}

/// What the broker knows about the sim when it answers a `Ping`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Liveness {
    /// Milliseconds since the logic thread last stamped the heartbeat, or
    /// `None` when it never has.
    pub last_heard_ms: Option<u64>,
    /// Whether that age is under the threshold.
    pub alive: bool,
    /// The effective value of the `enabled` key.
    pub enabled: bool,
}

/// What the reader thread asks of the rest of the broker.
///
/// A trait rather than the bridge itself, so the transport can be stood up
/// in a test with nothing behind it, and so the reader thread names what it
/// reads rather than reaching into the process state.
pub trait Answers: Send + Sync + 'static {
    /// The handshake to greet a connection with, as of now.
    fn handshake(&self) -> Record;
    /// What `Pong` carries, as of now.
    fn liveness(&self) -> Liveness;
    /// The live keys the reader decides by, as of now. The specification's
    /// defaults unless something says otherwise.
    fn limits(&self) -> Limits {
        Limits::default()
    }
    /// Match `secret` against the configured tokens, in constant time, and
    /// open a session on the one that carries it.
    fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError>;
    /// A session's connection has closed.
    fn disconnected(&self, session: &Session);
    /// The schema the broker serves, or `None` until the hook driver hands
    /// one over.
    fn schema(&self) -> Option<Record>;
    /// A consumer reports `seq` as the highest it has durably processed.
    fn seq_ack(&self, seq: u64);
    /// A consumer with the `reload` capability sets the kill switch.
    fn set_enabled(&self, enabled: bool);
    /// A record was refused for `reason`, and the `Rejected` that says so
    /// was sent when `answered` is true and withheld by its cap when it is
    /// not. Every refusal is counted by reason; a withheld one is counted
    /// as suppressed as well. With nothing behind the transport, nothing
    /// counts.
    fn rejected(&self, reason: RejectedReason, answered: bool) {
        let _ = (reason, answered);
    }
    /// Whether a record for Lua at `now` fits under `cap`, the total every
    /// connection together may send in a second. With nothing behind the
    /// transport there is no total, so everything fits.
    fn admit_total(&self, now: Instant, cap: u32) -> bool {
        let _ = (now, cap);
        true
    }
    /// A record for Lua: put it on the ring its route names, or hand it
    /// back. With nothing behind the transport there is no ring, so
    /// nothing is routed.
    fn deliver(&self, command: Command) -> Delivery {
        Delivery::Unrouted(command)
    }
}

/// Why the reader thread closed a connection.
#[derive(Debug)]
pub enum Close {
    /// The peer closed, or the socket failed.
    Io(io::Error),
    /// A frame claimed more than the frame cap.
    FrameTooLong(u32),
    /// The frame's envelope did not decode.
    Envelope(prost::DecodeError),
    /// The envelope carried no payload.
    NoPayload,
    /// The payload's type URL was over the URL cap.
    TypeUrlTooLong(usize),
    /// A topic that is not `Ping` or `Auth` arrived before authentication.
    Unauthenticated(String),
    /// The handshake timeout passed with no authentication.
    HandshakeTimeout,
    /// The payload of a message the broker answers itself did not decode.
    Payload(prost::DecodeError),
    /// The `Auth` failed; the result was sent first.
    AuthFailed(AuthError),
    /// The peer closed cleanly before authenticating.
    Closed,
    /// An authenticated connection sent a second `Auth`. A session is
    /// opened once, and a peer that asks again is not the protocol's.
    SecondAuth,
    /// A record for Lua pushed every connection's total over
    /// `inbound_records_per_sec_total`. Over its own rate a connection is
    /// refused and kept; over everyone's it is closed, because that is a
    /// capacity problem no refusal fixes.
    RateLimitedTotal,
}

impl From<io::Error> for Close {
    fn from(error: io::Error) -> Self {
        Close::Io(error)
    }
}

/// Read one frame: the length, checked against `max_frame_bytes`, then the
/// body, then the envelope out of it.
///
/// `body` is the reader's one buffer, reused across frames and grown to the
/// largest frame seen, so a burst of small frames allocates nothing after
/// the first. It cannot grow past the cap, because the length is checked
/// before the buffer is touched. Returns `None` at a clean end of stream,
/// between frames.
///
/// `limits` is asked once the length prefix has arrived, not before the
/// wait for it, so the caps in force when a frame arrives are the ones it
/// is checked against: a cap lowered while the reader waited binds on that
/// frame. Both caps are applied here, the frame's before the body is
/// allocated for and the type URL's before the envelope decodes.
pub fn read_frame(
    stream: &mut impl Read,
    body: &mut Vec<u8>,
    limits: impl FnOnce() -> Limits,
) -> Result<Option<Envelope>, Close> {
    let mut length = [0u8; 4];
    // The first read tells a clean close between frames from a cut inside
    // one, which `read_exact` cannot; it retries an interrupted read the
    // way `read_exact` does, so a signal is not a closed connection.
    let first = loop {
        match stream.read(&mut length) {
            Ok(n) => break n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(Close::Io(error)),
        }
    };
    match first {
        0 => return Ok(None),
        4 => {}
        n => stream.read_exact(&mut length[n..])?,
    }
    let length = u32::from_le_bytes(length);
    let limits = limits();
    if length > limits.max_frame_bytes {
        return Err(Close::FrameTooLong(length));
    }
    body.clear();
    body.resize(length as usize, 0);
    stream.read_exact(body)?;
    check_type_url(body, limits.max_type_url_bytes)?;
    Envelope::decode(&body[..])
        .map(Some)
        .map_err(Close::Envelope)
}

/// The wire type of a length-delimited field.
const LEN: u64 = 2;

/// The envelope's payload field, and the type URL field inside it.
const PAYLOAD_FIELD: u64 = 4;
const TYPE_URL_FIELD: u64 = 1;

/// Refuse `body` if a type URL in it is over `cap`, before the decoder
/// allocates for one.
///
/// The URL is the one string read out of every frame, and its length is
/// the peer's to write: this is the same check the frame length gets, one
/// descent further in. The walk reads field keys and lengths and nothing
/// else, descends into the payload field only, and stops at the first byte
/// it cannot read, leaving that to the decoder, which reads the bytes in
/// the same order and so cannot reach a string this did not.
fn check_type_url(body: &[u8], cap: usize) -> Result<(), Close> {
    let mut fields = Fields(body);
    while let Some((field, wire, value)) = fields.next() {
        if field == PAYLOAD_FIELD && wire == LEN {
            let mut inner = Fields(value);
            while let Some((field, wire, value)) = inner.next() {
                if field == TYPE_URL_FIELD && wire == LEN && value.len() > cap {
                    return Err(Close::TypeUrlTooLong(value.len()));
                }
            }
        }
    }
    Ok(())
}

/// A walk over the fields of one message's bytes: each step is the field
/// number, the wire type, and the bytes the field spans. A varint's bytes
/// are the varint; a fixed field's are its width; a length-delimited
/// field's are what its length names, and are the only ones anything
/// descends into.
///
/// The walk steps over what the decoder steps over and stops where it
/// stops, because a field the decoder skips and this walk halts at would
/// be a place to hide a string from the check. A group, which the format
/// once had and the decoder still skips, is stepped over whole: every
/// field inside it, groups included, up to the end tag that closes it.
struct Fields<'a>(&'a [u8]);

/// The wire types of a group's start and end tags.
const START_GROUP: u64 = 3;
const END_GROUP: u64 = 4;

impl<'a> Fields<'a> {
    /// The next field outside any group, or `None` at the end or at a byte
    /// the walk cannot read.
    fn next(&mut self) -> Option<(u64, u64, &'a [u8])> {
        let mut depth = 0u32;
        loop {
            if self.0.is_empty() {
                return None;
            }
            let key = self.varint()?;
            let (field, wire) = (key >> 3, key & 7);
            let value = match wire {
                0 => {
                    let start = self.0;
                    self.varint()?;
                    &start[..start.len() - self.0.len()]
                }
                1 => self.take(8)?,
                LEN => {
                    let len = usize::try_from(self.varint()?).ok()?;
                    self.take(len)?
                }
                5 => self.take(4)?,
                START_GROUP => {
                    depth = depth.checked_add(1)?;
                    continue;
                }
                END_GROUP => {
                    // An end with no start is a byte the decoder refuses.
                    depth = depth.checked_sub(1)?;
                    continue;
                }
                // A wire type the format has never had.
                _ => return None,
            };
            if depth == 0 {
                return Some((field, wire, value));
            }
        }
    }

    /// Read one varint, at most ten bytes, refusing a tenth that carries
    /// more than the one bit left, as the decoder does.
    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for (i, byte) in self.0.iter().take(10).enumerate() {
            if i == 9 && *byte > 1 {
                return None;
            }
            value |= u64::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                self.0 = &self.0[i + 1..];
                return Some(value);
            }
        }
        None
    }

    /// Take `len` bytes, or nothing if fewer remain.
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(taken)
    }
}

/// The topic a frame carries: its type URL with the prefix every runtime
/// writes taken off. The URL was held under `max_type_url_bytes` before
/// the envelope decoded. A URL without the prefix is a topic nothing
/// routes, and it comes back whole so the refusal can name it.
pub fn topic(envelope: &Envelope) -> Result<&str, Close> {
    let payload = envelope.payload.as_ref().ok_or(Close::NoPayload)?;
    let url = payload.type_url.as_str();
    Ok(url.strip_prefix(TYPE_URL_PREFIX).unwrap_or(url))
}

/// `dcsbridge.broker.Pong` as an envelope tail.
pub fn pong(liveness: Liveness) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES);
    e.begin(topic::PONG.as_bytes(), None);
    e.boolean(1, liveness.alive).expect("the answer fits");
    if let Some(ms) = liveness.last_heard_ms {
        // A uint64 is a varint of the same bits an int64 is.
        e.integer(2, ms as i64).expect("the answer fits");
    }
    e.boolean(3, liveness.enabled).expect("the answer fits");
    Record::from(e.commit().expect("the answer fits"))
}

/// What a connection is once it has authenticated: which token, and what
/// that token may do. Held by the reader thread for the connection's life.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    /// The token's id, which names it in stats and audit lines and is never
    /// the secret.
    pub token_id: String,
    /// The capabilities the token grants.
    pub caps: HashSet<Capability>,
}

impl Session {
    /// The capability set as the writer thread holds it, for the filter at
    /// fan-out.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
            .iter()
            .fold(Capabilities::NONE, |set, cap| set.with(cap.number()))
    }
}

/// An inbound record as the rings carry it from the reader thread to Lua:
/// who sent it, what topic it is on, and the payload's own bytes.
///
/// The topic is the type URL with the prefix taken off, which is the name
/// the route map and the generated decoders know it by. The bytes are the
/// `Any`'s value, decoded by whoever the topic is for and by nothing here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    /// The connection it arrived on, for the answer to be addressed to.
    pub from: ConnectionId,
    /// The topic, as registered.
    pub topic: String,
    /// The payload's bytes, opaque.
    pub value: Vec<u8>,
}

impl Command {
    /// Take the topic and the bytes out of a decoded envelope.
    ///
    /// The prefix is drained off the front of the string the decoder
    /// allocated, so the topic costs no second allocation.
    pub fn from_envelope(from: ConnectionId, envelope: Envelope) -> Result<Self, Close> {
        let Payload {
            mut type_url,
            value,
        } = envelope.payload.ok_or(Close::NoPayload)?;
        if type_url.starts_with(TYPE_URL_PREFIX) {
            type_url.drain(..TYPE_URL_PREFIX.len());
        }
        Ok(Command {
            from,
            topic: type_url,
            value,
        })
    }
}

/// What became of a command handed to the rings.
///
/// A command the rings do not keep comes back, as a ring hands back what
/// it turns away: the reader thread is the one that can tell the sender,
/// and it decides where the record is dropped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// The ring its route names holds it.
    Stored,
    /// No route map names its topic, so it went nowhere. Counted in
    /// `unrouted_topic_total`.
    Unrouted(Command),
    /// The ring its route names was full, so the newest record, this one,
    /// was turned away. Counted against that ring.
    Busy(Command),
}

/// Why a record was refused. Mirrors `dcsbridge.broker.RejectedReason`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectedReason {
    /// No route map names the topic.
    UnknownTopic = 1,
    /// The session's token lacks the capability the topic requires.
    NoCapability = 2,
    /// The connection is over `inbound_records_per_sec`.
    RateLimited = 3,
    /// The ring the route names had no room.
    Busy = 4,
}

impl RejectedReason {
    /// Every member, in wire order, for a counter per reason.
    pub const ALL: [RejectedReason; 4] = [
        RejectedReason::UnknownTopic,
        RejectedReason::NoCapability,
        RejectedReason::RateLimited,
        RejectedReason::Busy,
    ];
}

/// `dcsbridge.broker.Rejected` as an envelope tail: the sender's `seq` and
/// topic echoed, and why the record went nowhere. The topic is the
/// sender's own string, held under `max_type_url_bytes` when it was read,
/// so the answer is sized for it.
pub fn rejected(seq: u64, topic: &str, reason: RejectedReason) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES + topic.len());
    e.begin(topic::REJECTED.as_bytes(), None);
    // A uint64 is a varint of the same bits an int64 is.
    e.integer(1, seq as i64).expect("the answer fits");
    e.string(2, topic.as_bytes()).expect("the answer fits");
    e.integer(3, reason as i64).expect("the answer fits");
    Record::from(e.commit().expect("the answer fits"))
}

/// Why an `Auth` failed. Mirrors `dcsbridge.broker.AuthError`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    /// No configured token carries the secret.
    BadToken = 1,
    /// The token matched and grants nothing.
    EmptyCapabilitySet = 2,
    /// `max_connections` have authenticated already.
    ServerFull = 3,
}

/// `dcsbridge.broker.Auth` as the broker reads it: the token's secret and
/// nothing else.
#[derive(Clone, PartialEq, Message)]
pub struct Auth {
    /// The secret. Never logged, never echoed.
    #[prost(string, tag = "1")]
    pub token: String,
}

/// `dcsbridge.broker.SeqAck` as the broker reads it.
#[derive(Clone, PartialEq, Message)]
pub struct SeqAck {
    /// The highest `seq` the consumer has durably processed.
    #[prost(uint64, tag = "1")]
    pub seq: u64,
}

/// `dcsbridge.broker.SetEnabled` as the broker reads it.
#[derive(Clone, PartialEq, Message)]
pub struct SetEnabled {
    /// The value the `enabled` key takes.
    #[prost(bool, tag = "1")]
    pub enabled: bool,
}

/// What `Schema` says while there is no schema to serve.
pub const NO_SCHEMA: &str = "no schema has been handed to the broker";

/// `dcsbridge.broker.Schema` as an envelope tail: the set, or why not.
pub fn schema(set: Option<&[u8]>) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES + set.map_or(0, <[u8]>::len));
    e.begin(topic::SCHEMA.as_bytes(), None);
    match set {
        Some(set) => e.string(1, set).expect("the answer fits"),
        None => e.string(2, NO_SCHEMA.as_bytes()).expect("the answer fits"),
    }
    Record::from(e.commit().expect("the answer fits"))
}

/// `dcsbridge.broker.AuthResult` as an envelope tail.
pub fn auth_result(result: Result<(), AuthError>) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES);
    e.begin(topic::AUTH_RESULT.as_bytes(), None);
    e.boolean(1, result.is_ok()).expect("the answer fits");
    if let Err(error) = result {
        e.integer(2, error as i64).expect("the answer fits");
    }
    Record::from(e.commit().expect("the answer fits"))
}

/// How long the reader waits for a failed `AuthResult` to be written before
/// it closes the socket anyway. Loopback takes microseconds; a peer that
/// has stopped reading does not get to hold the reader. A peer that
/// flooded `Ping` past its ring before the `Auth` has had an answer
/// evicted, so the count is never reached and the whole wait is paid; that
/// holds its own reader thread and nothing else.
const FLUSH_WAIT: Duration = Duration::from_secs(2);

/// One connection's reader: read frames until the connection closes, and
/// answer what the broker answers itself.
///
/// The read blocks, so this is a thread of its own per connection, beside
/// the one that drains the connection's ring. Whichever of the two returns
/// first shuts the socket down, which returns the other. Nothing here
/// touches Lua or waits for the logic thread, which is what lets a `Ping`
/// be answered while the sim is loading a mission.
///
/// `written` is the drainer's count of frames it has put on the socket.
/// Before authentication nothing is fanned out to the connection, so every
/// frame on it is an answer this thread sent or the handshake, and this
/// thread can count them: that is how a failed `AuthResult` is known to
/// have reached the wire before the close that follows it.
pub fn serve(
    stream: TcpStream,
    id: ConnectionId,
    connections: &Connections<Record>,
    answers: &dyn Answers,
    written: &AtomicU64,
    session: &mut Option<Session>,
) -> Result<(), Close> {
    let mut body = Vec::new();
    // The handshake, then each answer sent before authentication.
    let mut answered: u64 = 1;
    // The deadline is wall-clock: every read under it, a frame's length
    // and its body alike, is armed with what is left, so neither a peer
    // that keeps sending `Ping` nor one that trickles a frame a byte at a
    // time can stay past it. It is lifted once the connection authenticates.
    let mut stream = Deadline {
        stream,
        until: Some(Instant::now() + answers.limits().handshake_timeout),
    };
    let mut refusals = Refusals::new(Instant::now());
    // What this connection has sent for Lua this second. The messages the
    // broker answers itself are not counted: the limit protects what the
    // sim driver can dispatch, and they never reach it.
    let mut sent = Window::new(Instant::now());

    loop {
        // Asked for as each frame arrives, so a cap a later `configure`
        // lowered binds on the next frame rather than the next connection.
        let envelope = match read_frame(&mut stream, &mut body, || answers.limits()) {
            Ok(Some(read)) => read,
            Ok(None) if session.is_some() => return Ok(()),
            Ok(None) => return Err(Close::Closed),
            Err(Close::Io(error))
                if session.is_none()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
            {
                return Err(Close::HandshakeTimeout);
            }
            Err(close) => return Err(close),
        };

        match (topic(&envelope)?, &*session) {
            (topic::PING, _) => {
                connections.answer(id, pong(answers.liveness()));
                answered += 1;
            }
            (topic::AUTH, None) => {
                let auth = Auth::decode(payload(&envelope)).map_err(Close::Payload)?;
                match answers.authenticate(auth.token.as_bytes()) {
                    Ok(opened) => {
                        connections.answer(id, auth_result(Ok(())));
                        // After the answer, on the same channel: the
                        // consumer reads its result before any record.
                        connections.authenticated(id, opened.capabilities());
                        // The deadline is lifted: an authenticated peer may
                        // be silent for as long as it likes.
                        stream.until = None;
                        stream.stream.set_read_timeout(None)?;
                        *session = Some(opened);
                    }
                    Err(error) => {
                        connections.answer(id, auth_result(Err(error)));
                        answered += 1;
                        wait_for_flush(written, answered);
                        return Err(Close::AuthFailed(error));
                    }
                }
            }
            (other, None) => return Err(Close::Unauthenticated(other.to_owned())),
            (topic::AUTH, Some(_)) => return Err(Close::SecondAuth),
            (topic::GET_SCHEMA, Some(_)) => {
                connections.answer(id, schema(answers.schema().as_deref()));
            }
            (topic::SEQ_ACK, Some(_)) => {
                let ack = SeqAck::decode(payload(&envelope)).map_err(Close::Payload)?;
                answers.seq_ack(ack.seq);
            }
            (refused @ topic::SET_ENABLED, Some(opened)) => {
                let set = SetEnabled::decode(payload(&envelope)).map_err(Close::Payload)?;
                // Nothing answers this when it is applied. A token without
                // `reload` leaves the switch alone and is told so.
                if opened.caps.contains(&Capability::Reload) {
                    answers.set_enabled(set.enabled);
                } else {
                    refuse(
                        connections,
                        answers,
                        id,
                        &mut refusals,
                        envelope.seq,
                        refused,
                        RejectedReason::NoCapability,
                    );
                }
            }
            // Every other topic is a record for Lua, on the ring its route
            // names. What the rings hand back is refused here, on the
            // thread that read it, with the sender's own `seq` so it can
            // tell which record went nowhere.
            (over, Some(_)) => {
                let seq = envelope.seq;
                // The connection's own rate first, then everyone's: a
                // record refused for the first is delivered nowhere, so it
                // is not counted against the second.
                let now = Instant::now();
                let limits = answers.limits();
                if !sent.admit(now, limits.inbound_records_per_sec) {
                    refuse(
                        connections,
                        answers,
                        id,
                        &mut refusals,
                        seq,
                        over,
                        RejectedReason::RateLimited,
                    );
                    continue;
                }
                if !answers.admit_total(now, limits.inbound_records_per_sec_total) {
                    return Err(Close::RateLimitedTotal);
                }
                let command = Command::from_envelope(id, envelope)?;
                let (command, reason) = match answers.deliver(command) {
                    Delivery::Stored => continue,
                    Delivery::Unrouted(command) => (command, RejectedReason::UnknownTopic),
                    Delivery::Busy(command) => (command, RejectedReason::Busy),
                };
                refuse(
                    connections,
                    answers,
                    id,
                    &mut refusals,
                    seq,
                    &command.topic,
                    reason,
                );
            }
        }
    }
}

/// What one connection is told of its refusals per second.
///
/// Two windows, because the two caps mean different things: an unknown
/// topic, a missing capability or the connection's own rate say the
/// consumer is misbuilt or misbehaving, and a low cap keeps one from
/// filling a log; a full ring says the broker is congested, and its cap is
/// set so a correct consumer hears of every record it lost. A refusal over
/// its cap is counted and answered with nothing. ADR 0026.
struct Refusals {
    rejected: Window,
    busy: Window,
}

impl Refusals {
    fn new(now: Instant) -> Self {
        Refusals {
            rejected: Window::new(now),
            busy: Window::new(now),
        }
    }

    /// Whether a refusal for `reason` at `now` is answered under `limits`.
    fn admit(&mut self, now: Instant, limits: &Limits, reason: RejectedReason) -> bool {
        match reason {
            RejectedReason::Busy => self.busy.admit(now, limits.busy_max_per_sec),
            RejectedReason::UnknownTopic
            | RejectedReason::NoCapability
            | RejectedReason::RateLimited => self.rejected.admit(now, limits.rejected_max_per_sec),
        }
    }
}

/// Answer the record numbered `seq` on `topic` with a `Rejected` for
/// `reason` if the connection's cap on that reason admits one, and count
/// it either way. The cap is asked for now, as the frame cap was, so a cap
/// lowered by a later `configure` binds on this refusal.
fn refuse(
    connections: &Connections<Record>,
    answers: &dyn Answers,
    id: ConnectionId,
    refusals: &mut Refusals,
    seq: u64,
    topic: &str,
    reason: RejectedReason,
) {
    let answered = refusals.admit(Instant::now(), &answers.limits(), reason);
    if answered {
        connections.answer(id, rejected(seq, topic, reason));
    }
    answers.rejected(reason, answered);
}

/// A socket read under a wall-clock deadline.
///
/// The socket's own timeout bounds one read, so a peer that trickles a
/// frame a byte at a time would pass a fixed timeout on every read and
/// never be caught by it. Before each read this re-arms the timeout with
/// what is left of the deadline, and reports a passed deadline as a timed
/// out read. With no deadline it is the socket, unbounded.
struct Deadline {
    stream: TcpStream,
    until: Option<Instant>,
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(until) = self.until {
            // A zero timeout is refused by the socket, so a deadline that
            // has passed is reported without a read.
            let left = until
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
            self.stream.set_read_timeout(Some(left))?;
        }
        self.stream.read(buf)
    }
}

/// The payload's bytes, which [`topic`] has already established are there.
fn payload(envelope: &Envelope) -> &[u8] {
    envelope
        .payload
        .as_ref()
        .map_or(&[], |payload| &payload.value[..])
}

/// Wait until the drainer has written `frames` frames, or [`FLUSH_WAIT`]
/// has passed.
fn wait_for_flush(written: &AtomicU64, frames: u64) {
    let deadline = Instant::now() + FLUSH_WAIT;
    while written.load(Ordering::Acquire) < frames && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
}

/// Run a reader to its end and shut the socket down after it, whatever the
/// end was: a clean close, a refusal, or a fault caught at this thread.
///
/// The shutdown is what returns the draining thread, which is blocked on
/// the same socket, and the drain thread's detach is what tells the writer
/// thread. A panic is caught here rather than left to end the thread on
/// its own so that the shutdown still happens; the panic has already been
/// reported by the hook, and a decoder fault is one connection's, never
/// the process's. A session that was open is reported closed, so the
/// count behind `max_connections` comes back down.
pub fn run(
    stream: TcpStream,
    id: ConnectionId,
    connections: Connections<Record>,
    answers: &dyn Answers,
    written: &AtomicU64,
) {
    // The session lives here rather than in `serve`, so that every way
    // out of it, a clean close, a refusal, a socket error or a caught
    // panic, reports the session closed: a session lost on any of those
    // paths would hold its slot under `max_connections` for the life of
    // the process, and eight such losses would refuse every consumer.
    let mut session = None;
    let reading = stream.try_clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match reading {
        Ok(reading) => serve(reading, id, &connections, answers, written, &mut session),
        Err(error) => Err(Close::Io(error)),
    }));
    drop(outcome);
    let _ = stream.shutdown(Shutdown::Both);
    if let Some(session) = session {
        answers.disconnected(&session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A type URL as a stock encoder writes one.
    fn type_url(topic: &str) -> String {
        format!("{TYPE_URL_PREFIX}{topic}")
    }

    /// A frame as a consumer's stock encoder writes it.
    fn frame(seq: u64, type_url: &str, value: &[u8]) -> Vec<u8> {
        let body = Envelope {
            seq,
            payload: Some(Payload {
                type_url: type_url.to_owned(),
                value: value.to_vec(),
            }),
        }
        .encode_to_vec();
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    /// A window admits its cap and no more, opens again a second after it
    /// opened, and counts nothing it refused; a cap of zero refuses every
    /// event and a cap raised mid-window binds at once.
    #[test]
    fn a_window_admits_its_cap_per_second() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut window = Window::new(start);

        assert!(window.admit(at(0), 2));
        assert!(window.admit(at(10), 2));
        assert!(!window.admit(at(20), 2), "a third event fit under two");
        assert!(!window.admit(at(999), 2), "the window closed early");
        assert!(window.admit(at(1000), 2), "the window did not open again");
        assert!(window.admit(at(1001), 2));
        assert!(!window.admit(at(1002), 2));

        // The refused events were not counted, so the cap raised by one
        // admits exactly one more.
        assert!(window.admit(at(1003), 3));
        assert!(!window.admit(at(1004), 3));

        // A second window opens at the first event after the boundary, not
        // at the boundary: an idle connection starts fresh.
        assert!(window.admit(at(5500), 1));
        assert!(!window.admit(at(6499), 1));
        assert!(window.admit(at(6500), 1));

        let mut shut = Window::new(start);
        assert!(!shut.admit(at(0), 0), "a cap of zero admitted an event");
        assert!(!shut.admit(at(2000), 0));

        // A clock that reads earlier than the window opened is inside it.
        let mut early = Window::new(at(100));
        assert!(early.admit(at(0), 1));
        assert!(!early.admit(at(50), 1));
    }

    /// The reader's limits are the configuration's rate keys as well as its
    /// size keys, so a rate lowered by a later `configure` binds like a
    /// cap does.
    #[test]
    fn the_limits_carry_the_rate_keys() {
        let config = Config {
            inbound_records_per_sec: 7,
            inbound_records_per_sec_total: 8,
            rejected_max_per_sec: 9,
            busy_max_per_sec: 10,
            ..Config::default()
        };
        let limits = Limits::from(&config);
        assert_eq!(limits.inbound_records_per_sec, 7);
        assert_eq!(limits.inbound_records_per_sec_total, 8);
        assert_eq!(limits.rejected_max_per_sec, 9);
        assert_eq!(limits.busy_max_per_sec, 10);
    }

    /// A frame decodes to its `seq` and its topic, and a topic with no
    /// prefix comes back whole.
    #[test]
    fn a_frame_decodes_to_seq_and_topic() {
        let bytes = frame(7, &type_url(topic::PING), &[]);
        let mut body = Vec::new();
        let envelope = read_frame(&mut &bytes[..], &mut body, Limits::default)
            .unwrap()
            .unwrap();
        assert_eq!(envelope.seq, 7);
        assert_eq!(topic(&envelope).unwrap(), topic::PING);

        let bytes = frame(8, topic::PING, &[]);
        let envelope = read_frame(&mut &bytes[..], &mut body, Limits::default)
            .unwrap()
            .unwrap();
        assert_eq!(topic(&envelope).unwrap(), topic::PING);

        // At a clean end of stream the limits are never asked for.
        assert!(matches!(
            read_frame(&mut &[][..], &mut body, || unreachable!("no frame arrived")),
            Ok(None)
        ));
    }

    /// A length over the cap is refused before the body is read, a body
    /// that is not an envelope is refused, and so is one with no payload
    /// or a type URL over its cap.
    #[test]
    fn a_bad_frame_is_refused_with_its_reason() {
        let mut body = Vec::new();
        let Limits {
            max_frame_bytes,
            max_type_url_bytes,
            ..
        } = Limits::default();

        // A read interrupted by a signal is retried, not taken for a close.
        struct Interrupted<'a>(&'a [u8], bool);
        impl Read for Interrupted<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if !self.1 {
                    self.1 = true;
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                self.0.read(buf)
            }
        }
        let bytes = frame(3, topic::PING, &[]);
        let envelope = read_frame(
            &mut Interrupted(&bytes, false),
            &mut Vec::new(),
            Limits::default,
        )
        .expect("an interrupted read is retried")
        .expect("a frame follows it");
        assert_eq!(envelope.seq, 3);

        let mut bytes = (max_frame_bytes + 1).to_le_bytes().to_vec();
        bytes.extend([0u8; 16]);
        assert!(matches!(
            read_frame(&mut &bytes[..], &mut body, Limits::default),
            Err(Close::FrameTooLong(n)) if n == max_frame_bytes + 1
        ));
        assert!(body.is_empty(), "a refused length grew the buffer");
        // The cap is whatever is answered once the length is in, so a
        // lower one refuses a frame the default would have read.
        let small = frame(1, topic::PING, &[]);
        let lowered = || Limits {
            max_frame_bytes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            read_frame(&mut &small[..], &mut body, lowered),
            Err(Close::FrameTooLong(_))
        ));

        let garbage = [3u8, 0, 0, 0, 0xff, 0xff, 0xff];
        assert!(matches!(
            read_frame(&mut &garbage[..], &mut body, Limits::default),
            Err(Close::Envelope(_))
        ));

        let short = [8u8, 0, 0, 0, 1, 2];
        assert!(matches!(
            read_frame(&mut &short[..], &mut body, Limits::default),
            Err(Close::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof
        ));

        let bare = Envelope {
            seq: 1,
            payload: None,
        };
        assert!(matches!(topic(&bare), Err(Close::NoPayload)));

        let long = frame(1, &"x".repeat(max_type_url_bytes + 1), &[]);
        assert!(matches!(
            read_frame(&mut &long[..], &mut body, Limits::default),
            Err(Close::TypeUrlTooLong(n)) if n == max_type_url_bytes + 1
        ));
        let raised = || Limits {
            max_type_url_bytes: max_type_url_bytes + 1,
            ..Limits::default()
        };
        let envelope = read_frame(&mut &long[..], &mut body, raised)
            .expect("a raised cap admits the URL")
            .expect("a frame follows");
        assert_eq!(
            envelope.payload.unwrap().type_url.len(),
            max_type_url_bytes + 1
        );
    }

    /// The URL cap is applied to the bytes before the envelope decodes:
    /// a URL at the cap passes, one over it is refused however it is
    /// placed in the frame, and a frame the walk cannot read is left to the
    /// decoder to refuse.
    #[test]
    fn the_type_url_is_checked_before_the_envelope_decodes() {
        let cap = 16;
        let at = "y".repeat(cap);
        let over = "y".repeat(cap + 1);

        assert!(check_type_url(&frame(1, &at, b"value")[4..], cap).is_ok());
        assert!(matches!(
            check_type_url(&frame(1, &over, b"value")[4..], cap),
            Err(Close::TypeUrlTooLong(n)) if n == cap + 1
        ));
        // A frame with no payload has no URL to check.
        let bare = Envelope {
            seq: 5,
            payload: None,
        }
        .encode_to_vec();
        assert!(check_type_url(&bare, cap).is_ok());

        // Fields ahead of the payload, of every wire type, are stepped over:
        // a varint, a fixed64, a length-delimited one and a fixed32.
        let mut ahead = vec![0x08, 0xff, 0x01, 0x11];
        ahead.extend([0u8; 8]);
        ahead.extend([0x1a, 0x02, 0xaa, 0xbb, 0x15, 0, 0, 0, 0]);
        let mut placed = ahead.clone();
        placed.extend(&frame(1, &over, &[])[4..]);
        assert!(matches!(
            check_type_url(&placed, cap),
            Err(Close::TypeUrlTooLong(n)) if n == cap + 1
        ));
        // Inside the payload the value can come first, and every URL is
        // checked: a second one over the cap is refused too.
        let mut payload = vec![0x12, 0x03, 1, 2, 3, 0x0a, cap as u8];
        payload.extend(at.as_bytes());
        payload.extend([0x0a, cap as u8 + 1]);
        payload.extend(over.as_bytes());
        let mut twice = vec![0x22, payload.len() as u8];
        twice.extend(&payload);
        assert!(matches!(
            check_type_url(&twice, cap),
            Err(Close::TypeUrlTooLong(n)) if n == cap + 1
        ));

        // A group the decoder skips is stepped over, not stopped at: an
        // empty group on an unused field, a group holding fields of every
        // kind and a group inside it, and a group holding what looks like
        // the payload, each ahead of a payload whose URL is over the cap.
        let over_payload = &frame(1, &over, &[])[4..];
        let inner_group = [0x2b, 0x08, 0x01, 0x2c];
        let mut nested = vec![0x33, 0x08, 0x01, 0x11];
        nested.extend([0u8; 8]);
        nested.extend([0x1a, 0x01, 0x00, 0x15, 0, 0, 0, 0]);
        nested.extend(inner_group);
        nested.extend([0x34]);
        let mut fake_payload = vec![0x2b];
        fake_payload.extend(over_payload);
        fake_payload.extend([0x2c]);
        for group in [vec![0x2b, 0x2c], nested, fake_payload] {
            let mut hidden = group;
            hidden.extend(over_payload);
            assert!(
                matches!(
                    check_type_url(&hidden, cap),
                    Err(Close::TypeUrlTooLong(n)) if n == cap + 1
                ),
                "a group hid the URL from the check: {hidden:02x?}"
            );
        }

        // A length past the end, an unfinished varint, an unclosed group, an
        // end with no start and a tenth varint byte past the one bit left
        // each end the walk without a verdict; the decoder refuses the frame.
        for unreadable in [
            vec![0x22u8, 0x7f, 0x0a, 0x01],
            vec![0x22u8, 0x80],
            vec![0x23u8, 0x0a, 0x01, 0x41],
            vec![0x2cu8, 0x0a, 0x01, 0x41],
            vec![
                0x08u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
            ],
        ] {
            assert!(check_type_url(&unreadable, cap).is_ok());
            let mut bytes = (unreadable.len() as u32).to_le_bytes().to_vec();
            bytes.extend(&unreadable);
            assert!(matches!(
                read_frame(&mut &bytes[..], &mut Vec::new(), Limits::default),
                Err(Close::Envelope(_))
            ));
        }
    }

    /// `AuthResult` decodes to `ok` with no error, or to the error's schema
    /// number with `ok` false, and `Auth` round-trips its secret.
    #[test]
    fn auth_messages_decode_as_the_schema_numbers_them() {
        #[derive(Clone, PartialEq, Message)]
        struct AuthResult {
            #[prost(bool, tag = "1")]
            ok: bool,
            #[prost(int32, tag = "2")]
            error: i32,
        }
        #[derive(Clone, PartialEq, Message)]
        struct Tail {
            #[prost(message, optional, tag = "4")]
            payload: Option<Payload>,
        }
        let decode = |tail: Record| {
            let any = Tail::decode(&tail[..]).unwrap().payload.unwrap();
            assert_eq!(any.type_url, type_url(topic::AUTH_RESULT));
            AuthResult::decode(&any.value[..]).unwrap()
        };

        assert_eq!(
            decode(auth_result(Ok(()))),
            AuthResult { ok: true, error: 0 }
        );
        for error in [
            AuthError::BadToken,
            AuthError::EmptyCapabilitySet,
            AuthError::ServerFull,
        ] {
            assert_eq!(
                decode(auth_result(Err(error))),
                AuthResult {
                    ok: false,
                    error: error as i32
                }
            );
        }

        let auth = Auth {
            token: "s3cret".into(),
        }
        .encode_to_vec();
        assert_eq!(Auth::decode(&auth[..]).unwrap().token, "s3cret");
    }

    /// `Schema` carries the set when there is one and the error when there
    /// is not, never both, and a set the size of a real one fits.
    #[test]
    fn schema_carries_the_set_or_the_error() {
        #[derive(Clone, PartialEq, Message)]
        struct Schema {
            #[prost(bytes = "vec", optional, tag = "1")]
            file_descriptor_set: Option<Vec<u8>>,
            #[prost(string, optional, tag = "2")]
            error: Option<String>,
        }
        #[derive(Clone, PartialEq, Message)]
        struct Tail {
            #[prost(message, optional, tag = "4")]
            payload: Option<Payload>,
        }
        let decode = |tail: Record| {
            let any = Tail::decode(&tail[..]).unwrap().payload.unwrap();
            assert_eq!(any.type_url, type_url(topic::SCHEMA));
            Schema::decode(&any.value[..]).unwrap()
        };

        assert_eq!(
            decode(schema(None)),
            Schema {
                file_descriptor_set: None,
                error: Some(NO_SCHEMA.into()),
            }
        );
        let set = vec![0x5a; 100_000];
        assert_eq!(
            decode(schema(Some(&set))),
            Schema {
                file_descriptor_set: Some(set),
                error: None,
            }
        );

        let ack = SeqAck { seq: u64::MAX }.encode_to_vec();
        assert_eq!(SeqAck::decode(&ack[..]).unwrap().seq, u64::MAX);
        let off = SetEnabled { enabled: false }.encode_to_vec();
        assert!(!SetEnabled::decode(&off[..]).unwrap().enabled);
    }

    /// `Pong` decodes to what it was given, with the age absent when the
    /// heartbeat was never stamped.
    #[test]
    fn pong_carries_the_liveness_it_was_given() {
        #[derive(Clone, PartialEq, Message)]
        struct Pong {
            #[prost(bool, tag = "1")]
            dcs_alive: bool,
            #[prost(uint64, optional, tag = "2")]
            dcs_last_heard_ms: Option<u64>,
            #[prost(bool, tag = "3")]
            bridge_enabled: bool,
        }
        #[derive(Clone, PartialEq, Message)]
        struct Tail {
            #[prost(message, optional, tag = "4")]
            payload: Option<Payload>,
        }
        let decode = |tail: Record| {
            let any = Tail::decode(&tail[..]).unwrap().payload.unwrap();
            assert_eq!(any.type_url, type_url(topic::PONG));
            Pong::decode(&any.value[..]).unwrap()
        };

        let never = decode(pong(Liveness {
            last_heard_ms: None,
            alive: false,
            enabled: true,
        }));
        assert_eq!(
            never,
            Pong {
                dcs_alive: false,
                dcs_last_heard_ms: None,
                bridge_enabled: true
            }
        );

        let recent = decode(pong(Liveness {
            last_heard_ms: Some(u64::MAX - 5),
            alive: true,
            enabled: false,
        }));
        assert_eq!(
            recent,
            Pong {
                dcs_alive: true,
                dcs_last_heard_ms: Some(u64::MAX - 5),
                bridge_enabled: false
            }
        );
    }
}
