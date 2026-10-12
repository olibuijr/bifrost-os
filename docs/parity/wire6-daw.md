# Wire 6: DAW (apps/daw) audio conversion export

Branch `wire6/daw`, app `springboard/apps/daw` (bifrost-daw), version 0.5.0. Built from release 0.5.43.
Scope: the audio file conversion row (PCM conversion and sample-rate change on export). Follows
`docs/parity/wire4-daw.md`.

## What is now wired

| Feature | Model | Wiring in the app |
|---|---|---|
| Sample-rate and sample-format conversion for export | `springboard/ui/src/pcm_convert.rs` (`Resampler`, `f32_to_s16`) through `bifrost_app::controls::pcm_convert` | `springboard/apps/daw/src/export.rs` `converted_wav`, `springboard/apps/daw/src/wav.rs` `encode_s16`; `main.rs` `export_region` |

### How a user reaches it

- **Export CD**: in a song's Session view, with no trim active, the header button "Export CD" (next to "Export")
  writes the song as a 16-bit stereo WAV at 44.1 kHz. The file is in the song store's `exports/` folder, named
  `<song>-<start ms>-<end ms>-cd.wav`, and the status line shows its path.
- **Export** (unchanged): 32-bit float WAV at the render rate (48 kHz), as before.
- Trim: with Trim active, "Export" writes the trimmed part. "Export CD" is hidden while trimming, because the
  Start and End buttons take its place in the header; finish the trim (Export) or cancel it first.

### Behaviour and approximations

- The conversion is linear interpolation (`Resampler`), not a band-limited filter. It is adequate for the
  48 kHz to 44.1 kHz step but adds some high-frequency loss and imaging that a sinc resampler would not.
- The 16-bit path has no dither. The float export and the render's own 16-bit output (`wav::encode16`, TPDF dither)
  are unchanged.
- The converter holds back the newest frame, so one frame of silence is appended before conversion and the output is
  cut to the length the rate ratio implies.

## Gaps (not wired in this round)

- **Sampler playback of complete audio** (`phase`, "Sampler playback of complete audio"): the DAW's sampler voices
  (`springboard/apps/daw/src/devices/inst/zones/voice.rs`) already play zones, but no sample-file loading path
  through the engine or asset models was added. Stays Partial.
- **Aggregate devices** (`core-audio`, "Aggregate devices"): `springboard/ui/src/audio_hardware.rs`
  (`AggregateDevice`) is modelled, but the audio service exposes only the default device. There is nothing to list
  in a device picker, so no picker was built. Stays Partial.
- The export rate is fixed at 44.1 kHz; there is no choice of rate or bit depth in the UI.
- The app's rendered output is stereo only, so there is no channel conversion in the export path.

## Evidence (official documentation)

Reference behaviour comes from developer.apple.com, as the DocC data (`python3 -I tools/parity/docc.py`) and the
rendered page (`tools/parity/rea-page.sh`).

| Topic | Page | REA evidence id |
|---|---|---|
| Audio Converter Services: converting between linear PCM formats and between linear PCM and compressed formats | `audiotoolbox/audio-converter-services` | `ev_553a607c139f809e0d1b2baa5e36f16aa478dfdecb0792d078e12988b5314dcf` |
| AudioHardwareAggregateDevice: an aggregate of subdevices with synchronised clocks (not wired) | `coreaudio/audiohardwareaggregatedevice` | `ev_4267cf9a542cf073dc4ca5914160befda993e89372642ff0b25a623b47a7778f` |

## How to verify on Midget

1. Publish and install the app on Midget (`ssh midget`, then `/usr/bin/springboard --install-store-app is.olibuijr.daw.desktop`).
2. Open DAW, open a song with clips, and tap a clip. Tap "Export CD" in the header.
3. The status line reads "Exported .../exports/<song>-0-<end>-cd.wav". Check the file exists on Midget under the
   song store's `exports/` folder.
4. Inspect the header: bytes 24 to 27 must read 44100 (little endian), and bytes 34 to 35 must read 16.
5. Tap "Trim", then check "Export CD" is hidden and "Start +1 Bar" and "End -1 Bar" are shown. Tap "Cancel Trim"
   and check "Export CD" returns.
6. Host tests (in `springboard/`): `cargo test -p bifrost-daw --lib export::` (the conversion tests: the header rate
   and bit depth, the converted length for a 2:1 step, and a zero rate refused), and `cargo test -p bifrost-daw --test
   wire4_rack` (unchanged).

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path | Status after this round |
|---|---|---|---|
| audio-toolbox | Audio codec and format conversion (Audio Converter, Audio Format Services) | springboard/apps/daw/src/export.rs (`converted_wav`), springboard/apps/daw/src/wav.rs (`encode_s16`), springboard/ui/src/pcm_convert.rs (`Resampler`, `f32_to_s16`) | Wired into the DAW export. Stays Partial: no general converter API for apps, no encoder. |
| core-audio | Aggregate devices | springboard/ui/src/audio_hardware.rs (`AggregateDevice`) | Not wired (gap above). Stays Partial. |
| phase | Sampler playback of complete audio | springboard/apps/daw/src/devices/inst/zones/voice.rs | Not wired (gap above). Stays Partial. |
