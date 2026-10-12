# Wire 3: share menu to compose post sheet

Owner: the share wiring round (branch `wire3/share-compose`). Builds on `docs/parity/social.md` (the compose model)
and `docs/parity/wire2-*.md`.

## What is now wired

- The share menu (`springboard/src/shell/sharing.rs`) offers **Compose Post** (target id `compose-post`) before Copy
  whenever the shared items are non-empty. It is offered for every share, not only for apps that handle the items.
- Choosing it opens the compose sheet (`springboard/src/shell/compose_sheet.rs`, a modal overlay like the power sheet).
  The sheet is the framework model `bifrost_ui::compose::ComposeSheet`, filled from the share items:
  - text items join with newlines into the post text;
  - file items become attachments (each file name is one attachment, at most 8);
  - `http(s)` URL items become links (at most 8); file URLs and other schemes are dropped.
- The sheet shows the title, the first line of the text (cut to 60 characters), the attachment and link counts, and
  the characters left against a limit of **280** (`CHARACTER_LIMIT`, BifrOSt's choice; the framework states no limit).
  A post over the limit, or an empty post, cannot be posted.
- **Post** copies the post text, attachment names and links to the clipboard (one per line) and reports
  `compose-post` to the sender as done (`ToApp::ShareDone`). **Cancel** reports an empty target (cancelled).
- The model result maps to `ComposeResult`: `Done` for a sent, non-empty post, `Cancelled` otherwise.

## Gaps

- The post text is shown, not edited. The shell has no text field for this sheet yet, so the text comes only from the
  share.
- Nothing is sent to a social network. There is no service type, account store or transport (see `social.md`).
- "Service availability check" (`isAvailable(forServiceType:)`) is not wired.
- Nearby share destinations: `springboard/ui/src/object_push.rs` exists (Bluetooth OBEX push), but it has no shell
  transport and is not offered in the share menu. No AirDrop-like model exists, so none is wired.
- The share menu still reports only the chosen target; it has no posted-versus-cancelled state of its own beyond the
  compose target id.

## How a user reaches it

Open an app that shares (for example a text selection or a file), choose Share, then **Compose Post**. Press **Post**
to copy the post, or **Cancel** to drop it.

## How to verify on Midget

1. `bifrost pc midget state` to confirm the shell is up; open a text-sharing app and select some text.
2. Trigger Share and check that the menu lists "Compose Post" above "Copy".
3. Choose it: the sheet shows "Compose Post", the preview, `N characters left` and the attachment counts.
4. Press Post; paste with `bifrost pc midget` (or any app's paste) and check the text is on the clipboard.
5. Repeat with Cancel; the clipboard must be unchanged. Use `bifrost pc midget screen` as evidence.
6. Use `bifrost pc midget tap-window` with window-local pixels for the buttons (raw `tap` is offset).

Host tests: `springboard/src/shell/compose_sheet.rs` (`tests`) and `springboard/src/shell/sharing.rs` (`tests`).

## REA evidence

- `ev_2f266bb157b67161c6023c205dc0f5d22b719eed2bb3b9ac054e82711fb03f83`: rendered page of
  `social/slcomposeviewcontroller` (the setters return false when content does not fit or the sheet is presented; only
  the user edits after presenting; the completion handler may run on any thread).
- Official symbol text read with `tools/parity/docc.py social/slcomposeviewcontroller`.
- Earlier capture kept in the matrix: `ev_480f845a1c91c745b77162d082f5325cd9a09bcf497f3ef55b819f71e29e6435`.

## Matrix rows

| slug | feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| social | Compose post sheet | springboard/src/shell/compose_sheet.rs, springboard/src/shell/sharing.rs, springboard/ui/src/compose.rs |
| social | Post contents (initial text, images, URLs) | springboard/src/shell/compose_sheet.rs (`sheet_for`) |
| social | Completion result | springboard/src/shell/compose_sheet.rs (`compose_done`), springboard/src/shell/sharing.rs |

The matrix data is not edited in this round (owner rule: the lead publishes it). Expected status after Midget
verification: the first and third rows stay `partial` until the text editor exists; the second is `partial` for the
same reason.
