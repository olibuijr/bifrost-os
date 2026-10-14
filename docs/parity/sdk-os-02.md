# Parity sdk-os-02: system capabilities as SDK models

Wave 7 task sdk-os-02 gave apps typed models for capabilities that the system already owns. Each model is pure: it checks
its inputs and computes results, and it opens no device, file, audio stream or display. The matrix rows listed below cite
these modules and are tested by `springboard/sdk/tests/parity_sdk-os-02.rs`.

## Modules (crate `bifrost-app`, `springboard/sdk/src/`)

| Module | Mirrors | What works | What remains |
|---|---|---|---|
| `gpu_device` | device enumeration | render node names, the default device (lowest minor), device paths | apps get no device handle; the presenter still opens its own node |
| `drawable_pool` | drawable life cycle | two or three drawables; take, present, release, abandon; resize only while idle | no application surface is connected to the pool |
| `gpu_pass` | render pass descriptions, reduced-resolution blur | one to eight colour attachments, load and store actions, three fixed pipelines, blur divisor and working size | apps cannot submit draw calls; the presenter does not read passes from this model |
| `pixel_buffer` | pixel buffer layout and interop | BGRA and NV12 planes, row padding, byte size, byte addressing | no buffer object tied to the hardware decoder |
| `biquad` | biquadratic IIR filters | low-pass, high-pass, peaking sections; cookbook coefficients; magnitude response; block processing | no spectrogram; the playback service does not run these filters for apps |
| `extent_map` | file extents and block mapping | packing, overlap refusal, file offset to disk offset | the kernel file systems allocate; no real extent read yet |

## Rows that stay `os`

- `metalfx` (command-buffer encoding of scaler passes): the presenter runs GLES passes and the shape of the reference
  encoder does not apply here.
- `core-services` (power and sleep): sleep decisions belong to the shell and logind; a sleep block needs a service that
  does not exist.
- `avfaudio` (playback audio out): the audio output trait is internal to the media player; no app-facing audio output
  exists yet.
- `oslog` (subsystem ownership on read): the rule is enforced by the system logger on read; there is no app-facing
  subsystem API to add.

## Rows changed to `partial` through existing code

- `avfaudio` route changes cite `audio_session` (its route events, override rules and current route are unit tested).
- `address-book` cites `contacts` (apps request access, fetch and save through the contacts service).
