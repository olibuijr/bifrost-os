# Wire 6: Login Items page (order, background agents, session start)

Area: service management. Builds on `wire-svc-login-items.md` (approval step, `login-items` service, SDK client,
Open at Login and Requires Approval groups), which stays the reference for the approval flow.

## What is now wired

- **Start Order (reorder).** Settings > General > Login Items gets a `START ORDER` group under `OPEN AT LOGIN`
  when two or more apps are listed. Each row shows its place (`1 of 3`). Choosing a row moves that app one place
  earlier. The order is the start order at login: `login_items_launch` starts the apps in the saved list order.
  - Model: `login_items_approval::move_earlier(list, app)` (`springboard/src/shell/services/login_items_approval.rs`).
  - Registry: `Registry::move_earlier` (`springboard/src/shell/extras.rs`), saved in the `login.items` pref.
  - Row builder: `springboard/src/shell/login_items_page.rs` (`order_group`), added by `pages::spec` for
    `PageId::LoginItems`.
  - Settings: `Action::LoginItemMove` moves the row in the page list and records the id; the shell applies it with
    `Shell::login_items_move` after the Settings touch.
- **Session start.** `login_items_launch` (called from `ext_tick` in `apphost.rs`) already starts the login items once
  per session, after the desktop is up and unlocked. It is unchanged; the order change above applies to it. No edit to
  `session.rs` was needed.

## Not wired (and why)

- **Allow in the Background (agents with approval toggles).** Not built. BifrOSt has no launch agents or login-item
  helpers. `bifrostd` is the system supervisor and is not exposed to apps (matrix rows "Launch agents" and "Embedded
  helper executables" stay Partial and Gap). A page section for agents would show nothing real, so none was added.
  This needs a background-item store and an app-facing register call before the page can list anything.

## How a user reaches it

1. Settings > General > Login Items.
2. Under `OPEN AT LOGIN`, the apps that start at login. Choose one to remove it (unchanged).
3. Under `START ORDER`, choose an app to move it one place earlier. Repeat to reach the position you want.

## How to verify on the test PC (Midget)

Host tests first (the lead runs the build): `cargo test -p springboard login_items` (the move model, the registry
order and the page rows) and `cargo test -p springboard login_items_page`.

On Midget, after `bifrost pc midget ota` until `phase=accepted`:

1. Allow three sample apps to open at login (register each one, then allow it under `REQUIRES APPROVAL`).
2. Open Settings > General > Login Items. The `START ORDER` group lists them as `1 of 3`, `2 of 3`, `3 of 3`.
3. Choose the third app. It shows `2 of 3`, and the second app shows `3 of 3`. The first app is unchanged.
4. Log out and in (or `bifrost pc midget update --restart`). The apps start in the new order (check with
   `bifrost pc midget state`, or the apps' logs).
5. Choose the first app: nothing changes (it is already first).

## REA evidence

- Reused from `wire-svc-login-items.md`: `ev_d9a39d832f64e677c23366275a06797159b8d24d27bf37db7c6ea0f3fa0bfb13`
  (rendered SMAppService page), `ev_39aba6b3e568011cfea868835c4412c957263dfe978455fcf6f4f3811739c6eb` (rendered
  SMAppService.Status page, `requiresApproval`).
- No new REA evidence was captured in this wave. The official SMAppService pages do not specify a start order;
  the `START ORDER` group is a BifrOSt decision taken from the task brief, not from Apple's behaviour. Treat it as a
  deliberate deviation to review.

## Remaining gaps

- Order is set by one-step moves only; there is no drag to reorder.
- Pending requests are still in memory only (see `wire-svc-login-items.md`).
- Background agents are not listed (see above).
- `openSystemSettingsLoginItems()` is still not reachable from an app.
- Not tested on Midget yet; host unit tests are the only check so far.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path |
|---|---|---|
| service-management | Settings page for login items | springboard/src/shell/login_items_page.rs; springboard/src/shell/pages.rs (Login Items page); springboard/src/shell/extras.rs (login.items pref) |
| service-management | Login item for the main app (SMAppService.mainApp) | springboard/src/shell/services/login_items_approval.rs; springboard/src/shell/extras.rs (Registry) |
| service-management | Launch agents | not wired (no launch agent store; see Not wired) |
