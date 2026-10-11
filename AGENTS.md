# BifrOSt PC: working instructions

This repo is the BifrOSt desktop for PC (bifrost-os): the desktop shell, the native installer, the Midget lab
machine, and the Linux desktop apps sold through the PC App Store.

Read `HANDOFF.md` first for the current state and open problems.

## Quick lookups (read these instead of searching)

| Need | Where |
|---|---|
| Current state, open problems | `HANDOFF.md` (top section is the latest) |
| GPU / rendering / glass | `docs/gpu-first.md`; code `springboard/src/gpu/wl.rs` (presenter), `src/bare/wayland.rs` (surfaces, dma-buf), `render/src/glass.rs` (glass hook) |
| Design decisions (macOS Golden Gate) | `docs/design-golden-gate.md`; HIG checklist skill `bifrost-hig` |
| Desktop shell drawing | `src/shell/desk.rs` (windows, dock), `desktop_bar.rs` (menu bar, menus, `desk_regions_now`), `mod.rs` `Shell::draw` |
| Settings pages | `src/shell/pages.rs` (specs), `settings.rs` (actions, sliders, prefs keys) |
| Golden Gate SDK adoption (batches of 10) | `docs/golden-gate-sdk-adoption.md`; kit API `docs/bifrost-ui.md` "Golden Gate kit"; `bifrost pc adoption`, `bifrost pc gallery` |
| Host preview of an app (Dark and Light) | `bifrost pc smoke APP [--light] --sizes 1200x780 --scale 1.5625` |
| UI kits (`bifrost-ui`, *Kit parity: `detect`, `uttype`, `cache`, ...) | `docs/bifrost-ui.md`; code `springboard/ui/src/`; SDK `bifrost_app::controls`; bench `tools/bench.py` (`bench_ui_*`) |
| Colours, type, radii | `springboard/render/src/ui.rs` |
| Release / Midget flow | `docs/bifrost-pc-workflow.md`; skill `bifrost-pc` |
| Testing (acceptance, SDK testing kit, snapshots, coverage) | `bifrost pc acceptance [--quick] [--bless]`; `docs/bifrost-ui.md` "Testing kit"; `springboard/sdk/src/testing.rs`, `sdk/tests/snapshots/`; `tools/check-sdk-coverage.py` |
| Benchmarks and history | `tools/bench.py` (host benches), `tools/bench.py --midget` (device), `docs/perf-history.jsonl` |
| Logging (Logger, registry, `bifrost-log`, Console app) | `docs/milestone-unified-logging.md`, `docs/bifrost-ui.md` "Unified Logging"; code `springboard/core/src/log.rs`, `springboard/apps/console-desktop`; `bifrost pc log` |
| Agent access to Midget (MCP tools, deploy-app) | `docs/bifrost-mcp.md`; code `springboard/mcp/src/` |
| Reference SDK parity (index, coverage per SDK, gaps, semantic search) | `bifrost pc parity status\|coverage\|gaps\|show\|search`; data `docs/parity-matrix/`, page `docs/apple-parity-matrix.md`; skill `bifrost-parity-matrix` |
| Trademark name guard | `bifrost pc names` (tools/check-names.py, allowlist tools/check-names-allow.txt); no platform or product names in code |
| Motion, live glass, window effects | `springboard/render/src/motion.rs`, `src/gpu/wl.rs` (live backdrop, transforms), `src/shell/window_fx.rs`; review in `docs/gpu-first.md` |
| Browser end-to-end on Midget | `bifrost pc browser-smoke`; MCP tools `browser`, `term`, `files` (`docs/bifrost-mcp.md`) |
| Website (bifrost.olibuijr.com) | `~/Projects/bifrost-site/site/` (served live from disk; `bifrost site check|doctest|shot|ship`; docs chapters in `site/docs/_src/`; skill `bifrost-pc` "Website") |

