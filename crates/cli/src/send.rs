//! The `send` verb: one record in, on the topic named, after the token.
//!
//! The broker reads a record's type URL and routes by it; what is inside the
//! payload it neither decodes nor checks. So `send` needs the payload's
//! bytes and nothing about their meaning: a file holding them, hex on the
//! command line, or nothing, for a record with no fields. Encoding a record
//! from text through the served schema waits until the schema holds a
//! command to encode; ADR 0025.
//!
//! Nothing answers a record yet. The token's answer is what says the
//! record was read, since the bridge reads frames in order and closes on a
//! refused token before it reaches the record behind it.

use std::io::{self, Read, Write};
use std::path::Path;
use std::{fs, str};

use crate::tail;
use crate::wire::{self, auth_result, read_frame};

/// What a run learned, once it stopped reading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The token was accepted, so the record behind it was read.
    Sent,
    /// The bridge refused the token, with this `AuthError` number, and the
    /// record behind it was never read.
    Refused(i32),
    /// The stream ended before the token was answered.
    Closed,
}

/// The second frame `send` sends, after the token: `value` on `topic`,
/// numbered 2.
///
/// A topic is a fully qualified message name, and the type URL is that
/// name behind one prefix. A user who pastes the URL out of a `tail` line
/// or a log has typed the prefix already, so it is taken off before the
/// frame puts it on, and the URL is never doubled.
pub fn record_frame(topic: &str, value: Vec<u8>) -> Vec<u8> {
    let topic = topic.strip_prefix(wire::TYPE_URL_PREFIX).unwrap_or(topic);
    wire::frame(2, topic, value)
}

/// The payload: the file's bytes whole, the hex decoded, or nothing.
///
/// Hex is two digits per byte, either case, with spaces allowed between
/// bytes so a dump can be pasted as it was printed. An odd digit or a
/// character that is not one names its position, because a payload one
/// nibble off is a record the handler misreads rather than one it refuses.
pub fn payload(file: Option<&Path>, hex: Option<&str>) -> Result<Vec<u8>, String> {
    match (file, hex) {
        (Some(path), _) => {
            fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
        }
        (None, Some(hex)) => decode_hex(hex),
        (None, None) => Ok(Vec::new()),
    }
}

/// `hex` as bytes, spaces between bytes allowed.
fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut high: Option<u8> = None;
    for (at, byte) in hex.bytes().enumerate() {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b' ' if high.is_none() => continue,
            _ => {
                let shown = str::from_utf8(&hex.as_bytes()[at..])
                    .ok()
                    .and_then(|rest| rest.chars().next())
                    .unwrap_or('?');
                return Err(format!(
                    "the hex is not hex at character {}: `{shown}`",
                    at + 1
                ));
            }
        };
        match high.take() {
            Some(high) => bytes.push((high << 4) | nibble),
            None => high = Some(nibble),
        }
    }
    if high.is_some() {
        return Err("the hex has an odd number of digits".into());
    }
    Ok(bytes)
}

/// Read frames from `reader` until the token is answered.
///
/// The handshake arrives first, unasked, and is passed over. An
/// `AuthResult` whose bytes do not decode reads as a refusal, as `tail`
/// reads it: the one frame that says whether the token was accepted did
/// not say so.
pub fn run(mut reader: impl Read) -> io::Result<Outcome> {
    while let Some(envelope) = read_frame(&mut reader)? {
        if let Some(result) = auth_result(&envelope) {
            return Ok(if result.ok {
                Outcome::Sent
            } else {
                Outcome::Refused(result.error)
            });
        }
    }
    Ok(Outcome::Closed)
}

