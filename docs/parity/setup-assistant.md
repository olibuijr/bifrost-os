# Setup Assistant (first run after install)

Slug `setup-assistant`. The first-run window that follows the installer: time zone, keyboard, network, accessibility
shortcut, the installed account, appearance, privacy, BifrOSt Account sign-in, then the desktop.

## Evidence used (clean room)

- Reference bundle extracted at `~/Projects/macos-ref/extract/ref27/System/Library/CoreServices/Setup Assistant.app`
  (index rows in `~/Projects/macos-ref/extract/ref27-index.tsv`, 65 rows under `Contents/Resources/Base.lproj`).
  The page set was read from the nib file names only: Welcome View, TimeZoneView, CountryController, MultilingualKeyboardController,
  SelectSource, SelectWirelessNetworkView, CreateUserAccountView, MBAppearanceViewController, PrivacyController,
  CustomizeDataView, iCloudLoginView, DoneView, Summary View.
- No REA decode (`decode_interface_builder`, `inspect_plist`) was run in this round. Nib contents, strings and artwork are
  not copied into the repo; the behaviour below is BifrOSt's own design, shaped by the page set above.
- The BifrOSt Account client: `grep -rn cloud springboard/src springboard/apps springboard/sdk` finds no client
  library. The web account page lives in `~/Projects/bifrost-cloud` (bifrost.olibuijr.com/account/). Sign In therefore
  opens that page in the browser; there is no native sign-in yet.

## Rows

| Entry (`tools/parity/os_inventory.py`) | BifrOSt path | Done | Needs Midget check |
|---|---|---|---|
| Setup Assistant | `springboard/src/setup.rs` (model, state file, shell apply), `springboard/apps/setup-assistant/` (window), `springboard/src/shell/mod.rs` `set_data_dir` (open once, apply once) | The eleven steps, the choices, the state file, the one-time copy into `settings.ini` (appearance, accent, accessibility shortcut, location, analytics), the time zone through `privd_client::Client::set_timezone`, the Sign In browser hand-off, the open-once rule | The window opens at the first login after install; every step paints and taps; Get Started writes `setup.ini`; the next start does not open it again; the look chosen is the look the desktop takes |

## What works, what is a model

Works in code (host unit tests only, not built in this round):

- `springboard::setup`: `Step` order and navigation, `Choices` defaults, `State` text round trip, damaged-value fallback,
  zone and locale probes, interface `online` rule, `apply_pending` copies once and marks `applied=1`.
- `apps/setup-assistant/src/window.rs`: every control of every step as a rectangle; `act_at` is the tap rule; narrow-width
  bounds; zone and keyboard chips; appearance modes and accents; switches; the Get Started footer on the last step.
- `apps/setup-assistant/src/main.rs`: taps fire on release over the same control as the press; Return continues, Escape
  goes back; Get Started applies the zone through privd (only when it differs from the current zone), writes the state,
  opens the account page for Sign In, and exits.

Still a model (no effect on the system yet):

- Keyboard layout: the choice is saved in `setup.ini` but not applied. The shell's layout model (`shell/kb_layouts.rs`)
  has only Icelandic and English and no pref key; the installer writes the keyboard config at install time only.
- Language: read-only confirmation of the install-time locale.
- Network: shows the state of non-loopback interfaces; it does not join a network. Wi-Fi joins stay in Settings.
- Accessibility shortcut: the choice reaches `settings.ini` under `accessibility_shortcut`; no gesture reads it yet.
- Location Services and Analytics & Diagnostics: the choices reach `settings.ini` under `location_services` and
  `diagnostics_sharing`; no code reads them yet, so the Privacy pane does not reflect them.
- BifrOSt Account: Sign In opens the web account page; there is no native sign-in or device registration.
- The window has no published accessibility nodes yet (`controls` is empty), so Orca and AT-SPI see no buttons.

## Gaps and decisions for the lead

- Existing machines without `setup.ini` (for example Midget after an OTA update) will see the assistant once at the next
  start. To skip it on a machine, write `done=1` and `applied=0` to `/var/lib/springboard/setup.ini`.
- The window is an SDK app (`bifrost-setup-assistant`, id `is.olibuijr.setup.desktop`). Its exit status 0 after Get Started
  closes it; the shell's app host treats that as a normal end.
- `Cargo.lock` needs the new workspace member (`apps/setup-assistant`); a build without `--locked` updates it.
- The shell hook runs in `set_data_dir`, after `apphost::register` (`src/bin/shell.rs` line 115). The app opens through
  `apphost::slot_of` and `Shell::open`, the same path as the Calendar and Weather actions.

## Shared-file edits (lead to merge with care)

- `springboard/src/lib.rs`: one line, `pub mod setup;`.
- `springboard/Cargo.toml`: `apps/setup-assistant` added to the workspace members.
- `springboard/src/shell/mod.rs`: two small blocks in `set_data_dir`: `apply_pending` right after the `settings.ini` load,
  and the open-once check right before the clock load.

## Migrations and deletions

- Installer look, language and keyboard pages stay where they are (install time). The assistant pre-fills from the same
  values: the keyboard from `installer::app::layout_for_locale`, the look from `settings.ini`, the account from
  `shell::account::load`. Nothing was deleted.
