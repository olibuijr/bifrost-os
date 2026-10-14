# Parity matrix data

Source of the Apple macOS 27 SDK vs BifrOSt matrix. The generated page is `docs/apple-parity-matrix.md`
(`bifrost pc parity build`); the published, database-backed view is linked from the `bifrost-parity-matrix` skill.

| File | What it is |
|---|---|
| `index.json` | Every framework the official documentation lists with macOS availability (`bifrost pc parity refresh-index`) |
| `data/<slug>.json` | One framework (or one part of AppKit, Foundation, SwiftUI, AVFoundation): status, summary, feature rows, gaps, REA evidence id |
| `apps/<slug>.json` | One reference app's feature inventory against its BifrOSt app: rows `feature, area, apple, status, bifrost, note, wire` (`bifrost pc parity apps`) |
| `os/<category>/*.json` | The OS layer: every component of the reference OS 27 image (26A434) in ten categories (`apps`, `assets`, `cli`, `daemons`, `drivers`, `frameworks`, `input-methods`, `settings`, `shaders`, `system-ui`), each with a BifrOSt counterpart and status. Written by `bifrost pc os-inventory` from the reference file index, then reviewed (below) |
| `ipados/index.json`, `ipados/data/<slug>.json` | The second reference platform, the tablet: every framework the official documentation lists with tablet availability (`refresh-index --platform ipados`; rows carry `ipados_since` and, when the framework is also on the desktop, `macos_since`), and the assessment files for frameworks that exist only there. Same row format as `data/` |
| `related.json`, `themes.json` | Nearest frameworks and clustered gap themes, written by `related` and `themes` |
| `agent-brief.md` | The brief agents follow to fill or refresh a data file (official docs via `tools/parity/docc.py`, REA via `tools/parity/rea-page.sh`) |

Statuses: `parity`, `partial`, `os`, `gap`, `n/a` (definitions in `agent-brief.md`). Coverage per framework: applicable rows
(not `n/a`) score parity 1, partial 0.5, os 0.5, gap 0, divided by the applicable rows.

Commands: `bifrost pc parity status | os-status | coverage [--group G] [--sort coverage|name|group] | show NAME |
gaps [--group G] [--status gap,partial] | check | build [OUT.md] | apps [--gaps APP] [--json] | import DIR [--new-only] | db-export DIR |
refresh-index | search QUERY [--top N] | related [--k N] | themes [--k N] | apply FILE.json [--dry-run] | verify plan|run|closures`.
`check` fails on unreadable files, cited repo paths that do not exist, frameworks not yet assessed, and invalid `apps/*.json`.
`search`, `related` and `themes` use embeddings from Titan (semantic search over the feature rows). `apply` takes closures
`[{slug, feature, status, bifrost, note}]`: each updates the row it matches (exact feature name, else rerank) or is appended.

Snapshot, 2026-10-13: 246 frameworks and parts assessed (185 with built work, 61 n/a), 2,442 feature rows, 71.0 % coverage and
86.5 % built (partial or parity over applicable rows); no framework as a whole is `parity` yet (a row needs a Midget check first). The OS layer has
6,807 components at 44.7 % over the 5,722 applicable ones. Re-run `status` and `os-status` for current numbers.

When a feature ships, edit its row in `data/<slug>.json` (status, BifrOSt path, note), run `bifrost pc parity check` and
`build`, and update the published database (see the skill).

## Tablet platform (`--platform ipados`)

`status`, `coverage`, `show`, `gaps`, `check` and `refresh-index` take `--platform macos|ipados` (default `macos`; `build`, `apps`,
`db-export` and the semantic commands stay macOS only). The tablet side is stored apart in `ipados/`, so the macOS matrix, its
`check` and `build` are not touched.

- `refresh-index --platform ipados` re-reads the official documentation index and writes `ipados/index.json`. A page that lists
  the tablet without a version gets the phone version.
- A framework that is in the tablet index and has a macOS assessment reuses those rows by reference: no file is needed, the
  rows are read from `data/` at load time (split frameworks such as AppKit parts and SwiftUI parts keep their parts).
  `status --platform ipados` shows how many frameworks reuse macOS rows and how many have their own file.
- To change what the tablet says about a shared framework, add `ipados/data/<slug>.json`. Without a `reuse` key it replaces the
  macOS rows. With `"reuse": "macos"` (or `"macos:<slug>"`, or `{"platform": "macos", "slug": "<slug>"}`) it extends them: the
  file's rows replace macOS rows of the same `feature` name and any other rows are added; `status`, `bifrost`, `summary`,
  `rea_evidence`, `gaps` and `doc` are taken from the file when present, else from the macOS assessment.
- A framework that exists only on the tablet gets its own `ipados/data/<slug>.json` (same format as `data/`). Until it does,
  `check --platform ipados` lists it under "not assessed" and exits 1.
