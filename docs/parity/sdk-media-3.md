# SDK media round 3 (sdk-media-3)

Reference evidence: the Core Audio and AVFAudio rows of the Media group (`pc parity gaps --group Media`). Evidence was
read from the shipped reference names only; no reference file was copied into the repo.

## Core Audio: device property listeners (partly closed)

- API: `bifrost_app::audio_devices::decode_event` now returns `Change::Volume(Volume)` for the `volume_changed` event.
- BifrOSt path: `springboard/src/shell/services/audio_devices.rs` (`Seen` keeps the level and mute, `diff` emits
  `volume_changed` with the same view as the `volume` method), `springboard/sdk/src/audio_devices.rs` (client side).
- Done: output level and mute changes reach every app, from any control (Control Center included), with the gain that plays.
  Default output changes already came through `route_changed`; device list changes through `devices_changed`.
- Still open: sample-rate change listener (no sample rate is reported by the device service yet), and an app-facing
  property listener for arbitrary device properties.
- Needs Midget check: change a volume from Control Center while an app with `audio_devices` listens; confirm one event per change.

## Consumers

- `springboard/apps/daw/src/system.rs`: the exhaustive match on the device change now ignores `Volume`
  (DAW 0.5.4, version bumped in `Cargo.toml`, `manifest.toml`, `Cargo.lock` and `CHANGELOG.md`).
- `springboard/apps/music-desktop/src/app.rs` matches `Change::Route` with `if let` and needs no change.

## Not advanced this round

- AVAudioSession category, mode and activation, interruptions and route-change notifications for apps: the pure model
  named in `docs/parity/theme-10.md` is not in this tree (no `springboard/ui/src/audio_session.rs`) and no shell
  service carries a session, so there is nothing to wire yet.
- AVAudioApplication record permission and input mute: `springboard/audio/policy/src/permissions.rs` has the
  microphone permission model, but the policy process is not started or reachable from the shell.
