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
//! [`Drain`] that [`Connections::attach`] hands back. A socket that stops
//! taking bytes fills its own ring, which evicts and counts, and stalls nothing
//! else. ADR 0011.
//!
//! A connection's ring is three, one per class, so that what a record may
//! evict is decided by which ring it is in: `LOSSY` evicts `LOSSY`, `DURABLE`
//! evicts `DURABLE`, and `LIFECYCLE` evicts nothing. The writer thread is the
//! one producer of all three and the connection's thread the one consumer,
//! merging them back into `seq` order. ADR 0009, ADR 0028.
//!
//! A record is fanned out to every connection, or addressed to one. A reply
//! or an acknowledgement answers the connection that sent the command, and
//! goes to that connection's ring and no other; the writer thread pushes it
//! there and clones nothing. So two connections see different record streams,
//! which is why the numbering below is per connection rather than global.
//!
//! The broker's own records to a connection, the handshake and the answers
//! the reader thread gives it, are addressed too, and they come from threads
//! that are not the logic thread. An answer reaches the writer thread through
//! [`Connections::answer`], the same channel that attaches a connection, so
//! the connection's ring keeps the writer thread as its one producer and the
//! answer takes its `seq` in order with everything else. The handshake rides
//! inside the attach itself, because two messages can be separated by a
//! commit and the handshake has to be first. ADR 0018.
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

/// The capabilities one connection's token grants, as the writer thread
/// holds them: bit `n` set for capability number `n`.
///
/// A mask rather than a set, because the writer thread asks it about every
/// fanned-out record for every connection, and because this module is built
/// under Loom, where the registry and its capability type are not. The
/// bridge's numbers are 1 to 49 and fit. A number past 63 is neither added
/// nor covered, so a record needing one is withheld from everyone rather
/// than disclosed; nothing registers such a number today.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(u64);

impl Capabilities {
    /// No capability at all.
    pub const NONE: Self = Self(0);

    /// This set with capability `number` added.
    #[must_use]
    pub const fn with(self, number: u32) -> Self {
        if number < u64::BITS {
            Self(self.0 | 1 << number)
        } else {
            self
        }
    }

    /// Whether a record needing capability `number` may be received.
    pub const fn covers(self, number: u32) -> bool {
        number < u64::BITS && self.0 & (1 << number) != 0
    }
}

/// Which of a connection's rings a record belongs in, and so what it may
/// evict and what may evict it.
///
/// Three members and not the schema's four, because this is the outbound
/// path: whatever has no outbound class of its own is `Durable` here. The
/// type lives in this module for the reason [`Capabilities`] does: it is
/// built under Loom, where the registry's types are not. ADR 0028.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Class {
    /// Discarded freely under pressure, oldest first.
    Lossy,
    /// Evicted only by another `Durable` record.
    Durable,
    /// Never evicted. A connection with no room for one is closed.
    Lifecycle,
}

/// A record as the commit ring carries it: for every connection, or for one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Addressed<T> {
    /// The one connection the record is for, or every connection.
    pub to: Option<ConnectionId>,
    /// The number of the capability a connection needs to receive the
    /// record. Consulted at fan-out and never for an addressed record: the
    /// one connection it names sent the command it answers.
    pub need: u32,
    /// The ring the record takes in each connection it reaches.
    pub class: Class,
    /// The slot of the retained set the record replaces, for a `LIFECYCLE`
    /// record on a registered topic. Bound at registration and carried
    /// here because the writer thread reads no envelope and holds no
    /// registry. ADR 0029.
    pub slot: Option<u32>,
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

/// The writer thread's end of one connection's outbound queue.
///
/// A type of its own so that what a connection's queue is made of is
/// decided here and nowhere else: the writer thread pushes a numbered
/// record with its class, and the connection's thread pops from the
/// [`Drain`] made with it.
struct Outbox<T> {
    lossy: Producer<Queued<T>>,
    durable: Producer<Queued<T>>,
    lifecycle: Producer<Queued<T>>,
}

/// A numbered record as a connection's ring holds it.
///
/// A ring hands back the record it evicts and nothing about it. Which ring
/// it came from says its class, and the one thing that does not say is
/// whether a record from the `DURABLE` ring was a broker answer, which is
/// counted apart. So that, and only that, is carried. ADR 0028.
struct Queued<T> {
    answer: bool,
    numbered: Numbered<T>,
}

/// What a push cost the connection it was for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lost {
    /// Nothing: the ring had room.
    Nothing,
    /// A `LOSSY` record.
    Lossy,
    /// A `DURABLE` record that was not a broker answer.
    Durable,
    /// A broker answer, or the handshake.
    BrokerAnswer,
    /// Nothing yet, and everything: a boundary record found its ring full,
    /// and the connection is to be closed and not counted against.
    Boundary,
}

impl<T> Outbox<T> {
    /// Queue a numbered record in its class's ring, and say what the ring
    /// turned away to do it. What is turned away is dropped here, on the
    /// writer thread. `answer` marks a broker answer, which is `Durable`.
    ///
    /// The drop rule is which ring a record is in. A `Lossy` record evicts
    /// the oldest `Lossy` and a `Durable` one the oldest `Durable`, so a
    /// flood in one class costs the others nothing. The `Lifecycle` ring
    /// never evicts: a full one refuses the record, which means the
    /// consumer is past saving. ADR 0009.
    ///
    /// An eviction is counted as what the evicted record was. A refusal,
    /// which the other two rings give only while their oldest record is
    /// being read, is counted as what the refused record was.
    fn push(&mut self, class: Class, answer: bool, numbered: Numbered<T>) -> Lost {
        let queued = Queued { answer, numbered };
        let durable = |lost: &Queued<T>| {
            if lost.answer {
                Lost::BrokerAnswer
            } else {
                Lost::Durable
            }
        };
        match class {
            Class::Lossy => match self.lossy.push(queued) {
                Push::Stored => Lost::Nothing,
                Push::Evicted(_) | Push::Refused(_) => Lost::Lossy,
            },
            Class::Durable => match self.durable.push(queued) {
                Push::Stored => Lost::Nothing,
                Push::Evicted(lost) | Push::Refused(lost) => durable(&lost),
            },
            Class::Lifecycle => match self.lifecycle.offer(queued) {
                Push::Stored => Lost::Nothing,
                Push::Evicted(_) | Push::Refused(_) => Lost::Boundary,
            },
        }
    }
}

/// The connection's end of its outbound queue: what its thread pops and
/// writes to the socket, in `seq` order.
///
/// Three rings, merged here. The writer thread numbers a record before it
/// pushes it, and pushes a connection's records in `seq` order, so the
/// lowest `seq` at the head of any ring is the next record, once every ring
/// has been looked at late enough. [`Drain::pop`] has what late enough is.
/// ADR 0028.
pub struct Drain<T> {
    rings: [Consumer<Queued<T>>; 3],
    /// The record popped from each ring and not yet handed on. A record
    /// held here is out of its ring, so nothing evicts it.
    held: [Option<Numbered<T>>; 3],
}

impl<T> Drain<T> {
    /// The next record in `seq` order, or `None` when nothing is queued.
    ///
    /// Each pass pops every ring nothing is held from, and the passes
    /// repeat until one pops nothing. Only then is the lowest held `seq`
    /// trusted. One pass is not enough: it may find the first ring empty,
    /// the writer thread may then push `seq` 9 there and `seq` 10 into the
    /// second, and the pass goes on to pop 10. After a pass that pops
    /// nothing, every record numbered below the lowest held was pushed
    /// before that pass began, so it is held, or its ring evicted it, which
    /// is a gap and not a reorder. A pass that pops fills one of three
    /// places, so this is four passes at most.
    pub fn pop(&mut self) -> Option<Numbered<T>> {
        loop {
            let mut popped = false;
            for (ring, held) in self.rings.iter_mut().zip(&mut self.held) {
                if held.is_none() {
                    *held = ring.pop().map(|queued| queued.numbered);
                    popped |= held.is_some();
                }
            }
            if !popped {
                break;
            }
        }

        self.held
            .iter_mut()
            .filter(|held| held.is_some())
            .min_by_key(|held| held.as_ref().map(|record| record.seq))
            .and_then(Option::take)
    }

