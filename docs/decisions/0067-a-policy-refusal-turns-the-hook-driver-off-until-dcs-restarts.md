# ADR 0067: A policy refusal turns the hook driver off until DCS restarts

## Status

Accepted

## Context

SPEC 11's failure table gives the response to a refused call:

> | `net.dostring_in` refused by policy | Distinct refusal, not nil and not `"Invalid state name"` | Hook driver logs the exact `autoexec.cfg` needed and disables itself. **No retry loop.** Mission unaffected. | Operator merges the config, or installs Route B |

SPEC 5.4 names a second way the policy stops the call:

> **Both keys, not one.** A configuration that sets `net.allow_dostring_in` and
> leaves `net.allow_unsafe_api` unset fails closed on the caller side: the API is
> not visible to a `Scripts\Hooks\*.lua` script at all.

ADR 0066 sorts the answer by its shape and leaves the policy cases to the
code that owns the disable, "made after a live install has shown the
refusal text". No build that can show it exists. DCS 2.9.18.12722 enforced
the gate and a hotfix reverted it (ADR 0046), and ADR 0066 records that the
policy case stays unobserved until a build enforces it. The plan row that
owns the disable cannot wait on a DCS release, so this record amends that
clause of ADR 0066: the rule below is chosen from the two shapes the
specification describes, without a measurement.

Two of ADR 0066's shapes fit. A call that finds no `net` namespace or no
`net.dostring_in` is the hidden API of the second quote. A string answer
with no `OK|` prefix is the distinct refusal of the first. That string has
other sources: a chunk that does not compile or raises outside its `pcall`
answers its message. Lua writes that message with a position, `chunk:line:`,
unless the raise was `error(x, 0)` or the state ran out of memory.

The kill switch (ADR 0063) is the operator's: the file sets it, a reload
sets it again, and `SetEnabled` moves it through `shim.tick`. DCS reads
`autoexec.cfg` once, at its start.

## Decision

Two cases turn the hook driver off for the life of the process, and each
logs the `autoexec.cfg` lines that fix it.

- **`absent`**, a new case name. `driver.inject` answers it for a call that
  did not run because `net` or `net.dostring_in` is missing. A raise inside
  `net.dostring_in` keeps the name `unreached` and does not turn anything
  off: its shape on an enforcing build is unknown, and the fix may not
  apply.
- **`noprefix` with a string answer that has no position prefix**, such as
  `not allowed`. A positioned string, such as `[string "x"]:1: boom`, and a
  value that is not a string stay a chunk fault and log ADR 0066's line
  alone. An unpositioned chunk fault, `error(x, 0)` or `not enough memory`,
  reads as policy. The injection chunk raises neither on purpose.
- `nil`, `invalid`, `epoch`, `failed`, `unknown`, `skipped` and `unprepared`
  turn nothing off.

The disable is a latch in `HookDriver.lua`, apart from the kill switch.

- `driver.enabled()` reads true only while the switch is on and the latch
  is not set, so every path that consults it stops. Nothing but a process
  restart clears the latch: the file it waits on is read only then, so a
  retry inside the process would find the same refusal.
- The operator switch keeps moving under the latch. `shim.tick`, a reload
  and `SetEnabled` still set it, so the bridge comes back in the state the
  operator chose once DCS restarts. While the latch is set, the switch's
  INFO line adds `, but refused until DCS restarts`, so a `bridge enabled`
  line does not read as a running bridge.
- `Pong`'s `bridge_enabled` keeps reporting the operator switch. The latch
  is the hook driver's; the broker never learns of it.
- Setting the latch logs one INFO line,
  `bridge disabled until DCS restarts: <reason>`.

The fix is logged to `DCSBridge.log`, one ERROR line per `autoexec.cfg`
line, each a whole Lua statement after the level, after the case's own
ERROR line. The logger writes one timestamp and level per call, so a line
with embedded newlines would leave its continuation lines without them, and
the operator copies each statement from after the level. Nothing goes to
`dcs.log` (ADR 0062). ADR 0068 says what the lines hold.

Alternatives:

- **Turn the broker's switch off.** The shim exposes `configure` and `tick`
  alone, and a later `SetEnabled` true would clear it.
- **Retry at the next mission load.** The file cannot change under a
  running DCS, so every retry logs the same refusal.
- **Fire on every `noprefix`.** ADR 0066 rejected it: a chunk fault would
  print a fix that does not apply.
- **Fire on a raise inside `net.dostring_in`.** Nothing says a policy
  refusal raises, and a raise has causes the fix does not touch.
- **Wait for a build that enforces the gate.** None is announced, and the
  plan row does not move until one ships.

## Consequences

The rule is unmeasured. A live refusal that carries a position prefix, or
arrives as a raise inside `net.dostring_in`, misses it: the hook driver
logs ADR 0066's line, injects nothing, and retries at the next load. The
first live refusal text, recorded verbatim with its DCS build in a pull
request's `With DCS` section, reopens this record: the fix is one change to
the predicate, or moving `unreached` into the firing set.

A chunk fault raised with `error(x, 0)` or an out-of-memory answer turns
the bridge off and prints a fix that does not apply, until DCS restarts.

`Pong` reports `bridge_enabled` true while the hook driver is latched off,
and `SetEnabled` cannot bring it back. The ERROR and INFO lines in
`DCSBridge.log` are where an operator reads why. Making the broker report
the latch is a new shim binding and an interface version.

A later path consults the latch only through `driver.enabled()`. One that
reads the switch directly runs while latched.
