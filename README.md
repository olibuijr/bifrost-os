<div align="center">

# BifrOSt for PC

**A macOS-style desktop for Linux, written in Rust.**

[![Licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/language-Rust-orange.svg)](springboard/Cargo.toml)

<img src="springboard/wallpapers/waterfall_idyllic.jpg" alt="The default BifrOSt wallpaper: a waterfall in Iceland" width="720">

</div>

BifrOSt runs as a full-screen shell on Wayland or directly on DRM/KMS. It ships with a native installer,
a suite of desktop apps, and an SDK and UI kit for building more.

- [Features](#features)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Fonts](#fonts)
- [Repository layout](#repository-layout)
- [Documentation](#documentation)
- [Tooling](#tooling)
- [Contributing](#contributing)
- [Licence and credits](#licence-and-credits)

## Features

- **Desktop shell.** A menu bar, a dock, windows with traffic-light controls, floating panels, sheets and
  system search.
- **Light, Dark and Auto appearance.** Auto switches between light and dark by time of day, with a
  cross-fade between the two.
- **GPU-first rendering.** The wallpaper, blur and glass materials are drawn on the GPU. See
  [`docs/gpu-first.md`](docs/gpu-first.md).
- **Native installer.** A DRM/KMS installer that runs without a desktop session, with an archiso profile for
  building the installer ISO.
- **Desktop apps.** About thirty apps under [`springboard/apps/`](springboard/apps), including Files, Browser,
  Music, Photos, Notes, Calendar, Contacts, Reminders, Weather and a DAW.
- **SDK and UI kit.** The `springboard/sdk` and `springboard/ui` crates give apps the same controls, text
  and testing support as the shell.
- **Royalty-free wallpapers.** Six photographs of Iceland, credited in [CREDITS.md](CREDITS.md). A fresh install
  starts on one of them at random.

## Requirements

- Linux. The shell targets Wayland (with `wlr` protocols) and DRM/KMS.
- A recent stable Rust toolchain (`rustup`).
- Network access on the first `tools/pack.py` run, to fetch the open-source UI fonts. See [Fonts](#fonts).

## Quick start

```sh
git clone https://github.com/olibuijr/bifrost-os.git
cd bifrost-os/springboard

# Optimised development build of the shell
cargo build --profile fast --bin shell --features desktop-wayland

# Unit and integration tests
cargo test
```

Several tests and previews load the UI fonts from `springboard/assets`, so pack the assets first (see [Fonts](#fonts)).

## Fonts

BifrOSt ships only open-source fonts (SIL Open Font Licence 1.1). The defaults are the closest open lookalikes
of the macOS system fonts, and Settings > General > Fonts can replace the system and monospaced fonts with
others from BifrOSt's font library.

**Defaults** (packed under the asset names the code loads):

| Asset file | Font | Stands in for | Why |
|---|---|---|---|
| `SFUI.ttf` | [Inter](https://rsms.me/inter/) | SF Pro | Variable, with both a weight (`wght`) and an optical size (`opsz`) axis, like SF Pro; the closest open match |
| `SFUIMono.ttf` | [JetBrains Mono](https://www.jetbrains.com/lp/mono/) | SF Mono | Variable (`wght` 100 to 800), with a neutral, screen-first design like SF Mono |
| `Symbols.ttf` | [Noto Sans Symbols 2](https://fonts.google.com/noto/specimen/Noto+Sans+Symbols+2) | Apple Symbols | Maps symbols by Unicode, as the shell looks them up; resolves 18 of the shell's 22 symbols |

No open-source font of SF Symbols exists. The shell falls back to `Symbols.ttf` and its hand-drawn glyphs
(Wi-Fi, battery and gear are hand-drawn).

**Font library** (`fonts/`, offered in Settings > Fonts): Geist, IBM Plex Sans and Source Sans 3 for the system
font; Geist Mono, Fira Code and Source Code Pro for the monospaced font. All are variable fonts, so every weight
the desktop uses renders. Fonts that apps install go in the same folder (`/var/lib/springboard/fonts`). Settings
lists only this library, never the host's or the distribution packages' fonts. A choice is saved in
`settings.ini` (`font_system`, `font_mono`), takes effect when BifrOSt restarts, and applies to every app;
a font that fails to load falls back to the default.

`springboard/tools/pack.py` fetches all of these from one pinned [google/fonts](https://github.com/google/fonts)
commit, checks each file's SHA-256 and caches them in `~/.cache/bifrost/fonts`:

```sh
python3 springboard/tools/pack.py ../bifrost-ref <assets-dir> <screen-width> <screen-height>
```

Notes:

- The pixel snapshot goldens in `springboard/sdk/tests/snapshots/` are recorded with the default fonts.
- The renderer test `cff_font_renders_through_the_same_path` needs a CFF symbol font
  (`assets/SFUISymbols-Regular.otf`) and is skipped without one; no default font uses CFF outlines.
- Icons and UI sounds still come from the local `bifrost-ref/` folder (git ignores it).

## Repository layout

| Path | Contents |
|---|---|
| [`springboard/`](springboard) | The Rust workspace: shell, installer, SDK, UI kit, renderer and apps |
| [`springboard/apps/`](springboard/apps) | Desktop apps, one crate per app |
| [`springboard/wallpapers/`](springboard/wallpapers) | Royalty-free wallpapers, packed for the shell by `tools/pack.py` |
| [`packaging/`](packaging) | Arch Linux packaging, the archiso profile for the installer ISO, and branding |
| [`tools/`](tools) | Development tools: the `bifrost` CLI, app packaging, Midget lab helpers, checks and benchmarks |
| [`docs/`](docs) | Design notes, SDK adoption guide, UI kit reference and workflow docs |
| [`android/midget-remote/`](android/midget-remote) | Companion Android app for controlling a BifrOSt machine |
| [`task/`](task) | Planned work items |

## Documentation

| Topic | Read |
|---|---|
| Rendering and materials | [`docs/gpu-first.md`](docs/gpu-first.md) |
| Design decisions and the macOS-style reference | [`docs/design-golden-gate.md`](docs/design-golden-gate.md) |
| SDK and UI kit | [`docs/bifrost-ui.md`](docs/bifrost-ui.md) |
| Moving apps to the SDK kit | [`docs/golden-gate-sdk-adoption.md`](docs/golden-gate-sdk-adoption.md) |
| Installer and boot | [`docs/installer.md`](docs/installer.md), [`docs/installer-and-boot.md`](docs/installer-and-boot.md) |
| Building, publishing and the test machine | [`docs/bifrost-pc-workflow.md`](docs/bifrost-pc-workflow.md) |

## Tooling

`tools/bifrost.py` is the development CLI. Run it from the repository root:

```sh
python3 tools/bifrost.py pc acceptance --quick    # tests, SDK coverage and app checks
python3 tools/bifrost.py pc names                 # trademark name guard
```

`tools/midget*.py` and `tools/midget-*.sh` drive the Midget lab machine.

## Contributing

Conventions are in [`AGENTS.md`](AGENTS.md) (also available as `CLAUDE.md`): commit messages, the design rules
and the review checklist. Before opening a pull request:

1. Keep the change focused on one thing.
2. Run the tests for the crate you touched, and `python3 tools/check-names.py` if you added text to the code.
3. Describe what you verified, and attach a screenshot for visual changes.

## Licence and credits

The code is released under the [MIT licence](LICENSE).

Photographs are royalty-free and credited in [CREDITS.md](CREDITS.md). The fonts are open source under the
SIL Open Font Licence 1.1 and are credited in [CREDITS.md](CREDITS.md) and described in [Fonts](#fonts).
