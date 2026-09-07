//! The `ping` verb: one `Ping` in, one `Pong` out, printed on one line.
//!
//! `Pong` reports the sim's liveness and not the broker's. The broker
//! answers it on the connection's reader thread, which never waits for the
//! logic thread, so a bridge whose logic thread is inside a mission load or
//! wedged still answers, and answers that the sim has not been heard from.
//! That is the case `ping` exists to show. It needs no token: `Ping` is
//! one of the two messages a connection may send before authenticating.

use std::io::{self, Read, Write};

use prost::Message;

use crate::wire::{self, read_frame};

/// `dcsbridge.broker.Pong`, as `proto/dcsbridge/broker/broker.proto` numbers it.
#[derive(Clone, PartialEq, Message)]
pub struct Pong {
    /// Whether the heartbeat's age is under the threshold.
    #[prost(bool, tag = "1")]
    pub dcs_alive: bool,
    /// Milliseconds since the logic thread last stamped the heartbeat, and
    /// absent when it never has.
    #[prost(uint64, optional, tag = "2")]
    pub dcs_last_heard_ms: Option<u64>,
    /// The effective value of the `enabled` key, so a disabled bridge can
    /// be told from a dead sim.
    #[prost(bool, tag = "3")]
    pub bridge_enabled: bool,
}

/// The one frame `ping` sends: an empty `Ping`, numbered 1.
pub fn ping_frame() -> Vec<u8> {
    wire::frame(1, "dcsbridge.broker.Ping", Vec::new())
}

/// Read frames from `reader` until a `Pong` arrives, print its three fields
/// on one line to `out`, and return it. `None` when the stream ends first.
///
/// The handshake arrives before the answer, unasked, and is passed over. A
/// `Pong` whose bytes do not decode is an error rather than a sim reported
/// any way at all.
pub fn run(mut reader: impl Read, mut out: impl Write) -> io::Result<Option<Pong>> {
    while let Some(envelope) = read_frame(&mut reader)? {
        if envelope.topic() != Some("dcsbridge.broker.Pong") {
            continue;
        }
        let any = envelope.payload.expect("a topic came from a payload");
        let pong = Pong::decode(&any.value[..])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write!(out, "dcs_alive={} dcs_last_heard_ms=", pong.dcs_alive)?;
        match pong.dcs_last_heard_ms {
            Some(age) => write!(out, "{age}")?,
            None => write!(out, "-")?,
        }
        writeln!(out, " bridge_enabled={}", pong.bridge_enabled)?;
        out.flush()?;
        return Ok(Some(pong));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcsbridge_broker::fanout::Writer;
    use dcsbridge_broker::transport::Listener;
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::time::Duration;

    /// A `Pong` frame numbered `seq`, as the bridge answers.
    fn pong(seq: u64, pong: Pong) -> Vec<u8> {
        wire::frame(seq, "dcsbridge.broker.Pong", pong.encode_to_vec())
    }

    /// The line prints the three fields, with `-` for a heartbeat that was
    /// never stamped, and the handshake before the answer prints nothing.
    #[test]
    fn a_pong_prints_its_three_fields_after_the_handshake() {
        let mut stream = wire::frame(1, "dcsbridge.broker.Handshake", vec![0x08, 0x01]);
        stream.extend(pong(
            2,
            Pong {
                dcs_alive: false,
                dcs_last_heard_ms: None,
                bridge_enabled: true,
            },
        ));
        let mut out = Vec::new();
        let answer = run(&stream[..], &mut out).unwrap().unwrap();
        assert!(!answer.dcs_alive);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "dcs_alive=false dcs_last_heard_ms=- bridge_enabled=true\n"
        );

        let stream = pong(
            2,
            Pong {
                dcs_alive: true,
                dcs_last_heard_ms: Some(250),
                bridge_enabled: false,
            },
        );
        let mut out = Vec::new();
        let answer = run(&stream[..], &mut out).unwrap().unwrap();
        assert!(answer.dcs_alive);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "dcs_alive=true dcs_last_heard_ms=250 bridge_enabled=false\n"
        );
    }

    /// A stream that ends before a `Pong` is no answer, and a `Pong` whose
    /// bytes do not decode is an error rather than a verdict. The frame
    /// `ping` sends is an empty `Ping` numbered 1.
    #[test]
    fn no_pong_is_no_answer_and_a_garbled_one_is_an_error() {
        let stream = wire::frame(1, "dcsbridge.broker.Handshake", vec![0x08, 0x01]);
        let mut out = Vec::new();
        assert!(run(&stream[..], &mut out).unwrap().is_none());
        assert!(out.is_empty(), "a missing answer printed something");

        let stream = wire::frame(2, "dcsbridge.broker.Pong", vec![0xff, 0xff, 0xff]);
        let mut out = Vec::new();
        let error = run(&stream[..], &mut out).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(out.is_empty(), "a garbled answer printed something");

        let frame = ping_frame();
        let envelope = read_frame(&mut &frame[..]).unwrap().unwrap();
        assert_eq!(envelope.seq, 1);
        assert_eq!(envelope.topic(), Some("dcsbridge.broker.Ping"));
        assert!(envelope.payload.unwrap().value.is_empty());
    }

    /// The answer comes from the reader thread while the logic thread does
    /// nothing: with no heartbeat ever stamped the bridge answers that the
    /// sim was never heard from, one stamp later the same connection's
    /// next `Ping` answers alive with a small age, no token was sent, and
    /// nothing reached the commit ring.
    #[test]
    fn a_live_bridge_answers_without_a_token_or_a_heartbeat() {
        let (writer, commit, connections) = Writer::spawn(64);
        let answers = Arc::new(dcsbridge_broker::state::Global);
        let listener = Listener::spawn("127.0.0.1:0", connections, 4, answers).unwrap();
        let mut client = TcpStream::connect(listener.local_addr()).expect("the listener accepts");
        client
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read timeout is set");

        // The bridge is a process static and the heartbeat is never
        // cleared, so this runs before any other test in the binary can
        // stamp it. Nothing else here does, and neither does `tail`'s.
        client.write_all(&ping_frame()).expect("the ping is sent");
        let mut out = Vec::new();
        let first = run(&mut client, &mut out).unwrap().expect("a pong arrives");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "dcs_alive=false dcs_last_heard_ms=- bridge_enabled=true\n"
        );
        assert_eq!(first.dcs_last_heard_ms, None);

        dcsbridge_broker::bridge().heartbeat();
        client.write_all(&ping_frame()).expect("the ping is sent");
        let mut out = Vec::new();
        let second = run(&mut client, &mut out).unwrap().expect("a pong arrives");
        let out = String::from_utf8(out).unwrap();
        assert!(
            second.dcs_alive,
            "a fresh heartbeat did not read alive:\n{out}"
        );
        assert!(
            second.dcs_last_heard_ms.is_some_and(|age| age < 10_000),
            "the age is not the heartbeat's:\n{out}"
        );
        assert!(
            out.starts_with("dcs_alive=true dcs_last_heard_ms="),
            "{out}"
        );
        assert!(commit.is_empty(), "an answer went through the commit ring");

        drop(listener);
        drop(writer);
    }
}
