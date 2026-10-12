# Wire: login items approval (`login-items` service)

Matrix row: Service Management, "Login item for the main app (SMAppService.mainApp)" and "Refresh and status of the
login item" (their `RequiresApproval` status). Wave: wire, branch `wire/svc-login-items`.

## What is now wired

- **Approval step.** A registration (`SMAppService.mainApp.register()`, or the `LoginItem` message) no longer
  becomes `Enabled` at once. It waits as `RequiresApproval` until the user allows it in Settings > General > Login
  Items. Only then does the app open at login.
- **Shell service `login-items`** (`springboard/src/shell/services/login_items_approval.rs`), one line in
  `services/mod.rs`. Methods: `status`, `register`, `unregister`. Each answers `{"status": "<name>"}` with
  `not_registered`, `enabled`, `requires_approval` or `not_found`. The caller is the app the connection belongs to.
- **Status events.** Every status change (request, approval, withdrawal) is sent to the app as service event
  `status` with the same body (`ToApp::ServiceEvent`, via the service tick).
- **SDK client** (`springboard/sdk/src/login_items_approval.rs`): `status`, `register`, `unregister` calls, the
  reply and event decoders `status_from_reply` and `status_from_event`, and the wire-name mapping.
- **Settings page.** Login Items gets a `REQUIRES APPROVAL` group above `OPEN AT LOGIN`. Choosing a row allows it: the
  row moves to `OPEN AT LOGIN`, the pref `login.items` is saved, and a running app receives the legacy
  `LoginItemStatus { Enabled }`.

No wire protocol change: `springboard/proto` is untouched. The new behaviour uses the existing `ServiceCall`,
`ServiceReply`, `ServiceEvent` and `LoginItemStatus` messages.

## How a user reaches it

1. An app calls `login_items_approval::register(ctx)` (or `SMAppService::main_app().register(ctx)`).
2. Settings > General > Login Items lists the app under `REQUIRES APPROVAL`.
3. Choosing the app allows it. It starts at the next login and the app hears `enabled`.

## How to verify on the test PC (Midget)

Host tests first (`cargo test -p springboard login_items` and `-p bifrost-sdk login_items_approval`; the lead runs
the build). Then on Midget, after `bifrost pc midget ota` until `phase=accepted`:

1. Open a sample app that calls `register`. Its reply is `requires_approval`.
2. Open Settings > General > Login Items. The app is in `REQUIRES APPROVAL`.
3. Choose it. It moves to `OPEN AT LOGIN`. Its status event is `enabled`
   (check with `bifrost pc midget state`, or the app's log).
4. Log out and in (or `bifrost pc midget update --restart`). The app starts again.
5. Remove it in Login Items. The app receives `not_registered` (if it is running).

## REA evidence

- `ev_d9a39d832f64e677c23366275a06797159b8d24d27bf37db7c6ea0f3fa0bfb13`: rendered SMAppService page (register,
  unregister, mainApp, openSystemSettingsLoginItems).
- `ev_39aba6b3e568011cfea868835c4412c957263dfe978455fcf6f4f3811739c6eb`: rendered SMAppService.Status page
  (`requiresApproval`: "the user needs to take action in System Preferences").
- DocC: `servicemanagement/smappservice`, `servicemanagement/smappservice/status-swift.enum` (with
  `/requiresapproval`), `servicemanagement/smappservice/register()`.

## Remaining gaps

- Pending requests are kept in memory only. An app registers again at its next start, so a request is lost at a
  shell restart until then.
- There is no deny action. A pending request ends only when the app unregisters, or when it is allowed.
- A status event reaches only a running app. An app that is not running learns its status from its next `status` call.
- `openSystemSettingsLoginItems()` is not reachable from an app.
- `proto/src/extras.rs` still says `RequiresApproval` is "never produced". That comment is now out of date; the
  wire value itself is unchanged. `sdk/src/service_management.rs` has the same out-of-date note.
- Not tested on Midget yet; the host unit tests are the only check so far.
