# Wire 3: Control Center (Focus, now playing, brightness, sound output, power)

Branch `wire3/control-center`, from main 0.5.38. Not built or run here (no builds in this round; the lead builds once).

## What is now wired

Most of the Control Center models in this area were already wired by earlier waves. This round checked each one and
wired the one that was still a pure model:

| Area | State on main | Where |
|---|---|---|
| Focus / Do Not Disturb (Control Center tile, Settings > Focus) | Already wired: the tile and Settings call `Shell::set_dnd`, which persists and silences banners. | `springboard/src/shell/focus.rs`, `control_center.rs` (`Tile::Focus`) |
| Focus filters for apps (App Intents) | **Wired in this round.** The model `bifrost_ui::focus_filter` now has a shell service `focus_filter`: apps store values per Focus, read the values for the Focus that is on, and get a `changed` event when Do Not Disturb turns on or off. | `springboard/src/shell/services/focus_filter.rs`, `springboard/sdk/src/focus_filter.rs`, `springboard/ui/src/focus_filter.rs` (`pairs()` added) |
| Now Playing (app sessions, transport) | Already wired by `wire-svc-now-playing` (service `now_playing`, Control Center media tile). Unchanged here. | `services/now_playing.rs` |
| Display brightness | Control Center brightness slider already drives the backlight (`Tile::Brightness`, `set_slider`). Unchanged here. | `control_center.rs` |
| Sound output (devices, default per role, volume, mute, route events) | Already wired by `wire-svc-audio` (service `audio-devices`). Unchanged here. | `services/audio_devices.rs` |
| Power (thermal state, Low Power Mode, battery) | Already wired by `wire-svc-power` (service `power`). Unchanged here. | `services/power_service.rs` |

The Control Center panel itself is not changed in this round: the Focus tile already reflects the state the new service reports.

### The Focus filter service

Service name `focus_filter`. One `FocusFilters` per app (the model), keyed by the app id. The Focus that is on is
`"Do Not Disturb"` (`DO_NOT_DISTURB`) when Do Not Disturb is on, otherwise none.

| Method | Params | Reply |
|---|---|---|
| `set_context` | `{focus: string, values: {key: string}}` | `null`; errors: missing or empty focus, a value that is not text |
| `remove_context` | `{focus: string}` | `{removed: bool}` |
| `current` | none | `{focus: string\|null, values: {key: string}\|null}` (the caller's values for the Focus that is on) |

Event `changed` `{focus: string|null}` to each app that has stored a context, when the Focus that is on changes (one per
change; the first tick only records the state).

SDK client `bifrost_app::focus_filter`: `set_context`, `remove_context`, `current` (each returns the request id),
`decode_current`, `decode_removed`, `changed_focus` (returns `Some(Some(focus))` or `Some(None)` for a `changed` event
of this service), and the constant `DO_NOT_DISTURB`. The model addition is `AppContext::pairs()`, which the reply needs
to list the values.

## How a user reaches it

- Turn on Do Not Disturb from the Focus tile in Control Center, or from Settings > Focus.
- An app that stored values for "Do Not Disturb" gets its values from `current` and a `changed` event, so it can
  adapt (for example, hide a chat list or mute its own sounds) while the Focus is on.
- There is no user-facing editor for an app's values yet; the app sets them itself.

## How to verify on the test PC (Midget)

Midget only (no VM).

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. `bifrost pc midget state` shows the desktop. Note whether Do Not Disturb is off.
3. From an app that calls `focus_filter::set_context(ctx, DO_NOT_DISTURB, &[("mode", "Quiet")])`, then `current`:
   the reply has `focus: null`, `values: null` while Do Not Disturb is off.
4. Open Control Center with a swipe from the top-right and tap the Focus tile. The app receives `changed` with
   `focus: "Do Not Disturb"`; then `current` returns `values: {mode: "Quiet"}`.
5. Tap the Focus tile again: the app receives `changed` with `focus: null`.
6. Screenshot with `bifrost pc midget screen PATH`; the Focus tile must show the on state (the purple tile) when on.

Host checks (run by the lead): `cargo test -p springboard focus_filter`, `cargo test -p bifrost-app focus_filter`,
`cargo test -p bifrost-ui focus_filter` (the model's `pairs` assertion).

## REA evidence

- `ev_97db0a7181c1e9134ff73116e80426531cc5a799ce78c06eb882794e04b45d99`: rendered official page
  `https://developer.apple.com/documentation/appintents/setfocusfilterintent` (via `tools/parity/rea-page.sh`, read
  this round). It states the protocol is "an interface for providing an app intent that you use to adapt your app's
  behavior when Focus changes", with `current`, `suggestedFocusFilters(for:)`, `appContext` and
  `invalidateFocusFilterAppContext()`.
- DocC symbol facts: `python3 -I tools/parity/docc.py appintents/setfocusfilterintent --depth 1` (the same four members;
  macOS 13.0 and later).
- Earlier, for the Now Playing, audio and power rows: see `wire-svc-now-playing.md`, `wire-svc-audio.md` and
  `wire-svc-power.md` (their evidence ids are recorded there).

## Matrix rows this round moves

The lead updates `docs/parity-matrix/` (not edited here).

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| app-intents | Focus filters | springboard/ui/src/focus_filter.rs, springboard/src/shell/services/focus_filter.rs, springboard/sdk/src/focus_filter.rs |

The row can move from `partial` to `parity` only after the Midget check above passes. Its note still reads "apps cannot
declare filter intents"; the declaration side is a gap (below).

## Remaining gaps

- Apps cannot declare a filter intent (`SetFocusFilterIntent`), so the system cannot offer the app's filter in Settings.
  The service stores values the app chooses for itself.
- Only one Focus exists: "Do Not Disturb". Scheduled Focus names, Work or Personal Focus, and Focus suggestions
  (`suggestedFocusFilters`) are not modelled. The Settings schedule still only turns Do Not Disturb on and off.
- There is no user interface to set an app's values. A user cannot see or edit another app's Focus values.
- `invalidateFocusFilterAppContext()` has no method; the app sees the next Focus change, not a request from itself.
- The `focus_filter` values are kept in memory, not on disk: they are lost when the shell restarts. Do Not Disturb itself
  is persisted (`prefs`), so after a restart the Focus is on, with no app values until each app sets them again.
- Control Center brightness, sound output, power and now-playing gaps are the ones listed in their own wire notes
  (for example, the lock-screen seek is still not routed, and Control Center has no output route picker).
- Not built or run here (no builds in this round). A build, host tests and a Midget run are still needed.
