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

use crate::wire::{Envelope, auth_error_name, auth_result, handshake_sha256, read_frame};
use dcsbridge_topic as topic;

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

/// The schema hash a handshake carries, as hex. `None` for a handshake
/// carrying none, and for any other frame.
fn schema_sha256(envelope: &Envelope) -> Option<String> {
    let hash = handshake_sha256(envelope)?;
    Some(hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// One line per frame: `seq`, the topic and the payload's size, then the
/// epoch and mission time only when the frame carries them. An `AuthResult`
/// says whether the token was accepted, and the handshake says which schema
/// the bridge serves, because those are the two records a person watching
/// needs the inside of.
fn write_frame_line(out: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    write!(out, "seq={}", envelope.seq)?;
    match (envelope.topic(), &envelope.payload) {
        (Some(topic), Some(any)) => {
            write!(out, " topic={topic} bytes={}", any.value.len())?;
        }
        _ => write!(out, " topic=- bytes=0")?,
    }
    if envelope.topic() == Some(topic::HANDSHAKE) {
        let hash = schema_sha256(envelope).unwrap_or_else(|| "-".into());
        write!(out, " schema_sha256={hash}")?;
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
    use crate::wire::{self, Auth, AuthResult, auth_frame};
    use dcsbridge_broker::encode::Encoder;
    use dcsbridge_broker::fanout::{Commit, Writer};
    use dcsbridge_broker::transport::{Listener, Record};
    use prost::Message;
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const TOPIC: &str = "dcsbridge.builtin.sim.UnitDestroyed";

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
            format!(
                "seq=1 topic={TOPIC} bytes=2\n\
                 seq=2 topic={TOPIC} bytes=2\n\
                 gap: 2 records dropped between seq 2 and 5\n\
                 seq=5 topic={TOPIC} bytes=2\n"
            )
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
                topic::AUTH_RESULT,
                AuthResult { ok, error }.encode_to_vec(),
            )
        };

        let mut out = Vec::new();
        let summary = run(&result(1, true, 0)[..], &mut out).unwrap();
        assert!(!summary.refused);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(&format!("topic={} bytes=2 ok=true", topic::AUTH_RESULT)),
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
        let garbled = wire::frame(1, topic::AUTH_RESULT, vec![0xff, 0xff, 0xff]);
        let mut out = Vec::new();
        let summary = run(&garbled[..], &mut out).unwrap();
        assert!(summary.refused, "a garbled result passed for a verdict");

        let frame = auth_frame("hunter2");
        let envelope = read_frame(&mut &frame[..]).unwrap().unwrap();
        assert_eq!(envelope.seq, 1);
        assert_eq!(envelope.topic(), Some(topic::AUTH));
        let any = envelope.payload.unwrap();
        assert_eq!(Auth::decode(&any.value[..]).unwrap().token, "hunter2");
    }

    /// The handshake line says which schema the bridge serves, as the hash
    /// in hex, and `-` while the bridge holds none. The frame is the
    /// broker's own encoding, so every field it carries is read past.
    #[test]
    fn the_handshake_prints_the_schema_hash_or_a_dash() {
        let handshake = |schema_sha256: Option<[u8; 32]>| {
            let tail = dcsbridge_broker::handshake::Handshake {
                protocol: 1,
                broker: "0.0.0-test",
                instance_id: 7,
                schema_sha256,
            }
            .encode();
            let mut frame = ((tail.len() + 2) as u32).to_le_bytes().to_vec();
            frame.extend([0x08, 0x01]);
            frame.extend_from_slice(&tail);
            frame
        };

        let mut out = Vec::new();
        run(&handshake(None)[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains(&format!("topic={} bytes=", topic::HANDSHAKE))
                && out.contains(" schema_sha256=-\n"),
            "no schema did not print a dash: {out}"
        );

        let mut hash = [0u8; 32];
        hash[0] = 0xba;
        hash[31] = 0x0f;
        let mut out = Vec::new();
        run(&handshake(Some(hash))[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains(
                " schema_sha256=ba0000000000000000000000000000000000000000000000000000000000000f\n"
            ),
            "the hash did not print as hex: {out}"
        );
    }

    /// A record on [`TOPIC`] carrying `bytes` of string in field 1.
    fn record(bytes: usize) -> Record {
        let mut e = Encoder::with_capacity(bytes + 128);
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
