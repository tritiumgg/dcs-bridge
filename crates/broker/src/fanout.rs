//! One producer ring, and the writer thread that fans it out.
//!
//! The logic thread writes one ring, whatever the number of connections. The
//! writer thread reads that ring and pushes each record into a ring per
//! connection, so a connection added at runtime costs the writer thread a push
//! and the logic thread nothing: `max_connections` never multiplies the cost of
//! a record on the thread that runs the simulation.
//!
//! Every ring here keeps one producer and one consumer. The commit ring's
//! producer is the logic thread, through [`Commit`], and its consumer is the
//! writer thread. A connection's ring has the writer thread as its producer and
//! the connection's own socket thread as its consumer, reached through the
//! [`Consumer`] that [`Connections::attach`] hands back. A socket that stops
//! taking bytes fills its own ring, which evicts and counts, and stalls nothing
//! else. ADR 0011.
//!
//! A record is fanned out to every connection, or addressed to one. A reply
//! or an acknowledgement answers the connection that sent the command, and
//! goes to that connection's ring and no other; the writer thread pushes it
//! there and clones nothing. So two connections see different record streams,
//! which is why the numbering below is per connection rather than global.
//!
//! The broker's own records to a connection, the handshake and the answers
//! the reader thread gives it, are addressed too, and they come from threads
//! that are not the logic thread. They reach the writer thread through
//! [`Connections::answer`], the same channel that attaches a connection, so
//! the connection's ring keeps the writer thread as its one producer and the
//! answer takes its `seq` in order with everything else. ADR 0018.
//!
//! The writer thread numbers what it pushes. Each connection has its own
//! `seq`, rising by one per record from one, assigned before the push and so
//! before the ring decides whether the record stays. A record the ring evicts
//! took its number with it, and the consumer reads the loss as a gap. An
//! addressed record moves only its own connection's `seq`, so it leaves no
//! gap anywhere else.
//!
//! The ring itself has no way to say a record arrived, so the writer thread
//! parks when the commit ring has stayed empty for a while and the logic thread
//! wakes it. Waking is a flag the writer raises before it parks and the logic
//! thread reads after each push, so a push into a ring the writer is awake for
//! costs one atomic load and no system call. The argument that no wake is lost
//! is beside [`Waker::wake_if_parked`] and [`ParkFlag::park_unless`], and Loom
//! checks it. A connection's thread sleeps on its ring the same way, with the
//! writer thread as the side that wakes it, through the [`Waker`] it hands
//! [`Connections::attach_with`]. ADR 0011.

// Loom's channel returns `std`'s error type rather than a model of its own.
use std::sync::mpsc::TryRecvError;

use crate::ring::{Consumer, Producer, Push, Ring};
use crate::sync::{Arc, AtomicBool, AtomicU64, Ordering, fence, mpsc, thread};

/// Names one connection for the life of the process, and is never reused.
///
/// Numbered from one, in the order connections attach, by a counter the
/// writer owns that only ever rises. The process starts one writer, behind
/// the bridge's outbound path, so a number handed out once is handed out
/// once for the life of the process: a late answer addressed to a closed
/// connection cannot reach a newer one, because no newer one has that
/// number. A test binary spawns many writers and each numbers from one,
/// which is what lets a test know the id an accepted socket was given.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// The id behind a number this writer handed out.
    ///
    /// Lua receives an id as a number and hands it back to address an answer,
    /// so this is the way back. A number the writer never handed out names
    /// no connection, and a record addressed to it is dropped and counted.
    pub const fn from_raw(n: u64) -> Self {
        Self(n)
    }

    /// The number behind the id.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A record as the commit ring carries it: for every connection, or for one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Addressed<T> {
    /// The one connection the record is for, or every connection.
    pub to: Option<ConnectionId>,
    /// The record itself.
    pub record: T,
}

/// A record as one connection receives it: numbered in that connection's
/// sequence.
///
/// `seq` starts at one and rises by one per record the writer thread pushed
/// into the connection's ring, whether or not the ring kept it. A consumer
/// that reads 4 after 2 has lost 3, and only 3.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Numbered<T> {
    /// The record's place in this connection's stream.
    pub seq: u64,
    /// The record itself.
    pub record: T,
}

