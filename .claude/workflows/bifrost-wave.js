export const meta = {
  name: 'bifrost-wave',
  description: 'One BifrOSt OS wave: scout (errors, hardening, cleanup, parity) -> code fixes in worktrees -> adversarial review',
  whenToUse: 'Each iteration of the BifrOSt daily-driver loop; the lead merges, tests and releases afterwards',
  phases: [
    { title: 'Scout', detail: '4 lenses pick concrete tasks' },
    { title: 'Code', detail: 'one Sonnet coder per task, own worktree, commit only', model: 'sonnet' },
    { title: 'Review', detail: 'skeptic per branch' },
  ],
}

// args: { wave: number, focus?: string, maxTasks?: number, avoid?: string[] }
const A = args || {}
const WAVE = A.wave || 1
const MAX = Math.min(A.maxTasks || 4, 4)
const AVOID = (A.avoid || []).join('; ') || 'none'
const FOCUS = A.focus ? `Extra focus from the lead: ${A.focus}.` : ''

const RULES = `Repo ~/Projects/bifrost-os (pure Rust, springboard/ workspace). Read AGENTS.md and the top of HANDOFF.md.
Architecture follows Apple's: kits mirror Apple frameworks (developer.apple.com is the reference, macOS 27 values via
REA/bifrost pc ref), HIG for UI (bifrost-hig skill). No platform or product names in code (tools/check-names.py).
Pure Rust, no new C deps. TDD + clean code. Use the akurai_index MCP (symbol_lookup/code_search) before grep.
Already done or in flight, skip: ${AVOID}. ${FOCUS}`

const TASKS = {
  type: 'object',
  properties: { tasks: { type: 'array', items: { type: 'object', properties: {
    title: { type: 'string' }, why: { type: 'string' }, files: { type: 'array', items: { type: 'string' } },
    plan: { type: 'string' }, parityRows: { type: 'string' }, impact: { type: 'number' },
  }, required: ['title', 'why', 'files', 'plan', 'impact'] } } },
  required: ['tasks'],
}

const LENSES = [
  ['errors', 'Find real defects: compiler warnings (cargo check --offline in springboard/), failing or #[ignore]d tests, unwrap/expect on fallible I/O or IPC paths in the shell/compositor/daemons, panics that would kill a daily-driver session, TODO/FIXME that hide a bug.'],
  ['harden', 'Hardening for daily-driver stability: crash recovery, IPC/socket input validation, unbounded buffers, file permissions, race conditions, resource leaks, timeouts, unsafe blocks lacking SAFETY justification.'],
  ['cleanup', 'Optimize and clean up: dead code, duplicated logic across kits/apps, hot-path allocations (see tools/bench.py, docs/perf-history.jsonl), modules that drift from the Apple framework shape they mirror.'],
  ['parity', 'Parity: run `bifrost pc parity gaps` and `bifrost pc parity os-status` (read-only). Pick gaps that most improve daily use (APIs AND databases/stores: Contacts, Calendar, Photos, Core Data/SwiftData-style persistence, Keychain, UserDefaults, Spotlight index), and partial rows whose model exists but is not wired to UI.'],
]

phase('Scout')
const scouted = await parallel(LENSES.map(([key, lens]) => () => agent(
  `${RULES}\n\nWave ${WAVE}, lens "${key}". READ ONLY, do not edit. ${lens}\nReturn up to 3 tasks, each small enough for one focused commit (one crate, <~400 lines), with exact files and a step plan including the test to write first. Verify each defect exists in the code before listing it. impact = 1-10 for daily-driver value.`,
  { label: `scout:${key}`, phase: 'Scout', schema: TASKS })))

const all = scouted.filter(Boolean).flatMap(r => r.tasks)
// Barrier: dedup by file overlap, keep the highest impact, so parallel coders don't collide.
all.sort((a, b) => b.impact - a.impact)
const picked = [], used = new Set()
for (const t of all) {
  if (picked.length >= MAX) break
  if (t.files.some(f => used.has(f))) continue
  t.files.forEach(f => used.add(f)); picked.push(t)
}
log(`wave ${WAVE}: ${all.length} candidates, running ${picked.length}; deferred: ${all.filter(t => !picked.includes(t)).map(t => t.title).join(' | ') || 'none'}`)

const DONE = {
  type: 'object',
  properties: { branch: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' }, untested: { type: 'string' } },
  required: ['branch', 'commits', 'summary'],
}
const VERDICT = {
  type: 'object',
  properties: { merge: { type: 'boolean' }, problems: { type: 'array', items: { type: 'string' } } },
  required: ['merge', 'problems'],
}

const results = await pipeline(picked,
  (t, _, i) => agent(
    `${RULES}\n\nImplement this task. Owner rule: agents only code and commit; do NOT run tests, builds, publish or Midget commands (a hook blocks them). First link the gitignored assets if missing (never commit them): ln -s ~/Projects/bifrost-os/springboard/assets springboard/assets; ln -s ~/Projects/bifrost-os/bifrost-ref bifrost-ref. Create branch wave${WAVE}/${i}-<slug>, write the test first, then the fix, keep it minimal, update docs/parity-matrix/data rows if ${t.parityRows || 'parity is affected'}, and commit with a conventional message.\n\nTask: ${t.title}\nWhy: ${t.why}\nFiles: ${t.files.join(', ')}\nPlan: ${t.plan}`,
    { label: `code:${t.title.slice(0, 40)}`, phase: 'Code', model: 'sonnet', isolation: 'worktree', schema: DONE }),
  (d, t) => d && agent(
    `${RULES}\n\nAdversarially review branch ${d.branch} (git diff main...${d.branch}) for task "${t.title}". Try to find: logic errors, missing test, behaviour that diverges from the Apple API it mirrors, product names in code, unsafe without SAFETY, scope creep. Default merge=false if unsure. Read only.`,
    { label: `review:${d.branch}`, phase: 'Review', schema: VERDICT }).then(v => ({ task: t.title, ...d, verdict: v })),
)

return { wave: WAVE, deferred: all.filter(t => !picked.includes(t)).map(t => t.title), results: results.filter(Boolean) }
