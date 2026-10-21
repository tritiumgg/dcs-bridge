# ADR 0059: A reload with the file absent keeps the running table

## Status

Accepted

## Context

ADR 0051 makes an absent `Config\DCSBridge.lua` the defaults at DCS start,
with one INFO line, because an operator who has written nothing expects them.

SPEC 13.2 gives `ReloadConfig` the discipline of a sim driver reload:

> compile without executing, apply only if the whole file parses and
> validates, and keep the previous table in memory to roll back to. A config
> that raises leaves the running configuration untouched and reports why.

It says nothing about a file that is gone when the reload runs. A
`configure` table is the whole configuration, so the defaults handed on a
reload replace every live key. `tokens` defaults to an empty list: every
consumer's token stops authenticating at its next `Auth`, the one that sent
the reload included.

A file absent at reload is most often one mid-save, renamed, or moved by
mistake, and not an operator asking for the defaults.

## Decision

A reload that finds no file changes nothing. The hook driver logs one ERROR
line, `no <path>: the running configuration stays in force`, returns false
with that reason, and makes no `configure` call.

- `config.read` says the file was absent with a third return value, so the
  start keeps ADR 0051's behavior and the reload alone refuses.
- Absent means ENOENT, as at start. A file that is there and will not open
  is refused on both paths already.

Alternatives:

- **Apply the defaults, as at start.** One missing file locks every consumer
  out of a running server, including the one that would put it back.
- **Apply the defaults to every key but `tokens`.** A rule for one key that
  an operator has to learn, and the other live keys still jump.

## Consequences

An operator cannot reach the defaults by deleting the file and sending
`ReloadConfig`. A file that returns `{}` does that, or a DCS restart with the
file gone.