/// What the attach side tells the writer thread.
///
/// These cross a channel rather than a ring because none of them is sent from
/// the logic thread, so a channel's allocation and its lock cost nothing that
/// matters here.
enum Control<T> {
    /// A connection exists; push every record from here on into this ring,
    /// and wake the thread draining it if it sleeps.
    Attach(ConnectionId, Producer<Numbered<T>>, Option<Waker>),
    /// A connection is gone; drop its ring's producer.
    Detach(ConnectionId),
    /// The broker answers this connection: push the record into its ring,
    /// numbered in its `seq`.
    Answer(ConnectionId, T),
    /// Return from the loop.
    Stop,
}

/// The sleeping side of the wake protocol: a flag a thread raises before it
/// parks on an empty ring, so that the thread filling the ring can tell a
/// sleeper from a thread still looking.
#[derive(Debug, Default)]
pub struct ParkFlag {
    /// Raised before the park, lowered after the wake.
    parked: AtomicBool,
}

impl ParkFlag {
    /// A flag that is down.
    pub fn new() -> Self {
        Self {
            parked: AtomicBool::new(false),
        }
    }

    /// Raise the flag, look once more, and park unless `has_work` says there
    /// is something to do.
    ///
    /// The look after the flag is what keeps a wake from being lost: a record
    /// that landed between the caller's last look and the flag is seen here,
    /// and one that lands after the flag finds it raised and wakes us.
    /// [`Waker::wake_if_parked`] has the argument.
    pub fn park_unless(&self, has_work: impl FnOnce() -> bool) {
        self.parked.store(true, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        if !has_work() {
            thread::park();
        }
        self.parked.store(false, Ordering::SeqCst);
    }
}

/// The waking side: a way to wake one sleeping thread, held by whoever fills
/// the ring it sleeps on.
#[derive(Debug)]
pub struct Waker {
    flag: Arc<ParkFlag>,
    sleeper: thread::Thread,
}

impl Waker {
    /// A waker for `sleeper`, which parks through `flag`.
    pub fn new(flag: Arc<ParkFlag>, sleeper: thread::Thread) -> Self {
        Self { flag, sleeper }
    }

    /// Wake the sleeper if it is parked, or about to be.
    ///
    /// This is the pushing side's half of the protocol, so it costs one load
    /// in the common case and a system call only when the sleeper has
    /// actually gone to sleep. Both sides use `SeqCst`, for the reason
    /// `ring.rs` gives: the strength is unmeasured and the cheaper mistake is
    /// the slow one.
    ///
    /// Why no wake is lost: the sleeper raises the flag and then reads the
    /// ring's depth; the pusher publishes the ring's write index and then
    /// reads the flag. Under one total order of those four operations, either
    /// the pusher's load comes after the sleeper's store and sees the flag,
    /// or the sleeper's load comes after the pusher's store and sees the
    /// record. A wake that arrives before the park makes the park return at
    /// once.
    ///
    /// The fence between the store and the load is what carries that total
    /// order. `SeqCst` on the accesses alone would too, but Loom models those as
    /// acquire and release and reports a lost wake the memory model forbids,
    /// while a `SeqCst` fence it models in full. On the product target the
    /// fence is one more full barrier per push, which is the measurable
    /// mistake rather than the corrupting one.
    pub fn wake_if_parked(&self) {
        fence(Ordering::SeqCst);
        if self.flag.parked.load(Ordering::SeqCst) {
            self.sleeper.unpark();
        }
    }

    /// Wake the sleeper whether or not it is parked.
    ///
    /// For a side that does not publish through the ring, so the flag argument
    /// above does not cover it. An unconditional wake does: the token is
    /// stored if the sleeper is not parked yet and the next park returns at
    /// once.
    pub fn wake(&self) {
        self.sleeper.unpark();
    }
}

impl Clone for Waker {
    fn clone(&self) -> Self {
        Self {
            flag: Arc::clone(&self.flag),
            sleeper: self.sleeper.clone(),
        }
    }
}

/// The logic thread's end of the commit ring.
///
/// `push` takes `&mut self`, so there is one committer, and it is whoever holds
/// this. Nothing in here blocks, allocates or waits on the writer thread.
pub struct Commit<T> {
    producer: Producer<Addressed<T>>,
    waker: Waker,
}

impl<T> Commit<T> {
    /// Commit a record for every connection.
    ///
    /// A full ring evicts its oldest record, which comes back here rather than
    /// being destroyed inside the ring. Dropping it is the caller's, and on the
    /// logic thread that is a deallocation per lost record under pressure;
    /// ADR 0011 accepts that until the record type is fixed and its drop cost
    /// is known.
    pub fn push(&mut self, value: T) -> Push<T> {
        self.push_addressed(None, value)
    }

