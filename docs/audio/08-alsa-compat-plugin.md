# Audio 08: ALSA compatibility plug-in

Crate `bifrost-alsa-plugin` (`springboard/audio/compat/alsa/`), a cdylib named `libasound_module_pcm_bifrost.so`.

## What is implemented

- `ffi.rs`: hand-declared ioplug structs and the alsa-lib symbols (protocol 1.0.2), layouts pinned by tests.
- `ring.rs`: lock-free SPSC frame ring (one ALSA buffer of interleaved f32) and S16, S24 (32-bit container), S32, F32
  sample conversion.
- `plugin.rs`: `_snd_pcm_bifrost_open` and the `__snd_pcm_bifrost_open_dlsym_pcm_001` version symbol; callbacks
  start, stop, pointer, transfer, delay, hw_params, hw_free, prepare, drain, pause, poll_revents, close.
  Playback and capture; MMAP and RW interleaved access (alsa-lib emulates mmap over `transfer`).
- Contract mapping: the plug-in is a HAL client (`System::connect("alsa:<process>")`, default Output or Input device,
  `add_io_proc`, `start`/`stop`). The IO procedure only touches the ring and does a futex wake; a helper thread turns
  that into the poll eventfd. `pointer` is the ring position, `delay` (`snd_pcm_delay`) is the ring fill.
- Offered rate is the device nominal rate (property `nsrt`, 48 kHz fallback); `plug` in the config converts rate,
  channels and format, so no resampler lives here.
- Negotiation (task 26, `plugin::Offer`): ioplug constrains periods and buffers in bytes only, so the plug-in fixes
  the frame size to make the frame ranges exact: the device's channel count (sum of the `sfmt` channels of its
  streams in the direction, 2 when unreported, at most 8) and 4-byte formats only (S24_LE, S32_LE, FLOAT_LE; S16 and
  everything else go through `plug`). PERIOD_BYTES is the list of power-of-two periods from the server quantum
  (`fsiz`, 256 when unreported, rounded up to a power of two in 16..4096) to 4096 frames, so alsa-lib's default
  choice (the smallest period) is the quantum. BUFFER_BYTES is two quanta to 65536 frames, PERIODS 2 to 1024.
  `hw_params` checks the result against the offer and refuses anything else with `-EINVAL` and a log line.
- Drain (task 26): returns at once when the stream is not running (paused, stopped, never started) or the queue is
  empty, `-EAGAIN` in non-blocking mode while frames play, else waits for the queue to play, bounded by the buffer
  length plus 200 ms and by a stall limit (two periods plus 100 ms without the IO procedure consuming a frame).
- Pause (task 26): `pause(1)` stops the IO procedure, `pause(0)` starts it again; the ring is untouched, so the
  position and `pointer` continue where they were. alsa-lib's ioplug owns the RUNNING/PAUSED state. A `running`
  flag in the plug-in state follows start, stop, pause, resume and teardown. No suspend `resume` callback (the
  plug-in never suspends).
- Poll (task 26): one descriptor in both directions, the eventfd, polled for POLLIN. The wake thread signals it each
  cycle the IO procedure frees space or delivers data; `prepare` signals it for playback (an empty ring is ready).
  `poll_revents` reports POLLOUT (playback) or POLLIN (capture) when `avail_min` (sw_params, else one period) frames
  are writable or readable, POLLERR when the descriptor failed, and empties the eventfd only when not ready, so it
  stays level-triggered. Before, playback polled the eventfd for POLLOUT, which an eventfd always reports.
- Packaging: `packaging/arch/alsa/50-bifrost.conf` (`pcm.!default` is `plug` over `pcm.bifrost`) installed to
  `/etc/alsa/conf.d/`, and `PC_ALSA_PLUGIN` (the built cdylib) installed to `/usr/lib/alsa-lib/` by
  `package-files.sh`. The repo has no task 00 `asound.conf` to remove.
- Logging: `is.olibuijr.audio/client` lines on stderr (the plug-in lives in third-party processes).

## Left

- `ctl.!default` points at `type hw card 0` (kernel mixer, no PCM exclusivity); a ctl plug-in that maps volume and mute
  to server properties is not written.
- The lead must set `PC_ALSA_PLUGIN=target/release/libasound_module_pcm_bifrost.so` in the packaging step (PKGBUILD).
- Xruns are counted in `PcmRing::xruns` but not logged yet; no channel maps.
- The server quantum and channel count are read once at open; a later quantum change does not renegotiate (the
  ring decouples the two, so a larger quantum only adds latency).
- The device's `stm#` and `fsiz` properties must be published by the server for the offer to follow the device;
  otherwise the plug-in offers stereo at a 256-frame quantum.
- Task 26 was written without a build (wave rule); the lead builds and runs `cargo test -p bifrost-alsa-plugin`.

## Midget verification (after task 06 and the server are deployed)

1. `bifrost pc midget update --restart`, then `ssh midget 'ls /usr/lib/alsa-lib/libasound_module_pcm_bifrost.so /etc/alsa/conf.d/50-bifrost.conf'`.
2. `ssh midget 'speaker-test -D default -c 2 -t sine -l 1'` while Music plays (`bifrost pc midget open-app ...`).
3. `ssh midget 'arecord -D default -f S16_LE -r 48000 -c 2 -d 2 /tmp/cap.wav && ls -l /tmp/cap.wav'`.
4. Start SoundCraft (`ai.storyteller.soundcraft`) with Music playing; both audible.
5. Start CADCraft and FilmCraft; `bifrost pc log` shows no audio errors.
6. `ssh midget 'LIBASOUND_DEBUG=1 aplay -D default -vv /usr/share/sounds/alsa/Front_Center.wav'` shows delay reporting.
7. Negotiation: `aplay -D bifrost -f FLOAT_LE -c 2 -r 48000 -d 1 -v /dev/zero` on the machine shows a power-of-two
   period equal to the server quantum; with `-f S16_LE` it fails cleanly (alsa-lib refuses during refinement), and
   `aplay -D default -f S16_LE ...` plays through `plug`.
8. Drain: `speaker-test -D default -c 2 -t sine -l 1` ends right after the tone. Pause and resume in a player that
   uses `snd_pcm_pause` (for example `mpv --ao=alsa`, space twice) continue without a jump.
9. Poll: while `aplay -D default` plays, `top -H -p $(pidof aplay)` shows aplay near 0 % CPU (it sleeps in poll
   instead of spinning).
