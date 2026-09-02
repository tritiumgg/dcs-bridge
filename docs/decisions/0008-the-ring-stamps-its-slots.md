# ADR 0008: The ring stamps every slot, so the producer can evict

## Status

Accepted

## Context

Every record the broker carries crosses a ring. SPEC 5.2 fixes what a ring is:

> **Single producer, single consumer.** Every ring, inbound and outbound, has
> one atomic write index and one atomic read index — which is why a broker
> answer is handed to the writer thread rather than pushed by its producer. Use
> no mutex. A lock would let the logic thread block behind a slow socket.

It fixes what a full ring does, in both directions:

> **Backpressure.** Allocate every ring once, at the first `shim.configure`,
> from the size it carried [...] When an inbound ring is full, drop the newest:
> a queued command is not stale. Never block the logic thread. Never allocate.

Outbound, the rule is the other way round — evict the oldest — and SPEC 5.2
puts that work on the ring's producer rather than on its consumer:

> **Only the writer thread pushes to an outbound ring.** [...] The writer
> thread then does all three things that must agree with each other: it assigns
> `seq`, it applies the drop rule, and it pushes.

Those three sentences do not fit together on two indices alone. Drop-oldest
destroys the value at the read index, and the read index is where the consumer
is. A producer that evicts is a producer that writes the slot its consumer is
reading, and nothing in a write index and a read index can stop it.

Every repair that stays inside the two indices fails somewhere:

- The producer advances the read index after evicting. The read index then has
  two writers. A plain store clobbers a consumer that moved on and loses a
  record with no drop counted; an add races on its base and skips a live one; a
  compare-exchange loop is correct and is a spin on the logic thread, which is
  the hazard the no-mutex rule exists to prevent.
- The producer overwrites and the consumer notices it was lapped. This is sound
  with exactly two indices, and it is the cheapest producer available. But the
  producer overwrites the old value without dropping it, and a consumer cannot
  report what it never saw — SPEC 12 counts `records_dropped_total` by class,
  and task 2.18 has the producer skip `LIFECYCLE` records when it chooses a
  victim. Neither is expressible by a consumer reading the wreckage.

SPEC 5.2 names the shape of the problem while arguing a different point, that a
second producer would be too expensive:

> A second producer on the outbound ring would need a published-commit index, a
> `seq` counter ordered against the ring claim rather than against itself, and
> an eviction rule that is a read-modify-write of interior slots while another
> producer appends.

One producer removes two of those three. The read-modify-write of interior
slots remains, because eviction is that read-modify-write, and the thread it
races is the consumer.

## Decision

Every slot carries one atomic stamp beside its value, packing the absolute
index the slot holds and a state in the low bits: empty, full, held by the
producer, or held by the consumer. A single compare-exchange on that stamp
takes exclusive ownership of one slot, so the producer evicts a record the
consumer cannot be reading, and the consumer reads a record the producer cannot
be overwriting.

Indices are monotonic and reduced only to select a slot, so a stamp says which
generation of a slot it describes and a lap cannot be mistaken for an arrival.
Each side keeps its own cursor and advances it by one against a wrap
comparison, so no division sits on the push path and a capacity need not be a
power of two.

Neither side writes the other's index. The producer decides a slot's fate from
the slot's own stamp and never reads the read index; the consumer never reads
the write index. The two atomic indices SPEC 5.2 names stay exactly as
described, and they feed the depth gauges rather than the algorithm.

Eviction is one compare-exchange and every outcome makes progress. Winning it,
the producer moves the evicted record out, writes the new one, and republishes
the slot. Losing it means the consumer claimed that record first, so the oldest
record is already on its way out and there is nothing older left to evict —
and the slot it is leaving is the only one the new record could go in. The
producer
reloads the stamp once, takes the slot if the consumer has finished with it, and
otherwise turns the pushed record away and counts that instead. One reload,
never a loop.

A record the ring does not keep goes back to the caller rather than being
destroyed inside it. The ring is blind to what a record is, so the caller is the
only one that can count a loss against the record's class, which is what the
class-aware rule at task 2.18 has to do. It also means the only code between
claiming a slot and republishing it is a move, so neither side can unwind while
it holds a slot and no slot can be stranded.