    /// Commit a record for one connection and no other.
    ///
    /// The record is queued whether or not `to` is still attached: the writer
    /// thread is the one that knows, and it drops and counts a record whose
    /// connection is gone. Nothing here looks the connection up, so the call
    /// costs the logic thread what `push` does.
    pub fn push_to(&mut self, to: ConnectionId, value: T) -> Push<T> {
        self.push_addressed(Some(to), value)
    }

    /// Push with an address, and hand back what the ring turned away as the
    /// record alone: where it was going is nobody's concern once it is lost.
    fn push_addressed(&mut self, to: Option<ConnectionId>, record: T) -> Push<T> {
        let pushed = match self.producer.push(Addressed { to, record }) {
            Push::Stored => Push::Stored,
            Push::Evicted(lost) => Push::Evicted(lost.record),
            Push::Refused(lost) => Push::Refused(lost.record),
        };
        self.waker.wake_if_parked();

        pushed
    }

    /// How many records the commit ring has turned away.
    pub fn dropped(&self) -> u64 {
        self.producer.dropped()
    }

    /// How many records the commit ring holds, as a gauge.
    pub fn len(&self) -> usize {
        self.producer.len()
    }

    /// Whether the commit ring holds nothing.
    pub fn is_empty(&self) -> bool {
        self.producer.is_empty()
    }
}

/// The side that adds and removes connections.
///
/// Cloneable, because the thread that accepts a socket and the thread that
/// notices one has closed need not be the same thread. Neither is the logic
/// thread.
pub struct Connections<T> {
    control: mpsc::Sender<Control<T>>,
    waker: Waker,
    /// The last connection id handed out.
    last_id: Arc<AtomicU64>,
}

impl<T> Clone for Connections<T> {
    fn clone(&self) -> Self {
        Self {
            control: self.control.clone(),
            waker: self.waker.clone(),
            last_id: Arc::clone(&self.last_id),
        }
    }
}

impl<T> Connections<T> {
    /// Add a connection with a ring of `capacity` records, and hand back the
    /// end its socket thread drains.
    ///
    /// The ring is allocated here, on the attaching thread, so the writer
    /// thread allocates nothing. Records committed after the writer thread
    /// receives the attachment reach the new ring; a record in flight before
    /// it may or may not.
    ///
    /// After the [`Writer`] is gone the consumer returned never receives, and
    /// that is not an error here: a connection accepted while the broker is
    /// shutting down has nothing to receive.
    ///
    /// # Panics
    ///
    /// If `capacity` is zero, for the reason [`Ring::split`] gives.
    pub fn attach(&self, capacity: usize) -> (ConnectionId, Consumer<Numbered<T>>) {
        self.attach_inner(capacity, None)
    }

    /// [`attach`](Self::attach), with a thread to wake: the writer thread
    /// wakes it through `waker` after each push into the new ring, so the
    /// thread draining the ring may park on it through the waker's flag.
    pub fn attach_with(
        &self,
        capacity: usize,
        waker: Waker,
    ) -> (ConnectionId, Consumer<Numbered<T>>) {
        self.attach_inner(capacity, Some(waker))
    }

