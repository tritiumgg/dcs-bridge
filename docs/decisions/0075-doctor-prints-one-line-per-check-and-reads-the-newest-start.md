# ADR 0075: doctor prints one line per check and reads the newest start

## Status

Accepted

## Context

`dcsb doctor` is the command a user runs after an install that did not
work, and its output is what they paste into a report. Its first increment
checks the shipped files, the banner and the hook driver load in
`Logs\dcs.log`, both interface versions, and the port. Four facts shape how
it reads and reports them:

- `dcs.log` carries a real prefix before `DCSBridge` that no file in the
  tree records, and a log can hold more than one start.
- The banner names the loader and the hook driver by size and modification
  time, formatted UTC by `os.date('!%Y-%m-%d %H:%M:%S')`, because no file
  name carries a version (ADR 0048).
- A sim mid-load reads as dead for up to 30 seconds while the loading flag
  has no setter, and a bridge disabled through `SetEnabled` still answers.
- `ping` exits 0, 1 or 2 by what it learned.

## Decision

`doctor` prints one line per check, marked `ok`, `fail` or `note`, and
exits 0 when nothing failed, 1 when a check failed, and 2 when it found no
write directory to check.

- **The newest start in `dcs.log`** is the last line holding `DCSBridge`
  and either the banner or the disabled line, and every `DCSBridge` line
  after it. A line is matched on the word `DCSBridge` and the loader's
  message text, never on the prefix. The file is read as bytes, decoded
  lossily, and a trailing carriage return is dropped, so a user name that
  is not UTF-8 does not stop the read.
- **The banner's sizes and times are compared** with the installed loader
  and hook driver, the time read from the file system and formatted UTC
  the way the loader formats it. A difference is a failure saying the file
  changed since DCS last started.
- **The port answering is `ok`.** `dcs_alive` false and `bridge_enabled`
  false are each a `note`, because neither is a broken install. The port
  is asked through `ping`'s own frame and deadline, and a refusal names the
  address tried and says `--addr` names another, because a port set in
  `Config\DCSBridge.lua` is one `doctor` cannot read.
- **`Config\DCSBridge.lua` is a `note`,** present or absent, since the
  hook driver runs on defaults without it.

Alternatives:

- **Stop at the first failure.** A broken install often has two causes, and
  the user would run the command once per cause.
- **Exit 1 for a sim reported dead.** A sim mid-load would read as a broken
  install.

## Consequences

The modification-time comparison holds only while the time DCS's `lfs`
reports and the one `std::fs::metadata` reports agree to the second. A
live start on an untouched install is the check of that, and comparing
sizes alone is the fallback if it fails.

The checks `doctor` gains later add lines in the same form, so a script
that reads the marks keeps working.
