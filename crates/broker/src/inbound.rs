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
//! The length prefix is checked against [`MAX_FRAME_BYTES`] before a byte of
//! the frame is allocated for, and the payload's type URL, which is the one
//! string read out of every frame, is checked against [`MAX_TYPE_URL_BYTES`].
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
//! does a connection that has not authenticated within
//! [`HANDSHAKE_TIMEOUT`]. Authentication is one `Auth` carrying the token's
//! secret and one `AuthResult` back; after a failed one the connection is
//! closed, once the result has reached the wire. After a successful one the
//! writer thread is told, and records begin to fan out to the connection.
//! What an authenticated connection may send beyond `Ping` is the next
//! thing built, so for now any other topic closes it.

use std::collections::HashSet;
use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use prost::Message;

use crate::encode::{Encoder, TYPE_URL_PREFIX};
use crate::fanout::{ConnectionId, Connections};
use crate::state::Capability;
use crate::transport::Record;

/// The most bytes a frame may claim. Read before anything is allocated for
/// the frame, because the length is the peer's to write.
///
/// A single large reply is the largest legitimate frame and this is about
/// ten times one. The first `configure` is where the configured value
/// arrives.
pub const MAX_FRAME_BYTES: u32 = 1 << 20;

/// The most bytes a payload's type URL may take. A real one is about forty.
pub const MAX_TYPE_URL_BYTES: usize = 256;

/// How long a connection has to authenticate before it is closed. A
/// connection that has not done so in this time is not going to.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// The bytes an answer takes at most: a wrapper and a few short fields.
const ANSWER_BYTES: usize = 128;

/// `dcs.bridge.Envelope`, as much of it as the broker reads: `seq` and the
/// payload's `Any`. `epoch` and `mission_time` are the broker's to write and
/// no consumer's to send, so they are skipped as unknown fields.
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
    /// How long a connection has to authenticate. The specification's
    /// default unless something says otherwise.
    fn handshake_timeout(&self) -> Duration {
        HANDSHAKE_TIMEOUT
    }
    /// Match `secret` against the configured tokens, in constant time, and
    /// open a session on the one that carries it.
    fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError>;
    /// A session's connection has closed.
    fn disconnected(&self, session: &Session);
}

/// Why the reader thread closed a connection.
#[derive(Debug)]
pub enum Close {
    /// The peer closed, or the socket failed.
    Io(io::Error),
    /// A frame claimed more than [`MAX_FRAME_BYTES`].
    FrameTooLong(u32),
    /// The frame's envelope did not decode.
    Envelope(prost::DecodeError),
    /// The envelope carried no payload.
    NoPayload,
    /// The payload's type URL was over [`MAX_TYPE_URL_BYTES`].
    TypeUrlTooLong(usize),
    /// A topic that is not `Ping` or `Auth` arrived before authentication.
    Unauthenticated(String),
    /// [`HANDSHAKE_TIMEOUT`] passed with no authentication.
    HandshakeTimeout,
    /// The payload of a message the broker answers itself did not decode.
    Payload(prost::DecodeError),
    /// The `Auth` failed; the result was sent first.
    AuthFailed(AuthError),
    /// The peer closed cleanly before authenticating.
    Closed,
    /// An authenticated connection sent a topic nothing here handles yet.
    Unrouted(String),
}

impl From<io::Error> for Close {
    fn from(error: io::Error) -> Self {
        Close::Io(error)
    }
}

/// Read one frame: the length, checked, then the body, then the envelope
/// out of it.
///
/// `body` is the reader's one buffer, reused across frames and grown to the
/// largest frame seen, so a burst of small frames allocates nothing after
/// the first. It cannot grow past [`MAX_FRAME_BYTES`], because the length is
/// checked before the buffer is touched. Returns `None` at a clean end of
/// stream, between frames.
pub fn read_frame(stream: &mut impl Read, body: &mut Vec<u8>) -> Result<Option<Envelope>, Close> {
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
    if length > MAX_FRAME_BYTES {
        return Err(Close::FrameTooLong(length));
    }
    body.clear();
    body.resize(length as usize, 0);
    stream.read_exact(body)?;
    Envelope::decode(&body[..])
        .map(Some)
        .map_err(Close::Envelope)
}

/// The topic a frame carries: its type URL with the prefix every runtime
/// writes taken off, checked for length first. A URL without the prefix is
/// a topic nothing routes, and it comes back whole so the refusal can name
/// it.
pub fn topic(envelope: &Envelope) -> Result<&str, Close> {
    let payload = envelope.payload.as_ref().ok_or(Close::NoPayload)?;
    let url = payload.type_url.as_str();
    if url.len() > MAX_TYPE_URL_BYTES {
        return Err(Close::TypeUrlTooLong(url.len()));
    }
    Ok(url
        .strip_prefix(std::str::from_utf8(TYPE_URL_PREFIX).expect("the prefix is ASCII"))
        .unwrap_or(url))
}

