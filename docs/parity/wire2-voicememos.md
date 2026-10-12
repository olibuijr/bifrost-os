# Wave 2 wiring: Voice Memos (1.4.0)

App: `springboard/apps/voicememos-desktop/` (`bifrost-voicememos`, now 1.4.0 in `Cargo.toml` and `manifest.toml`).
Branch `wire2/voicememos`. No builds were run in this round; the lead builds once. Host tests were written by hand and are
listed below but not run.

Focus of this round: trim handles and Trim / Export menu items (the trim already existed from the earlier wire round, so
this round adds the menu and the export), and a multi-line text view for notes.

## What is now wired

| Matrix slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | Model | BifrOSt path |
|---|---|---|---|
| `appkit-text` | Text Display: multi-line text view widget | `bifrost_app::controls::textview::TextView` (`springboard/ui/src/textview.rs`) | `src/voicememos.rs` `draw_notes`, `notes_key`, `notes_place`, `notes_box`, `notes_metrics`, `begin_notes`, `end_notes`; `src/store.rs` `notes_path`, `read_notes`, `write_notes` |
| `avkit` | Media trimming and export | `bifrost_app::controls::recording::Trim` (handles, already wired) and the new export copy | `src/voicememos.rs` `menu` (MENU_TRIM, MENU_EXPORT), `export_selected`; `src/store.rs` `export_copy` |
| `replaykit` | Preview and edit UI for a recording | same trim, now also from the Recording menu | `src/voicememos.rs` `begin_trim`, `menu`; `src/main.rs` `main_menu`, `menu_action` |

### Behaviour, for a user

- **Recording menu** (between the app menu and File / Edit / View / Window / Help):
  - **Trim…** (cmd+t): opens the trim view, the same as the toolbar's Trim button.
  - **Notes…** (cmd+shift+n): opens the notes card for the selected recording.
  - **Rename…**: the same as the toolbar's Rename.
  - **Export to Downloads** (cmd+shift+e): copies the recording as `<title>.wav` into `~/Downloads` (numbered when the name is
    taken). The original is untouched. The caption under the record button then shows `Exported to <path>`.
  - All four are greyed out when no recording is selected, and while one is being recorded or edited.
- **Notes**: a Notes button in the toolbar (between Trim and Rename) opens a full-pane card with Cancel and Done.
  - Type to write; Enter starts a new line; Backspace deletes; the arrow keys move the caret (Up and Down move by wrapped
    line, keeping the column).
  - Press in the card to place the caret; drag to select. Selected text is shown in the system selection colour.
  - **Done** saves the notes to a hidden `.<title>.notes.txt` beside the recording. **Cancel** discards the edits. Leaving
    the app saves them (like Done). Empty notes remove the file.
  - The notes follow the recording when it is renamed, and are removed with it when it is deleted. The hidden file is not
    listed as a recording.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Skip the VM.
2. `bifrost pc midget open-app is.olibuijr.voicememos.desktop`. Record a short take and stop it, then select it.
3. Open the **Recording** menu (between the app menu and File). Trim…, Notes…, Rename… and Export to Downloads are enabled.
4. **Notes**: type two lines, use Enter, Backspace and the arrow keys. Press Done. Reopen Notes: the text is still there.
   Rename the take: the notes stay with it. `ls ~/Downloads` after **Export to Downloads** shows the copy.
5. Click the window with `bifrost pc midget tap-window APP X Y` (window-local pixels). Use
   `bifrost pc midget key` for the typed text and arrows.
6. `bifrost pc midget screen /tmp/voicememos-notes.png` for the screenshot record.

Host tests added (not run; expected values worked out by hand):

- `store::tests::notes_sidecar_is_hidden_follows_the_rename_and_empty_notes_remove_it`
- `store::tests::export_copies_the_recording_and_keeps_the_original`
- `voicememos::tests::notes_save_beside_the_recording_and_the_menu_opens_them`

## REA and official-doc evidence

Official DocC and rendered pages (`python3 -I tools/parity/docc.py`, `tools/parity/rea-page.sh`):

- NSTextView (`appkit-text`, multi-line text view; the caret, selection, wrapping and scroll follow the view's model):
  `ev_a1b3ef78d4a0fcd94c13f8c5d06d5e71d098816689bb8a3584ab146bb59b711a`.
- NSMenu (menus; app menus sit between App and the standard menus, one item per command with key equivalents):
  `ev_d12ce87859d8723941b43671be11e2e79c8c0d5b77f2c0fdfe90c18e6f81df19`.
- Earlier rows from wave 1 still apply: AVAssetExportSession `ev_98898fb52f7856d5681302580007bace40fb726db8f8530c44922cadb31bd6aa`
  (trim by export) and RPPreviewViewController `ev_d92ca1738cada62eb3b8b206db8c121d0c4848c9cdbb5038f777727722949774`
  (preview and edit of a recording).

## Models not wired, and why

- **`media_trim::TrimSession`** (`springboard/ui/src/media_trim.rs`, the AVKit trim-mode state machine): not wired. The app
  already keeps its trim range in `recording::Trim`, which holds the same rules (the in-point and out-point stay inside the
  recording and at least the minimum apart), so a second model would duplicate state. The trim behaviour in the app is
  therefore unchanged apart from the menu entry.
- **Notes as a transcript**: the notes are typed text, not a speech-to-text transcript. No transcription service exists in
  the audio channel.
- **Export as a save panel**: the export goes to `~/Downloads`; there is no file-chooser service wired to this app.
- **Undo for the notes card**: the `TextView` keeps its editor's undo levels, but the card has no Undo menu item yet (the
  shell's Edit menu is not declared by this app).
- **Notes search, rich text, and the spell checker**: not in the model; plain text only.

## Remaining gaps

- **Compliance snapshots**: `tests/snapshots/voicememos-desktop-{dark,light}.png` show the old toolbar. The toolbar has a new
  Notes button, so the snapshots need a bless (`BIFROST_BLESS=1`) after the lead's build.
- **Lock file**: `springboard/Cargo.lock` lists `bifrost-voicememos` at the old version. It is outside this app folder and was
  not edited; the build refreshes it.
- **Matrix rows**: `docs/parity-matrix/` is not edited in this round. The three rows above should move to Partial only after
  the Midget check and the lead's update.
- **Key codes**: the arrow keys were added to the app's key map (codes 38 and 40, Up and Down), alongside the existing Left and Right (37 and 39).

## Matrix rows

| Slug | Feature (as in the json) | BifrOSt path |
|---|---|---|
| `appkit-text` | Text Display: multi-line text view widget | `springboard/apps/voicememos-desktop/src/voicememos.rs` (notes card), `src/store.rs` (notes sidecar) |
| `avkit` | Media trimming and export | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`menu`, `export_selected`), `src/store.rs` (`export_copy`) |
| `replaykit` | Preview and edit UI for a recording | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`begin_trim`, `menu`), `src/main.rs` (`main_menu`) |
