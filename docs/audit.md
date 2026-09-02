# What the documents disagree about

Checked once, on 2026-09-01, before anything was built. The specifications are
frozen, so nothing about them gets fixed. The plan is not, so the last two
entries here can be settled by editing it. The rest is what you will hit while
building, written down so it does not surprise you twice.

## The documents themselves are sound

Every check ran against the files, not from reading.

| Check | Result |
|---|---|
| Ledger stamps against the living bytes | 8 of 8 match |
| Anchors present, unique, inside their section | 553 of 553 |
| Glossary subjects joining a ledger subject | all |
| Prefixed cross-references (`SPEC §`, `SIM §`, …) | 231 resolve |
| Bare `Section N.M` references | 597 resolve |
| `PROBE-n` references against the registers | 64 resolve against 13 probes |
| Duplicate task IDs in the plan | none across 115 rows |

## Two unmeasured claims with no probe behind them

**The policy gate.** SPEC §5.4 records that ED's shipped reference and ED's
announcement give different `net.allow_dostring_in` values, with no measurement
distinguishing them. Its ledger row is `UNVERIFIED`. Four plan tasks rest on
which list is correct: 4.8, 4.9, 9.C1 and 10.2. Task 4.9 promises the operator
a paste-ready `autoexec.cfg`, and its contents are the unmeasured thing.
Neither probe register covers it. PROBE-A2 is adjacent but asks a different
question and needs a build that enforces the gate, which PLAN §3 says was
reverted.

    tools/ledger.sh find SPEC policy-gate

**The binding blacklist.** SPEC §4.2 records a seventh crasher with two
unattributed candidates, and says to treat the hook driver's `terrain` table as
a crasher family until probed member by member. Its row is `UNVERIFIED`. Task
5.2 ships the blacklist in Phase 5 and task 6.3 then calls a member of that
table in Phase 6. No probe covers the attribution.

    tools/ledger.sh find SPEC crasher-bindings

## Four constants fixed before they are measured

Tasks 2.15 and 2.18 size the rings and set `ring_out_lifecycle_reserve`.
PROBE-7 gives those four keys a measured basis at task 9.7. PLAN §4 states this
deliberately. The consequence worth knowing: 2.18's done-when is only
observable against a chosen reserve, so 9.7 reopens it rather than confirming
it.

PROBE-9 has the same shape and cannot move. The DLL ships at task 1.4, SPEC
§14.8 says a tainted verdict has a network consequence, and PLAN §3 says the
probe needs a shipped build and a real server.

## The glossaries cannot join the unmeasured claims

Eleven ledger rows carry `UNVERIFIED`. Ten of their subjects have no glossary
row at all: `budget`, `crasher-bindings`, `integrity`, `policy-gate`,
`correction latency`, `mission reload argument`, `onChatMessage arity`,
`onPlayerConnect name argument`, `onPlayerTryConnect definition` and
`open questions`. Only `mods-directory` has one, through the term `bait file`.

The glossary is the join key, so asking "what depends on this unmeasured
claim?" returns nothing for ten of eleven. Both problems above were found by
reading, and the next one will have to be too.

## Nine terms marked undefined that are defined

SPEC's Terms section defines Route A, Route B, a topic's target, the CLI's
diagnostic verb, and the four record classes. The glossary marks all nine
`UNDEFINED` while marking five terms from the same paragraph as defined in
`Terms`. A wrong `UNDEFINED` denies you a jump target and sends you searching.

87 of 96 SPEC glossary rows carry `UNDEFINED`. Most of the other 78 look
correct.

## Task 7.10 does not exist

Phase 7 runs 7.1 to 7.9 and then 7.11. PLAN §1 says a task ID is a name rather
than a position, which makes a retired ID legitimate, and it explains the case
where tasks 8.1 to 8.4 sit under Phase 11. It offers no note for 7.10.

## Task 2.16's done-when belongs to 2.18

Found on 2026-09-02, while deciding the outbound ring's shape, so it is later
than the pass above and not part of it.

2.16 registers the three maps, and its done-when opens "`LOSSY` drops before
`DURABLE` under pressure and `LIFECYCLE` survives". That is the class-aware
drop rule, which 2.18 builds. At 2.16 the rings evict by age alone, so the
clause cannot be observed when it is written, and a reviewer who takes it at
face value either blocks 2.16 on work two tasks away or waves the check through.

The registration half of that done-when — a second registrar merging, a
conflicting row refused whole, an outbound-only topic registering with no route
— is 2.16's own and stands. The plan is not frozen, so moving the first clause
to 2.18 settles this. It is left here rather than edited because which task owns
a done-when is a build-order decision.
