# ADR 0019: `configure` checks the pairs its own keys form

## Status

Accepted

## Context

A `configure` applies as one swap or not at all, and part of what it checks
before the swap is the rules that tie one key to another. SPEC 13.2:

> **Apply a `configure` as one swap, or not at all.** Validate every value,
> then check the broker's cross-key invariants — `max_unauthenticated_connections`
> below `max_connections`, `recent_idempotency_keys` at least
> `ring_in_sim_driver_records`, `recent_admin_keys` at least
> `ring_in_hook_driver_records` where the hook driver built-ins are deployed
> (HOOK §1) — against effective values, not file values: a restart-tier key
> may carry a pending file value. Reject the whole call on any failure.

and, in the paragraph after it:

> **An invariant is checked where both its keys are visible.** `configure`
> carries only the rows Section 13.1 marks **broker** (Section 5.1), so an
> invariant over sim-driver-tier keys cannot be checked there

Two of the three named pairs have a key `configure` never carries:
`recent_idempotency_keys` is the sim driver's and `recent_admin_keys` is the
hook driver's, by the group headings in SPEC 13.1. The named list and the
rule after it disagree about them, and the rule is the one that can be
built.

SPEC 13.1's basis column ties four more pairs of broker keys together
without naming them in 13.2's list. `max_unauthenticated_connections` "must
stay below `max_connections`". `ring_out_lifecycle_reserve` is "slots each
outbound ring keeps free", which partitions `ring_out_records`. Thirty
`heartbeat_interval_ms` "fit inside `dcs_alive_threshold_ms`, so a verdict
never rests on one missed beat". `dcs_alive_threshold_loading_ms` is "equal
to `load_timeout_ms` by construction — a lower value would flag DCS dead
during silence Section 11 calls normal". And SPEC 14.3 gates a public
`bind_address` on `allow_public_bind`, refusing to listen without it.

## Decision

`configure` checks every pair whose keys are both the broker's and whose
basis text says one bounds the other, and no pair with a key it does not
carry.

The pairs, each refused as the whole call:

- `max_unauthenticated_connections` below `max_connections`.
- `ring_out_lifecycle_reserve` below `ring_out_records`. A reserve at or
  past the ring's size leaves no slot for anything but `LIFECYCLE`.
- `heartbeat_interval_ms` below `dcs_alive_threshold_ms`. An interval at or
  past the threshold reads the sim as dead between two beats.
- `dcs_alive_threshold_loading_ms` at least `load_timeout_ms`. The
  specification pins them equal; at least is what the basis argues, and it
  leaves an operator free to widen the loading threshold alone.
- `bind_address` loopback or private, or `allow_public_bind` set. Refused
  at `configure` rather than at the bind, because the first `configure` is
  the bind and a later one changes neither key.

The two pairs over a sim driver or hook driver key are the hook driver's to
check where it assembles those tiers, as 13.2 says of
`subscription_max_evals`.

Alternatives:

- **The three named pairs and nothing else.** Two of them cannot be checked
  here, and the four pairs above have a basis as plain as the one that can.
- **The one checkable named pair and nothing else.** A reserve wider than
  its ring, or a heartbeat slower than its threshold, would then apply and
  fail in a way that reads as a broker fault rather than a file error.

## Consequences

A configuration the specification's list would pass can be refused here, on
one of the four added pairs; the error names both keys and what to do.

`dcs_alive_threshold_loading_ms` above `load_timeout_ms` is accepted where
the specification says equal. A consumer adopting the advisory
`load_timeout_ms` then times out before the broker gives up on the sim,
which is the safe side of the pin.

A pair added to the broker's keys later is added here in the same change,
under the same rule. A pair whose second key belongs to another component
is that component's to check, and this record does not cover it.
