# Wire 2: Files app (wire2/files)

Scope: `springboard/apps/files/` (Files 1.18.0). Three of the four focus items were wired; two were not and are
listed under gaps.

## What is now wired

1. **Dark content background.** The content area behind the list, grid and columns is painted with
   `WINDOW_FRAME_FOCUSED` (`render/src/ui.rs`, dark 52,52,55 / light 236) instead of `DETAIL_BG` (pure black in
   Dark). Code: `apps/files/src/window.rs` `paint`, import in `apps/files/src/main.rs`. The list row stripes
   (`CARD_BG`) stay visible in both schemes. `DETAIL_BG` remains only for the View Options card
   (`views/options.rs`).
2. **Disk image inspection in Get Info.** For `.iso` and `.img` files, Get Info adds: Image (ISO 9660 disc image or
   Raw disk image), Volume, Volume Size and Image Status (Complete, or "Truncated: the file is shorter than its
   volume"). It reads the first 34 KB of the file and calls `bifrost_app::controls::disk_image::inspect`.
   Code: `apps/files/src/info.rs` (`disk_image_rows`, `image_rows`, tests in `image_tests`).
3. **Live folder refresh from the event journal.** Watch events are written into an `EventJournal` (256 entries).
   The open folder applies the events it has not applied yet (`replay_since` from its cursor). Events that arrive
   while the folder is still loading wait in the journal and are applied when the listing finishes. If the journal
   dropped events the folder had not seen, the folder lists again. Code: `apps/files/src/perf.rs` (`JOURNAL_CAP`,
   `unseen`, `record_events`, `catch_up`, test `unseen_events_replay_in_order_or_force_a_listing`), with the
   fields in `apps/files/src/main.rs` and the calls in `tick`.

## How a user reaches it

- Dark content: open Files in Dark appearance (Settings > Appearance, or Control Centre). The content area is grey.
- Get Info: select an `.iso` or `.img` file and press I with Cmd (or choose Get Info from its context menu). The rows appear below the
  Kind, Size and dates rows.
- Live refresh: copy or create files in the open folder from another app or the terminal; the list updates without
  a manual reload, including after a slow listing.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Dark: `bifrost pc midget appearance dark`, open Files; the content area is grey (not black) and the list stripes
   are visible. Then `bifrost pc midget appearance light` and check the content area is light grey.
3. Get Info: put a valid ISO (for example a small ISO 9660 image) and a truncated copy in a folder, open Get Info on
   each. Expected: "ISO 9660 disc image", its volume name, and "Complete" for the full file and "Truncated..." for the
   short one. A file named `.img` with no descriptor shows "Raw disk image" only.
4. Live refresh: open a folder in Files, then in a terminal on Midget run `touch <folder>/new-file` and
   `cp -r` a large folder into it while the listing is loading. The new items appear without navigating again.

## REA evidence

- `ev_b95bb7ac998a6b33f028e258d1d6dc0176dcf589e48676568c02d8f6ae510050`: DiskImageKit overview page, rendered
  (topic "Inspect disk images with image properties", `DiskImage` properties).
- `ev_62eee5ce5370fc4dff4e0b33f373e0bbebff1d1fcec76509bd8d8f852cbc2eff`: Core Services framework page, rendered
  (topic "File System Events": "Get notifications when the contents of a directory hierarchy change").
- The `docc.py` JSON endpoints for these paths returned 404 in this session; the rendered pages above are the
  evidence used.

## Remaining gaps

- **Quick Look embedded preview and data-based previews** (`sdk/src/peek.rs` `PreviewView`,
  `ui/src/preview.rs` `PreviewProvider` / `Registry::preview`): not wired. Files keeps its own preview panel
  (`apps/files/src/peek.rs`), which does not use the SDK models. Needs an SDK-hosted view an app can place, and a
  preview provider registry the shell consults. Not done in this round.
- **Connect to Server for ftp://** (`sdk/src/ftp.rs`): not wired. The model has no socket code by design; Files
  needs a control and data connection client, a location sheet, and a sidebar entry. Not done in this round.
- Not verified: no build or test run in this round (the lead builds once). The snapshot images
  `apps/files/tests/snapshots/files-dark.png` and `files-light.png` will differ in the content colour and need a
  bless (`bifrost pc acceptance --bless`).
- The journal is in memory: it does not survive a restart (as the model states).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| diskimagekit | Inspect image properties | springboard/apps/files/src/info.rs (disk_image_rows, image_rows) |
| core-services | Event history and event IDs | springboard/apps/files/src/perf.rs (catch_up, unseen, record_events) |

Rows are not promoted to `parity` yet: each needs the Midget check above first (`bifrost pc parity` update after
Midget passes).
