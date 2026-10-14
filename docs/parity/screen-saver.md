# Screen Saver parity

Branch `parity/team-n-screen-saver`, cut from `arm`. Matrix rows: `docs/parity-matrix/data/screen-saver.json`.
Not yet built, tested or run on Midget; the lead builds and checks.

## What this branch adds

- `springboard/sdk/src/screen_saver.rs`: the SDK path for the screen saver model in `bifrost_ui::screensaver`.
  It re-exports `ScreenSaverView`, `BackingStoreType`, `ConfigureSheet`, `SaverValue`, `preview_rect`, the
  `ss_random_*` helpers and `ss_centered_rect_in_rect`. `SaverRunner` wraps a view and advances it from the caller's
  clock, counting each due frame with `animate_one_frame`.
- `springboard/sdk/src/idle_watch.rs`: `IdleWatch`, an app-facing wrapper over `bifrost_core::idle_monitor`. It
  queues `BecameIdle` and `BecameActive` so an app can read changes between frames with `drain_events`.
- `springboard/sdk/src/lib.rs`: two module declarations.

Unit tests cover the frame schedule, the catch-up cap, stopped runners, the event queue and timeout changes.

## Still missing

- No `.saver` bundle host and no screen saver engine. The shell drives its own Clock, Colours and Black savers.
- No configuration sheet in Settings. `ConfigureSheet` checks option values but nothing shows it.
- Apps do not receive the shell's last-input time. `IdleWatch` works only with the times its caller passes in.
- Display-sleep holds have no app option. The SDK activity tracker in `springboard/sdk/src/process_info.rs` covers
  system sleep only.

## Integration notes

- `ScreenSaverDefaults` in `springboard/sdk/src/screen_saver_defaults.rs` already exists and is not changed here.
- Settings and the shell are not touched. Wiring `IdleWatch` to the shell's last-input time needs a control message
  in `springboard/proto`, which is a separate change.
