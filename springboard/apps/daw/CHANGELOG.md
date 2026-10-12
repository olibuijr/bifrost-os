# Changelog

## 0.2.6 — 2026-10-12

- MIDI input: a recorder turns note-on and note-off messages (raw MIDI bytes or events) into song notes at their song position, with running status and the sustain pedal holding notes until it is released.
- MIDI output: a clip's notes can be scheduled as note-on and note-off bytes in song order, with note-offs first when a key repeats.
- MIDI clock: a song position in sixteenth steps converts to MIDI clock pulses (24 per quarter note), with start, continue and stop bytes.
- Device parameters: each instrument and effect exposes its parameters as a parameter tree with names, units and clamped values, and the engine reads those values back.
- Presets: a device's current values can be saved as a named preset, applied again, and kept as JSON.

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
