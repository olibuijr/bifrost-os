# Wire 4: system notification bus (Darwin Notify rows)

## What is now wired

The name-keyed notification model in `springboard/core/src/notify_center.rs` (tokens, polling marks, per-name state,
suspend and resume) is reached through a shell service and an SDK client.

1. **Shell service `notify_bus`** (`springboard/src/shell/services/notify_bus.rs`). It owns one `NotifyCenter` and a
   record of each token's owner app. Calls: `post`, `register` (mode `event` or `poll`), `check`, `suspend`,
   `resume`, `cancel`, `valid`, `set_state`, `state`. A token call is answered only for the app that owns the token.
   Limits: names up to 128 bytes with no control characters; 64 tokens per app; state values are whole numbers up to
   2^53. Names that start with `system.` can be posted and set only by the shell.
2. **Event delivery.** An event token gets a `notified` event (body `{token, name}`) on the tick after a post. Several
   posts before one tick make one event, as the mark is a flag.
3. **System names posted by the shell** (after the first tick, which only records the state):
   - `system.appearance.change` when the scheme goes Light or Dark (`bifrost_render::appearance::target_is_light`);
   - `system.screen.locked` and `system.screen.unlocked` when `Shell::is_locked` changes;
   - `system.network.change` when `Info::online` or `Info::wired` changes.
4. **SDK client** (`springboard/sdk/src/notify_bus.rs`, `bifrost_app::notify_bus`). Request functions `post`,
   `register(ctx, name, Mode)`, `check`, `suspend`, `resume`, `cancel`, `valid`, `set_state`, `state`, reply readers
   `token_from_reply`, `flag_from_reply`, `state_from_reply`, and the event reader `notified(service, event, body)`.
   The system names are constants there too.

No wire protocol change: the calls travel as `ToShell::ServiceCall` and the events as `ToApp::ServiceEvent`.
Shared-file edits: `springboard/src/shell/services/mod.rs` (one `pub mod` line, one `Box::new` line) and
`springboard/sdk/src/lib.rs` (one `pub mod` line).

## How a user reaches it

A user does not reach this directly: it is an interface for apps. An app calls
`bifrost_app::service::call` through `notify_bus::register` and reads the reply in `App::service_reply`
(`notify_bus::token_from_reply`), then handles `App::service_event` with `notify_bus::notified`. No shipped app calls
the service yet, so the user-visible effect is in the apps that adopt it (for example a Settings or Files
refresh on `system.appearance.change`). The shell's side (the system names) runs for every session.

## How to verify on Midget

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`.
2. Host tests (not run in this round, the lead builds once): `cargo test -p springboard --lib notify_bus` covers the
   service rules and the system-name diff; `cargo test -p bifrost-app notify_bus` covers the SDK readers.
3. On Midget, the change is visible only through an app that uses the client. Until one does, the check is that the
   `system` service still answers `services` with `notify_bus` in the list, and that the shell runs without a panic
   when the scheme is switched (`bifrost pc midget appearance light`, then `dark`) and the screen is locked and
   unlocked (`bifrost pc midget key enter` after an idle lock).
4. A matrix row is `partial` until an app uses it on Midget. Nothing in this round is marked parity.

## Reference evidence

- Darwin Notify collection page, rendered: `ev_b6bd637635d31f9420f1c56fe3d6ea6f1e41f5fdbb887594f85f06be329c70a1`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/darwinnotify`). It lists the Functions, Data
  Types and Macros groups and the macOS 10.14+ availability.
- Functions group page, tried as `documentation/darwinnotify/darwin-notify-functions`: not found (404). Evidence
  `ev_244165aba07cc428631d5cf97724b1f1b79b8e2e79e54a3d0238991a88c16d32` records the miss. The per-function pages
  were not reachable through `tools/parity/docc.py` (404 for `darwinnotify/notify_post`, `notify_check`,
  `notify_suspend` and the functions group), so the function behaviour is taken from the matrix row and the notes
  `docs/parity/theme-20.md` and `docs/parity/w5-theme-02.md` (evidence `ev_525d60606f901be8b59a405caf5a5af1d6f7b2d77f2724156320402b56271398`).
- The names `system.*` are BifrOSt's own. They mirror the change classes of the Darwin system names (time zone, clock,
  volume, network, host name, cache, log database) only for the three the shell now posts; the spelling is ours.

## Remaining gaps

- No signal delivery (`notify_register_signal`) and no file descriptor delivery (`notify_register_file_descriptor`) to apps.
  The model exists in `core/src/notify_center.rs`, but the service does not expose it.
- No Mach ports (`n/a`, Linux has no equivalent).
- Tokens are not released when their app quits. The app must cancel them; the shell does not prune them yet.
- Observers still run on the app's main thread; no caller-chosen dispatch queue (`notify_register_dispatch`).
- No `suspensionBehavior`: a post to an app that is not running is missed.
- The system names cover appearance, lock and network only. Time zone, clock, volume mount and unmount, low disk space,
  host name, cache invalidation and log database updates are not posted.
- No Midget verification yet, and no host build has been run for this round.

## Matrix rows

Rows are in `docs/parity-matrix/data/darwin-notify.json`. The features this round touches; each stays `partial`.

| slug | feature (as written in the matrix json) | BifrOSt path |
|---|---|---|
| darwin-notify | Posting a named notification (notify_post) | springboard/src/shell/services/notify_bus.rs (`post`), springboard/sdk/src/notify_bus.rs (`post`) |
| darwin-notify | Polling a registration token (notify_check) | springboard/src/shell/services/notify_bus.rs (`register` mode `poll`, `check`), springboard/sdk/src/notify_bus.rs (`check`) |
| darwin-notify | Token lifecycle (notify_cancel, notify_suspend, notify_resume, notify_is_valid_token) | springboard/src/shell/services/notify_bus.rs (`cancel`, `suspend`, `resume`, `valid`), springboard/sdk/src/notify_bus.rs |
| darwin-notify | Per-name state values (notify_set_state, notify_get_state) | springboard/src/shell/services/notify_bus.rs (`set_state`, `state`), springboard/sdk/src/notify_bus.rs |
| darwin-notify | Registering a handler for a name (notify_register_dispatch) | springboard/src/shell/services/notify_bus.rs (`notified` event), springboard/sdk/src/notify_bus.rs (`notified`); the main-thread gap remains |
| darwin-notify | System event names (kNotifyTimeZoneChange, kNotifyClockSet, kNotifyVFSMount, kNotifyVFSUnmount, kNotifyVFSLowDiskSpace, kNotifySCNetworkChange, kNotifySCHostNameChange, kNotifyDSCacheInvalidation*, kNotifyASLDBUpdate) | springboard/src/shell/services/notify_bus.rs (`system.appearance.change`, `system.screen.locked`, `system.screen.unlocked`, `system.network.change`); the other names are not posted |
