# Wiring the MIDI synthesizer into the audio decode path (wire3)

Branch `wire3/midi-synth`, from `main` (release 0.5.38). Scope: a new pure-Rust synthesizer in
`springboard/core/src/midi_synth.rs`, one `pub mod` line in `springboard/core/src/lib.rs`, a `.mid`/`.midi`
dispatch in the audio decode path (`springboard/src/media/audiofile.rs`), and the Music queue no longer leaving
MIDI files out (`springboard/apps/music-desktop/src/music.rs`, `play_songs`). No builds or tests were run in this
round: the lead builds once. The wire2 note `docs/parity/wire2-music.md` said MIDI files are "listed and timed, not
played (no sound bank)". That is now changed: they play, with a synthesizer that needs no sound bank.

## What is now wired

1. **Synthesizer** (`bifrost_core::midi_synth`, std only). `Renderer::new(&Smf, rate)` takes the parsed file from
   `bifrost_core::smf` and renders stereo S16 a block at a time (`next_block`), with `seek(seconds)` and
   `is_finished()`. `render(&Smf, rate)` returns the whole file.
   - Tone voices: additive, a sine plus harmonics, chosen by the General MIDI program. Programs 0 to 31 are
     `Soft` (weights 1, 0.5, 0.25; attack 10 ms, decay 0.10 s, sustain 0.7), 32 to 55 are `Warm` (four
     harmonics; attack 80 ms, decay 0.30 s, sustain 0.8), the rest are `Bright` (five harmonics; attack 20 ms,
     decay 0.15 s, sustain 0.6). Release is a linear fade over 0.2 s at note off.
   - Drums: channel 10 (index 9) plays each key as a noise burst with a decay chosen by key (bass drum 0.25 s,
     closed hi-hats 0.05 s, cymbals 0.6 s, others 0.15 s). Drums ring out on their own; note off does not cut them.
   - Controls followed: pitch bend (plus or minus 2 semitones), channel volume (CC 7), expression (CC 11), pan
     (CC 10, constant power), all sound off (CC 120), all notes off (CC 123), program change. At most 32 voices;
     a new note past that takes the oldest.
   - Not modelled: sustain pedal (CC 64), reverb, chorus, recorded instrument samples. Volume and expression
     changes apply to notes already sounding (they are read per sample), but a pitch bend applies to the voices
     of its channel only when it arrives.
2. **Decode path** (`springboard/src/media/audiofile.rs`). `open(path)` first checks `is_midi(path)` (extension
   `mid` or `midi`, any case). A `Midi` value reads the file (16 MiB limit, the same as the Music scan), parses it
   with `Smf::parse` and renders at the player's output rate, 48 kHz, so the player does not resample. A damaged
   file returns an error ("not a Standard MIDI File", "MIDI file ends early"), and the player shows it like any
   unplayable song. `Midi` implements `AudioDecoder`, so gapless queues, pause, next/previous and seeking work as
   they do for other files. `seek_ms` replays the controls up to the point, so the sound after a seek matches a
   render that played through.
3. **Music queue** (`play_songs`). The filter that left MIDI songs out of the queue is removed: a `.mid` file in
   the Songs list now queues and plays like a song. The start index is clamped to the queue.

## How a user reaches it

- **Music app:** put a `.mid` file in `~/Music`. It is listed in Songs with its length and the artist "MIDI file".
  Tap it: it plays through the same player as the other songs, with the position and the mini player.
- **Apps:** any app that hands a `.mid` path to the audio service's play queue (the `audio` SDK client's queue
  call, which the Music app uses) gets the same decoder, because the service opens files with
  `audiofile::open`.

## How to verify on the test PC (Midget)

1. Copy a short `.mid` file: `ssh midget 'cp <some>.mid ~/Music/'`. Open Music:
   `bifrost pc midget open-app is.olibuijr.music.desktop`, then `bifrost pc midget screen /tmp/midi-1.png`.
   The file is listed in Songs with its length and the artist "MIDI file".
2. Tap its row: `bifrost pc midget tap-window is.olibuijr.music.desktop X Y`. The mini player shows the title and
   the position counts up. The audio service writes samples muted until the speaker route is opened (see
   `springboard/src/media/audioplayer.rs`, `set_muted`), so the sound is heard only on a route that is unmuted.
   Check the position in the screenshot, then the sound on that route.
3. Pause, seek and next work: `bifrost pc midget tap-window` on the mini player controls, then `screen`.
4. A damaged `.mid` file (for example `ssh midget 'head -c 40 ~/Music/x.mid > ~/Music/bad.mid'`) is listed or
   skipped by the scan and does not crash Music. Playing it reports an error and the player stops.

Expected: no crash on any input; the scan does not block the window (it runs on the utility queue).

## REA evidence

- AVMIDIPlayer page (reused from wire2): `ev_88898b5c15a7ce64e376a4d6949751eacd3ff883a08bc159b10d077931cc4a0e`.
- Official DocC read in this round: `python3 -I tools/parity/docc.py avfaudio/avmidiplayer`. The reference
  behaviour is "plays MIDI data through a system sound module" with `prepareToPlay`, `play`, `stop`, `rate`,
  `currentPosition` and `duration`. The reference needs a sound bank; this implementation does not, and says so.

No new REA capture in this round: the reused id covers the same page.

## Matrix rows

| Slug (data file) | Feature (as written in the matrix JSON) | BifrOSt path |
|---|---|---|
| `avfaudio` | `MIDI playback (AVMIDIPlayer)` | `springboard/core/src/midi_synth.rs`, `springboard/src/media/audiofile.rs` (`Midi`, `is_midi`), `springboard/apps/music-desktop/src/music.rs` (`play_songs`) |

No `core-midi` row changes status: the synthesizer reads MIDI messages but does not encode them (no MIDI 1.0 or
UMP writer), and it has no device or endpoint layer. The `MIDI Messages` row keeps its gaps.

## Remaining gaps

1. **Reference sound bank.** The synth is additive and noise-drum only. The reference plays a sound bank with real
   instruments. Closing that needs a bank loader and a sampler, not this module.
2. **Sustain pedal, reverb, chorus** are not modelled. Notes held by CC 64 end at their note off.
3. **Seek** restarts sounding notes silent: a note held across the seek point is silent until its next note on.
4. **Duration** is the file's length in seconds from its tempo map; a file with tempo changes is timed by them.
5. **No app API for MIDI-specific playback** (rate, position, completion handler, as `AVMIDIPlayer` has). Apps use
   the audio service's queue, which takes any file the decoder opens.
6. **No menu or settings for MIDI.** Nothing new in the menus.
