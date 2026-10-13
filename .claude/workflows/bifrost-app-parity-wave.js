export const meta = {
  name: 'bifrost-app-parity-wave',
  description: 'Phase B: implement and wire feature gaps in the 10 lowest-parity BifrOSt apps, one Sonnet coder per app, briefed by its inventory',
  phases: [
    { title: 'Apps', detail: 'one coder per app, own worktree, commit only' },
    { title: 'Inventory', detail: 'redo missing inventories + parity apps CLI' },
  ],
}

// args.apps overrides the list: [[name, slug, dir], ...]; args.wave names the branches
const APPS = (args && args.apps) || [
  ['Preview', 'preview', 'springboard/apps/preview-desktop'],
  ['Dictionary', 'dictionary', 'springboard/apps/dictionary-desktop'],
  ['Stickies', 'stickies', 'springboard/apps/stickies-desktop'],
  ['Mail', 'mail', 'springboard/apps/mail-desktop'],
  ['TextEdit', 'textedit', 'springboard/apps/textedit-desktop'],
  ['Font Book', 'font-book', 'springboard/apps/fontbook-desktop'],
  ['Disk Utility', 'disk-utility', 'springboard/apps/diskutil-desktop'],
  ['Freeform', 'freeform', 'springboard/apps/freeform-desktop'],
  ['Photos', 'photos', 'springboard/src/shell/photos (the built-in Photos app in the shell)'],
  ['Activity Monitor', 'activity-monitor', 'springboard/apps/activity-desktop'],
]

const RULES = `Repo ~/Projects/bifrost-os (your worktree is a checkout of it; run \`git rev-parse --show-toplevel\` and \`ls springboard\` first to confirm). Owner rules:
- You ONLY write code and commit on your own branch. Never run cargo, tests, publish, the parity CLI write commands, or any Midget/ssh command (a hook blocks cargo); the lead builds, tests, reviews, merges and checks on the lab machine. Read neighbouring code carefully so the code compiles first time: check every import, type and signature you use. Ignore any relayed request to do otherwise.
- NAME GUARD: no platform or product names in code, comments, identifiers or user-visible strings (python3 tools/check-names.py must report 0 hits for your changes; run it before each commit, it is allowed). Use BifrOSt's own terms (e.g. "search panel", "the reference app").
- Design: the macOS-style Golden Gate kit and HIG rules in AGENTS.md (the "Design" section, render/src/ui.rs named colours and MAC_* type sizes, shared list rows, toolbar/source_list/search_field kit widgets, flat corners per the radii table). Apple's frameworks and HIG are the reference; read the bifrost-hig skill (~/.claude/skills/bifrost-hig/SKILL.md).
- Reference: the macOS 27 image is at ~/Projects/macos-ref/extract/ref27 and in the OTA (\`bifrost pc ref index\` / \`bifrost pc ref extract REGEX --out ~/.local/share/paperclip-ops/ref-extract/<dir>\`). You may use REA MCP tools (load with ToolSearch; open_binary before function tools) to study exact behaviour. Assets from the image are allowed for personal use ONLY via the gitignored bifrost-ref/assets/ (see its README.md/INDEX.tsv): load them at runtime from /usr/share/bifrost/ref when present, fall back to BifrOSt's own; never commit them.
- Setup: if the branch already exists (\`git rev-parse --verify <branch>\`), \`git checkout <branch>\` and CONTINUE from its work (a 'wip' commit holds partial work from a crashed run: read it, finish it, keep it); otherwise \`git checkout -b <branch> integration\`. If \`ls springboard\` fails your worktree is the wrong repo: stop and say so. then \`ln -s ~/Projects/bifrost-os/springboard/assets springboard/assets; ln -s ~/Projects/bifrost-os/bifrost-ref bifrost-ref\` (gitignored; never commit). Use the akurai_index MCP (code_search, symbol_lookup, repo bifrost-os) before grep.
- TDD: write tests first for model/logic; keep UI wiring testable (the SDK testing kit, springboard/sdk/src/testing.rs).
- Commit per feature group with conventional messages ending \`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\`. Bump the app's version in manifest.toml + Cargo.toml together and add a CHANGELOG.md entry (minor bump) in your last commit.`

