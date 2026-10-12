# Wave 4, theme-06: media reading and writing

Pure Rust models in `springboard/ui/src/` that close four gap rows across Core Media, AVFAudio, AVFoundation
(Capture, Audio) and Core Audio. Each module is a model with host unit tests; none is wired into apps or the shell,
and none has been built or run on the VM or Midget (no builds were run for this wave, per the task rules).

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/media_block.rs` | `CMBlockBuffer`, `CMMemoryPool` | `BlockBuffer`: owned byte blocks read as one run (`append`, `append_slice` with range check, `append_buffer`, `read` at an offset, `contiguous`). `MemoryPool`: bounded free list with best-fit `take` (zeroed), `give`, and created/reused counters. |
| `springboard/ui/src/sample_attach.rs` | `CMSampleBuffer` attachments and data notifications | `SampleAttachments` flags (not sync, depends on others, do not display, display immediately, dropped reason), `is_sync_point`, `is_displayable`. `SampleSet`: samples with a pending/ready/failed data state, a notification queue (`DataBecameReady`, `DataFailed`) drained by `take_events`, and `sync_points`. |
| `springboard/ui/src/audio_engine.rs` | `AVAudioEngine` graph (attach, connect, detach, prepare, start, render) | `AudioEngine` with a fixed output node (`OUTPUT`), player, gain and mixer nodes. `connect` checks the format at both ends, refuses a second input on single-input nodes, and refuses cycles. `prepare` orders only the nodes upstream of the output. `render(frames)` pulls interleaved float blocks; players advance their cursor and render silence after their end. |
| `springboard/ui/src/channel_layout.rs` | `ManagedAudioChannelLayout` | `ChannelLayout`: presets (mono, stereo, quadraphonic, 5.1, 7.1), `from_labels` with empty, duplicate and length checks, `index_of`, and a stereo fold (`downmix_frame`, `downmix_stereo`) that keeps the front pair, adds centre and side or rear channels at 1/sqrt(2), and drops LFE. |

Each module has its own `#[cfg(test)] mod tests` with hand-worked expected values (sample sums, gains, cycle
refusals, pool best-fit, notification order, downmix coefficients). `springboard/ui/src/lib.rs` gains exactly four
`pub mod` lines. `python3 tools/check-names.py` exits 0.

## Official API names mirrored

- Core Media block buffers: `CMBlockBuffer` (append, append slice, read via memory access, `isEmpty`).
  https://developer.apple.com/documentation/coremedia/cmblockbuffer
- Core Media memory pool: `CMMemoryPool`, "optimizes memory allocation when working with large blocks of memory".
  https://developer.apple.com/documentation/coremedia/cmmemorypool
- Sample attachments and notifications: `CMSampleBuffer.sampleAttachments`, `CMSampleBuffer.AttachmentKey`,
  `CMSampleBuffer.SampleAttachments`, `dataBecameReady`, `dataFailed`.
  https://developer.apple.com/documentation/coremedia/cmsamplebuffer
- Audio engine graph: `AVAudioEngine` (`init`, `attach`, `detach`, `attachedNodes`, `outputNode`, `connectNode(_:to:format:)`,
  `disconnectNodeInput`, `prepare`, `start`, `stop`, `isRunning`, `reset`).
  https://developer.apple.com/documentation/avfaudio/avaudioengine
- Channel layouts: `ManagedAudioChannelLayout`.
  https://developer.apple.com/documentation/avfaudio/managedaudiochannellayout (the DocC JSON endpoint returned 404 for
  this path; the REA capture below is the official page)

Gaps against the Apple surfaces that remain: no reference-counted shared buffers (`append_buffer` copies), no
block allocator or deallocator callbacks, no pool sharing across threads, no `CMSampleBuffer` format descriptions,
no `AVAudioPlayerNode` scheduling or completion callbacks, no MIDI connections, no manual rendering mode, no
device routing, and no `connectNode` multi-destination fan-out (a mixer is the only fan-in).

## REA evidence

| Official page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/coremedia/cmblockbuffer | `ev_73d713065752ff6575ee6d1bb4ca91e9b9ae0d89e6904a97492165678835273a` |
| https://developer.apple.com/documentation/coremedia/cmmemorypool | `ev_3f24bbd132ccca5b097a2083045ecdc6c75df2113912cb5265096d5160a2ce25` |
| https://developer.apple.com/documentation/coremedia/cmsamplebuffer | `ev_e2f62aaff1280948b065f9be487eb7decdf975fac6581ba8bd5f584b3620986c` |
| https://developer.apple.com/documentation/avfaudio/avaudioengine | `ev_51eb1dab888c722ba7254766e490aafdb37de757520f3843b80258a41df34f2f` |
| https://developer.apple.com/documentation/avfaudio/managedaudiochannellayout | `ev_01a63a8a7171adeafc7a5330725a307b10de80944d3a6693edea0edaea8aa794` (page title came back as the generic site title; treat as weak evidence) |

DocC (`tools/parity/docc.py`) was read for `coremedia/cmblockbuffer`, `coremedia/cmmemorypool`,
`coremedia/cmsamplebuffer` and `avfaudio/avaudioengine`. `coremedia/cmattachment` and `avfaudio/managedaudiochannellayout`
returned 404 from DocC; the sample attachment surface was read from the `cmsamplebuffer` page instead.

## Matrix rows closed (status set to partial)

| Framework slug | Feature (as written in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| core-media | Block buffers | partial | springboard/ui/src/media_block.rs (BlockBuffer) |
| core-media | Memory pool | partial | springboard/ui/src/media_block.rs (MemoryPool) |
| core-media | Sample attachments and notifications | partial | springboard/ui/src/sample_attach.rs (SampleAttachments, SampleSet) |
| avfaudio | Audio engine graph (AVAudioEngine) | partial | springboard/ui/src/audio_engine.rs (AudioEngine) |
| avfoundation-capture-audio | Audio engine and processing graph | partial | springboard/ui/src/audio_engine.rs (AudioEngine) |
| core-audio | Channel layouts and data types | partial | springboard/ui/src/channel_layout.rs (ChannelLayout, Label) |

Not closed this wave (left as gap): `avfoundation-capture-audio` "Audio settings keys" (no settings-key model was
built), and the other theme-06 rows that need device, codec or GPU access.

## Verification done

- `python3 tools/check-names.py`: exit 0 (no Apple, product or platform identifiers in the new .rs files).
- Read-only checks: parity search and the akurai index found no existing node graph, channel layout, block buffer or
  memory pool in `springboard/ui/src` or `springboard/core/src`; `audio_render.rs` is a plug-in render unit model and
  was left unchanged.
- Not run: any cargo build or test (per the task rules). The test expectations were worked out by hand.
