# ADR 0055: The hook driver loads its generated file before configure

## Status

Accepted

## Context

SPEC 8.3 says the hook driver passes the hash `HookDriver.gen.lua` carries
"at the first `configure` as `schema_sha256`". The hook driver has to load
the file to read the hash, so it loads it before that call.

SPEC 6.10 puts `HookDriver.gen.lua` first in the hook driver's extension
chain, where each file is contained: a file that raises is skipped and
counted, and the rest load. The plan's task 4.11 builds that chain, and its
task 4.12 registers the generated tables.

The first `configure` binds the listener (ADR 0054). Anything that stops the
start after it leaves a broker listening behind a hook driver that did not
start.

## Decision

The hook driver loads `Mods\services\DCSBridge\lua\HookDriver.gen.lua` once,
at start, after the interface check and before the first `configure`, and
keeps the table as `state.gen`.

- It runs the chunk with the shim as its argument, as ADR 0040 defines.
- A file that is missing, does not compile, raises, or returns no 64-digit
  `schema_sha256` goes through ADR 0051's path: an ERROR line naming the
  path, a raise, and the loader's did-not-start line. Nothing binds.
- `state.gen.schema_sha256` is added to the broker's table as
  `schema_sha256` (ADR 0052).
- The release zip carries the file beside `HookDriver.lua`, so a missing one
  is a broken install rather than an operator's choice.

Alternatives:

- **Contain the generated file like any extension, and run without it.**
  Every built-in topic's class, route and capability is in it. A bridge
  without them serves no built-in record, and looks healthy while doing so.
- **Load it a second time where the tables are registered.** Two loads can
  disagree if the file changes between them, and the hash compared would not
  belong to the tables registered.

## Consequences

The extension chain starts after the generated file. `hook_driver_files_failed_total`
never counts it, and `hook_driver_disabled_files` cannot name it.

Registering the generated tables uses `state.gen` and does not load the file
again.

A hand-edited generated file whose hash line no longer reads as a hash stops
the start. One whose hash is a different valid hash runs, and the broker
counts the mismatch (ADR 0052).
