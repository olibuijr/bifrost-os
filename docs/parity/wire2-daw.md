# Wire 2: DAW (apps/daw) parity models reached by a user

Branch `wire2/daw`, app `springboard/apps/daw` (bifrost-daw), version 0.3.0. Built from release 0.5.37.
Scope: Standard MIDI File import, trim and export of a region, and the audio unit parameter models.

## What is now wired

| Feature | Model | Wiring in the app |
|---|---|---|
| Standard MIDI File import | `springboard/core/src/smf.rs` (`Smf`), `springboard/apps/daw/src/midi.rs` (`Recorder`) | `src/midi_file.rs` (`import_notes`, `fill_clip`, `clip_length`); `main.rs` `import_midi` |
| Media trim (in and out points, minimum length) | `springboard/ui/src/media_trim.rs` (`TrimSession`, `TrimResult`) | `main.rs` `toggle_trim`, `nudge_trim`; header buttons Trim, Start +1 Bar, End -1 Bar |
| Region export as WAV | `springboard/apps/daw/src/export.rs` (new), `src/wav.rs` (`encode_f32`, `write_file`) | `main.rs` `export`; header button Export |

The device parameter tree and presets (`springboard/apps/daw/src/params.rs`, over
`bifrost_ui::plugin_host`) were already in the crate with host tests but had no user path. This wave does not
change them. They are still not reachable from the Rack view, so the parameter row stays Partial.

### How a user reaches each feature

- **Import MIDI**: in a song's Session view, the grid's add row has an "Import MIDI" label to the right of
  "+ Scene". Tap it and the open panel offers .mid and .midi files. The notes become a new track on the synth
  (or the first instrument when there is no synth). They go into the clip of the selected clip's scene, or the
  first scene when no clip is selected. The clip grows to whole bars that cover the last note. The status line
  reports the note count, or the parse error for a file that is not a Standard MIDI File.
- **Trim**: header button "Trim" selects the whole song. "Start +1 Bar" moves the in-point one bar later and
  "End -1 Bar" moves the out-point one bar earlier. The session keeps at least one bar and stays inside the
  song. "Cancel Trim" leaves trimming without exporting.
- **Export**: header button "Export". With trimming on, it exports the selected region and ends trimming.
  Otherwise it exports the whole song. The file is 32-bit float stereo WAV in `<store>/exports/`, named
  `<project id>-<start ms>-<end ms>.wav`, and the status line shows the path.

### Behaviour and approximations

- Import places notes at the **song tempo**: seconds from the file are multiplied by the song BPM in sixteenth
  steps. A file whose own tempo differs plays for the same length it has on its own, which keeps the notes in
  step with the audio the song renders. The file's tempo map is not applied to the song.
- Only note-on, note-off and control change messages reach the recorder. Program change, pitch bend and
  pressure are ignored, and the note model has no channel field, so all channels land in one track.
- Trim moves in whole bars (4 beats at the song tempo). The meter of the song is not used.
- Export renders on the UI thread with the same renderer the player uses (`RenderOpts::default()`, two loops
  in scene mode). A long song stalls the window for the length of the render.

## Evidence (official documentation)

Reference behaviour comes from developer.apple.com as the DocC data and as the rendered page.

| Topic | Page | REA evidence id |
|---|---|---|
| AVMIDIPlayer: plays a MIDI file through a sound bank; the page says external synthesis is done with AVAudioEngine | `avfaudio/avmidiplayer` | `ev_b27bfc6104bf5df735ff232be0b28826993c5546c06923d8d7b31a52926f5636` |
| AUParameterTree: the tree of groups and parameters that an audio unit exposes | `audiotoolbox/auparametertree` | `ev_00ee52df56320b213e450037b4253ad41fdcb2e021791d6779671055975c98ef` |
| AVPlayerViewTrimResult: the OK and Cancel results of the trim UI | `avkit/avplayerviewtrimresult` | `ev_63964c79fc86d9d25495850f769660775c4ce66193acfa38fb77be38b72f26a5` |

## How to verify on Midget

1. Publish and install the app on Midget (`ssh midget`, then `/usr/bin/springboard --install-store-app is.olibuijr.daw.desktop`).
2. Open DAW, open or create a song (New Song), and add a scene with `+ Scene` if there is none.
3. Import: tap "Import MIDI", pick a .mid file, and check the track appears with notes in the clip. The status line
   shows the note count. Then tap the clip and Play. Use `bifrost pc midget state` to confirm the window.
4. Trim: tap "Trim", then "Start +1 Bar" and "End -1 Bar" a few times, then "Export". Check the status line
   path and the file in `<store>/exports/` (a 32-bit float WAV of the expected length, about one bar shorter per
   nudge at the song tempo).
5. Cancel: "Trim" then "Cancel Trim" must leave the song unchanged and export nothing.
6. Host tests: `cargo test -p bifrost-daw` (the `midi_file` and `export` unit tests) and `tests/parity_wire.rs`.

## Remaining gaps

- The rack has no preset or parameter-tree UI yet. `params.rs` (tree, presets, JSON) is ready to hook in.
- No plug-in host: external Audio Unit or AUv3 plug-ins cannot load. The parameter row stays Partial.
- SMF import reads a file only. Playing a .mid file through the DAW's synth is not wired (AVMIDIPlayer remains Partial).
- Export is not on a worker thread, and it writes WAV only. No MP3 or AAC encoder is linked.
- Trim is in whole bars only. There is no sub-bar drag, no waveform, and no timeline.
- Import into a track picks the synth. There is no channel split or instrument choice per MIDI channel.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path |
|---|---|---|
| avkit | Media trimming and export | springboard/apps/daw/src/export.rs, springboard/apps/daw/src/main.rs (`toggle_trim`, `export`), springboard/ui/src/media_trim.rs |
| avfaudio | MIDI playback (AVMIDIPlayer) | springboard/apps/daw/src/midi_file.rs (`import_notes`), springboard/core/src/smf.rs |
| audio-unit | MIDI input and note data | springboard/apps/daw/src/midi_file.rs, springboard/apps/daw/src/midi.rs |
| audio-unit | Parameters and presets | springboard/apps/daw/src/params.rs (not yet wired to the Rack view; stays Partial) |
