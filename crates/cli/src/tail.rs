//! The `tail` verb: frames off the wire, one line each, and a line for every
//! gap in their numbering.
//!
//! The broker numbers `seq` per connection, from one, before it decides
//! whether a record under pressure stays, so a `seq` that skips means
//! records were dropped and nothing else. That is what this prints, and it
//! is the first thing a person at a live install can see of the drop policy.
//!
//! The record inside a frame is not decoded at all: with no schema loaded,
//! its type URL is all `tail` knows of it, and the type URL is the topic.

use std::io::{self, Read, Write};

use prost::Message;

use crate::wire::{self, Envelope, read_frame};

/// What a run saw, for the closing line and for the tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Frames printed.
    pub frames: u64,
    /// Places where `seq` skipped.
    pub gaps: u64,
    /// Records the skips add up to.
    pub dropped: u64,
    /// The bridge answered the token with an error, and closed after it.
    pub refused: bool,
}

/// `dcs.bridge.Auth`: the token's secret and nothing else.
#[derive(Clone, PartialEq, Message)]
struct Auth {
    #[prost(string, tag = "1")]
    token: String,
}

/// `dcs.bridge.AuthResult`, as the bridge answers an `Auth`.
#[derive(Clone, PartialEq, Message)]
struct AuthResult {
    #[prost(bool, tag = "1")]
    ok: bool,
    #[prost(int32, tag = "2")]
    error: i32,
}

/// The one frame `tail` sends: an `Auth` carrying `secret`, as the first
/// frame on the connection, numbered 1.
///
/// The handshake arrives from the bridge unasked and the result answers
/// this; both print as frames like any other, and the records follow.
pub fn auth_frame(secret: &str) -> Vec<u8> {
    wire::frame(
        1,
        "dcs.bridge.Auth",
        Auth {
            token: secret.to_owned(),
        }
        .encode_to_vec(),
    )
}

/// The name the schema gives an `AuthError` number.
fn auth_error_name(error: i32) -> &'static str {
    match error {
        1 => "BAD_TOKEN",
        2 => "EMPTY_CAPABILITY_SET",
        3 => "SERVER_FULL",
        _ => "UNSPECIFIED",
    }
}

/// Print each frame from `reader` to `out` until the stream ends, with a
/// line before any frame whose `seq` skips past the one before it.
///
/// Each line is flushed as it is written, so a person watching sees a frame
/// when it arrives rather than when a buffer fills.
pub fn run(mut reader: impl Read, mut out: impl Write) -> io::Result<Summary> {
    let mut summary = Summary::default();
    let mut last_seq = 0;

    while let Some(envelope) = read_frame(&mut reader)? {
        if envelope.seq > last_seq + 1 {
            let missing = envelope.seq - last_seq - 1;
            if last_seq == 0 {
                writeln!(
                    out,
                    "gap: {missing} records dropped before seq {}",
                    envelope.seq
                )?;
            } else {
                writeln!(
                    out,
                    "gap: {missing} records dropped between seq {last_seq} and {}",
                    envelope.seq
                )?;
            }
            summary.gaps += 1;
            summary.dropped += missing;
        } else if envelope.seq <= last_seq {
            writeln!(out, "seq {} after {last_seq}: out of order", envelope.seq)?;
        }

        if let Some(result) = auth_result(&envelope) {
            summary.refused |= !result.ok;
        }
        write_frame_line(&mut out, &envelope)?;
        out.flush()?;
        last_seq = envelope.seq;
        summary.frames += 1;
    }

    Ok(summary)
}

/// The `AuthResult` a frame carries, if it is one.
///
/// A result whose bytes do not decode is a refusal: the one frame that
/// says whether the token was accepted did not say so, and a run that went
/// on as though it had would exit as a success.
fn auth_result(envelope: &Envelope) -> Option<AuthResult> {
    if envelope.topic() != Some("dcs.bridge.AuthResult") {
        return None;
    }
    let any = envelope.payload.as_ref()?;
    Some(AuthResult::decode(&any.value[..]).unwrap_or(AuthResult {
        ok: false,
        error: 0,
    }))
}

