# The documents, and how to read them

**The three specifications are frozen.** `docs/specs/` holds them. They are the
starting point, they are not maintained, and the build will drift from them.
That is expected. Do not edit them. Where the build needs to go somewhere they
did not anticipate, write a decision record; `decision-records.md` says how.

**Nothing else in `docs/` is frozen.** The plan changes as build order changes,
and says so in its own first paragraph: *This document changes weekly. The
specifications it points at do not.* `audit.md`, `conventions/` and
`decisions/` all change too.

`docs/index.tsv` records which is which, in its `frozen` column, and
`tools/ledger.sh` reads it.

## Reading them

The specifications are large. The largest is 4,887 lines and its biggest
section is over 600. Loading one whole costs most of a context window and buys
nothing the ledger cannot locate more precisely.

Each document carries two companions. `<name>-ledger.tsv` holds one row per
claim, and `<name>-glossary.tsv` maps the names a subject goes by. Locate with
the ledger, then retrieve the prose. `tools/ledger.sh` does both, and a
`PreToolUse` hook refuses an unbounded `Read` of a specification.

## The ledger

Tab-separated. Three stamp lines, then a header, then six columns:
`subject`, `kind`, `status`, `claim`, `section`, `anchor`.

`kind` is `definition`, `interface`, `behavior`, `constraint`, `dependency`,
`goal` or `nongoal`. `status` is empty, or `UNVERIFIED` where evidence could
not decide. Eleven rows carry `UNVERIFIED`, and each names something the
documents rest on that nobody measured.

The `anchor` is a verbatim line from the document. It occurs exactly once and
sits inside the section its row names. That is what makes retrieval exact.

The glossary is `term`, `subject`, `defined_in`, `note`. Synonyms share a
`subject`, which matches a ledger subject.

## The anchor beats the claim

The anchor is verbatim specification text and is authoritative. The claim
beside it is a summary somebody wrote, and a summary can misread what it
summarizes. Where they disagree, the anchor wins.

Never quote a claim as though it were specification text.

## The stamp

The first three lines of each ledger and glossary record the SHA-256 of the
document they describe. `tools/ledger.sh stamp` compares it against the file.

For a frozen specification this never drifts, so a `MISMATCH` means somebody
edited a document that was not supposed to change. For the plan a mismatch is
expected and reports `STALE` instead, which fails nothing.

`.gitattributes` disables line-ending conversion for the same reason. A
converted checkout would break every stamp at once.

## The plan is not a requirement

`docs/plan/plan.md` states build order and nothing else. Where the plan and a
specification disagree, the specification is the better source, and neither is
binding once building starts.

Edit it when build order changes. Its ledger then reports `STALE` rather than
failing, and regenerating those rows is worthwhile but not urgent.

A task ID in the plan is a name, not a position. Tasks `8.1` through `8.4` sit
under Phase 11. Read the phase from the heading above the row.
