# BifrOSt for PC

BifrOSt is a desktop environment for Linux, written in Rust. It runs as a full-screen shell on Wayland or
directly on DRM/KMS, and ships with a native installer, a set of desktop apps and a development toolkit
(SDK and UI kit) for building more.

The design follows a macOS-style layout: a menu bar, a dock, windows with traffic-light controls, floating
panels, sheets and light/dark appearance. The rendering is GPU-first, with materials (blur and glass) computed
on the GPU.

## What is in the repository

| Path | Contents |
|---|---|
| `springboard/` | The Rust workspace: the desktop shell, installer, SDK, UI kit, renderer, and the desktop apps under `apps/` |
| `springboard/wallpapers/` | Royalty-free wallpapers (see [CREDITS.md](CREDITS.md)) |
| `packaging/` | Arch Linux packaging, the archiso profile for the installer ISO, and branding assets |
| `tools/` | Development tools: the `bifrost` CLI, app packaging, Midget lab helpers, benchmarks and checks |
| `docs/` | Design notes, the Golden Gate SDK adoption guide, the UI kit reference and workflow docs |
| `android/midget-remote/` | A companion Android app for controlling a BifrOSt machine |
| `task/` | Planned work items |

## Building

You need a recent stable Rust toolchain. Most development happens in the optimised `fast` profile:

```sh
cd springboard
cargo build --profile fast --bin shell --features desktop-wayland
cargo test
```

Several tests and previews read the UI fonts (see below), so they need the licensed files in place first.

## Fonts and licensed assets

The UI uses proprietary system fonts that are not part of this repository. To build a working desktop,
place the font files in the licensed reference folder `bifrost-ref/` (see `tools/pack.py` for the
expected names), or point the tools at your own copy. Without them, text will not render, although the
code still compiles.

## Contributing

Read [AGENTS.md](AGENTS.md) (also linked as `CLAUDE.md`) for the conventions: commit style, the name
guard (`tools/check-names.py`), the design rules and the review checklist. Keep changes focused and
run the tests for the crate you touched before opening a pull request.

## Licence

The code is released under the [MIT licence](LICENSE). Photographs and third-party material are credited in
[CREDITS.md](CREDITS.md).
