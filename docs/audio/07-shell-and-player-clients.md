# 07 Shell, media player and SDK clients

Task: `tasks/audio/07-shell-and-player-clients.md`. Contract: `docs/audio-architecture.md`.

## Implemented

- `media/speaker.rs`: `SpeakerOut` (the `AudioOut` the players use) opens an IO procedure on the default output
  device through `bifrost-audio-hal` (`System::connect("is.olibuijr.mediaplayer")`, `default_device(Role::Output)`).
  A lock-free SPSC `Ring` (atomics, no `unsafe`) carries S16 stereo from the player thread; the IO procedure
  (`Renderer`) reads it, converts to f32, resamples linearly when the device's nominal rate differs from the source
  rate, fills the cycle's buffers (extra channels silent) and writes silence while paused or starved. No
  allocation, locks or syscalls in the procedure. `played()` counts frames read; `discard` skips the queue; `flush`
  lets the queue play out (at most 600 ms) then drops it. Idle for 5 s releases the stream.
  `SpeakerOut::system_sounds` uses `Role::SoundEffects`.
- `media/alsalib.rs` deleted (the libasound `dlopen` host); `media/mod.rs` and `alsa.rs` comments updated.
- `media/capture.rs`: `Recorder::start` opens the default input device (mono, the device's nominal rate); an IO
  procedure converts to S16 into a `CaptureRing`, a reader thread collects the samples. Mixer routing,
  `/proc/asound` discovery and the kernel-ioctl capture are gone. `PermissionDenied` from the server maps to
  `io::ErrorKind::PermissionDenied`.
- `media/volume.rs`: `VolumeTarget`, implemented for the HAL `Device` (`VOLUME_SCALAR` and `MUTE`, output scope,
  main element). `Volume::apply(Option<&dyn VolumeTarget>)` sets the device controls; without a server, or on a
  refused write, the software gain carries the level. Also `default_output()` and `Volume::from_device`. The mixer
  path (`CONTROL_DEVICE`, `desktop_control`, `range_value`) is removed.
- `src/bin/shell.rs`: the volume effect, headset route changes and start-up apply the level through the device
  (lazy connect, reconnect after an error). `control_center.rs` and `volume_hud.rs` needed no change: they emit
  `Effect::Volume`, which now reaches the device.
- `audiod`: stays the crate and the wire id and socket `is.olibuijr.audio`, so `proto::svc::audio` and the apps do
  not change. On the audio server it is the client `is.olibuijr.mediaplayer` (`media::speaker::CLIENT`) and opens
  no `/dev/snd` node. Cargo gains the hal and types dependencies.
- `sdk/src/system_sound.rs`: `OutputRole`, `SoundKind::role()`, `PlayRequest::role()`: every system sound and
  alert maps to the sound-effects role (test included).

## Left

- Wire the audio service to play `SystemSounds` requests and the lock sound through `SpeakerOut::system_sounds`
  (nothing plays system sounds yet; key clicks stay off by design).
- Subscribe the shell to `VOLUME_SCALAR` with `System::listen` so changes from other clients move the HUD and
  slider (`Volume::from_device` is ready).
- Rename the crate, socket and manifest to `is.olibuijr.mediaplayer` once the apps' service lookup changes.
- This compiles against skeleton `todo!("audio-01")` and `todo!("audio-06")` bodies (`BufferList::frames`, the HAL
  calls); the unit tests that touch `BufferList` pass only after those merge.

## Midget verification (lead)

1. Build and push: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in `springboard/`,
   then `tools/midget-push-shell.sh --no-build shot.png`; deploy audiod and bifrost-audiod.
2. `bifrost pc midget open-app is.olibuijr.music`, play a track: sound out, log line `audio.speaker: opened`.
3. Lock the screen while playing: the lock sound (once wired) plays over the music.
4. Voice Memos: Record, Stop, play back; `audio.capture: recording ...` in the log.
5. Volume keys: `bifrost pc midget key` volume up and down; log lines `springboard: volume N% -> device x`; the
   device volume property changes on bifrost-audiod.
6. `ssh midget 'lsof /dev/snd/pcm* 2>/dev/null | awk "{print \$1}" | sort -u'` lists only `bifrost-audiod`.

## Reconnect storm on Midget 0.6.2 (found by reading the code; not yet verified on the device)

Symptom: the server journal showed `client N: is.olibuijr.mediaplayer (pid ..., policy false)` then `client N gone`,
322 times a minute. Cause: `shell/services/audio_devices.rs` throttled its connection attempts with constants in
seconds (`RECONNECT_EVERY_S = 10.0`, `PROBE_EVERY_S = 2.0`), but the shell clock `t` that `services_tick` passes is in
milliseconds, so every tick (about 5 per second) probed again, and a probe that found no input device (`discover_with`
returned an empty list) set `self.system = None`, hanging up the connection it had just made. The unit test used
seconds, so it never saw it. Fixes:

- the constants are milliseconds (`PROBE_EVERY_MS`), reconnects back off 1 s, doubling to 30 s (`RECONNECT_MAX_MS`);
- the connection is kept while the server answers: no default input (`UnknownObject`) is not a reason to hang up, only
  other errors drop it;
- `SpeakerOut` (`media/speaker.rs`) backs off the same way (1 s to 30 s) after a failed `Stream::open`, so a player
  that writes in a loop no longer reconnects on every write.

The server closes a client only on EOF or a receive error (`control::serve`); nothing closes it after Hello.

## Bench `--live` cannot connect (Io)

`System::connect` mapped every failure to a bare `AudioError`, and `Io` from the handshake is the 5 s reply timeout
(`Conn::request`): the socket connected (a missing socket is `NoServer`) but no Welcome was decoded in time. Static
reading found no path or protocol mismatch (path `/run/bifrost/audio/control`, `PROTOCOL_VERSION` 2 on both sides,
SEQPACKET on both sides, no peer-credential check on Hello, no memfd before Welcome), so the root cause on Midget is
still unconfirmed. `System::connect_detailed` now returns a `ConnectError` naming the stage (socket, connect, hello),
the path and the OS error or the handshake outcome, and `bifrost-audio-bench` prints it without the usage text and
exits 1 (2 stays for a bad command line). Next step on Midget: run `bifrost-audio-bench audio-roundtrip --live` and read
the new message; "no Welcome within 5 s" with a `client N: bifrost-audio-bench` line in the journal would point at the
reply path (reader thread), without that line at the server's accept loop.

## tools/audio-bench.py failures

A bench that exits non-zero, prints usage text, prints no JSON, times out or reports all-zero statistics is now a
failure: it is listed on stderr, the script exits 1, and `--append` writes nothing (not even the benches that passed)
to `docs/perf-history.jsonl`.
