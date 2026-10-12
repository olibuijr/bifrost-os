# Wave 5, theme 17: console user and session information

Theme: "Console user and session information" (size 5). The theme's five rows span System Configuration, Security,
ScreenCaptureKit, FSKit and Security Foundation. This round closes one row honestly: the console user.

## What was built

`springboard/core/src/console_user.rs` (pure model, std only, host unit tests).

- `parse_sessions(text)` reads session records from `Key=Value` text separated by blank lines (`Id`, `UID`, `Name`,
  `Seat`, `Class`, `State`, `Remote`).
- `console_user(&[Session])` returns the account at the console. Only a session on `seat0`, of class `user` and not
  remote can qualify. `active` beats `online`, `closing` never qualifies, and ties go to the lowest session id. With
  no qualifying session the answer is `None`, which is the sign-in screen.
- The module reads no session manager, login record or dynamic store and starts no watcher. A caller supplies the
  records.

Gaps kept open: a session record carries no group id, so the answer has the name and uid only. No change
notification is delivered; the caller polls. Nothing is wired into the SDK or the shell yet.

## Official API mirrored

- `SCDynamicStoreCopyConsoleUser` (System Configuration), which returns the console user's name, uid and gid.
  The reference page for the function itself returned 404 on the official site
  (`documentation/systemconfiguration/scdynamicstorecopyconsoleuser`). The function is documented on the
  `SCDynamicStore` collection page, which rendered, so the mirror is taken from that page and its listing.

## REA evidence

- `ev_22d3d9a6ad757a7be97611439dc07d9b98f468d2e44ade0957796e650f62f9da`: SCDynamicStore collection page, rendered.
- `ev_3d073b469ffcef268f0deea73466ff4c5e4f90e5d1b5172b6306741e7e7f0b1a`: the function's own URL, rendered as a
  "page not found" page (recorded so the gap in the official source is on file).

## Matrix rows closed

| framework slug | feature (as in docs/parity-matrix/data/system-configuration.json) | status | BifrOSt path |
|---|---|---|---|
| system-configuration | Console user and session information | partial | springboard/core/src/console_user.rs |

Rows in this theme that are not closed here: Security "Code signing services" (no signature inspection),
ScreenCaptureKit "Permission for screen recording and persistent capture", FSKit "Unary file system extension",
and Security Foundation "Credential invalidation". None of them is the console user, so each needs its own work.

## Tests

`cargo` was not run (the wave rules forbid builds). The tests in `console_user.rs` are worked out by hand:
parse of two records with an empty seat, a missing key message, a non-numeric uid, active over online with the
lowest id winning ties, remote and closing exclusion, the sign-in screen, and a session on another seat.
