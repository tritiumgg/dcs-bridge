# Decision records

The three specifications are frozen. An architecture decision record is where
the project goes somewhere they did not. The plan is not frozen, so a change to
build order is an edit to the plan rather than a record.

A record states one choice and why. Records live in `docs/decisions/`, numbered
`NNNN-slug.md` and cited as `ADR NNNN`. Numbers are four digits, sequential,
and never reused or renumbered. Copy `docs/decisions/TEMPLATE.md` to start one.

A record is written once. The one edit it accepts afterwards is its `Status`.

## Format

Michael Nygard's four headings, and no others:

```
# ADR 0007: Ship the wider autoexec union

## Status
## Context
## Decision
## Consequences
```

`Status` is exactly `Accepted` or `Superseded by [ADR MMMM](MMMM-slug.md)`.
There is no draft status: a record is written once the decision is made.

`Context` names the specification section the decision departs from, and quotes
the anchor rather than the ledger's claim. It says nothing when the documents
said nothing on the subject.

## Superseding

Writing a record that replaces an earlier one means two edits in one commit.
The new record's `Context` names the old one and says what changed to justify
revisiting it. The old record's `Status` becomes `Superseded by [ADR
MMMM](MMMM-slug.md)`, and nothing else in it is rewritten — it stays as the
record of what was decided, and why, at the time.

Nothing enforces the pair. Check it with
`grep -H '^Superseded' docs/decisions/*.md`.

## When to write one

When the reasoning would otherwise be lost: a departure from what a
specification says, a risk knowingly accepted, an option rejected for a reason
someone will question later. A change with one obvious answer needs no record.

A probe answer is a record. The specifications call several figures provisional
pending a measurement, and the measurement's result belongs here rather than in
a document nobody is maintaining.

Say in `Consequences` what would reopen the decision, when something would.

## Reading them

Newest first. A later record that contradicts an earlier one wins, names it in
its `Context`, and says why.
