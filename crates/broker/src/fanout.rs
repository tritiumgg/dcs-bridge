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
//! The writer thread numbers what it pushes. Each connection has its own
//! `seq`, rising by one per record from one, assigned before the push and so
//! before the ring decides whether the record stays. A record the ring evicts
//! took its number with it, and the consumer reads the loss as a gap. Two
//! connections see different record streams once addressing exists, so the
//! numbering is per connection rather than global.
//!
//! The ring itself has no way to say a record arrived, so the writer thread
//! parks when the commit ring has stayed empty for a while and the logic thread
//! wakes it. Waking is a flag the writer raises before it parks and the logic
//! thread reads after each push, so a push into a ring the writer is awake for
//! costs one atomic load and no system call. The argument that no wake is lost
//! is beside [`Waker::wake_if_parked`] and [`Writer::run`], and Loom checks it.
//! ADR 0011.

// Loom's channel returns `std`'s error type rather than a model of its own.
use std::sync::mpsc::TryRecvError;

use crate::ring::{Consumer, Producer, Push, Ring};
use crate::sync::{Arc, AtomicBool, AtomicU64, Ordering, fence, mpsc, thread};

/// Names one connection for the life of the writer, and is never reused.
///
/// Numbered from one, in the order connections attach. The rule that an id is
/// unique for the process rather than for one writer is a later task's, and
/// this is the counter it will draw from.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// The number behind the id.
    pub fn get(self) -> u64 {
        self.0
    }
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
    /// A connection exists; push every record from here on into this ring.
    Attach(ConnectionId, Producer<Numbered<T>>),
    /// A connection is gone; drop its ring's producer.
    Detach(ConnectionId),
    /// Return from the loop.
    Stop,
}

/// What the three handles share with the writer thread.
struct Shared {
    /// Raised by the writer thread before it parks, lowered after it wakes.
    parked: AtomicBool,
    /// The last connection id handed out.
    last_id: AtomicU64,
}

/// A way to wake the writer thread, held by every handle that may need to.
struct Waker {
    shared: Arc<Shared>,
    writer: thread::Thread,
}

impl Waker {
    /// Wake the writer thread if it is parked, or about to be.
    ///
    /// This is the logic thread's half of the protocol, so it costs one load in
    /// the common case and a system call only when the writer thread has
    /// actually gone to sleep. Both sides use `SeqCst`, for the reason `ring.rs`
    /// gives: the strength is unmeasured and the cheaper mistake is the slow
    /// one.
    ///
    /// Why no wake is lost: the writer raises `parked` and then reads the ring's
    /// depth; the pusher publishes the ring's write index and then reads
    /// `parked`. Under one total order of those four operations, either the
    /// pusher's load comes after the writer's store and sees the flag, or the
    /// writer's load comes after the pusher's store and sees the record. A
    /// wake that arrives before the park makes the park return at once.
    ///
    /// The fence between the store and the load is what carries that total
    /// order. `SeqCst` on the accesses alone would too, but Loom models those as
    /// acquire and release and reports a lost wake the memory model forbids,
    /// while a `SeqCst` fence it models in full. On the product target the
    /// fence is one more full barrier per push, which is the measurable
    /// mistake rather than the corrupting one.
    fn wake_if_parked(&self) {
        fence(Ordering::SeqCst);
        if self.shared.parked.load(Ordering::SeqCst) {
            self.writer.unpark();
        }
    }

    /// Wake the writer thread whether or not it is parked.
    ///
    /// For the attach side, which does not run on the logic thread and does not
    /// publish through the ring, so the flag argument above does not cover it.
    /// An unconditional wake does: the token is stored if the writer is not
    /// parked yet and the next park returns at once.
    fn wake(&self) {
        self.writer.unpark();
    }
}

/// The logic thread's end of the commit ring.
///
/// `push` takes `&mut self`, so there is one committer, and it is whoever holds
/// this. Nothing in here blocks, allocates or waits on the writer thread.
pub struct Commit<T> {
    producer: Producer<T>,
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
        let pushed = self.producer.push(value);
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
}

