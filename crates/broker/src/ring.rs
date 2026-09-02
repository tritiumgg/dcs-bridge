//! A fixed-size ring with one producer, one consumer, and drop-oldest.
//!
//! The ring is allocated once, at the size it is given, and never grows. A push
//! into a full ring evicts the oldest record and counts it, so a consumer that
//! stops reading costs its own records and never the producer's progress. Push
//! neither blocks nor allocates, because the thread pushing is the one running
//! the simulation.
//!
//! Eviction is what makes this more than an index pair. The oldest record is
//! where the consumer is, so a producer that evicts is a producer reaching into
//! the slot its consumer is reading. Each slot therefore carries an atomic
//! stamp naming the absolute index it holds and who owns it, and one
//! compare-exchange on that stamp is what hands a slot from one side to the
//! other. Neither side writes the other's index. ADR 0008.
//!
//! The other drop rule the build needs — turn the newest record away and leave
//! the queue alone, which is what a ring of pending commands wants — is a
//! shorter path through the same protocol rather than a policy flag: claim a
//! free slot, and give the record back if there is none. It arrives as its own
//! method when something needs it.

use std::mem::MaybeUninit;

use crate::sync::{Arc, AtomicU64, Ordering, UnsafeCell};

/// The slot holds nothing, and the producer may fill it at the index stamped.
const EMPTY: u64 = 0;
/// The slot holds the record its stamp names.
const FULL: u64 = 1;
/// The producer has taken the slot to evict what it holds.
const HELD_BY_PRODUCER: u64 = 2;
/// The consumer has taken the slot to read what it holds.
const HELD_BY_CONSUMER: u64 = 3;

/// The bits a stamp gives its state; the rest carry the absolute index.
const STATE_BITS: u32 = 2;
/// The mask those bits make.
const STATE_MASK: u64 = (1 << STATE_BITS) - 1;

/// Pack an absolute index and a state into one stamp.
const fn stamp(index: u64, state: u64) -> u64 {
    (index << STATE_BITS) | state
}

/// The state a stamp carries.
const fn state_of(stamp: u64) -> u64 {
    stamp & STATE_MASK
}

/// One record's worth of storage, and the stamp that says who owns it.
///
/// The stamp names an absolute index rather than a position, so a slot on its
/// second lap is never mistaken for the same slot on its first. That is what
/// lets a compare-exchange decide the fate of a record rather than of a
/// position: whoever wins one knows which record it won.
struct Slot<T> {
    stamp: AtomicU64,
    value: UnsafeCell<MaybeUninit<T>>,
}

/// What a push did with the record it was given.
///
/// A record the ring does not keep comes back to the caller rather than being
/// destroyed inside it. The ring knows nothing about what a record is, so a
/// caller that counts its losses by kind is the only one that can, and the
/// deallocation lands wherever the caller decides rather than always on the
/// thread that pushed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Push<T> {
    /// The ring had room and now holds the record.
    Stored,
    /// The ring was full, so the oldest record was evicted to make room, and
    /// comes back here.
    Evicted(T),
    /// The ring was full and its oldest record was already being read, so the
    /// pushed record comes back here instead.
    ///
    /// Nothing older remained to evict, and the slot a record must go in is the
    /// one the oldest record is leaving. This is the one push that turns away
    /// the newest record rather than the oldest, and it is counted the same
    /// way. The consumer is draining at that moment by definition, so the next
    /// push finds room.
    Refused(T),
}

/// The shared body of a ring: its slots, its indices and its drop count.
///
/// Reached only through [`Producer`] and [`Consumer`], which is what keeps the
/// one-producer-one-consumer rule a fact rather than a comment.
pub struct Ring<T> {
    slots: Box<[Slot<T>]>,
    write: AtomicU64,
    read: AtomicU64,
    dropped: AtomicU64,
}

// SAFETY: a slot's value is touched by exactly one thread at a time, because a
// thread reaches it only by winning the compare-exchange that stamps the slot
// with its own ownership, and it publishes the slot again only after it has
// finished. Compare-exchanges on one atomic form a single modification order
// and no two of them read the same value, so the producer's claim and the
// consumer's claim on one slot cannot both succeed. Values are moved in on one
// thread and out on the other, so sending a ring between threads sends T
// between threads and nothing more: `T: Send` is the whole requirement, and
// `T: Sync` is not, because no reference to a value in a slot is ever shared.
unsafe impl<T: Send> Send for Ring<T> {}
// SAFETY: as above. `Sync` is what makes `Arc<Ring<T>>` sendable, which is how
// the two handles reach two threads.
unsafe impl<T: Send> Sync for Ring<T> {}