    /// Whether nothing is queued or held. What a draining thread asks
    /// before it parks.
    pub fn is_empty(&self) -> bool {
        self.held.iter().all(Option::is_none) && self.rings.iter().all(Consumer::is_empty)
    }

    /// How many records the three rings have turned away.
    pub fn dropped(&self) -> u64 {
        self.rings.iter().map(Consumer::dropped).sum()
    }
}

/// How many records each of a connection's three rings holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacities {
    /// The `LOSSY` ring.
    pub lossy: usize,
    /// The `DURABLE` ring, which broker answers share.
    pub durable: usize,
    /// The `LIFECYCLE` ring. Filling it closes the connection, so this is
    /// how many boundary records a consumer may fall behind by.
    pub lifecycle: usize,
}

impl Capacities {
    /// Every ring at `records`. A test's sizes; the product's come from the
    /// configuration.
    pub const fn each(records: usize) -> Self {
        Self {
            lossy: records,
            durable: records,
            lifecycle: records,
        }
    }

    /// The three together.
    pub const fn total(self) -> usize {
        self.lossy + self.durable + self.lifecycle
    }
}

/// An outbound queue, as its two ends.
///
/// # Panics
///
/// If any capacity is zero, for the reason [`Ring::split`] gives.
fn outbound<T>(capacities: Capacities) -> (Outbox<T>, Drain<T>) {
    let (lossy, from_lossy) = Ring::split(capacities.lossy);
    let (durable, from_durable) = Ring::split(capacities.durable);
    let (lifecycle, from_lifecycle) = Ring::split(capacities.lifecycle);

    (
        Outbox {
            lossy,
            durable,
            lifecycle,
        },
        Drain {
            rings: [from_lossy, from_durable, from_lifecycle],
            held: [None, None, None],
        },
    )
}

/// How the writer thread closes a connection it can no longer serve: the
/// attaching side's way to end the socket, called once, on the writer
/// thread.
///
/// A closure because this module holds no socket and is built where none
/// exists, and because nothing else reaches the thread that would have to
/// act: the drainer of a consumer that stopped reading is blocked in a
/// write. ADR 0028.
pub type Close = Box<dyn FnOnce() + Send>;

/// What the attach side tells the writer thread.
///
/// These cross a channel rather than a ring because none of them is sent from
/// the logic thread, so a channel's allocation and its lock cost nothing that
/// matters here.
enum Control<T> {
    /// A connection exists; push every record from here on into this ring,
    /// and wake the thread draining it if it sleeps. The record carried,
    /// if any, is the connection's first: it rides the attach so nothing
    /// fanned out between two control messages can be numbered ahead of it.
    Attach(
        ConnectionId,
        Outbox<T>,
        Option<Waker>,
        Option<T>,
        Option<Close>,
    ),
    /// A connection is gone; drop its ring's producer.
    Detach(ConnectionId),
    /// The broker answers this connection: push the record into its ring,
    /// numbered in its `seq`, whether or not it has authenticated.
    Answer(ConnectionId, T),
    /// The connection has authenticated under a token granting these
    /// capabilities; fan out to it from here on what they cover.
    Authenticated(ConnectionId, Capabilities),
    /// Say when the writer thread has taken every message sent before this
    /// one. Only a test asks: [`Connections::attach`] promises nothing about
    /// a record committed while the attach is in flight, and a test that
    /// counts what a connection receives has to commit after it.
    #[cfg(all(test, not(loom)))]
    Barrier(std::sync::mpsc::Sender<()>),
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
    /// A `LIFECYCLE` record the commit ring evicts before the writer thread
    /// keeps it is a loss the retained set never sees, so it is counted
    /// here, on the thread that finds out. ADR 0029.
    lifecycle_evicted: Arc<AtomicU64>,
}

impl<T> Commit<T> {
    /// Commit a record for every connection whose capabilities cover
    /// capability number `need`.
    ///
    /// A full ring evicts its oldest record, which comes back here rather than
    /// being destroyed inside the ring. Dropping it is the caller's, and on the
    /// logic thread that is a deallocation per lost record under pressure;
    /// ADR 0011 accepts that until the record type is fixed and its drop cost
    /// is known.
    pub fn push(&mut self, need: u32, class: Class, value: T) -> Push<T> {
        self.push_addressed(None, need, class, None, value)
    }

    /// Commit a `LIFECYCLE` record for every connection whose capabilities
    /// cover `need`, and keep it in `slot` of the retained set for the
    /// connections that authenticate later.
    ///
    /// Queued as [`push`](Self::push) queues; the slot rides with the
    /// record to the writer thread, which is the one that holds the set.
    pub fn push_retained(&mut self, need: u32, slot: u32, value: T) -> Push<T> {
        self.push_addressed(None, need, Class::Lifecycle, Some(slot), value)
    }

    /// Commit a record for one connection and no other.
    ///
    /// The record is queued whether or not `to` is still attached: the writer
    /// thread is the one that knows, and it drops and counts a record whose
    /// connection is gone. Nothing here looks the connection up, so the call
    /// costs the logic thread what `push` does.
    pub fn push_to(&mut self, to: ConnectionId, class: Class, value: T) -> Push<T> {
        // An addressed record is not filtered, so it needs nothing.
        self.push_addressed(Some(to), 0, class, None, value)
    }

    /// Push with an address, and hand back what the ring turned away as the
    /// record alone: where it was going is nobody's concern once it is lost.
    fn push_addressed(
        &mut self,
        to: Option<ConnectionId>,
        need: u32,
        class: Class,
        slot: Option<u32>,
        record: T,
    ) -> Push<T> {
        let pushed = match self.producer.push(Addressed {
            to,
            need,
            class,
            slot,
            record,
        }) {
            Push::Stored => Push::Stored,
            Push::Evicted(lost) => {
                if lost.slot.is_some() {
                    self.lifecycle_evicted.fetch_add(1, Ordering::Relaxed);
                }
                Push::Evicted(lost.record)
            }
            Push::Refused(lost) => Push::Refused(lost.record),
        };
        self.waker.wake_if_parked();

        pushed
    }

    /// How many records the commit ring has turned away.
    pub fn dropped(&self) -> u64 {
        self.producer.dropped()
    }

    /// How many `LIFECYCLE` records the commit ring evicted before the
    /// writer thread could keep them, `lifecycle_evicted_total`: each is a
    /// boundary the retained set never held.
    pub fn lifecycle_evicted(&self) -> u64 {
        self.lifecycle_evicted.load(Ordering::Relaxed)
    }

    /// The count behind [`lifecycle_evicted`](Self::lifecycle_evicted), for
    /// a reader that must not take the lock the committer holds this under.
    pub fn lifecycle_evicted_shared(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.lifecycle_evicted)
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
    /// Add a connection with a ring per class, sized by `capacities`, and
    /// hand back the end its socket thread drains.
    ///
    /// The rings are allocated here, on the attaching thread, so the writer
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
    /// If a capacity is zero, for the reason [`Ring::split`] gives.
    pub fn attach(&self, capacities: Capacities) -> (ConnectionId, Drain<T>) {
        self.attach_inner(capacities, None, None, None)
    }

    /// [`attach`](Self::attach), with a thread to wake and a first record.
    ///
    /// The writer thread wakes the thread through `waker` after each push
    /// into the new ring, so the thread draining the ring may park on it
    /// through the waker's flag. `first`, when given, is pushed into the
    /// ring as the writer attaches it, numbered 1, in the same control
    /// message: sent as a separate answer it could arrive after the writer
    /// had already taken the attach, found the channel empty, and fanned a
    /// record into the new ring ahead of it. The handshake is what rides
    /// here. ADR 0018.
    ///
    /// `close`, when given, is what the writer thread calls if the
    /// connection's `LIFECYCLE` ring fills: see [`Close`]. With none the
    /// writer thread forgets the connection all the same.
    pub fn attach_with(
        &self,
        capacities: Capacities,
        waker: Waker,
        first: Option<T>,
        close: Option<Close>,
    ) -> (ConnectionId, Drain<T>) {
        self.attach_inner(capacities, Some(waker), first, close)
    }

