# Control Center and menu bar extras (slug: control-center)

Round 75. Base: `wave-base` (3355822c). Reference artifacts under `~/Projects/macos-ref/extract/ref27/` were not
consulted in this run; behaviour comes from the existing shell code and the shared menu extras model.

## Done

| Area | BifrOSt path | State |
| --- | --- | --- |
| Clock menu options (show date, show weekday, 12/24-hour) | `springboard/ui/src/menu_extras.rs` (`ClockStyle::text`), `springboard/src/shell/options.rs` (`clock_date`, `clock_day`, `clock_style()`), `springboard/src/shell/desktop_bar.rs` (`clock_text`), `springboard/src/shell/mod.rs` (`clock` field replaces `clock_24h`), `springboard/src/shell/pages.rs` and `settings.rs` (Date & Time switches) | Wired end to end; needs a Midget check of the bar with the date hidden |
| Battery menu shows the charge level and charging state | `springboard/src/shell/desktop_bar.rs` (`Menu::Power` rows) | Wired; the power-profile list keeps its "unavailable" fallback |

## Still open (not in this round)

- Control Center modules `Nearby`, `Mirror` and `Timer` are tap no-ops in `activate_tile`
  (`springboard/src/shell/control_center.rs`). No nearby beacon and no mirroring backend is wired yet.
- Stage Manager and tiling modules have no tile in the Control Center.
- Drag-to-menu-bar for Control Center modules is not built; the module order is only set through
  `MODULES_PREF` (`control_center_modules`).
- Menu bar extras: the shell still has its own `options::MenuExtra` (CPU, Memory, Battery, Network, Control Center,
  u8 bits) beside the shared `bifrost_ui::menu_extras::SystemExtra` (12 extras, u16). Migrating the bar to
  `SystemExtra` and deleting `MenuExtra` is open; it needs a decision on CPU and Memory, which have no `SystemExtra`.
- `ClockStyle::show_seconds` and `flash_separator` are model fields only; the shell does not draw them yet.
- Now Playing, the sound output picker, the AirDrop-style Nearby panel and the display brightness module already
  exist in `control_center.rs` and were not changed.

## Needs Midget checking

- Menu bar clock width with the date and weekday hidden, and with 12-hour time.
- Battery menu row "Battery NN%, charging" on a machine with a battery.
