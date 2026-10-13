# App Store daily flow (slug: app-store)

Code: `springboard/src/shell/store/` (mod.rs, api.rs), with the keyboard routing in `springboard/src/shell/hwkeys.rs`
and `springboard/src/shell/mod.rs`.

## Reference evidence

The App Store bundle (`System/Applications/App Store.app`) is listed in `~/Projects/macos-ref/extract/ref27-index.tsv`,
but its Info.plist and `Base.lproj` resources are not yet under `~/Projects/macos-ref/extract/ref27/` at the time of
this round. No REA evidence was gathered here. The work is based on the existing store code and the catalogue format
it already reads. Re-check the menus, Search field placement and product-page order against the extracted nibs
once they land.

## What was there before this round

Tabs (Today, Games, Apps, Updates, Search), product pages with a version history, rollback, progress rings with
cancel, the purchase sheet and Restore Purchases, the Updates tab with notes, purchased and system sections, and
category chips on Search.

## Gaps closed in this round

| Feature | Before | Now | BifrOSt path |
|---|---|---|---|
| Search by text | Search tab had category chips only | A query field (`Row::Search`) above the chips. Matches every word across name, summary, category and id, case-insensitive. A "No results" note when nothing matches. Typing comes from the on-screen keyboard (raised by a tap on the field) or a hardware keyboard. | `store/mod.rs` (`matches_query`, `AppStore::key`, `take_keyboard_request`), `shell/mod.rs` (`type_out`, touch dispatch), `shell/hwkeys.rs` (`hw_type_app`) |
| Screenshots on product pages | Catalogue had no screenshots | `screenshots` (URL list, at most 6) parsed in `api::parse_screenshots`. A strip of 260 pt tall pictures sits between the facts row and the description. Pictures load on worker threads, are cached by URL and resized once per height. Placeholder tiles until they arrive. Failed fetches retry after a minute, like icons. | `store/api.rs`, `store/mod.rs` (`fetch_shots`, `draw_shots`, `shots_pt`) |
| Uninstall | Only the store's own uninstall for a failed package; no user action | "Remove" on an installed app's page (right of the button row, red). First tap reads "Confirm", second tap removes via `install::uninstall`. Hidden for system components, background services, preinstalled packages, and while downloading. Refreshes the installed list and the home screen. | `store/mod.rs` (`can_remove`, `remove`, `remove_step`, `remove_btn_rect`) |

Progress for install, update and cancel already existed (`Btn::Progress`, `Job`, `PROGRESS_MS`). Uninstall is a single
local step with no download, so it has no progress ring.

## Midget checks still needed

- Search field: the on-screen keyboard covers the bottom of the pane. Check that the field stays visible and the
  results list scrolls above the keyboard. The keyboard is not raised on desktops (`desk_active`), so check hardware
  typing there.
- Screenshot strip: check the height and gap on a phone-sized and a desktop-sized window, and the no-screenshot case
  (no strip, layout unchanged).
- Remove: check the two-tap flow, that the home screen loses the icon, and that a failed removal shows the error under
  the page's install line.

## Not done

- Screenshots are not tappable (no full-screen viewer), and there is no horizontal scroll past the strip width.
- The App Store app itself (`springboard/apps/`) has no App Store app; the store is shell code, so no app version bump.
- No unit tests were run: this round is code-only (the lead builds).
