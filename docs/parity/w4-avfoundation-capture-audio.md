# Parity wave 4: AVFoundation (Capture, Audio)

Two pure models close two gap rows of the AVFoundation (Capture, Audio) framework. Both live in the `bifrost_ui`
crate, which holds the pure models for apps and the shell. Neither opens a device, shows a dialog or writes a file.
Nothing is wired into the shell or the apps yet, and no host build or device test has run in this wave (no builds were
allowed for this round). The rows move from gap to partial, not to parity.

## What was built

### `springboard/ui/src/capture_access.rs`

Capture permission for one app, one status per media type (video and audio), as the system keeps them.

- `Status`: not determined, restricted, denied, authorized.
- `request(media, granted)`: answers the one-time prompt. A decided status is never changed by a later answer.
- `set_by_user(media, allowed)`: a change made in Settings. Refused while the system restricts the media type.
- `set_restricted(media, restricted)`: a system restriction. Lifting it returns the type to not determined.
- `can_capture(media)`: true only when the status is authorized. The shell asks this before it opens a device.

Mirrors the official per-media-type capture authorization: `authorizationStatus(for:)`, `requestAccess(for:completionHandler:)`
and the `AVAuthorizationStatus` cases `notDetermined`, `restricted`, `denied`, `authorized`.

### `springboard/ui/src/audio_settings.rs`

The recording format as a settings table: format identifier, sample rate, channel count, linear PCM sample layout
(bit depth, float, byte order, interleaving) and encoder quality.

- `Settings::linear_pcm(rate, channels)`: a 16-bit, interleaved, little-endian, integer request.
- `Settings::resolve(supported)`: checks the request against the (rate, channels) pairs the recorder tries, in the
  order the rules run: format, rate range (`MIN_RATE` 8 kHz to `MAX_RATE` 192 kHz), sample layout, exact rate, exact
  channels. Returns `Pcm { sample_rate, channels, bit_depth }` or the first rule the request breaks.
- AAC is named (`FormatId::Aac`) so a request is refused clearly. The recorder does not encode it.
- Encoder quality is accepted and ignored for linear PCM, as the official settings define it for compressed formats only.

Mirrors the official format settings: the format identifier, sample rate and channel count keys, and the linear PCM
settings (bit depth, float, big-endian, non-interleaved). The encoder quality field follows the encoder settings group.

## Official API names mirrored

| Name | Official page |
|---|---|
| `AVCaptureDevice.authorizationStatus(for:)` | https://developer.apple.com/documentation/avfoundation/avcapturedevice/authorizationstatus(for:) |
| `AVCaptureDevice.requestAccess(for:completionHandler:)` | https://developer.apple.com/documentation/avfoundation/avcapturedevice/requestaccess(for:completionhandler:) |
| `AVAuthorizationStatus` (notDetermined, restricted, denied, authorized) | https://developer.apple.com/documentation/avfoundation/avauthorizationstatus |
| Format settings (format identifier, sample rate, channel count) | https://developer.apple.com/documentation/avfoundation/format-settings |
| Linear PCM format settings (bit depth, float, big-endian, non-interleaved) | https://developer.apple.com/documentation/avfoundation/linear-pcm-format-settings |
| Audio settings (group overview) | https://developer.apple.com/documentation/avfoundation/audio-settings |

## REA evidence

- Framework landing page (`https://developer.apple.com/documentation/avfoundation`): `ev_0eecc1cf3b25d77f59f06347b6275ca90874dd2880f4dc2dc554d88bbcdb7692`.
  The rendered page lists the capture topics ("Configure built-in cameras and microphones...", "Capture audio and video
  directly to media files...") and the audio settings group that these models mirror.

## Matrix rows closed

Status is the new status for the row after this wave: partial. The matrix itself is updated centrally by the lead.

| Framework slug | Feature (as written in `docs/parity-matrix/data/avfoundation-capture-audio.json`) | New status | BifrOSt path |
|---|---|---|---|
| avfoundation-capture-audio | Camera/microphone permission | partial | springboard/ui/src/capture_access.rs |
| avfoundation-capture-audio | Audio settings keys | partial | springboard/ui/src/audio_settings.rs |

## Still open for these rows

- Camera and microphone prompts are not shown by the shell yet. The camera app and Voice Memos do not call
  `can_capture` or `request`. Restriction sources (parental controls, device policy) are not read from the system.
- The recording settings are not used by the shell recorder. It still picks the first working PCM format from its own
  table and records interleaved S16 only. AAC and ALAC encoding do not exist.
- No host unit test run and no device test (VM or Midget) has been done for either module in this wave.
