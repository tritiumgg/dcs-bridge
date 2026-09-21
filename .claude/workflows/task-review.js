export const meta = {
  name: 'task-review',
  description: 'Review a finished branch from four sides, then have a second model try to refute each finding',
  whenToUse: 'When a branch is finished and before it is pushed. args: {claim: "...", task: "2.18"} or {claim: "...", branch: "fix/..."}; base defaults to main.',
  phases: [
    { title: 'Review', detail: 'the claim, concurrency, project rules, test quality' },
    { title: 'Refute', detail: 'each finding, by a different model from the one that found it' },
  ],
}

// Every model is pinned, for the reason task-brief.js gives.
// docs/developing.md says why each tier is what it is.

const input = args || {}
const claim = input.claim
const base = input.base || 'main'
const task = input.task
if (!claim) {
  throw new Error('task-review needs the claim the branch makes, as {claim: "...", task: "2.18"} or {claim: "...", branch: "..."}')
}
if (task && !/^[0-9]+\.C?[0-9]+$/.test(task)) throw new Error(`"${task}" is not a plan task id`)

const READ_ONLY =
  'You are read-only: edit nothing, commit nothing, push nothing, and leave the toolchain alone. ' +
  'You may run `mise exec -- cargo test`, `mise exec -- cargo clippy` and the scripts under tools/. Do not run Miri; it takes minutes and CI runs it.'

const SUBJECT = `The branch under review is ${input.branch || 'the one checked out'}. Read it with \`git log --oneline ${base}..HEAD\` and \`git diff ${base}...HEAD\`, and one commit at a time with \`git show\`.
The claim the branch makes: ${claim}
${task ? `It is plan task ${task}: find its row in docs/plan/plan.md by searching for "| ${task} |" and hold the branch to its completion condition, the "Done when" cell.` : 'It belongs to no plan task, so the claim above is what it is held to.'}`

const str = { type: 'string' }
const FINDINGS = {
  type: 'object',
  required: ['findings'],
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['file', 'line', 'summary', 'breaks'],
        properties: { file: str, line: { type: 'number' }, summary: str, breaks: str, fix: str },
      },
    },
  },
}
const VERDICT = {
  type: 'object',
  required: ['refuted', 'why'],
  properties: { refuted: { type: 'boolean' }, why: str },
}

const FINDING_RULES = `Report defects only, each with the file and line, one sentence of what is wrong, and in "breaks" the concrete input, schedule, platform or reader that it fails for. A finding with no such case is an opinion: leave it out. No findings is an answer; do not pad.`

// The concurrency side takes the strongest model and so does its refuter: a
// wrong "no findings" on a wake or an unsafe block costs the most, and a
// refuter weaker than the finder throws true findings away.

const SIDES = [
  {
    key: 'claim',
    model: 'sonnet',
    effort: 'high',
    refuter: 'opus',
    refuterEffort: 'high',
    prompt: `Try to break the claim. For each clause of it${task ? ' and of the completion condition' : ''}, find the code and the test that make it true, and look for the case where it is not: an input the code mishandles, a clause no test reaches, a test that would pass with the change reverted, an error path that leaks or panics. A parser fault must drop one connection and never the process.`,
  },
  {
    key: 'concurrency',
    model: 'fable',
    effort: 'xhigh',
    refuter: 'fable',
    refuterEffort: 'xhigh',
    prompt: `Review every change that touches threads, atomics, rings, channels, parking or an unsafe block. For each, name the hostile schedule and walk it: a lost wake, a record read while it is evicted, two producers on a ring that allows one, an ordering the code assumes between the control channel and the commit ring, a Relaxed load that needed more. Check that each unsafe block's written ownership argument still holds after the change, and that Loom models what the change added where Loom can. If the branch touches none of this, return no findings.`,
  },
  {
    key: 'project-rules',
    model: 'sonnet',
    effort: 'low',
    refuter: 'opus',
    refuterEffort: 'high',
    prompt: `Hold the branch to the project's written rules, and report only a rule it breaks. No task id, no specification citation and none of the plan's own words for a task in code, comments, error messages or the README (run \`sh tools/nospecrefs.sh\`). README.md changed in this branch if what a user downloads, installs, configures or runs changed, with the matching "not final" or "planned" note taken out. One STATE.md commit for a plan task, within budget (\`sh tools/statecheck.sh\`). A decision record where the build went somewhere the specifications did not. docs/specs/ untouched. Shell that is POSIX sh and awk: no bash arrays, no [[, no local, no sed -i, no grep -P. Each commit one thing, under about 300 lines, with a Conventional Commit subject. US English. A document describes the current state and carries no history.`,
  },
  {
    key: 'test-quality',
    model: 'sonnet',
    effort: 'high',
    refuter: 'opus',
    refuterEffort: 'high',
    prompt: `Review the tests the branch adds or changes. For each: does it prove what its name and comment say, or would it pass with the behavior broken? Does it depend on thread timing: a sleep, a deadline standing in for a signal, a control message sent and then a record committed with nothing making the writer take the first before the second? Does it run under Miri and Loom where the module's other tests do, or is it kept off them without a stated reason? Is a clause the branch claims left with no test at all?`,
  },
]

const results = await pipeline(
  SIDES,
  (side) =>
    agent(`${READ_ONLY}\n\n${SUBJECT}\n\nYour side of the review: ${side.key}.\n${side.prompt}\n\n${FINDING_RULES}`, {
      label: `review ${side.key}`,
      phase: 'Review',
      model: side.model,
      effort: side.effort,
      schema: FINDINGS,
    }),
  (review, side) => {
    if (!review) {
      log(`the ${side.key} reviewer returned nothing; that side is unreviewed`)
      return { side: side.key, unreviewed: true, judged: [] }
    }
    return parallel(
      review.findings.map((f) => () =>
        agent(
          `${READ_ONLY}\n\n${SUBJECT}\n\nA reviewer reports this defect. Try to refute it: read the code it names, run what can be run, and find the reason it is not a defect. Answer refuted only when you have that reason and can point at it; where you cannot tell, the finding stands.\n\n${f.file}:${f.line}\n${f.summary}\nFails for: ${f.breaks}`,
          {
            label: `refute ${side.key} ${f.file}:${f.line}`,
            phase: 'Refute',
            model: side.refuter,
            effort: side.refuterEffort,
            schema: VERDICT,
          },
        ).then((verdict) => ({ ...f, side: side.key, verdict })),
      ),
    ).then((judged) => ({ side: side.key, unreviewed: false, judged: judged.filter(Boolean) }))
  },
)

const sides = results.filter(Boolean)
const all = sides.flatMap((s) => s.judged)
// A finding whose refuter died stands: nothing argued it away.
const confirmed = all.filter((f) => !f.verdict || !f.verdict.refuted)
const refuted = all.filter((f) => f.verdict && f.verdict.refuted)
const unreviewed = SIDES.map((s) => s.key).filter((key) => {
  const side = sides.find((s) => s.side === key)
  return !side || side.unreviewed
})

log(`${confirmed.length} finding(s) stand, ${refuted.length} refuted${unreviewed.length ? `, unreviewed: ${unreviewed.join(', ')}` : ''}`)

// The refuted ones go back too, with the reason, so a true finding argued
// away can be seen and not only counted.
return { confirmed, refuted, unreviewed }
