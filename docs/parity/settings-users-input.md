# Settings: Users & Groups, Trackpad, Mouse (slug settings-users-input)

Evidence: the shipped macOS 27 reference index lists `MouseExtension.appex` and `TrackpadExtension.appex` under
`System/Library/ExtensionKit/Extensions/` (`~/Projects/macos-ref/extract/ref27-index.tsv`). The REA measurement of their
Info.plist and string tables was not done in this round, so the pane labels and option lists come from the existing
Settings code, not from the reference bundles.

## Users & Groups (PageId::Users, UserDetail, Groups, GroupDetail)

- API: privd accounts methods `user.add`, `user.delete`, `user.set_admin`, `user.set_password`, `account.set_password`,
  `group.add`, `group.delete`, `group.member`, through `privd_client::Client`.
- BifrOSt paths: `springboard/src/shell/users_page.rs` (pages, edits, flows), `springboard/src/shell/forms.rs` (one-field
  prompts), `springboard/src/privd/accounts.rs` (the root-side validation and argv), `springboard/src/privd_client.rs`.
- Done in this round (`settings.rs`):
  - Pages are built from `users_page::spec` with the live `/etc/passwd` and `/etc/group` view and the last error note.
  - `Action::Users` opens the add, password and current-password prompts (`Sheet::form`) or applies the edit now
    (promote or demote, group membership), with errors shown under the list.
  - Form submits go through `users_page::finish`, which runs the next prompt step or the privd call.
  - `Command::Account` (delete user or group) has a confirmation alert and runs `users_page::run`; a deleted account's
    detail page is popped.
  - Detail pages record their name in `detail` on push (`UserDetail`, `GroupDetail`).
- Still needs Midget checking: a real add (standard and administrator), a password change for another account, the
  installed account's own password (old password first), deletion, and the error note for a refused name.

## Trackpad and Mouse

- API: `bare::pointer_prefs::Tuning` (tracking speed, natural scrolling, tap to click, secondary click, swap buttons).
- BifrOSt paths: `springboard/src/shell/pages.rs` (panes), `settings.rs` (sliders, toggles, picks, `apply_pointer`),
  `options.rs` (persisted choices, `pointer_tuning`), `springboard/src/bare/pointer_prefs.rs`, `springboard/src/bare/input.rs`
  (decoders read `pointer_prefs::get()` for each event).
- Status: already wired end to end before this round. Settings publishes the tuning at start-up and on each change;
  input applies speed, scroll direction, tap to click, the two secondary-click modes and swapped buttons. No change made.
- Still needs Midget checking: each slider and switch on a real trackpad and mouse.

## Matrix entries

Not edited (lead closes rows). Candidate rows for the lead: Users & Groups (account, group and password management),
Trackpad (tracking speed, natural scrolling, tap to click, secondary click), and Mouse (tracking speed, scrolling speed,
natural scrolling, swap buttons).
