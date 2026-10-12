# 27: MIDI system exclusive and the midid kernel sides

Follow-up to `15-midi-server.md`. Owner files: `springboard/audio/midi/`, `springboard/audio/midid/`. Contract:
`docs/audio-architecture.md` (Core MIDI row); no contract change.

## What changed

### `bifrost-midi` `ump.rs`: system exclusive bytes with the high bit set

The MIDI 1.0 rule now holds in both directions. Inside system exclusive a byte with the high bit set is either a
real-time byte (F8 to FF), which is its own message and does not end the system exclusive, or any other status,
which ends it as truncated (what arrived before it is delivered; the rest is not part of the message).

- Bytes to UMP: `sysex_to_ump(group, payload)` applies the rule (it used to pack bytes above 7F as they were). A
  real-time byte becomes a type 1 packet at that point. F7 ends the message. Any other status truncates it. Every
  packed byte is masked to 7 bits.
- `Midi1ToUmp` packs system exclusive as it arrives (`SysexPacker`: a start or continue packet leaves with the
  seventh byte and only six bytes are held). A device that never sends F7 used to grow a `Vec` without limit; now
  it costs one packet. Real-time bytes in a long message land between its packets, where they arrived.
- UMP to bytes (playback): new `UmpToMidi1` writes the packets of one group as a wire stream (F0 ... F7 written as
  the packets come). A real-time byte inside a type 3 packet is written where it stands, which is legal on the
  wire. Any other status closes the message with F7 and drops the rest. A channel or system common packet inside
  an open message closes it first. A new start closes the open one. Orphaned continue and end packets are dropped.
- `ump_to_bytes` refuses packets whose status would corrupt the wire: a type 2 packet without a channel voice
  status, or a type 1 packet that is not F1, F2, F3, F6 or F8 to FF.
- `SysexAssembler` keeps payloads 7-bit. Real-time bytes found in packets go to `take_realtime()` (at most
  `SYSEX_REALTIME_HELD`, 64). Any other status ends the message as truncated.
- `sysex_bytes(payload)` keeps real-time bytes and stops at any other status, so a payload cannot put a status on
  the wire.
- `system_packet(group, status, d1, d2)` is public (used by the sequencer side).

### `bifrost-midid`: review of `seq.rs`, `rawmidi.rs` and the loop

- Sequencer reads, short buffer: the kernel copies whole events only. If the first waiting event's payload is
  larger than the buffer, it returns EAGAIN, the same answer as for an empty queue, and leaves the event at the
  head. The port then stalled forever with `poll` always ready, a busy loop. `Seq::read_events` now tells the two
  cases apart with a zero-timeout poll and grows its own buffer (doubling, capped at `READ_MAX` = header +
  `SYSEX_MAX_BYTES`). EINTR retries. ENOSPC (the kernel cleared an overflowed input queue) is logged once per new
  error text, not every loop.
- `parse_events` returns each event with its payload (`SeqRead`). `ext.len` is masked with `EXT_LEN_MASK`
  (the top two bits are kernel pointer flags). A header or payload cut short ends parsing instead of misframing
  what follows, and a huge claimed length cannot overflow the cursor.
- Event types: system common and real-time events (song position, song select, quarter frame, tune request, clock,
  start, continue, stop, active sensing, reset) become type 1 packets (`SeqEvent::to_ump`). The kernel stores song
  position as msb * 128 + lsb. Every other kind is still ignored. Sequencer system exclusive is no longer skipped:
  its payload chunks go through one `Midi1ToUmp` per sender (client, port), so a message split over events stays
  whole.
- Rawmidi: `read` and `write` retry on EINTR. Before, a signal counted as the device going away. A device that
  only opens write-only (output-only) is no longer read or polled. Before, the EBADF read dropped it and the 2 s
  rescan reopened it, adding a new registry device each time. `open` reports the first (read-write) error, not
  the last fallback's.
- Lifetime: dead ports are keyed by node path, not by name, so two identical keyboards stay apart. The port's
  registry device is removed (`remove_external_device`) when it goes away, and dropping the `HwPort` closes its
  fd once. `HwPort::conv` lives as long as the open port, so running status and an open system exclusive carry
  across reads.
- Timestamps: each rawmidi port and the sequencer are stamped with `CLOCK_MONOTONIC_RAW` right after their own
  read. Before, one stamp was taken after `poll` for all ports. All bytes of one read still share a stamp.

Tests (expected words worked out by hand): `ump::tests::{sysex_payload_high_bit_bytes_follow_the_midi1_rule,
sysex_assembler_high_bit_bytes, byte_stream_sysex_is_packed_as_it_arrives, playback_edge_writes_safe_wire_bytes}`,
`seq::tests::{variable_length_payload_is_kept, short_reads_drop_the_partial_event, ext_len_flag_bits_are_not_length,
system_events_become_type_1_packets, read_buffer_growth_is_capped}`, `rawmidi::tests::open_errors_are_the_first_mode_s`.
Nothing was built or run in this round (owner rule). The lead builds once.

## Left

- Output: hub destinations are not yet written to rawmidi or the sequencer. `UmpToMidi1` is the playback edge that
  writer should use, with one per port and group.
- Per-byte kernel timestamps: `SNDRV_RAWMIDI_MODE_FRAMING_TSTAMP` (rawmidi) and sequencer real-time queue stamps
  would remove the shared stamp per read. Kernel UMP endpoints (`/dev/snd/umpC*D*`) and the UMP sequencer client
  (`midi_version` 1, 32-byte events) are not used.
- A sequencer event larger than `READ_MAX` would still stall the port (the kernel pools make this impossible in
  practice).
- `seq_sysex` converters are kept per sender for the daemon's life (at most 65 536, a few bytes each). They are
  not dropped when a client leaves.

## Midget verification

1. Lead build, then on Midget: `bifrost-midid --dump 30`.
2. USB keyboard: notes print `2090KKVV` lines with growing stamps. Send a SysEx dump from the keyboard (or
   `amidi -p hw:N,0 -S 'F0 7E 7F 06 01 F7'` into a loopback): lines `30..` for the packets. The identity request
   prints one complete packet `30047E7F 06010000`. No data byte in a type 3 word is above 7F.
3. Sequencer: `aconnect <kbd> <BifrOSt MIDI>`, then press keys and send SysEx: `seq c:p` lines for notes and type 3
   packets for the SysEx. A clock source (`aseqsend` or a DAW sending clock) prints `10F80000` lines.
4. Replug the keyboard twice during `--dump`: "went away" then "attached" once each, and `--list` shows it once.
5. Send a large SysEx over the sequencer (8 KB, for example with `aseqsend` from a file): it arrives whole, with no
   stall and no CPU spin (`top` shows `bifrost-midid` idle afterwards).
