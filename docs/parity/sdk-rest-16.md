# Parity sdk-rest-16: intent presentation, system audio client, force-feedback cooperation, provider host

Four matrix rows that earlier rounds judged impossible to build now have a real, tested surface. Each module is a pure
model over an interface the system already has, and none is wired into an app or the shell yet.

## What is built

| Row (matrix file) | Module | Mirrors | Tested in |
|---|---|---|---|
| Schema domains and visual presentation (`app-intents`) | `springboard/sdk/src/intent_presentation.rs` | The reference intent schema domains and the title, subtitle and symbol an entity shows | `springboard/sdk/tests/parity_sdk-rest-16.rs` |
| System audio service (`audio-unit`) | `springboard/sdk/src/system_audio.rs` | The audio service wire protocol (`bifrost_proto::svc::audio`) that `audiod` serves | `springboard/sdk/tests/parity_sdk-rest-16.rs` (socket pair) |
| Cooperative level and coordinate system (`force-feedback`) | `springboard/core/src/ff_cooperation.rs` | Cooperative level flags and the polar and Cartesian direction forms | `springboard/core/tests/parity_sdk-rest-16.rs` |
| App extensions host (`network-extension`) | `springboard/core/src/provider_host.rs` | The configuration a network provider app declares for an extension point | `springboard/core/tests/parity_sdk-rest-16.rs` |

## Rules the modules enforce

- Intent presentation: a title is required (trimmed, at most 80 characters, no control characters); a subtitle is at most
  120 characters; a symbol is lowercase ASCII letters, digits and single dots; a domain comes from a closed set of thirteen.
- System audio: a command exists only after its constructor checks it (volume 0 to 1, seek finite and not negative, queue
  non-empty with a start inside it, recording to an absolute `.wav` path). The socket is `BIFROST_SERVICE_SOCK` when set,
  otherwise the default path that `audiod` listens on.
- Cooperative level: exclusive access needs foreground focus. A device has one exclusive holder; exclusive requests are
  refused while another app holds the device, and shared requests are refused while another app holds it exclusively.
- Directions: a polar angle is in degrees counter-clockwise from the positive x axis, in `[0, 360)`. A Cartesian vector of
  length zero has no direction.
- Provider host: a bundle identifier is reverse-DNS; a provider class starts with a letter; one provider is enabled per
  extension point; a provider process takes connections only after the launcher reports it running.

## What is not built

- Nothing reads the schema domains or the presentation. There is no schema registry, no picker and no visual intelligence
  entry point. The side button and the action button need hardware that BifrOSt does not have.
- No app uses the audio client. It has not been run against `audiod` on the lab machine.
- The force-feedback arbiter is not used by an input grab (`EVIOCGRAB`) or an effect upload. Three-axis directions are not
  modelled.
- Nothing launches a provider process, and no traffic is handed to a provider.

## What needs the lab machine

- The audio client round trip against a running `audiod` (ping, subscribe, a queue, volume).
- Any app that adopts the intent presentation, the audio client or the provider host.