Fast loop: `cargo build --offline --profile fast --bin shell --features desktop-wayland` (in `springboard/`), then
`tools/midget-push-shell.sh --no-build shot.png` to try it on Midget. Release: bump `springboard/Cargo.toml` +
`CHANGELOG.md`, commit, `bifrost pc publish`, `bifrost pc midget update --restart` (reboots; check
`python3 tools/midget.py recovery` says `phase=accepted`), `bifrost pc midget key enter` to unlock.
Every window takes input through `bifrost pc midget tap-window APP X Y` (window-local px; APP as `state` names it:
Settings, AppStore, External(N)), not `tap`: raw `tap` uses the shell's geometry, offset from the screenshot.

## Apple parity (standing rule)

When an SDK or kit API is missing something that Apple's framework has, expand ours to match it (the
`*Kit` docs on developer.apple.com are the reference). Record the gap and the decision in `docs/bifrost-ui.md`;
if Apple's behaviour cannot be matched yet (missing data, for example), say so explicitly rather than skipping it.

## Skills (read the matching one before deciding)

All skills below live in `~/.claude/skills/`. Pi sees `bifrost-pc` through a symlink at `~/.pi/agent/skills/bifrost-pc`; the others are read from `~/.claude/skills/` only.

| Skill | Path | Use it for |
|---|---|---|
| apple-hig | `~/.claude/skills/apple-hig/SKILL.md` (+ `references/`) | Apple HIG rules for macOS windows, menus, sidebars, toolbars, type, colour, materials |
| apple-appkit | `~/.claude/skills/apple-appkit/SKILL.md` (+ `references/`) | AppKit API questions and the Objective-C reference (not used by the Rust shell) |
| bifrost-hig | `~/.claude/skills/bifrost-hig/SKILL.md` | Review checklist (rule IDs LAY, CHR, TYP, COL, CTL, CNT, APP, GG for the Golden Gate kit) for shell and app UI changes |
| bifrost-pc | `~/.claude/skills/bifrost-pc/SKILL.md` | Build, publish, Midget control (`tap-window`), host previews (`bifrost pc smoke/gallery`), kit adoption, release order |
| bifrost-appearance | `~/.pi/agent/skills/bifrost-appearance/SKILL.md` | Switching Midget to Light, Dark or Auto (`bifrost pc midget appearance`) |
| bifrost-app-control | `~/.claude/skills/bifrost-app-control/SKILL.md` | Calling running PC apps over their control sockets |

Apple HIG and the Bifrost checklist are the design authority; cite the section (for example `macos-structure.md`, "The menu bar") when a decision rests on it. Codex's macOS plugin skills (`~/.codex/.tmp/plugins/plugins/build-macos-apps/skills/`, for example `liquid-glass`, `window-management`) are optional references for Liquid Glass and window behaviour.

## Working

- Read the code before changing it. Prefer real files and command output over memory.
- Make the smallest correct change. Keep changes focused on what was asked.
- Verify before claiming it works: run the focused tests, and for visual work compare a preview or a Midget
  screenshot against the design rules below. Report the commands and their results.
- Commit each finished change with a conventional message. If you are on `main`, create a feature branch first.
- Do not push, merge or deploy unless asked.
- **Parity status** (owner rule, 2026-10-11): parity with the reference SDKs is tracked with the `bifrost-parity-matrix`
  skill and `bifrost pc parity`. A change that adds or completes a capability updates its rows in
  `docs/parity-matrix/data/` in the same round (`check`, `build`, then the published database). Test on the host,
  then the x86 VM, then Midget, which is always the final testbed; a row is `parity` only once it works on Midget.
- **Release every iteration** (owner rule, 2026-10-09): each finished round is released without asking: bump the
  version and `CHANGELOG.md`, `bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`, then a Midget
  screenshot. Update the docs in the same round: `HANDOFF.md` (new top section), the feature's doc in `docs/`, and
  `docs/bifrost-ui.md` for SDK or Apple-parity changes.

## PC development

- Explicit PC or desktop work: read `docs/bifrost-pc-workflow.md` and `docs/build-performance.md`.
- GPU and rendering work: read `docs/gpu-first.md` (GPU-first Wayland presenter, Intel today, vendor neutral).
- Iterate with the optimized incremental `fast` profile (`cargo build --profile fast`, lld). Full LTO is opt-in.
- Run the focused crate tests and previews while iterating. Run the full acceptance once per batch before publishing.
- System changes ship with `bifrost pc publish`, then `bifrost pc midget update [--restart]` (or Settings >
  Software Update on a PC). No ISO rebuilds for ordinary system or app iteration.
