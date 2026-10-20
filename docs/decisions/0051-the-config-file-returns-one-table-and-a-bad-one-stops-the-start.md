# ADR 0051: The config file returns one table, and a bad one stops the start

## Status

Accepted

## Context

SPEC 13.2 says the hook driver parses, validates and distributes
`Config\DCSBridge.lua`, and that it "is a Lua file". It does not say what
shape the Lua takes. DCS's own files under `Config\` assign globals, as in
`options = { ... }`.

For a bad file, SPEC 13.2 speaks only of `ReloadConfig`:

> compile without executing, apply only if the whole file parses and
> validates, and keep the previous table in memory to roll back to. A config
> that raises leaves the running configuration untouched and reports why.

At DCS start there is no previous table to keep.

The file is optional. An operator who has written nothing expects the
defaults.

## Decision

`Config\DCSBridge.lua` is a Lua chunk that returns one table, as in
`return { port = 7742, log_level = 'debug' }`.

- The hook driver reads the file, compiles it with `loadstring` under the
  file's name, gives the chunk an empty environment with `setfenv`, and runs
  it under `pcall`. The chunk reaches no global: no `os`, no `io`, no `lfs`,
  no `net`.
- A leading UTF-8 byte-order mark, which some Windows editors write and Lua
  5.1 does not compile, is dropped.
- An absent file is the defaults and one INFO line in `DCSBridge.log`
  naming the path. Absent means `io.open` failed with ENOENT, the same
  number on every platform, where messages differ. Any other failure to
  open the file stops the start.
- Each key the table names overlays a copy of the defaults. A key no row in
  `KEYS` names is a WARN line and is dropped (ADR 0050).
- The start stops on any of these: the file does not open or compile, it
  raises, it returns something other than a table, a value's Lua type
  differs from its default's, or a check the hook driver owns fails. The reason goes to
  `DCSBridge.log` as an ERROR line, `start` raises it, and the loader writes
  its "did not start" line to `dcs.log`. `shim.configure` is never called,
  so nothing listens.
- Range checks on the broker's own values stay in the broker, which names
  the key when it refuses one. A refusal there stops the start the same way.

Alternatives:

- **Run on the defaults and warn.** The bridge then listens with values the
  operator did not write, and looks healthy while their file is ignored.
- **Accept DCS's assignment style as well.** Two shapes to document and
  test, and globals assigned by one file leak into whatever runs it next.
- **Give the chunk the standard library.** A config that can open files or
  run commands is an eval script under another name, run at every start
  with no audit.

## Consequences

A file that does not read takes the bridge down for that DCS run, and the
reason is in both logs. Fixing the file and restarting DCS is the remedy.

`ReloadConfig` reuses the reader. There a bad file keeps the table in force,
since a previous table exists.

The Route B bootstrap's read of the sim driver's keys uses the same shape and
the same empty environment, so one file reads the same way on both routes.

A chunk that loops forever stops DCS's start with it. The file is the
operator's own, and no instruction budget is set on it.