The alternatives, and the line that rejected each:

- **A slot state with no index in it.** A state alone says a slot is full but
  not which generation filled it, so a compare-exchange can win against a slot
  that was refilled after the decision to evict it was made.
- **A mutex around each ring.** Refused by SPEC 5.2, and for the stated reason:
  the logic thread would queue behind a slow socket.
- **`crossbeam-queue`'s `ArrayQueue::force_push`.** Vetted where this code
  cannot be, and it evicts the head unconditionally, which is the one thing
  task 2.18 says not to do. It is also multi-producer, so it charges for a
  second producer that SPEC 5.2 spends a page ruling out.
- **Defer the atomics and land a single-threaded ring.** The done-when is a
  single-threaded test, so this would pass. It defers the only hard part of the
  task to one whose own subject is a thread.

## Consequences

This is the first `unsafe` in the broker. It is `unsafe` for a performance
argument rather than because an FFI boundary demands it, which is a lower bar
than `crates/lua-module` clears, and it is why the verification is part of the
work rather than a follow-up.

A test on three hosts cannot establish that a lock-free structure is correct.
Two of the three runners are x86-64, where a missing acquire-release pair is
unobservable in principle, and x86-64 Windows is the only target that ships, so
the host most able to expose an ordering fault is the host that never runs the
product. Loom enumerates the interleavings instead, and Miri reads the unsafe
blocks for undefined behavior. Neither reaches the shipped artifact: Loom is a
dev-dependency behind a `cfg` no shipped build sets, and nightly lives in one CI
job.

Every stamp and index operation is `SeqCst`, which is stronger than the argument
beside it needs. Loom checks which thread may touch a slot and cannot tell a
`SeqCst` publish from a `Relaxed` one, so the strength of these labels is the
one part of the design no tool here checks. Of the two mistakes available, too
strong is slower and a measurement finds it, while too weak is memory corruption
inside DCS on hardware that cannot reproduce it. Nothing has measured this path:
the put-call crossing cost is **[PROBE-3]** at task 2.6 and the rings are
**[PROBE-7]** at 9.7. Until one of them reports, the ring takes the mistake a
measurement can find. Relax an operation when a probe prices it, and name the
measurement in the commit.

Loom establishes less here than it does for most lock-free code. Slots change
hands through read-modify-writes, and Loom gives one of those more causality
than the memory model promises, so the model passes with the publishing store
written `Relaxed`. What it does check is the ownership protocol: over every
interleaving, no schedule lets both ends into one slot and none reorders what
comes out. The acquire-release pairings rest on the argument written beside each
of them and on what Miri catches, which is worth knowing before trusting a green
run.

The evicted record comes back to whoever pushed, so a caller on the logic thread
that simply drops it has put a deallocation on the frame budget, once per lost
record, exactly when the process is already under pressure. The ring no longer
decides that; the caller does, and task 2.4 is where the producer's cost is
measured.

A record can be turned away for being newest rather than oldest, in the window
where the consumer holds the oldest slot and the ring is otherwise full. The
producer cannot evict its way out of that: the slot it must write is the one the
consumer is emptying, and freeing any other slot does not free that one. The
loss is counted either way and the window is one value's move. It stops being
acceptable at task 2.18, where a `LIFECYCLE` record must not be the one turned
away, and the reserve that task adds is what closes it.

The two ends are handles that take `&mut self`, so a second producer is a
compile error. That fits the rings whose ends both belong to threads the broker
spawns. It does not obviously fit an inbound ring, whose consumer is the logic
thread reaching through the process-global bridge whenever Lua polls: a shared
reference cannot hand out `&mut`, and neither a lock on that path nor an
`UnsafeCell` in the state module is attractive. Task 2.12 is where that lands,
and the answer may be `&self` methods on the ring itself, with the single-caller
rule documented rather than compiled. The protocol does not change either
way — only where the cursors live.

This reopens at task 2.18. Evicting the oldest record that is not `LIFECYCLE`
turns a decision about one slot into a search across several, and the
alternative is a ring per class, where the rule costs nothing and
`ring_out_lifecycle_reserve` becomes an allocation rather than a comparison.
That trade buys O(1) eviction and pays for it by reordering records across
classes, which per-connection `seq` may not allow.
