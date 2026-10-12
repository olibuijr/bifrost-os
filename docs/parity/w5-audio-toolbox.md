# Wave 5: Audio Toolbox (parity)

Two gap and OS-only rows of the Audio Toolbox framework now have pure Rust models in `springboard/ui/src/`. Both
are exposed to apps through `bifrost_app::controls` (the SDK re-exports `bifrost_ui` as `controls`). Neither is wired
into the audio service, the shell or an app yet, and neither has run on the VM or on Midget. No builds or tests were
run for this wave (host tests are pending the integration build).

## What was built

### `springboard/ui/src/echo_cancel.rs` (voice I/O echo cancellation)

- `EchoCanceller::new(taps, step)`: a normalised LMS (NLMS) echo canceller over a ring of far-end (playback
  reference) samples. Taps are 1 to `MAX_TAPS` (4096). The step must be in (0, 2).
- `process(far, near)`: pushes one far-end sample, then returns the near-end (microphone) sample with the echo
  estimate subtracted. Non-finite samples count as silence.
- Double-talk guard: adaptation is frozen on a sample whose near-end magnitude exceeds `DOUBLE_TALK_RATIO` (0.5) of
  the largest far-end magnitude in the window (Geigel-style). With no far-end signal, nothing is learnt.
- `reset()`, `taps()`.

Not covered: the voice-processing properties for automatic gain control, output mute, bypass, ducking of other audio,
and muted-speech activity events. Those need a system voice unit, which the capture path does not have.

### `springboard/ui/src/pcm_convert.rs` (audio codec and format conversion, linear PCM)

- Sample formats: `s16_to_f32` / `f32_to_s16` (range -32767..32767, clamped, NaN is silence), `u8_to_f32` /
  `f32_to_u8` (128 is silence), and whole-buffer helpers `s16_buffer_to_f32` / `f32_buffer_to_s16`.
- `remap_channels(input, from, to)`: mono to N copies the sample, N to mono averages each frame, and between
  multi-channel layouts the first `to` channels are kept and extra outputs repeat the last input channel.
- `Resampler::new(from_rate, to_rate, channels)` and `process(block)`: a stateful linear sample-rate converter. The
  output position and the held frame carry over between blocks, so a split stream converts like one block. `reset()`.

Not covered: sinc or higher-quality resampling, dithering, encoders, and compressed-format conversion. The AAC
decode path stays in `springboard/src/media/aac.rs`.

## Apple API names mirrored

- Audio Toolbox, "Audio Unit Voice I/O": the echo-cancellation part of voice processing (the matrix row "Voice I/O
  (Audio Unit Voice I/O, echo cancellation)"). Official page: https://developer.apple.com/documentation/audiotoolbox/audio-unit-voice-i-o
- Audio Toolbox, "Audio Converter Services" (`AudioConverterNew`, `AudioConverterConvertBuffer`,
  `AudioConverterReset`, `AudioConverterDispose`): the linear PCM conversion part (the matrix row "Audio codec and
  format conversion (Audio Converter, Audio Format Services)"). Official page:
  https://developer.apple.com/documentation/audiotoolbox/audio-converter-services

Official topic lists were read with `tools/parity/docc.py audiotoolbox/audio-unit-voice-i-o` and
`audiotoolbox/audio-converter-services`. The framework page (https://developer.apple.com/documentation/audiotoolbox)
was read as well.

## REA evidence

- Audio Toolbox landing page: `ev_90a68fdd88b29f2661b3e2ce38c8f7d6bff1c4963ca4084930ad12f5941dbe9d`
- Audio Unit Voice I/O: `ev_7d17f7d1f741cf881f8f21d22d62faa4ec0aa2387dfa0efc49daf1085379d1bf`
- Audio Converter Services: `ev_df93a4de7806ce8d9bf3c527fb8f077f41b23183af7bde064dfcff9420936de1`

## Matrix rows closed

Feature names are exactly as written in `docs/parity-matrix/data/audio-toolbox.json`. The rows move from `gap` and `os`
to `partial`.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| audio-toolbox | Voice I/O (Audio Unit Voice I/O, echo cancellation) | partial | springboard/ui/src/echo_cancel.rs |
| audio-toolbox | Audio codec and format conversion (Audio Converter, Audio Format Services) | partial | springboard/ui/src/pcm_convert.rs |

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked expected values:

- `echo_cancel`: refused construction, pass-through with no far-end signal, one-tap learning in one step, a delayed
  echo on the second tap, double-talk freezing adaptation, reset, and non-finite input.
- `pcm_convert`: 16-bit and 8-bit mappings, channel remapping, refused zero and ragged input, buffer helpers, rate
  converter refusals, same-rate pass-through across blocks, 2x upsampling with a held frame, 2x downsampling,
  stereo interpolation, and reset.

## Check

`python3 tools/check-names.py` exits 0. The `pub mod` lines for both modules are in `springboard/ui/src/lib.rs`.
