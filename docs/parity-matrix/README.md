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
import DIR | db-export DIR | refresh-index | verify plan|run|closures` (promote rows from Midget test evidence). `check` fails on unreadable files, cited repo paths that do not exist, and
frameworks not yet assessed.

When a feature ships, edit its row in `data/<slug>.json` (status, BifrOSt path, note), run `bifrost pc parity check` and
`build`, and update the published database (see the skill).

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
