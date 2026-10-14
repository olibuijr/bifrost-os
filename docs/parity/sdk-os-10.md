# Parity sdk-os-10: system rows with an app-facing model

Fourteen matrix rows had status `os`: the system or a daemon did the work and apps had no SDK surface. Each row now
has a model that apps can call, or a model in `bifrost_ui` that the SDK re-exports. Every model is pure: no device
is opened, no sound card is touched and no GPU runs. Tests are headless in
`springboard/sdk/tests/parity_sdk-os-10.rs` and `springboard/ui/tests/parity_sdk-os-10.rs`.

## New SDK modules (`springboard/sdk/src`)

| Module | Mirrors | What it does | Wired to the system |
|---|---|---|---|
| `audio_hal.rs` | the audio hardware object model (system, device, stream) | builds devices, streams and stable object ids from the kernel PCM listing text | no; the shell reads only the default input |
| `hid_virtual.rs` | virtual keyboard reports | eight-byte boot keyboard reports, key and name checks, the boot keyboard report descriptor | no; no virtual device is created from it |
| `input_grab.rs` | exclusive device ownership | apps are refused a grab; the shell is refused for power and lid devices | the rule is `Roles::grabbable` in `springboard/src/bare/input.rs` |
| `video_decode.rs` | the choice of a video decoder | hardware when the codec is supported, software otherwise, or hardware required | no; the shell's probe is not passed in yet |
| `media_extension.rs` | format reader entitlements | a declaration grants entitlements; only a declaration with the format reader entitlement can register a reader | no; extensions run in-process |

## Existing models now cited by rows

- `springboard/sdk/src/audio_session.rs`: session categories, activation, interruptions and route changes.
  Nothing in the shell or `audiod` posts to it yet.
- `springboard/sdk/src/fswatch.rs`: `EventJournal` numbers change events and replays them after an id.
- `springboard/sdk/src/sign_in.rs`: callback scheme and one-time code lookup.
- `springboard/ui/src/av_player.rs`, `av_queue_player.rs`: the playback model.
- `springboard/ui/src/hid_report.rs`: report descriptors, field reads and writes.
- `springboard/ui/src/compute.rs`, `compute_dispatch.rs`: placement and grid arithmetic.
- `springboard/ui/src/format_readers.rs`: the reader registry and its professional gate.
- `springboard/src/media/codec.rs`, `capture.rs`: audio kind by extension or magic bytes; WAV duration.

## Shell change

`Roles::grabbable` in `springboard/src/bare/input.rs` is now `pub`, so the SDK applies the same grab rule as the
shell. Its behaviour is unchanged.

## What needs the lab machine

- Confirming that the PCM listing format matches the kernel on Midget.
- Wiring the session hub, the decoder probe and the format reader registry to the running shell and `audiod`.
- Any app that uses these models must be tested on Midget before its row becomes `parity`.
