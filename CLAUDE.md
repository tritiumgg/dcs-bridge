# DCS-Bridge

A bridge between DCS World and external applications, allowing them to
interact with and receive from the simulation state through events,
subscriptions, and commands. The bridge is comprised of three parts:
A message broker that acts as the data transport, and two scripts that
interact with DCS World inside and outside of the simulation.

## Layout

```
STATE.md               what was just done, what is next, what carries forward
docs/
  specs/               frozen: the three specifications, with their ledgers
  plan/                build order. Changes. Not frozen.
  developing.md        the source tree, the build, and the release, for contributors
  audit.md             what the documents disagree about, checked once
  conventions/         how to read them, and how to write a decision record
  decisions/           where the build goes somewhere the specifications did not
proto/                 the record schema. buf.yaml configures the lint
vendor/lua/lua.def     the import definition for DCS's Lua
tests/lua/             the module opened by a stock Lua 5.1, no DCS present
tools/                 ledger.sh, luatest.sh, miri.sh, mkimplib.sh, mkschema.sh, readmeopen.sh, statecheck.sh
.github/workflows/     CI, release, version bump
.claude/               hooks: the read guard, the frozen-write guard, the shell guard,
                       the commit checks, the session start, the stop check
```

## `STATE.md` is the handoff between sessions

Read it before anything else. It names what is half-done, what was just
finished, what to pick up, and what carries forward from earlier work.

Update it before a session ends, not only when a task finishes. Sessions stop
mid-task, and the next one should not have to re-derive where things stood.

At the end of a task: move it into "Just finished", clear "In progress", pull
the next task up, and delete any carry-forward entry it resolved, saying where.

At the end of a session that stops mid-task: fill in "In progress" with the
task, what is done, what is not, and where to resume. Say what is committed and
what is only in the working tree. Say what is knowingly broken.

Stamp the "Last updated" line each time. It carries a date and nothing else.
A status clause there says what "Just finished" says two lines below and goes
stale on the next change, so `tools/statecheck.sh` refuses one.

**One `STATE.md` commit per task, on the task's own branch.** The pull request
is the landing, so write the entry the way the merge will make true rather than
describing a review in progress. A second pull request to say the first one
landed is a round trip that records nothing the merge did not.

Where a task lands as a sequence, that commit rides the last branch. An entry
on the first would claim a completion three reviews early.

That means a "Just finished" entry cites its pull request and not a CI run: a
run id exists only after the run, and amending the commit to add it starts a
different run.

"In progress" is for a session that stops with work an open pull request does
not already account for. A branch waiting on review is not that.

**Keep it small.** It is loaded cold every session, so its size is a tax on
every session. Each section has a line budget, `tools/statecheck.sh` enforces
them, and CI fails when the file is over. Over budget, nothing is deleted — it
moves. A completion older than the last few goes to git log. A choice with
reasoning behind it becomes a decision record. A durable fact about the project
belongs in this file, not that one. A resolved carry-forward is deleted.

Never write a paragraph where a line will do, and never copy a fact into
`STATE.md` that already lives in `CLAUDE.md`.

## `README.md` is for users, and the change that moves it updates it

The README says what a user downloads, installs, configures and runs. A task
that changes any of that updates the README in the same pull request. Where
the build has not reached something the README describes, the sentence says
so, on one line, with "not final", "not built" or "planned";
`tools/readmeopen.sh` lists those lines, and the task that settles one takes
the note out. Every pull request body answers the `README` heading of
`.github/PULL_REQUEST_TEMPLATE.md`. `docs/developing.md` is the contributor
document, and what a user does not need goes there instead.

## The specifications are frozen. Nothing else is.

`docs/specs/` is the starting point. Not maintained, and the build will drift
from it. Do not edit it, and do not offer to bring it up to date. Where the
build needs to go somewhere the specifications did not anticipate, write a
decision record: copy `docs/decisions/TEMPLATE.md` and number it next.

The plan is not frozen. It states build order. Edit it when the order changes.
Its ledger then reports `STALE`, which fails nothing; regenerate those rows
when convenient.

`docs/index.tsv` has a `frozen` column and the tooling reads it.

**A task ID never appears in the code.** Tasks are ephemeral. The plan retires
when the build ships, and a comment reading "task 1.3" then points at nothing.
Comments, error messages and the README say what the code does and why it is
that way; where a choice needs an argument, they cite the decision record that
holds it. Task IDs belong in `STATE.md`, the plan and `docs/audit.md`.
Neither does the word "done-when": it is the plan's word for a task's
completion condition and retires with the plan. A test comment says what the
test proves.

