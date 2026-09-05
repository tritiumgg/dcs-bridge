//! The listener, and the thread each consumer connection gets.
//!
//! DCS listens and consumers connect. The listener's thread accepts a socket,
//! attaches a ring for it to the writer thread, and starts a thread that does
//! nothing but drain that ring into the socket. A write blocks, so a consumer
//! that stops reading stalls its own thread and no other: the writer keeps
//! pushing past it, its ring evicts and counts, and every other connection
//! carries on. ADR 0011.
//!
//! A frame is a little-endian `u32` length and then one `Envelope`. The
//! connection's thread writes the envelope in two pieces: `seq`, which is
//! this connection's and comes from the number the writer thread assigned,
//! and the tail the record was committed as, which every connection shares by
//! reference and none copies. ADR 0014.
//!
//! A connection's first frame is the handshake, at `seq` 1. The listener
//! thread asks for it at accept and hands it to the writer thread inside
//! the message that attaches the connection, so the writer numbers it as it
//! attaches the ring and nothing fanned out can come first.
//!
//! A connection gets a second thread that reads its socket, because a read
//! blocks the way a write does and neither may wait on the other. What it
//! reads, and what it answers, is the inbound module's; whichever of the
//! two threads returns first shuts the socket down, which returns the other.

use std::io::{self, IoSlice, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use crate::encode::{varint_len, write_varint};
use crate::fanout::{ConnectionId, Connections, LOOKS_BEFORE_PARK, Numbered, ParkFlag, Waker};
use crate::inbound::{self, Answers};
use crate::ring::Consumer;

/// A committed record as the rings carry it: the envelope tail, shared by
/// every connection it is fanned out to.
///
/// One allocation per record, at `commit`, where the tail is copied out of
/// the encoder's buffer. A slab that hands the allocation back is a later
/// task's. ADR 0014.
pub type Record = Arc<[u8]>;

/// `Envelope.seq`'s tag: field 1, varint.
const SEQ_TAG: u8 = 0x08;

/// The most bytes a frame's header takes: the length, the `seq` tag, and a
/// ten-byte varint.
const HEADER_MAX: usize = 4 + 1 + 10;

/// The listening socket and its thread. Dropping it closes every connection
/// and waits for their threads.
pub struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    accepting: Option<thread::JoinHandle<()>>,
    open: Arc<Mutex<Vec<Open>>>,
}

/// What the listener keeps about a connection it accepted, so that dropping
/// the listener can close it and wake its thread.
struct Open {
    stream: TcpStream,
    thread: thread::Thread,
}

impl Listener {
    /// Bind `addr` and start accepting.
    ///
    /// Each connection accepted is attached to `connections` with a ring of
    /// `ring_capacity` records, sent the handshake `answers` gives as its
    /// first record, and given two threads: one to drain the ring and one
    /// to read the socket, which answers through `answers`. The handshake is
    /// asked for per connection, on the listener thread, because what it
    /// carries can change between two accepts: the schema hash arrives after
    /// the listener is up. The bind is the one thing here that fails, and it
    /// fails before any thread starts.
    ///
    /// # Panics
    ///
    /// If `ring_capacity` is zero, or if a thread cannot be spawned.
    pub fn spawn(
        addr: impl ToSocketAddrs,
        connections: Connections<Record>,
        ring_capacity: usize,
        answers: Arc<dyn Answers>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let open = Arc::new(Mutex::new(Vec::new()));

        let accepting = {
            let stop = Arc::clone(&stop);
            let open = Arc::clone(&open);
            thread::Builder::new()
                .name("dcsbridge-listener".into())
                .spawn(move || {
                    accept_loop(listener, connections, ring_capacity, answers, stop, open);
                })
                .expect("the listener thread spawns")
        };

        Ok(Self {
            addr,
            stop,
            accepting: Some(accepting),
            open,
        })
    }

    /// The address bound, with the port the system chose if the caller asked
    /// for port zero.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Listener {
    /// Stop accepting, close every connection, and wait for their threads.
    ///
    /// `accept` has no way to be interrupted, so a connection to the listener
    /// itself is what returns it. A join fails only if the thread panicked,
    /// and that panic has already been reported where it happened.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for Open { stream, thread } in self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let _ = stream.shutdown(Shutdown::Both);
            thread.unpark();
        }
        drop(TcpStream::connect(self.addr));
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