    fn attach_inner(
        &self,
        capacity: usize,
        waker: Option<Waker>,
    ) -> (ConnectionId, Consumer<Numbered<T>>) {
        let (producer, consumer) = Ring::split(capacity);
        let id = ConnectionId(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        self.send(Control::Attach(id, producer, waker));

        (id, consumer)
    }

    /// Remove a connection. Records already in its ring stay for its consumer
    /// to drain; nothing further arrives.
    pub fn detach(&self, id: ConnectionId) {
        self.send(Control::Detach(id));
    }

    /// Answer a connection: queue `record` for it alone, numbered in its
    /// `seq` in order with whatever the writer thread pushes to it before and
    /// after.
    ///
    /// This is how the handshake and every broker answer reach a connection.
    /// It goes through the writer thread rather than into the ring directly
    /// because the ring has one producer and that is the writer thread, and
    /// because the number an answer takes has to be the next one in the
    /// stream, which only the writer thread knows. It goes through this
    /// channel rather than the commit ring because the commit ring's producer
    /// is the logic thread's, and a second thread on it is the contention
    /// the bridge refuses and counts. A record whose connection is gone by
    /// the time the writer reaches it is dropped and counted as unaddressed.
    /// ADR 0018.
    pub fn answer(&self, id: ConnectionId, record: T) {
        self.send(Control::Answer(id, record));
    }

    fn send(&self, control: Control<T>) {
        // A send fails only when the receiver is gone, which means the writer
        // thread has returned and there is no one to tell.
        let _ = self.control.send(control);
        self.waker.wake();
    }
}

/// The writer thread. Dropping it stops the thread and waits for it.
pub struct Writer<T> {
    handle: Option<thread::JoinHandle<()>>,
    control: mpsc::Sender<Control<T>>,
    thread: thread::Thread,
    /// Records addressed to a connection that was gone when the writer
    /// thread reached them.
    unaddressed: Arc<AtomicU64>,
}

impl<T: Clone + Send + 'static> Writer<T> {
    /// Start the writer thread over a commit ring of `capacity` records.
    ///
    /// Returns the thread's owner, the logic thread's handle and the attach
    /// side. Nothing about the size is decided here; the commit ring has no
    /// configuration key yet, and the caller supplies what it was given.
    /// ADR 0011.
    ///
    /// # Panics
    ///
    /// If `capacity` is zero, or if the thread cannot be spawned. A broker that
    /// cannot start its writer thread has no way to deliver anything, and the
    /// first `shim.configure` is where that surfaces.
    #[must_use]
    pub fn spawn(capacity: usize) -> (Self, Commit<T>, Connections<T>) {
        let (producer, consumer) = Ring::split(capacity);
        let (control, inbox) = mpsc::channel();
        let flag = Arc::new(ParkFlag::new());
        let unaddressed = Arc::new(AtomicU64::new(0));

        let sleeping = Arc::clone(&flag);
        let counting = Arc::clone(&unaddressed);
        let handle = thread::Builder::new()
            .name("dcsbridge-writer".into())
            .spawn(move || Self::run(consumer, inbox, sleeping, &counting))
            .expect("the writer thread spawns");
        let thread = handle.thread().clone();

        let waker = Waker::new(flag, thread.clone());
        let commit = Commit {
            producer,
            waker: waker.clone(),
        };
        let connections = Connections {
            control: control.clone(),
            waker,
            last_id: Arc::new(AtomicU64::new(0)),
        };
        let writer = Self {
            handle: Some(handle),
            control,
            thread,
            unaddressed,
        };

        (writer, commit, connections)
    }

    /// How many addressed records found their connection gone and were
    /// dropped on the writer thread.
    ///
    /// The count is what the specification asks for a `begin_to` record whose
    /// connection has closed: discarded, and counted. It is one number for
    /// the writer rather than one per closed connection, because a
    /// connection that is gone has nothing left to hold a count on.
    pub fn unaddressed(&self) -> u64 {
        self.unaddressed.load(Ordering::Relaxed)
    }

