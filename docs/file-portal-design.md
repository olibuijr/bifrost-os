# File grant and picker portal design

Status (2026-10-13): phases 1 to 3 are built and wired into the shell (details below, written at round 21 and 0.4.81).
Since then: 20 app manifests declare `documents` and 48 still declare `storage` (Files and Notes hold both, so the home-wide
grant is not removed anywhere yet). Drag sources mint read grants (`Broker::drag_grant`, released when the drag ends); a drop
that no window takes goes to the Dock icon of an app (files open in it) or to the desktop (`shell/dnd.rs`). The only portal
interface served is `org.freedesktop.portal.Settings` (`bifrost-portal`, appearance for Linux toolkits); there is no
FileChooser proxy.

Historical status (2026-10-09, round 21): phases 1 and 2 were built and tested on the host, not wired into the shell yet.
- Phase 1, broker: `springboard/src/bare/docbroker.rs` (`Opener`: open as the account with `openat2`
  `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, regular files only; `begin_save`/`commit`/`abort`;
  `Grants`: per-app records, 0600, atomic writes, `Stale` on a replaced file). 3 tests.
- Phase 2, protocol: `ToShell::DocOpen/DocSave/DocCommit/DocAbort/DocReopen/DocRelease`, `ToApp::DocGranted/DocError`
  (tags 16-17 and 0x110-0x115, bounded filters and titles). Decision: unknown and foreign grant ids both answer
  `Denied`, so an app cannot probe for grants. The fd rides on SCM_RIGHTS like the frame buffers.
- Phase 3 (0.4.81): the `documents` permission (`app_policy.rs`, binds nothing; `tools/aap.py` accepts it), the panel
  (`springboard/src/shell/docpanel.rs`: a desktop popup `Menu::Document`, modal over the desktop, kept shown until
  answered), routing in `apphost.rs` `ext_tick` (replies carry the fd via `AppHost::send_file`), listing as the account
  (`Opener::list`), and the SDK call (`Ctx::open_document`/`save_document`/`commit_document`/`abort_document`/
  `reopen_document`/`release_document`, answers in `App::document` as `DocumentEvent`). Decisions: one panel at a
  time (a second request is answered `Cancelled`); a commit is answered with the saved file under the same grant id; a
  quit app's unfinished saves are removed.
- Phase 4 started: Notes 1.6.0 exports (Share -> save panel) and imports (Cmd+O). Notes keeps `storage` until its
  attachments move to the panels.
- Not yet (still open 2026-10-13 unless noted): Settings page to revoke grants, removal revoking an app's grants (`Grants` has
  the call to drop all of an app's grants; check the removal path before relying on it), persistent "last folder", `NSOpenPanel`
  multiple selection and folder choice, native (non-SDK) apps (section 4.2), and the XDG FileChooser proxy.

Original status: design only (task S4). Implements the goal of
`task/006-document-grants.md` without changing it. Scope: installed PC packages
(SDK and native) and the shell picker, which shares the same broker.

## 1. Current grant model (verified 2026-10-08)

- `springboard/src/bare/app_policy.rs` `Policy::from_permissions` accepts
  `network`, `storage`, `audio`, `notifications`, and `remote-desktop` (only for
  `is.olibuijr.remote.desktop`). Anything else fails launch.
- `command()` (app_policy.rs:162) builds a fail-closed bubblewrap command:
  `--unshare-all`, `--cap-drop ALL`, uid/gid of the installed non-root account,
  `--disable-userns` for SDK apps, `--tmpfs /tmp`, `--ro-bind /usr /etc ...`.
- Storage: `if policy.storage { --bind <account.home> <account.home> }`
  (app_policy.rs:230-231). This is read-write and covers the whole home
  directory. This is the only file access grant an app can request.
- Without `storage` the app gets a private home: `data/home` is bound over
  `account.home` (app_policy.rs:232-236). Neighbouring files are not visible.
- Manifests declaring `storage`: 34 `springboard/apps/*/manifest.toml` entries
  at the time of writing (48 on 2026-10-13). Examples: `files`,
  `notes-desktop`.
- Native apps additionally get the compositor socket, read-only `session.json`,
  `/sys`, `/run/udev`, and `/dev/dri`. There is no D-Bus socket.
- `docs/app-isolation.md` (lines 25, 29-30, 33) states the same: storage is the
  full home, and a document-picker portal is future work. This doc agrees.

Corrections to `task/006-document-grants.md`:
- Its citation `app_policy.rs:228-234` is off. The storage bind is at 230-231;
  the private-home fallback is 232-236.
- The SDK channel already passes file descriptors (SCM_RIGHTS) for shared
  buffers (`springboard/proto/src/lib.rs:80`, test at line 458). The portal can
  reuse that transport. The task does not mention this.
- `ToApp::Open { url }` (`springboard/sdk/src/runtime.rs:165`) carries a URL
  only. It is not a document-open event.
- The Notes data path `/data/local/springboard/Notes` is a legacy path
  (`springboard/apps/notes-desktop/src/notes/mod.rs:41`). `notes/main.rs:15-23` uses it
  only when it exists, otherwise the private `ctx.data_dir`. The storage grant
  is therefore not obviously needed for Notes' own notes. This must be confirmed
  (attachments, import/export) before Notes loses it.
- The Files app already has its own picker code (`open_picker`,
  `springboard/src/files/mod.rs:1083`) for copy/move destinations. It is not a
  document picker for other apps.

Problems with the current model:
1. Any app with `storage` can read and overwrite every file in the home,
   including other apps' documents, SSH keys and browser profile data.
2. The grant is static: it is decided at install time and never revoked per file.
3. Apps cannot save-as to a chosen location without the broad grant.

## 2. Threat model

In scope: a malicious or buggy installed app trying to read or write files the
user did not choose; symlink and rename races on the chosen path; replaced or
swapped files between grant and open; a picker that is itself confused into
opening a path for the app.

Out of scope for this phase: kernel exploits, malicious root, Arch apps launched
outside `app_policy.rs` (see `docs/app-isolation.md`), and the Browser's nested
sandbox internals.

## 3. Proposed model

Principle: the shell is the only component that resolves user paths. An app
never receives a path it can traverse. It receives a file descriptor for one
user-selected file, scoped to a grant.

### 3.1 Grant record

Stored by the shell under the state directory (root-owned, mode 0700):

| Field | Meaning |
|---|---|
| `grant_id` | random 128-bit id, returned to the app |
| `app_id` | manifest id that requested it; checked on every use |
| `mode` | `read` or `write` (write implies save-as via temp file) |
| `path` | canonical path chosen by the user (for reopen only) |
| `dev`, `ino` | file identity at grant time |
| `sha256` | content hash at last successful save or grant (optional for read) |
| `created`, `expires` | expiry for transient grants (default: session) |
| `revoked` | set by the user from Settings or by app removal |

A grant is per app and per file. It does not grant the directory. Revocation
removes the record and closes any descriptor the shell still holds.

### 3.2 Opening as the user, not root

The shell runs as root. An open performed as root would bypass the user's file
permissions, so the picker and reopen must open as the account:
`setfsgid(gid)` / `setfsuid(uid)` around `openat2` (or fork a short-lived
helper running as the account). The descriptor is then passed to the app. If
the user cannot read the file, the request fails with `EACCES` and the app
learns nothing about the file.

Path resolution uses `openat2` with `RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH`
from a directory descriptor the shell holds. The file is opened `O_NOFOLLOW`.
The resulting descriptor is checked with `fstat` against the recorded
`dev`/`ino` on reopen. Mismatch means the file was replaced: the shell asks the
user before continuing, and the app sees `GRANT_STALE`.

### 3.3 Picker (runs in the shell)

- The shell renders the picker UI. The app process never draws it and never
  sees the directory listing.
- The user may choose multiple files, change directory, and cancel. Cancel
  returns `CANCELLED` and creates no grant.
- Filters (extensions and MIME hints) are advisory for the picker UI only; they
  do not widen access.
- PC presentation: macOS-style per `AGENTS.md`. The picker uses the same broker and records.

### 3.4 Save-as

1. App sends `SaveRequest { suggested_name, mode }`.
2. The shell chooses the destination with the picker and opens its directory with
   `O_DIRECTORY|O_NOFOLLOW`.
3. The shell creates a temp file with `openat(dirfd, tmpname, O_CREAT|O_EXCL)`
   and returns only that descriptor. The temp name stays inside the shell.
4. On `SaveCommit`, the shell calls `fsync`, `renameat(dirfd, tmp, dirfd, final)`
   and `fsync` on the directory, then updates `sha256` and `dev`/`ino`.
5. On `SaveAbort` or app exit, the temp file is unlinked.

The atomic-writer requirement in `task/006` (step 2) maps to this sequence.

### 3.5 Recovery of unsaved edits

Apps keep unsaved buffers in their private data directory (`BIFROST_DATA`).
After a grant is revoked, the app restarts, or the shell crashes, the recovery
file stays in private data until a save to a granted file succeeds. The app
may reopen the file only through a new picker or reopen request. No grant is
needed to read the app's own private data.

## 4. API surface

### 4.1 SDK apps (`springboard/proto`, `springboard/sdk`)

Requires a new protocol version. Existing tags are 1-13 and 0x101+; new tags
are allocated from the next free value. Names below are proposals.

App to shell (`ToShell`):
- `DocOpen { mode: Read|Write, multiple: bool, title: String, filters: Vec<String> }`
- `DocSave { suggested_name: String, filters: Vec<String> }`
- `DocCommit { request: u32 }`, `DocAbort { request: u32 }`
- `DocReopen { grant_id: [u8; 16] }`
- `DocRelease { grant_id: [u8; 16] }`

Shell to app (`ToApp`):
- `DocGranted { request: u32, grant_id, name: String, size: u64 }` with the
  descriptor passed as SCM_RIGHTS, the same way buffers are passed.
- `DocError { request: u32, code: DocErrorCode }`, where `DocErrorCode` is one of
  `Cancelled`, `Denied`, `NotFound`, `Stale`, `Revoked`, `Unsupported`.

SDK wrapper (Rust, illustrative):

```rust
let doc = ctx.open_document(Mode::Read, Filters::text())?;   // Cancelled -> Err
let mut f: std::fs::File = doc.file();                       // read-only fd
let save = ctx.save_document("notes.md")?;                    // write fd in shell temp
save.commit()?; // or save.abort() / drop
```

The app gets `std::fs::File`, not `PathBuf`. The `Path` is never exposed.

### 4.2 Native apps

Native apps have no SDK channel. Proposal: the shell creates a second
`socketpair` per launch for the document broker, passes it as
`BIFROST_DOC_FD` (the same mechanism as `BIFROST_SOCK`), and the child
preserves only that descriptor plus the Wayland socket. The message set is the
same as 4.1, framed on that socket. This must be checked against `apphost.rs`
and the native launch path before implementation (not yet verified).

### 4.3 Manifest

- `documents` (new): may request picker-based read/write grants. No directory
  bind. Default for new editors.
- `storage` (existing): unchanged. Still binds the whole home. Reserved for
  Files and documented exceptions.
- Unsupported permission names keep failing launch.

### 4.4 Linux XDG FileChooser (PC, later)

`task/006` step 5 asks for XDG Desktop Portal FileChooser. This is not in the
first phase and does not exist yet (`bifrost-portal` serves only the Settings interface). Apps are not given the session or system bus (`app-isolation.md`).
A portal client would need a D-Bus proxy that filters to
`org.freedesktop.portal.FileChooser` and returns descriptors through
the same broker. It is only useful for Arch applications that are currently
outside the installed-package policy. Decide after phase 3.

## 5. Migration

1. **Broker and records (no app change).** Add grant store, `openat2` helpers,
   stale checks and revocation in the shell. No manifest changes. Behaviour of
   existing `storage` is unchanged.
2. **SDK protocol.** Add the Doc messages and bump `sdk_version`. Old apps keep
   working. An app that sends Doc messages to an older shell gets `Unsupported`.
3. **Picker UI.** Shell picker, one shared model.
4. **One real editor migration.** Use Notes. Remove `storage`
   only after confirming attachment/import needs (section 1). Route: open,
   save-as, cancel, permission failure, recovery.
5. **Documentation and new packages.** New editors request `documents`. Document
   the route for remaining document apps (Books, Calculator history, Music
   import) as separate tasks.
6. **Storage review.** Each `storage` manifest is reviewed and either keeps it
   with a written reason or moves to `documents`. Existing packages keep
   `storage` until a documented, versioned permission change; installs are not
   broken silently.
7. **Optional:** XDG FileChooser proxy (section 4.4).

Rollback: the shell keeps `storage` behaviour, so reverting a migrated app to
its previous package restores its old access. Grant records are ignored by
older shells.

## 6. Test plan

Unit (shell, `cargo test -p springboard` at batch acceptance; not run for this doc):
- Permission parse: `documents` accepted; unknown still rejected; `storage`
  unchanged.
- Grant record: app id mismatch rejected; revoked grant rejected; expired grant
  rejected.
- Identity: replaced file (different ino) returns `Stale`; same file after
  rename within the directory is detected by ino.
- Path resolution: symlink in any component refused (`RESOLVE_NO_SYMLINKS`);
  `..` escape refused (`RESOLVE_BENEATH`).
- Save: temp file in the same directory; abort removes it; commit replaces the
  destination atomically.

Integration (Midget, later batch run; this task sends nothing to the device):
- Negative: an app with `documents` only cannot open a neighbouring file, its
  home, `/home/<user>/.ssh`, or another app's data.
- Negative: user cannot read file (`EACCES`) gets `Denied`, not root content.
- Symlink race: swap the chosen file for a symlink to `/etc/shadow` between
  picker and open; expect `Stale` or `Denied`, never the target content.
- Confused deputy: app sends a grant id from another app; expect `Denied`.
- Revocation mid-edit: unsaved buffer survives in `BIFROST_DATA`; save after
  re-grant succeeds.
- Restart: shell restart keeps grant records; reopen after restart checks
  identity.
- Cancel: no grant record, no descriptor, no temp file left.
- Protocol: old SDK app against new shell, and new app against old shell
  (expects `Unsupported`), no crash.

The picker is checked with a host preview before merge. No sound is involved.

Acceptance (from `task/006`): migrated editor has no home-wide grant; it opens
and saves only selected documents; negative tests above pass; no data loss on
revocation or restart.

## 7. Open questions

- Native launch descriptor plumbing (section 4.2): confirm in `apphost.rs`.
- Whether Notes needs `storage` for attachments or import.
- Whether a grant should persist across sessions or default to session-only.
- Expiry policy for write grants held by a running app.
- Whether Files should also move to a broker model (it lists the home by design).