/// The listener thread's loop: accept, attach, and start a thread per
/// connection. Joins every connection thread on the way out.
fn accept_loop(
    listener: TcpListener,
    connections: Connections<Record>,
    ring_capacity: usize,
    answers: Arc<dyn Answers>,
    stop: Arc<AtomicBool>,
    open: Arc<Mutex<Vec<Open>>>,
) {
    let mut serving = Vec::new();

    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else {
            // A failed accept is the peer's, not the listener's: the socket
            // it was for is gone and the next one is unaffected.
            continue;
        };
        // Each record is its own frame and a consumer wants it now, so
        // nothing waits for a fuller segment. A socket this cannot be set on
        // still carries frames.
        let _ = stream.set_nodelay(true);

        // Two threads on one socket, one reading and one writing, each with
        // its own handle to it; the original stays with the listener so
        // that dropping it can close the socket under both.
        let (Ok(mut writing), Ok(reading)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };

        // The draining thread parks on its ring, and the writer thread
        // wakes it, so the thread has to exist before the ring is attached:
        // it starts by waiting for the ring, which arrives once the writer
        // thread knows whom to wake.
        let flag = Arc::new(ParkFlag::new());
        // Raised by the reader when it has closed the socket, so the
        // drainer returns from an empty ring rather than parking on it.
        let closing = Arc::new(AtomicBool::new(false));
        // Frames the drainer has put on the socket, for the reader: before
        // authentication every one of them is an answer, and the reader
        // counts them to know a failed result is out before it closes.
        let written = Arc::new(AtomicU64::new(0));
        let (hand, take) = std::sync::mpsc::channel();
        let handle = {
            let flag = Arc::clone(&flag);
            let stop = Arc::clone(&stop);
            let connections = connections.clone();
            let closing = Arc::clone(&closing);
            let written = Arc::clone(&written);
            thread::Builder::new()
                .name("dcsbridge-conn".into())
                .spawn(move || {
                    let Ok((id, consumer)) = take.recv() else {
                        return;
                    };
                    let _ = serve(&mut writing, consumer, &flag, &stop, &closing, &written);
                    // A socket that refused a write is done in both
                    // directions; the shutdown returns the reader.
                    let _ = writing.shutdown(Shutdown::Both);
                    connections.detach(id);
                })
                .expect("a connection thread spawns")
        };

        let waker = Waker::new(Arc::clone(&flag), handle.thread().clone());
        // The handshake rides the attach, so the writer thread numbers it 1
        // as it attaches the ring and nothing fanned out can come first.
        let attached: (ConnectionId, Consumer<Numbered<Record>>) =
            connections.attach_with(ring_capacity, waker, Some(answers.handshake()));
        let id = attached.0;

        // The reader: a second thread on the same socket, because a read
        // blocks the way a write does and neither may wait on the other. It
        // shuts the socket down when it returns, which returns a drainer
        // blocked in a write; a drainer parked on an empty ring is woken
        // through its flag, since no socket event reaches a parked thread.
        let reader = {
            let connections = connections.clone();
            let answers = Arc::clone(&answers);
            let closing = Arc::clone(&closing);
            let drainer = Waker::new(flag, handle.thread().clone());
            thread::Builder::new()
                .name("dcsbridge-read".into())
                .spawn(move || {
                    inbound::run(reading, id, connections, &*answers, &written);
                    closing.store(true, Ordering::SeqCst);
                    drainer.wake();
                })
                .expect("a reader thread spawns")
        };

        open.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Open {
                stream,
                thread: handle.thread().clone(),
            });
        // A send fails only if the thread is gone, and it cannot be: it
        // returns only after receiving.
        let _ = hand.send(attached);
        serving.push(handle);
        serving.push(reader);
    }

    for handle in serving {
        let _ = handle.join();
    }
}

/// A connection thread's loop: drain the ring into the socket, and sleep on
/// the ring when it is empty, the way the writer thread sleeps on the commit
/// ring. Returns when the socket refuses a write, the reader has closed it,
/// or the listener stops.
fn serve(
    stream: &mut TcpStream,
    mut consumer: Consumer<Numbered<Record>>,
    flag: &ParkFlag,
    stop: &AtomicBool,
    closing: &AtomicBool,
    written: &AtomicU64,
) -> io::Result<()> {
    let mut empty_passes = 0;

    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        // `closing` is the reader's: it has shut the socket down, and
        // nothing in the ring is going anywhere. Checked once the ring is
        // empty, so what was queued before the close is still written.
        match consumer.pop() {
            Some(record) => {
                write_frame(stream, &record)?;
                written.fetch_add(1, Ordering::Release);
                empty_passes = 0;
            }
            None if closing.load(Ordering::SeqCst) => return Ok(()),
            None if empty_passes < LOOKS_BEFORE_PARK => {
                empty_passes += 1;
                thread::yield_now();
            }
            None => {
                empty_passes = 0;
                flag.park_unless(|| {
                    !consumer.is_empty()
                        || stop.load(Ordering::SeqCst)
                        || closing.load(Ordering::SeqCst)
                });
            }
        }
    }
}

