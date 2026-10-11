# 006 — Phase 4: document access through grants

Status: planned. Depends on: 003, 005. Parent: `001`.

## Problem (verified)

- `springboard/src/bare/app_policy.rs` gives installed PC apps a bubblewrap sandbox. Storage is all-or-nothing: `storage` binds the entire account home (`app_policy.rs:228-234`, `--bind <home> <home>`).
- Native apps also receive compositor and GPU access.
- `apps/calculator/src/history.rs` and others write directly through the shell's atomic writer; there is no document grant concept.

## Steps

1. Define the grant model: read or write mode, persistent grant ID, owning app manifest ID, revocation, expiry where applicable, cancellation, and stale or replaced-file detection (compare file identity and content hash at reopen).
2. Implement a brokered open and save-as: the shell's picker (or the portal, on PC) returns a file descriptor or a scoped handle. The app never receives a path that it can traverse. Define controlled reopen (re-check identity) and coordinated save (write to a temporary file in the same directory and rename, with the strict writer from 003).
3. Protect against symlink races and confused-deputy requests: open with `O_NOFOLLOW` and resolve through the descriptor, not the path.
4. Provide a BifrOSt-native picker matching each presentation: macOS-style.
5. On PC, integrate with XDG Desktop Portal (FileChooser). Use only the documented portal interface. Do not expose the unrestricted session or system bus. Reference: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html
6. Keep broad storage only where justified, such as Files compatibility. Define the permission migration: existing packages with `storage` keep working until an explicit, documented change; new editor packages request grants instead.
7. Migrate one real editor (Notes desktop) end to end: reopen, save-as, cancel, permission failure, and launch by document. Document the route for the remaining document apps.

## Acceptance

- The migrated editor has no home-wide storage grant.
- It opens and saves only selected documents and cannot read neighboring files (negative test).
- Revocation and restart do not lose data: an unsaved edit is preserved in a recovery file until a save succeeds.
- Permission-failure and symlink-race negative tests.
- Checks: focused tests for the grant broker and the editor; full run belongs to the batch acceptance.