    fn attach_inner(
        &self,
        capacities: Capacities,
        waker: Option<Waker>,
        first: Option<T>,
        close: Option<Close>,
    ) -> (ConnectionId, Drain<T>) {
        let (outbox, drain) = outbound(capacities);
        let id = ConnectionId(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        self.send(Control::Attach(id, outbox, waker, first, close));

        (id, drain)
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

    /// Report a connection authenticated under a token granting `caps`.
    /// Records fanned out after the writer thread takes this reach it when
    /// the set covers them; earlier ones passed it over.
    ///
    /// Sent after the answer that says so, on the same channel, so the
    /// consumer reads its `AuthResult` before the first record.
    pub fn authenticated(&self, id: ConnectionId, caps: Capabilities) {
        self.send(Control::Authenticated(id, caps));
    }

    /// Wait until the writer thread has taken every control message sent
    /// before this call, so a record committed next is fanned out under
    /// them. The channel keeps order, which is what makes one barrier enough.
    ///
    /// A writer thread that has returned drops the barrier unanswered, and
    /// that settles too: it takes nothing further.
    #[cfg(all(test, not(loom)))]
    fn settle(&self) {
        use std::sync::mpsc::RecvTimeoutError;

        let (taken, wait) = std::sync::mpsc::channel();
        self.send(Control::Barrier(taken));
        match wait.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("the writer thread did not reach the barrier")
            }
        }
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
    counts: Arc<Counts>,
}

/// What the writer thread counts, for any thread to read.
struct Counts {
    /// Records addressed to a connection that was gone when the writer
    /// thread reached them.
    unaddressed: AtomicU64,
    /// Records withheld at fan-out from a connection whose capabilities did
    /// not cover them, one per connection per record.
    filtered: AtomicU64,
    /// Connections closed because their `LIFECYCLE` ring was full.
    lifecycle_disconnects: AtomicU64,
    /// Retained records pushed to a connection at its authentication, one
    /// per connection per record.
    replayed: AtomicU64,
    /// Records a connection's ring turned away, by what was lost.
    dropped_lossy: AtomicU64,
    dropped_durable: AtomicU64,
    dropped_broker_answer: AtomicU64,
}

/// `records_dropped_total` across every connection, by label.
///
/// The classes fill and drop independently, so a `lossy` count says
/// nothing about `durable`. A boundary record is never dropped: the
/// connection it had no room in is closed, and counted in
/// `lifecycle_disconnects_total`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Dropped {
    /// `LOSSY` records evicted by newer ones.
    pub lossy: u64,
    /// `DURABLE` records evicted by newer ones, broker answers apart.
    pub durable: u64,
    /// Broker answers and handshakes lost from the `DURABLE` ring: a
    /// `BUSY` or a `Pong` a consumer was owed and did not get.
    pub broker_answer: u64,
}

