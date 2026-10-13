# Photos and Music (slice photos-music, round 75)

Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists the shipped Photos and Music bundles. Matrix
rows read from `docs/parity-matrix/data/photokit.json`, `media-library.json` and `image-i-o.json`, and the
`os/apps` entries for Photos and Music. The matrix rows were not edited (lead closes them). No REA evidence was
gathered this round: the lead's extracted nibs and Info.plists were not present when this slice ran.

## Done this round

### Non-destructive edits (Photos)
- API: none app-facing. Shell feature (`springboard/src/shell/photos/`).
- Before: the editor saved a PNG copy beside the original ("IMG edited.png") and Revert to Original moved that
  copy to Recently Deleted.
- Now: the edit is a recipe line in the settings store (`edit:<path>`, module `photos/recipe.rs`). The original is
  never written. The picture is decoded and adjusted with the recipe for the grid thumbnails, the viewer, the
  slideshow, Set as Wallpaper and Share › Copy / Save to Files (`decode_edited`). Reopening an edited photo starts
  from its saved adjustments (the editor decodes the original). Revert to Original clears the recipe.
- Deleted: `edit::save_copy`, `edit::copy_path`, `Done::Edited`, `edited_original`, `revert_to_original`, the
  `edited:<copy>` settings links, the copy tests in `edit.rs`.
- Matrix features it moves: "Editing a photo (rotate, crop, brightness, contrast, filters)" (photokit).
- Still needs Midget check: the editor's open-time decode of the original is synchronous on the UI thread (one
  JPEG decode per click on Edit). Check it on a large camera photo. Check that the grid thumbnail refreshes after Save.

### Folder import (Photos), model only
- `photos/import.rs`: `import(from, library) -> Report { copied, skipped, failed }`. Recursive, copies (never moves),
  skips hidden files and a picture whose name and size the library already holds, numbers a clashing name
  ("IMG_3 2.jpg"), removes a half-written copy on error. Tests: `copies_pictures_recursively...`,
  `a_name_the_library_uses_for...`, `a_missing_source_is_reported_not_panicked`.
- NOT WIRED: no Photos UI calls it yet. The sidebar has no Import row and there is no folder chooser. Next step: an
  Import row in the Photos sidebar (or File menu) that picks a source folder and calls `import`, then rescans.
- Matrix: "Asset resources and creating assets" is not closed by this (no SDK API).

## Reviewed, not changed

- Music Now Playing: `springboard/apps/music-desktop/src/music/nowplay.rs` (the module doc) says the session is
  not published to Control Center and that system commands need the shell's `now-playing` events. That is out of
  date: `app.rs` `publish_step` publishes the session, and `system_command` maps incoming commands to the transport
  (`Music::remote_command`, `Music::remote_seek`). Doc fix left for a Music round (it would need a version bump).
- Gapless playback: `springboard/src/media/audioplayer.rs` (the engine that `audiod` owns, `audiod/src/main.rs`)
  advances to the next queued song without draining the device, with a test
  (`plays_the_queue_gaplessly_to_the_end_and_rests_on_the_first_song`). Music talks to it through
  `bifrost_app::audio::Client`. Not verified on Midget.
- Music library from ~/Music, playlists, queue, Control Center tile and media keys: all present in
  `springboard/apps/music-desktop/src/music/`. Not changed.

## Not done (honest gaps)

- Photos albums, crop, rotate and adjust UI: existing (`albums.rs`, `edit.rs`, `slideshow.rs`), not re-measured.
- Photos share sheet: Slideshow, Copy, Save to Files and Send Nearby rows exist (`Target::SheetNearby`); Send Nearby's
  behaviour was not checked this round.
- Photos library import from a camera: not wired (see above).
- The matrix rows (`photokit`, `media-library`) still read Partial; the lead decides closures.
