//! Logic-thread cost per commit, against the number of consumers.
//!
//! The claim under test is that a connection costs the logic thread nothing
//! per record: the writer thread pays for each extra ring, and the committing
//! thread pays the same whether zero or eight are attached. This times the
//! committing thread alone, once per consumer count, and prints a table. It
//! asserts nothing, because a figure from a shared machine is a reading rather
//! than a verdict, and the column that should stay flat is the maintainer's to
//! read.
//!
//! Three regimes. **Bursts** is the one the claim is about: one frame's worth
//! of records back to back, then a frame of quiet, which is how the drain
//! delivers them. The writer thread keeps up, the ring stays near empty, and
//! the figure is a push, a fence, a flag load, and one wake per burst spread
//! over the burst. **Flood** pushes as fast as the thread can, which no
//! drain does; the writer thread falls behind past a few consumers, the ring
//! runs full, and the figure rises with the contention of evicting against the
//! slots the writer is reading. It is here so that the reader can see where
//! the bursts figure would go if the drain were ever that fast. **Starved**
//! is the flood into a commit ring of sixteen, so that nearly every push takes
//! the eviction path and the cost of that path is on record.
//!
//! The record is a `u64`, so the figure is the fan-out's and not the record's.
//! A real record's clone and drop cost belong to the task that fixes its type.
//!
//! Run it in release, on a quiet machine: `mise run bench-fanout`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use dcsbridge_broker::fanout::Writer;
use dcsbridge_broker::ring::{Push, Ring};

/// The record. Small and owned, so nothing about it is shared across threads.
type Record = u64;

/// The consumer counts the table has a row for.
const CONSUMERS: [usize; 5] = [0, 1, 2, 4, 8];

/// A ring per connection sized as the configuration defaults size it.
const CONNECTION_CAPACITY: usize = 4096;

/// Pushes per flood row: enough that the timer's own cost disappears.
const FLOOD_PUSHES: u32 = 2_000_000;

/// Records per burst: a heavy load's 3,040 records per second at 70 frames.
const BURST_RECORDS: u32 = 43;

/// Bursts per row.
const BURSTS: u32 = 200;

/// The quiet between bursts: one frame at 70 Hz.
const FRAME: Duration = Duration::from_micros(14_300);

/// How long a drainer sleeps on an empty ring.
const DRAINER_NAP: Duration = Duration::from_micros(100);

fn main() {
    // One pass discarded, so the first row is not also paying for the first
    // thread spawn and the first page faults.
    let _ = measure(1, CONNECTION_CAPACITY, Regime::Flood);

    println!(
        "bursts: {BURST_RECORDS} records back to back, then {} ms quiet, {BURSTS} times",
        FRAME.as_millis()
    );
    print_table(CONNECTION_CAPACITY, Regime::Bursts);

    println!();
    println!("flood: {FLOOD_PUSHES} pushes as fast as the thread can; a bare ring is the floor");
    let floor = ring_alone(CONNECTION_CAPACITY);
    print_row("ring", &floor);
    print_table(CONNECTION_CAPACITY, Regime::Flood);

    println!();
    println!("starved: the flood into a commit ring of 16, so nearly every push evicts");
    print_table(16, Regime::Flood);
}

/// How the committing thread paces itself.
#[derive(Clone, Copy)]
enum Regime {
    Bursts,
    Flood,
}

fn print_table(commit_capacity: usize, regime: Regime) {
    println!(
        "{:>9}  {:>10}  {:>10}  {:>10}  {:>10}",
        "consumers", "ns/push", "ns/first", "evicted", "refused"
    );
    for consumers in CONSUMERS {
        let row = measure(consumers, commit_capacity, regime);
        print_row(&consumers.to_string(), &row);
    }
}

fn print_row(label: &str, row: &Row) {
    let first = match row.ns_first_push {
        Some(first) => format!("{first:>10.1}"),
        None => format!("{:>10}", "-"),
    };
    println!(
        "{label:>9}  {:>10.1}  {first}  {:>10}  {:>10}",
        row.ns_per_push, row.evicted, row.refused
    );
}

/// One measurement's result.
struct Row {
    /// Time per push, over every push but a burst's first.
    ns_per_push: f64,
    /// Time for a burst's first push, which finds the writer parked and pays
    /// the wake. Bursts only.
    ns_first_push: Option<f64>,
    evicted: u64,
    refused: u64,
}

/// Push through `push`, paced by `regime`, timing only the pushes.
fn time_pushes(regime: Regime, mut push: impl FnMut(Record) -> Push<Record>) -> Row {
    let mut evicted = 0;
    let mut refused = 0;

    let mut account = |result: Push<Record>| match result {
        Push::Stored => {}
        Push::Evicted(_) => evicted += 1,
        Push::Refused(_) => refused += 1,
    };

    let (ns_per_push, ns_first_push) = match regime {
        Regime::Flood => {
            let start = Instant::now();
            for value in 0..FLOOD_PUSHES {
                account(push(Record::from(value)));
            }
            let per_push = start.elapsed().as_nanos() as f64 / f64::from(FLOOD_PUSHES);

            (per_push, None)
        }
        Regime::Bursts => {
            let mut in_first = Duration::ZERO;
            let mut in_rest = Duration::ZERO;

            for _ in 0..BURSTS {
                let start = Instant::now();
                account(push(0));
                in_first += start.elapsed();

                let start = Instant::now();
                for value in 1..BURST_RECORDS {
                    account(push(Record::from(value)));
                }
                in_rest += start.elapsed();

                thread::sleep(FRAME);
            }

            let rest = in_rest.as_nanos() as f64 / f64::from(BURSTS * (BURST_RECORDS - 1));
            let first = in_first.as_nanos() as f64 / f64::from(BURSTS);

            (rest, Some(first))
        }
    };

    Row {
        ns_per_push,
        ns_first_push,
        evicted,
        refused,
    }
}

/// A bare ring with one thread popping: what the commit costs before the
/// writer thread, the fence and the flag.
fn ring_alone(capacity: usize) -> Row {
    let (mut producer, mut consumer) = Ring::<Record>::split(capacity);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let popper = thread::spawn(move || {
        loop {
            if consumer.pop().is_some() {
                continue;
            }
            if stopping.load(Ordering::Acquire) {
                break;
            }
            thread::yield_now();
        }
    });

    let row = time_pushes(Regime::Flood, |record| producer.push(record));

    stop.store(true, Ordering::Release);
    popper.join().expect("the popper only pops");

    row
}

/// Commits with `consumers` connections attached and draining.
fn measure(consumers: usize, commit_capacity: usize, regime: Regime) -> Row {
    let (writer, mut commit, connections) = Writer::<Record>::spawn(commit_capacity);
    let stop = Arc::new(AtomicBool::new(false));

    let drainers: Vec<_> = (0..consumers)
        .map(|_| {
            let (id, mut consumer) = connections.attach(CONNECTION_CAPACITY);
            connections.authenticated(id);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                loop {
                    if consumer.pop().is_some() {
                        continue;
                    }
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    // A sleep rather than a yield: eight drainers spinning on
                    // their rings would take the cores the writer thread needs,
                    // and the figure would then be the scheduler's. A socket
                    // thread blocks on its socket, and a sleep is the nearest
                    // thing to that here.
                    thread::sleep(DRAINER_NAP);
                }
            })
        })
        .collect();

    let row = time_pushes(regime, |record| commit.push(record));

    drop(writer);
    stop.store(true, Ordering::Release);
    for drainer in drainers {
        drainer.join().expect("a drainer only pops");
    }

    row
}
