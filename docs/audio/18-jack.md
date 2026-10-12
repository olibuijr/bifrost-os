# Audio task 18: the JACK ABI (`bifrost-jack`)

Crate `springboard/audio/compat/jack` builds `libjack.so` (cdylib, lib name `jack`) and an rlib. SCHED_DEADLINE is out
of scope (owner decision, `docs/audio-architecture.md`, Decisions).

## What is implemented

| File | Content |
|---|---|
| `engine.rs` | The process-local graph: clients (unique names `-01`, exact-name option), ports (audio and MIDI, input/output, physical, terminal), connections (type and direction rules, `EEXIST`), dependency-ordered client execution (Kahn, cycles tolerated), the cycle (capture mix, process callbacks, playback mix), latency ranges and totals, transport (stopped/rolling/starting, slow-sync clients with timeout, locate, timebase master with conditional takeover), deferred notifications (graph order, port registration/connect/rename, client registration, buffer size, xrun, shutdown, freewheel, latency). `Position` is `jack_position_t` (packed, 136 bytes, offsets pinned against the C compiler). |
| `driver.rs` | `Runtime`: one engine per process, one HAL IO procedure on the default output device (2 capture + 2 playback channels, buffer from the device, chunked to 8192 frames), the `jack-notify` thread that delivers buffer-size, xrun and kicked-client notifications unlocked, `BIFROST_JACK_DUMMY=1` wall-clock driver for server-less ABI tests. |
| `ffi.rs` | The C ABI: client open/close/activate, callbacks, ports, connections, `jack_get_ports` (regex), latency, time (`jack_get_time`, frame/time conversion, cycle times), transport (query, locate, reposition, start, stop, sync, timebase), `jack_midi_*`, version, `jack_free`. `jack_client_open` is declared with its three fixed parameters (variadic server name ignored, System V ABI safe). |
| `midi.rs` | MIDI port buffer layout (ours, only reached through `jack_midi_*`), time-ordered merge of several sources. |
| `ringbuffer.rs` | `jack_ringbuffer_*` with the header's struct layout. |
| `pattern.rs` | Small regex matcher for port name and type patterns. |

Mapping to the contract: the graph model maps one to one (client = node, port = pad, connection = link); the device side
is the HAL (`bifrost_audio_hal::Device::add_io_proc`), so the server promotes the real-time thread (SCHED_FIFO plus
uclamp and affinity) and the library never touches scheduling itself. Real-time rule: `Engine::cycle` allocates nothing
and the IO procedure only `try_lock`s the engine; a cycle that loses the race outputs silence and is counted as an xrun.
No libasound anywhere. Logging goes to stderr with the `[is.olibuijr.audio:<category>]` prefix (the crate cannot reach
`core::log`).

`unsafe` is confined to `ffi.rs`, `ringbuffer.rs` and the raw port buffers in `engine.rs`, each with a SAFETY comment.

## Tests (host, no hardware)

`cargo test -p bifrost-jack` (the lead runs it): engine tests with hand-built buffer lists (mixing, sources summed,
offset slices, order, kick on failing process, cycle times, transport rolling/locate/stop, slow-sync start and timeout,
timebase BBT 120 bpm at 48 kHz, close, rename, latency), MIDI buffer and merge, ring buffer wrap, regex, the
`Position` layout, and an end-to-end FFI test with the dummy driver (open, duplicate name, ports, connect callback,
`jack_get_ports`, process callback computing `0.5 * in + 0.25`, transport, time round trip, runtime torn down on the
last close).

## What is left (todo_left)

- Clients in other processes: the engine is process-local. Cross-process JACK clients need a graph-client protocol in
  `bifrost-audio-types::control` and server support (shared port buffers and futex triggering, like the PulseAudio
  taps). Today every JACK application is its own graph with its own HAL client, which is enough for a single app such
  as Ardour or Carla but not for patching two apps together.
- `jack_set_process_thread`, `jack_cycle_wait/signal`, internal clients, session and metadata calls, uuid calls.
- Real freewheel mode (the flag and callbacks exist; the engine still follows the device clock).
- Physical MIDI ports (midid) and the device's real channel count (fixed at 2 in, 2 out; `PHYS_CHANNELS`).
- Buffer-size changes depend on the device property `BUFFER_FRAMES` being settable by this client.
- Packaging: install as `libjack.so.0` (soname) with a `jack.pc`; not done here.

## Midget verification steps (for the lead)

1. Build and push: `cd springboard && cargo build --profile fast -p bifrost-jack --examples` and copy
   `target/fast/libjack.so` to `/usr/lib/libjack.so.0` on Midget (`scp`).
2. ABI check without the server: `BIFROST_JACK_DUMMY=1 ./examples/simple_client` (exit 0, no crash) and
   `nm -D /usr/lib/libjack.so.0 | grep -c ' T jack_'` (about 100 symbols).
3. Build the C client against our library: `cc simple_client.c -o simple_client -ljack -lm`, then with
   `bifrost-audiod` running: `./simple_client` and listen on the device, or capture on the loopback:
   `Loopback,0` as default output and `arecord -D hw:Loopback,1 -f FLOAT_LE -r 48000 -c 2 -d 3 /tmp/j.wav`, then check
   a 440 Hz tone (`sox /tmp/j.wav -n stat` shows a non-zero RMS near 0.14).
4. Load: run `stress-ng --cpu 8` and `bifrost pc audio top`; the `jack-notify` log prints
   `[is.olibuijr.audio:...]` lines only on failures; xruns stay 0 at 64 and 256 frames.
5. A real application: `ardour` or `carla` starts, lists `system:playback_*` and plays a clip.
