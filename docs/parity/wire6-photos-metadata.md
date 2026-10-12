# Wire 6: Photos metadata, thumbnail preparation and Revert to Original

Branch `wire6/photos-metadata`, from `main` (0.5.43). Nothing was built or run in this round (the round forbids
builds). The new tests are written with hand-worked expected values and have not been executed; the lead builds once.

## What is now wired

| Piece | Before | Now |
|---|---|---|
| Info sheet (Photos, the viewer's Info button) | name, date, file size, and from the JPEG EXIF: camera, exposure, size, location | adds **Lens** (Exif LensModel, tag 0xA434), listed after Camera |
| Thumbnail preparation for the grid | the shell's own window bookkeeping (`thumb_wanted`) only | the grid's thumbnail window is also held in `bifrost_ui::photo_caching::Manager` (one preparation per visible or near-visible thumbnail, at the thumbnail size); preparations that scroll away are stopped |
| Revert to Original | the editor's Revert only reset unsaved sliders | an edited copy's Info sheet offers **Revert to Original**; the edited copy goes to Recently Deleted and the original opens |

The EXIF rows that already existed (Date, Camera, Exposure, Size, Location) are unchanged. The Info sheet keeps its
test (`the_info_sheet_lists_the_file_details_and_closes_on_a_tap`): a PNG still shows Name, Date and File Size only.

## How a user reaches it

1. **Info panel.** Open a photo in Photos and tap **Info** in the viewer's bottom bar. For a JPEG the sheet lists Date,
   Camera, Lens (when the file has one), Exposure, Size, Location and File Size. Tap anywhere to close.
2. **Revert to Original.** Edit a photo and tap **Save**, which writes an edited copy beside the original
   (`IMG_1 edited.png`). Open the edited copy, tap **Info**: a blue **Revert to Original** row sits under the detail
   rows. Tap it. The edited copy moves to Recently Deleted (recoverable for 30 days, as for any deleted photo), the
   original opens in the viewer, and the original file is never changed. The row is not offered for a copy in
   Recently Deleted, nor when the original is missing.
3. **Thumbnail preparation.** Not visible to the user. The grid behaves as before; the manager is the record of which
   thumbnails are being prepared.

The link between an edited copy and its original is stored in the Photos preferences file as
`edited:<copy path>` = `<original path>`, written when the copy is saved.

## How to verify on the test PC (Midget)

1. Deploy the lead's build (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Lens: copy a JPEG that carries a LensModel tag (a camera original) into `~/Pictures`, open Photos, open it, tap
   Info (`bifrost pc midget tap-window Photos X Y`, window-local pixels, centre-left of the bottom bar). The sheet shows a
   **Lens** row under **Camera**. A PNG shows no Lens row.
3. Revert: open a photo, tap Edit, rotate it, tap Done. Open the new `... edited.png` from the grid (it is the newest
   one near it), tap Info: **Revert to Original** sits at the foot of the sheet. Tap it. Check that the edited copy is
   gone from the grid, the original is open, and Recently Deleted lists the copy. Recover it there if needed.
4. Grid: scroll a long library up and down; the grid stays smooth and the thumbnails load as before (no visible change
   is expected; this is a regression check).
5. Screenshot the Info sheet with `bifrost pc midget screen PATH` and check it against the design rules: the sheet
   uses `SHEET`, `SEPARATOR` and `SECONDARY` as before, and the revert row uses `BLUE` (LINK), not `RED`.

## Host tests added (not run)

- `springboard/src/shell/photos/exif.rs`: `the_lens_model_is_read_and_listed_after_the_camera`. A TIFF with Make and
  LensModel only in the Exif sub-IFD: `lens` is `Some("EF24mm f/1.4G")` and `rows()` is exactly
  `[("Camera", "Canon"), ("Lens", "EF24mm f/1.4G")]` (Camera from the make alone, with no model).
- `springboard/src/shell/photos/mod.rs`:
  - `thumbnail_requests_cover_the_visible_cells_and_drop_the_rest` now also checks `preheat.len() == 12` (12 visible
    thumbnails, 12 distinct path keys, no extra rows ahead because all 12 are on screen) and that the first photo's key
    at `thumbs::SIZE` is started.
  - `revert_to_original_moves_the_edit_to_recently_deleted_and_opens_the_original`: two PNGs (`IMG_0000` original,
    `IMG_0001` edited copy); the link is set by hand; viewing index 1, Info, then a tap on the Revert row at
    `sy + row * (info.len() + 1) + row / 2` (info has Name, Date, File Size, so the row is the fifth). Expected: the
    copy is gone, the original exists, `trash.len() == 1`, and the viewer shows the original.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/photos/phadjustmentdata (rendered): "For each asset, Photos stores only one PHAdjustmentData object, representing the edit made to the asset's content"; the adjustment lets an app "reconstruct or revert" prior edits | `ev_423cd96616ca33a51b229fe8dd2cc402bb9ecca017339aacb5c06efee001e670` |

Official DocC (`tools/parity/docc.py`, read, no separate evidence id): `photos/phcontenteditinginput` (adjustment data,
previous edits), `photos/phadjustmentdata`, `photos/phcachingimagemanager` (start, stop, stop all, allows high-quality
images). The ImageIO EXIF page path given in the old notes did not resolve in DocC, so the EXIF LensModel tag comes from
the EXIF standard (tag 0xA434), not from an Apple page; no REA evidence was taken for it.

Earlier evidence (wave 5 and the image-i-o notes) still applies to the EXIF keys and the caching manager.

## Remaining gaps

- **Lens and EXIF for apps.** The EXIF reader is a Photos-app helper. No SDK call returns the metadata of an asset, so
  `Image and I/O metadata for apps` stays a gap (matrix rows `image-i-o` and `photokit` stay as they are).
- **Caching is bookkeeping.** `photo_caching::Manager` records which thumbnails are being prepared; decoding still runs
  on the shell's job pool. No delivery mode (fast, high quality) or content mode is used by the grid, and no app can
  start preparations.
- **Revert is a Photos-app action.** It moves the edited copy to Recently Deleted; it does not remove adjustments from
  an asset in place, because edits in this app are saved as copies. Apps have no Revert API (`PHAdjustmentData` is not
  exposed to SDK apps).
- **Edit links are per file.** `edited:<copy>` is in the Photos prefs file; a copy written by another app or moved by
  hand has no link and no Revert row.
- **Revert for a non-copy original.** An original with edited copies offers no revert; only the copy does.
- **XMP, IPTC and captions** (`springboard/core/src/imgmeta.rs`) are not read by the Info sheet.
- **Not built, not tested on Midget.** All code is unverified until the lead builds and runs the Photos tests.

## Matrix rows touched (for the lead to import; not edited in this round)

| Slug | Feature (exact text in `docs/parity-matrix/data/<slug>.json`) | Status | BifrOSt path |
|---|---|---|---|
| photokit | Asset metadata (EXIF and extended attributes) | partial | springboard/src/shell/photos/exif.rs (Lens row, LensModel tag 0xA434); springboard/src/shell/photos/mod.rs (Info sheet, `draw_info`) |
| photokit | Image and thumbnail loading and caching | partial | springboard/src/shell/photos/mod.rs (`request_thumbs`: `preheat` is `bifrost_ui::photo_caching::Manager`) |
| photokit | Editing a photo (rotate, crop, brightness, contrast, filters) | partial | springboard/src/shell/photos/mod.rs (Revert to Original on an edited copy: `revert_to_original`, `edited_original`) |
| image-i-o | EXIF metadata read (camera, date, exposure, size) | os | springboard/src/shell/photos/exif.rs (Lens added to the info sheet's rows) |
| core-location | Location metadata in photos (not Core Location) | partial | springboard/src/shell/photos/exif.rs:110 (format_gps); unchanged, shown in the Info sheet's Location row |

Suggested status: `photokit` / Editing a photo can move to a `partial` note that mentions Revert to Original; the
`image-i-o` row stays `os` because the metadata is still Photos-only.