    /// The writer thread's loop.
    ///
    /// Each pass takes every control message, then every record the commit ring
    /// holds. Control before records, so a detached connection stops receiving
    /// at the first pass after the detach. An empty pass yields, and only after
    /// [`LOOKS_BEFORE_PARK`] empty passes in a row does the thread park.
    fn run(
        mut commit: Consumer<Addressed<T>>,
        inbox: mpsc::Receiver<Control<T>>,
        flag: Arc<ParkFlag>,
        unaddressed: &AtomicU64,
    ) {
        let mut connections: Vec<Connection<T>> = Vec::new();
        let mut empty_passes = 0;

        loop {
            loop {
                match inbox.try_recv() {
                    Ok(Control::Attach(id, producer, waker)) => connections.push(Connection {
                        id,
                        producer,
                        next_seq: 1,
                        waker,
                    }),
                    Ok(Control::Detach(id)) => connections.retain(|held| held.id != id),
                    Ok(Control::Answer(id, record)) => {
                        fan_out(
                            &mut connections,
                            Addressed {
                                to: Some(id),
                                record,
                            },
                            unaddressed,
                        );
                    }
                    Ok(Control::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }

            let mut fanned = false;
            while let Some(record) = commit.pop() {
                fan_out(&mut connections, record, unaddressed);
                fanned = true;
            }
            if fanned {
                empty_passes = 0;
                continue;
            }

            // Records arrive in bursts, one frame's worth at a time with a
            // frame of quiet after, and within a burst they are microseconds
            // apart. Parking on the first empty look would put a system call on
            // the logic thread for every record in the burst, because each one
            // would find the writer asleep again. Looking a while longer costs
            // this thread a little idle spinning per burst and the logic thread
            // one wake per burst instead. ADR 0011.
            if empty_passes < LOOKS_BEFORE_PARK {
                empty_passes += 1;
                thread::yield_now();
                continue;
            }
            empty_passes = 0;

            flag.park_unless(|| !commit.is_empty());
        }
    }
}

/// How many empty passes a draining thread makes, yielding between them,
/// before it parks.
///
/// A yield is a few microseconds on each host, so this is on the order of a
/// hundred microseconds of looking: longer than the gap between two records in
/// one frame's drain, and far shorter than the frame of quiet that follows it.
/// Nothing has measured this; it moves when a probe prices the wake.
#[cfg(not(loom))]
pub const LOOKS_BEFORE_PARK: u32 = 32;

/// Under Loom every yield is a branch, and what the model checks is the park
/// handshake, which the looking only delays. So the sleeper parks at once.
#[cfg(loom)]
pub const LOOKS_BEFORE_PARK: u32 = 0;

impl<T> Drop for Writer<T> {
    /// Stop the thread and wait for it.
    ///
    /// A join fails only if the thread panicked, and that panic has already
    /// been reported where it happened; a drop is no place to raise a second.
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        self.thread.unpark();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// One connection, as the writer thread holds it.
struct Connection<T> {
    id: ConnectionId,
    producer: Producer<Numbered<T>>,
    /// The `seq` the next record pushed here takes.
    next_seq: u64,
    /// The thread draining the ring, if it sleeps on it.
    waker: Option<Waker>,
}

impl<T> Connection<T> {
    /// Number a record and push it, whether or not the ring keeps it, then
    /// wake the drainer if it sleeps.
    ///
    /// The number is taken before the push, so an evicted record leaves the
    /// gap that tells its consumer it was lost.
    fn push(&mut self, record: T) {
        let seq = self.next_seq;
        self.next_seq += 1;
        drop(self.producer.push(Numbered { seq, record }));
        if let Some(waker) = &self.waker {
            waker.wake_if_parked();
        }
    }
}

/// Push one record where it is addressed: into every connection's ring, or
/// into one, numbered per connection either way.
///
/// Fanned out, each connection but the last gets a clone, and the last gets
/// the record itself, so a single connection costs no clone at all. With no
/// connection attached the record is dropped and counted nowhere: there was
/// no one to lose it.
///
/// Addressed, the one connection gets the record and no other connection's
/// `seq` moves. A connection that has detached, or a number that was never
/// handed out, is a record with nowhere to go: dropped here and counted in
/// `unaddressed`, because somebody sent it.
///
/// A record a ring turns away is dropped here, on the writer thread, and the
/// ring has already counted it against that connection.
fn fan_out<T: Clone>(
    connections: &mut [Connection<T>],
    addressed: Addressed<T>,
    unaddressed: &AtomicU64,
) {
    let Addressed { to, record } = addressed;

    if let Some(to) = to {
        match connections.iter_mut().find(|held| held.id == to) {
            Some(connection) => connection.push(record),
            None => {
                unaddressed.fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }

    let Some((last, rest)) = connections.split_last_mut() else {
        return;
    };

    for connection in rest {
        connection.push(record.clone());
    }
    last.push(record);
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    /// A capacity no test here fills, so a drop means the fan-out lost a record
    /// rather than a ring evicted one.
    const ROOMY: usize = 4096;

    /// Pop until `count` records have arrived, or fail after a while rather
    /// than hang the suite. The records alone, for the tests that are not
    /// about numbering.
    fn drain_until<T>(consumer: &mut Consumer<Numbered<T>>, count: usize) -> Vec<T> {
        drain_numbered(consumer, count)
            .into_iter()
            .map(|numbered| numbered.record)
            .collect()
    }

    /// Pop until `count` records have arrived, with their numbers.
    fn drain_numbered<T>(consumer: &mut Consumer<Numbered<T>>, count: usize) -> Vec<Numbered<T>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut arrived = Vec::with_capacity(count);

        while arrived.len() < count {
            match consumer.pop() {
                Some(record) => arrived.push(record),
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "only {} of {count} records arrived",
                        arrived.len()
                    );
                    thread::yield_now();
                }
            }
        }

        arrived
    }

    /// Spin until `condition` holds, or fail after a while.
    fn wait_for(condition: impl Fn() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !condition() {
            assert!(Instant::now() < deadline, "{what}");
            thread::yield_now();
        }
    }

    /// The fan-out itself: one commit, every consumer sees it, in order, with
    /// nothing lost anywhere along the way.
    #[test]
    fn every_consumer_sees_every_record_in_order() {
        let pushes: u32 = if cfg!(miri) { 64 } else { 2_000 };
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);

        let consumers: Vec<_> = (0..4)
            .map(|_| {
                let (_, mut consumer) = connections.attach(ROOMY);
                thread::spawn(move || {
                    let arrived = drain_until(&mut consumer, pushes as usize);
                    (arrived, consumer.dropped())
                })
            })
            .collect();

        for value in 0..pushes {
            assert_eq!(commit.push(value), Push::Stored, "{value} found no room");
        }

        let expected: Vec<u32> = (0..pushes).collect();
        for consumer in consumers {
            let (arrived, dropped) = consumer.join().expect("the consumer only pops");
            assert_eq!(arrived, expected, "a consumer missed or reordered records");
            assert_eq!(dropped, 0, "a roomy ring turned a record away");
        }
        assert_eq!(commit.dropped(), 0, "the commit ring turned a record away");

        drop(writer);
    }

    /// A consumer that stops reading costs its own records and nobody else's,
    /// the loss is counted on its ring, and it shows in the numbering as a
    /// gap: the forced drop is what a consumer sees as missing `seq` values.
    #[test]
    fn a_stalled_consumer_loses_only_its_own_records_and_sees_the_gap() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (_, mut stalled) = connections.attach(4);
        let (_, mut reading) = connections.attach(ROOMY);

        for value in 0..10u32 {
            commit.push(value);
        }

        let arrived = drain_numbered(&mut reading, 10);
        let records: Vec<u32> = arrived.iter().map(|n| n.record).collect();
        let seqs: Vec<u64> = arrived.iter().map(|n| n.seq).collect();
        assert_eq!(
            records,
            (0..10).collect::<Vec<_>>(),
            "the reader lost records"
        );
        assert_eq!(
            seqs,
            (1..=10).collect::<Vec<_>>(),
            "the reader's seq skipped"
        );
        assert_eq!(reading.dropped(), 0, "a roomy ring turned a record away");

        // The reader has all ten, so the writer thread has fanned all ten,
        // and the stalled ring's state is final.
        assert_eq!(
            stalled.dropped(),
            6,
            "six evictions were not counted as six"
        );
        let survivors: Vec<Numbered<u32>> = std::iter::from_fn(|| stalled.pop()).collect();
        let records: Vec<u32> = survivors.iter().map(|n| n.record).collect();
        let seqs: Vec<u64> = survivors.iter().map(|n| n.seq).collect();
        assert_eq!(
            records,
            [6, 7, 8, 9],
            "the wrong records survived the stall"
        );
        assert_eq!(
            seqs,
            [7, 8, 9, 10],
            "the evictions did not leave a gap of six before the survivors"
        );

        drop(writer);
    }

