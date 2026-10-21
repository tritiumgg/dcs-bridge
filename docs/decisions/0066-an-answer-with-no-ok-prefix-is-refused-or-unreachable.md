# ADR 0066: An answer with no OK| prefix is refused or unreachable

## Status

Accepted

## Context

SPEC 5.3 on what `net.dostring_in` answers:

> - Treat every returned string as untrusted. A chunk that raises returns its
>   error text as a normal string, and `pcall` still succeeds. Return `"OK|"`
>   plus the body. Check the prefix before you parse.
> - Separate three refusal cases. An unknown state name returns nil. A known but
>   unavailable state name returns `"Invalid state name"`. A call refused by
>   operator policy is a third case. These three shapes are measured behaviour,
>   not documented behaviour: ED's shipped reference specifies no refusal
>   contract. See Section 5.4. Report each distinctly and fail closed.

The first two refusal cases have a shape. The third does not: ED documents
no refusal text, and the specification records none. The only thing a policy
refusal is known not to be is nil, `Invalid state name`, or a string that
starts with `OK|`.

The first bullet names another source of the same shape. The injection
chunk (ADR 0065) runs the sim driver under `pcall`, but its first line,
which assigns `DCSBridge_load`, and its last lines, which read the epoch
back, run outside it. A raise there, or a chunk that does not compile,
answers its message as a plain string with no prefix. A value that is not
a string has no prefix either.

DCS 2.9.18.12722 enforced the policy gate and a hotfix reverted it
(ADR 0046), so no current build is known to produce the policy refusal at
all.

## Decision

The hook driver sorts the answer into a case by its shape, and names each
case for what the shape shows, not for a cause it cannot see.

- `nil`: no answer. Logged as no Lua state by the target's name.
- `invalid`: `Invalid state name`. Logged as the state not being available.
- `noprefix`: any other answer without `OK|`, a non-string among them.
  Logged as refused or unreachable, with the answer verbatim, so an operator
  and a later task can read which it was.
- `loaded`, `epoch` and `failed`: the bodies the chunk writes, ADR 0065's
  read-back and the sim driver's raise.
- `unknown`: an `OK|` body that is none of those. The prefix proves the
  chunk ran, so this is a fault in the chunk, kept apart from `noprefix`.

Each case logs one ERROR line of its own, `loaded` excepted. `driver.inject`
returns false and the case name for every case but `loaded`, which returns
true and its name, so the code that acts on a refusal branches on the name
without reading log text. The paths that never reach an answer return a
name too: `skipped` for a disabled bridge or the modified route,
`unprepared` for a refusal before the call, `unreached` when the call did
not run.

None of these cases disables the hook driver. Which of them count as a
policy refusal is the decision of the code that owns the disable, made
after a live install has shown the refusal text.

Alternatives:

- **Call every `noprefix` answer a policy refusal**, as the exclusion
  reading allows. A chunk fault would then disable the bridge and print an
  `autoexec.cfg` fix that does not apply.
- **Match a refusal text.** There is none documented or measured to match.
- **Fold `unknown` into `noprefix`.** The prefix is the one sign the chunk
  ran, and dropping it loses the difference between a chunk fault and a
  state that never ran the chunk.

## Consequences

The `noprefix` line cannot say which of its sources it saw. The verbatim
answer it carries is what tells them apart, and a live install with the
gate removed is what measures the policy text.

On a DCS build that does not enforce the gate, removing `"server"` from
`net.allow_dostring_in` is expected to change nothing, and the load then
reads as `loaded`.
The policy case stays unobserved until a build enforces it.

The case names are a contract between `driver.inject` and its callers. A
new shape is a new name, never a reuse of one.