- Apps: `tools/pc-app.py build|publish <dir>`, where `<dir>` is the `-desktop` app directory. Each release bumps
  the version in `manifest.toml` and `Cargo.toml` together and adds a `CHANGELOG.md` entry. Desktop packages
  are `architecture = "x86_64-linux"`.
- Installing a published app on Midget: `ssh midget` then `/usr/bin/springboard --install-store-app <id>`.
- Release = commit, publish, then update Midget, when asked to release.

## Lab

- Titan is 192.168.1.12 and the lab network is 10.99.0.1. Midget is 192.168.20.140 (VLAN 20 "Midget Lab" on port 8 of the US-8-60W, routed to the home LAN; Wi-Fi 192.168.1.187 as fallback). `ssh midget` from Titan and
  `ssh titan` as Midget root both work.
- Midget is the test machine. Recover it first; only the installer may erase it, and only Midget. Never wipe Titan.
- Midget controls: `bifrost pc midget state | screen PATH | tap-window APP X Y | tap X Y | swipe ... | key enter |
  open-app ID | ota | appearance light|dark | trace on|off`. `screen` is compositor pixels; `tap`/`swipe` use the
  shell's window geometry from `state`, which can be offset, so use `tap-window` with window-local pixels.

## Design (macOS-style PC)

- The PC presentation follows the user's macOS-style design: a menu bar, a dock, traffic-light windows,
  floating panels and sheets.
- Type: the PC desktop uses the macOS text styles in `springboard/render/src/ui.rs` (`MAC_*_PT`, from the HIG
  macOS table: large title 26, title 1 22, title 2 17, title 3 15, headline and body 13, callout 12, subhead 11,
  footnote 10, caption 10). Footers and group
  headers are footnote size, secondary grey. Do not add literal sizes. The `mac_type_scale_matches_the_hig_macos_table`
  test guards the values.
- Colour: use the named constants in `render/src/ui.rs` only. Text links and bar buttons are `LINK` (systemBlue).
  `DESTRUCTIVE` (systemRed) is for destructive actions only; the calendar's today marker is the one other red.
  Add a named constant instead of an ad hoc colour.
- Helpers: use `footer()` for footers and `large_title()` for root-list titles. Lists use the shared
  `shell/list.rs` rows.
- Navigation: root screens have a large title; detail screens have an inline title with a back control in `LINK`.
- Radii (2026-10-09 owner decision, replaces "every radius is at most 6"): window 17 pt, grouped cards 13 pt, sidebar selection plates 9 pt,
  Settings icon tiles 5 pt, search fields are capsules. Values live in `springboard/ui/src/prefs_layout.rs` (`m::*`), measured from the
  Golden Gate Settings reference (`docs/settings-layout-spec.md`). Small legacy controls keep 6 pt until restyled.
- Appearance: Light, Dark and Auto (light 07:00 to 19:00) with a 280 ms cross-fade (`render/src/appearance.rs`). Colours that follow it are
  `Pair`s in `render/src/ui.rs` resolved with `.now()`; text that follows it is rendered white and drawn with `Surface::blit_colored`.
- Materials: panels, menus and Spotlight are translucent. Blur is required and still missing on Spotlight.
- New apps look like stock macOS apps: sidebar or list navigation, one tint, and no custom dashboards.
- Bifrost will scale to any screen size. Do not design for one panel: sizes come from the named tokens and `ui.pt()`,
  layouts derive from the window size, and no new code assumes a fixed resolution, aspect ratio or pixel position.
- Visual changes are checked against a host preview (`bifrost pc smoke APP`, plus `--light`) or a Midget screenshot
  before merging. Apps draw their chrome with the SDK kit (`source_list`, `toolbar`, `search_field`, `path_bar`) and a
  sidebar app uses a full-size window (`window = "full-size"`); see `docs/golden-gate-sdk-adoption.md`.
