# Wiring wave 6: Passwords (keychain groups, Settings page, SDK fill and save)

Branch `wire6/passwords`. Builds on the access-group service in `wire-svc-keychain.md` (wave 5 wiring). No new
parity model: the wiring uses the keychain store and the `keychain-groups` service that already exist, and reads
the matrix rows listed below. The wire protocol (`springboard/proto`) is unchanged.

## What is now wired

| Piece | Where |
|---|---|
| Store: each item records the time it was last written; `list()` names each item (service, account, time) with no secret. A file written before the time column still reads (time 0). | `springboard/src/keychain.rs` (`Listing`, `Store::list`, `now`, the 4th tab field in `render`/`parse`) |
| Passwords group: `PASSWORDS_GROUP = "is.bifrost.passwords"`, `logins()`, `secret()`, `remove()` for the shell | `springboard/src/shell/services/keychain_groups.rs` |
| Pure page model: the list of logins by site, account and last-modified time; the revealed row with Copy, Hide and Delete | `springboard/src/shell/passwords_page.rs` (`spec`, `PageState`, `Act`, `Shown`) |
| Settings > Passwords page: a row under Privacy and Passcode in the root list; reveal asks for the passcode when one is set (`Purpose::Reveal` only confirms the code, it changes nothing) | `springboard/src/shell/pages.rs` (`PageId::Passwords`, `Action::Passwords`), `springboard/src/shell/settings.rs` (`spec_for`, `activate`, `passwords_act`, `reveal_password`, overlay Done/Cancelled), `springboard/src/shell/passcode.rs` (`Purpose::Reveal`) |
| SDK client for apps: `save`, `update`, `fill`, `remove`, `decode_password` over the passwords group | `springboard/sdk/src/passwords.rs`, one line in `springboard/sdk/src/lib.rs` |
| Module registration | `springboard/src/shell/mod.rs` (one line) |

## How a user reaches it

1. Settings > Passwords (root list, below Passcode). The page lists each saved login: the site as the label, and
   the account and the time it was last written as the value. An empty store says "No Saved Passwords".
2. Tap a login. If a passcode is set, the keypad asks for it (title "Enter your passcode"); a wrong code is refused
   and counts toward the lockout as usual. With the right code, or with no passcode, the page shows the password
   with three more rows:
   - Copy Password: puts the password on the clipboard (`clipboard::set_text`).
   - Hide Password: hides it again.
   - Delete Password: removes the login from the passwords group (red row, footer says every app that saves to the
     group loses it).

An app that fills forms (the Browser is the intended caller) reaches the same items through the SDK:

1. Its manifest sets `permissions = ["keychain"]` and `keychain_groups = ["is.bifrost.passwords"]`.
2. `passwords::save(ctx, site, account, password)` when a login is submitted. On a `DuplicateItem` answer, call
   `passwords::update` instead.
3. `passwords::fill(ctx, site, account)`, then `passwords::decode_password(result)` in `App::service_reply` gives the
   password (`Result<String, Error>`).

## How to verify on the test PC (Midget)

Midget is the only test machine (no VM builds). Nothing here has been built or run yet; the lead's build is the first
check.

1. Host tests (lead): `cargo test -p springboard keychain` covers the store listing and the legacy time-less file;
   `passwords_page` tests cover the empty page, the modified-time text (`Oct 4, 00:23` for 1791073395 at offset 0,
   checked with `date -u`), and the revealed rows; `keychain_groups` covers the passwords group round trip.
2. Seed a login: build a test app with the keychain permission and `keychain_groups = ["is.bifrost.passwords"]`,
   call `passwords::save(ctx, "example.com", "alice", "hunter2")`, publish and install it on Midget
   (`ssh midget`, then `/usr/bin/springboard --install-store-app <id>`).
3. Open Settings > Passwords on Midget: one row, "example.com", value "alice  ·  <date>".
4. With no passcode, tap the row: "Password" shows `hunter2`; Copy Password then pastes `hunter2`; Hide returns the
   list; Delete Password empties the list.
5. Turn a passcode on (Settings > Passcode), tap a row: the keypad asks first; the wrong code is refused; the right
   code shows the password.
6. Screenshots of the page in Light and Dark appearance (`bifrost pc midget screen`), checked against the bifrost-hig
   rules (content area `CONTENT_BG`, footers, list rows).

## Reference behaviour (official documentation)

- Security, keychain items and access groups: `docs/parity/wire-svc-keychain.md` cites
  `kSecAttrAccessGroup` and REA evidence `ev_d841fc09eb19d111145d99ce7601cd842cbb42fcb8f1bb48d35e3ee3efcc7ebd`, and the
  Security collection evidence `ev_3bcba33c922f0c6a1f91f930d57aed415e4f75de88d819ed415661efaa4fcee6`. Those are the
  evidence ids this wave relies on. No new REA capture was taken in this round.
- Password AutoFill and Shared Web Credentials (the reference for the Passwords page) is a cloud-sync feature of the
  reference platform.
  The matrix already records it as `n/a` (no counterpart), so the page follows the keychain's own item model, not
  that feature's sync.

## Remaining gaps

- The Browser does not call the SDK yet. The client exists, but filling and saving logins in web forms needs the
  Browser's manifest change and its form hooks, which live in the Browser app's repository.
- Edit: no edit sheet. Changing a saved password means saving it again from the app (update) or deleting and
  re-saving; the Settings page has no edit row.
- Delete has no confirmation alert. A tap on Delete Password removes the login at once.
- The revealed password stays in memory while the Settings window is open (it is cleared only by Hide, by Delete,
  or by closing Settings). It is not shown on any other page.
- AutoFill suggestions are not wired: `springboard/ui/src/credential_identity.rs` (`IdentityStore`,
  `identities_for`) is still a pure model, not fed from the passwords group.
- The keychain save panel (`springboard/ui/src/keychain_save.rs`) is not wired; the Settings Passwords page is the
  only UI for saved logins.
- Access groups are still the wave 5 model: a group is named on every call and there is no search across groups.
- Passwords are stored in the same encrypted per-group file format as before; the file now has a fourth column (the
  time). An older shell reading a newer file would reject it, so a downgrade of the shell needs the file removed.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status here |
|---|---|---|---|
| security | Keychain services (generic passwords) | `springboard/src/keychain.rs` (`Store::list`, timed items); `springboard/sdk/src/passwords.rs`; Settings > Passwords (`springboard/src/shell/passwords_page.rs`) | wired, not yet run on Midget |
| authentication-services | Password credentials (sign-in form and keychain sharing) | `springboard/sdk/src/passwords.rs` (save, fill, update, remove); `springboard/src/shell/services/keychain_groups.rs` (`PASSWORDS_GROUP`) | wired for the shell page and SDK; the Browser does not call it yet |
| authentication-services | AutoFill credential provider | `springboard/ui/src/credential_identity.rs` | not wired (gap above) |
| security-interface | Keychain save panel | `springboard/ui/src/keychain_save.rs` | not wired (gap above) |
