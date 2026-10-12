# Wave 4: Core Media gap rows (pure Rust)

Framework: Core Media (`docs/parity-matrix/data/core-media.json`). Three gap rows were built as pure models in
the existing crates. No builds or tests were run for this wave (the wave rules forbid it); the host tests are
written and the integration build runs at the end of the wave.

## What was built

| Module | Mirrors (official Apple names) | What it does |
|---|---|---|
| `springboard/core/src/block_pool.rs` (`BlockPool`, `PoolStats`) | `CMMemoryPool` (class, "optimizes memory allocation when working with large blocks of memory") | Reusable byte blocks with at least `block_size` capacity. `take` reuses an idle block, `give` keeps up to `max_idle` blocks and refuses undersized or surplus ones, `trim` releases idle blocks. Counters: allocated, reused, dropped, idle. |
| `springboard/ui/src/sample_attachments.rs` (`SampleAttachments`, `BufferAttachments`, `SampleList`, `DroppedFrameReason`) | Sample attachment keys (`kCMSampleAttachmentKey_*`, `kCMSampleBufferAttachmentKey_*`), the dropped frame reason values, and `CMSampleBufferGetSampleAttachmentsArray(_:createIfNecessary:)` | Per-sample flags (not sync, partial sync, depends on others, depended on by others, display immediately, do not display, earlier display times allowed, redundant coding) with the documented defaults. Per-buffer values (empty media markers, dropped frame reason, speed multiplier validated as finite and above zero, force key frame, reverse, decoder reset). `SampleList` creates a sample's entry only when asked, as the array call does. |
| `springboard/ui/src/text_markup.rs` (`TextMarkup`, `Argb`, `Layout`, `Alignment`) | `CMTextMarkup` attribute keys (fonts, styles, colours, layout, placement percentages) | One cue's markup. `merge` applies a later cue over an earlier one (set attributes replace, style flags only turn on). `font_height_px` computes the font height from the base size (percent of video height) and the relative size. `origin` places the text block along and across the writing direction. `Argb` packs 0xAARRGGBB. Reuses `captions::EdgeStyle` for the edge style. |

Each module has a `#[cfg(test)] mod tests` with behaviour tests whose expected values were worked out by hand from
the rules above (for example, 1080 px at 5% with 50% relative size is 27 px; a horizontal block at 50% and 90% in a
1920x1080 box starts at (960, 972)).

Notes and limits:

- The sample attachments model does not yet cover sample buffer notifications (`Sample Buffer Notifications`), so the
  row is `partial`, not `parity`.
- The text markup model stores the attributes and resolves font height and placement. It does not draw text, and it
  does not parse subtitle tracks (the WebM parser still drops them).
- `Layout` has two values, horizontal and vertical. Reference values for the vertical direction are not in the
  attribute page, so the model keeps the two directions the captions need.
- The generic font family is kept as a string. The reference page lists the attribute but not its value set.
- The memory pool is a pure model. Nothing in the media pipeline uses it yet.

## Official API references

- `https://developer.apple.com/documentation/coremedia/cmmemorypool` (docc: `coremedia/cmmemorypool`)
- `https://developer.apple.com/documentation/coremedia/sample-attachment-keys` (docc: `coremedia/sample-attachment-keys`)
- `https://developer.apple.com/documentation/coremedia/cmtextmarkup` (docc: `coremedia/cmtextmarkup`)
- Framework landing page: `https://developer.apple.com/documentation/coremedia`

## REA evidence

- `ev_db73c0f33951e7a133ac868ad48b617bbb62103e3f5de3897f5b92da5fe26292`: Core Media landing page (topics listed,
  including CMMemory Pool, CMAttachment and CMText Markup).
- `ev_be12ccd85fa722bbc6e135b7b0065acd9ed7d8cef6c0d710f25a241bb7f50b1e`: CMMemoryPool page (class, "optimizes memory
  allocation when working with large blocks of memory").
- `ev_431d808f73cdee7eb6dbeaa88c74f7c96d65df801f1ed0b64d7eb09ca6e3ff69`: Sample Attachment Keys page.
- `ev_df9cce6e40abd6605df3a3646dbf9cb0bc7d454ceae75baa3b7dc8709ff7bf45`: CMTextMarkup page (font, style, colour and
  layout attribute keys).

## Matrix rows closed

Status is the new status proposed for the central matrix update. The lead applies it.

| Framework slug | Feature (as written in `core-media.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-media | Memory pool | partial | springboard/core/src/block_pool.rs |
| core-media | Sample attachments and notifications | partial | springboard/ui/src/sample_attachments.rs |
| core-media | Text markup for legible media | partial | springboard/ui/src/text_markup.rs |
