# Notification Center (slice notification-center)

Matrix rows touched (not closed, lead to decide): system-ui `NotificationCenter` (partial), settings
`Notifications` (partial), settings `FocusSettingsExtension` (partial), frameworks `Focus` (os).

Reference evidence: the shipped macOS 27 extracts under `~/Projects/macos-ref/extract/ref27/` were not consulted in this
round (no REA evidence ids recorded). Behaviour comes from the existing BifrOSt code and its docs.

## Per row

- **Focus (Do Not Disturb, schedules, allowed apps)**: `springboard/src/shell/notify/store.rs`, `notify/mod.rs`.
  Done: a per-app "Allow During Focus" switch on Settings > Notifications > App (`notify_page.rs`, `settings.rs`).
  The allowance is stored in `notifications.ini` under `[focus]`, and `Notify::delivers` lets an allowed app banner and
  wake the screen while Focus is on. A mute still wins. A banner of an allowed app survives Focus switching on.
  Still needs Midget checking: the allowed-app row sits on the app page only (no Focus-page list yet).
- **Per-app page switches**: Lock Screen and Notification Centre now leave an app out of the lock list, the sheet
  and the desktop column (`notify/list.rs` `listed_in`, `notify/mod.rs` `visible_groups`). Badges switches an app's
  badge off (`store_changed`). Changing the page refreshes the lists.
- **Banners vs alerts per app**: already present (`AppNotif::style`, `notify_page.rs`, `presentation_of`).
- **Grouping and clear / clear all**: already present (`Store::groups`, `center.rs` Clear, Clear All).
- **Widgets area**: already present (`center.rs` Edit Widgets gallery on existing widget kits).

## Not done

- **Notification actions and replies**: the usernotif registry (`src/shell/usernotif.rs`, `usernotif_respond`) has the
  action and text-input model, but no Notification Center or banner UI calls it. A card needs its category's action
  buttons and, for a text-input action, a reply field. This needs a card and banner UI pass.
- **Show Previews** (per-app Always / When Unlocked / Never): stored and shown on the page but not honoured, because
  the list's privacy flag is global, not per app.
- **Notification Grouping** (Automatic / By App / Off): stored and shown on the page but not honoured.
- **Sounds**: stored and shown on the page; the notification path plays no sound.
- **Focus page list**: no list of every allowed app on the Focus page; only the per-app switch.

## Shared-file edits

- `springboard/src/shell/mod.rs`: the notify settings snapshot passes `focus_allowed`; the change match gains
  `FocusAllowed`, and `App` now refreshes the store.
- `springboard/src/shell/settings.rs`: `set_notify` takes `focus_allowed`; `notify_edit` gains the `FocusAllowed` arm.
