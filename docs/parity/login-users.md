# Login window and users (slice login-users, round 75)

Status: partial. This round adds the password hint end to end. The other items in the slice are still models or
missing, listed below. Nothing was built or run (brief rule: the lead builds). No reference evidence was used:
the macOS 27 extract under `~/Projects/macos-ref/extract/ref27` was checked for loginwindow strings only by name in
the index and was not measured with REA this round.

## Password hint

- Behaviour: an account may have a hint. The login window shows `Hint: <text>` under the message line once the
  wrong-password count reaches `HINT_AFTER` (3). A correct password clears the count. No hint, no line.
- privd: `springboard/src/privd/auth.rs` `parse_hint` reads the `password_hint=` line of the account file
  (`/var/lib/springboard/account`). Control characters are dropped, blank text means no hint, and text is capped at
  `HINT_MAX` (200) characters. `AccountInfo.password_hint` carries it; `springboard/src/privd/gate.rs` `info_json`
  sends it as `password_hint` in the `login.account` reply.
- Shell: `springboard/src/shell/login.rs` `Login.hint`, `Login.wrong_count`, `Login::hint_line`, drawn under the
  message line. `Privd::account` parses `password_hint` from the reply.
- Tests: `privd/auth_tests.rs` (`the_password_hint_is_read_from_the_account_file_and_cleaned`,
  `hint_parsing_drops_control_characters_blank_text_and_long_text`); `shell/login.rs`
  (`the_password_hint_shows_after_three_wrong_passwords_and_only_when_set`).
- Still open: no setter. Nothing in Settings > Users or privd writes `password_hint=` yet, so a hint can only be
  set by editing the account file as root. Needs a `account.set_password_hint` method and a field in Users & Groups.
- Midget check needed: the hint line position under the field and the wrong-count reset after a success.

## Lockout timing (already present, unchanged)

- `privd/auth.rs` `lockout_secs`: 0 for 0 to 4 failures, then 60 s, 300 s, 900 s, 3600 s. The count and the lock
  time persist in `/var/lib/springboard/privd-fails`.
- Login window: `Login::message` shows "Try again in N minute(s)/second(s)". Tests exist in `shell/login.rs`.

## Name and password mode (already present, unchanged)

- `Login.name_mode` (set from `options().login_name_password`) puts a name field above the password field and checks
  the name before any password attempt.

## Still open in this slice

- User list on the login window (show accounts with avatars, pick one). Only the single installed account is served
  by privd (`Auth::account` never takes a client name), so this needs a multi-account privd method first.
- Fast user switching menu (no code found under `springboard/src`).
- Guest user (no login or privd path; `guest` appears only in unrelated virtual-machine code).
- Login items started per user: the `login-items` service and `login_items_approval` SDK client exist for apps
  (`shortcuts-desktop`), but nothing in the shell starts them at login for each user. Not touched.