    /// A connection that attaches mid-stream sees what comes after it, and one
    /// that detaches keeps what it was given and stops there.
    #[test]
    fn attach_and_detach_take_effect_mid_stream() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (first_id, mut first) = connections.attach(ROOMY);

        for value in 0..5u32 {
            commit.push(value);
        }
        assert_eq!(drain_until(&mut first, 5), (0..5).collect::<Vec<_>>());

        let (second_id, mut second) = connections.attach(ROOMY);
        assert!(
            second_id > first_id,
            "ids did not rise: {first_id:?} then {second_id:?}"
        );

        for value in 5..10u32 {
            commit.push(value);
        }
        let on_first = drain_numbered(&mut first, 5);
        let on_second = drain_numbered(&mut second, 5);
        for (arrived, what) in [(&on_first, "first"), (&on_second, "second")] {
            let records: Vec<u32> = arrived.iter().map(|n| n.record).collect();
            assert_eq!(
                records,
                (5..10).collect::<Vec<_>>(),
                "on the {what} connection"
            );
        }
        // Numbering is per connection: the late one starts at one while the
        // first carries on from where it was.
        let seqs = |arrived: &[Numbered<u32>]| arrived.iter().map(|n| n.seq).collect::<Vec<_>>();
        assert_eq!(seqs(&on_first), (6..=10).collect::<Vec<_>>());
        assert_eq!(seqs(&on_second), (1..=5).collect::<Vec<_>>());

