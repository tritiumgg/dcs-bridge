# ADR 0068: The hook driver reads autoexec.cfg to log the union

## Status

Accepted

## Context

SPEC 5.4.2 on the file both keys live in:

> - Never write `autoexec.cfg`. Merge into it.
> - **Union both lists, under both keys.** Never replace either.

and it gives the union to `doctor`, which "prints the **union** the operator
should have. **It never prints this bridge's own minimum**, which would break
another tool if an operator pasted it over the file." SPEC 11 has the hook
driver log "the exact `autoexec.cfg` needed" on a policy refusal, and the
plan row that builds it asks for "the union `autoexec.cfg`".

The README gives two fixed lines for a file that has neither key, and tells
an operator whose file has one to add the missing names to it. A fixed line
pasted over another tool's line drops that tool's names.

`autoexec.cfg` is DCS's own file, a Lua chunk DCS runs at start. Besides
the two keys it may call DCS functions, such as `log.set_output`, and read
globals DCS defines. The hook driver's state is not the one DCS ran it in.

## Decision

On a policy refusal (ADR 0067) the hook driver runs `Config/autoexec.cfg`
from the write directory, takes the two lists it sets, and logs each key as
the union of the file's names and the bridge's.

- **The order.** The file's names first, in file order, then each of ADR
  0046's names the file lacks, in ADR 0046's order. A name is kept once. An
  entry that is not a string is left out, since it names no state.
- **The environment.** The file runs under `pcall` in a table of its own: a
  `net` table it assigns into, copies of `table`, `string` and `math`, and
  the base functions that touch no state. Every other name reads as a
  permissive stub that any index or call answers with itself, so a file
  that calls `log.set_output` or reads a global DCS defines still runs to its
  two keys. Nothing the file does reaches the hook driver's own globals. A
  file that replaces `net` with a table of its own is read from that table.
- **The text.** A byte-order mark is dropped and the chunk is named by its
  path, as `config.read` does for `DCSBridge.lua`. Each line is written in
  the README's form, `net.allow_dostring_in = {"server", "mission"}`.
- **The fallback.** A file that is absent gives the bridge's names alone,
  which is the whole union, with one INFO line saying to create it. A file
  that does not open, does not compile or raises anyway gives the bridge's
  names alone too, with one WARN line naming the reason and telling the
  operator to keep the names the file holds.

Alternatives:

- **Log the README's fixed lines.** Pasted over another tool's line, they
  drop its names, which SPEC 5.4.2 forbids for `doctor` for that reason.
- **Match the two keys with a pattern.** A Lua table has more spellings
  than one pattern reads: a key set over two lines, a `table.insert`, a
  name in single quotes.
- **Run the file with an empty environment**, as `config.read` does. Any
  call to a DCS function raises, and the file falls back to the fixed
  lines.
- **Leave the union to `doctor`.** It is phases away, and the plan row asks
  the hook driver for it.

## Consequences

A file whose names depend on what a stub returns, such as a list built in a
loop over a DCS call, yields the names the stub run gives, which can differ
from what DCS set. A loop that ends only on a DCS function's answer can run
forever under the stubs and hang DCS at the refusal. No such file is known,
and no instruction budget bounds the run.

The union shows the file as it reads now, while DCS read it at start. An
operator who edits the file before the refusal sees the edit in the lines.

`doctor` and the installer's merge build the same union. When they land,
the order here is the one they match, or a record says why not.