/// Write one frame: the length, then `seq` as the envelope's first field,
/// then the shared tail.
///
/// Two pieces, because the tail is shared and the header is this
/// connection's, and one system call, because the call is what a frame
/// costs: a live burst on Windows loopback drained one frame per two calls
/// at tens of microseconds each, and nothing downstream was slower. The
/// header is built on the stack and nothing is copied to join them.
fn write_frame(stream: &mut TcpStream, record: &Numbered<Record>) -> io::Result<()> {
    let tail = &record.record;
    let length = u32::try_from(1 + varint_len(record.seq) + tail.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the frame outgrew its length"))?;

    let mut header = [0u8; HEADER_MAX];
    header[..4].copy_from_slice(&length.to_le_bytes());
    header[4] = SEQ_TAG;
    let end = 5 + write_varint(&mut header[5..], record.seq);

    write_all_vectored(stream, &header[..end], tail)
}

/// Write `first` then `second`, handing the socket both at once and finishing
/// whatever a short write left.
///
/// A socket may take fewer bytes than offered, so the loop advances past what
/// went and offers the rest; `advance_slices` drops a slice once it is
/// wholly written, so the second call carries only what remains.
fn write_all_vectored(stream: &mut TcpStream, first: &[u8], second: &[u8]) -> io::Result<()> {
    let mut bufs = [IoSlice::new(first), IoSlice::new(second)];
    let mut pending: &mut [IoSlice<'_>] = &mut bufs;
    while !pending.is_empty() {
        match stream.write_vectored(pending) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "the socket took none of the frame",
                ));
            }
            Ok(written) => IoSlice::advance_slices(&mut pending, written),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{Encoder, TYPE_URL_PREFIX};
    use crate::fanout::Writer;
    use crate::inbound::{AuthError, Session};
    use prost::Message;
    use std::io::Read;
    use std::sync::RwLock;
    use std::time::{Duration, Instant};

    /// `dcs.bridge.Envelope` as a consumer with no record schema reads it:
    /// `seq` and an `Any` whose type URL is all it knows of the payload.
    #[derive(Clone, PartialEq, Message)]
    struct Envelope {
        #[prost(uint64, tag = "1")]
        seq: u64,
        #[prost(message, optional, tag = "4")]
        payload: Option<prost_types::Any>,
    }

    const TOPIC: &[u8] = b"dcs.builtin.UnitDestroyed";

    /// A record on [`TOPIC`] carrying `n` in field 1.
    fn record(n: i64) -> Record {
        let mut e = Encoder::with_capacity(256);
        e.begin(TOPIC);
        e.integer(1, n).unwrap();
        Arc::from(e.commit().unwrap())
    }

    /// Read one frame, or fail after a while rather than hang the suite.
    fn read_frame(stream: &mut TcpStream) -> Envelope {
        let mut length = [0u8; 4];
        stream
            .read_exact(&mut length)
            .expect("a frame's length arrives");
        let mut body = vec![0u8; u32::from_le_bytes(length) as usize];
        stream
            .read_exact(&mut body)
            .expect("a frame's body arrives");
        Envelope::decode(&body[..]).expect("a stock decoder reads the envelope")
    }

    fn client(addr: SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(addr).expect("the listener accepts");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read timeout is set");
        stream
    }

    /// What stands behind a test listener: a fixed handshake and a sim that
    /// was heard from a moment ago.
    struct Stub;

    impl Answers for Stub {
        fn handshake(&self) -> Record {
            crate::handshake::Handshake {
                protocol: crate::PROTOCOL_VERSION,
                broker: crate::BROKER_VERSION,
                instance_id: 42,
                schema_sha256: None,
            }
            .encode()
        }

        fn liveness(&self) -> inbound::Liveness {
            inbound::Liveness {
                last_heard_ms: Some(12),
                alive: true,
                enabled: true,
            }
        }

        /// One token, `SECRET`, with every capability.
        fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
            if secret == SECRET {
                Ok(Session {
                    token_id: "stub".into(),
                    caps: [
                        crate::state::Capability::Read,
                        crate::state::Capability::Command,
                        crate::state::Capability::Reload,
                    ]
                    .into_iter()
                    .collect(),
                })
            } else {
                Err(AuthError::BadToken)
            }
        }

        fn disconnected(&self, _: &Session) {}

        fn schema(&self) -> Option<Record> {
            None
        }

        fn seq_ack(&self, _: u64) {}

        fn set_enabled(&self, _: bool) {}

        fn refused_no_capability(&self, _: &str) {}
    }

    const SECRET: &[u8] = b"open-sesame";

    /// The stub with the switch and the counters the commands land in, and
    /// a token that carries `reload` or does not.
    struct Switch {
        reload: bool,
        enabled: Arc<AtomicBool>,
        acked: Arc<AtomicU64>,
        refused: Arc<AtomicU64>,
    }

    impl Answers for Switch {
        fn handshake(&self) -> Record {
            Stub.handshake()
        }
        fn liveness(&self) -> inbound::Liveness {
            inbound::Liveness {
                enabled: self.enabled.load(Ordering::SeqCst),
                ..Stub.liveness()
            }
        }
        fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
            let session = Stub.authenticate(secret)?;
            Ok(if self.reload {
                session
            } else {
                Session {
                    caps: [crate::state::Capability::Read].into_iter().collect(),
                    ..session
                }
            })
        }
        fn disconnected(&self, _: &Session) {}
        fn schema(&self) -> Option<Record> {
            None
        }
        fn seq_ack(&self, seq: u64) {
            self.acked.store(seq, Ordering::SeqCst);
        }
        fn set_enabled(&self, enabled: bool) {
            self.enabled.store(enabled, Ordering::SeqCst);
        }
        fn refused_no_capability(&self, _: &str) {
            self.refused.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn listener(connections: crate::fanout::Connections<Record>) -> Listener {
        Listener::spawn("127.0.0.1:0", connections, 64, Arc::new(Stub)).unwrap()
    }

    /// `dcs.bridge.AuthResult` as a consumer decodes it.
    #[derive(Clone, PartialEq, Message)]
    struct AuthResult {
        #[prost(bool, tag = "1")]
        ok: bool,
        #[prost(int32, tag = "2")]
        error: i32,
    }

    /// Send an `Auth` carrying `secret` and read the result, which is the
    /// next frame: nothing fans out to an unauthenticated connection, so
    /// nothing can come between.
    fn authenticate(client: &mut TcpStream, secret: &[u8]) -> (Envelope, AuthResult) {
        let auth = inbound::Auth {
            token: String::from_utf8(secret.to_vec()).unwrap(),
        }
        .encode_to_vec();
        client
            .write_all(&inbound(1, "dcs.bridge.Auth", &auth))
            .expect("the auth is sent");
        let frame = read_frame(client);
        let any = frame.payload.as_ref().expect("a payload");
        assert_eq!(any.type_url, "type.googleapis.com/dcs.bridge.AuthResult");
        let result = AuthResult::decode(&any.value[..]).expect("the result decodes");
        (frame, result)
    }

    /// Read the handshake, which is the first frame and numbered 1.
    fn read_handshake(client: &mut TcpStream) -> Envelope {
        let frame = read_frame(client);
        assert_eq!(frame.seq, 1, "the handshake was not frame one");
        let mut url = TYPE_URL_PREFIX.to_vec();
        url.extend_from_slice(crate::handshake::TOPIC);
        assert_eq!(
            frame.payload.as_ref().unwrap().type_url.as_bytes(),
            url,
            "the first frame was not the handshake"
        );
        frame
    }

    /// Read the handshake and authenticate, then commit numbered records, a
    /// few at a time, until `client` has read its first record. The
    /// `Authenticated` control is queued behind the result, so once the
    /// result has arrived every later commit reaches this connection; the
    /// loop is for a commit that raced it. Returns the first record frame
    /// and the number it carried.
    fn first_frame(commit: &mut crate::fanout::Commit<Record>, client: &mut TcpStream) -> Envelope {
        read_handshake(client);
        let (result, decoded) = authenticate(client, SECRET);
        assert_eq!(
            result.seq, 2,
            "the auth result did not follow the handshake"
        );
        assert!(decoded.ok, "the stub refused its own secret");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut n = 0;
        client
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("a read timeout is set");
        loop {
            assert!(Instant::now() < deadline, "no frame arrived");
            commit.push(record(n));
            n += 1;
            let mut length = [0u8; 4];
            if stream_peek(client, &mut length) {
                client
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .expect("a read timeout is set");
                return read_frame(client);
            }
        }
    }

    /// Whether a frame's length is waiting, without consuming it.
    fn stream_peek(client: &mut TcpStream, buf: &mut [u8; 4]) -> bool {
        matches!(client.peek(buf), Ok(4))
    }

    /// The value in field 1 of a frame's payload.
    fn value(envelope: &Envelope) -> i64 {
        #[derive(Clone, PartialEq, Message)]
        struct One {
            #[prost(int64, tag = "1")]
            n: i64,
        }
        let any = envelope.payload.as_ref().expect("a payload");
        One::decode(&any.value[..]).expect("the record decodes").n
    }

    /// A capture names its record types with no schema loaded: the frame
    /// decodes as an envelope, `seq` counts from one with the handshake
    /// first, and the type URL carries the topic.
    #[test]
    fn frames_carry_seq_and_name_the_topic() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);
        let mut client = client(listener.local_addr());

        let first = first_frame(&mut commit, &mut client);
        assert_eq!(
            first.seq, 3,
            "the first record did not follow the handshake and the auth result"
        );
        let mut url = TYPE_URL_PREFIX.to_vec();
        url.extend_from_slice(TOPIC);
        assert_eq!(first.payload.as_ref().unwrap().type_url.as_bytes(), url);

        let start = value(&first);
        for n in 1..=2 {
            commit.push(record(start + 100 + n));
        }
        // Whatever the warm-up committed after the first frame arrives before
        // these two; skip to them.
        let mut seq = first.seq;
        loop {
            let frame = read_frame(&mut client);
            assert_eq!(frame.seq, seq + 1, "seq skipped or repeated");
            seq = frame.seq;
            if value(&frame) == start + 101 {
                break;
            }
        }
        let frame = read_frame(&mut client);
        assert_eq!(frame.seq, seq + 1);
        assert_eq!(value(&frame), start + 102);

        drop(listener);
        drop(writer);
    }

    /// Two connections are two streams: each numbers from one, its handshake
    /// first and its first record second, and one closing leaves the other
    /// receiving.
    #[test]
    fn each_connection_numbers_from_one_and_closes_alone() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);

        let mut first = client(listener.local_addr());
        assert_eq!(first_frame(&mut commit, &mut first).seq, 3);

        let mut second = client(listener.local_addr());
        let on_second = first_frame(&mut commit, &mut second);
        assert_eq!(
            on_second.seq, 3,
            "the second connection did not start at one"
        );

        drop(first);
        let last = value(&on_second);
        commit.push(record(last + 1000));
        loop {
            let frame = read_frame(&mut second);
            if value(&frame) == last + 1000 {
                break;
            }
        }

        drop(listener);
        drop(writer);
    }

    /// A record addressed to one connection arrives there as its next frame,
    /// and the other connection's next frame is the fan-out record after it
    /// with no gap: a reply to one consumer is not missing data at another.
    ///
    /// The writer numbers connections from one in attach order, and the
    /// listener attaches in accept order, so the first client is connection
    /// one. Its first frame is read before the second connects, which is
    /// what fixes the order.
    #[test]
    fn an_addressed_record_reaches_one_connection_and_leaves_no_gap_on_the_other() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);

        let mut first = client(listener.local_addr());
        let on_first = first_frame(&mut commit, &mut first);
        let mut second = client(listener.local_addr());
        let on_second = first_frame(&mut commit, &mut second);

        commit.push_to(ConnectionId::from_raw(1), record(7_001));
        commit.push(record(7_002));

        // Whatever the warm-ups committed after each first frame arrives
        // before these; read through it, and every frame on the way numbers
        // one past the last.
        let read_until =
            |client: &mut TcpStream, mut seq: u64, want: i64| -> (Envelope, Vec<i64>) {
                let mut passed = Vec::new();
                loop {
                    let frame = read_frame(client);
                    assert_eq!(frame.seq, seq + 1, "seq skipped or repeated");
                    seq = frame.seq;
                    if value(&frame) == want {
                        return (frame, passed);
                    }
                    passed.push(value(&frame));
                }
            };

        let (addressed, _) = read_until(&mut first, on_first.seq, 7_001);
        let (fanned, _) = read_until(&mut first, addressed.seq, 7_002);
        assert_eq!(
            fanned.seq,
            addressed.seq + 1,
            "the fan-out record did not follow the addressed one"
        );

        let (_, passed) = read_until(&mut second, on_second.seq, 7_002);
        assert!(
            !passed.contains(&7_001),
            "the other connection received the addressed record"
        );

        drop(listener);
        drop(writer);
    }

    /// Dropping the listener returns its thread and every connection's, and
    /// a client that was connected reads end of stream.
    #[test]
    fn dropping_the_listener_closes_every_connection() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);
        let mut idle = client(listener.local_addr());
        let mut busy = client(listener.local_addr());
        first_frame(&mut commit, &mut busy);

        drop(listener);

        // The idle client received the warm-up's frames too; past them it
        // reads end of stream, or a reset, and not a timeout.
        let mut sink = [0u8; 4096];
        loop {
            match idle.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    panic!("the idle client was not closed")
                }
                Err(_) => break,
            }
        }
        drop(writer);
    }

    /// A frame as a consumer's stock encoder writes it: the length, then an
    /// envelope carrying `topic` and `value`.
    fn inbound(seq: u64, topic: &str, value: &[u8]) -> Vec<u8> {
        let body = inbound::Envelope {
            seq,
            payload: Some(inbound::Payload {
                type_url: format!("type.googleapis.com/{topic}"),
                value: value.to_vec(),
            }),
        }
        .encode_to_vec();
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    /// Whether `client` reads end of stream, or a reset, before a timeout.
    /// A socket that refuses the timeout has already been reset under a
    /// write, which is closed too.
    fn is_closed(client: &mut TcpStream) -> bool {
        if client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .is_err()
        {
            return true;
        }
        let mut sink = [0u8; 4096];
        loop {
            match client.read(&mut sink) {
                Ok(0) => return true,
                Ok(_) => continue,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return false;
                }
                Err(_) => return true,
            }
        }
    }

    /// `dcs.bridge.Pong` as a consumer decodes it.
    #[derive(Clone, PartialEq, Message)]
    struct Pong {
        #[prost(bool, tag = "1")]
        dcs_alive: bool,
        #[prost(uint64, optional, tag = "2")]
        dcs_last_heard_ms: Option<u64>,
        #[prost(bool, tag = "3")]
        bridge_enabled: bool,
    }

    /// The three commands the broker handles itself, after authentication:
    /// `GetSchema` is answered with the error while there is no schema,
    /// `SeqAck` is consumed and answered by nothing, `SetEnabled` flips the
    /// switch for a token with `reload` and is counted for one without, and
    /// `Pong` shows the switch. None of them reaches the commit ring, and
    /// before authentication each closes the connection.
    #[test]
    fn the_three_commands_are_handled_on_the_reader_and_reach_no_ring() {
        #[derive(Clone, PartialEq, Message)]
        struct Schema {
            #[prost(bytes = "vec", optional, tag = "1")]
            file_descriptor_set: Option<Vec<u8>>,
            #[prost(string, optional, tag = "2")]
            error: Option<String>,
        }
        let pong = |client: &mut TcpStream| -> Pong {
            client
                .write_all(&inbound(1, "dcs.bridge.Ping", &[]))
                .expect("the ping is sent");
            let frame = read_frame(client);
            Pong::decode(&frame.payload.unwrap().value[..]).expect("the pong decodes")
        };

        let enabled = Arc::new(AtomicBool::new(true));
        let acked = Arc::new(AtomicU64::new(0));
        let refused = Arc::new(AtomicU64::new(0));
        let switch = |reload: bool| {
            Arc::new(Switch {
                reload,
                enabled: Arc::clone(&enabled),
                acked: Arc::clone(&acked),
                refused: Arc::clone(&refused),
            })
        };

        let (writer, commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64, switch(true)).unwrap();

        for early in [
            "dcs.bridge.GetSchema",
            "dcs.bridge.SeqAck",
            "dcs.bridge.SetEnabled",
        ] {
            let mut offender = client(listener.local_addr());
            read_handshake(&mut offender);
            offender
                .write_all(&inbound(1, early, &[]))
                .expect("the frame is sent");
            assert!(is_closed(&mut offender), "{early} was accepted before auth");
        }

        let mut admin = client(listener.local_addr());
        read_handshake(&mut admin);
        assert!(authenticate(&mut admin, SECRET).1.ok);

        admin
            .write_all(&inbound(2, "dcs.bridge.GetSchema", &[]))
            .expect("the request is sent");
        let frame = read_frame(&mut admin);
        let any = frame.payload.expect("a payload");
        assert_eq!(any.type_url, "type.googleapis.com/dcs.bridge.Schema");
        assert_eq!(
            Schema::decode(&any.value[..]).expect("the schema decodes"),
            Schema {
                file_descriptor_set: None,
                error: Some(inbound::NO_SCHEMA.into()),
            }
        );

        let ack = inbound::SeqAck { seq: 41 }.encode_to_vec();
        admin
            .write_all(&inbound(3, "dcs.bridge.SeqAck", &ack))
            .expect("the ack is sent");
        let off = inbound::SetEnabled { enabled: false }.encode_to_vec();
        admin
            .write_all(&inbound(4, "dcs.bridge.SetEnabled", &off))
            .expect("the switch is sent");
        // The pong follows both on the reader thread, so its answer
        // shows their effect.
        assert!(!pong(&mut admin).bridge_enabled, "the switch did not flip");
        assert_eq!(acked.load(Ordering::SeqCst), 41);

        assert!(commit.is_empty(), "a command reached the commit ring");
        assert_eq!(commit.dropped(), 0);
        drop(listener);
        drop(writer);

        // The second half starts from its own state rather than what the
        // first left: the switch is on, so an applied `SetEnabled` would
        // have to turn it off to show.
        enabled.store(true, Ordering::SeqCst);
        let (writer, _commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64, switch(false)).unwrap();
        let mut reader = client(listener.local_addr());
        read_handshake(&mut reader);
        assert!(authenticate(&mut reader, SECRET).1.ok);
        let off = inbound::SetEnabled { enabled: false }.encode_to_vec();
        reader
            .write_all(&inbound(2, "dcs.bridge.SetEnabled", &off))
            .expect("the switch is sent");
        assert!(
            pong(&mut reader).bridge_enabled,
            "a token without reload flipped the switch"
        );
        assert_eq!(refused.load(Ordering::SeqCst), 1);

        drop(listener);
        drop(writer);
    }

    /// A `Ping` is answered with a `Pong` while the logic thread commits
    /// nothing and the writer thread is parked, the answer is numbered
    /// after the handshake, and nothing reaches the commit ring: the
    /// answer never touched the logic thread's side.
    #[test]
    fn a_ping_is_answered_with_the_writer_parked_and_no_commit() {
        let (writer, commit, connections) = Writer::spawn(64);
        let listener = listener(connections);
        let mut client = client(listener.local_addr());
        read_handshake(&mut client);

        client
            .write_all(&inbound(1, "dcs.bridge.Ping", &[]))
            .expect("the ping is sent");
        let frame = read_frame(&mut client);
        assert_eq!(frame.seq, 2, "the pong did not follow the handshake");
        let any = frame.payload.as_ref().expect("a payload");
        assert_eq!(any.type_url, "type.googleapis.com/dcs.bridge.Pong");
        assert_eq!(
            Pong::decode(&any.value[..]).expect("the pong decodes"),
            Pong {
                dcs_alive: true,
                dcs_last_heard_ms: Some(12),
                bridge_enabled: true,
            }
        );
        assert!(commit.is_empty(), "the answer went through the commit ring");
        assert_eq!(commit.dropped(), 0);

        drop(listener);
        drop(writer);
    }

    /// A frame over the cap, a body that is not an envelope, and a topic
    /// that is not `Ping` before authentication each close the connection
    /// that sent them, and the other connection keeps receiving.
    #[test]
    fn a_bad_frame_closes_its_connection_alone() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);
        let mut staying = client(listener.local_addr());
        let on_staying = first_frame(&mut commit, &mut staying);

        let oversize = {
            let mut bytes = (inbound::Limits::default().max_frame_bytes + 1)
                .to_le_bytes()
                .to_vec();
            bytes.extend([0u8; 64]);
            bytes
        };
        let garbage = vec![3u8, 0, 0, 0, 0xff, 0xff, 0xff];
        let early = inbound(1, "dcs.builtin.Resync", &[]);

        for bad in [oversize, garbage, early] {
            let mut offender = client(listener.local_addr());
            read_handshake(&mut offender);
            offender.write_all(&bad).expect("the frame is sent");
            assert!(is_closed(&mut offender), "the offender was not closed");
        }

        commit.push(record(9_001));
        let frame = read_frame(&mut staying);
        assert_eq!(
            frame.seq,
            on_staying.seq + 1,
            "the staying connection saw a gap"
        );
        assert_eq!(value(&frame), 9_001);

        drop(listener);
        drop(writer);
    }

    /// A connection the reader closes is detached from the writer while
    /// nothing is committed: the drainer is parked on an empty ring, and no
    /// socket event reaches a parked thread, so the reader wakes it. The
    /// detach is read as an answer to that connection counted unaddressed;
    /// the first accepted socket is connection 1.
    #[test]
    fn a_connection_the_reader_closes_is_detached_with_nothing_committed() {
        let (writer, _commit, connections) = Writer::spawn(64);
        let answering = connections.clone();
        let listener = listener(connections);

        let mut offender = client(listener.local_addr());
        read_handshake(&mut offender);
        offender
            .write_all(&[3u8, 0, 0, 0, 0xff, 0xff, 0xff])
            .expect("the frame is sent");
        assert!(is_closed(&mut offender), "the offender was not closed");

        // Detach travels the control channel ahead of this answer, so the
        // answer finds no connection once the drainer has returned.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            answering.answer(ConnectionId::from_raw(1), record(1));
            thread::sleep(Duration::from_millis(10));
            if writer.unaddressed() > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the closed connection was never detached"
            );
        }

        drop(listener);
        drop(writer);
    }

    /// A peer that trickles a frame a byte at a time is cut at the
    /// deadline, not one read past it: the deadline is wall-clock and every
    /// read under it is armed with what is left.
    #[test]
    fn a_trickled_frame_does_not_outlive_the_handshake_timeout() {
        struct Quick;
        impl Answers for Quick {
            fn handshake(&self) -> Record {
                Stub.handshake()
            }
            fn liveness(&self) -> inbound::Liveness {
                Stub.liveness()
            }
            fn limits(&self) -> inbound::Limits {
                inbound::Limits {
                    handshake_timeout: Duration::from_millis(300),
                    ..inbound::Limits::default()
                }
            }
            fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
                Stub.authenticate(secret)
            }
            fn disconnected(&self, _: &Session) {}
            fn schema(&self) -> Option<Record> {
                None
            }
            fn seq_ack(&self, _: u64) {}
            fn set_enabled(&self, _: bool) {}
            fn refused_no_capability(&self, _: &str) {}
        }
        let (writer, _commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64, Arc::new(Quick)).unwrap();
        let mut trickling = client(listener.local_addr());
        read_handshake(&mut trickling);

        let started = Instant::now();
        let frame = inbound(1, "dcs.bridge.Ping", &[]);
        let mut sent = 0;
        // One byte every 100 ms would keep a per-read timeout of 300 ms
        // reset forever; the deadline closes the socket at 300 ms anyway.
        while sent < frame.len() && trickling.write_all(&frame[sent..=sent]).is_ok() {
            sent += 1;
            thread::sleep(Duration::from_millis(100));
            if started.elapsed() > Duration::from_secs(3) {
                break;
            }
        }
        assert!(
            is_closed(&mut trickling),
            "the trickling peer was not closed"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the deadline let a trickled frame run {:?}",
            started.elapsed()
        );

        drop(listener);
        drop(writer);
    }

    /// A session is reported closed however its connection ends, a refused
    /// message, a clean close by the peer, or a cut mid-frame, and a failed
    /// authentication opened nothing to close: a session lost on any path
    /// would hold its slot under `max_connections` for the process's life.
    #[test]
    fn a_session_is_reported_closed_however_the_connection_ends() {
        /// The stub, counting sessions opened and closed.
        struct Counting {
            opened: Arc<AtomicU64>,
            closed: Arc<AtomicU64>,
        }
        impl Answers for Counting {
            fn handshake(&self) -> Record {
                Stub.handshake()
            }
            fn liveness(&self) -> inbound::Liveness {
                Stub.liveness()
            }
            fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
                let session = Stub.authenticate(secret)?;
                self.opened.fetch_add(1, Ordering::SeqCst);
                Ok(session)
            }
            fn disconnected(&self, _: &Session) {
                self.closed.fetch_add(1, Ordering::SeqCst);
            }
            fn schema(&self) -> Option<Record> {
                None
            }
            fn seq_ack(&self, _: u64) {}
            fn set_enabled(&self, _: bool) {}
            fn refused_no_capability(&self, _: &str) {}
        }
        let opened = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicU64::new(0));
        let (writer, _commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn(
            "127.0.0.1:0",
            connections,
            64,
            Arc::new(Counting {
                opened: Arc::clone(&opened),
                closed: Arc::clone(&closed),
            }),
        )
        .unwrap();
        let wait_closed = |n: u64| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while closed.load(Ordering::SeqCst) < n {
                assert!(
                    Instant::now() < deadline,
                    "a session was never reported closed"
                );
                thread::sleep(Duration::from_millis(5));
            }
        };

        // A refused message after authentication: the broker closes.
        let mut refused = client(listener.local_addr());
        read_handshake(&mut refused);
        assert!(authenticate(&mut refused, SECRET).1.ok);
        refused
            .write_all(&inbound(2, "dcs.builtin.Resync", &[]))
            .expect("the frame is sent");
        assert!(is_closed(&mut refused));
        wait_closed(1);

        // A peer that closes cleanly, and one that closes inside a frame.
        let mut leaving = client(listener.local_addr());
        read_handshake(&mut leaving);
        assert!(authenticate(&mut leaving, SECRET).1.ok);
        drop(leaving);
        wait_closed(2);

        let mut cut = client(listener.local_addr());
        read_handshake(&mut cut);
        assert!(authenticate(&mut cut, SECRET).1.ok);
        cut.write_all(&[9u8, 0, 0, 0, 1])
            .expect("half a frame is sent");
        drop(cut);
        wait_closed(3);

        // A failed authentication opened nothing and closes nothing.
        let mut wrong = client(listener.local_addr());
        read_handshake(&mut wrong);
        assert!(!authenticate(&mut wrong, b"wrong").1.ok);
        assert!(is_closed(&mut wrong));
        assert_eq!(opened.load(Ordering::SeqCst), 3);
        assert_eq!(closed.load(Ordering::SeqCst), 3);

        drop(listener);
        drop(writer);
    }

    /// A wrong secret is answered with the error and then closed, with the
    /// result on the wire before the close; a right one is answered `ok`
    /// and the connection then receives what is fanned out, numbered
    /// straight after the result. Until then it receives `Pong` and no
    /// record, and a second `Auth` after success closes it.
    #[test]
    fn auth_is_answered_then_refused_or_admitted() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = listener(connections);

        let mut wrong = client(listener.local_addr());
        read_handshake(&mut wrong);
        let (frame, result) = authenticate(&mut wrong, b"wrong");
        assert_eq!(frame.seq, 2);
        assert_eq!(
            result,
            AuthResult {
                ok: false,
                error: AuthError::BadToken as i32
            }
        );
        assert!(
            is_closed(&mut wrong),
            "a failed auth left the connection open"
        );

        let mut pending = client(listener.local_addr());
        read_handshake(&mut pending);
        commit.push(record(1));
        pending
            .write_all(&inbound(1, "dcs.bridge.Ping", &[]))
            .expect("the ping is sent");
        let frame = read_frame(&mut pending);
        assert_eq!(
            frame.payload.unwrap().type_url,
            "type.googleapis.com/dcs.bridge.Pong",
            "an unauthenticated connection received a record"
        );
        assert_eq!(frame.seq, 2, "the withheld record moved seq");

        let (frame, result) = authenticate(&mut pending, SECRET);
        assert_eq!(frame.seq, 3);
        assert!(result.ok);
        commit.push(record(2));
        let frame = read_frame(&mut pending);
        assert_eq!(frame.seq, 4, "the first record did not follow the result");
        assert_eq!(value(&frame), 2);

        pending
            .write_all(&inbound(2, "dcs.bridge.Auth", &[]))
            .expect("the second auth is sent");
        assert!(is_closed(&mut pending), "a second auth was accepted");

        drop(listener);
        drop(writer);
    }

    /// A connection that sends nothing but `Ping` is closed once the
    /// handshake timeout passes, and one that sends nothing at all is too.
    #[test]
    fn an_unauthenticated_connection_is_closed_at_the_timeout() {
        struct Quick;
        impl Answers for Quick {
            fn handshake(&self) -> Record {
                Stub.handshake()
            }
            fn liveness(&self) -> inbound::Liveness {
                Stub.liveness()
            }
            fn limits(&self) -> inbound::Limits {
                inbound::Limits {
                    handshake_timeout: Duration::from_millis(300),
                    ..inbound::Limits::default()
                }
            }
            fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
                Stub.authenticate(secret)
            }
            fn disconnected(&self, _: &Session) {}
            fn schema(&self) -> Option<Record> {
                None
            }
            fn seq_ack(&self, _: u64) {}
            fn set_enabled(&self, _: bool) {}
            fn refused_no_capability(&self, _: &str) {}
        }
        let (writer, _commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64, Arc::new(Quick)).unwrap();

        let mut silent = client(listener.local_addr());
        let mut pinging = client(listener.local_addr());
        read_handshake(&mut silent);
        read_handshake(&mut pinging);

        let started = Instant::now();
        let mut pings = 0;
        while started.elapsed() < Duration::from_millis(200) {
            pinging
                .write_all(&inbound(pings, "dcs.bridge.Ping", &[]))
                .expect("the ping is sent");
            read_frame(&mut pinging);
            pings += 1;
            thread::sleep(Duration::from_millis(20));
        }
        assert!(pings > 1, "no ping was answered inside the timeout");

        assert!(is_closed(&mut silent), "the silent connection stayed open");
        assert!(
            is_closed(&mut pinging),
            "pinging kept the connection open past the timeout"
        );

        drop(listener);
        drop(writer);
    }

    /// A cap lowered while a connection is open binds on that connection's
    /// next frame: the reader asks for the limits as each frame arrives,
    /// not once at accept and not before it waits. A `Ping` that passed
    /// under the default is refused once the frame cap is under its length,
    /// and the connection closes at once. The connection is authenticated
    /// first, so the handshake deadline cannot be what closes it.
    #[test]
    fn a_cap_lowered_under_load_binds_on_the_next_frame() {
        struct Live(Arc<RwLock<inbound::Limits>>);
        impl Answers for Live {
            fn handshake(&self) -> Record {
                Stub.handshake()
            }
            fn liveness(&self) -> inbound::Liveness {
                Stub.liveness()
            }
            fn limits(&self) -> inbound::Limits {
                *self.0.read().unwrap_or_else(PoisonError::into_inner)
            }
            fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
                Stub.authenticate(secret)
            }
            fn disconnected(&self, _: &Session) {}
            fn schema(&self) -> Option<Record> {
                None
            }
            fn seq_ack(&self, _: u64) {}
            fn set_enabled(&self, _: bool) {}
            fn refused_no_capability(&self, _: &str) {}
        }
        let limits = Arc::new(RwLock::new(inbound::Limits::default()));
        let (writer, _commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn(
            "127.0.0.1:0",
            connections,
            64,
            Arc::new(Live(Arc::clone(&limits))),
        )
        .unwrap();
        let mut client = client(listener.local_addr());
        read_handshake(&mut client);
        assert!(authenticate(&mut client, SECRET).1.ok);

        let ping = inbound(1, "dcs.bridge.Ping", &[]);
        client.write_all(&ping).expect("the ping is sent");
        let answered = read_frame(&mut client);
        assert_eq!(answered.seq, 3, "the first ping was not answered");

        // One swap while the reader waits for the next length prefix; it
        // asks for the cap once that prefix is in, so this is the cap the
        // frame meets.
        limits.write().unwrap().max_frame_bytes = (ping.len() - 4 - 1) as u32;
        let started = Instant::now();
        client.write_all(&ping).expect("the ping is sent");
        assert!(
            is_closed(&mut client),
            "a frame over the lowered cap kept the connection"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the close took {:?}, which is a timeout rather than the cap",
            started.elapsed()
        );

        drop(listener);
        drop(writer);
    }
}
