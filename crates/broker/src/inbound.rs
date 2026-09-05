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
//! [`HANDSHAKE_TIMEOUT`]. The thread that reads a socket and applies that
//! order is the transport's to start, and nothing starts it yet: what is
//! here is the frame, the answers, and what the reader asks of the broker.

use std::io::{self, Read};
use std::time::Duration;

use prost::Message;

use crate::encode::{Encoder, TYPE_URL_PREFIX};
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
