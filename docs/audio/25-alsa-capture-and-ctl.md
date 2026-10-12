# Task 25: capture converter delay and PCM channel maps (`springboard/audio/alsa`)

Follow-up of 21 ("Not done: capture-side `PCM_DELAY`") and of 04's `ctl.rs`. Not built by the agent; the lead runs
`cargo test -p bifrost-audio-alsa`.

## What changed

| Change | Where | Notes |
|---|---|---|
| Converter latency for both directions: FIFO + `PCM_DELAY` minus the frames in the ring | `pcm.rs` `Pcm::converter_latency`, `ring_frames`, `converter_frames` | The kernel's delay is the ring frames plus `runtime->delay` (codec and link). Ring frames are `appl - hw` for playback and `hw - appl` (captured, not yet read) for capture; never negative. The pointers come from the status read right after `PCM_DELAY`, which has just synced the hardware pointer. A failed `PCM_DELAY` counts the FIFO alone. |
| `converter_delay` no longer overflows on a hostile delay | `pcm.rs` | `saturating_sub`. |
| Channel maps: TLV parser, current map, layout | new `chmap.rs` | `parse_chmap_tlv` reads the container `[TLVT_CONTAINER, bytes, [CHMAP_FIXED/VAR/PAIRED, bytes, pos...]...]` (or a bare entry) with every length checked against the bytes present; `current_map` reads the control's values (all zero before hw params); `Chmap::labels`/`layout` map `SNDRV_CHMAP_*` to `Label`. |
| Reading the chmap control | `ctl.rs` `chmap_id`, `Ctl::pcm_chmaps`, `pcm_chmap`, `pcm_layout` | Element: interface PCM, `device` = PCM device, `index` = substream, name `Playback Channel Map` / `Capture Channel Map`. TLV read with 4 KiB of room, one retry with 64 KiB on `ENOMEM`. |

Position to label: FL Left, FR Right, FC and MONO Center, LFE Lfe, SL/SR LeftSurround/RightSurround. RL/RR are the
surround pair when the map has no side pair (ALSA's quad and 5.1), and LeftRear/RightRear when it has one (7.1).
Phase-inverted positions keep their speaker; driver-specific positions, NA, RC and the height/wide positions are
`Unknown`. ALSA's 5.1 order (FL FR RL RR FC LFE) is not the standard 5.1 order, so its layout is tagged
`Descriptions`; FL FR FC LFE SL SR is tagged `Surround51`.

`ctl.rs` was reviewed against `asound.h`: the struct sizes, offsets and ioctl numbers pinned by its tests match;
`snd_ctl_tlv` is a header of 8 bytes (the ioctl number counts only it) followed by `length` bytes of room.

## For 24 (driver.rs)

Replace in `AlsaIo::start`:

```rust
self.conv_in = self.inp.as_mut().map_or(0, |p| p.converter_latency());
self.conv_out = self.out.as_mut().map_or(0, |o| o.converter_latency());
```

Posted in `#bifrost-audio`. Until then the input side still counts only the FIFO.

## Left

- The driver does not yet report the device layout: `AlsaDriver` enumeration could call `Ctl::pcm_layout` and
  publish it as the stream's preferred channel layout; the server-side property is 24's.
- Setting a map on VAR/PAIRED devices (the chmap control is read-only in the kernel's PCM core; HDMI drivers make
  it writable) is not done.
- `Ctl::tlv` (dB) still reads 16 bytes and decodes only `DB_SCALE`; `DB_RANGE` and `DB_MINMAX` (common on USB
  devices) are not decoded.

## Midget verification

1. `cargo test -p bifrost-audio-alsa` (host).
2. On Midget with the daemon stopped, `amixer -c0 controls | grep 'Channel Map'` lists the controls the new code
   reads; `amixer -c0 cget iface=PCM,name='Playback Channel Map'` while a stream plays shows the current map.
3. Start `bifrost-audiod` with a capture device (after 24 wires the call): the journal line
   `converter latency: playback X frames, capture Y frames` shows Y larger than the FIFO alone on HDA (the ADC
   path); compare with `bifrost-audio-bench audio-roundtrip --driver alsa:0` `p50_ms` against `reported_ms`.