A dated document may name a task, because it says what was true on its date and
that stays true. A decision record and a commit message are both dated, and a
task branch is named for its task already.

**A specification citation never appears in the code either.** A comment, a doc
comment or an error message says why the code is the way it is, in its own
words. `SPEC §5.1.1` sends the reader to a document the build is drifting away
from, to find a reason that would have fitted in the comment. Where the
argument is too long for one, cite the decision record holding it.

`tools/nospecrefs.sh` enforces that and the CI `Preflight` job runs it.
Everything under `docs/`, plus this file, `README.md` and `STATE.md`, is
exempt, because documents cite documents.

## Never read a specification whole

The ledger beside each document holds a row per claim with an anchor that
locates the prose, so retrieval happens in `grep` and `awk` rather than in
a context window. A `PreToolUse` hook refuses an unbounded `Read` of a
specification, and another refuses an edit to one.

```
tools/ledger.sh codes                     document codes and paths
tools/ledger.sh subjects SPEC             every subject in a ledger
tools/ledger.sh find SPEC <text>          rows matching subject, claim or section
tools/ledger.sh show SPEC "<anchor>"      the prose around one anchor
tools/ledger.sh sections SPEC             the heading tree with line counts
tools/ledger.sh read SPEC "6.10"          one whole section
tools/ledger.sh lint                      every stamp, anchor and glossary join
```

Start from `subjects` or `find`, not from `sections`. The ledger is the index.

For a frozen specification, `lint` is tamper detection rather than maintenance:
a `MISMATCH` means one was edited. For the plan a mismatch reports `STALE` and
passes.

## Two rules that override anything else

**The anchor beats the claim.** The anchor is verbatim specification text. The
claim beside it is a summary and can misread what it summarizes.

**Say who verifies.** Most done-whens in the plan need a running DCS install or
a person watching. Before starting a task, decide whether an agent can observe
the result itself, whether it needs a maintainer reading a CI result, or whether
only somebody at a live install can see it. Write it in `STATE.md` under the
task. An agent that skips this declares victory on something it never observed.

## Toolchain

Tool versions come from `mise.toml`. Do not install toolchains globally, and do
not use a language's own version manager (`rustup` and similar) directly.

Run every project command through mise, because a non-interactive shell does not
pick up mise's PATH activation:

```sh
mise exec -- cargo test
mise exec -- lua5.1 script.lua
mise run test  # preferred, see [tasks] in mise.toml
```

Change tool versions with `mise use <tool>@<version>`, not by editing the
`mise.toml` by hand. Commit both `mise.toml` and `mise.lock`.

**Rust is the exception, and `mise use rust@<version>` breaks it.** The version
lives in `rust-toolchain.toml`, which also carries the components and the
Windows target, and mise reads that file. `mise use` writes a second version
into `[tools]` and mise stops reading it. Edit `channel`, then `mise install`.

On a fresh checkout, run `mise install` before anything else. If a command
reports the config is untrusted, run `mise trust`.

## Building for Windows

The product target is `x86_64` Windows and it is cross-compiled, so no
contributor needs a Windows machine.

```
cargo install --locked cargo-xwin      # once
sh tools/mkimplib.sh                   # check this machine can build the import library
cargo xwin build --release --target x86_64-pc-windows-msvc
```

`vendor/lua/lua.def` pins the 114 Lua symbols the broker may link against.
A full LLVM install is the one prerequisite; a rustup `llvm-tools` component
ships neither `llvm-dlltool` nor `llvm-lib`.

SPEC §14.2 forbids `panic = "abort"`, because a parser fault must drop one
connection rather than the process. CI fails the build if any `Cargo.toml` sets
it.

SPEC §13.3's four compared versions — `protocol`, `interface`,
`GRAMMAR_VERSION`, `STATE_VERSION` — never move for a reason outside their own
row. Automation touches the release version only.

## Portability

Developed on macOS and Windows from one checkout. Write POSIX `sh` and `awk`.
No bash arrays, no `[[`, no `local`. No `sed -i`, no `grep -P`, no
`readlink -f`. Avoid `sed` for tabs, because BSD and GNU disagree on `\t`.