/// Print every frame from `reader` to `out`, as `tail` prints them, until
/// the stream ends or a read runs out of time. Returns how many were
/// printed.
///
/// The time running out is the expected end, since nothing promises an
/// answer: a `Rejected` or a `CommandAck` comes back when something sends
/// one, and the wait is there to show it.
pub fn wait(mut reader: impl Read, mut out: impl Write) -> io::Result<usize> {
    let mut printed = 0;
    loop {
        match read_frame(&mut reader) {
            Ok(Some(envelope)) => {
                tail::write_frame_line(&mut out, &envelope)?;
                out.flush()?;
                printed += 1;
            }
            Ok(None) => return Ok(printed),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(printed);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::AuthResult;
    use dcsbridge_topic as topic;
    use prost::Message;

    /// A command topic: any name, since the wire reads none of them.
    const TOPIC: &str = "dcsbridge.builtin.hook.Kick";

    /// The token's answer, numbered `seq`.
    fn auth_result(seq: u64, ok: bool, error: i32) -> Vec<u8> {
        wire::frame(
            seq,
            topic::AUTH_RESULT,
            AuthResult { ok, error }.encode_to_vec(),
        )
    }

    /// The frame carries the bytes on the topic, numbered 2, and a topic
    /// typed with its URL prefix is not prefixed again.
    #[test]
    fn the_record_frame_is_numbered_two_and_never_doubles_the_prefix() {
        let frame = record_frame(TOPIC, vec![0x08, 0x2a]);
        let envelope = read_frame(&mut &frame[..]).unwrap().unwrap();
        assert_eq!(envelope.seq, 2);
        assert_eq!(envelope.topic(), Some(TOPIC));
        assert_eq!(envelope.payload.unwrap().value, [0x08, 0x2a]);

        let pasted = format!("{}{TOPIC}", wire::TYPE_URL_PREFIX);
        assert_eq!(
            record_frame(&pasted, Vec::new()),
            record_frame(TOPIC, Vec::new())
        );
    }

    /// The payload is the file whole, the hex decoded in either case with
    /// spaces between bytes, or empty; a nibble short, a character that is
    /// not hex and a missing file each say so.
    #[test]
    fn the_payload_comes_from_the_file_the_hex_or_nowhere() {
        let dir = std::env::temp_dir().join(format!("dcsb-send-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("record.bin");
        fs::write(&path, [0x08, 0x2a, 0x00, 0xff]).unwrap();
        assert_eq!(
            payload(Some(&path), None).unwrap(),
            [0x08, 0x2a, 0x00, 0xff]
        );
        assert_eq!(
            payload(Some(&path), Some("ignored")).unwrap(),
            [0x08, 0x2a, 0x00, 0xff]
        );
        let missing = dir.join("missing.bin");
        assert!(
            payload(Some(&missing), None)
                .unwrap_err()
                .starts_with("cannot read ")
        );
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            payload(None, Some("082a00FF")).unwrap(),
            [0x08, 0x2a, 0x00, 0xff]
        );
        assert_eq!(
            payload(None, Some("08 2a 00 ff")).unwrap(),
            [0x08, 0x2a, 0x00, 0xff]
        );
        assert_eq!(payload(None, Some("")).unwrap(), []);
        assert_eq!(payload(None, None).unwrap(), []);
        assert_eq!(
            payload(None, Some("082")).unwrap_err(),
            "the hex has an odd number of digits"
        );
        assert_eq!(
            payload(None, Some("08 2 a")).unwrap_err(),
            "the hex is not hex at character 5: ` `"
        );
        assert_eq!(
            payload(None, Some("08zz")).unwrap_err(),
            "the hex is not hex at character 3: `z`"
        );
    }

    /// An accepted token is the record sent, a refused one stops the read
    /// with the error number, an answer that does not decode is a refusal,
    /// and a stream that ends first is no answer.
    #[test]
    fn the_token_answer_says_whether_the_record_was_read() {
        let mut stream = wire::frame(1, topic::HANDSHAKE, Vec::new());
        stream.extend(auth_result(2, true, 0));
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Sent);

        assert_eq!(
            run(&auth_result(2, false, 1)[..]).unwrap(),
            Outcome::Refused(1)
        );

        let garbage = wire::frame(2, topic::AUTH_RESULT, vec![0xff, 0xff, 0xff]);
        assert_eq!(run(&garbage[..]).unwrap(), Outcome::Refused(0));

        let stream = wire::frame(1, topic::HANDSHAKE, Vec::new());
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Closed);
    }

    /// The wait prints each frame as `tail` does and counts them, and a
    /// stream that ends is the count so far rather than an error.
    #[test]
    fn the_wait_prints_what_comes_back_until_the_stream_ends() {
        let mut stream = wire::frame(3, topic::COMMAND_ACK, vec![0x18, 0x01]);
        stream.extend(wire::frame(4, TOPIC, Vec::new()));
        let mut out = Vec::new();
        assert_eq!(wait(&stream[..], &mut out).unwrap(), 2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "seq=3 topic={} bytes=2\nseq=4 topic={TOPIC} bytes=0\n",
                topic::COMMAND_ACK
            )
        );

        let mut out = Vec::new();
        assert_eq!(wait(&[][..], &mut out).unwrap(), 0);
        assert!(out.is_empty());
    }

    /// Against the shared bridge with one route, the token and the record
    /// go out together, the record is polled from the ring its route names
    /// with the sender's id, the topic and the bytes, the other ring holds
    /// nothing, a topic in no route map reaches neither, a wrong token is
    /// refused and the record behind it never arrives, and the wait ends
    /// on its deadline with nothing printed.
    #[test]
    fn a_record_sent_over_loopback_is_polled_from_its_ring_and_no_other() {
        use dcsbridge_broker::fanout::{ConnectionId, Writer};
        use dcsbridge_broker::registry::{Capability, Target};
        use dcsbridge_broker::state::Token;
        use dcsbridge_broker::transport::Listener;
        use std::io::Write;
        use std::net::TcpStream;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        const UNROUTED: &str = "dcsbridge.builtin.sim.SetFlag";

        // The bridge is a process static shared by every test in the
        // binary. `tail`'s test sets the token table too, in parallel, so
        // both set the same two tokens and neither order loses one.
        // Nothing else starts the inbound rings or registers a route, so
        // what the rings hold is what this test put there.
        let bridge = dcsbridge_broker::bridge();
        bridge.set_tokens(vec![
            Token {
                id: "tail".into(),
                secret: b"tail-secret".to_vec(),
                caps: [Capability::Read].into_iter().collect(),
            },
            Token {
                id: "send".into(),
                secret: b"send-secret".to_vec(),
                caps: [Capability::Command].into_iter().collect(),
            },
        ]);
        bridge.start_inbound(4, 4);
        bridge
            .register_routes([(TOPIC.to_string(), Target::HookDriver)])
            .expect("a new route merges");
        bridge
            .register_caps([(TOPIC.to_string(), Capability::Command)])
            .expect("a new capability merges");

        let (writer, commit, connections) = Writer::spawn(64);
        let answers = Arc::new(dcsbridge_broker::state::Global);
        let listener = Listener::spawn("127.0.0.1:0", connections, 4, answers).unwrap();

        // A `Ping` behind the record, answered after it, is how the reader
        // is known to have got through the record before the ring is read.
        // Without the barrier, what comes back is left for `wait`.
        let send = |secret: &str, topic: &str, value: &[u8], barrier: bool| {
            let mut client =
                TcpStream::connect(listener.local_addr()).expect("the listener accepts");
            let mut request = wire::auth_frame(secret);
            request.extend(record_frame(topic, value.to_vec()));
            if barrier {
                request.extend(wire::frame(3, topic::PING, Vec::new()));
            }
            client.write_all(&request).expect("the request is sent");
            let mut reader = wire::Deadline::new(client, Duration::from_secs(30));
            let outcome = run(&mut reader).unwrap();
            if outcome == Outcome::Sent && barrier {
                loop {
                    let envelope = read_frame(&mut reader)
                        .expect("the pong arrives")
                        .expect("the connection stayed open");
                    if envelope.topic() == Some(topic::PONG) {
                        break;
                    }
                }
            }
            (outcome, reader)
        };

        let (outcome, reader) = send("send-secret", TOPIC, &[0x08, 0x2a], true);
        assert_eq!(outcome, Outcome::Sent);
        let polled = bridge
            .poll(Target::HookDriver)
            .expect("the rings exist")
            .expect("the record is on the hook ring");
        assert_eq!(polled.from, ConnectionId::from_raw(1), "the sender's id");
        assert_eq!(polled.topic, TOPIC, "the topic as registered");
        assert_eq!(polled.value, [0x08, 0x2a], "the payload's bytes");
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(None),
            "the sim ring saw it"
        );
        assert_eq!(
            bridge.poll(Target::HookDriver),
            Ok(None),
            "the hook ring holds a second"
        );

        // Nothing answers the record, so the wait spends its deadline and
        // prints nothing.
        let started = Instant::now();
        let mut out = Vec::new();
        let reader = reader.again(Duration::from_millis(200));
        assert_eq!(wait(reader, &mut out).unwrap(), 0);
        assert!(
            out.is_empty(),
            "something answered: {}",
            String::from_utf8_lossy(&out)
        );
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "the wait gave up early"
        );

        // An unrouted topic is answered with a `Rejected` naming the record
        // by the number `send` gave it, which is what the wait prints.
        let (outcome, reader) = send("send-secret", UNROUTED, &[], false);
        assert_eq!(outcome, Outcome::Sent);
        let mut out = Vec::new();
        let reader = reader.again(Duration::from_secs(2));
        assert_eq!(wait(reader, &mut out).unwrap(), 1);
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with(&format!("seq=3 topic={} bytes=", topic::REJECTED))
                && out.ends_with(&format!(
                    " rejected_seq=2 rejected_topic={UNROUTED} reason=UNKNOWN_TOPIC\n"
                )),
            "the wait did not print the refusal: {out}"
        );
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(None),
            "an unrouted topic reached the sim ring"
        );
        assert_eq!(
            bridge.poll(Target::HookDriver),
            Ok(None),
            "an unrouted topic reached the hook ring"
        );
        assert_eq!(bridge.unrouted_topic(), 1);

        let (outcome, _reader) = send("wrong", TOPIC, &[0x08, 0x2a], true);
        assert_eq!(outcome, Outcome::Refused(1));
        assert_eq!(
            bridge.poll(Target::HookDriver),
            Ok(None),
            "a refused token's record arrived"
        );

        assert!(commit.is_empty(), "an answer went through the commit ring");
        drop(listener);
        drop(writer);
    }
}
