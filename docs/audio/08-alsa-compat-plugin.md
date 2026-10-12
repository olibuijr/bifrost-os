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
  channels and format, so no resampler lives here. Channels 1 to 8.
- Packaging: `packaging/arch/alsa/50-bifrost.conf` (`pcm.!default` is `plug` over `pcm.bifrost`) installed to
  `/etc/alsa/conf.d/`, and `PC_ALSA_PLUGIN` (the built cdylib) installed to `/usr/lib/alsa-lib/` by
  `package-files.sh`. The repo has no task 00 `asound.conf` to remove.
- Logging: `is.olibuijr.audio/client` lines on stderr (the plug-in lives in third-party processes).

## Left

- `ctl.!default` points at `type hw card 0` (kernel mixer, no PCM exclusivity); a ctl plug-in that maps volume and mute
  to server properties is not written.
- The lead must set `PC_ALSA_PLUGIN=target/release/libasound_module_pcm_bifrost.so` in the packaging step (PKGBUILD).
- Xruns are counted in `PcmRing::xruns` but not logged yet; no channel maps.

## Midget verification (after task 06 and the server are deployed)

1. `bifrost pc midget update --restart`, then `ssh midget 'ls /usr/lib/alsa-lib/libasound_module_pcm_bifrost.so /etc/alsa/conf.d/50-bifrost.conf'`.
2. `ssh midget 'speaker-test -D default -c 2 -t sine -l 1'` while Music plays (`bifrost pc midget open-app ...`).
3. `ssh midget 'arecord -D default -f S16_LE -r 48000 -c 2 -d 2 /tmp/cap.wav && ls -l /tmp/cap.wav'`.
4. Start SoundCraft (`ai.storyteller.soundcraft`) with Music playing; both audible.
5. Start CADCraft and FilmCraft; `bifrost pc log` shows no audio errors.
6. `ssh midget 'LIBASOUND_DEBUG=1 aplay -D default -vv /usr/share/sounds/alsa/Front_Center.wav'` shows delay reporting.
