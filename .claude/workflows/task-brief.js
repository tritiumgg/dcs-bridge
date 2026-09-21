export const meta = {
  name: 'task-brief',
  description: 'Read everything one plan task rests on and return its ordered commit list, who verifies it, and what a critic found wrong with that',
  whenToUse: 'Before the branch for a plan task exists. args: {task: "2.18"} or the id alone.',
  phases: [
    { title: 'Scout', detail: 'the plan row and the work list it implies', model: 'sonnet' },
    { title: 'Read', detail: 'decision records, specification anchors, code, README', model: 'sonnet' },
    { title: 'Plan', detail: 'the ordered commit list and who verifies', model: 'fable' },
    { title: 'Critique', detail: 'a different model tries to break the plan', model: 'opus' },
  ],
}

// Every model is pinned. An agent with none inherits whatever the session
// runs that day, and a saved workflow should cost and behave the same in
// every session. docs/developing.md says why each tier is what it is.

const id = typeof args === 'string' ? args.trim() : args && args.task
if (!id || !/^[0-9]+\.C?[0-9]+$/.test(id)) {
  throw new Error('task-brief needs a plan task id, as {task: "2.18"} or "2.18"')
}

const READ_ONLY =
  'You are read-only: edit nothing, commit nothing, push nothing. ' +
  'Report what you read, with the path and line for each fact, and label anything you did not check as unverified.'

const str = { type: 'string' }
const list = (items) => ({ type: 'array', items })

// Scout. One cheap agent turns the row into a work list, so the readers are
// sent to what this task touches and not to the whole repository.

phase('Scout')
const scout = await agent(
  `${READ_ONLY}

Plan task ${id}. Build the work list the readers after you will follow.

1. Find the row for ${id} in docs/plan/plan.md (search for "| ${id} |") and copy its task cell and its completion condition, the "Done when" cell, verbatim. Find the milestone row that lists ${id}, if one does.
2. Read STATE.md. Copy every line under "Next", "In progress" and "Carries forward" that names ${id} or that this task would settle, change or depend on.
3. Search docs/audit.md for ${id} and copy what it says.
4. List the decision records under docs/decisions/ that the row, those STATE.md lines or the audit cite, and any other whose title is plainly about this task's subject.
5. Name the ledger lookups a specification reader should run: a document code from \`sh tools/ledger.sh codes\` and a search text for \`sh tools/ledger.sh find <code> <text>\`. Run \`sh tools/ledger.sh subjects <code>\` to pick texts that exist. Never read a specification whole.
6. Name the source files and directories the task will change or build on, found by searching the code for the names the row uses.
7. Run \`sh tools/readmeopen.sh\` and copy the open README lines this task could settle.`,
  {
    label: `scout ${id}`,
    model: 'sonnet',
    effort: 'low',
    schema: {
      type: 'object',
      required: ['task', 'doneWhen', 'milestone', 'stateLines', 'audit', 'decisions', 'ledgerLookups', 'sources', 'readmeLines'],
      properties: {
        task: str,
        doneWhen: str,
        milestone: str,
        stateLines: list(str),
        audit: list(str),
        decisions: list(str),
        ledgerLookups: list({ type: 'object', required: ['code', 'text'], properties: { code: str, text: str } }),
        sources: list(str),
        readmeLines: list(str),
      },
    },
  },
)
if (!scout) throw new Error(`the scout returned nothing for task ${id}`)

const row = `Plan task ${id}.\nTask: ${scout.task}\nDone when: ${scout.doneWhen}\nMilestone: ${scout.milestone || 'none'}`

// Read. A barrier, because the plan is written from all four at once.

phase('Read')
const FACTS = {
  type: 'object',
  required: ['facts', 'unverified'],
  properties: {
    facts: list({
      type: 'object',
      required: ['fact', 'where'],
      properties: { fact: str, where: str },
    }),
    unverified: list(str),
  },
}

const readers = [
  {
    key: 'decisions',
    effort: 'low',
    prompt: `${row}

Read these decision records and the audit lines, and report every constraint they put on this task: what it must do, must not do, and what it leaves to a later task.
Records: ${scout.decisions.join(', ') || 'none named; list docs/decisions/ and read any whose title is about this task'}
Audit: ${scout.audit.join(' | ') || 'nothing'}
STATE.md: ${scout.stateLines.join(' | ') || 'nothing'}`,
  },
  {
    key: 'specification',
    effort: 'medium',
    prompt: `${row}

Report what the specifications say this task must do. Go through the ledger only: \`sh tools/ledger.sh find <code> <text>\` for rows, then \`sh tools/ledger.sh show <code> "<anchor>"\` for the prose around one. Never read a specification whole; a hook refuses it.
Start from these lookups and follow what they turn up: ${JSON.stringify(scout.ledgerLookups)}
For each fact give the anchor verbatim in "where". The anchor beats the claim beside it: where the two disagree, report the anchor and say so.`,
  },
  {
    key: 'code',
    effort: 'medium',
    prompt: `${row}

Map the code this task changes or builds on, starting from: ${scout.sources.join(', ') || 'search for the names the row uses'}
Report: the types and functions the task will touch, with path and line; the tests that already cover them and how they are driven (unit, Loom, Miri, loopback, tests/lua); every seam where the work could be cut into a commit that passes \`mise run check\` by itself; and anything a test here would need that depends on thread timing.`,
  },
  {
    key: 'readme',
    effort: 'low',
    prompt: `${row}

These README lines are marked open and this task may settle some: ${scout.readmeLines.join(' | ') || 'none found'}
Read README.md around each and around anything else the task changes in what a user downloads, installs, configures or runs. Report which paragraphs this task must rewrite, which open notes it takes out, and which stay open.`,
  },
]

