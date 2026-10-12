# Wire 6: Recent Items in the leading system menu (wire6/recent-items)

Scope: the Core Services row "Shared file lists (recent items, favourites)", driven by the `bifrost_ui::shared_list`
model (Wave 4), plus the start of the Core Services "Role handlers" and "Quarantine properties" rows. This round wires
the recent-items half of the shared file list into the desktop shell. Nothing was built or run (the lead builds once).
Every expected value in the tests was worked out by hand from the code.

## What is now wired

1. **Recent Items in the leading system menu** (`springboard/src/shell/desktop_bar.rs`, `Menu::System`). After App Store
   the menu shows a "Recent Items" header, then the newest apps (up to 10), then "Clear Menu", then a separator and the
   power rows as before. The list is the shared `SharedList` model from `springboard/ui/src/shared_list.rs`, capacity 10.
2. **What gets recorded**: every app launch through `Shell::desk_open` (`springboard/src/shell/desk.rs`, one line).
   That covers the dock, the home screen, the menus and the search panel. Reopening an app moves it to the
   front; the list never holds an app twice.
3. **Choosing an entry** opens that app (`Action::Recent(n)` runs `desk_open`), so the entry moves to the front again.
4. **Clear Menu** (`Action::ClearRecent`) empties the list. When the list is empty the row is greyed out (a label).
5. **Helpers** live in `springboard/src/shell/recent_items.rs`: `note`, `app_at`, `key` (the `AppId` debug form, unique
   per catalogue entry, so an external app's slot maps back to the same app) and the `Shell` methods
   `recent_note`, `recent_titles`, `recent_app_at`, `recent_clear`. The field `recent_apps` on `Shell`
   (`springboard/src/shell/mod.rs`) holds the list.
6. **Tests**: three pure tests in `recent_items.rs` (order, reuse, capacity, clear, key form) and one shell test
   (`the_system_menu_lists_launches_newest_first_and_clear_menu_empties_it`, gated on `desktop-wayland` like the other
   menu tests) that opens Terminal then Settings, expects the rows `Recent Items, Settings, Term, Clear Menu`, chooses
   the second entry (rows become `Recent Items, Term, Settings, Clear Menu`), then Clear Menu leaves
   `Recent Items, Clear Menu`.

## How a user reaches it

- Click the leading system menu (the BifrOSt mark, left of the bar), or press the menu keys and go to the first menu.
- Under "Recent Items" are the apps opened most recently. Choose one to open it again.
- "Clear Menu" empties the list. It stays greyed out when nothing is listed.

## Decision: a flat group, not a flyout

Apple's Recent Items is a submenu. The desktop bar's `Row` model has no submenu (the app-published menus only indent
children). A flyout would need a second popup, a hover path and keyboard steps, which is a larger change to the shared
bar. This round uses a labelled group inside the system menu: the same rows, in the same order, with the header as a
greyed label. A flyout is listed under the gaps below.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Click the leading system menu. Expect "Recent Items" under "App Store...", followed by "Clear Menu" greyed out and no
   entries (a fresh session has no launches).
3. Open Settings from the Dock, then Term from the Dock. Reopen the system menu. Expect `Term` first, then `Settings`,
   then "Clear Menu" enabled.
4. Choose `Settings` in the list. Settings comes to the front, and the list reads `Settings`, `Term`.
5. Choose "Clear Menu". Expect the entries to disappear and "Clear Menu" to grey out.
6. Caveat: the list lives for the shell session. A shell restart clears it (see gaps).
7. Use `tap-window` on the bar for the menu, not raw `tap`: the menu sits in the shell's own geometry.

## REA evidence

- `ev_7e0325a5e3f1db5c5ec71f91e1716ae775ebab1c0e603db9d4aef7cdf46d6c4d`: rendered official page
  `https://developer.apple.com/documentation/appkit/nsdocumentcontroller`. The Managing the Open Recent Menu topic lists
  `maximumRecentDocumentCount`, `clearRecentDocuments(_:)`, `noteNewRecentDocumentURL(_:)`,
  `noteNewRecentDocument(_:)` and `recentDocumentURLs`. The overview says the document controller manages the Open
  Recent menu for apps that do not subclass it. `docc.py appkit/nsdocumentcontroller` lists the same symbols.
- The earlier wire5 evidence (`ev_cf47282d...`, `ev_ada821e1...`) still covers Launch Services and workspace opening.

## Matrix rows

The rows are not edited in this round (`docs/parity-matrix/` is shared). The statuses below are the proposal for the
database update.

| slug | feature (as in the matrix json) | BifrOSt path | proposed status |
|---|---|---|---|
| core-services | Shared file lists (recent items, favourites) | springboard/ui/src/shared_list.rs (model); springboard/src/shell/recent_items.rs (wired); springboard/src/shell/desktop_bar.rs (Menu::System rows, Recent and ClearRecent actions) | partial (recent apps are wired for the session; favourites, documents and persistence are not) |
| core-services | Role handlers (viewer, editor, shell) | springboard/ui/src/launch.rs (Role) | partial (not changed this round; see gaps) |
| core-services | Quarantine properties | springboard/core/src/quarantine.rs | partial (not changed this round; see gaps) |

## Remaining gaps

- **Recent documents are not recorded.** Only app launches are. Documents opened through `defaultapps` or Files need a
  hook that calls `recent_note`-style code with a document key and title (the list keys are plain strings, so a path
  works).
- **Recent Items is a flat group, not a submenu.** A flyout is a bar change (second popup, hover, keyboard path).
- **The list is not persisted.** It is cleared when the shell restarts. Persisting it needs a prefs key and a restore
  step in `Shell::new`.
- **Favourites are not wired.** The model supports them (`set_favourite`, favourites listed first and kept past the cap),
  but no shell or app sets them. A sidebar Favourites section needs an app change.
- **Quarantine is not wired.** `springboard/core/src/quarantine.rs` has a reader and writer for the record, but nothing
  writes one: downloads are not tagged (the browser does not write a record). A first-open confirmation needs the
  download path to write the record, and the shell's open path to read it and show a sheet. Both are outside this round.
- **Role handlers are not wired.** `springboard/ui/src/launch.rs` (`Role`) keeps one default per type with no roles.
  The Settings Default Apps page and `default_app_for_file` do not take a role, so a viewer and an editor cannot be set
  separately.
- Not verified: no build or test run (the lead builds once). The `desk_open` hook runs before the existing body, so
  an app that fails to open is still recorded; that is intended for the menu but worth a look if it matters.