Assume nothing beyond a stock machine: no `jq`, no `python`, no `gawk`. Detect
`sha256sum` versus `shasum`. No symlinks.

Leave `.gitattributes` alone. It disables line-ending conversion, without which
every ledger stamp breaks on a Windows checkout.

## Version control

This section overrides the global rules in `~/.config/agents/AGENTS.md`, which
are stricter.

Conventional Commits: `type(scope): summary`, imperative, under 72 characters.
Add a body when the change needs explaining.

**A branch per plan task**, named `task/<id>-<summary>`, such as
`task/1.1-cargo-workspace`. Work that belongs to no task takes the `type` it
would commit under: `fix/`, `docs/`, `build/`.

**A task too large to review at once lands as a sequence.** Past roughly 400
changed lines, split it into stacked branches — `task/2.1-1-crate-split`, then
`task/2.1-2-lua-surface` — landing ff-only in order. The plan names them and
what each is reviewable against, because that is the last moment the boundaries
are free to move. A preparatory refactor always takes its own, claiming no
change in behavior: sharing a diff with the feature hides which lines moved
among the lines that changed.

**Estimate before cutting, and cut at the phase, not at the task.** A branch
is sized before its first line is written: estimate its changed lines with
tests counted as code, and split any estimate past the 400 above then, since
a split found by measuring the diff afterwards costs a rewrite of every
branch above it. An estimate near the line names the seam it would split at.
The estimates go in the plan's sequence table. A phase's remaining
rows are cut at the start of the phase and again at each of its milestones,
which the plan names; a milestone is something a person sees, and reaching
one is when the rows ahead are re-measured, re-cut and re-ordered.

**Review the stack locally, run `mise run ci`, then push everything once.**
A push starts a CI run per branch, and a fix found after the push costs a
rebase and a run for every branch above it. So a task's branches are
finished on the machine first: one adversarial reviewer per branch, on
Sonnet, read-only, told the claim the branch makes and asked to break it;
every finding fixed on the branch that introduced it and the stack rebased
down its length; then `mise run ci` on the top, which runs what the Linux
job runs, Miri included. Only then is anything pushed, every branch at once,
and the pull requests opened. `mise run check` alone is what a Windows host
can run and what a mid-task commit needs; it is not what a push needs.

**History is linear. Rebase, never merge-commit.** Bring a branch up to date
with `git rebase origin/main`; the maintainer lands it with `git merge
--ff-only`. If the fast-forward is refused, fix the branch rather than reaching
for a merge commit. Rebasing re-signs, so the commit signatures survive the
rewrite.

**Work reaches `main` through a pull request the maintainer reviews.** Open it,
report it as waiting, and stop there.

**Every pull request body follows `.github/PULL_REQUEST_TEMPLATE.md`.** `gh pr
create --body` does not read the template, so write the body to its headings.
Summary ends with what the change is reviewed against: the plan task, the
decision record, or for a stacked branch the claim that branch alone makes.
Testing says how a reader runs the tests, not that they were run. Its steps
are numbered, start from a clean checkout, and are grouped by phase (without
DCS, then with DCS) and by platform (PowerShell on Windows, bash elsewhere).
Each step is an imperative sentence: an action, or a `Verify ...` naming what
the tester sees when the steps before it worked. Put a Verify wherever the
tester needs to know that before going on. The With DCS steps go build,
install, files to edit, then what to do in DCS. README names the paragraph the
change updated and any note it took out. Each heading reads `none`
where nothing applies.

Do these without asking: branch, commit, rebase onto `main`, push a topic
branch, force-with-lease a topic branch that is yours, delete a branch that is
already merged, and open a pull request with `gh`.

The hooks in `.claude/hooks/` hold the rest of this section: the shell guard
refuses a merge without `--ff-only` and a force push without a lease, and
turns a push to `main`, a tag, a release and a pull request merge into a
prompt for the maintainer. The commit checks run `tools/nospecrefs.sh` and
`tools/statecheck.sh` before a commit and check the message after. `mise run
docs` runs `tools/hooktest.sh` against every hook.

**Ask first before merging a pull request, before pushing `main`, before
rewriting history that has been pushed, and before tagging.** A small diff and
a green CI run are not permission, and permission given for one pull request
does not carry to the next. A tag is not a label here: `release.yml` fires on
`v*` and publishes to GitHub, so tagging is a release and the maintainer makes
it.

