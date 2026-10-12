# Audio task 16: PulseAudio native protocol server

Crate `bifrost-audio-pulse` (`springboard/audio/compat/pulse/`), plus the user units
`packaging/arch/bifrost-audio-pulse.{socket,service}`.

## What it does

A process of its own (not inside `bifrost-audiod`) that speaks protocol 35 (accepts clients from 13 up) on
`$XDG_RUNTIME_DIR/pulse/native`, so Servo, `paplay`, `mpv --ao=pulse`, SDL and GStreamer `pulsesink` play without
changes. Every playback or record stream is a HAL client: one IO procedure on the stream's device, with its own
cycle (the stream's `minreq` or `fragsize`, converted to the device rate, a power of two in 64 to 4096 frames).
The audio server mixes the streams, so they play together with SoundCraft.

| File | Role |
|---|---|
| `tagstruct.rs` | Tagged value codec (u32, strings, sample spec, channel map, cvolume, proplist, format info ...) |
| `proto.rs` | Frame descriptor and reassembly, command and error numbers, sample formats (U8, S16, S24, S24 in 32, S32, F32, both endians where defined), cubic volume curve |
| `ring.rs` | Lock-free SPSC ring (atomics only, no unsafe) and a linear resampler |
| `backend.rs` | `Backend` trait; `HalBackend` (bifrost-audio-hal); real-time `PlaybackProc` and `CaptureProc` (no allocation, locks or syscalls); `MockBackend` for tests |
| `session.rs` | Per-client state machine: commands in, frames out, no socket IO |
| `server.rs` | poll loop, socket binding, systemd socket activation (fd 3) |

Commands: auth, client name and proplists, server info, stat, sink and source info (single, list, lookup),
client info, sink input and source output info, subscribe with events (new, change, remove for sinks, sources,
sink inputs, source outputs, clients, server), sink, source and stream volume and mute, default sink and source,
create, delete, cork, flush, trigger, prebuf and drain of playback streams, create, delete, cork, flush of record
streams, playback and record latency, stream name and proplist updates, buffer attribute changes (reply with the
resolved values), sample rate updates, kill. Unknown commands get `NOTIMPLEMENTED`.

Flow control follows the reference server: `REQUEST` keeps `tlength` bytes in flight, `prebuf` holds playback until
filled, an underrun sends `UNDERFLOW` once and rebuffers, `drain` replies when the ring is empty.

Real-time contract: the IO procedure only touches the ring and `StreamCtl` atomics. The protocol thread polls at 5 ms
while a stream exists; the IO procedure never wakes it (no syscall other than the futex is allowed there).

## Contract mapping

- Task 16 "Work": protocol 35, auth, client and server info, sinks and sources, sink inputs, playback and record
  streams, volume, subscribe, each stream a HAL client with its own buffer size: done in session.rs and backend.rs.
- Architecture "Compatibility": speaks the existing protocol; libasound is not used.
- Logging: `is.olibuijr.audio`, category `client`, through `eprintln!` with the prefix (the logger crate is not a
  dependency of this crate).
- Dependencies: only std, libc and the skeleton's audio crates. Unsafe: `hostname` (gethostname), `poll`, `getuid`,
  and the inherited socket fd, each with a SAFETY comment.

## Contract changes

None.

## Left (todo_left)

- Cross-client views: `pactl list` shows only the asking client's own sink inputs, source outputs and client.
- Authentication is by the socket's 0600 mode and 0700 directory; the cookie is ignored. No SCM_CREDENTIALS check.
- No shared memory transport (memfd/shm): the server never offers it, clients fall back to the socket.
- No sinks' ports, no monitor sources (taps are task 17), no card info, no module or sample cache commands, no
  ALAW/ULAW formats, no passthrough, no stream moves.
- `HalBackend` reads `STREAMS`, `DEVICE_UID`, `NAME`, `NOMINAL_RATE`, `LATENCY` and writes `VOLUME_SCALAR`/`MUTE`;
  channel count is fixed at 2 until the layout property is used. The HAL bodies are `todo!("audio-06")` in this
  branch, so only the host tests run until the lead merges.
- Per-app volume policy (the policy process) is not consulted; stream volume is applied in the stream's own IO
  procedure.
- Sample rate conversion is linear interpolation; a better filter can replace `Resampler` without API change.

## Verification

Host (the lead runs, from `springboard/`):

```
cargo test --offline -p bifrost-audio-pulse
```

Covers the codec, framing, sample conversion, ring, resampler, the real-time procs, every session behaviour listed
above on the mock backend, and a real Unix socket round trip.

Midget (after `bifrost pc publish`, `bifrost pc midget update --restart`, audiod running, units installed):

```
ssh midget systemctl --user enable --now bifrost-audio-pulse.socket
ssh midget 'ls -l $XDG_RUNTIME_DIR/pulse/native'
ssh midget 'pactl info'                                  # Server Name: bifrost-audio-pulse, protocol 35
ssh midget 'pactl list short sinks'
ssh midget 'paplay /usr/share/sounds/freedesktop/stereo/complete.oga'   # or any wav
ssh midget 'mpv --ao=pulse --no-video /path/to/music.flac &'
bifrost pc midget open-app <soundcraft id>               # SoundCraft plays at the same time
bifrost pc midget open-app <browser id>                  # play a page with audio in the Browser (Servo)
ssh midget 'pactl set-sink-input-volume <index> 50%; pactl set-sink-mute @DEFAULT_SINK@ toggle'
ssh midget 'parecord --channels=2 --rate=48000 /tmp/rec.wav'            # Ctrl-C after 3 s, then aplay-free check with ffprobe
bifrost pc audio top                                     # one client per stream, no xruns
journalctl --user -u bifrost-audio-pulse -n 50
```

Done when all three (Browser, `paplay`, `mpv`) sound together with SoundCraft and `bifrost pc audio top` shows no
xruns. The package must install the two unit files to `/usr/lib/systemd/user/` and the binary to `/usr/bin/`
(`packaging/arch/package-files.sh`, not touched here).
