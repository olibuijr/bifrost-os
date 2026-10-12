# BifrOSt audio architecture (plan)

Owner request 2026-10-12: a low-latency, pure Rust, hardware-level pro audio server. Take the best ideas from
PipeWire (and JACK), and structure the code the way the reference platform structures its audio stack. Like
`docs/graphics-architecture.md`, this is the contract each component is written against. When code and this document
disagree, fix one of them in the same change. The tasks are in `tasks/audio/`.

Reference names (the platform's framework names) appear only in this document. Code, comments, strings and file
names use the neutral names in the right-hand columns (`bifrost pc names`).

## Why: what Midget does today (0.5.44, measured 2026-10-12)

- **No sound server.** Midget runs plain ALSA: no PipeWire, PulseAudio or JACK, and no `/etc/asound.conf`.
  `aplay -L` lists only `default:CARD=PCH` and `sysdefault:CARD=PCH`, with no `dmix`.
- **The first client takes the device for everyone.** With SoundCraft (a store app) open, `pcmC0D0p` is RUNNING,
  owned by SoundCraft's PID and mmapped. Then `speaker-test -D default` and `aplay -D hw:0,0` both fail with
  `Device or resource busy`. The shell's sounds, Music, Podcasts, Voice Memos and the browser cannot play at the same
  time as a pro app, or as each other.
- **Hardware:** card 0 is Intel HDA PCH with an ALC233 codec. It has analog output on device 0, three HDMI/DP outputs
  on devices 3, 7 and 8 (pcm=3 is the G27q-20 over DP; its jack reads `on`), an internal "Speaker Phantom Jack", and
  front headphone, line-out and two mic jacks (all unplugged). Master is at 0 % because the shell's volume is 0. The
  CPU is an i7-6700T with 8 threads. The kernel is 7.2.9-arch1 with `PREEMPT_DYNAMIC`, running `preempt=full`, at
  `HZ=1000`. `CONFIG_UCLAMP_TASK=y`, and `snd-aloop` is available as a module.
- **Our code today:**
  - `springboard/audiod` (`is.olibuijr.audio`) is a media *player* service (queue, tracks, record). It is not a
    device server.
  - Playback goes through `media/alsalib.rs`, which loads the system libasound at run time with `dlopen`, on the
    `default` PCM.
  - Capture uses the kernel's ALSA ioctls directly (`media/alsa.rs`), with the uapi struct layouts pinned by tests.
  - The mixer, jack routing and volume are in `media/mixer.rs`, `route.rs` and `volume.rs`.
  - The kit models in `springboard/ui/src/audio_*.rs` (`audio_hardware`, `hardware_clock`, `process_tap`,
    `audio_engine`, `plugin_host`, `audio_session`, ...) are shapes with tests, not a running engine.
- **Parity matrix:** Core Audio 46 %, Audio Toolbox 50 %, Audio Unit 44 %, AVFAudio 50 %, Core MIDI 38 %,
  CoreAudioKit 42 %. None of these frameworks has a single row at `parity`.

## How the reference platform builds it

### Layers (lowest first)

| Reference | What it is | Notes that matter for us |
|---|---|---|
| Kernel audio family (IOAudioFamily, deprecated) and AudioDriverKit (12+) | Kernel or DriverKit side of a hardware driver: DMA ring buffer, the zero timestamps, sample-rate and format controls | The driver does almost nothing per cycle. It publishes the ring buffer and a timestamp each time the ring wraps |
| AudioServerPlugIn (10.9+) | User-space driver plug-ins loaded by the audio server (`coreaudiod`) | Fixed IO operation sequence per cycle: ReadInput, ConvertInput, ProcessInput, ProcessOutput, MixOutput, ProcessMix, ConvertMix, WriteMix. Plug-ins run sandboxed and may not call the client HAL API |
| `coreaudiod` | The audio server. Owns every device, mixes the clients for each device, and applies routing and the default devices | One server process; mechanism and policy are in one daemon |
| Core Audio HAL (the CoreAudio framework) | Client library and object model: every system, device, stream, control, clock and tap is an `AudioObject` with properties addressed by (selector, scope, element), with get, set and listeners. Clients register an IOProc per device | The IOProc runs on a real-time thread inside the *client*, gets input and output for the same cycle in one call, and is timed from the device's timestamps |
| Audio Workgroups (11+) | `os_workgroup` per device IO thread. Every real-time thread that works on the same deadline joins it | Tells the scheduler which threads share a deadline (core choice, frequency) |
| AudioToolbox | AudioConverter, AudioFile/ExtAudioFile, AudioQueue, system sounds, AUGraph (deprecated) | Services one level up from the HAL |
| Audio Units (v2, v3 `AUAudioUnit`) | Plug-in component model: render block, parameter tree, presets, MIDI in, out-of-process extensions | The render block is real-time code; the host owns the thread |
| AVFAudio (`AVAudioEngine`) | Node graph (players, mixers, effects, the IO node) with manual and offline rendering | The default way an app plays audio |
| Core MIDI and `MIDIServer` | MIDI endpoints, timestamps on the host clock, MIDI 2.0 UMP (`MIDIEventList`) | A separate server process |
| Audio MIDI Setup, CoreAudioKit | Device and aggregate configuration UI; plug-in view hosting | Users build aggregate and multi-output devices here |
| Process taps (14.2+) | Capture another process's or the system's output as a tap object, read through a private aggregate device | Gated by a privacy permission |

### The ideas we take from it

1. **One object model for everything.** System, devices, streams, controls, clocks, boxes and taps are all objects
   with typed properties, get and set, and listeners, addressed as (selector, scope, element). Every tool, the
   Settings page, the MCP tool and the SDK use the same surface. This replaces ad hoc RPCs per feature.
2. **Timestamps are the contract.**
   - The device publishes a *zero timestamp* on each ring wrap: sample time, host time and a rate scalar.
   - Every client computes where the hardware is and when to wake from those timestamps.
   - Sample-accurate scheduling (MIDI, automation, A/V sync) maps between host time and sample time through the same
     clock.
3. **Safety offset and latency are properties, not guesses.** Each device and stream reports its latency and safety
   offset, and the server adds its own buffer, so a DAW can compensate exactly.
4. **Full-duplex IOProc.** Input and output for the same cycle arrive in one callback, which is what a DAW needs for
   monitoring through effects.
5. **Per-client buffer size.** Each client can ask for its own IO buffer size within the device's range. The server
   runs the device at the smallest size any client asked for, and calls the other clients at their own rate.
6. **Hog mode.** A pro app can take a device exclusively. Everyone else is moved to another device or silenced, and
   the device is released when the app quits. This is the explicit version of what SoundCraft does to Midget by
   accident today.
7. **Aggregate devices with drift compensation.** Several devices become one. Each sub-device has its own clock
   domain, and the ones that are not the clock source are resampled to it.
8. **Workgroups.** Real-time threads that share a deadline are grouped, and the kernel is told about the group.
9. **Drivers are plug-ins of the server.** The server stays small and the hardware details sit behind one interface.
10. **SDK layering.** Low level (HAL), then services (Toolbox), then components (Units), then the high-level engine
    (AVFAudio). Each layer uses only the one below it.
11. **Privacy.** Microphone and tap access is a permission, and a menu-bar indicator shows while any input is open.

## The ideas we take from PipeWire (and JACK)

1. **Wait-free cross-process graph scheduling.**
   - Each node has an *activation record* in shared memory (memfd): status, the `pending` and `required` counters,
     and its timing.
   - The driver starts a cycle by resetting the counters and signalling the nodes that have no inputs.
   - A finished node decrements each target's `pending` counter, and signals any target whose counter reaches zero.
   - There are no locks and no server round trip per node, so clients run in their own processes at server
     latency.
2. **Driver and follower.** One node per graph drives it, from a timer or from hardware. The others follow. The
   graph's *quantum* (frames per cycle) is the smallest one any follower asks for. The rate and quantum can change
   per graph.
3. **ALSA timer scheduling** (also PulseAudio's tsched):
   - Period interrupts are off (`SNDRV_PCM_HW_PARAMS_NO_PERIOD_WAKEUP`), and a buffer as large as the hardware allows
     is used.
   - The driver wakes from a timer, and a delay-locked loop (DLL) tracks the hardware rate against the host clock.
   - The quantum can then drop to any value, independent of the period sizes the hardware supports.
4. **Adaptive resampling for followers** in other clock domains, with the DLL providing the rate. This is the same
   job as aggregate drift compensation.
5. **Mechanism and policy are separate.** The daemon moves samples, and a session manager decides links, defaults
   and permissions. (WirePlumber is that manager for PipeWire.)
6. **Profiler data in the activation record.** Signal, awake and finish times per node per cycle, read live by a
   `top`-style tool.
7. **Speak the existing protocols.** A PulseAudio protocol server, an ALSA plug-in and a JACK library make unmodified
   Linux apps work.

What we do *not* take from it:
- **The generic SPA plug-in and POD type system.** We use plain Rust traits and fixed-layout `#[repr(C)]` shared
  structs.
- **Lua and conf-file policy.** Policy is Rust code with typed settings.
- **One graph for video and audio.** Video belongs to the window server and graphics stack.

## The architecture

### Process model

```
bifrost-audiod (system user `audio`, SCHED_FIFO IO threads, CAP_SYS_NICE)
  |  owns /dev/snd/* ; drivers: alsa (pure Rust ioctls), null, loopback, (later) bluetooth, network
  |  object model + graph + mixing + taps + aggregates
  |  control socket: /run/bifrost/audio/control (SOCK_SEQPACKET, fd passing: memfd, eventfd)
  +-- bifrost-audio-policy   (session manager: defaults, routing, ducking, hog grants, permissions, jack events)
  +-- bifrost-midid          (MIDI server: ALSA rawmidi/seq + BLE MIDI, timestamps on the audio host clock)
  +-- SDK apps               (bifrost_app::audio_* ; IOProc on an RT thread in the app, shared-memory ring)
  +-- pulse compat server    (bifrost-audio-pulse: PulseAudio native protocol on $XDG_RUNTIME_DIR/pulse/native)
  +-- ALSA compat plug-in    (libasound_module_pcm_bifrost.so: "default" for ALSA apps such as SoundCraft)
  +-- media player service   (today's springboard/audiod, renamed is.olibuijr.mediaplayer: an ordinary client)
```

The shell, Settings (Sound), Control Center volume, the menu-bar input indicator and the MCP `audio` tool all talk
to the daemon through the object model. None of them opens a PCM.

### Crates (new workspace members under `springboard/audio/`)

| Reference | BifrOSt crate (path) | Role |
|---|---|---|
| CoreAudioTypes | `bifrost-audio-types` (`audio/types`) | `StreamFormat` (ASBD), `TimeStamp` (sample time, host time, rate scalar, flags), `BufferList`, `ChannelLayout`, the property address (`Selector`, `Scope`, `Element`), error codes. `no_std` + `alloc`, no `unsafe`. Absorbs `ui/src/channel_layout.rs` and `hardware_clock.rs` |
| Shared memory ABI | `bifrost-audio-shm` (`audio/shm`) | `#[repr(C)]` activation record, ring buffer header, zero timestamp seqlock, the futex and eventfd wake helpers. Layout pinned by tests. The only `unsafe` outside the drivers |
| AudioServerPlugIn interface | `bifrost-audio-driver` (`audio/driver`) | `trait Driver` and `trait DeviceIo` (see the contracts below), plus `NullDriver` and `LoopbackDriver` |
| Kernel driver glue | `bifrost-audio-alsa` (`audio/alsa`) | Pure Rust ALSA through the kernel uapi: PCM hw and sw params, mmap of the data, status and control pages, the DLL, timerfd scheduling, ctl elements and jack events, and UCM-free HDA routing. Grows from `media/alsa.rs` and `media/mixer.rs` |
| `coreaudiod` | `bifrost-audiod` (`audio/server`) | The object tree, IO cycle, mixing, clients, aggregates, taps, hog mode, latency accounting, profiler |
| Core Audio HAL client | `bifrost-audio-hal` (`audio/hal`) | Client library: `AudioObject` get, set and listen; `Device::add_io_proc`; RT thread creation; the workgroup join; the client side of the ring and activation |
| Audio Workgroups | `bifrost-audio-hal::workgroup` | Join the device's workgroup. On Linux: SCHED_FIFO (or SCHED_DEADLINE, runtime and period from the cycle), `sched_util_min` (uclamp) for the frequency hint, and CPU affinity hints |
| AudioToolbox | `bifrost-audio-toolbox` (`audio/toolbox`) | Converter (format, channel map, a high-quality resampler for files, the adaptive resampler for drift), files (moved from `media/audiofile.rs` and the codecs), queue, system sounds |
| Audio Units | `bifrost-audio-units` (`audio/units`) | Component trait (render block, parameter tree, presets, MIDI in, latency and tail), in-process and out-of-process hosting through the same activation records. Absorbs `ui/src/plugin_host.rs` and `audio_render.rs` |
| AVFAudio | `bifrost-audio-engine` (`audio/engine`), re-exported by the SDK | Node graph (player, mixer, effect, IO node, tap), offline and manual rendering. Absorbs `ui/src/audio_engine.rs` and `audio_mix.rs` |
| Core MIDI and `MIDIServer` | `bifrost-midi` and `bifrost-midid` (`audio/midi`, `audio/midid`) | Endpoints, UMP (MIDI 2.0) and MIDI 1.0, timestamps on the host clock, a virtual port per app. Absorbs `core/src/midi_endpoints.rs`, `ui/src/midi_*.rs` |
| Session policy (AVAudioSession, defaults) | `bifrost-audio-policy` (`audio/policy`) | Defaults per role (output, input, sound effects), the routing rules on jack events, ducking, hog grants, the microphone permission and indicator. Absorbs `ui/src/audio_session.rs`, `default_device.rs`, `media/route.rs` |
| Process taps | object class `Tap` in `bifrost-audiod` | A per-process or system mix tap, behind the permission |
| Compatibility | `bifrost-audio-pulse` (`audio/compat/pulse`), `bifrost-alsa-plugin` (`audio/compat/alsa`, a cdylib for alsa-lib's ioplug), later `bifrost-jack` (`audio/compat/jack`, a libjack ABI) | Unmodified Linux apps |
| Audio MIDI Setup | app `audio-setup-desktop` | Devices, formats, aggregate and multi-output devices, MIDI studio |
| CoreAudioKit | SDK `bifrost_app::audio_ui` | Plug-in view hosting, the device picker |

Dependencies point down only: types, then shm, then driver and alsa, then server. Separately, types, then shm, then
hal, then toolbox and units, then engine, then the SDK. The server never depends on the client crates.

## Contracts

### Object model

- `ObjectId(u32)`. `Class`: System, Plugin, Device, Stream, Control (Volume, Mute, DataSource, Selector), Box, Clock,
  Tap, AggregateDevice.
- `PropertyAddress { selector: FourCC, scope: Global|Input|Output|PlayThrough, element: u32 }` (element 0 = main).
- `get`, `set`, `listen` and `unlisten` over the control socket. Values are typed in `bifrost-audio-types`
  (`PropertyValue`). Listeners get coalesced change notices, then re-read the value.
- Required device properties (the reference names for the mapping are in `tasks/audio/02-object-model.md`):
  - name, manufacturer, UID and model UID, transport type, is-alive and is-running
  - nominal sample rate and available rates
  - streams per scope, each with a virtual format (f32 non-interleaved) and a physical format
  - buffer size and its range
  - latency, safety offset, and zero timestamp period
  - clock domain
  - hog PID
  - preferred stereo channels and channel layout
  - jack connected, data sources, and volume and mute controls
  - the IO thread workgroup

### Clock and timestamps

- Host time is `CLOCK_MONOTONIC_RAW` in nanoseconds, the same clock the window server uses for presentation
  (`docs/graphics-architecture.md`, frame clock). A/V sync maps both through it.
- Each device publishes a zero timestamp `{ sample_time, host_time, rate_scalar, seed }` in a seqlock in shared
  memory:
  - after every `zero_timestamp_period` frames (the ALSA buffer size), and
  - on every DLL update.
  Clients read it without a syscall.
- The DLL: second order, with a bandwidth that adapts from wide at start to narrow (JACK and PipeWire use the same
  idea). Its input is `(hw_ptr, tstamp)` read from the mmapped ALSA status page with `SNDRV_PCM_TSTAMP_TYPE_MONOTONIC_RAW`.

### IO cycle (server, per device or aggregate)

1. Sleep on a timerfd until `next_wake = predicted host time of (cycle start - safety margin)`.
2. Read the hardware position (status page). Update the DLL and the zero timestamp. Detect an xrun.
3. Reset the activation counters of every node in this graph. Signal the nodes that have no pending inputs: client
   IOProcs, taps, and processing nodes.
4. Each client's IO thread wakes on its futex, reads input for this cycle from its input ring, runs its IOProc, writes
   output, and decrements its targets' counters. The last one signals the driver.
5. The driver wakes, then mixes, applies each stream's volume and the device volume, converts to the physical format,
   and writes into the mmapped DMA buffer at `appl_ptr`. It commits `appl_ptr` through the control page. Then it
   records the profile.
6. A client that missed the deadline is not waited for. Its output for this cycle counts as silence, and it is told
   (an overload notice, the same as the reference's `kAudioDeviceProcessorOverload`). The device never stalls on a
   client.

Order within a cycle: ReadInput, ConvertInput, ProcessInput (taps, input processing), ClientIO, MixOutput,
ProcessMix, ConvertMix, WriteMix. These are the same steps and the same names as the reference plug-in's IO
operations, so drivers can hook the same points.

### Client IO

- On `add_io_proc`, the client gets a memfd with its rings (input and output, f32, its own buffer size), an
  activation record, and a futex word, which is an eventfd for clients that use epoll.
- The IOProc signature is `fn(&IoContext, now: &TimeStamp, input: &BufferList, input_time: &TimeStamp, output: &mut
  BufferList, output_time: &TimeStamp)`. It is the reference's IOProc, made safe.
- Real-time rules are enforced in debug builds. In the IO thread, a guard allocator panics on allocation, locking
  or a syscall other than the futex. The SDK documents the rules next to the trait.
- A client with a larger buffer than the device quantum is called once every N cycles, and the server accumulates
  for it. This is the reference's per-client buffer size.

### Driver interface (`bifrost-audio-driver`)

```rust
pub trait Driver: Send {
    fn name(&self) -> &str;
    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>);          // hot-plug through `poll_events`
    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError>;
    fn poll_events(&mut self, out: &mut Vec<DriverEvent>);       // jack, hot-plug, rate change, control change
    fn controls(&mut self, uid: &str) -> Vec<ControlDesc>;
    fn set_control(&mut self, uid: &str, id: ControlId, v: ControlValue) -> Result<(), DriverError>;
}
pub trait DeviceIo: Send {                                         // called only on the IO thread
    fn start(&mut self) -> Result<ZeroTimeStamp, DriverError>;
    fn position(&mut self) -> HwPosition;                          // hw_ptr, tstamp, state; no syscall on the alsa driver
    fn read_input(&mut self, frames: u32, dst: &mut BufferList) -> Result<(), Xrun>;
    fn write_mix(&mut self, frames: u32, src: &BufferList) -> Result<(), Xrun>;
    fn stop(&mut self);
    fn latency(&self) -> Latency;                                  // device + safety offset, per scope
}
```

ALSA driver specifics (pure Rust, no libasound):
- `SNDRV_PCM_IOCTL_HW_PARAMS` with the format refined to S32 or S24 at the native rate, the largest buffer,
  `NO_PERIOD_WAKEUP` when the device supports it, and MMAP_INTERLEAVED or MMAP_NONINTERLEAVED access.
- mmap the status and control pages. Fall back to `SYNC_PTR` when they cannot be mapped (some kernels or
  architectures).
- Full duplex on one card: link capture and playback with `SNDRV_PCM_IOCTL_LINK` so they start in the same cycle.
- Jack detection: subscribe to ctl events (`SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS`) and read the `* Jack` elements
  (`Front Headphone Jack`, `Line Out Jack`, `HDMI/DP,pcm=N Jack`).
- HDMI/DP: one device per connected pcm (ELD for the name and the supported formats).
- Routing without UCM: the HDA codec's kernel auto-mute, plus a small table per codec family. The table starts with
  ALC233 on Midget. UCM is read later if needed.

### Real-time threads (the Linux version of workgroups)

- The server IO threads use SCHED_FIFO at priority 88. The daemon has CAP_SYS_NICE and memory locked with
  `mlockall(MCL_CURRENT|MCL_FUTURE)`. The IO paths never page fault, because the buffers are pre-faulted.
- Client IO threads: the server grants priority through a `rtkit`-style call on the control socket, and sets
  SCHED_FIFO 80 on the client's TID with a pidfd check. Clients never need CAP_SYS_NICE. SCHED_DEADLINE (runtime =
  measured p99 work, period = the cycle) is evaluated in phase 6.
- Frequency hint: set `sched_util_min` (uclamp) on the workgroup's threads while the device runs, so the CPU does not
  start the cycle at a low clock.
- A watchdog (RLIMIT_RTTIME, plus the server's own check) demotes a thread that spins.

### Formats

The internal format is f32, non-interleaved, at the device's nominal rate. Conversion (integer formats, channel
maps, and resampling for clients at another rate) happens only at the edges: the client ring and the driver. The
adaptive resampler (for aggregates and drift) is a separate, cheap, low-latency kind. A high-quality resampler is
for files and offline work only.

### Latency targets (Midget, ALC233, 48 kHz)

| Mode | Quantum | Expected output latency | Use |
|---|---|---|---|
| Pro (hog or low-latency session) | 64 frames (1.33 ms) | ≤ 4 ms total (quantum + safety + DMA) | DAW monitoring |
| Default | 256 frames (5.3 ms) | ≤ 12 ms | Desktop, music, browser |
| Power save (only idle and background clients) | 1024 frames (21.3 ms) | ≤ 45 ms | Battery and idle |

The graph quantum is the smallest one any running client asked for. At idle (no running streams) the device stops
and the server sleeps: no timers and no wakeups (`docs/snow-leopard.md` rules).

### Hog mode, aggregates and taps

- Hog: `set(device, HogMode, pid)`. Granted by policy (the requesting app needs the `audio-exclusive` permission, or
  the user confirms). Other clients move to the next default device, or keep running with silent output. The hog is
  released on exit (pidfd).
- Aggregate: an ordered list of sub-device UIDs, one clock source, and per-sub-device drift compensation. Created by
  the Audio Setup app or the API. Private aggregates belong to one client.
- Tap: a description (process list or global, mono, stereo or matching, muted or not). It is readable as an input
  stream of a private aggregate, as on the reference. It needs the `audio-capture` permission. The menu-bar
  indicator shows while it runs.

### Policy (`bifrost-audio-policy`)

A separate process, a client of the daemon with policy rights. It owns:
- the default output, input and sound-effects devices, persisted in prefs
- the jack and hot-plug rules (headphones in: switch and pause the media player, as today in `media/route.rs`)
- ducking for notifications and speech
- the microphone permission (TCC parity) and the indicator
- hog grants
- per-app volume

The daemon keeps working with the policy process gone, using the last defaults.

### Compatibility

- **ALSA plug-in.** `libasound_module_pcm_bifrost.so` is a Rust cdylib on alsa-lib's ioplug. `/etc/alsa/conf.d/50-bifrost.conf`
  makes `pcm.!default` and `ctl.!default` point at it. That covers cpal apps such as SoundCraft and the ArtCraft apps.
  It is the only part that touches libasound, and only inside the apps that already load it.
- **PulseAudio native protocol server.** Protocol version 35 and later; enough for Firefox, SDL, mpv and GStreamer
  `pulsesink`.
- **JACK.** A libjack ABI replacement, for the pro Linux apps (Ardour, Carla). The graph model maps one to one.
- **Our own `media/alsalib.rs`** goes away. The shell and the media player become HAL clients.

### Observability and tools

- Unified Logging subsystem `is.olibuijr.audio`, with categories `io`, `driver`, `client`, `policy`, `xrun`.
- `bifrost pc audio top` reads the profiler records: per node, the signal, awake and finish times, the xruns, the
  quantum and the DLL rate. `bifrost pc audio ls|get|set|listen` cover the object model.
- An MCP `audio` tool with the same surface (`docs/bifrost-mcp.md`).
- Every device counts xruns, and so does every client. `bench.py --midget` records idle wakeups and the p99 cycle
  time.

## Testing (no speakers on Midget)

- **Host:** `NullDriver`, with a manual clock, runs the whole server deterministically in `cargo test`. That covers
  graph order, missed deadlines, quantum changes, hog, aggregates with a simulated drift of ±200 ppm, and taps.
- **Loopback:** `snd-aloop` on Midget (and the host). Play a known signal (an MLS or a click) out of
  `Loopback,0` and capture it from `Loopback,1`. That gives the round-trip latency in frames, bit exactness, and
  glitch detection, all without speakers.
- **Real hardware without speakers:** the ALC233 analog output runs into nothing, but the DMA pointer still runs.
  On 2026-10-12, hw_ptr advanced 9 140 760 frames in 190.4 s, exactly 48 000 Hz. The tests check that the hw_ptr
  rate matches the DLL estimate, that no xruns occur at 64 and 256 frames under `stress-ng --cpu 8`, and that jack
  events fire. HDMI/DP audio to the G27q-20 is a second real sink.
- **Acceptance:** two clients play at the same time; SoundCraft and Music play together through the ALSA plug-in;
  a DAW IOProc at 64 frames has no xruns for 10 minutes at idle and 1 minute under load; zero idle wakeups with no
  streams.

## Phases

Each phase ends with the Midget acceptance in its task file, updated parity rows, and a release (the owner's rules).

| Phase | Outcome | Tasks |
|---|---|---|
| 0 | Fix today's exclusivity: ship a `dmix`/`dsnoop` `asound.conf` for the HDA analog device as a stopgap, so the store apps and the shell share the device | `00-stopgap-dmix.md` |
| 1 | Types and shared memory ABI; pure Rust ALSA driver with the DLL and timer scheduling; the server with a null and an ALSA device, mixing two clients | `01` to `05` |
| 2 | HAL client library, IOProcs and RT threads; the shell, the media player and the SDK move off libasound; the ALSA plug-in for third-party apps | `06` to `08` |
| 3 | Policy process: defaults, jack routing, permissions, indicator, per-app volume; Settings > Sound and Control Center on the object model | `09` |
| 4 | Pro: hog mode, 64-frame quantum, full duplex, latency reporting, aggregates with drift compensation, the Audio Setup app | `10` to `12` |
| 5 | Toolbox, Units, Engine in the SDK (AVAudioEngine parity); the DAW and Voice Memos on them; MIDI server | `13` to `15` |
| 6 | Pulse protocol server, taps, JACK, SCHED_DEADLINE evaluation | `16` to `18` |

## Open decisions (owner)

1. Run the daemon as its own `audio` system user (recommended), or inside the root shell's service set.
2. Keep libasound on the system for the ALSA plug-in (recommended until JACK and Pulse cover the apps), or remove it.
3. Is SCHED_DEADLINE in scope for phase 6, or is SCHED_FIFO plus uclamp enough?
4. Does the phase 0 stopgap ship now, before the server exists? It is one config file and fixes the exclusivity bug
   today.

## Sources

- Apple, What Is Core Audio (Core Audio Overview):
  https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/WhatIsCoreAudio/WhatIsCoreAudio.html
- AudioServerPlugIn.h (IO operations, plug-in host rules):
  https://jenkins.heirloomcomputing.com/downloads/MacOSX10.8.sdk/System/Library/Frameworks/CoreAudio.framework/Headers/AudioServerPlugIn.h
- CoreAudio API changes in 10.9 (AudioServerPlugIn):
  https://developer.apple.com/library/archive/releasenotes/General/APIDiffsMacOSX10_9/CoreAudio.html
- WWDC21 10190, Create audio drivers with DriverKit: https://developer.apple.com/videos/play/wwdc2021/10190/
- WWDC 2002 507, the HAL object model: https://nonstrict.eu/wwdcindex/wwdc2002/507/
- Process taps (14.2+): https://github.com/insidegui/AudioCap, https://www.recall.ai/blog/core-audio-taps
- PipeWire scheduling (activation records, driver and followers, quantum):
  https://docs.pipewire.org/page_scheduling.html
- Midget measurements: this session, 2026-10-12 (`/proc/asound/card0/pcm0p/sub0/status`, `fuser /dev/snd/*`,
  `amixer -c0 contents`, `/proc/config.gz`).