impl<T: Clone + Send + 'static> Writer<T> {
    /// Start the writer thread over a commit ring of `capacity` records,
    /// with a retained set of the specification's default 64 slots.
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
        Self::spawn_with(capacity, 64)
    }

    /// [`spawn`](Self::spawn) with a retained set of `slots` entries, which
    /// is `max_lifecycle_topics` as the bridge froze it. The set is
    /// allocated once, here, and lives on the writer thread. ADR 0029.
    #[must_use]
    pub fn spawn_with(capacity: usize, slots: usize) -> (Self, Commit<T>, Connections<T>) {
        let (producer, consumer) = Ring::split(capacity);
        let (control, inbox) = mpsc::channel();
        let flag = Arc::new(ParkFlag::new());
        let counts = Arc::new(Counts {
            unaddressed: AtomicU64::new(0),
            filtered: AtomicU64::new(0),
            lifecycle_disconnects: AtomicU64::new(0),
            replayed: AtomicU64::new(0),
            dropped_lossy: AtomicU64::new(0),
            dropped_durable: AtomicU64::new(0),
            dropped_broker_answer: AtomicU64::new(0),
        });
        let retained = Retained::new(slots);

        let sleeping = Arc::clone(&flag);
        let counting = Arc::clone(&counts);
        let handle = thread::Builder::new()
            .name("dcsbridge-writer".into())
            .spawn(move || Self::run(consumer, inbox, sleeping, retained, &counting))
            .expect("the writer thread spawns");
        let thread = handle.thread().clone();

        let waker = Waker::new(flag, thread.clone());
        let commit = Commit {
            producer,
            waker: waker.clone(),
            lifecycle_evicted: Arc::new(AtomicU64::new(0)),
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
            counts,
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
        self.counts.unaddressed.load(Ordering::Relaxed)
    }

    /// How many times a fanned-out record was withheld from a connection
    /// whose capabilities did not cover it.
    ///
    /// One per connection per record, so a record three connections may not
    /// see counts three. A withheld record is not a dropped one: it was
    /// never numbered for the connection, so the connection's `seq` shows
    /// no gap, and no drop count moves.
    pub fn filtered(&self) -> u64 {
        self.counts.filtered.load(Ordering::Relaxed)
    }

    /// How many connections were closed because their `LIFECYCLE` ring had
    /// no room for a record: `lifecycle_disconnects_total`.
    ///
    /// The record that found the ring full is not a dropped one. It went
    /// with the connection, and the consumer that reconnects is sent the
    /// retained set, which is where a boundary record lives on.
    pub fn lifecycle_disconnects(&self) -> u64 {
        self.counts.lifecycle_disconnects.load(Ordering::Relaxed)
    }

    /// How many retained records were pushed to connections at their
    /// authentication: `lifecycle_replayed_total`, one per connection per
    /// record. A record withheld by the capability filter counts in
    /// `filtered` instead.
    pub fn replayed(&self) -> u64 {
        self.counts.replayed.load(Ordering::Relaxed)
    }

    /// How many records the connections' rings have turned away, by label.
    pub fn dropped(&self) -> Dropped {
        Dropped {
            lossy: self.counts.dropped_lossy.load(Ordering::Relaxed),
            durable: self.counts.dropped_durable.load(Ordering::Relaxed),
            broker_answer: self.counts.dropped_broker_answer.load(Ordering::Relaxed),
        }
    }

    /// The writer thread's loop.
    ///
    /// Each pass takes every control message, then every record the commit ring
    /// holds. Control before records, so a detached connection stops receiving
    /// at the first pass after the detach. An empty pass yields, and only after
    /// [`LOOKS_BEFORE_PARK`] empty passes in a row does the thread park.
    ///
    /// A popped record with a slot is kept in the retained set before it is
    /// fanned out, and the set is replayed to a connection as it is reported
    /// authenticated, ahead of anything fanned out to it after. ADR 0029.
    fn run(
        mut commit: Consumer<Addressed<T>>,
        inbox: mpsc::Receiver<Control<T>>,
        flag: Arc<ParkFlag>,
        mut retained: Retained<T>,
        counts: &Counts,
    ) {
        let mut connections: Vec<Connection<T>> = Vec::new();
        let mut empty_passes = 0;

        loop {
            loop {
                match inbox.try_recv() {
                    Ok(Control::Attach(id, outbox, waker, first, close)) => {
                        let mut connection = Connection {
                            id,
                            outbox,
                            next_seq: 1,
                            waker,
                            caps: None,
                            close,
                            closed: false,
                        };
                        if let Some(first) = first {
                            // The handshake is the broker's own record, and
                            // durable as its answers are. ADR 0028.
                            connection.push(Class::Durable, true, first, counts);
                        }
                        connections.push(connection);
                    }
                    Ok(Control::Detach(id)) => connections.retain(|held| held.id != id),
                    Ok(Control::Answer(id, record)) => {
                        fan_out(
                            &mut connections,
                            Addressed {
                                to: Some(id),
                                need: 0,
                                // A broker answer is durable. ADR 0028.
                                class: Class::Durable,
                                slot: None,
                                record,
                            },
                            true,
                            counts,
                        );
                    }
                    Ok(Control::Authenticated(id, caps)) => {
                        if let Some(held) = connections.iter_mut().find(|held| held.id == id) {
                            held.caps = Some(caps);
                            retained.replay(held, caps, counts);
                        }
                        connections.retain(|held| !held.closed);
                    }
                    #[cfg(all(test, not(loom)))]
                    Ok(Control::Barrier(taken)) => {
                        let _ = taken.send(());
                    }
                    Ok(Control::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }

            // A bounded drain, so a commit ring fed as fast as it is
            // emptied cannot keep the control channel from being read: an
            // attach or an answer waits at most one batch, never a flood.
            let mut fanned = false;
            for _ in 0..RECORDS_PER_PASS {
                let Some(record) = commit.pop() else {
                    break;
                };
                if let Some(slot) = record.slot {
                    retained.keep(slot, record.need, record.record.clone());
                }
                fan_out(&mut connections, record, false, counts);
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

/// How many records the writer thread fans out before it looks at the
/// control channel again.
///
/// Large enough that a frame's burst, a few dozen records, is one pass, and
/// small enough that a connection attaching under a flood waits
/// microseconds rather than for the flood to end.
const RECORDS_PER_PASS: usize = 256;

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

/// The retained set: the latest `LIFECYCLE` record per slot, and the
/// occupied slots in emit order.
///
/// Owned by the writer thread and touched by no other, so it needs no lock.
/// A slot holds a reference to the record the rings share, not a copy, and
/// what bounds a record's size is the refusal at commit. ADR 0029.
struct Retained<T> {
    /// By slot: the capability number the record needs, and the record.
    slots: Vec<Option<(u32, T)>>,
    /// The occupied slots, oldest emit first. A replaced slot moves to the
    /// back.
    order: Vec<u32>,
}

impl<T: Clone> Retained<T> {
    /// An empty set of `slots` entries.
    fn new(slots: usize) -> Self {
        Retained {
            slots: (0..slots).map(|_| None).collect(),
            order: Vec::with_capacity(slots),
        }
    }

    /// Keep `record` as the latest of its slot, after every other slot in
    /// the order. A slot past the set is nothing the registry could have
    /// bound, since both are sized from one number, so it is not kept.
    fn keep(&mut self, slot: u32, need: u32, record: T) {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };
        if entry.is_some() {
            self.order.retain(|held| *held != slot);
        }
        *entry = Some((need, record));
        self.order.push(slot);
    }

    /// Push the set to `connection` in emit order, through the same filter
    /// fan-out applies: a record `caps` does not cover is withheld and
    /// counted as filtered, so the connection sees no gap for it. Stops at
    /// a connection its `LIFECYCLE` ring closed, since nothing more reaches
    /// it.
    fn replay(&self, connection: &mut Connection<T>, caps: Capabilities, counts: &Counts) {
        for slot in &self.order {
            if connection.closed {
                return;
            }
            let Some((need, record)) = &self.slots[*slot as usize] else {
                continue;
            };
            if !caps.covers(*need) {
                counts.filtered.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            connection.push(Class::Lifecycle, false, record.clone(), counts);
            counts.replayed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One connection, as the writer thread holds it.
struct Connection<T> {
    id: ConnectionId,
    outbox: Outbox<T>,
    /// The `seq` the next record pushed here takes.
    next_seq: u64,
    /// The thread draining the ring, if it sleeps on it.
    waker: Option<Waker>,
    /// What its token grants, once it has authenticated. A fanned-out
    /// record reaches it when the set covers the record's need; an
    /// addressed one always does.
    caps: Option<Capabilities>,
    /// How to end its socket, until that has been done.
    close: Option<Close>,
    /// Whether its `LIFECYCLE` ring has refused a record. Such a connection
    /// is forgotten when the fan-out that found it out returns.
    closed: bool,
}

impl<T> Connection<T> {
    /// Number a record and push it, whether or not the ring keeps it, then
    /// wake the drainer if it sleeps.
    ///
    /// The number is taken before the push, so an evicted record leaves the
    /// gap that tells its consumer it was lost.
    ///
    /// A `LIFECYCLE` record its ring has no room for is not a loss to
    /// count. The consumer is so far behind that it has missed mission
    /// boundaries, and a stream with one missing would tell it a world
    /// still stands that does not, so the connection is closed instead
    /// and the consumer starts again from the retained set. ADR 0009.
    fn push(&mut self, class: Class, answer: bool, record: T, counts: &Counts) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let dropped = match self.outbox.push(class, answer, Numbered { seq, record }) {
            Lost::Nothing => None,
            Lost::Lossy => Some(&counts.dropped_lossy),
            Lost::Durable => Some(&counts.dropped_durable),
            Lost::BrokerAnswer => Some(&counts.dropped_broker_answer),
            Lost::Boundary => {
                self.close(counts);
                return;
            }
        };
        if let Some(dropped) = dropped {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(waker) = &self.waker {
            waker.wake_if_parked();
        }
    }

    /// End the socket and mark the connection for forgetting.
    ///
    /// The wake is unconditional: a drainer parked on rings that will
    /// never fill again has to look once more to find its socket closed.
    fn close(&mut self, counts: &Counts) {
        counts.lifecycle_disconnects.fetch_add(1, Ordering::Relaxed);
        self.closed = true;
        if let Some(close) = self.close.take() {
            close();
        }
        if let Some(waker) = &self.waker {
            waker.wake();
        }
    }
}

/// Push one record where it is addressed: into every connection's ring, or
/// into one, numbered per connection either way.
///
/// Fanned out, each receiving connection but the last gets a clone, and the
/// last gets the record itself, so a single connection costs no clone at
/// all. A connection receives when it has authenticated under a token whose
/// capabilities cover the record's need. One that has not authenticated is
/// passed over, and one whose capabilities do not cover the record is
/// withheld from and counted in `filtered`; either way its `seq` does not
/// move, so it sees no gap: nothing it was not entitled to was ever
/// numbered for it. With no connection to receive the record it is dropped
/// and counted nowhere: there was no one to lose it.
///
/// Addressed, the one connection gets the record, authenticated or not,
/// whatever its capabilities, and no other connection's `seq` moves. A
/// connection that has detached, or a number that was never handed out, is
/// a record with nowhere to go: dropped here and counted in `unaddressed`,
/// because somebody sent it.
///
/// A record a ring turns away is dropped here, on the writer thread, and the
/// ring has already counted it against that connection. A connection whose
/// `LIFECYCLE` ring turned one away is closed by the push and forgotten
/// here, once the walk that may still be holding it is over.
fn fan_out<T: Clone>(
    connections: &mut Vec<Connection<T>>,
    addressed: Addressed<T>,
    answer: bool,
    counts: &Counts,
) {
    deliver(connections, addressed, answer, counts);
    connections.retain(|held| !held.closed);
}

/// [`fan_out`]'s pushes, which forget nobody.
fn deliver<T: Clone>(
    connections: &mut [Connection<T>],
    addressed: Addressed<T>,
    answer: bool,
    counts: &Counts,
) {
    // The slot is the retained set's concern, settled before the push.
    let Addressed {
        to,
        need,
        class,
        slot: _,
        record,
    } = addressed;

    if let Some(to) = to {
        match connections.iter_mut().find(|held| held.id == to) {
            Some(connection) => connection.push(class, answer, record, counts),
            None => {
                counts.unaddressed.fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }

    // The walk below visits every connection, so a count taken as each
    // one is passed over is a count of every connection withheld from.
    let mut receiving = connections.iter_mut().filter(|held| {
        let Some(caps) = held.caps else {
            return false;
        };
        let covered = caps.covers(need);
        if !covered {
            counts.filtered.fetch_add(1, Ordering::Relaxed);
        }
        covered
    });
    let Some(mut previous) = receiving.next() else {
        return;
    };
    for connection in receiving {
        previous.push(class, answer, record.clone(), counts);
        previous = connection;
    }
    previous.push(class, answer, record, counts);
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    /// A capacity no test here fills, so a drop means the fan-out lost a record
    /// rather than a ring evicted one.
    const ROOMY: usize = 4096;

    /// The capability number every record here needs, and the set that
    /// covers it and one more, so a token can be narrowed to `READ` alone.
    const READ: u32 = 1;
    const COMMAND: u32 = 2;
    const ALL: Capabilities = Capabilities::NONE.with(READ).with(COMMAND);

    /// Attach a connection and report it authenticated with every
    /// capability, which is the state every test here but the gating ones
    /// wants: a connection that receives what is fanned out.
    ///
    /// Returns once the writer thread holds the connection. A record
    /// committed while the attach is in flight may be fanned out before it,
    /// and a test that counts what arrives would then wait for records the
    /// connection was never sent.
    fn attached<T>(connections: &Connections<T>, capacity: usize) -> (ConnectionId, Drain<T>) {
        let (id, consumer) = connections.attach(Capacities::each(capacity));
        connections.authenticated(id, ALL);
        connections.settle();
        (id, consumer)
    }

    /// Pop until `count` records have arrived, or fail after a while rather
    /// than hang the suite. The records alone, for the tests that are not
    /// about numbering.
    fn drain_until<T>(consumer: &mut Drain<T>, count: usize) -> Vec<T> {
        drain_numbered(consumer, count)
            .into_iter()
            .map(|numbered| numbered.record)
            .collect()
    }

    /// Pop until `count` records have arrived, with their numbers.
    fn drain_numbered<T>(consumer: &mut Drain<T>, count: usize) -> Vec<Numbered<T>> {
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
        // Miri's clock counts what it interprets, and four consumers spinning
        // on an empty merge of three rings spend it fast: 64 records took 46
        // of its seconds against a deadline of 30. What Miri checks here is
        // ownership, which 16 records move through the same paths.
        let pushes: u32 = if cfg!(miri) { 16 } else { 2_000 };
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);

        let consumers: Vec<_> = (0..4)
            .map(|_| {
                let (_, mut consumer) = attached(&connections, ROOMY);
                thread::spawn(move || {
                    let arrived = drain_until(&mut consumer, pushes as usize);
                    (arrived, consumer.dropped())
                })
            })
            .collect();

        for value in 0..pushes {
            assert_eq!(
                commit.push(READ, Class::Durable, value),
                Push::Stored,
                "{value} found no room"
            );
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
        let (_, mut stalled) = attached(&connections, 4);
        let (_, mut reading) = attached(&connections, ROOMY);

        for value in 0..10u32 {
            commit.push(READ, Class::Durable, value);
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
        let (first_id, mut first) = attached(&connections, ROOMY);

        for value in 0..5u32 {
            commit.push(READ, Class::Durable, value);
        }
        assert_eq!(drain_until(&mut first, 5), (0..5).collect::<Vec<_>>());

        let (second_id, mut second) = attached(&connections, ROOMY);
        assert!(
            second_id > first_id,
            "ids did not rise: {first_id:?} then {second_id:?}"
        );

        for value in 5..10u32 {
            commit.push(READ, Class::Durable, value);
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
            commit.push(READ, Class::Durable, value);
        }
        assert_eq!(drain_until(&mut first, 5), (10..15).collect::<Vec<_>>());

        // A number is never handed out twice: a connection attached after a
        // detach takes a new one, so an answer addressed to the old
        // connection cannot reach the new.
        let (third_id, _third) = attached(&connections, ROOMY);
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
        let (first_id, mut first) = attached(&connections, ROOMY);
        let (_, mut second) = attached(&connections, ROOMY);

        commit.push(READ, Class::Durable, 0u32);
        commit.push_to(first_id, Class::Durable, 1);
        commit.push_to(first_id, Class::Durable, 2);
        commit.push(READ, Class::Durable, 3);

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

    /// A record that rides the attach is the connection's first, numbered
    /// 1, while the logic thread commits without pause: the writer takes
    /// the attach and the first record as one message, so no commit can
    /// land between them. Sent as two messages, two in a hundred attaches
    /// read a fanned-out record first.
    ///
    /// The committer is paced at a heavy load's rate rather than the
    /// thread's, and the ring is large, so the first record is read before
    /// the flood behind it can evict it: eviction is the ring's business
    /// and not what this checks.
    ///
    /// Not run under Miri: a thread committing against the clock is what
    /// the race needs, and Miri interprets a paced committer at minutes per
    /// attach. What Miri checks here is the ring's ownership, which this
    /// test moves through the same paths as the others.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn the_first_record_rides_the_attach_ahead_of_a_continuous_commit() {
        let attaches: u32 = 500;
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let stop = Arc::new(AtomicBool::new(false));

        let committing = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut n = 1_000_000u32;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..16 {
                        commit.push(READ, Class::Durable, n);
                        n = n.wrapping_add(1);
                    }
                    thread::sleep(Duration::from_micros(100));
                }
            })
        };

        for greeting in 0..attaches {
            let flag = Arc::new(ParkFlag::new());
            let waker = Waker::new(Arc::clone(&flag), thread::current());
            let (_, mut consumer) =
                connections.attach_with(Capacities::each(1 << 16), waker, Some(greeting), None);
            let first = drain_numbered(&mut consumer, 1);
            assert_eq!(
                first,
                vec![Numbered {
                    seq: 1,
                    record: greeting
                }],
                "a fanned-out record came before the attach's own"
            );
        }

        stop.store(true, Ordering::Relaxed);
        committing.join().expect("the committer only pushes");
        drop(writer);
    }

    /// A fanned-out record passes over a connection until it is reported
    /// authenticated, moving its `seq` by nothing, while an answer reaches
    /// it regardless. Once authenticated it receives from the next record
    /// with no gap: nothing it was not entitled to was numbered for it.
    #[test]
    fn fan_out_withholds_from_an_unauthenticated_connection_without_a_gap() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (pending_id, mut pending) = connections.attach(Capacities::each(ROOMY));
        let (_, mut trusted) = attached(&connections, ROOMY);

        commit.push(READ, Class::Durable, 0u32);
        connections.answer(pending_id, 1);
        assert_eq!(
            drain_numbered(&mut pending, 1),
            vec![Numbered { seq: 1, record: 1 }],
            "the unauthenticated connection received a fanned record, or a gap"
        );
        assert_eq!(
            drain_numbered(&mut trusted, 1),
            vec![Numbered { seq: 1, record: 0 }]
        );

        connections.authenticated(pending_id, ALL);
        connections.settle();
        commit.push(READ, Class::Durable, 2);
        assert_eq!(
            drain_numbered(&mut pending, 1),
            vec![Numbered { seq: 2, record: 2 }],
            "the first record after authentication did not follow the answer"
        );
        assert_eq!(
            drain_numbered(&mut trusted, 1),
            vec![Numbered { seq: 2, record: 2 }]
        );
        assert_eq!(
            writer.filtered(),
            0,
            "an unauthenticated connection was counted as filtered"
        );

        drop(writer);
    }

    /// A fanned-out record a connection's capabilities do not cover is
    /// withheld from it and counted, once per connection withheld from,
    /// while a connection whose set covers it receives it. The withheld
    /// connection's `seq` does not move, so the next record it may see
    /// follows with no gap, and an answer reaches it whatever it needs.
    #[test]
    fn fan_out_withholds_a_record_the_capabilities_do_not_cover_without_a_gap() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (reader_id, mut reader) = connections.attach(Capacities::each(ROOMY));
        connections.authenticated(reader_id, Capabilities::NONE.with(READ));
        let (_, mut trusted) = attached(&connections, ROOMY);

        // Each step is drained before the next, because an answer crosses
        // the control channel and a record the ring, and the writer thread
        // orders the two only within a pass.
        commit.push(READ, Class::Durable, 0u32);
        commit.push(COMMAND, Class::Durable, 1);
        assert_eq!(
            drain_numbered(&mut reader, 1),
            vec![Numbered { seq: 1, record: 0 }]
        );
        connections.answer(reader_id, 2);
        assert_eq!(
            drain_numbered(&mut reader, 1),
            vec![Numbered { seq: 2, record: 2 }],
            "the read-only connection saw the command record, or a gap"
        );
        commit.push(READ, Class::Durable, 3);
        assert_eq!(
            drain_numbered(&mut reader, 1),
            vec![Numbered { seq: 3, record: 3 }],
            "the record after the withheld one did not take the next number"
        );
        assert_eq!(
            drain_numbered(&mut trusted, 3),
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
                Numbered { seq: 3, record: 3 },
            ],
            "the trusted connection did not receive every record"
        );
        assert_eq!(
            writer.filtered(),
            1,
            "one connection was withheld from once"
        );
        assert_eq!(reader.dropped(), 0, "a withheld record counted as dropped");
        assert_eq!(commit.dropped(), 0, "a withheld record counted as dropped");

        drop(writer);
    }

    /// An addressed record reaches its connection whether or not the
    /// connection's capabilities cover anything, and is not counted as
    /// filtered: the filter is for fan-out alone.
    #[test]
    fn an_addressed_record_is_not_filtered() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (id, mut consumer) = connections.attach(Capacities::each(ROOMY));
        connections.authenticated(id, Capabilities::NONE);
        connections.settle();

        commit.push(READ, Class::Durable, 0u32);
        commit.push_to(id, Class::Durable, 1);
        assert_eq!(
            drain_numbered(&mut consumer, 1),
            vec![Numbered { seq: 1, record: 1 }],
            "the addressed record was withheld, or the fanned one was not"
        );
        assert_eq!(writer.filtered(), 1);

        drop(writer);
    }

    /// A retained push fans out as a plain one does: numbered in order
    /// with the records around it, into the `LIFECYCLE` ring, withheld from
    /// a connection its need is not covered by.
    #[test]
    fn a_retained_push_fans_out_as_a_plain_one_does() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (reader_id, mut reader) = connections.attach(Capacities::each(ROOMY));
        connections.authenticated(reader_id, Capabilities::NONE.with(READ));
        let (_, mut trusted) = attached(&connections, ROOMY);

        commit.push(READ, Class::Durable, 0u32);
        commit.push_retained(READ, 0, 1);
        commit.push_retained(COMMAND, 1, 2);
        commit.push(READ, Class::Durable, 3);

        assert_eq!(
            drain_numbered(&mut trusted, 4),
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
                Numbered { seq: 3, record: 2 },
                Numbered { seq: 4, record: 3 },
            ]
        );
        assert_eq!(
            drain_numbered(&mut reader, 3),
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
                Numbered { seq: 3, record: 3 },
            ],
            "the retained record the token does not cover reached it, or left a gap"
        );
        assert_eq!(writer.filtered(), 1);
        assert_eq!(writer.lifecycle_disconnects(), 0);

        drop(writer);
    }

    /// A connection authenticated after three retained pushes, two of them
    /// to one slot, receives the latest per slot in emit order, numbered
    /// from one, and a live record after them with no gap.
    #[test]
    fn the_retained_set_replays_latest_per_slot_in_emit_order() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (_, mut early) = attached(&connections, ROOMY);

        commit.push_retained(READ, 0, 10u32);
        commit.push_retained(READ, 1, 11);
        commit.push_retained(READ, 0, 12);
        assert_eq!(drain_until(&mut early, 3), vec![10, 11, 12]);

        let (_, mut late) = attached(&connections, ROOMY);
        commit.push(READ, Class::Durable, 13);
        assert_eq!(
            drain_numbered(&mut late, 3),
            vec![
                Numbered { seq: 1, record: 11 },
                Numbered { seq: 2, record: 12 },
                Numbered { seq: 3, record: 13 },
            ],
            "the replay is not the latest per slot in emit order, ahead of the live record"
        );
        assert_eq!(drain_until(&mut early, 1), vec![13]);
        assert_eq!(writer.replayed(), 2);
        assert_eq!(writer.lifecycle_disconnects(), 0);
        assert_eq!(late.dropped(), 0);

        drop(writer);
    }

    /// The replay passes the capability filter: a retained record the
    /// token does not cover is withheld and counted as filtered, the
    /// connection's `seq` does not move for it, and nothing is replayed.
    #[test]
    fn the_replay_withholds_what_the_capabilities_do_not_cover() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (_, mut early) = attached(&connections, ROOMY);
        commit.push_retained(COMMAND, 0, 0u32);
        assert_eq!(drain_until(&mut early, 1), vec![0]);

        let (reader_id, mut reader) = connections.attach(Capacities::each(ROOMY));
        connections.authenticated(reader_id, Capabilities::NONE.with(READ));
        connections.settle();
        commit.push(READ, Class::Durable, 1);

        assert_eq!(
            drain_numbered(&mut reader, 1),
            vec![Numbered { seq: 1, record: 1 }],
            "the withheld replay reached the connection, or left a gap"
        );
        assert_eq!(writer.filtered(), 1);
        assert_eq!(writer.replayed(), 0);

        drop(writer);
    }

    /// A retained record fanned out between a connection's attach and its
    /// authentication passed the connection over then, and reaches it once,
    /// by the replay.
    #[test]
    fn a_record_fanned_out_before_authentication_arrives_once_by_replay() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (pending_id, mut pending) = connections.attach(Capacities::each(ROOMY));
        connections.settle();

        commit.push_retained(READ, 0, 0u32);
        // The early connection proves the record was fanned out before
        // the authentication below is sent.
        let (_, mut early) = attached(&connections, ROOMY);
        commit.push_retained(READ, 1, 1);
        assert_eq!(drain_until(&mut early, 2), vec![0, 1]);

        connections.authenticated(pending_id, ALL);
        // The live record is committed only once the writer thread holds
        // the authentication, or a pass could pop it first and pass the
        // connection over, and a plain record is never replayed.
        connections.settle();
        commit.push(READ, Class::Durable, 2);
        assert_eq!(
            drain_numbered(&mut pending, 3),
            vec![
                Numbered { seq: 1, record: 0 },
                Numbered { seq: 2, record: 1 },
                Numbered { seq: 3, record: 2 },
            ],
            "a record was replayed twice, or missed"
        );

        drop(writer);
    }

    /// A replay of a full retained set fits a `LIFECYCLE` ring of the
    /// shipped size, and every record is counted as replayed.
    #[test]
    fn a_full_replay_fits_the_lifecycle_ring() {
        // Miri interprets every push; 16 slots move through the same paths
        // as 64.
        let slots: u32 = if cfg!(miri) { 16 } else { 64 };
        let (writer, mut commit, connections) = Writer::spawn_with(ROOMY, slots as usize);
        for slot in 0..slots {
            commit.push_retained(READ, slot, slot);
        }
        // The early connection has every retained record, by fan-out or by
        // replay, before the live one; so every push was popped and kept
        // before the late connection attaches.
        let (_, mut early) = attached(&connections, ROOMY);
        commit.push(READ, Class::Durable, u32::MAX);
        assert_eq!(
            drain_until(&mut early, slots as usize + 1).last(),
            Some(&u32::MAX)
        );

        // Whether the early connection was replayed to or fanned out to is
        // thread schedule, so the count is read as a difference.
        let before = writer.replayed();
        let (late_id, mut late) = connections.attach(Capacities {
            lossy: 4,
            durable: 4,
            lifecycle: 256,
        });
        connections.authenticated(late_id, ALL);
        let replayed = drain_numbered(&mut late, slots as usize);
        assert_eq!(
            replayed.iter().map(|n| n.record).collect::<Vec<_>>(),
            (0..slots).collect::<Vec<_>>()
        );
        assert_eq!(replayed.last().map(|n| n.seq), Some(u64::from(slots)));
        assert_eq!(writer.replayed() - before, u64::from(slots));
        assert_eq!(writer.lifecycle_disconnects(), 0);

        drop(writer);
    }

    /// A `LIFECYCLE` record the commit ring evicts before the writer thread
    /// pops it is counted as evicted, and a plain record evicted the same
    /// way is not: the count is the boundaries the retained set never
    /// held. Built over a bare ring with no writer thread, so the eviction
    /// is certain rather than a matter of schedule.
    #[test]
    fn a_lifecycle_record_the_commit_ring_evicts_is_counted() {
        let (producer, _consumer) = Ring::split(2);
        let mut commit: Commit<u32> = Commit {
            producer,
            waker: Waker::new(Arc::new(ParkFlag::new()), thread::current()),
            lifecycle_evicted: Arc::new(AtomicU64::new(0)),
        };

        assert!(matches!(commit.push_retained(READ, 0, 0), Push::Stored));
        assert!(matches!(commit.push(READ, Class::Durable, 1), Push::Stored));
        assert!(matches!(
            commit.push(READ, Class::Durable, 2),
            Push::Evicted(0)
        ));
        assert_eq!(commit.lifecycle_evicted(), 1);
        assert!(matches!(commit.push_retained(READ, 1, 3), Push::Evicted(1)));
        assert_eq!(
            commit.lifecycle_evicted(),
            1,
            "an evicted plain record counted as a lifecycle one"
        );
        assert_eq!(commit.dropped(), 2);
    }

    /// The set is a mask by number: a number added is covered, one not
    /// added is not, and a number past the mask is neither added nor
    /// covered, so a record needing it is withheld rather than disclosed.
    #[test]
    fn a_capability_set_covers_what_was_added_and_nothing_past_the_mask() {
        let set = Capabilities::NONE.with(1).with(49);
        assert!(set.covers(1));
        assert!(set.covers(49));
        assert!(!set.covers(2));
        assert!(!set.covers(0));
        assert!(!Capabilities::NONE.covers(1));

        let past = Capabilities::NONE.with(64).with(u32::MAX);
        assert_eq!(past, Capabilities::NONE);
        assert!(!past.covers(64));
        assert!(!ALL.covers(64));
    }

    /// An answer from off the logic thread is numbered in its connection's
    /// stream in order with the records committed around it, reaches no
    /// other connection, and finds the writer thread parked: a `Pong` has to
    /// go out while the logic thread commits nothing.
    #[test]
    fn an_answer_is_numbered_in_order_and_wakes_a_parked_writer() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (first_id, mut first) = attached(&connections, ROOMY);
        let (_, mut second) = attached(&connections, ROOMY);

        commit.push(READ, Class::Durable, 0u32);
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

        commit.push(READ, Class::Durable, 2);
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

    /// Wait until the writer thread has fanned out everything committed and
    /// taken every control message, so a connection nobody drains holds
    /// what it is going to hold.
    fn fanned<T>(commit: &Commit<T>, connections: &Connections<T>) {
        wait_for(|| commit.is_empty(), "the commit ring to empty");
        // The barrier is taken at the top of a pass, so it is answered
        // after the fan-out of the last record popped has returned.
        connections.settle();
    }

    /// A `LOSSY` flood at a connection nobody drains evicts `LOSSY` and
    /// nothing else: the handshake, a `DURABLE` record, a boundary record
    /// and a broker answer queued before it are all still there, in order,
    /// and the gap in `seq` is exactly the flood's lost records.
    #[test]
    fn a_lossy_flood_evicts_no_durable_no_lifecycle_and_no_answer() {
        const HANDSHAKE: u32 = 1000;
        const PONG: u32 = 3000;
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let waker = Waker::new(Arc::new(ParkFlag::new()), thread::current());
        let (id, mut stalled) =
            connections.attach_with(Capacities::each(4), waker, Some(HANDSHAKE), None);
        connections.authenticated(id, ALL);
        connections.settle();

        commit.push(READ, Class::Durable, 10);
        commit.push(READ, Class::Lifecycle, 20);
        fanned(&commit, &connections);
        connections.answer(id, PONG);
        connections.settle();
        for value in 0..100 {
            commit.push(READ, Class::Lossy, value);
        }
        commit.push(READ, Class::Lifecycle, 21);
        fanned(&commit, &connections);

        let kept = [(1, HANDSHAKE), (2, 10), (3, 20), (4, PONG)]
            .into_iter()
            .chain((101..=104).zip(96..100))
            .chain([(105, 21)])
            .map(|(seq, record)| Numbered { seq, record })
            .collect::<Vec<_>>();
        assert_eq!(drain_numbered(&mut stalled, kept.len()), kept);
        assert_eq!(stalled.pop(), None);
        assert_eq!(stalled.dropped(), 96, "something but the flood was lost");
        assert_eq!(
            writer.dropped(),
            Dropped {
                lossy: 96,
                ..Dropped::default()
            }
        );

        drop(writer);
    }

    /// A `DURABLE` flood evicts `DURABLE`, oldest first, and leaves the
    /// other two rings as they were.
    #[test]
    fn a_durable_flood_evicts_only_durable() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (_, mut stalled) = attached(&connections, 4);

        commit.push(READ, Class::Lossy, 100u32);
        commit.push(READ, Class::Lifecycle, 200);
        for value in 0..10 {
            commit.push(READ, Class::Durable, value);
        }
        fanned(&commit, &connections);

        let kept = [(1, 100), (2, 200)]
            .into_iter()
            .chain((9..=12).zip(6..10))
            .map(|(seq, record)| Numbered { seq, record })
            .collect::<Vec<_>>();
        assert_eq!(drain_numbered(&mut stalled, kept.len()), kept);
        assert_eq!(stalled.dropped(), 6);
        assert_eq!(
            writer.dropped(),
            Dropped {
                durable: 6,
                ..Dropped::default()
            }
        );

        drop(writer);
    }

    /// A broker answer evicted from the `DURABLE` ring is counted as one,
    /// apart from the `DURABLE` records around it: a consumer that was owed
    /// a `Pong` or a `BUSY` and did not get it is a different fault from
    /// one that fell behind on events.
    #[test]
    fn an_evicted_broker_answer_is_counted_apart() {
        const PONG: u32 = 3000;
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (id, mut stalled) = attached(&connections, 4);

        connections.answer(id, PONG);
        connections.settle();
        for value in 0..4 {
            commit.push(READ, Class::Durable, value);
        }
        fanned(&commit, &connections);
        assert_eq!(
            writer.dropped(),
            Dropped {
                broker_answer: 1,
                ..Dropped::default()
            },
            "the fourth record evicts the answer"
        );

        commit.push(READ, Class::Durable, 4);
        fanned(&commit, &connections);
        assert_eq!(
            writer.dropped(),
            Dropped {
                durable: 1,
                broker_answer: 1,
                ..Dropped::default()
            }
        );
        assert_eq!(drain_until(&mut stalled, 4), [1, 2, 3, 4]);

        drop(writer);
    }

    /// A connection nobody drains is closed by the boundary record its
    /// `LIFECYCLE` ring has no room for: the closure it attached with runs
    /// once, the close is counted once, and nothing fanned out afterwards
    /// reaches it. What it was sent before the close is all there, with no
    /// boundary record missing. A connection beside it that does drain
    /// receives every record and is not closed.
    #[test]
    fn a_full_lifecycle_ring_closes_the_connection_and_no_other() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let closes = Arc::new(AtomicUsize::new(0));
        let closing = Arc::clone(&closes);
        let close: Close = Box::new(move || {
            closing.fetch_add(1, Ordering::SeqCst);
        });
        let waker = Waker::new(Arc::new(ParkFlag::new()), thread::current());
        let (id, mut stalled) =
            connections.attach_with(Capacities::each(4), waker, None, Some(close));
        connections.authenticated(id, ALL);
        let (_, mut reading) = attached(&connections, ROOMY);

        for value in 0..4u32 {
            commit.push(READ, Class::Lifecycle, value);
        }
        fanned(&commit, &connections);
        assert_eq!(closes.load(Ordering::SeqCst), 0, "a ring that fits closed");
        assert_eq!(writer.lifecycle_disconnects(), 0);

        commit.push(READ, Class::Lifecycle, 4);
        commit.push(READ, Class::Lifecycle, 5);
        commit.push(READ, Class::Durable, 6);
        fanned(&commit, &connections);

        assert_eq!(closes.load(Ordering::SeqCst), 1, "the closure ran once");
        assert_eq!(writer.lifecycle_disconnects(), 1);
        assert_eq!(drain_until(&mut stalled, 4), [0, 1, 2, 3]);
        assert_eq!(stalled.pop(), None, "a closed connection was sent more");
        assert_eq!(drain_until(&mut reading, 7), [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(reading.dropped(), 0);

        // An answer to the closed connection finds it gone, as it would
        // after a detach, and the late detach its thread sends is a no-op.
        connections.answer(id, 99);
        connections.detach(id);
        connections.settle();
        assert_eq!(writer.unaddressed(), 1);

        drop(writer);
    }

    /// Only a boundary record closes a connection. A flood of either other
    /// class at full rings evicts, and the closure never runs.
    #[test]
    fn a_lossy_or_durable_flood_closes_nothing() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let closes = Arc::new(AtomicUsize::new(0));
        let closing = Arc::clone(&closes);
        let close: Close = Box::new(move || {
            closing.fetch_add(1, Ordering::SeqCst);
        });
        let waker = Waker::new(Arc::new(ParkFlag::new()), thread::current());
        let (id, stalled) = connections.attach_with(Capacities::each(4), waker, None, Some(close));
        connections.authenticated(id, ALL);
        connections.settle();

        for value in 0..50u32 {
            commit.push(READ, Class::Lossy, value);
            commit.push(READ, Class::Durable, value);
        }
        fanned(&commit, &connections);

        assert_eq!(closes.load(Ordering::SeqCst), 0);
        assert_eq!(writer.lifecycle_disconnects(), 0);
        assert_eq!(stalled.dropped(), 92);
        assert_eq!(
            writer.dropped(),
            Dropped {
                lossy: 46,
                durable: 46,
                broker_answer: 0,
            }
        );

        drop(writer);
    }

    /// A record addressed to a connection that has detached, or to a number
    /// never handed out, is dropped on the writer thread and counted, and
    /// reaches nobody else.
    #[test]
    fn an_addressed_record_to_a_missing_connection_is_counted_and_reaches_nobody() {
        let (writer, mut commit, connections) = Writer::spawn(ROOMY);
        let (gone_id, gone) = attached(&connections, ROOMY);
        let (_, mut staying) = attached(&connections, ROOMY);
        connections.detach(gone_id);
        connections.settle();
        drop(gone);

        commit.push_to(gone_id, Class::Durable, 0u32);
        commit.push_to(ConnectionId::from_raw(u64::MAX), Class::Durable, 1);
        commit.push(READ, Class::Durable, 2);

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
            commit.push(READ, Class::Durable, Counted(Arc::clone(&drops)));
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
        let (_, mut consumer) = attached(&connections, ROOMY);

        for value in 0..3u32 {
            commit.push(READ, Class::Durable, value);
        }
        assert_eq!(drain_until(&mut consumer, 3), vec![0, 1, 2]);

        drop(writer);

        assert_eq!(
            consumer.pop(),
            None,
            "a record arrived after the writer stopped"
        );
        assert_eq!(
            commit.push(READ, Class::Durable, 3),
            Push::Stored,
            "a commit after the writer stopped was refused"
        );
        assert_eq!(
            consumer.pop(),
            None,
            "a record was delivered with no writer thread"
        );

        let (_, mut orphan) = attached(&connections, ROOMY);
        assert_eq!(
            orphan.pop(),
            None,
            "an attachment after the writer stopped received"
        );
    }

    /// Push `seq` into the ring `class` names, as the writer thread would
    /// have numbered it.
    fn numbered(outbox: &mut Outbox<u64>, class: Class, seq: u64) -> Lost {
        outbox.push(class, false, Numbered { seq, record: seq })
    }

    /// Everything the drain holds, as the `seq` of each record in the
    /// order it came out.
    fn drained(drain: &mut Drain<u64>) -> Vec<u64> {
        std::iter::from_fn(|| drain.pop())
            .map(|record| record.seq)
            .collect()
    }

    /// Records spread over the three rings come out in `seq` order, whatever
    /// ring each is in.
    #[test]
    fn the_drain_merges_three_rings_on_seq() {
        let (mut outbox, mut drain) = outbound(Capacities::each(8));
        let classes = [
            Class::Durable,
            Class::Lossy,
            Class::Lossy,
            Class::Lifecycle,
            Class::Durable,
            Class::Lifecycle,
            Class::Lossy,
        ];
        for (seq, class) in (1..).zip(classes) {
            assert_eq!(numbered(&mut outbox, class, seq), Lost::Nothing);
        }

        assert!(!drain.is_empty());
        assert_eq!(drained(&mut drain), [1, 2, 3, 4, 5, 6, 7]);
        assert!(drain.is_empty());
    }

    /// A record evicted from one ring leaves a gap in the merged stream and
    /// moves nothing else: what is left still comes out in order.
    #[test]
    fn an_eviction_leaves_a_gap_and_no_reorder() {
        let (mut outbox, mut drain) = outbound(Capacities::each(2));
        assert_eq!(numbered(&mut outbox, Class::Lossy, 1), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Durable, 2), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lossy, 3), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lifecycle, 4), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lossy, 5), Lost::Lossy);

        assert_eq!(drained(&mut drain), [2, 3, 4, 5]);
        assert_eq!(drain.dropped(), 1);
    }

    /// A record the drain has popped and not yet handed on is out of its
    /// ring: a flood of that class evicts what is behind it and not it.
    /// The drain is not empty while it holds one, so its thread does not
    /// park on it.
    #[test]
    fn a_held_record_survives_a_flood_of_its_ring() {
        let (mut outbox, mut drain) = outbound(Capacities::each(2));
        assert_eq!(numbered(&mut outbox, Class::Durable, 1), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lossy, 2), Lost::Nothing);

        // Popping 1 looks at every ring, and leaves 2 held.
        assert_eq!(drain.pop().map(|record| record.seq), Some(1));
        assert!(!drain.is_empty(), "a held record read as nothing to do");

        for seq in 3..=6 {
            let _ = numbered(&mut outbox, Class::Lossy, seq);
        }
        assert_eq!(drained(&mut drain), [2, 5, 6]);
    }

    /// A full `LIFECYCLE` ring refuses the newest record and keeps what it
    /// holds, where the other two evict their oldest.
    #[test]
    fn the_lifecycle_ring_refuses_and_never_evicts() {
        let (mut outbox, mut drain) = outbound(Capacities::each(2));
        assert_eq!(numbered(&mut outbox, Class::Lifecycle, 1), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lifecycle, 2), Lost::Nothing);
        assert_eq!(numbered(&mut outbox, Class::Lifecycle, 3), Lost::Boundary);

        assert_eq!(drained(&mut drain), [1, 2]);
    }
}

