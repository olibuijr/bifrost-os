# Settings as an OTA-updatable System App (plan, 2026-10-09)

Goal: Settings ships as its own package, so a Settings update arrives through the App Store/OTA path without restarting the shell
(today it is compiled into `springboard`, ~6,600 lines in `springboard/src/shell/{settings,pages,list,...}.rs`).

## Why it is not a move-the-files job
Settings reads and writes shell state directly: `Prefs` (settings.ini), `Info` (network, battery, display), `Effect`s (Wi-Fi, Bluetooth,
dock, power, update), the appearance/accent atomics, the dock model and the desktop window. An app process has none of that.
The SDK (`bifrost_app`, `proto`) today is pixels plus app-level messages (`ToShell`/`FromShell`), with no system access.

## Steps
1. **Host trait.** Put every shell touch point behind `trait SettingsHost { prefs, info, effects, dock, ... }` inside the shell and make
   `Settings` depend on it only. No behaviour change; tests keep passing.
2. **Crate.** Move `settings*.rs`, `pages.rs`, `list.rs`, `row_glyphs.rs`, `controls.rs` pieces it owns into `springboard/apps/settings`
   (workspace member, builds with `render` + `ui` + `sdk`). `list.rs` and `controls.rs` are shared, so they move to `ui`/`render` first.
3. **System channel.** Add a privileged request/response pair to `proto` (`ToShell::System(Request)` / `ToApp::System(Reply)`) or a
   `bifrost-systemd` socket, covering exactly what the `SettingsHost` trait needs; only apps with `kind = "system"` may use it.
4. **Package.** `manifest.toml` with `kind = "system"`, `category = "System"`, no Remove in the App Store; installed at first setup
   with the other dock apps; `tools/pc-app.py publish`. The shell's Settings dock icon launches the app.
5. **Hot update.** The app store updater replaces the package; the running Settings app is told to quit (`ToApp::Quit`) and relaunches.
   No shell restart because the shell no longer contains Settings.
6. Remove the built-in Settings from the shell once the app reaches parity (keep a minimal fallback for recovery).

Risks: Settings preview/test harness (`preview_settings`) must move with it; the sidebar/pane layout uses shell scale (`ui.pt`), which the
SDK already provides; keyboard and clipboard come through the existing app protocol.
