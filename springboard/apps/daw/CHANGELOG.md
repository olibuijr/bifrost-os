# Changelog

## 0.4.0 — 2026-10-12

- Rack inspector: the Rack view shows each parameter of the track's instrument and effects as a slider with its value in the device's unit. Click a slider to set the value; click a parameter's name to open it for automation as before.
- Track stream format: the Rack shows the track's playback format under the title, the sample rate, the channel layout and its channel names.
- Presets: "Save Preset" stores the instrument's current values as a numbered preset ("Preset 1", "Preset 2"...). Tap a preset in the list to apply it to the instrument. Presets are kept in the store's presets folder per instrument, so they are available in every song.
- Previous Track and Next Track move the Rack to another track.
## 0.3.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 0.3.0 — 2026-10-12

- Import MIDI: the "Import MIDI" button under the track list opens a panel for a Standard MIDI File (.mid). The notes become a new track on the synth, placed in the scene of the selected clip (or the first scene), and the clip grows to whole bars that fit them. Notes keep their timing at the song tempo.
- Trim and Export: Trim selects the whole song, and the Start and End buttons move the in-point and out-point one bar at a time (a bar is four beats at the song tempo). Export writes the trimmed part, or the whole song when not trimming, as a 32-bit float WAV in the song store's exports folder, and shows the file path in the status line. Cancel Trim leaves the song as it was.
- The export renders on the main thread, so a long song takes a moment to finish.

## 0.2.6 — 2026-10-12

- MIDI input: a recorder turns note-on and note-off messages (raw MIDI bytes or events) into song notes at their song position, with running status and the sustain pedal holding notes until it is released.
- MIDI output: a clip's notes can be scheduled as note-on and note-off bytes in song order, with note-offs first when a key repeats.
- MIDI clock: a song position in sixteenth steps converts to MIDI clock pulses (24 per quarter note), with start, continue and stop bytes.
- Device parameters: each instrument and effect exposes its parameters as a parameter tree with names, units and clamped values, and the engine reads those values back.
- Presets: a device's current values can be saved as a named preset, applied again, and kept as JSON.
- Output device: the DAW reads the default output from the shell and shows a status line when the output route changes (Speaker or Headphones) or the device list changes.
- Now Playing: while a song plays, Control Center shows it as a session with the project name; the tile's play, pause and stop control the DAW, and the session goes away on stop.
- Pause: a song can be paused from the Now Playing tile and resumed from the same position.
- Low Power Mode: a warning line shows at the bottom of the window while the shell reports Low Power Mode.

## 0.2.5 — 2026-10-11

- Full-size window: the header controls (Back, Play, Arrange, Rack, Auto, Stop, New Song) start right of the traffic lights and are published as accessible buttons.
- Surfaces follow Light and Dark through the label tokens; font sizes and spacing use the macOS and spacing tokens.
- Has an app icon, and the reverb parameter table is one entry per line.

## 0.2.4 — 2026-10-10

- Follows the system Light and Dark appearance: backgrounds, lists, track lanes and the empty state. The "DAW" title no longer overlaps the empty-state text.

## 0.2.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 0.2.2 — 2026-10-08

- Session view ports the mobile clip grid: tapping an empty slot makes a one-bar clip named after its track, and the clip shows its name with a note preview.
- + Track opens an instrument picker (every instrument the engine can build), and + Scene adds a scene up to eight.
- Stop sits in the scene row; M and S are narrower so track names are readable.

## 0.2.1 — 2026-10-08

- The header is taller, so the title no longer runs into the list below it.
- New Song in the song list creates an empty song (one scene) and opens it.
- + Track under the last track adds a synth track with a clip slot per scene, and saves the song.

## 0.2.0 — 2026-10-08

- Native DSP core: biquads (RBJ, WebAudio semantics), delay lines, PolyBLEP oscillators, ADSR, PRNG, equal-power pan,
  FFT-partitioned convolution for the reverbs.
- All 17 effects ported from the mobile plugins (filter, drive, eq, comp, delay, tremolo, chorus, phaser, autofilter,
  crush, stereo, duck, limiter, reverb, space, lofi, tapedelay) with the same parameter names and ranges.
- Instruments: 27 sampled devices on the zone engine (zones.json, velocity layers, round robin, loops, one-shot,
  choke groups), 6 subtractive synths (synth, pad, supersaw, bass, reese, wobble), pluck (Karplus-Strong), fm (2-op),
  procedural drums and piano, drum rack and sampler (WAV from the samples folder).
- Offline renderer: arrangement and scene timing, swing, meter, per-quarter-step automation, track/group volume and
  solo/mute, equal-power pan, master headroom, master effects, bus compressor, safety clip; 16-bit TPDF WAV.
- Listening lab: integrated LUFS (BS.1770 gating), true peak (4x linear), clipped samples, crest, band split, and the
  AI composer's target check.
- Session window: song list, track by scene grid, mute/solo, piano roll for the selected clip, transport.
- Playback through the audio service (render to cache, then queue).
- AI co-producer client (Anthropic Messages API; key read at runtime from the opencode auth store) with the core
  operations applied: set_bpm, set_swing, rename_track, set_mix, set_clip, clear_clip. `daw-ai` CLI.

Known differences from the mobile DAW (not yet ported or approximated):
- Sampled devices: no legato glide/portamento, articulation switching, sustain pedal, loop-seam and release-tail
  crossfades, release noise, brass envelopes.
- Subtractive synths: PWM, glide and the 303 accent/slide are approximated; the Reese beat is an LFO on cutoff.
- FM: first modulator only (algorithm switch not ported). Piano is additive, not the web model.
- Space/Lo-Fi use the shell's decoder at 16-bit; convolution adds one 512-frame block of latency.
- True peak is 4x linear interpolation, not a polyphase filter.
- Operations not yet applied by the AI client: scenes, arrangement, instruments, effects, compose.
- UI does not yet have editing (notes, automation lanes, devices, arrangement view).

## 0.1.0 — 2026-10-08

- Typed project model reading and writing the mobile DAW's project JSON without losing unknown fields.
- `daw-import` converts `data/projects` and `data/revisions` into the desktop store.
- Song list window.
