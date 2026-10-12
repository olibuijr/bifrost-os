# Wiring Up Next into Music: library and analysis (1.6.0)

Branch `wire6/music`, from `main` (release 0.5.43). Scope: `springboard/apps/music-desktop/` plus this note. No builds
were run in this round: the lead builds once. `Cargo.lock` still names `bifrost-music` 1.5.0 (lead updates it).

Earlier notes: `docs/parity/wire5-music.md` (Clear Up Next, repeat and shuffle), `docs/parity/wire2-music.md` (library
change observation, load options, MIDI), `docs/parity/wire-app-music.md` (Now Playing session, remote commands).

## What was already wired (checked in the code, not changed here)

- **Songs, Artists and Albums browse.** The app groups songs itself (`music/library.rs` `Library::build`, `music/browse.rs`
  for the Songs sort and search). Artists and Albums are the Tab::Artists and Tab::Albums pages.
- **Library sync.** `bifrost_app::controls::media_changes::MediaLibrary` watches the folders (wire2-music).
- **Playlists.** `music/playlists.rs` reads and writes `.m3u` files.

## What is now wired

1. **Tempo column (music-understanding Rhythm, BPM).** `music/tempo.rs`:
   - `bpm_of(path)` decodes the first 60 seconds of a song with `springboard::media::audiofile::open` (the same decoders
     the player uses), mixes it to mono and runs `bifrost_app::music_understanding::analyze_rhythm`. It returns the
     beats per minute, or 0 for a file that cannot be decoded, a song with no beat (silence, speech) and MIDI files.
   - `label(bpm)` rounds to a whole number; 0 has no label.
   - `springboard/apps/music-desktop/src/music.rs`:
     - `start_tempo()` runs from `pump()` while the Songs list is on screen (the Songs tab, the root page, not the Now
       Playing sheet, not while the library is scanning). It starts analyses for the songs in list order, at most
       `TEMPO_JOBS` (2) at a time, on the Utility background lane (`Jobs<(PathBuf, f64)>`). Results are cached by path
       (`bpm`), so a rescan keeps them and no song is analysed twice. A song that gave 0 is cached too and not tried again.
     - `draw_song_rows` (the Songs list only, the `thumbs` rows) draws the number right-aligned `BPM_RIGHT_PT` (96 pt)
       from the trailing edge, left of the duration, in the secondary label colour (`LABEL_2`), at the sub-headline size.
   - The number is the tempo, not a key. No colour, no new control and no menu item are added.

## How a user reaches it

- Open Music and stay on Songs. Each row's tempo appears left of its length as the background analysis finishes, two
  songs at a time, in list order. Scrolling does not restart it; switching to Albums or Artists pauses it, and
  returning to Songs resumes it.

## How to verify on the test PC (Midget)

1. `bifrost pc midget open-app is.olibuijr.music.desktop`, then `bifrost pc midget screen /tmp/music-bpm-0.png`.
2. Wait about 30 seconds and take `/tmp/music-bpm-1.png`. Expected: the first few songs of the list show a whole number
   left of their length (for example 120), the rows below still have none.
3. Compare one song with a known tempo (a track with a clear beat, played from the list). Expected: the number is within
   a few beats per minute of the tempo a metronome or the file's own tag gives. Half or double tempo is a known
   limitation of autocorrelation tempo, not a bug to fix in this round.
4. Switch to Albums, then back to Songs. Expected: the already-known numbers are still shown at once; new ones resume.
5. Light and Dark: the number uses the secondary label colour in both appearances (`LABEL_2`).
6. A MIDI file in the list (artist "MIDI file") shows no number.

## REA evidence

- Music Understanding landing page (official documentation, rendered):
  `ev_58278152dd282d5522aec0f10d5ee0b8605e2ac0c883c0a75336d6b1bbf8a99c`. The page lists `RhythmResult` ("beats, bars,
  and beats per minute (BPM)") and says the framework runs file-based analysis over a complete audio track.
- Official DocC index read with `python3 -I tools/parity/docc.py musicunderstanding`.
- Earlier evidence for the rhythm and key models: see `docs/parity/w5-music-understanding.md`.

## Tests

Unit tests in `music/tempo.rs`, worked out by hand:

- `mono_passes_through_and_stereo_is_averaged`: `[8192]` mono is 0.25; `[16384, 16384, -16384, -16384]` stereo is
  `[0.5, -0.5]` (each sample is 16384 / 32768 = 0.5, the mean of two equal channels).
- `no_channels_gives_no_samples`: a channel count of 0 gives an empty buffer.
- `a_trailing_partial_frame_is_dropped`: `[16384, 16384, 8192]` stereo is `[0.5]`.
- `the_label_is_the_whole_bpm_and_unknown_is_none`: 119.6 gives "120", 92.2 gives "92", 0 and -3 give no label.
- `a_missing_file_has_no_tempo`: a path that does not exist gives 0.

The draw path and the job scheduling are not unit tested (the shell is not run in unit tests). The compliance
snapshot (`tests/snapshots/music-desktop-*.png`) may change where a fixture library has songs and analysis finishes
before the snapshot: the lead decides whether to bless it.

## Remaining gaps

1. **Browse by Genres.** `springboard/src/media/audiofile.rs` `Tags` has no genre field, so the scan cannot read the
   genre tag. `springboard/ui/src/media_query.rs` already has `Grouping::Genre` and `Query::genres()`, but the app has no
   genre data to group. Closing this needs the tag reader (in `springboard/src/`, outside this app) and a Genres tab.
2. **Smart playlists from predicates.** Not built. The query model (`media_query.rs`) matches text with equal or contains
   and numbers with equal only, and the app keeps no play counts, ratings or dates. A built-in smart list would be a list
   of empty tests, so none is shown. A user-built rule needs a rule sheet, which is a larger change than this round.
3. **Query model not used for browse.** Artists and Albums are grouped by the app's own index (`library.rs`), not by
   `media_query::Query`, because the query needs the same records and gives the same groups.
4. **Sort by BPM and a column header.** The number has no heading and the list cannot be sorted by it. The sort cycle
   (Title, Artist, Album, Recently Added) is unchanged.
5. **Key over time for songs.** `analyze_key` is in the SDK (wave 5) but is not shown in the app.
6. **Tempo of the whole song.** Only the first 60 seconds are read. A song whose beat starts late may read differently.
7. **Cost on a slow disk.** Each analysis decodes up to 60 seconds of audio. The Utility lane keeps it off the UI, but a
   large library takes several minutes to finish on first view.

## Matrix rows this touches

Paths are relative to the repo. Feature strings are exactly as in `docs/parity-matrix/data/<slug>.json`. Status stays
`partial`: none is complete on Midget yet.

| Slug | Feature (as in the matrix) | BifrOSt path |
|---|---|---|
| music-understanding | Rhythm (beats, bars, BPM) | springboard/apps/music-desktop/src/music/tempo.rs (bpm_of, label); springboard/apps/music-desktop/src/music.rs (start_tempo, draw_song_rows) |
| media-player | Media item queries and filters | not wired here: springboard/ui/src/media_query.rs is not used by the app (see gaps 1 to 3) |
| itunes-library | Media items (tracks) | unchanged: springboard/apps/music-desktop/src/music/library.rs (no genre tag, see gap 1) |
