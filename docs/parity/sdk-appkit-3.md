# AppKit (User Interface) parity: SDK round 3 (sdk-appkit-3)

Slice: App Frameworks, appkit-* rows. Round 75 wave, branch from `wave-base` (7059083c).

## Row advanced

| Matrix row | Gap text | Status after this round |
|---|---|---|
| AppKit (User Interface) | App dock tile badges/menus and custom cursors | Dock menu items done; custom cursor images still missing |

## Dock menu (menus, Dock)

- API: `springboard/sdk/src/dock_tile.rs` `MenuItem { id, title, enabled }`, `menu_params`, `set_menu(ctx, &[MenuItem])`.
- Shell service: `springboard/src/shell/services/dock_tile.rs` method `menu`, `MENU_MAX` 8, `MENU_TITLE_MAX` 64,
  stored on the app host as `AppHost::dock_menu` (`springboard/src/shell/apphost.rs`).
- Shell drawing: `springboard/src/shell/desktop_bar.rs` `Menu::Dock` appends the app's items after its Quit row, then a
  separator. `Action::DockMenu(slot, id)` sends `ToApp::ContextAction(id)` to the app, which reads it as
  `Input::ContextAction`. `springboard/src/shell/desktop_dock.rs` `dock_popup_rect` grows the popup by 24 pt per item.
- Done: the service, its validation, the SDK params and call, the menu rows and dispatch, popup height.
- Midget check needed: the popup height for eight items (the row height 24 pt is an estimate), the greyed row for a
  disabled item, and a click reaching the app (`ContextAction` handler).

## Still missing in this row

- Custom cursor images: the cursor service (`cursor_service.rs`) only takes the fixed `Kind` shapes. An app cannot register
  its own image and hot spot.

## Evidence

- Matrix text read with `python3 tools/bifrost.py pc parity show "AppKit (User Interface)"` (2026-10-13).
- Existing dock service and tile state: `springboard/src/shell/services/dock_tile.rs`, `springboard/sdk/src/dock_tile.rs`.

## Verification

Not built or run by instruction (code and commit only). The lead builds and tests the touched crates. Unit tests added:
shell `dock_tile.rs` (3 tests), SDK `dock_tile.rs` (1 test).