        connections.detach(second_id);
        for value in 10..15u32 {
            commit.push(value);
        }
        assert_eq!(drain_until(&mut first, 5), (10..15).collect::<Vec<_>>());

        // A number is never handed out twice: a connection attached after a
        // detach takes a new one, so an answer addressed to the old
        // connection cannot reach the new.
        let (third_id, _third) = connections.attach(ROOMY);
        assert!(
            third_id > second_id,
            "a detached id was reused: {second_id:?} then {third_id:?}"
        );

        // The detach and the five pushes race on the writer thread, so how many
        // of the five reached the second ring is the scheduler's. What every
        // schedule owes is order, and nothing from before the detach going
        // missing.
        drop(writer);
        let late: Vec<u32> = std::iter::from_fn(|| second.pop().map(|n| n.record)).collect();
        assert!(
            late.len() <= 5 && late.iter().zip(10..).all(|(got, want)| *got == want),
            "a detached connection received out of order: {late:?}"
        );
    }

    /// An addressed record reaches the one connection it names, numbered in
    /// that connection's sequence, and no other connection's `seq` moves: a
    /// reply to one consumer is not a gap at every other.
    #[test]
    fn an_addressed_record_reaches_one_connection_and_moves_no_other_seq() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (first_id, mut first) = connections.attach(ROOMY);
        let (_, mut second) = connections.attach(ROOMY);

        commit.push(0u32);
        commit.push_to(first_id, 1);
        commit.push_to(first_id, 2);
        commit.push(3);

        let on_first = drain_numbered(&mut first, 4);
        let on_second = drain_numbered(&mut second, 2);
        assert_eq!(
            on_first,
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
                Numbered { seq: 3, record: 2 },
                Numbered { seq: 4, record: 3 },
            ],
            "the addressed connection did not receive its records in sequence"
        );
        assert_eq!(
            on_second,
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 3 },
            ],
            "the other connection saw the addressed records, or a gap for them"
        );
        assert_eq!(
            writer.unaddressed(),
            0,
            "a delivered record was counted lost"
        );

        drop(writer);
    }

    /// An answer from off the logic thread is numbered in its connection's
    /// stream in order with the records committed around it, reaches no
    /// other connection, and finds the writer thread parked: a `Pong` has to
    /// go out while the logic thread commits nothing.
    #[test]
    fn an_answer_is_numbered_in_order_and_wakes_a_parked_writer() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (first_id, mut first) = connections.attach(ROOMY);
        let (_, mut second) = connections.attach(ROOMY);

        commit.push(0u32);
        assert_eq!(
            drain_numbered(&mut second, 1),
            vec![Numbered { seq: 1, record: 0 }]
        );
        // The commit ring is empty and stays so; the writer parks and the
        // answer alone wakes it.
        connections.answer(first_id, 1);
        assert_eq!(
            drain_numbered(&mut first, 2),
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
            ],
            "the answer did not follow the record before it"
        );

        commit.push(2);
        assert_eq!(
            drain_numbered(&mut first, 1),
            vec![Numbered { seq: 3, record: 2 }],
            "the record after the answer did not take the next number"
        );
        assert_eq!(
            drain_numbered(&mut second, 1),
            vec![Numbered { seq: 2, record: 2 }],
            "the other connection saw the answer, or a gap for it"
        );
        assert_eq!(
            writer.unaddressed(),
            0,
            "a delivered answer was counted lost"
        );

        drop(writer);
    }

    /// A record addressed to a connection that has detached, or to a number
    /// never handed out, is dropped on the writer thread and counted, and
    /// reaches nobody else.
    #[test]
    fn an_addressed_record_to_a_missing_connection_is_counted_and_reaches_nobody() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (gone_id, gone) = connections.attach(ROOMY);
        let (_, mut staying) = connections.attach(ROOMY);
        connections.detach(gone_id);
        drop(gone);

        commit.push_to(gone_id, 0u32);
        commit.push_to(ConnectionId::from_raw(u64::MAX), 1);
        commit.push(2);

        assert_eq!(
            drain_numbered(&mut staying, 1),
            vec![Numbered { seq: 1, record: 2 }],
            "a record addressed elsewhere reached the staying connection"
        );
        // The staying connection has the fan-out record, so the writer thread
        // has passed both addressed records before it.
        assert_eq!(
            writer.unaddressed(),
            2,
            "two records with nowhere to go were not counted as two"
        );

        drop(writer);
    }

    /// A record committed with no connection attached goes nowhere, and is
    /// dropped on the writer thread rather than kept.
    #[test]
    fn a_record_with_no_connection_is_dropped_off_the_logic_thread() {
        struct Counted(Arc<AtomicUsize>);
        impl Clone for Counted {
            fn clone(&self) -> Self {
                Self(Arc::clone(&self.0))
            }
        }
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let (writer, mut commit, _connections) = Writer::<Counted>::spawn(ROOMY);

        for _ in 0..5 {
            commit.push(Counted(Arc::clone(&drops)));
        }
        wait_for(
            || drops.load(Ordering::Relaxed) == 5,
            "the writer thread kept records nobody could receive",
        );

        drop(writer);
    }

    /// Dropping the writer stops its thread. What was already fanned out stays
    /// for its consumer, and a commit after that neither blocks nor panics.
    #[test]
    fn dropping_the_writer_stops_the_thread_and_keeps_what_was_delivered() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (_, mut consumer) = connections.attach(ROOMY);

        for value in 0..3u32 {
            commit.push(value);
        }
        assert_eq!(drain_until(&mut consumer, 3), vec![0, 1, 2]);

        drop(writer);

        assert_eq!(
            consumer.pop(),
            None,
            "a record arrived after the writer stopped"
        );
        assert_eq!(
            commit.push(3),
            Push::Stored,
            "a commit after the writer stopped was refused"
        );
        assert_eq!(
            consumer.pop(),
            None,
            "a record was delivered with no writer thread"
        );

        let (_, mut orphan) = connections.attach(ROOMY);
        assert_eq!(
            orphan.pop(),
            None,
            "an attachment after the writer stopped received"
        );
    }
}

