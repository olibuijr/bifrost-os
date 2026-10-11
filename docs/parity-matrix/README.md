# Parity matrix data

Source of the Apple macOS 27 SDK vs BifrOSt matrix. The generated page is `docs/apple-parity-matrix.md`
(`bifrost pc parity build`); the published, database-backed view is linked from the `bifrost-parity-matrix` skill.

| File | What it is |
|---|---|
| `index.json` | Every framework the official documentation lists with macOS availability (`bifrost pc parity refresh-index`) |
| `data/<slug>.json` | One framework (or one part of AppKit, Foundation, SwiftUI, AVFoundation): status, summary, feature rows, gaps, REA evidence id |
| `agent-brief.md` | The brief agents follow to fill or refresh a data file (official docs via `tools/parity/docc.py`, REA via `tools/parity/rea-page.sh`) |

Statuses: `parity`, `partial`, `os`, `gap`, `n/a` (definitions in `agent-brief.md`). Coverage per framework: applicable rows
(not `n/a`) score parity 1, partial 0.5, os 0.5, gap 0, divided by the applicable rows.

Commands: `bifrost pc parity status | coverage [--group G] [--sort coverage] | show NAME | gaps [--group G] | check | build |
import DIR | db-export DIR | refresh-index`. `check` fails on unreadable files, cited repo paths that do not exist, and
frameworks not yet assessed.

When a feature ships, edit its row in `data/<slug>.json` (status, BifrOSt path, note), run `bifrost pc parity check` and
`build`, and update the published database (see the skill).