impl<T> Ring<T> {
    /// Build a ring of `capacity` records and split it into its two ends.
    ///
    /// The allocation happens here and nowhere else. Nothing about the size is
    /// decided in this module: a ring is given the size its caller was
    /// configured with.
    ///
    /// # Panics
    ///
    /// If `capacity` is zero. A ring that can hold nothing would turn away
    /// every record it was ever given, which is a misconfiguration rather than
    /// a backpressure policy.
    #[must_use]
    pub fn split(capacity: usize) -> (Producer<T>, Consumer<T>) {
        assert!(capacity > 0, "a ring needs room for at least one record");

        let slots = (0..capacity)
            .map(|index| Slot {
                stamp: AtomicU64::new(stamp(index as u64, EMPTY)),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let ring = Arc::new(Self {
            slots,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });

        let producer = Producer {
            ring: Arc::clone(&ring),
            index: 0,
            cursor: 0,
        };
        let consumer = Consumer {
            ring,
            index: 0,
            cursor: 0,
        };

        (producer, consumer)
    }

    /// How many records the ring was built to hold.
    fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many records the ring holds.
    ///
    /// Exact while nothing is in flight, and an estimate while both ends are
    /// working, which is what a depth gauge needs. The published indices count
    /// every record the producer wrote and every slot the consumer left behind,
    /// so their difference runs ahead of the truth by the evictions the consumer
    /// has not yet stepped over, and it is capped at the capacity for that
    /// reason.
    fn len(&self) -> usize {
        let write = self.write.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Acquire);
        let depth = usize::try_from(write.saturating_sub(read)).unwrap_or(usize::MAX);

        depth.min(self.capacity())
    }
}

impl<T> Drop for Ring<T> {
    /// Drop the records still in the ring.
    ///
    /// Both handles are gone by the time this runs, so no slot can change under
    /// it and the stamps can be read without atomics. A slot held by either side
    /// cannot be seen here: between claiming a slot and publishing it again,
    /// each side does nothing but move a value, and a move does not unwind.
    fn drop(&mut self) {
        for slot in &self.slots {
            if state_of(slot.stamp.load(Ordering::Relaxed)) == FULL {
                // SAFETY: the stamp says the slot holds a value, and a slot is
                // stamped `FULL` only after a value is written into it. This
                // runs behind `&mut self`, so no thread is inside the ring and
                // nothing else in this loop reaches the same slot: the value is
                // dropped once.
                slot.value
                    .with_mut(|value| unsafe { (*value).assume_init_drop() });
            }
        }
    }
}

/// The end of a ring that pushes, and the only thread that may.
///
/// `push` takes `&mut self`, so a second producer is a compile error rather
/// than a comment asking for one producer.
pub struct Producer<T> {
    ring: Arc<Ring<T>>,
    index: u64,
    cursor: usize,
}

impl<T> Producer<T> {
    /// Push a record, evicting the oldest if the ring is full.
    ///
    /// Never blocks, never allocates, and never waits on the consumer. A full
    /// ring costs one compare-exchange, and every way that can go leaves the
    /// producer further along than it started.
    pub fn push(&mut self, value: T) -> Push<T> {
        let slot = &self.ring.slots[self.cursor];
        let free = stamp(self.index, EMPTY);
        let mut reloaded = false;

        loop {
            let current = slot.stamp.load(Ordering::Acquire);

            if current == free {
                // SAFETY: the stamp says the slot is empty and ready for this
                // index. The consumer stamps a slot empty only after it has
                // moved the old value out, and the acquire load above pairs
                // with that release, so nothing is overwritten here. No other
                // thread writes a slot stamped for the producer's index.
                slot.value.with_mut(|slot| unsafe { (*slot).write(value) });
                slot.stamp.store(stamp(self.index, FULL), Ordering::Release);
                self.advance();

                return Push::Stored;
            }

            match state_of(current) {
                // The ring is full and this slot holds the oldest record. A
                // slot the producer has not yet reached on this lap can only
                // hold the record from one capacity ago.
                FULL => {
                    let oldest = self.index - self.ring.capacity() as u64;
                    debug_assert_eq!(
                        current,
                        stamp(oldest, FULL),
                        "a full slot held something other than the record one lap back"
                    );

                    if slot
                        .stamp
                        .compare_exchange(
                            stamp(oldest, FULL),
                            stamp(oldest, HELD_BY_PRODUCER),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        // SAFETY: winning the exchange makes this thread the
                        // slot's only owner until it publishes a new stamp, and
                        // the consumer's claim on the same slot cannot also
                        // have won. The stamp taken said `FULL`, so the value
                        // is initialized and this is the only move of it.
                        let evicted = slot
                            .value
                            .with_mut(|slot| unsafe { (*slot).assume_init_read() });
                        // SAFETY: the same ownership. The slot is uninitialized
                        // after the move above, so writing it drops nothing.
                        slot.value.with_mut(|slot| unsafe { (*slot).write(value) });
                        slot.stamp.store(stamp(self.index, FULL), Ordering::Release);
                        self.ring.dropped.fetch_add(1, Ordering::Relaxed);
                        self.advance();

                        return Push::Evicted(evicted);
                    }
                }
                // The consumer is reading the oldest record, so it is already
                // on its way out and there is nothing older to evict. The slot
                // it is leaving is the only one this record could go in. Look
                // once more in case it has finished, then give the record back
                // rather than wait on a thread that owes us nothing.
                HELD_BY_CONSUMER => {
                    if reloaded {
                        self.ring.dropped.fetch_add(1, Ordering::Relaxed);

                        return Push::Refused(value);
                    }

                    reloaded = true;
                }
                _ => unreachable!("a slot the producer does not hold is stamped by its owner"),
            }
        }
    }

