# Wave 5: Media Toolbox parity (pure Rust models)

Three gap rows of the Media Toolbox framework (`docs/parity-matrix/data/media-toolbox.json`) now have pure Rust
models in `springboard/ui/src`. Each is a model only: none is wired into the audio service, the shell's media
module, or the SDK, and none has been tested on the VM or Midget (no builds were run in this wave).

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/audio_processing_tap.rs` | Audio processing taps: `MTAudioProcessingTapCreate`, `MTAudioProcessingTapCreateWithPreferredFormat`, `MTAudioProcessingTapGetSourceAudio`, `MTAudioProcessingTapGetStorage`, `MTAudioProcessingTap`, `MTAudioProcessingTapFlags` | `AudioTap<P: TapProcessor>` with create, create with a preferred format (checked up front), prepare (negotiates the format, resets the processor), process one interleaved `f32` block in place (checks the frame count and buffer length), unprepare (told once; also on drop). `storage()` and `storage_mut()` give the processor's state, as the storage pointer does. `PeakMeter` is a built-in processor. Placement is `PreEffects` or `PostEffects`. |
| `springboard/ui/src/format_readers.rs` | Professional video format reader registration: `MTRegisterProfessionalVideoWorkflowFormatReaders` | `ReaderRegistry` with named `FormatReader` entries (extensions, header bytes, professional flag). Register refuses empty names, duplicate names and readers that match nothing. Professional readers are hidden until `enable_professional_workflow(true)`. Lookups by extension (case and leading dot ignored) and by header bytes return the first visible match in registration order. |
| `springboard/ui/src/media_type_names.rs` | Localized media type and subtype names: `MTCopyLocalizedNameForMediaType`, `MTCopyLocalizedNameForMediaSubType` | `type_name(code, language)` for the five media type codes (English and Icelandic), `subtype_name(media, subtype)` for thirteen codec codes scoped to their media type. Codes must be exactly four bytes. `Language::from_tag` reads the primary subtag. |

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. Each has a `tests` module with
five or more behaviour tests whose expected values were worked out from the rules the code implements.

### Relation to existing code

- `springboard/ui/src/process_tap.rs` captures what a process plays into a ring for recording and does not change the
  mix. The audio processing tap is an in-line processor that may change the samples. The two stay separate; they share no types.
- `springboard/src/media/codec.rs` (the shell's codec kinds) is not imported, because it lives in the shell crate. The
  codec codes in `media_type_names.rs` are the codes the shell's containers use, written out as a table.
- `springboard/src/media/demux/mod.rs` keeps its built-in demuxers. `format_readers.rs` is a registry of descriptions only.

### Gaps that remain

- Audio taps: no hook in `springboard/audiod`, so no app can attach a tap to a real mix. The placement is recorded but not acted on.
  The tap does not convert the source into the preferred format; the caller must supply blocks in `AudioTap::format()`.
- Reader registration: readers are described, not loaded. No reader code runs and no out-of-process reader exists.
  No byte source or sidecar handling.
- Localized names: two languages only. An unlisted code has no name, and codec names are not translated.

## Official API names mirrored

Official Media Toolbox landing page (`https://developer.apple.com/documentation/mediatoolbox`, read with
`python3 -I tools/parity/docc.py mediatoolbox`):

- Audio Taps: `MTAudioProcessingTapCreate(_:_:_:_:)`, `MTAudioProcessingTapGetSourceAudio(_:_:_:_:_:_:)`,
  `MTAudioProcessingTapGetStorage(_:)`, `MTAudioProcessingTapGetTypeID()`, `MTAudioProcessingTapFlags`,
  `MTAudioProcessingTap`, `MTAudioProcessingTapCreateWithPreferredFormat(_:_:_:_:_:)`.
- Professional video workflows: `MTRegisterProfessionalVideoWorkflowFormatReaders()`.
- Utility: `MTCopyLocalizedNameForMediaType(_:)`, `MTCopyLocalizedNameForMediaSubType(_:_:)`.

The per-symbol DocC pages (`mediatoolbox/mtaudioprocessingtap` and the others) returned HTTP errors when fetched, so the
symbol-level facts come from the landing page's list only. The GetTypeID call and the individual flag values are
not modelled.

## REA evidence

- `ev_219054dc890874ba032143b0cbb610af3e29837791f84ca3a24b84ae33059eda`: the Media Toolbox framework page (recorded in the
  matrix data for this framework, not re-run in this wave).
- `ev_5aeb119be4193f0b292938fbd0a6fbdd42cf923c9ff341e21b4d55f632c5eeeb`: `tools/parity/rea-page.sh` on
  `https://developer.apple.com/documentation/mediatoolbox/audio-processing-taps`. The rendered page is the site's
  "page you're looking for can't be found" page, so this URL does not exist on the official site. The audio taps facts
  above come from the landing page's own list.

## Matrix rows closed

These rows are for the lead to apply to `docs/parity-matrix/`. Status is the new `partial`.

| Framework slug | Feature (as written in the data file) | New status | BifrOSt path |
|---|---|---|---|
| media-toolbox | Audio processing taps | partial | springboard/ui/src/audio_processing_tap.rs |
| media-toolbox | Professional video format reader registration | partial | springboard/ui/src/format_readers.rs |
| media-toolbox | Localized media type and subtype names | partial | springboard/ui/src/media_type_names.rs |
