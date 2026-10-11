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
- [Fonts and licensed assets](#fonts-and-licensed-assets)
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
- UI fonts for a working desktop: the free open-source set below, or your own licensed copies. See
  [Fonts and licensed assets](#fonts-and-licensed-assets).

## Quick start

```sh
git clone https://github.com/olibuijr/bifrost-os.git
cd bifrost-os/springboard

# Optimised development build of the shell
cargo build --profile fast --bin shell --features desktop-wayland

# Unit and integration tests
cargo test
```

Several tests and previews load the UI fonts, so install the fonts first (see the next section).

## Fonts and licensed assets

The UI is designed around the macOS system fonts, which are proprietary and are not distributed here. Free,
open-source lookalikes work in their place, and they are the recommended setup for a fresh clone:

| Asset file | Proprietary original | Open-source lookalike | Licence |
|---|---|---|---|
| `SFUI.ttf` | SF Pro (variable) | [Inter](https://rsms.me/inter/) 4.1, `InterVariable.ttf` | [OFL 1.1](https://github.com/rsms/inter/blob/master/LICENSE.txt) |
| `SFUIMono.ttf` | SF Mono | [JetBrains Mono](https://www.jetbrains.com/lp/mono/) 2.304, `JetBrainsMono[wght].ttf` | [OFL 1.1](https://github.com/JetBrains/JetBrainsMono/blob/master/OFL.txt) |
| `Symbols.ttf` | Apple Symbols | [Noto Sans Symbols 2](https://fonts.google.com/noto/specimen/Noto+Sans+Symbols+2) | [OFL 1.1](https://github.com/google/fonts/blob/main/ofl/notosanssymbols2/OFL.txt) |
| `SFUISymbols-Regular.otf` | SF Symbols | none (optional; see below) | |

Why these:

- **Inter** is the closest open match to SF Pro. Like SF Pro it is a variable font with both a weight (`wght`)
  and an optical size (`opsz`) axis, so the renderer's weight and text/display instances work unchanged. Values
  outside Inter's ranges (`opsz` 14 to 32) are clamped.
- **JetBrains Mono** is a variable monospace font (`wght` 100 to 800) with a similar neutral, screen-first design
  to SF Mono. Fira Mono and Source Code Pro are close alternatives.
- **Noto Sans Symbols 2** maps symbols by Unicode, which is how the shell looks them up in `Symbols.ttf`. It
  resolves 18 of the shell's 22 symbols, more than Apple Symbols does. Wi-Fi, battery and the gear fall back to
  the hand-drawn glyphs.
- There is no open-source font of SF Symbols. The shell looks those glyphs up by name, finds none, and uses
  `Symbols.ttf` and the hand-drawn glyphs instead, so leave `SFUISymbols-Regular.otf` out.

Fetch them into a local `bifrost-ref/fonts/` folder at the repository root (git ignores it), under the asset
file names the code expects:

```sh
mkdir -p bifrost-ref/fonts && cd bifrost-ref/fonts
curl -LO https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
unzip -j Inter-4.1.zip InterVariable.ttf && mv InterVariable.ttf SFUI.ttf
curl -LO https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
unzip -j JetBrainsMono-2.304.zip 'fonts/variable/JetBrainsMono[[]wght].ttf' && mv 'JetBrainsMono[wght].ttf' SFUIMono.ttf
curl -L -o Symbols.ttf https://github.com/google/fonts/raw/main/ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf
rm *.zip
```

If you hold licences for the original fonts, put your copies in the same folder under the same names instead.
The expected file names are listed in [`springboard/tools/pack.py`](springboard/tools/pack.py).

The shell's assets are built from that folder:

```sh
python3 springboard/tools/pack.py ../bifrost-ref <assets-dir> <screen-width> <screen-height>
```

Notes:

- Inter and JetBrains Mono have different metrics from the originals, so text is set slightly wider. The pixel
  snapshot goldens in `springboard/sdk/tests/snapshots/` were recorded with the original fonts, so expect those
  comparisons to differ.
- The renderer test `cff_font_renders_through_the_same_path` (`springboard/render/src/font.rs`) exercises the
  CFF outlines of `SFUISymbols-Regular.otf` and needs the original file; the CFF parser test skips without it.
- Without any fonts, the code still compiles, but text does not render and many tests fail.

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

Photographs are royalty-free and credited in [CREDITS.md](CREDITS.md). The original UI fonts are proprietary
and are not included; the recommended open-source replacements (Inter, JetBrains Mono, Noto Sans Symbols 2) are
under the SIL Open Font Licence 1.1 and are fetched separately, as described in
[Fonts and licensed assets](#fonts-and-licensed-assets).
