# Changelog

## 0.2.11

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.2.10

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.2.9

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.2.8

- Documentation only: the doc comment of the one-time-code clock helper is back above it.

## 0.2.7

- Copied passwords are concealed: the shell keeps them in memory (never in `clipboard.txt`), gives them only to the app in
  front, marks them so other programs and clipboard managers leave them out, and clears them 45 seconds after the copy
  when nothing else was copied. They are no longer put on the plain-text clipboard.

## 0.2.6

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.2.5 — 2026-10-14

- Write CSV exports through a synced, owner-only sibling file and atomic replacement, preserving the previous export if
  the new write is interrupted or fails. Failed password reads now abort the export instead of publishing a partial CSV.
- Report partial keychain imports accurately; imports consist of individual keychain operations and cannot be rolled
  back transactionally.

## 0.2.4 — 2026-10-14

- Add the generated app compliance harness.

## 0.2.3 — 2026-10-14

- Render and hit-test password entries with the shared source-list widget.

## 0.2.2 — 2026-10-14

- Use appearance-aware selected, field and separator fills.

## 0.2.1 — 2026-10-14

- Add the Passwords store icon.

## 0.2.0

- Passwords lists three kinds of item in the shared passwords group: Logins, Notes and Certificates tabs. A note or
  certificate is found by search, shows its first line after Reveal, copies and deletes like a login; Generate and
  the CSV import and export stay with logins.
- The item model and the store name live in one place: the shell's credential store (`springboard::credentials`),
  which the Settings Passwords page and the SDK also use. Notes and certificates are kept under a `note:` or
  `cert:` service prefix, so no site name can be taken for one.
- Still missing: creating a note or certificate (needs a text editor), showing a whole multi-line note, and the
  passcode check in this app before reveal (the Settings page asks for it; this app does not yet).

## 0.1.0

- New app Passwords (`is.olibuijr.passwords.desktop`): lists the logins in the shared passwords group (the same
  logins Settings > Passwords shows), searchable by site or account; reveal, copy, generate a strong password for a
  login, delete; import a CSV with site, account and password columns; export the logins to a CSV file.
- Still missing: the passcode check before reveal and export (the shell has no authentication call for apps yet),
  editing a login's site, account or password by hand (needs a text field), and a Passwords icon from the Mail set.
