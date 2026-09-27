//! What a verb's loopback test stands up: a record for the broker's rings,
//! a consumer that authenticates against the bridge's own token table, and
//! the reads that take frames off its socket. Every verb that reads a fan-out
//! stream tests against the same listener, so the helpers live once.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::wire::auth_frame;
use dcsbridge_broker::encode::Encoder;
use dcsbridge_broker::fanout::{Class, Commit};
use dcsbridge_broker::transport::Record;

/// A fan-out topic: any record, since the wire reads none of them.
pub(crate) const TOPIC: &str = "dcsbridge.builtin.sim.UnitDestroyed";

/// The capability number most records committed here need, which the
/// `tail` token grants.
pub(crate) const READ: u32 = dcsbridge_broker::registry::Capability::Read as u32;

/// A record on [`TOPIC`] carrying `bytes` of string in field 1.
pub(crate) fn record(bytes: usize) -> Record {
    record_on(TOPIC, bytes)
}

/// A record on `topic` carrying `bytes` of string in field 1.
pub(crate) fn record_on(topic: &str, bytes: usize) -> Record {
    let mut e = Encoder::with_capacity(bytes + 128);
    e.begin(topic.as_bytes(), None);
    e.string(1, &vec![b'x'; bytes]).unwrap();
    Arc::from(e.commit().unwrap())
}

/// Connect and authenticate against the bridge's own token table, which
/// is what lets the fanned-out burst reach this socket. The handshake
/// and the auth result are the first two frames it reads.
pub(crate) fn client(addr: SocketAddr) -> TcpStream {
    use dcsbridge_broker::registry::Capability;
    use dcsbridge_broker::state::Token;

    // The table is the bridge's one table and `send`'s test sets it
    // too, in parallel, so both set the same two tokens and neither
    // order loses one.
    dcsbridge_broker::bridge().set_tokens(vec![
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
    let mut stream = TcpStream::connect(addr).expect("the listener accepts");
    stream
        .write_all(&auth_frame("tail-secret"))
        .expect("the auth is sent");
    stream
}

/// Read one frame's bytes off the socket, whole.
pub(crate) fn take_frame(client: &mut TcpStream) -> Vec<u8> {
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
pub(crate) fn warm_up(commit: &mut Commit<Record>, client: &TcpStream) {
    let deadline = Instant::now() + Duration::from_secs(30);
    client
        .set_read_timeout(Some(Duration::from_millis(20)))
        .expect("a read timeout is set");
    let mut length = [0u8; 4];
    loop {
        assert!(Instant::now() < deadline, "no frame arrived");
        drop(commit.push(READ, Class::Durable, record(1)));
        if matches!(client.peek(&mut length), Ok(4)) {
            return;
        }
    }
}

/// Everything `client` receives until it has been quiet for a while.
pub(crate) fn drain(client: &mut TcpStream) -> Vec<u8> {
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
