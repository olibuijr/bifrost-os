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
- **Royalty-free wallpapers.** Six photographs with credits in [CREDITS.md](CREDITS.md).

## Requirements

- Linux. The shell targets Wayland (with `wlr` protocols) and DRM/KMS.
- A recent stable Rust toolchain (`rustup`).
- The licensed UI fonts for a working desktop. See [Fonts and licensed assets](#fonts-and-licensed-assets).

## Quick start

```sh
git clone https://github.com/olibuijr/bifrost-os.git
cd bifrost-os/springboard

# Optimised development build of the shell
cargo build --profile fast --bin shell --features desktop-wayland

# Unit and integration tests
cargo test
```

Several tests and previews load the UI fonts, so they fail on a clone without the licensed files.

## Fonts and licensed assets

The UI uses proprietary system fonts, which this repository does not distribute. Keep your licensed copies in a
local `bifrost-ref/` folder at the repository root (git ignores it), with the fonts under `bifrost-ref/fonts/`.
The expected file names are listed in [`springboard/tools/pack.py`](springboard/tools/pack.py).

The shell's assets are built from that folder:

```sh
python3 springboard/tools/pack.py ../bifrost-ref <assets-dir> <screen-width> <screen-height>
```

Without the fonts, the code still compiles, but text does not render and some tests fail.

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

Two groups of scripts need hardware or a second project:

- `tools/tablet/` drives a Samsung Tab A8 over adb. Those scripts take their device address from
  `ANDROID_SERIAL`, and two of them also need the Tab A8 `tools/tab.py`, which lives outside this repository.
  See [`tools/tablet/README.md`](tools/tablet/README.md).
- `tools/midget*.py` and `tools/midget-*.sh` drive the Midget lab machine.

## Contributing

Conventions are in [`AGENTS.md`](AGENTS.md) (also available as `CLAUDE.md`): commit messages, the design rules
and the review checklist. Before opening a pull request:

1. Keep the change focused on one thing.
2. Run the tests for the crate you touched, and `python3 tools/check-names.py` if you added text to the code.
3. Describe what you verified, and attach a screenshot for visual changes.

## Licence and credits

The code is released under the [MIT licence](LICENSE).

Photographs are royalty-free and credited in [CREDITS.md](CREDITS.md). The UI fonts are proprietary and are
not included.
