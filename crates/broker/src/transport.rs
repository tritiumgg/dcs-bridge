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
//! What a connection can say, and the handshake that precedes any of it, are
//! later tasks'. A socket that connects here receives from its first frame.

use std::io::{self, IoSlice, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use crate::encode::varint_len;
use crate::fanout::{ConnectionId, Connections, LOOKS_BEFORE_PARK, Numbered, ParkFlag, Waker};
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
    /// `ring_capacity` records and given a thread to drain it. The bind is
    /// the one thing here that fails, and it fails before any thread starts.
    ///
    /// # Panics
    ///
    /// If `ring_capacity` is zero, or if a thread cannot be spawned.
    pub fn spawn(
        addr: impl ToSocketAddrs,
        connections: Connections<Record>,
        ring_capacity: usize,
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
                .spawn(move || accept_loop(listener, connections, ring_capacity, stop, open))
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

        // The connection's thread parks on its ring, and the writer thread
        // wakes it, so the thread has to exist before the ring is attached:
        // it starts by waiting for the ring, which arrives once the writer
        // thread knows whom to wake.
        let flag = Arc::new(ParkFlag::new());
        let (hand, take) = std::sync::mpsc::channel();
        let handle = {
            let flag = Arc::clone(&flag);
            let stop = Arc::clone(&stop);
            let connections = connections.clone();
            let stream = match stream.try_clone() {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            thread::Builder::new()
                .name("dcsbridge-conn".into())
                .spawn(move || {
                    let Ok((id, consumer)) = take.recv() else {
                        return;
                    };
                    let _ = serve(stream, consumer, &flag, &stop);
                    connections.detach(id);
                })
                .expect("a connection thread spawns")
        };

        let waker = Waker::new(flag, handle.thread().clone());
        let attached: (ConnectionId, Consumer<Numbered<Record>>) =
            connections.attach_with(ring_capacity, waker);
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
    }

    for handle in serving {
        let _ = handle.join();
    }
}

/// A connection thread's loop: drain the ring into the socket, and sleep on
/// the ring when it is empty, the way the writer thread sleeps on the commit
/// ring. Returns when the socket refuses a write or the listener stops.
fn serve(
    mut stream: TcpStream,
    mut consumer: Consumer<Numbered<Record>>,
    flag: &ParkFlag,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut empty_passes = 0;

    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        match consumer.pop() {
            Some(record) => {
                write_frame(&mut stream, &record)?;
                empty_passes = 0;
            }
            None if empty_passes < LOOKS_BEFORE_PARK => {
                empty_passes += 1;
                thread::yield_now();
            }
            None => {
                empty_passes = 0;
                flag.park_unless(|| !consumer.is_empty() || stop.load(Ordering::SeqCst));
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
    let mut n = 5;
    let mut seq = record.seq;
    while seq >= 0x80 {
        // Truncation keeps the low seven bits, which is the point.
        header[n] = (seq as u8) | 0x80;
        seq >>= 7;
        n += 1;
    }
    header[n] = seq as u8;

    write_all_vectored(stream, &header[..=n], tail)
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
    use prost::Message;
    use std::io::Read;
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

    /// Commit numbered records, a few at a time, until `client` has read
    /// its first frame. A record committed before the writer thread knows the
    /// connection is not delivered to it, and nothing outside the writer
    /// thread can say when that is, so the test keeps committing until
    /// something arrives. Returns the first frame and the number it carried.
    fn first_frame(commit: &mut crate::fanout::Commit<Record>, client: &mut TcpStream) -> Envelope {
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
    /// decodes as an envelope, `seq` counts from one, and the type URL
    /// carries the topic.
    #[test]
    fn frames_carry_seq_and_name_the_topic() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64).unwrap();
        let mut client = client(listener.local_addr());

        let first = first_frame(&mut commit, &mut client);
        assert_eq!(first.seq, 1);
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

    /// Two connections are two streams: each numbers from one, and one
    /// closing leaves the other receiving.
    #[test]
    fn each_connection_numbers_from_one_and_closes_alone() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64).unwrap();

        let mut first = client(listener.local_addr());
        assert_eq!(first_frame(&mut commit, &mut first).seq, 1);

        let mut second = client(listener.local_addr());
        let on_second = first_frame(&mut commit, &mut second);
        assert_eq!(
            on_second.seq, 1,
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

    /// Dropping the listener returns its thread and every connection's, and
    /// a client that was connected reads end of stream.
    #[test]
    fn dropping_the_listener_closes_every_connection() {
        let (writer, mut commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 64).unwrap();
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
}
