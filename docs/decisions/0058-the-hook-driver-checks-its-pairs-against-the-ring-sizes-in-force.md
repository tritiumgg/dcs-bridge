# ADR 0058: The hook driver checks its pairs against the ring sizes in force

## Status

Accepted

## Context

SPEC 13.2 checks the configuration's pairs against what is in force:

> check the broker's cross-key invariants — `max_unauthenticated_connections`
> below `max_connections`, `recent_idempotency_keys` at least
> `ring_in_sim_driver_records`, `recent_admin_keys` at least
> `ring_in_hook_driver_records` where the hook driver built-ins are deployed
> (HOOK §1) — against effective values, not file values: a restart-tier key may
> carry a pending file value.

The two `ring_in_*` pairs are the hook driver's to check, since the broker
never sees the memories (ADR 0050). The ring sizes are restart tier: the
broker allocates the rings at the first `configure` and never again. After
that call the file's ring size is not the ring's.

The hook driver's reader checked the file's values alone. On a reload, a file
that lowers a ring to 128 and its memory with it passes that check, while the
ring stays at 256 and the memory holding 128 forgets a key before its
duplicate can arrive.

## Decision

`config.read(path, held)` takes the values in force for the keys whose file
value is not the effective one, and runs every check with them in place.

- On a reload the hook driver passes the two `ring_in_*` sizes its first
  `configure` handed the broker. At start it passes nothing: the file's sizes
  are the ones the first call allocates.
- The values the reader returns are still the file's, so the broker is
  handed the file's ring sizes and reports them pending.
- A refusal names both keys and the value in force, as any pair refusal does.

Alternatives:

- **Check file values only.** It passes a memory shallower than the ring that
  is running.
- **Substitute the sizes in the values returned.** The broker would then
  never see the file's size and could not report it pending.
- **Ask the broker for its effective sizes.** `shim.configure` answers no
  values, and the first call's table already holds them.

## Consequences

A file that lowers a ring and its memory together is refused on reload until
DCS restarts, though the same file passes at start. The refusal's reason says
which ring size is in force.

The sim driver pair `subscription_max_evals` against `max_subscriptions` has
no restart-tier key and is unaffected.
