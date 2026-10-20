# ADR 0049: A disabled loader writes one line to dcs.log

## Status

Accepted

## Context

`Scripts\Hooks\DCSBridge.lua` carries an `ENABLED` flag, which keeps the
bridge installed but inactive. SPEC 13.1 gives it a tier, DCS restart, and
does not say whether the loader says anything when it is off.

SPEC 13 asks for a banner in `dcs.log` before anything registers: "No banner
means the file never parsed." A silent disabled loader looks the same as a
loader that never parsed, and `doctor` reads `dcs.log` to tell them apart.

## Decision

With `ENABLED` false, the loader writes one line to `dcs.log` naming the
flag and its file, and returns. It writes no banner, opens no log of its
own, and runs no hook driver. `Logs\DCSBridge\` is not touched.

Alternatives:

- **The banner, then a line saying the loader is disabled.** Two lines
  where one says both, and a search for the banner would match a disabled
  install.
- **Nothing.** A disabled install reads as a broken one.

## Consequences

An operator who turns the bridge off sees one line naming the file to edit
to turn it back on.

`doctor` tells three cases apart by `dcs.log` alone: a banner, the disabled
line, or neither.