impl<T> Clone for Connections<T> {
    fn clone(&self) -> Self {
        Self {
            control: self.control.clone(),
            waker: Waker {
                shared: Arc::clone(&self.waker.shared),
                writer: self.waker.writer.clone(),
            },
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
        let (producer, consumer) = Ring::split(capacity);
        let id = ConnectionId(self.waker.shared.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        self.send(Control::Attach(id, producer));

        (id, consumer)
    }

    /// Remove a connection. Records already in its ring stay for its consumer
    /// to drain; nothing further arrives.
    pub fn detach(&self, id: ConnectionId) {
        self.send(Control::Detach(id));
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
        let shared = Arc::new(Shared {
            parked: AtomicBool::new(false),
            last_id: AtomicU64::new(0),
        });

        let running = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("dcsbridge-writer".into())
            .spawn(move || Self::run(consumer, inbox, running))
            .expect("the writer thread spawns");
        let thread = handle.thread().clone();

        let waker = || Waker {
            shared: Arc::clone(&shared),
            writer: thread.clone(),
        };
        let commit = Commit {
            producer,
            waker: waker(),
        };
        let connections = Connections {
            control: control.clone(),
            waker: waker(),
        };
        let writer = Self {
            handle: Some(handle),
            control,
            thread,
        };

        (writer, commit, connections)
    }

    /// The writer thread's loop.
    ///
    /// Each pass takes every control message, then every record the commit ring
    /// holds. Control before records, so a detached connection stops receiving
    /// at the first pass after the detach. An empty pass yields, and only after
    /// [`LOOKS_BEFORE_PARK`] empty passes in a row does the thread park.
    fn run(mut commit: Consumer<T>, inbox: mpsc::Receiver<Control<T>>, shared: Arc<Shared>) {
        let mut connections: Vec<Connection<T>> = Vec::new();
        let mut empty_passes = 0;

        loop {
            loop {
                match inbox.try_recv() {
                    Ok(Control::Attach(id, producer)) => connections.push(Connection {
                        id,
                        producer,
                        next_seq: 1,
                    }),
                    Ok(Control::Detach(id)) => connections.retain(|held| held.id != id),
                    Ok(Control::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }

            let mut fanned = false;
            while let Some(record) = commit.pop() {
                fan_out(&mut connections, record);
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

            // Raise the flag, then look once more. A record that landed
            // between the drain above and the flag is seen here; one that lands
            // after the flag finds it raised and wakes us. `Waker::wake_if_parked`
            // has the argument.
            shared.parked.store(true, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            if commit.is_empty() {
                thread::park();
            }
            shared.parked.store(false, Ordering::SeqCst);
        }
    }
}

/// How many empty passes the writer thread makes, yielding between them,
/// before it parks.
///
/// A yield is a few microseconds on each host, so this is on the order of a
/// hundred microseconds of looking: longer than the gap between two records in
/// one frame's drain, and far shorter than the frame of quiet that follows it.
/// Nothing has measured this; it moves when a probe prices the wake.
#[cfg(not(loom))]
const LOOKS_BEFORE_PARK: u32 = 32;

/// Under Loom every yield is a branch, and what the model checks is the park
/// handshake, which the looking only delays. So the writer parks at once.
#[cfg(loom)]
const LOOKS_BEFORE_PARK: u32 = 0;

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
}

impl<T> Connection<T> {
    /// Number a record and push it, whether or not the ring keeps it.
    ///
    /// The number is taken before the push, so an evicted record leaves the
    /// gap that tells its consumer it was lost.
    fn push(&mut self, record: T) {
        let seq = self.next_seq;
        self.next_seq += 1;
        drop(self.producer.push(Numbered { seq, record }));
    }
}

/// Push one record into every connection's ring, numbered per connection.
///
/// Each connection but the last gets a clone, and the last gets the record
/// itself, so a single connection costs no clone at all. A record a ring turns
/// away is dropped here, on the writer thread, and the ring has already counted
/// it against that connection. With no connection attached the record is
/// dropped and counted nowhere: there was no one to lose it.
fn fan_out<T: Clone>(connections: &mut [Connection<T>], record: T) {
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
}
