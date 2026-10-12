# Wiring the library, load options and MIDI into Music (1.4.0)

Branch `wire2/music`, from `main` (release 0.5.37). Scope: `springboard/apps/music-desktop/` plus this note. No
builds were run in this round: the lead builds once. The earlier note `docs/parity/wire-app-music.md` (1.3.0) covers
the Now Playing session and is not changed here.

## What is now wired

1. **Library change observation** (`src/music.rs`, `MediaLibrary` from `bifrost_app::controls::media_changes`).
   - The scan job returns the library and a stamp per file (`library::stamps_of`: the path, and a revision made from
     the modification time and size).
   - Every 5 seconds (`LIBRARY_CHECK_SECS`), when no scan is running, a background job re-reads the folders' file
     list and stamps. `MediaLibrary::observe` compares it with the last scan. A change (added, removed or changed
     file) starts a new scan, and the Songs, Albums and Artists lists are rebuilt from it.
   - Notifications are switched on at start (`begin_generating`), so the change is reported.
2. **Library unloading** (`Music::reload_library`, `MediaLibrary::unload`). Drops the snapshot, then starts a scan.
   The function is public. No menu item calls it yet (see gaps).
3. **Library load options** (`src/music/library.rs`, `LoadOptions::from_keys`, `library_load_options`, `plan`).
   The Music roots are sources: the root (its own files) and each folder directly inside it. `LoadOptions::filter`
   keeps the sources a load keeps, and `plan` turns the result into the folders to walk and the folders to skip.
   The keys are read from the app's defaults (`ctx.defaults()`):
   - `music.library.loadFolders` (bool; absent means folders load),
   - `music.library.include` (paths, one per line; absent means no include filter),
   - `music.library.exclude` (paths, one per line).
4. **MIDI files** (`src/music/library.rs`, `scan_midi`, `is_midi`, `Song::is_midi`; the reader is
   `springboard/core/src/smf.rs`, `Smf::parse`). `.mid` and `.midi` files are found by the scan, read with the
   Standard MIDI File reader, and listed with the sequence's length (`Smf::duration`, seconds to ms). They are listed
   under the artist "MIDI file". A file over 16 MiB or one that does not parse is skipped.
   - A MIDI song is never put in the queue (`play_songs` filters it out and moves the start to the same song).
     The audio service has no MIDI decoder, so a MIDI file could not play.

## How a user reaches it

- **Change observation:** copy, rename or delete a song in `~/Music`. Within about 5 seconds the lists update. Nothing
  needs a restart.
- **MIDI:** a `.mid` file in `~/Music` appears in the Songs list with its length and the artist "MIDI file".
  Tapping it does not play it (no sound bank yet).
- **Load options:** there is no settings page for them. Set them in the app's `defaults.txt` (in the app's data
  directory, `is.olibuijr.music.desktop`). The file is one `kind<TAB>key<TAB>value` line per key; `b` is a bool
  (`1` or `0`), `s` is a string. Example for excluding `~/Music/Podcasts` and keeping the root's files:
  `s<TAB>music.library.exclude<TAB>/home/<user>/Music/Podcasts` (one folder per line). Restart Music after editing.

## How to verify on the test PC (Midget)

1. `bifrost pc midget open-app is.olibuijr.music.desktop`, then `bifrost pc midget screen /tmp/music-1.png`. The
   Songs list is the same as before this change.
2. Change observation: `ssh midget 'cp /usr/share/doc/some.mp3 ~/Music/'` (any audio file), wait 10 seconds, then
   screen again. The new song is listed. Then `ssh midget 'rm ~/Music/<that file>'` and wait 10 seconds. The song is
   gone. The Dock tile shows scanning progress while the scan runs.
3. MIDI: `ssh midget 'cp <a .mid file> ~/Music/'`. The file is listed with a length and artist "MIDI file". Tap its
   row (`bifrost pc midget tap-window is.olibuijr.music.desktop X Y`): nothing starts, and the mini player stays as it
   was.
4. Load options: write the `music.library.exclude` line from above into `defaults.txt`, restart Music, and check that
   the excluded folder's songs are gone while the others remain.

Expected: no crash on a damaged `.mid` file (it is skipped), and the scan does not block the window (it runs on the
utility queue).

## REA evidence

