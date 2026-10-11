# Quick Look parity: data-based previews and thumbnail requests

Framework: Quick Look (https://developer.apple.com/documentation/quicklook), matrix row in
`docs/parity-matrix/data/quick-look.json`.

## What was built

Pure models in `springboard/ui/src/preview.rs` (`bifrost_ui::preview`), re-exported to apps as
`bifrost_app::peek::{Attachment, Content, ModelError, Registry, Reply, Representation, ThumbnailRequest}`
in `springboard/sdk/src/peek.rs`. Std only, no unsafe, no new crates.

- `Registry` (mirrors the supported-types list of a preview extension, with `QLPreviewProvider` as the
  provider role): `register(provider, types)` replaces a provider's types. `provider_for(file_name)` takes the
  extension to a type with `uttype::type_for_extension`, then `provider_for_type` picks the registration whose type
  is nearest in the type tree (`uttype::distance`). A registration for the exact type beats one for an ancestor;
  ties go to the earlier registration.
- `Reply` and `Content` (mirror `QLPreviewReply`: HTML with attachments, PDF page size, bitmap context, file URL).
  `Reply::new` validates: PDF page sizes finite and positive, bitmap edges 1 to 16384 px, attachment ids unique and
  free of whitespace and quotes so they work in `cid:` URLs, file replies with a name. `display_title` falls back to
  the file name when the title is empty, as the official page describes. `resolve_cid` finds an attachment from a
  `cid:<id>` URL; a bare id does not match.
- `ThumbnailRequest` and `Representation` (mirror `QLThumbnailGenerator.Request`: `size`, `scale`,
  `representationTypes`). `new` refuses a non-positive or non-finite size, a scale above 4, and an empty
  representation list. `pixel_size` rounds size times scale to whole pixels, at least 1. `best_of` returns the best
  requested representation that was produced (thumbnail, then low quality, then icon).

Existing code kept and extended, not duplicated: `springboard/sdk/src/peek.rs` (`PreviewPanel`, `thumbnail_path`,
`ThumbnailCache`) gained the re-exports and three tests.

## Official API names mirrored

- `QLPreviewProvider`, `QLFilePreviewRequest`, `QLPreviewReply`, `QLPreviewReplyAttachment`:
  https://developer.apple.com/documentation/quicklook/data-based-preview-extensions (DocC, Mac Catalyst 15.0)
- `QLPreviewReply` initializers and properties (`title`, `attachments`, `init(data:contentType:)`):
  https://developer.apple.com/documentation/quicklook/qlpreviewreply
- `QLPreviewReplyAttachment`: https://developer.apple.com/documentation/quicklook/qlpreviewreplyattachment
- `QLThumbnailGenerator.Request` (`init(fileAt:size:scale:representationTypes:)`, `size`, `scale`,
  `representationTypes`): https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator/request
- `QLThumbnailGenerator.generateBestRepresentation`: https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator

## Evidence

- REA evidence of the rendered Quick Look landing page: `ev_182b464c6ee38b37c3521e821881b194fbed559a89f535750789496e3e44f2c4`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/quicklook`). The landing page says that
  a data-based preview extension implements a `QLPreviewProvider` subclass that returns a `QLPreviewReply` for a
  `QLFilePreviewRequest`, and that a view-controller-based extension conforms to `QLPreviewingController`.
- The previous matrix row cited `ev_8f8e5e93fdbdd4c20445855d1b664ab16b942f834b9fc8f25ec0ddcada546ba3`; it is
  kept in the history of the JSON and not re-checked here.
- DocC (`python3 -I tools/parity/docc.py ...`) read for `quicklook`, `quicklook/qlpreviewreply`,
  `quicklook/qlfilepreviewrequest`, `quicklook/qlpreviewreplyattachment`, `quicklook/qlpreviewprovider`,
  `quicklookthumbnailing/qlthumbnailgenerator`, `quicklookthumbnailing/qlthumbnailgenerator/request`.
  The nested `.../request/representationtypes` and `.../representation` paths returned 404.

## Tests

- `springboard/ui/src/preview.rs`: 9 tests (content validation, attachment ids, title fallback, `cid:` resolution,
  provider choice by type distance, replacing a registration, app-defined types, thumbnail pixels and scale limits,
  best representation).
- `springboard/sdk/src/peek.rs`: 5 tests in total (2 existing, 3 new through the SDK path).
- Not run: per the task, no cargo or rustc build was run in this round. The next stage must build and test.

## Remaining gaps

- The shell does not draw the floating preview panel (`ToShell::Preview` is decoded but has no handler).
- No process runs a data-based preview provider. `Registry` and `Reply` are models only.
- No thumbnail renderer. `ThumbnailRequest` models the request; nothing produces the image, and no icon-mode
  badge or update handler exists.
- No view-controller-based preview extension (`QLPreviewingController`).
- PDF: no rendered multi-page context and no PDF style option. `Content::Pdf` carries a page size only.
- No `QLPreviewView`, `previewItemDisplayState`, transition frames or full-screen mode.
- The `RepresentationTypes` case names (`Thumbnail`, `LowQuality`, `Icon`) are BifrOSt's; the nested official
  page did not load, so the exact Apple case names are not confirmed.
- Doc errors in `docs/bifrost-ui.md` lines 1420 and 1423 (file names and cache directory) remain; this round did not
  edit shared docs.