/// Loom drives the wake protocol over every interleaving of a committer and a
/// writer thread that parks, which is the one part of this module a test on
/// real hardware cannot be trusted to reach.
#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// No schedule leaves a record in the commit ring with the writer parked:
    /// every record committed reaches the connection.
    ///
    /// The committer spins on its consumer, which Loom explores as a branch per
    /// spin, so the branch budget is raised well past the default. The
    /// preemption bound keeps the schedule count finite: three preemptions is
    /// more than the protocol has decision points.
    #[test]
    fn a_committed_record_always_wakes_the_writer() {
        let mut model = loom::model::Builder::new();
        model.max_branches = 100_000;
        model.preemption_bound = Some(3);

        model.check(|| {
            let (writer, mut commit, connections) = Writer::spawn(2);
            let (_, mut consumer) = connections.attach(2);

            for value in 0..2u32 {
                commit.push(value);
            }

            let mut arrived = Vec::new();
            while arrived.len() < 2 {
                match consumer.pop() {
                    Some(record) => arrived.push(record),
                    None => thread::yield_now(),
                }
            }
            assert_eq!(
                arrived,
                vec![
                    Numbered { seq: 1, record: 0 },
                    Numbered { seq: 2, record: 1 }
                ],
                "records arrived out of order"
            );

            drop(writer);
        });
    }

    /// The same protocol with the writer thread on the waking side: no
    /// schedule leaves a record in a connection's ring with its drainer
    /// parked.
    #[test]
    fn a_fanned_record_always_wakes_the_connection() {
        let mut model = loom::model::Builder::new();
        model.max_branches = 100_000;
        model.preemption_bound = Some(3);

        model.check(|| {
            let (writer, mut commit, connections) = Writer::spawn(2);

            let flag = Arc::new(ParkFlag::new());
            let sleeping = Arc::clone(&flag);
            let (hand, take) = mpsc::channel::<Consumer<Numbered<u32>>>();
            let drainer = thread::spawn(move || {
                let mut consumer = take.recv().expect("the consumer is handed over");
                let mut arrived = Vec::new();
                while arrived.len() < 2 {
                    match consumer.pop() {
                        Some(record) => arrived.push(record.seq),
                        None => sleeping.park_unless(|| !consumer.is_empty()),
                    }
                }
                arrived
            });

            let waker = Waker::new(flag, drainer.thread().clone());
            let (_, consumer) = connections.attach_with(2, waker);
            hand.send(consumer).expect("the drainer is waiting");

            for value in 0..2u32 {
                commit.push(value);
            }

            let arrived = drainer.join().expect("the drainer only pops");
            assert_eq!(arrived, vec![1, 2], "records arrived out of order");

            drop(writer);
        });
    }
}
