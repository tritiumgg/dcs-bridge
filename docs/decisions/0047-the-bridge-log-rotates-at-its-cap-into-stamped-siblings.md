# ADR 0047: The bridge log rotates at its cap into stamped siblings

## Status

Accepted

## Context

The hook driver writes `Logs\DCSBridge\DCSBridge.log` under three keys.
SPEC 13.1 gives `log_max_bytes` as:

> Bounds `DCSBridge.log`, rotated, oldest rotated file deleted first — the
> same shape as the two audit logs.

and `log_retention_days` as "An upper bound on age, applied after the size
cap." It does not say whether the cap bounds the live file or the live file
and its rotated files together, how many rotated files are kept, or how
they are named.

Lua 5.1's `os.rename` is C `rename`, which on Windows fails when the
destination exists. A `.1`, `.2` chain therefore renames each step onto a
name it has just deleted. A fault between the delete and the rename loses a
file. A rename can also fail because another process holds the file open.

`lfs.attributes` gives `modification` in whole seconds, but copying a file
resets it.

## Decision

`log_max_bytes` caps the live file, and the rotated files together share a
second budget of the same size.

- Before a line is written, the writer reads the live file's size. When
  the line would take it past `log_max_bytes`, and the file is not empty,
  the file is renamed and a new one is started. A line longer than the
  cap still lands, alone, in an empty file.
- A rotated file is named `DCSBridge.<UTC>.log`, with `<UTC>` as
  `YYYYMMDD-HHMMSS`. A second file in the same second gets `-2`, and each
  later one the suffix after the highest in use, even when a prune has
  deleted the ones below it. This
  is the stamp DCS puts in its own rotated logs' names and the form the
  eval result logs take. No rename ever lands on a name that exists.
- After each rotation, and when the logger opens, the writer lists the
  rotated files and orders them by stamp, then by suffix, with no suffix
  as 1. It deletes the oldest until their sizes sum to at most
  `log_max_bytes`. It then deletes any whose stamp is older than
  `log_retention_days`. A name that does not parse is left alone.
- When the rename fails, the writer says so once in `dcs.log` and
  truncates the live file, so the cap holds.
- When the live file cannot be opened again, the writer says so once in
  `dcs.log` and drops lines rather than raising into its caller. Each
  later line tries the file again.

Alternatives:

- **One budget for the live file and its rotated files together.** The
  live file would rotate at a fraction of the cap, and "the log stops
  growing at its cap" would read as half the cap.
- **A fixed count of rotated files.** A second key with no default in the
  specification, and a limit with no default is a defect there.
- **Age from `modification`.** A copied file loses it. A name keeps its
  stamp and costs no call per file.

## Consequences

The log directory holds at most twice `log_max_bytes`, 64 MiB at the
default.

Age applies to rotated files only. On a quiet install the live file can
hold lines older than `log_retention_days` until it fills.

A failed rename loses the live file's contents. That happens when another
process holds the file open without delete sharing, and `dcs.log` names it.

Sorting by stamp assumes the clock does not run backward. A clock set back
makes a newer file sort as older and be deleted first.