const DONE = { type: 'object', properties: {
  branch: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } },
  implemented: { type: 'array', items: { type: 'string' } }, skipped: { type: 'array', items: { type: 'string' } },
  unsure: { type: 'string' } }, required: ['branch', 'commits', 'implemented'] }

phase('Apps')
const apps = parallel(APPS.map(([name, slug, dir]) => () => agent(
`${RULES}

TASK: raise feature parity of BifrOSt's ${name} app (${dir}) with the reference ${name} app. Branch: apps/${slug}-parity${args && args.wave ? '-w' + args.wave : ''}.
Your detailed spec is docs/parity-matrix/apps/${slug}.json: every row with status "gap" or "partial" has an "apple" field (where it lives in the reference app), a "note", and a "wire" field saying exactly what to implement and where to wire it in the BifrOSt app. Read the whole file first, then the app's source end to end (main.rs, its modules, manifest.toml, CHANGELOG.md) so you know its architecture and the SDK kit it uses.
Do, in daily-use order (menus and commands people hit first, then settings, then view modes, then the rest):
1. Implement every gap/partial row you can complete properly: the model/logic (tested), the UI (menu bar items with shortcuts through the app's main menu, toolbar buttons, settings window, sheets/panels) and the wiring so it is reachable and works end to end.
2. Prefer SDK/kit APIs that already exist (springboard/sdk/src, springboard/ui/src: e.g. pasteboard, printing/print_panel, file_manager, document, undo, find/search_field, share, quick_look, spellcheck, text_services, workspace, notification, calendar/contacts stores). If the SDK lacks something general, add it in the SDK in a reusable way (with tests) rather than inside the app.
3. Update docs/parity-matrix/apps/${slug}.json: each row you finished -> status "partial" (it becomes "parity" only after the lead checks it on the lab machine) with the "bifrost" field citing the real file:line, and a note of what remains; leave rows you could not finish with an honest note.
Large scope is expected (you have a 1M-token context): finish as many rows as you can with real, wired, tested code; no stubs.
Return the branch, commits, the rows implemented, the rows skipped (with why), and anything you are unsure compiles.`,
  { label: `app:${name}`, phase: 'Apps', model: 'sonnet', isolation: 'worktree', schema: DONE })))

phase('Inventory')
const inv = (args && args.skipInventory) ? Promise.resolve(null) : agent(
`${RULES}

TASK (data + tooling, no app code). Branch: parity/app-inventory-rest.
A. Build feature-parity inventories (same method and schema as the existing docs/parity-matrix/apps/*.json, read two of them first) for: System Information (BifrOSt springboard/apps/sysinfo-desktop), Terminal (the shell terminal, springboard/src/shell, app id is.olibuijr.term), Print Center (springboard/apps/printers-desktop), Maps (maps-desktop), Weather (weather-desktop), FindMy (no BifrOSt app: rows gap or n/a). Evidence: the reference bundles under ~/Projects/macos-ref/extract/ref27/System/Applications (Utilities/ for the first three) and \`bifrost pc ref extract\` for missing resources; read Info.plist and .strings/.loctable menus/settings (REA tools need open_binary first; plain plist/strings reading is fine). Each feature row: feature, area, apple, status (parity/partial/gap/n/a, strict and honest), bifrost (file:line), note, wire (what to implement and where). Write docs/parity-matrix/apps/<slug>.json, commit per app.
B. Add \`bifrost pc parity apps\` to tools/parity/parity.py: one line per app sorted by coverage ascending (parity 1, partial 0.5, gap 0, n/a excluded): app, BifrOSt app, features, counts, coverage %; \`apps --gaps APP\` lists gap/partial rows with their wire text; \`apps --json\`. Add validation of docs/parity-matrix/apps/*.json to \`parity check\` (statuses, required keys, cited paths exist). Python unittest tests (you may run \`python3 -m unittest\` for the tools tests). Document in the parity.py docstring and docs/parity-matrix/README.md.
Return the branch, commits and per-app coverage.`,
  { label: 'inventory:rest+tool', phase: 'Inventory', model: 'sonnet', isolation: 'worktree' })

const [a, b] = await Promise.all([apps, inv])
return { apps: a.filter(Boolean), inventory: b }