/// One line per frame: `seq`, the topic and the payload's size, then the
/// epoch and mission time only when the frame carries them. An `AuthResult`
/// says whether the token was accepted, because that is the one record a
/// person watching needs the inside of.
fn write_frame_line(out: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    write!(out, "seq={}", envelope.seq)?;
    match (envelope.topic(), &envelope.payload) {
        (Some(topic), Some(any)) => {
            write!(out, " topic={topic} bytes={}", any.value.len())?;
        }
        _ => write!(out, " topic=- bytes=0")?,
    }
    if let Some(result) = auth_result(envelope) {
        if result.ok {
            write!(out, " ok=true")?;
        } else {
            write!(out, " ok=false error={}", auth_error_name(result.error))?;
        }
    }
    if let Some(epoch) = envelope.epoch {
        write!(out, " epoch={epoch}")?;
    }
    if let Some(mission_time) = envelope.mission_time {
        write!(out, " mission_time={mission_time}")?;
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcsbridge_broker::encode::Encoder;
    use dcsbridge_broker::fanout::{Commit, Writer};
    use dcsbridge_broker::transport::{Listener, Record};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const TOPIC: &str = "dcs.builtin.UnitDestroyed";

    /// A record frame on [`TOPIC`], for a stream built by hand.
    fn frame(seq: u64) -> Vec<u8> {
        wire::frame(seq, TOPIC, vec![0x08, 0x2a])
    }

    /// The gap line's arithmetic: `seq` 1, 2, 5 is two records missing
    /// between 2 and 5, and the frames on either side print as frames.
    #[test]
    fn a_skip_in_seq_prints_the_records_missing() {
        let mut stream = Vec::new();
        for seq in [1, 2, 5] {
            stream.extend(frame(seq));
        }

        let mut out = Vec::new();
        let summary = run(&stream[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert_eq!(
            summary,
            Summary {
                frames: 3,
                gaps: 1,
                dropped: 2,
                refused: false,
            }
        );
        assert_eq!(
            out,
            "seq=1 topic=dcs.builtin.UnitDestroyed bytes=2\n\
             seq=2 topic=dcs.builtin.UnitDestroyed bytes=2\n\
             gap: 2 records dropped between seq 2 and 5\n\
             seq=5 topic=dcs.builtin.UnitDestroyed bytes=2\n"
        );
    }

    /// An `AuthResult` prints whether the token was accepted and, refused,
    /// which error, and a refusal is what the summary says it saw. The
    /// `Auth` frame `tail` sends decodes to the secret it was given.
    #[test]
    fn an_auth_result_prints_its_verdict_and_a_refusal_is_reported() {
        let result = |seq: u64, ok: bool, error: i32| {
            wire::frame(
                seq,
                "dcs.bridge.AuthResult",
                AuthResult { ok, error }.encode_to_vec(),
            )
        };

        let mut out = Vec::new();
        let summary = run(&result(1, true, 0)[..], &mut out).unwrap();
        assert!(!summary.refused);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("topic=dcs.bridge.AuthResult bytes=2 ok=true"),
            "an accepted token did not print ok=true"
        );

        let mut out = Vec::new();
        let summary = run(&result(1, false, 1)[..], &mut out).unwrap();
        assert!(summary.refused, "a refusal was not reported");
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(" ok=false error=BAD_TOKEN"),
            "a refused token did not print its error"
        );

        // A result whose bytes do not decode gave no verdict, which is a
        // refusal rather than a pass.
        let garbled = wire::frame(1, "dcs.bridge.AuthResult", vec![0xff, 0xff, 0xff]);
        let mut out = Vec::new();
        let summary = run(&garbled[..], &mut out).unwrap();
        assert!(summary.refused, "a garbled result passed for a verdict");

        let frame = auth_frame("hunter2");
        let envelope = read_frame(&mut &frame[..]).unwrap().unwrap();
        assert_eq!(envelope.seq, 1);
        assert_eq!(envelope.topic(), Some("dcs.bridge.Auth"));
        let any = envelope.payload.unwrap();
        assert_eq!(Auth::decode(&any.value[..]).unwrap().token, "hunter2");
    }

    /// A record on [`TOPIC`] carrying `bytes` of string in field 1.
    fn record(bytes: usize) -> Record {
        let mut e = Encoder::with_capacity(bytes + 64);
        e.begin(TOPIC.as_bytes());
        e.string(1, &vec![b'x'; bytes]).unwrap();
        Arc::from(e.commit().unwrap())
    }

    /// Connect and authenticate against the bridge's own token table, which
    /// is what lets the fanned-out burst reach this socket. The handshake
    /// and the auth result are the first two frames it reads.
    fn client(addr: SocketAddr) -> TcpStream {
        use dcsbridge_broker::state::{Capability, Token};

        dcsbridge_broker::bridge().set_tokens(vec![Token {
            id: "tail".into(),
            secret: b"tail-secret".to_vec(),
            caps: [Capability::Read].into_iter().collect(),
        }]);
        let mut stream = TcpStream::connect(addr).expect("the listener accepts");
        stream
            .write_all(&auth_frame("tail-secret"))
            .expect("the auth is sent");
        stream
    }

    /// Read one frame's bytes off the socket, whole.
    fn take_frame(client: &mut TcpStream) -> Vec<u8> {
        client
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read timeout is set");
        let mut length = [0u8; 4];
        client
            .read_exact(&mut length)
            .expect("a frame's length arrives");
        let mut bytes = length.to_vec();
        bytes.resize(4 + u32::from_le_bytes(length) as usize, 0);
        client
            .read_exact(&mut bytes[4..])
            .expect("a frame's body arrives");
        bytes
    }

    /// Commit small records until `client` has a frame waiting. A record
    /// committed before the writer thread knows the connection has
    /// authenticated is passed over, and nothing outside that thread says
    /// when that is. The handshake and the auth result have to be off the
    /// socket first, or their arrival is what this returns on.
    fn warm_up(commit: &mut Commit<Record>, client: &TcpStream) {
        let deadline = Instant::now() + Duration::from_secs(30);
        client
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("a read timeout is set");
        let mut length = [0u8; 4];
        loop {
            assert!(Instant::now() < deadline, "no frame arrived");
            drop(commit.push(record(1)));
            if matches!(client.peek(&mut length), Ok(4)) {
                return;
            }
        }
    }

    /// Everything `client` receives until it has been quiet for a while.
    fn drain(client: &mut TcpStream) -> Vec<u8> {
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("a read timeout is set");
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 1 << 16];
        loop {
            match client.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("the read failed: {error}"),
            }
        }
        bytes
    }

    /// The forced drop, observed the way an operator observes it: a consumer
    /// stops reading, its ring evicts behind the blocked socket, and when it
    /// reads again `tail` prints a gap where the evicted records were.
    ///
    /// The ring holds four records and the burst is far larger than the
    /// loopback socket can buffer, so the connection's thread blocks on the
    /// socket while the writer thread pushes the rest of the burst past it.
    #[test]
    fn a_stalled_consumer_sees_its_evictions_as_a_gap() {
        let (writer, mut commit, connections) = Writer::spawn(4096);
        let answers = Arc::new(dcsbridge_broker::state::Global);
        let listener = Listener::spawn("127.0.0.1:0", connections, 4, answers).unwrap();
        let mut client = client(listener.local_addr());
        // The handshake and the auth result, kept so `tail` reads the
        // stream from seq 1; nothing fans out to the connection before the
        // result, so the burst waits for it.
        let mut bytes = take_frame(&mut client);
        bytes.extend(take_frame(&mut client));
        warm_up(&mut commit, &client);

        let big = record(64 << 10);
        for _ in 0..512 {
            drop(commit.push(Arc::clone(&big)));
        }

        bytes.extend(drain(&mut client));
        drop(listener);
        drop(writer);

        let mut out = Vec::new();
        let summary = run(&bytes[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(!summary.refused, "the token was refused:\n{out}");
        assert!(summary.frames >= 3, "no record was read:\n{out}");
        assert!(summary.gaps >= 1, "the stall left no gap:\n{out}");
        assert!(summary.dropped >= 1, "a gap dropped nothing:\n{out}");
        assert!(
            out.contains("gap: ") && out.contains(" records dropped between seq "),
            "the gap was not printed:\n{out}"
        );
        assert!(
            out.contains(&format!(" topic={TOPIC} bytes=")),
            "the topic was not printed without its prefix:\n{out}"
        );
        assert!(!out.contains("out of order"), "seq went backwards:\n{out}");

        let seqs: Vec<u64> = out
            .lines()
            .filter_map(|line| line.strip_prefix("seq="))
            .map(|rest| rest.split(' ').next().unwrap().parse().unwrap())
            .collect();
        assert!(
            seqs.windows(2).all(|pair| pair[0] < pair[1]),
            "seq did not rise strictly:\n{out}"
        );
    }
}