- Media Library page: `ev_7a88fb87b8e265f868236c762574f55e412c300e05377dbcfa0ce05a18ebb826` (taken in wave 6, reused).
- iTunes Library page: `ev_f50af9fca12473e24e70819f376bf075d9ed4655d6893da96c34b3e7d3984614` (taken in wave 6, reused).
- AVMIDIPlayer page: `ev_88898b5c15a7ce64e376a4d6949751eacd3ff883a08bc159b10d077931cc4a0e` (taken in wave 6, b11, reused).
- Official DocC read in this round with `python3 -I tools/parity/docc.py ituneslibrary/itlibrary` (reloadData and
  unloadData: "Refreshes the data" and "Unloads the data") and `avfaudio/avmidiplayer` (playback "through a system
  sound module", which needs a sound bank).

No new REA page was captured in this round: the reused ids cover the same pages.

## Remaining gaps

1. **MIDI sound.** `springboard/core/src/smf.rs` has the parser and the `Playback` timeline, but no sound bank and no
   audio output. A MIDI file is listed and timed, not played. Closing this needs a soundbank player in audiod.
2. **Reload menu item.** `Music::reload_library` is not reachable from a menu yet. Adding it touches the menu list
   (the compliance test checks menu rules), so it is left for a menu change.
3. **Load options page.** The options are read from `defaults.txt` only. A settings sheet would replace that.
4. **Observation for other apps.** No library-change message is published to other apps (`DidChangeLibraryMessage`
   has no SDK publication). The stamps are polled every 5 seconds, and the check is a directory walk, not an event
   stream. It also does not see changes to the tags of a file whose size and modification time stay the same.
5. **`apiVersion`** of `ITLibrary` is not modelled (no SDK control reports the library version).
6. **Cargo.lock.** The version of `bifrost-music` in `springboard/Cargo.lock` is still 1.3.0. The lead updates the lock
   on the next build (it is outside this folder).
7. **Shell photo scan.** `springboard/src/shell/photos` still rescans on its own; not changed here.

## Matrix rows this touches

Paths are relative to the repo. The feature strings are exactly as in `docs/parity-matrix/data/<slug>.json`. Status
stays `partial` for each row: none is complete on Midget yet.

| Slug | Feature (as in the matrix) | BifrOSt path |
|---|---|---|
| media-library | Library entry point (MLMediaLibrary) | springboard/apps/music-desktop/src/music/library.rs (library_load_options, plan, find_files_except) |
| media-library | Media sources (MLMediaSource) | springboard/apps/music-desktop/src/music/library.rs (plan, child_folders) |
| media-library | Live changes to the collection | springboard/apps/music-desktop/src/music.rs (watch, tick, apply: Done::Stamps) |
| itunes-library | Library entry point (ITLibrary, reloadData, unloadData, apiVersion) | springboard/apps/music-desktop/src/music.rs (reload_library, MediaLibrary::unload); apiVersion not modelled |
| itunes-library | Change notification (DidChangeLibraryMessage) | springboard/apps/music-desktop/src/music.rs (MediaLibrary::observe); no SDK publication to other apps |
| avfaudio | MIDI playback (AVMIDIPlayer) | springboard/apps/music-desktop/src/music/library.rs (scan_midi); springboard/core/src/smf.rs; no sound bank |

## Tests

Unit tests in `src/music/library.rs` (run by the lead with `cargo test -p bifrost-music`):

- `midi_files_are_recognised_by_extension`: `.MID`, `.midi` true; `.mp3`, no extension false.
- `a_midi_file_is_listed_with_its_sequence_length_and_one_that_does_not_parse_is_skipped`: a format 0 file with a note
  at tick 0 and the end of track at tick 960 (480 ticks per quarter, default tempo) is 1000.0 ms; the file that does
  not parse is skipped; the title is the file stem; the song is MIDI.
- `the_load_options_choose_the_folders_a_library_walks`: four cases over a root with folders `a` and `b`, worked out by
  hand (all, `a` excluded, folders off, only `b` included).
- `stamps_name_each_file_and_move_when_it_changes`: the revision is stable for an unchanged file and changes when the
  size changes.
- `the_library_load_keys_are_read_from_the_defaults`: the three keys, blank lines dropped, paths trimmed.

The change-observation model (`MediaLibrary::observe`) has its own tests in `springboard/ui/src/media_changes.rs`.
Nothing new is tested on the Music window (the shell is not run in unit tests).

`python3 tools/check-names.py`: exit 0, no hits.