- `check --platform ipados` also fails for a data file that matches no index row and for cited repo paths that do not exist (rows
  reused from macOS are checked by the macOS `check`).

Tests: `python3 -m unittest discover -s tools/parity` (`test_platform.py` covers the tablet platform).

## App inventories (`apps`)

`apps/<slug>.json` holds `app`, `bifrost_app`, `evidence` and `features`; an optional `summary` must match the rows. A row's `status`
is `parity`, `partial`, `gap` or `n/a` (strict: parity only when it works as the reference does); `bifrost` is a `file:line` in
this repo (empty for a gap); `wire` says what to implement and where, and is required on every `gap` and `partial` row.
Coverage: parity 1, partial 0.5, gap 0, `n/a` left out.

- `bifrost pc parity apps` one line per app, lowest coverage first: app, BifrOSt app, features, parity/partial/gap/n/a, coverage %.
- `bifrost pc parity apps --gaps APP` the gap and partial rows of one app with their wire text (APP is a slug or name, or a unique part of one).
- `bifrost pc parity apps --json` (also with `--gaps APP`) the same data as JSON.
- `bifrost pc parity check` also validates every `apps/*.json` (required keys, statuses, wire text, summary, cited paths exist).

Tests: `python3 -m unittest discover -s tools/parity`.

## `verify`: promote partial rows on Midget evidence

`bifrost pc parity verify plan|run|closures` (`tools/parity/verify.py`):

1. `plan [--group G] [--slug S] [--out plan.json]` resolves each `partial` row's cited repo paths to a crate and its `#[test]`
   functions and classifies the row: `logic` and `ui-headless` (drawing code whose cited tests render offscreen) can be promoted;
   `ui`, `hardware` and `untested` rows never are, they need a screenshot or a manual check.
2. `run PLAN --version V [--out results.json]` builds the test binaries (`cargo test --no-run` per crate), copies them to
   `/var/tmp/parity-verify/` on Midget and runs the rows' tests there.
3. `closures RESULTS --version V [--out closures.json]` emits `apply` closures for rows whose tests all passed, and writes
   `needs-manual.md` next to the output listing the rest by framework.

Owner rule (2026-10-12): test on the host, then on Midget; a row is `parity` only once it works on Midget.

## The OS layer: how the rows are produced

- `bifrost pc ref extract REGEX... [--out DIR]` / `bifrost pc ref index` decode the reference OS payload (LZRAVEN) and write the matching
  files or the full index outside the repo; `bifrost pc os-inventory [--index TSV]` turns the index into `os/<category>/*.json`.
  Index mode has names and paths only; document types, URL schemes and dyld images need an extracted tree (ROOT mode). Clean-room:
  names, identifiers, versions and structure only; no file contents are copied.
- `apps`, `settings` and `system-ui` rows come from reviewed tables in `os_inventory.py` (`REVIEWED_APPS`, `REVIEWED_SETTINGS`,
  `REVIEWED_SYSTEM_UI`), so a re-run reproduces them. Other rows get a heuristic counterpart (method `heuristic`) that is
  re-guessed on every run; replace it by setting `status` and dropping `method`, and a re-run keeps it.
- `tools/parity/os_frameworks.py [--write]` syncs the `os/frameworks` rows from the SDK matrix: public frameworks take the status of
  the SDK rows, private ones inherit from their owner (`os_private_owner_map.json`) and are never better than `partial`.
- `bifrost pc parity os-status` prints the OS layer per category.

## OS layer: command-line tools (`verify os-cli`)

The `os/cli/*.json` rows start as heuristic `os` (0.5 each). `bifrost pc parity verify os-cli` assesses them and checks each
tool on Midget (`tools/parity/os_cli.py`, mapping in `tools/parity/os_cli_map.json`):

1. `verify os-cli plan [--out plan.json]` writes `{id, command, probe}` per row. The command is the row name unless the map renames it
   (`bsdtar` is `tar`) or says there is none (`null`, a gap). Rows in an n/a class are left out: Xcode toolchain shims,
   Apple boot/firmware/security-chip tools, Apple `/usr/libexec` daemons and agents, and DTrace D scripts, each with a stated reason.
2. `verify os-cli run plan.json --version V [--out results.json]` runs every probe in one batched ssh session on Midget
   (`command -v X`, then `X --version` or `X --help` with a 5 s timeout). Tools that change machine state (`no_exec` in the map,
   for example `reboot`, `diskutil`, `pmset`) are only checked for presence.
3. `verify os-cli closures results.json --version V [--dry-run]` rewrites the shards with method `assessed`: `parity` (probe passed,
   `bifrost` = command path, note "verified on Midget V"), `partial` (installed but the probe failed or is not run headless),
   `gap` (no equivalent or not installed), `n/a` (with the class reason).
