# Audio 13: bifrost-audio-toolbox

Crate `springboard/audio/toolbox` (contract: `docs/audio-architecture.md`, task `docs/audio/13-toolbox.md`).
Dependencies: `bifrost-audio-types` only. No `unsafe`.

## Implemented

| Module | What |
|---|---|
| `converter` | `SampleFormat` (U8, S8, S16, S24 packed, S24In32, S32, F32, F64; either endianness), `decode_samples` / `encode_samples`, `StreamFormat` bridging (`from_stream`, `stream_format`), channel mixing (`default_matrix`, `apply_matrix`, incl. 5.1 to stereo), `interleave` / `deinterleave`, and `Converter` (format + channels + rate in one object). Integers decode as `v / 2^(bits-1)` and encode as `round(x * 2^(bits-1))` clamped, so S16 round trips exactly (the old `pcm_convert.rs` used 32767 on the way back; this is the deliberate difference). |
| `resample` | `SincResampler`: Kaiser-windowed sinc, 512-phase table with linear interpolation, three qualities (16, 64, 128 taps), streaming (blocks give the same result as one shot), exact integer position arithmetic, `finish()` flushes the tail (`ceil(frames * to / from)` output frames). `thd_n_db` measures THD+N. The cheap adaptive drift resampler is `varispeed` (task 11) and was not touched. |
| `file` | Read/write WAV (incl. extensible, streamed sizes, extra chunks skipped), AIFF and AIFF-C (`NONE`, `twos`, `sowt`, `fl32`, `fl64`; 80-bit rate), CAF `lpcm`. `probe`, `decode`, `encode`, `read`, `write`, `FileReader` (frame-wise reads and seeks). |
| `queue` | `AudioQueue`: enqueue buffers, start/pause/stop(flush), volume, `render` (real-time safe: preallocated, no locks), `drain_completed`, sample time. |
| `system_sound` | `SystemSounds`: sounds registered (samples or file) and converted to the output rate/channels, `play`, additive `mix_into` for the sound-effects role stream, built-in alert tone (id 0). `ROLE = "sound-effects"`. |

Errors: malformed file bytes `Protocol`, unsupported but valid `BadValue`, file system `Io`.

## Contract

No contract changes. `StreamFormat::float32/integer` were `todo!("audio-01")` when this was written and are implemented now; the toolbox still `SampleFormat::stream_format` builds the struct literally.

## Left (todo_left)

- Move the compressed decoders (`media/audiofile.rs`: MP3, AAC/M4A, FLAC, Vorbis, MIDI rendering) and `media/codec.rs` etc. into the toolbox. They depend on `bifrost-core` and `springboard/src/media/*`, which the toolbox is not allowed to depend on yet (only std, libc, audio crates), so the old files stay for callers to switch later.
- `ui/src/pcm_convert.rs` is superseded (copy-equivalent plus more) but left in place.
- Planar (non-interleaved) byte buffers in `Converter` (only f32 planar via `interleave`).
- No dithering, no 8-bit CAF/AIFF unsigned, no CAF 24-in-32.
- Logging (`is.olibuijr.audio`) is not wired: the crate does not log.
- Nothing here ran: per the rules only `cargo check` was used. All `#[cfg(test)]` values are worked by hand and must be run by the lead.

## Verification for the lead

Host:

```
cd springboard
cargo test -p bifrost-audio-toolbox
cargo run --release --example thdn -p bifrost-audio-toolbox
```

Record the `thdn` output here (THD+N, 1 kHz at -6 dBFS). Unit test thresholds: 44.1 to 48 kHz Standard below -60 dB (expected far lower); if `thd_n_44100_to_48000_is_below_minus_60_db` fails, the kernel table (`resample.rs`) is the place to look.

| Case | THD+N |
|---|---|
| 44100 to 48000 Standard | to be measured |

Midget: the toolbox has no device code. Check it end to end once the server is on Midget by playing a file through `AudioQueue::render` into a client stream (task 14 SDK) and a 44.1 kHz WAV to the 48 kHz device with `Converter`; confirm no clicks at block boundaries (block results equal the one-shot result in `blocks_match_one_shot`).
