# ADR 0001: The specifications are frozen

## Status

Accepted

## Context

The three specifications under `docs/specs/` run to 6,433 lines carrying 497
anchored claims across three ledgers. Each ledger row holds a stamp over the
document and an anchor quoting it verbatim, so every edit to a specification
invalidates a stamp and can break an anchor.

Keeping them current alongside the code is therefore a standing cost on a
document whose job ends once the code exists. Eleven ledger rows carry
`UNVERIFIED`, and several figures are marked provisional pending a probe that
has not run.

## Decision

Nothing in `docs/specs/` is edited. The specifications are a starting point
the build is allowed to leave behind.

- Where the build goes somewhere they did not anticipate, that is an ADR.
- The plan is not frozen. It states build order, build order changes, and its
  own first paragraph says so: *This document changes weekly. The
  specifications it points at do not.*

## Consequences

- Retrieval gets stronger. Stamps never drift, so a `MISMATCH` means tampering
  rather than staleness, and the anchors stay valid permanently. The ledger is a
  fixed index into a fixed document.
- The code becomes the record of what the code does. A reader who needs current
  behavior reads the code, not `docs/specs/`.
- The `UNVERIFIED` rows and the provisional figures stay as written. A probe's
  answer lands in an ADR, and the specification's statement that the figure was
  provisional stays true forever.
- The specifications are not operator documentation. Whatever an operator needs
  to read after 1.0 is a separate deliverable that does not exist yet.
- Drift between the specifications and the build is expected rather than a
  defect, so the ADRs are the only place that reconciles them.