/// Loom drives the wake protocol over every interleaving of a committer and a
/// writer thread that parks, which is the one part of this module a test on
/// real hardware cannot be trusted to reach.
#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// The one capability the records here need and the set that grants it.
    const READ: u32 = 1;
    const ALL: Capabilities = Capabilities::NONE.with(READ);

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
            let (id, mut consumer) = connections.attach(Capacities::each(2));
            connections.authenticated(id, ALL);

            for value in 0..2u32 {
                commit.push(READ, Class::Durable, value);
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
            let (hand, take) = mpsc::channel::<Drain<u32>>();
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
            let (id, consumer) = connections.attach_with(Capacities::each(2), waker, None, None);
            connections.authenticated(id, ALL);
            hand.send(consumer).expect("the drainer is waiting");

            for value in 0..2u32 {
                commit.push(READ, Class::Durable, value);
            }

            let arrived = drainer.join().expect("the drainer only pops");
            assert_eq!(arrived, vec![1, 2], "records arrived out of order");

            drop(writer);
        });
    }

    /// No schedule has the drain hand on `seq` 2 ahead of `seq` 1 when the
    /// two are pushed into different rings, and none leaves either in its
    /// ring with the drainer parked. The drain looks at the `LOSSY` ring
    /// first and the `LIFECYCLE` ring last, so 1 goes into the first and 2
    /// into the last: a drain that trusted one pass finds `LOSSY` empty,
    /// both pushes land, and the same pass pops 2.
    ///
    /// The pushing side is the two rings' producer directly. The writer
    /// thread's loop adds schedules and nothing to the claim, which is
    /// about two pushes in `seq` order against one merge.
    #[test]
    fn the_drain_never_hands_on_a_record_ahead_of_a_lower_seq() {
        let mut model = loom::model::Builder::new();
        model.max_branches = 100_000;
        model.preemption_bound = Some(3);

        model.check(|| {
            let (mut outbox, mut drain) = outbound::<u32>(Capacities::each(2));

            let flag = Arc::new(ParkFlag::new());
            let sleeping = Arc::clone(&flag);
            let drainer = thread::spawn(move || {
                let mut arrived = Vec::new();
                while arrived.len() < 2 {
                    match drain.pop() {
                        Some(record) => arrived.push(record.seq),
                        None => sleeping.park_unless(|| !drain.is_empty()),
                    }
                }
                arrived
            });
            let waker = Waker::new(flag, drainer.thread().clone());

            let _ = outbox.push(Class::Lossy, false, Numbered { seq: 1, record: 0 });
            waker.wake_if_parked();
            let _ = outbox.push(Class::Lifecycle, false, Numbered { seq: 2, record: 0 });
            waker.wake_if_parked();

            let arrived = drainer.join().expect("the drainer only pops");
            assert_eq!(arrived, vec![1, 2], "the merge reordered two records");
        });
    }
}