const read = await parallel(
  readers.map((r) => () =>
    agent(`${READ_ONLY}\n\n${r.prompt}`, {
      label: `read ${r.key}`,
      phase: 'Read',
      model: 'sonnet',
      effort: r.effort,
      schema: FACTS,
    }).then((out) => ({ key: r.key, out })),
  ),
)
const missing = readers.filter((r, i) => !read[i] || !read[i].out).map((r) => r.key)
if (missing.length) log(`no report from: ${missing.join(', ')}; the plan is written without them and says so`)
const findings = JSON.stringify(read.filter(Boolean).filter((r) => r.out), null, 1)

// Plan. The one judgment in the brief: where the commits are cut and how
// large each is, tests counted as code.

const BRIEF = {
  type: 'object',
  required: ['commits', 'agentVerifies', 'maintainerVerifies', 'liveSteps', 'readme', 'decisionRecords', 'stateNext', 'risks', 'questions'],
  properties: {
    commits: list({
      type: 'object',
      required: ['message', 'holds', 'test', 'lines'],
      properties: { message: str, holds: str, test: str, lines: { type: 'number' } },
    }),
    agentVerifies: list(str),
    maintainerVerifies: list(str),
    liveSteps: list(str),
    readme: list(str),
    decisionRecords: list(str),
    stateNext: str,
    risks: list(str),
    questions: list(str),
  },
}

const PLAN_RULES = `Write the brief for this task.
- commits: the ordered list for one branch and one pull request. Each leaves \`mise run check\` passing and does one thing. About 100 changed lines is easy to review, 300 is fine for one logical change, and nothing reaches 1000; tests count as lines. A preparatory refactor is its own commit and changes no behavior. "message" is a Conventional Commit subject, "holds" is what the commit contains, "test" is what proves it, "lines" is the estimate.
- agentVerifies / maintainerVerifies: split every clause of the completion condition by who can observe it: an agent over loopback or in CI, or only the maintainer at a live DCS install. liveSteps are the copy-paste steps for the second kind.
- readme: each paragraph to rewrite and each open note to take out.
- decisionRecords: each choice that goes somewhere the specifications did not, as a title and one sentence of why.
- stateNext: the two or three lines STATE.md's "Next" should carry.
- risks: what could make an estimate wrong, and any test that would depend on thread timing.
- questions: only what the maintainer must decide before work starts.
Do not pad: an empty list is an answer.`

phase('Plan')
let brief = await agent(
  `${READ_ONLY}\n\n${row}\n\nWhat the readers found${missing.length ? ` (no report from: ${missing.join(', ')})` : ''}:\n${findings}\n\n${PLAN_RULES}`,
  { label: `plan ${id}`, model: 'fable', effort: 'high', schema: BRIEF },
)
if (!brief) throw new Error(`no plan was written for task ${id}`)

// Critique. A different model from the author's, so it does not repeat the
// author's mistakes, told to break the plan and not to improve it.

phase('Critique')
const critique = await agent(
  `${READ_ONLY}

${row}

A plan for this task follows. Break it. Check the code and the documents yourself where a claim can be checked. Look for: a completion condition clause no commit reaches; a commit that cannot pass \`mise run check\` without a later one; an estimate that ignores tests or is past 300 lines with no cut; a clause given to an agent that only a live install can show, or the reverse; a constraint the readers reported that the plan drops; a test that would pass or fail by thread schedule; a README paragraph or decision record left out.
"blocking" means the plan must change before work starts.

Readers' findings:
${findings}

The plan:
${JSON.stringify(brief, null, 1)}`,
  {
    label: `critique ${id}`,
    model: 'opus',
    effort: 'high',
    schema: {
      type: 'object',
      required: ['problems'],
      properties: {
        problems: list({
          type: 'object',
          required: ['blocking', 'about', 'problem', 'fix'],
          properties: { blocking: { type: 'boolean' }, about: str, problem: str, fix: str },
        }),
      },
    },
  },
)

// One revision, and only for what blocks. What does not block goes back to
// the maintainer beside the plan, unresolved, where it can be seen.

const problems = critique ? critique.problems : []
const blocking = problems.filter((p) => p.blocking)
if (!critique) log('the critic returned nothing; the plan is unreviewed')
if (blocking.length) {
  phase('Plan')
  log(`${blocking.length} blocking problem(s); revising once`)
  const revised = await agent(
    `${READ_ONLY}\n\n${row}\n\nWhat the readers found:\n${findings}\n\nYour plan:\n${JSON.stringify(brief, null, 1)}\n\nA critic found these blocking problems. Fix each, or say in "risks" why it is not one:\n${JSON.stringify(blocking, null, 1)}\n\n${PLAN_RULES}`,
    { label: `revise ${id}`, model: 'fable', effort: 'high', schema: BRIEF },
  )
  if (revised) brief = revised
  else log('the revision returned nothing; the first plan stands with its blocking problems')
}

return {
  task: id,
  row: { task: scout.task, doneWhen: scout.doneWhen, milestone: scout.milestone },
  brief,
  revised: blocking.length > 0,
  problems,
  unreviewed: !critique,
  readersMissing: missing,
}
