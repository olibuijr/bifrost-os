export const meta = {
  name: 'bifrost-parity-sprint',
  description: 'Triage the most important macOS 27 API/SDK parity gaps, then many Sonnet coders fill them in parallel (SDK side only)',
  whenToUse: 'Fast parity push; apps are migrated afterwards in a separate wave; the lead reviews and merges',
  phases: [
    { title: 'Triage', detail: 'rank gaps by daily-driver importance into disjoint SDK tasks' },
    { title: 'Build', detail: 'one Sonnet coder per task, own worktree, commit only', model: 'sonnet' },
  ],
}

const EXTRA = " GITHUB TEXT: commit messages, code comments, doc comments and docs must be professional, plain, human prose: short conventional commit subjects, bodies stating what changed and why, comments that explain intent; no AI-slop wording (no seamless/robust/comprehensive/leverage/delve, no 'not just X but Y', no emoji, no filler summaries); see ~/.claude/skills/avoid-ai-writing/SKILL.md."

// args: { sprint: number, tasks?: number, avoid?: string[], context?: string, taskList?: Task[] (skips triage) }
const A = args || {}
const N = Math.min(A.tasks || 10, 10)
const AVOID = (A.avoid || []).join('; ') || 'none'

const RULES = `Repo ~/Projects/bifrost-os (pure Rust, springboard/ workspace). AGENTS.md and the top of HANDOFF.md hold the rules.
BifrOSt mirrors macOS 27: each SDK module mirrors one Apple framework type by type (developer.apple.com is the reference; macOS 27 values
via \`bifrost pc ref\` and REA). NAME GUARD: no platform or product names in code, comments, identifiers or strings (no NS/UI/AV-prefixed type names, no product names like the search panel's or file manager's brand names); run \`python3 tools/check-names.py\` before every commit and get 0 hits (it is allowed to run). Pure Rust, no new C deps. TDD + clean code.
Use the akurai_index MCP (symbol_lookup/code_search, repo bifrost-os) before grep. Parity data lives in docs/parity-matrix/data/<framework>.json
(\`bifrost pc parity show|gaps|search\` read it). Owned by others now, do not touch: ${AVOID}.`

const TASKS = {
  type: 'object',
  properties: { tasks: { type: 'array', items: { type: 'object', properties: {
    framework: { type: 'string' }, title: { type: 'string' }, why: { type: 'string' },
    apple: { type: 'string' }, files: { type: 'array', items: { type: 'string' } },
    parityFile: { type: 'string' }, plan: { type: 'string' }, model: { type: 'string', enum: ['sonnet', 'haiku'] }, appsToMigrateLater: { type: 'array', items: { type: 'string' } },
  }, required: ['framework', 'title', 'why', 'apple', 'files', 'parityFile', 'plan'] } } },
  required: ['tasks'],
}

phase('Triage')
// A task list the lead prepared while the previous sprint ran skips the triage agent.
const triage = A.taskList ? { tasks: A.taskList } : await agent(
  `${RULES}${EXTRA}\n\nYou are the parity triage lead. READ ONLY. Goal: pick the ${N} most important missing macOS 27 API/SDK capabilities for BifrOSt as a daily-driver desktop OS, as independent tasks that Sonnet coders can do in parallel.
Inputs: \`bifrost pc parity status\`, \`bifrost pc parity coverage\`, \`bifrost pc parity gaps\` (and \`--group\`), \`bifrost pc parity os-status\`, docs/parity-matrix/data/*.json. ${A.context ? 'Also read the lead\'s survey: ' + A.context : ''}
Rank by daily-driver impact: what real apps and users hit first (files/documents, networking/URLSession, pasteboard/drag, notifications, preferences/defaults, keychain, contacts/calendar/reminders/photos stores, text input, printing, audio/video playback, accessibility, power/displays, launch services/workspace, search/metadata, sharing, undo/document model).
Rules for tasks: SDK/API side only (springboard/sdk, springboard/ui, springboard/core, shell services under springboard/src/shell/services); NO app changes (list apps that should adopt it later in appsToMigrateLater). Each task: one framework, one parity json file, files disjoint from every other task, LARGE: agents have 1M context with auto-compaction, so give each agent a whole framework or several related frameworks (thousands of lines with tests is fine). model = 'sonnet' for design-heavy or cross-cutting work, 'haiku' for mechanical, well-specified work. Verify each gap is real by reading the existing module first. 'apple' = the exact Apple types/methods being matched. plan = numbered steps including the tests to write first and the parity rows to update.`,
  { label: 'triage', phase: 'Triage', schema: TASKS })

// one-line `mod` additions to lib.rs/mod.rs merge cleanly, so they never count as overlap
const owned = t => t.files.filter(f => !/(lib|mod)\.rs \(.*mod lines? only/.test(f))
const seen = new Set(), tasks = []
for (const t of (triage?.tasks || [])) {
  if (owned(t).some(f => seen.has(f))) { log(`dropped overlapping task: ${t.title}`); continue }
  owned(t).forEach(f => seen.add(f)); tasks.push(t)
}
log(`sprint ${A.sprint || 1}: ${tasks.length} tasks: ${tasks.map(t => t.framework).join(', ')}`)

const DONE = {
  type: 'object',
  properties: { branch: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' }, unsure: { type: 'string' } },
  required: ['branch', 'commits', 'summary'],
}

phase('Build')
const results = await parallel(tasks.map((t, i) => () => agent(
  `${RULES}${EXTRA}\n\nImplement one parity task. Owner rule: only code and commit; do NOT run cargo build/test/check, publish or Midget commands (a hook blocks them), so read neighbouring code carefully to get imports and types right.
Setup: \`git checkout -b sprint${A.sprint || 1}/${i}-${t.framework.toLowerCase().replace(/[^a-z0-9]+/g, '-')}\`; link the gitignored assets: \`ln -s ~/Projects/bifrost-os/springboard/assets springboard/assets; ln -s ~/Projects/bifrost-os/bifrost-ref bifrost-ref\` (never commit them).
Write the tests first, then the code; mirror Apple's names and semantics (${t.apple}); update ${t.parityFile} rows for what you built (status stays partial; parity needs a Midget check), and document new API in docs/bifrost-ui.md under the framework. Do not touch apps. Commit with a conventional message ending in \`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\`.

Framework: ${t.framework}\nTask: ${t.title}\nWhy: ${t.why}\nFiles: ${t.files.join(', ')}\nPlan:\n${t.plan}`,
  { label: `build:${t.framework}`, phase: 'Build', model: t.model === 'haiku' ? 'haiku' : 'sonnet', isolation: 'worktree', schema: DONE })
  .then(d => d && { ...d, framework: t.framework, title: t.title, appsToMigrateLater: t.appsToMigrateLater || [] })))

return { sprint: A.sprint || 1, results: results.filter(Boolean), failed: tasks.filter((t, i) => !results[i]).map(t => t.title) }
