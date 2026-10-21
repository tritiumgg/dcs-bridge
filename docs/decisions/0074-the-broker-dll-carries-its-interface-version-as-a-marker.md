# ADR 0074: The broker DLL carries its interface version as a marker

## Status

Accepted

## Context

The hook driver refuses a broker from another build by comparing the
interface version the opened module reports with its own (ADR 0054). Both
versions are wanted from `dcsb doctor`, which runs outside DCS. The hook
driver's is one line of its file, `local INTERFACE = '...'`. The broker's
is a constant compiled into `lua-dcsbridge.dll`, and `dcsb` links no part
of the module and cannot load a DLL built against DCS's `lua.dll`.

Two sources were weighed:

- **`Logs\DCSBridge\DCSBridge.log`.** It names the broker's version in the
  `configured: interface N` line or the mismatch ERROR. The file is opened
  for append at every start, so an older run's line survives a newer run
  that stopped before `configure`, and only the lines after the last
  banner belong to the newest start. The `configured` line is INFO, so a
  `log_level` of `warn` or `error` hides it. And a log says nothing until
  DCS has run once with the installed files.
- **The DLL's own bytes.** `tools/stage-release.sh` already reads the DLL
  as bytes to find `lua.dll` and `luaopen_dcsbridge`. The version itself,
  `"5"`, is too short to find that way.

## Decision

`lua-dcsbridge.dll` carries the bytes `dcsbridge-interface=<version>`
followed by a NUL, and `doctor` scans the installed DLL for them.

- **One source.** The marker is built at compile time from
  `INTERFACE_VERSION`, and the string `luaopen_dcsbridge` hands the hook
  driver is a slice of the marker, so the bytes on disk are the bytes the
  hook driver compares and the marker cannot be optimized away.
- **The NUL ends the version,** since whatever the linker places after the
  marker can be printable.
- **No marker is a failure,** naming the DLL as from an older build or not
  this project's.
- **`doctor` does not read `DCSBridge.log`** for the version. The marker
  answers before DCS has ever run, and the mismatch a start would log is
  the same comparison of the same two values.

Alternatives:

- **The log, scoped to the newest run.** Rejected for the three reasons
  above: the scoping, the hiding `log_level`, and nothing to read before
  the first start.
- **A PE version resource.** It needs a resource compiler in the
  cross-build and a parser in `dcsb`, for one string.

## Consequences

The marker prefix is written twice, once in `crates/lua-module` and once in
`dcsb`, which links neither. A test in `dcsb` reads the module's source and
fails when the prefix there changes.

A DLL from before this record has no marker, and `doctor` reports it as
from an older build. That is true of every such DLL, since none ships in a
release with a hook driver that `doctor` would pass.