/// `dcs.bridge.Pong` as an envelope tail.
pub fn pong(liveness: Liveness) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES);
    e.begin(b"dcs.bridge.Pong");
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

/// Why an `Auth` failed. Mirrors `dcs.bridge.AuthError`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    /// No configured token carries the secret.
    BadToken = 1,
    /// The token matched and grants nothing.
    EmptyCapabilitySet = 2,
    /// `max_connections` have authenticated already.
    ServerFull = 3,
}

/// `dcs.bridge.Auth` as the broker reads it: the token's secret and
/// nothing else.
#[derive(Clone, PartialEq, Message)]
pub struct Auth {
    /// The secret. Never logged, never echoed.
    #[prost(string, tag = "1")]
    pub token: String,
}

/// `dcs.bridge.AuthResult` as an envelope tail.
pub fn auth_result(result: Result<(), AuthError>) -> Record {
    let mut e = Encoder::with_capacity(ANSWER_BYTES);
    e.begin(b"dcs.bridge.AuthResult");
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
        until: Some(Instant::now() + answers.handshake_timeout()),
    };

    loop {
        let envelope = match read_frame(&mut stream, &mut body) {
            Ok(Some(envelope)) => envelope,
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
            ("dcs.bridge.Ping", _) => {
                connections.answer(id, pong(answers.liveness()));
                answered += 1;
            }
            ("dcs.bridge.Auth", None) => {
                let auth = Auth::decode(payload(&envelope)).map_err(Close::Payload)?;
                match answers.authenticate(auth.token.as_bytes()) {
                    Ok(opened) => {
                        connections.answer(id, auth_result(Ok(())));
                        // After the answer, on the same channel: the
                        // consumer reads its result before any record.
                        connections.authenticated(id);
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
            (other, Some(_)) => return Err(Close::Unrouted(other.to_owned())),
        }
    }
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

    /// A frame decodes to its `seq` and its topic, and a topic with no
    /// prefix comes back whole.
    #[test]
    fn a_frame_decodes_to_seq_and_topic() {
        let bytes = frame(7, "type.googleapis.com/dcs.bridge.Ping", &[]);
        let mut body = Vec::new();
        let envelope = read_frame(&mut &bytes[..], &mut body).unwrap().unwrap();
        assert_eq!(envelope.seq, 7);
        assert_eq!(topic(&envelope).unwrap(), "dcs.bridge.Ping");

        let bytes = frame(8, "dcs.bridge.Ping", &[]);
        let envelope = read_frame(&mut &bytes[..], &mut body).unwrap().unwrap();
        assert_eq!(topic(&envelope).unwrap(), "dcs.bridge.Ping");

        assert!(matches!(read_frame(&mut &[][..], &mut body), Ok(None)));
    }

    /// A length over the cap is refused before the body is read, a body
    /// that is not an envelope is refused, and so is one with no payload
    /// or a type URL over its cap.
    #[test]
    fn a_bad_frame_is_refused_with_its_reason() {
        let mut body = Vec::new();

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
        let bytes = frame(3, "dcs.bridge.Ping", &[]);
        let envelope = read_frame(&mut Interrupted(&bytes, false), &mut Vec::new())
            .expect("an interrupted read is retried")
            .expect("a frame follows it");
        assert_eq!(envelope.seq, 3);

        let mut bytes = (MAX_FRAME_BYTES + 1).to_le_bytes().to_vec();
        bytes.extend([0u8; 16]);
        assert!(matches!(
            read_frame(&mut &bytes[..], &mut body),
            Err(Close::FrameTooLong(n)) if n == MAX_FRAME_BYTES + 1
        ));
        assert!(body.is_empty(), "a refused length grew the buffer");

        let garbage = [3u8, 0, 0, 0, 0xff, 0xff, 0xff];
        assert!(matches!(
            read_frame(&mut &garbage[..], &mut body),
            Err(Close::Envelope(_))
        ));

        let short = [8u8, 0, 0, 0, 1, 2];
        assert!(matches!(
            read_frame(&mut &short[..], &mut body),
            Err(Close::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof
        ));

        let bare = Envelope {
            seq: 1,
            payload: None,
        };
        assert!(matches!(topic(&bare), Err(Close::NoPayload)));

        let long = Envelope {
            seq: 1,
            payload: Some(Payload {
                type_url: "x".repeat(MAX_TYPE_URL_BYTES + 1),
                value: Vec::new(),
            }),
        };
        assert!(matches!(
            topic(&long),
            Err(Close::TypeUrlTooLong(n)) if n == MAX_TYPE_URL_BYTES + 1
        ));
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
            assert_eq!(any.type_url, "type.googleapis.com/dcs.bridge.AuthResult");
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
            assert_eq!(any.type_url, "type.googleapis.com/dcs.bridge.Pong");
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
