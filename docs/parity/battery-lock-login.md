# Battery, Energy, Lock Screen and login window (slice battery-lock-login)

Matrix entries this slice covers: `Battery`, `EnergySaver`, `EnergySaverPref`, `LockScreen`, `loginwindow`,
`ScreenSaverEngine` (all partial). Matrix JSON is not edited here; the lead closes rows.

Reference evidence: not measured in this run. The `ref27` extracts for loginwindow and the Battery pane were not
read; the behaviour below is taken from the existing BifrOSt code and its CHANGELOG entries.

## Rows

| Area | BifrOSt path | Done in this slice | Still needs Midget or a device check |
| --- | --- | --- | --- |
| Battery page: level, status, last charged, 24 h graph, health | `springboard/src/shell/pages.rs` (`PageId::Battery`), `src/bin/shell.rs` (`battery::health()` from sysfs), `shell/battery.rs`, `shell/low_battery.rs` | Already real before this slice (sysfs, sample history). Unchanged. | Health rows on a real laptop battery |
| Energy: sleep when display is off | `pages.rs` (`PageId::Energy`), `settings.rs` (`Pick::SystemSleep`), `bare/sleep.rs` | Already applied without restart (`Effect::SystemSleepAfter`). Unchanged. | Suspend on hardware |
| Lock Screen > Require Password | `options.rs` (`REQUIRE_AFTER`, `lock_grace_s`, `grace_lapsed`), `pages.rs` (`PageId::RequirePassword`, `Pick::RequirePassword`), `settings.rs` (pick handler), `mod.rs` (`display_off`, `screen_woke`) | New. The display-off lock (idle Auto-Lock and the power key) waits for the chosen grace; a wake inside it returns to the screen as it was, a wake after it shows the login window. Immediately keeps the old behaviour. Pref `lock_grace_seconds`. | Wake from the power key, touch and alarm inside and after the grace |
| Login window: Name and Password | `login.rs` (`name_mode`, `typed_name`, `name_focus`, `Msg::NameMismatch`, `Hit::Name`), `login.rs` (`login_ensure` sets the mode from `Options::login_name_password`) | New. The picture and name line give way to a name field above the password field. Return moves from name to password. A name that is not the account is refused before the password reaches the helper (no attempt counted). | Layout at 1200x800 and on a phone-sized display; keyboard focus |
| Login window: password hints | `options.rs` `login_hints` | Pref only. No hint is stored anywhere, so nothing is shown. | Needs a hint store (not built) |
| Login message | `login.rs` (`load_lock_message`) | Unchanged. The message is read from `lock-message` in the data directories; there is no Settings control to write it. | A Settings text control (not built) |
| Automatic login | `users_page.rs` (row shows "Disk unlock") and `login.rs` (`take_autologin`) | Unchanged. Automatic login is set by the loader from the disk unlock. There is no safe toggle to add here. | Decision on a user-facing switch |
| Screen Saver and lock | `screensaver.rs` | Unchanged. A saver does not lock by itself; the display-off lock and its Require Password grace apply when the display goes off. The footer text says "then the lock applies as usual", which is only true for the display-off path. | Whether a saver should also start the grace |

## Notes

- `shell/mod.rs` keeps one new field, `grace_from: Option<(f64, Screen)>`, set by `display_off` and cleared by
  `sleep_screen`. `lock_screen()` (the menu and the `lock` verb) still locks at once.
- Auto-Lock already reaches the shell through `apply_options` (`idle_off_ms`); the `Effect::AutoLock` that the shell
  logs is not the path that applies it.
