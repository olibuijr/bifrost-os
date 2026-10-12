# 15: MIDI server (bifrost-midi, bifrost-midid)

Contract: `docs/audio-architecture.md` (Core MIDI row). Task: `tasks/audio/15-midi-server.md`.

## Implemented

`springboard/audio/midi` (`bifrost-midi`, pure, no unsafe):

- `endpoints.rs`: the registry of clients, devices, entities, sources and destinations (absorbed by copy from
  `core/src/midi_endpoints.rs`).
- `events.rs`, `thru.rs`, `ble.rs`: MIDI 1.0 parser/encoder/note tracker, thru connections, BLE MIDI packet
  codec (copied from `ui/src/midi_{events,thru,ble}.rs`; the GATT UUID helper was dropped).
- `ump.rs`: UMP words inside. MIDI 1.0 (type 2) and MIDI 2.0 (type 4) channel voice, 7/16/32-bit min-centre-max
  scaling, 7-bit sysex as type 3 with an assembler, a full byte-stream to UMP converter (running status,
  real-time bytes anywhere, system common, sysex) and UMP back to wire bytes at the edge.
- `clock.rs`: `HostClock` maps the audio host clock (`CLOCK_MONOTONIC_RAW` ns, `TimeStamp.host_ns`) to sample time
  and to a frame offset inside a cycle (late events land on frame 0, later ones wait).
- `hub.rs`: routing. `create_app` gives an app a virtual source and destination; events keep host timestamps and a
  stable order (equal stamps keep arrival order); per-client inbox and per-destination queues with `drain_*`.

`springboard/audio/midid` (`bifrost-midid`, unsafe with SAFETY comments):

- `rawmidi.rs`: `/dev/snd/midiC*D*` through the kernel uapi (open, read, write, `SNDRV_RAWMIDI_IOCTL_INFO`).
- `seq.rs`: `/dev/snd/seq` client, port creation, subscribe, event decode (28-byte `snd_seq_event`).
- `ble_link.rs`: BLE notification to timestamped UMP (13-bit ms unwrap, anchor to host clock, never in the future).
- `main.rs`: daemon loop (hardware rescan every 2 s, poll, stamp with the host clock on arrival, route through the
  hub); `--list` and `--dump [SECONDS]` for checks. Log prefix `is.olibuijr.audio/midi` on stderr.

All struct sizes, offsets and ioctl numbers are pinned by tests against values measured with a C compiler on
`sound/asound.h` and `sound/asequencer.h`. No libasound.

## Done-when mapping

- Host loopback keeps order and timestamps: `hub::tests::loopback_keeps_order_and_timestamps`,
  `sample_accurate_delivery_at_64_frames`.
- Midget keyboard at 64 frames: needs hardware, see below.

## Left (todo_left)

- Client IPC: a unix socket protocol between apps and `bifrost-midid` (client side library in `bifrost-midi`, shm
  event rings to the audio server so a unit gets MIDI in the render cycle) is not written; the hub is the API it
  will sit on. The audio server side (units) consuming `HostClock::frame_in_cycle` is task 05/13.
- BLE link (scan/connect/GATT characteristic) is the Bluetooth service's job; `BleIngress::ingest` is the entry
  point. The device picker model (`ui/src/midi_ble_picker.rs`) was not moved (it depends on the ui GATT and
  advertising types).
- Sequencer output (sending to other seq clients) and rawmidi output from hub destinations; kernel UMP endpoints
  (`/dev/snd/umpC*D*`) and per-event kernel timestamps (`SNDRV_RAWMIDI_MODE_FRAMING_TSTAMP`) are not used yet;
  events are stamped when the daemon reads them, which adds up to one poll wake-up of jitter.
- systemd unit for the `audio` user.

## Midget verification (lead)

1. `bifrost pc` build as usual; on Midget run `bifrost-midid --list`: lists `/dev/snd/midiC*D*` with names.
2. Attach a USB MIDI keyboard; `bifrost-midid --dump 20`, press keys: lines `NS name 2090XXYY` with growing
   nanosecond stamps (host clock), note-on `2090...`, note-off `2080...`.
3. With the sequencer: `aconnect -l` shows client "BifrOSt MIDI"; `aconnect <kbd> <BifrOSt client>` then `--dump`
   shows `seq` lines. (aconnect is only a test tool.)
4. 64-frame DAW check waits for the client IPC and the DAW (tasks 05 and 13).