    /// How many records the ring has turned away.
    ///
    /// Counts an eviction and a refusal alike: both are a record that went into
    /// the ring's care and did not come out of the far end.
    pub fn dropped(&self) -> u64 {
        self.ring.dropped.load(Ordering::Relaxed)
    }

    /// How many records the ring was built to hold.
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }

    /// How many records the ring holds.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether the ring holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Move to the next slot and publish the index reached.
    fn advance(&mut self) {
        self.index += 1;
        self.cursor += 1;
        if self.cursor == self.ring.capacity() {
            self.cursor = 0;
        }
        self.ring.write.store(self.index, Ordering::Release);
    }
}

/// The end of a ring that pops, and the only thread that may.
pub struct Consumer<T> {
    ring: Arc<Ring<T>>,
    index: u64,
    cursor: usize,
}

impl<T> Consumer<T> {
    /// Take the oldest record, or nothing if the ring is empty.
    ///
    /// Steps over any record the producer evicted while this end was elsewhere.
    /// Those are counted where they happened, so stepping over one counts
    /// nothing here.
    ///
    /// Nothing is returned while the producer is inside the slot this would
    /// have read, so an empty answer means nothing right now rather than
    /// nothing ever.
    pub fn pop(&mut self) -> Option<T> {
        loop {
            let slot = &self.ring.slots[self.cursor];
            let current = slot.stamp.load(Ordering::Acquire);

            if current == stamp(self.index, EMPTY) {
                return None;
            }

            // The exchange names the index as well as the state, so winning it
            // proves the slot still holds the record this end is owed. A slot
            // the producer has refilled fails the exchange rather than handing
            // back the newest record in the oldest one's place.
            if current == stamp(self.index, FULL)
                && slot
                    .stamp
                    .compare_exchange(
                        current,
                        stamp(self.index, HELD_BY_CONSUMER),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                // SAFETY: winning the exchange makes this thread the slot's
                // only owner until it publishes a new stamp, and the producer's
                // eviction of the same slot cannot also have won. The stamp
                // taken said `FULL`, so the producer's release of that stamp is
                // visible here, the value is initialized, and this is the only
                // move of it.
                let value = slot
                    .value
                    .with_mut(|slot| unsafe { (*slot).assume_init_read() });
                let next = self.index + self.ring.capacity() as u64;
                slot.stamp.store(stamp(next, EMPTY), Ordering::Release);
                self.advance();

                return Some(value);
            }

            // Either the producer evicted this record or it is evicting it now.
            // Either way it is gone, and the next slot is where the ring's
            // oldest record can be.
            self.advance();
        }
    }

    /// How many records the ring has turned away.
    pub fn dropped(&self) -> u64 {
        self.ring.dropped.load(Ordering::Relaxed)
    }

