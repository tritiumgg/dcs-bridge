# ADR 0007: One bridge per process, and a second open shares it

## Status

Accepted

## Context

Both DCS Lua states load the broker, so `luaopen_dcsbridge` runs more than once
in one process. SPEC 5.1.1 says what that has to produce:

> If both states load the broker, `luaopen_*` runs twice and each state gets its
> own Lua table. Make the rings, the sockets, the threads, and the class, route
> and capability maps process-global. The maps have two registrars — the hook
> driver and the sim driver, in different states (Section 5.1) — and a per-table
> map means the two registrars never see each other.

That fixes the goal and leaves the mechanics open. The specification says
nothing about what the second call does: there is no idempotence rule, no error,
and no ordering guarantee. Nor is the second open a tidy repeat of the first.
SPEC 5.1.2:

> Where the sim driver can reach the broker without the hook driver, it writes
> into the broker directly. Under Route B it always can, because the sim driver
> captured `package.loadlib` at bootstrap.

So the second open can arrive from the mission-scripting state, at a mission
load, after the hook driver has already configured the broker and its threads
are running. Whatever the second call does, it must not disturb any of that.

The one boundary the specification does draw is on allocation. SPEC 5.1:

> **`configure` comes first.** The hook driver calls it once before any other
> Interface A call. Until it does, the broker has allocated no ring and opened
> no listener.

The specification's own once-only calls are no help as precedent, because they
differ from each other. `shim.schema` refuses a second call outright. A
registration naming an already-registered topic with the identical value is a
no-op. A later `configure` applies the live keys and reallocates nothing. An
open is closest to the third, and nothing says so.

## Decision

One `Bridge` per process, behind a `OnceLock`. The first open creates it, every
later open takes the same one, and no open initializes anything twice.

An open allocates nothing. It creates no ring, binds no socket and starts no
thread, because SPEC 5.1 gives the first `shim.configure` that job. What an open
does is register itself: a counter rises, and the call returns its own number,
so the first open is 1 and the second is 2.

The module puts that number on the Lua table it returns, as `opens`. It is the
smallest field that reads through to shared state, and it is what makes the
process-global property observable from Lua at all — two distinct tables reading
one counter is the same evidence as two distinct tables reading one map, and it
is available before any map has anything in it. SPEC 15 gives it a second reader
later, where `doctor` checks "hook driver load, sim driver load".

Reading the registration maps recovers from lock poisoning rather than
propagating it. SPEC 14.2 has a parser fault drop one connection rather than the
process; a lock that takes the whole bridge down on someone else's unwind would
undo that.

The alternatives, and the line that rejected each:

- **Refuse the second open, the way `shim.schema` refuses a second schema.** It
  contradicts SPEC 5.1.1, which says both states load the broker and each gets
  its own table.
- **Per-state state handed back through the table.** The failure SPEC 5.1.1
  names in the same breath: the two registrars never see each other.
- **Create the bridge at the first `configure` instead of at the first open.**
  The maps would then have no owner during the window between the two calls,
  and a second open before `configure` would observe nothing at all — which is
  exactly the case a test with no DCS present can reach.
- **Return the same Lua table to every state.** Lua tables belong to a state,
  and handing one across is a crash rather than a shortcut.

## Consequences

`Bridge` is where everything process-global goes from here. The rings, the
listener and the threads join it as they are built, and a reviewer's question
about any new shared thing is whether it is a field on `Bridge` or a mistake.

`opens` is an addition to Interface A that the specification did not anticipate.
It is read-only and costs one integer, and the risk is that it becomes a habit:
a diagnostic on the shim table is cheap to add and never removed. Nothing else
joins it without its own reason.

Recovering from poisoning means a panic that happened mid-write leaves whatever
the writer had done visible to the next reader. Today the maps are empty and
there is no writer, so nothing is at stake. This is worth revisiting when the
registration merge lands, because a merge that panics halfway is precisely the
case where a torn map matters, and the answer there may be to build the new map
beside the old one and swap it.

The counter is process-wide, so a test binary shares one across every test in
it. Tests assert that the number rises rather than pinning it to 1 and 2; the
exact numbers are checked from Lua, where the harness gets a process to itself.

This reopens if the process ever needs two bridges. Nothing wants that today —
DCS runs one broker — but a test that wanted an isolated instance would have to
take `&Bridge` rather than reaching for the global, and the code is written so
that change is a signature change and not a rewrite.
