# Wire: `privacy` system service (App Permissions)

Branch `wire/svc-privacy`, on top of `parity/integration`. Not built: the lead builds once.

REA evidence (official article, rendered): `ev_9fa64cd138bf4fc1e7f415c6fc08bf40b0f9e13a4b96b6978b8ddd6904409c25`
(AVFoundation, "Requesting authorization to capture and save media"). The article says the system asks once per app
and per device type, remembers the answer, and that the person changes it later in the privacy settings. That is the
model `bifrost_ui::capture_access` follows. The `docc.py` outline (`avfoundation/requesting-authorization-to-capture-and-save-media`)
gave the topic list only.

## What is now wired

| Matrix slug | Feature (docs/parity-matrix/data) | BifrOSt path |
|---|---|---|
| `avfoundation` | Capture authorization (camera, microphone) | `springboard/ui/src/capture_access.rs` via the new service |
| `screencapturekit` | Screen recording permission | same model, `Media::Screen` |

- `springboard/src/shell/services/privacy_permissions.rs` (new). Implements `SystemService` under the name `privacy`.
  Per-app `CaptureAccess` state (camera, microphone, screen) in a thread-local store, because every service call and
  every Settings change runs on the shell thread. Methods: `status {media}`, `can_capture {media}`, `all {}`. A pure
  `answer()` does the work so tests need no `Shell`. Unknown media or method is an error.
- `springboard/src/shell/services/mod.rs`: one `pub mod` line and one `Box::new` line.
- `springboard/sdk/src/privacy_permissions.rs` (new). Typed client over `service::call`: `status`, `can_capture`, `all`,
  and pure decoders `decode_status`, `decode_allowed`, `decode_all`. It uses `controls::capture_access::{Media, Status}`.
- `springboard/sdk/src/lib.rs`: one `pub mod privacy_permissions;` line.
- `springboard/src/shell/pages.rs`: `PageId::PrivacyPermissions`, `Toggle::Permission(Media, index)`, the
  "App Permissions" row in Privacy, and `permissions_page(rows)`, which builds one group per media type with a switch per
  installed app. Test: `app_permissions_lists_each_media_type_with_one_switch_per_app`.
- `springboard/src/shell/settings.rs`: spec dispatch, the toggle arm (flips the answer through `set_by_user`, shows the
  answer that stands), and a search entry.

Unit tests: the store and service (`privacy_permissions.rs`), the page spec (`pages.rs`), and the SDK decoders
(`sdk/src/privacy_permissions.rs`). None were run (no builds in this round).

## How a person reaches it

Settings > Privacy > App Permissions. Three groups: Camera, Microphone, Screen Recording. Each lists the installed apps
with a switch. A switch is on only when the person allowed that app. Apps that have not been answered show off.

## How an app uses it

```rust
let id = bifrost_app::privacy_permissions::can_capture(ctx, Media::Video);
// App::service_reply: privacy_permissions::decode_allowed(&reply) == Some(true) means it may open the camera
```

## How to verify on the test PC (Midget)

1. `cargo build --offline --profile fast --bin shell --features desktop-wayland` in `springboard/`, then
   `tools/midget-push-shell.sh --no-build shot.png`.
2. `bifrost pc midget state` shows Settings. Open Settings > Privacy > App Permissions. The three group headers must
   show, and with no app installed each shows "No apps installed".
3. Install a test app that calls `privacy` `status` for camera and logs the reply. Its row appears under Camera and
   Microphone with the switch off. Turn it on with `bifrost pc midget tap-window Settings X Y`. The app's next
   `can_capture` reply must be `allowed: true`.
4. Light and Dark: `bifrost pc midget appearance light` and `dark`, then a screenshot. Check the rows against the
   bifrost-hig checklist.

## Gaps

- **Location** and **Files** are not wired. There is no location authorization model (see `docs/parity/core-location.md`,
  "Authorization and usage descriptions (gap)"), and file access is the `documents` grant in `app_policy.rs`, which has
  no per-user state. Adding them needs a model first.
- **No prompt.** The shell never asks the person from an app's call, so `request` is not exposed. An app with no answer
  stays `not_determined` until the person switches it on in Settings.
- **No persistence.** The answers reset when the shell restarts. They need a file in the shell data dir before release.
- **No change events.** Apps are not told when the person changes an answer; the service only answers calls.
- **Restricted** is never set: nothing in the shell reads a system restriction, so no app is restricted yet.
- The wire names `camera`, `microphone`, `screen` and the status names are written in the service and in the SDK
  client separately. They must match; the decoder tests cover the SDK side.
