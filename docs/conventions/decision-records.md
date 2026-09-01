# Decision records

The three specifications are frozen. A decision record is where the project
goes somewhere they did not. The plan is not frozen, so a change to build order
is an edit to the plan rather than a record.

A record states one choice and why. Records live in `docs/decisions/`, numbered
`NNNN-slug.md` and cited as `DR-NNNN`. Copy `docs/decisions/TEMPLATE.md` to
start one.

A record is written once. The one edit it accepts afterwards is its
`superseded-by` line.

## Format

```
# DR-0007: Ship the wider autoexec union

date: 2026-11-02
supersedes: DR-0003
superseded-by: none
diverges-from: SPEC §5.4

## Question
## Decision
## Why
```

`diverges-from` names the specification section this departs from, or `none`
when the documents said nothing on the subject.

The two pointers are a pair. `supersedes` points backward from the
replacement; `superseded-by` points forward from the replaced. Writing DR-0007
with `supersedes: DR-0003` means setting DR-0003's `superseded-by` to
`DR-0007` in the same commit. Both carry `none` until they carry something, so
a missing line is a defect rather than an absence of news.

Nothing enforces the pair. Check it with
`grep -H supersede docs/decisions/*.md`.

## When to write one

When the reasoning would otherwise be lost: a departure from what a
specification says, a risk knowingly accepted, an option rejected for a reason
someone will question later. A change with one obvious answer needs no record.

A probe answer is a record. The specifications call several figures provisional
pending a measurement, and the measurement's result belongs here rather than in
a document nobody is maintaining.

Say in `Why` what would reopen the decision, when something would.

## Reading them

Newest first. A later record that contradicts an earlier one wins, names it in
`supersedes`, and says why.
