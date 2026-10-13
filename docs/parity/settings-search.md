# Settings: smooth pages, search and deep links (slice settings-search)

Code: `springboard/src/shell/settings.rs`, `settings_search.rs`, `settings_load.rs` (new), `pages.rs`.
Not built or run in this round (the lead builds, tests and checks on Midget).

## What changed

- **Search indexes every pane and setting.** `settings_search::index` walks the root page's live specs
  (`Settings::spec_for`) through every pane a row links to, and lists each pane by its own name and each
  setting row (one that has an action) under the page it sits on. Matching folds case and accents, ignores
  punctuation ("wifi" finds "Wi-Fi") and needs every typed word to match the label, the page title or the
  keywords. `pages::KEYWORDS` holds the keywords per label. Results replace the root rows (phone and sidebar)
  while there is a query; a tap opens the match's page.
  - Not walked (listed by their own row only): Wallpaper, Passwords, Internet Accounts, Diagnostics, Storage,
    Passcode, Developer, Screen Time, Users & Groups, Game Controllers, Font pick, Graphics. Detail pages
    (Wi-Fi network, Bluetooth device, notification app, one user, one group, one controller) are never listed.
- **Deep links by name.** `Settings::open_page` resolves a name in this order: the hard-coded names it always
  had, `settings_search::alias` ("Wi-Fi Settings", "Network Settings", "Bluetooth Settings", "Sound",
  "Displays", "Desktop & Dock", "Privacy & Security", "Users & Groups", "Print & Scan", "Keyboard Shortcuts",
  "Lock Screen", "Screen Time", "Trackpad", "Mouse"), then a pane or setting with that label
  (`settings_search::find_page`). The index is built for the lookup and not kept.
- **No disk or service reads on open.** `settings_load::Loaded<T>` runs a read on a worker thread and keeps
  the last value. `Settings` holds these in `Loads`: font roles and library, graphics adapters, crash logs,
  latest backup, saved logins, internet accounts, users and groups, sharing, printers and scanners, and Screen
  Time config and seven days of usage. A page opens at once from the last value; `tick` takes the fresh one
  when it lands and rebuilds the page on top. `next_wake` wakes every 100 ms while a read is running.
  - `pages::spec` no longer reads anything: `Model` carries `sharing`, `printers` and `users` (statics
    `NO_SHARING`, `NO_PRINTERS`, `NO_USERS` for models without them). The Fonts page no longer probes a font
    file per row: `FontsView.names` holds each saved override's family, read with the library.
  - Actions that change one of these (Users, Sharing, Print & Scan, internet accounts, a deleted login, crash
    log deletion, a backup) start a fresh read after they apply.

## Still synchronous on open (small, or must stay on the UI thread)

- Passcode state (`passcode.ini`, `screentime_passcode.ini`) and `DevState::read` on Storage, Passcode,
  Developer, Passwords and Screen Time. Small files; they are read on each rebuild of those pages.
- Privacy > App Permissions: `privacy_permissions::rows()` reads thread-local stores, so it cannot run on a worker
  thread. It still scans installed apps when the page opens.
- Taps that change data (Users, Sharing, Print & Scan) still read the current view to apply the edit.

## Removed (adopt and delete)

- `settings_search::filter` (title filter over the root rows), replaced by `results`.
- `Settings.fonts` and `Settings.gpus`, replaced by `Settings.loads`; `load_fonts` now starts a read.
- The synchronous reads inside `pages::spec` for Sharing, Print & Scan and Users.

## Needs Midget checking

- Open each pane from the sidebar and from the phone list: no visible stall; the list fills within a frame or two
  for Fonts, Graphics, Diagnostics, Passwords, Internet Accounts, Users, Sharing, Print & Scan and Screen Time.
- Type "wifi", "display", "sound", "battery", "print": the matches appear and a tap opens the pane.
- Shell menu "Network Settings…" still opens Network. To make a Wi-Fi menu row deep-link, point it at
  `Action::Settings("Wi-Fi Settings")` in `desktop_bar.rs` (a shared file this slice does not edit).
- Fonts: picking a font shows its family name at once in the row.

## Tests added (run by the lead)

- `settings_load`: a read lands once; a newer read replaces a running one.
- `settings_search`: index contents and private pages, matching and ranking, results rows, aliases and deep links.
- `settings`: `a_deep_link_from_another_app_opens_its_pane`; `settle` in tests waits for background reads.
