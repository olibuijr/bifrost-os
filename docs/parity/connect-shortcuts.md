# Connect: Shortcuts app to the notification-schedule and login-items services

Branch `connect/shortcuts`, app `springboard/apps/shortcuts-desktop/` (bifrost-shortcuts). The app uses two shell
services that the wire wave added: `notification-schedule` (time-based reminders) and `login-items` (open at login).
Only the app's own files changed, plus the manifest permission line. The shell, the SDK and other apps are untouched.

## What is connected

### Daily reminders (notification-schedule)

- Each shortcut has a "Daily Reminder" row in its editor. Tapping it steps through 07:00, 09:00, 12:00, 18:00,
  21:00, 22:30 and then Off.
- The list file stores the time as a fifth column (`HH:MM`). Lines without it load as Off, and a bad time drops
  only the time, not the shortcut.
- `src/services.rs` turns the list into one calendar request per timed shortcut: id `shortcut.<name>`, every day at
  that time, repeating. The body says the device cannot run shortcut actions yet.
- The app sends only what changed. It remembers the reminders it last sent. On the first sync (window start) it
  clears the app's pending reminders, so reminders left by an earlier run do not linger, and then sends the list.
  After that a sync with no change sends nothing.
- A sync runs after a touch (cheap compare, sends only on change), after a script change, and at configure time.
  It never runs per frame.

### Open at Login (login-items)

- "Open at Login" is the first row of All Shortcuts. It reads Off, On, or Needs Approval.
- Tapping it calls `login_items_approval::register` (or `unregister` when it is on). The shell answers
  `requires_approval` until the user allows the app in Settings > General > Login Items.
- At window start the app asks for the status once. The reply and any `status` event set the row. A reply with an
  error, or no answer, leaves the row at Off and logs one line to stderr.
- Needs Approval and On both count as "on" for the next tap, so a second tap turns the request off.

### Error handling

- An older shell that answers "no service ..." makes the reply an `Err`. The app logs it and keeps going: the list,
  the window and the other reminders still work. Nothing retries on its own; the next list change sends again.

## How a user sees it

1. Open Shortcuts. All Shortcuts shows "Open at Login" at the top. Tap it: it reads Needs Approval.
2. Open Settings > General > Login Items. Shortcuts is listed under Requires Approval. Allow it. The row in Shortcuts
   changes to On, and Shortcuts starts at the next login.
3. Open a shortcut. Tap "Daily Reminder" until it shows a time (for example 21:00). The list is saved.
4. At that time a notification appears: "The scheduled time has come. This device cannot run shortcut actions yet."
   It repeats every day until the row is set to Off.

## How to verify on the test PC (Midget)

Host tests first, run by the lead: `cargo test -p bifrost-shortcuts` (the services and model tests are in
`src/services.rs` and `src/model.rs`). Then, after `bifrost pc midget ota` until `phase=accepted`:

1. `bifrost pc midget open-app is.olibuijr.shortcuts.desktop`, then tap "Open at Login" (use `tap-window`, not `tap`).
   The row should read Needs Approval.
2. Open Settings > General > Login Items, allow Shortcuts, and check the Shortcuts row reads On.
3. Open a shortcut, set Daily Reminder to a time a few minutes ahead (the steps are fixed, so pick the nearest one
   and set the clock if needed). Wait for the notification.
4. `bifrost pc midget update --restart` (or log out and in). Shortcuts starts again and the row still reads On.
5. Set Open at Login to Off. Login Items no longer lists Shortcuts.

## Gaps

- **Reminders only notify.** Shortcut actions cannot run on this device yet, so a reminder never runs a shortcut.
  The notification says so.
- **Older shells.** On a shell without `notification-schedule`, the add and remove calls fail and are logged. The app
  does not retry until the list changes.
- **Pending state in memory.** The shell keeps pending requests in memory only. The app re-sends on every start (the
  first sync clears and re-adds), so reminders come back after a shell restart when the app next starts.
- **Login items.** No deny action; a request ends only when the app unregisters or the user allows it. A status event
  reaches only a running app. The row shows Off until the first status reply arrives.
- **Tests need the font.** The unit tests in `src/services.rs` read `springboard/assets/SFUI.ttf`, which is gitignored
  and must exist in the checkout that builds (the same as the reminders tests).
- **Compliance snapshots.** The grid and the editor gained rows, so the golden images in
  `tests/snapshots/shortcuts-desktop-*.png` will differ. Re-bless them with `BIFROST_BLESS=1` after reviewing the
  new screens.
- **Not built here.** No build or test was run on this branch; the lead builds once.
- **Parity rows.** The matrix rows for these services are not edited here (`docs/parity-matrix/` is updated centrally).
