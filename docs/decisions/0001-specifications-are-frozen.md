# DR-0001: The specifications are frozen

date: 2026-09-01
supersedes: none
superseded-by: none
diverges-from: none

## Question

Are the three specifications maintained alongside the code, or are they a
starting point the build is allowed to leave behind?

## Decision

They are a starting point. Nothing in `docs/specs/` is edited after this
commit. Where the build goes somewhere they did not anticipate, that is a
decision record.

The plan is not frozen. It states build order, and build order changes. Its own
first paragraph says so: *This document changes weekly. The specifications it
points at do not.* Edit it as the order changes.

## Why

Keeping the specifications current costs more than it returns. They run to
6,433 lines with 497 anchored claims across three ledgers. Every edit
invalidates a stamp and can break an anchor, and the discipline to keep that
sound is a permanent tax on a document whose job ends once the code exists. The
code is then the record of what the code does.

Freezing makes retrieval better rather than worse. The stamps never drift, so a
`MISMATCH` means tampering rather than staleness, and the anchors stay valid
permanently. The ledger becomes a fixed index into a fixed document.

Eleven ledger rows carry `UNVERIFIED`, and several figures are marked
provisional pending a probe. Those stay as written. A probe's answer lands in a
record, and the specification's own statement that the figure was provisional
stays true forever.

The specifications are specifications, not user documentation. Whatever an
operator needs to read after 1.0 is a separate deliverable that does not exist
yet.