    /// How many records the ring was built to hold.
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }

    /// How many records the ring holds.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether the ring holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Move to the next slot and publish the index reached.
    fn advance(&mut self) {
        self.index += 1;
        self.cursor += 1;
        if self.cursor == self.ring.capacity() {
            self.cursor = 0;
        }
        self.ring.read.store(self.index, Ordering::Release);
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// A record that says when it is dropped, so a test can account for every
    /// value it handed to a ring.
    struct Counted {
        value: u32,
        drops: Arc<AtomicUsize>,
    }

    impl Counted {
        fn new(value: u32, drops: &Arc<AtomicUsize>) -> Self {
            Self {
                value,
                drops: Arc::clone(drops),
            }
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The done-when's first two thirds: a ring takes exactly the records it
    /// was sized for, and gives them back oldest first.
    #[test]
    fn fills_to_capacity_and_drains_in_order() {
        let (mut producer, mut consumer) = Ring::split(4);

        for value in 0..4 {
            assert_eq!(producer.push(value), Push::Stored, "{value} found no room");
        }
        assert_eq!(producer.len(), 4, "a filled ring reports the wrong depth");

        for value in 0..4 {
            assert_eq!(
                consumer.pop(),
                Some(value),
                "the ring reordered its records"
            );
        }
        assert_eq!(
            consumer.pop(),
            None,
            "a drained ring gave up a fifth record"
        );
        assert_eq!(
            producer.dropped(),
            0,
            "a ring that never filled turned something away"
        );
    }

    /// The done-when's last third, and the drop policy itself: the oldest
    /// records go, the newest stay, and the count says how many went.
    #[test]
    fn overflow_evicts_the_oldest_and_counts_it() {
        let (mut producer, mut consumer) = Ring::split(4);

        for value in 0..4 {
            assert_eq!(producer.push(value), Push::Stored, "{value} found no room");
        }
        for value in 4..7 {
            assert_eq!(
                producer.push(value),
                Push::Evicted(value - 4),
                "{value} evicted the wrong record"
            );
        }

        assert_eq!(
            producer.dropped(),
            3,
            "three evictions were not counted as three"
        );
        assert_eq!(producer.len(), 4, "the ring grew past its capacity");

        let survivors: Vec<u32> = std::iter::from_fn(|| consumer.pop()).collect();
        assert_eq!(
            survivors,
            vec![3, 4, 5, 6],
            "the wrong records survived a flood"
        );
    }

    /// A ring is a ring: the indices keep rising while the slots are reused, and
    /// the second lap has to behave like the first.
    ///
    /// The capacity is three so the wrap lands somewhere other than a power of
    /// two, which is what the slot arithmetic would quietly assume if it ever
    /// grew a mask.
    #[test]
    fn wraps_without_losing_order() {
        let (mut producer, mut consumer) = Ring::split(3);
        let mut popped = Vec::new();

        for value in 0..10 {
            producer.push(value);
            if value % 2 == 1 {
                popped.push(consumer.pop().expect("a push preceded this pop"));
            }
        }
        while let Some(value) = consumer.pop() {
            popped.push(value);
        }

        assert!(
            popped.windows(2).all(|pair| pair[0] < pair[1]),
            "records came out of order across a wrap: {popped:?}"
        );
        assert_eq!(
            popped.len() as u64 + producer.dropped(),
            10,
            "records neither arrived nor were counted as turned away"
        );
    }

    /// The smallest ring there is, which is also the one where every push after
    /// the first evicts and there is no other slot to reach for.
    #[test]
    fn a_ring_of_one_holds_the_newest_record() {
        let (mut producer, mut consumer) = Ring::split(1);

        assert_eq!(
            producer.push(1),
            Push::Stored,
            "the first record found no room"
        );
        assert_eq!(
            producer.push(2),
            Push::Evicted(1),
            "the second record evicted the wrong one"
        );
        assert_eq!(consumer.pop(), Some(2), "the ring kept the older record");
        assert_eq!(producer.dropped(), 1, "the eviction was not counted");
    }

    /// A ring with no slots has no policy to express, so it is refused where it
    /// is built rather than at every push for the life of the process.
    #[test]
    #[should_panic(expected = "a ring needs room for at least one record")]
    fn a_ring_of_nothing_is_refused() {
        let _ = Ring::<u32>::split(0);
    }

    /// A record the ring turns away goes back to the caller, so an eviction
    /// costs the ring nothing and the caller decides what a lost record means.
    #[test]
    fn an_evicted_record_comes_back_whole() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (mut producer, _consumer) = Ring::split(2);

        producer.push(Counted::new(1, &drops));
        producer.push(Counted::new(2, &drops));

        let Push::Evicted(evicted) = producer.push(Counted::new(3, &drops)) else {
            panic!("a push into a full ring did not evict");
        };

        assert_eq!(evicted.value, 1, "the ring evicted the wrong record");
        assert_eq!(
            drops.load(Ordering::Relaxed),
            0,
            "the ring dropped a record it had handed back"
        );
    }

    /// The ring owns what it holds, so every record it takes is dropped exactly
    /// once: where its caller lets go of it, or with the ring itself.
    ///
    /// This is the assertion that catches a leak in an eviction and a double
    /// drop in a hand-off, neither of which any ordering test would notice.
    #[test]
    fn every_record_is_dropped_exactly_once() {
        let drops = Arc::new(AtomicUsize::new(0));

        {
            let (mut producer, mut consumer) = Ring::split(4);
            for value in 0..7 {
                // The three evicted records are dropped here, as they come back.
                let _ = producer.push(Counted::new(value, &drops));
            }

            assert_eq!(
                drops.load(Ordering::Relaxed),
                3,
                "the three evicted records were not dropped when they came back"
            );

            let held = consumer.pop().expect("the ring holds four records");
            assert_eq!(held.value, 3, "the wrong record survived");
            drop(held);

            assert_eq!(
                drops.load(Ordering::Relaxed),
                4,
                "a record its reader let go of was not dropped"
            );
        }

        assert_eq!(
            drops.load(Ordering::Relaxed),
            7,
            "the ring did not drop the three records it still held"
        );
    }

    /// Two threads, and the only claims that hold under every schedule: no
    /// record is invented, duplicated, reordered or quietly lost.
    ///
    /// Nothing here asserts how much was dropped or which records went. That
    /// depends on how the two threads interleave, and a test that pinned it
    /// would be asserting the scheduler. What every schedule owes is that each
    /// record either came out of the far end or came back to the pusher, once,
    /// and that what came out came out in order.
    #[test]
    fn two_threads_account_for_every_record() {
        let pushes: u32 = if cfg!(miri) { 512 } else { 200_000 };
        let (mut producer, mut consumer) = Ring::split(64);
        let done = Arc::new(AtomicBool::new(false));
        let pushing = Arc::clone(&done);

        let pusher = std::thread::spawn(move || {
            let mut returned = Vec::new();
            for value in 0..pushes {
                match producer.push(value) {
                    Push::Stored => {}
                    Push::Evicted(record) | Push::Refused(record) => returned.push(record),
                }
            }
            pushing.store(true, Ordering::Release);

            (producer.dropped(), returned)
        });

        let mut arrived = Vec::new();
        loop {
            if let Some(record) = consumer.pop() {
                arrived.push(record);
                continue;
            }

            // An empty ring is not a finished one, and the two have to be read
            // in this order. Finding the ring empty says nothing about pushes
            // that had not happened yet at the moment it was read, so the flag
            // is what settles it — and the flag is stored after the last push,
            // so once it reads true every push is visible here and a ring that
            // drains to empty is a ring that is done.
            if done.load(Ordering::Acquire) {
                while let Some(record) = consumer.pop() {
                    arrived.push(record);
                }

                break;
            }
        }

        let (dropped, returned) = pusher.join().expect("the pushing thread only pushes");

        assert!(
            arrived.windows(2).all(|pair| pair[0] < pair[1]),
            "records arrived out of order or twice"
        );
        assert_eq!(
            dropped as usize,
            returned.len(),
            "the drop counter disagrees with what came back to the pusher"
        );

        let mut accounted = vec![false; pushes as usize];
        for record in arrived.iter().chain(returned.iter()) {
            let seen = &mut accounted[*record as usize];
            assert!(!*seen, "{record} was accounted for twice");
            *seen = true;
        }
        assert!(
            accounted.iter().all(|seen| *seen),
            "a record neither arrived nor came back"
        );
    }
}

/// Loom drives the same protocol over a ring small enough to enumerate, so the
/// ownership rules are checked against every interleaving rather than against
/// whatever the CI hosts happen to schedule.
#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// Every interleaving of a producer that evicts and a consumer that drains
    /// leaves the records in order, with no slot reached by both ends at once.
    ///
    /// The ring holds two and takes three records, so the producer must evict
    /// while the consumer is somewhere in the ring — which is the case the
    /// per-slot stamp exists for, and the one no test on real hardware can be
    /// trusted to reach.
    #[test]
    fn a_producer_that_evicts_never_races_its_consumer() {
        loom::model(|| {
            let (mut producer, mut consumer) = Ring::split(2);

            let pushing = loom::thread::spawn(move || {
                for value in 0..3u32 {
                    producer.push(value);
                }
            });

            let mut arrived = Vec::new();
            while let Some(record) = consumer.pop() {
                arrived.push(record);
            }
            pushing.join().expect("the pushing thread only pushes");
            while let Some(record) = consumer.pop() {
                arrived.push(record);
            }

            assert!(
                arrived.windows(2).all(|pair| pair[0] < pair[1]),
                "records arrived out of order or twice: {arrived:?}"
            );
        });
    }
}
