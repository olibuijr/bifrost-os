# Parity: thumbnails (Quick Look Thumbnailing, round 75)

Slice `thumbnails`. Matrix rows: `docs/parity-matrix/data/quick-look-thumbnailing.json` (framework "Quick Look
Thumbnailing") and the thumbnail parts of `quicklook-ui.json`. The matrix is not edited here; the lead closes rows.

## Evidence

- Reference inventory: `~/Projects/macos-ref/extract/ref27-index.tsv` lists the thumbnail extensions shipped with
  the macOS 27 build: `System/Applications/Books.app/Contents/PlugIns/BooksThumbnail.appex` (a book's first page),
  `System/Library/Frameworks/Quartz.framework/Versions/A/Frameworks/ImageKit.framework/Versions/A/PlugIns/OFD_Thumbnail.appex`
  and `System/Applications/Shortcuts.app/Contents/PlugIns/ThumbnailExtension.appex`. So a document's thumbnail is its
  first page or its embedded picture, and the system draws an icon otherwise. No REA evidence id was taken for this
  slice; the design uses only the list above and the existing BifrOSt code.
- Existing BifrOSt code read before the change: `springboard/src/thumbs/` (providers and generator, wave in progress),
  `springboard/src/shell/services/thumbnails.rs` (service, not registered), `springboard/ui/src/thumb_service.rs`
  (model), `springboard/sdk/src/peek.rs` (private cached-path thumbnails), `springboard/ui/src/thumbnail.rs` and
  `springboard/ui/src/preview.rs` (a second thumbnail request model), `springboard/src/shell/preview_panel.rs` (its own
  picture decoder), `springboard/apps/files/src/views/options.rs` (its own picture decoder).

## Rows

| API (reference name) | BifrOSt path | Done | Needs Midget |
|---|---|---|---|
| QLThumbnailGenerator, generateBestRepresentation, Request (file, size, scale) | `ui/src/thumb_service.rs` (`ThumbRequest`, `edge_px`); `shell/services/thumbnails.rs` (service `thumbnails`, method `request`); `sdk/src/thumbnails.rs` (`request`, `reply`, `event`, `load_pixels`) | Request, cache key and the pending/ready flow; registered in `shell/services/mod.rs`; SDK client | Thumbnails in Files and the Quick Look panel on the device |
| QLThumbnailGenerator update handler (progressive) | `shell/services/thumbnails.rs` (pending, then the `ready` or `failed` event to every asker) | Pending answer, then the event, deduplicated per key | Event delivery to Files on the device |
| QLThumbnailRepresentation, RepresentationTypes (icon, low quality, thumbnail) | `ui/src/thumb_service.rs` (`ThumbKind::Icon`, `ThumbKind::Thumbnail`); `thumbs/mod.rs` (`Registry::render`) | Icon and thumbnail kinds. No low-quality tier | No |
| QLThumbnailError | `sdk/src/thumbnails.rs` (`State::Failed(message)`) | Message only; no error domain or codes | No |
| Thumbnail generation for images, text, PDF, fonts, audio and video, packages, contacts, calendar, clippings | `springboard/src/thumbs/` (`images.rs`, `text.rs`, `media.rs`, `cards.rs`) | Images, Office embedded pictures, text and RTF pages, HTML titles, fonts, audio and video covers, archives and packages (icon with a format badge), vCards, calendar events, clippings. PDF: missing (see Open) | Pictures and cards on the device |
| Thumbnail cache on disk | `shell/photos/thumbs.rs` (`DiskCache`, RIMG files, least recently used go first); `thumbs/mod.rs` (`cache_dir`, `$XDG_CACHE_HOME/bifrost/thumbnails`) | Disk cache of raw images, bounded by count and bytes; the service's cache is shared by every app | Cache survives a restart |
| Files icon view and gallery (system-wide thumbnails in Files) | `apps/files/src/views/options.rs` (`ViewOpts::thumb`, `blit_thumb`, `thumb_answer`); `apps/files/src/system.rs` (`request_thumbnails`, `Effect::Thumbnail`, `take_thumb`) | Done: the view asks, the service draws, the answer fills a slot. The picture decoder is deleted | Icon view, gallery and the preview column on the device |
| Quick Look panel draws the item's picture | `src/shell/preview_panel.rs` (`Body::Loading`, `Panel::take_wanted`, `Panel::thumbnail`); `src/shell/services/quick_look.rs` (`PreviewService` asks through its `Thumbnails`) | Done: pictures, PDFs, audio and other items wait for the service; icon answers show the kind's line; text stays a read of a few lines | Panel on the device, arrow keys across items |
| Searchable item thumbnails (Spotlight and search panel) | not wired | No. `sdk/src/quicksearch.rs` stores a thumbnail path that the search panel does not draw | Open |

## Deleted (adopt and delete)

- `ui/src/thumbnail.rs` (Quick Look thumbnail model: `FileThumbnailRequest`, `ThumbnailReply`, `Progressive`,
  `ThumbnailError`) and the module line in `ui/src/lib.rs`. Its callers were the SDK re-exports only.
- `ui/src/preview.rs`: `Representation`, `ThumbnailRequest`, `MAX_SCALE`, `ModelError::NoRepresentation` and their tests.
  `bifrost_app::peek` no longer re-exports them.
- `ui/src/thumb_service.rs`: `eviction_victims` (the photos disk cache keeps the one used on disk).
- `sdk/src/peek.rs`: `thumbnail_path`, `ThumbnailCache` and their tests. The panel's pictures come from the service.
- `apps/files/src/views/options.rs`: `picture_thumb`, its tests, and the decode-on-paint path with its `is_image_name`
  and size gates. `ViewOpts::thumbs` is now a slot map of service answers.
- `src/shell/preview_panel.rs`: `MAX_IMAGE_BYTES` and the shell's own `cg::imageio::decode` of a picture.

## Open (not done in this round)

- PDF thumbnails. The provider needs a PDF reader. The Books engine (`src/shell/books/engine/pdf/mod.rs`) declares
  `doc`, `content`, `font`, `image` and `obj` modules whose files now live under `render/src/cg/pdf/`, and that module
  is still commented out (`render/src/cg/mod.rs`: "re-enabled by the PDF slice"). Until it compiles, a PDF gets its type
  icon with the badge `PDF`, and the panel says "Open it in Books". The planned provider: the first page's text, with
  the title, through `text::render_page` (same as the text provider). This also blocks the Books engine build; the
  lead should check it.
- Spotlight and the search panel do not draw thumbnails (`quicksearch.rs` stores a path only).
- Books has its own cover and thumbnail code (`src/shell/books/engine/mod.rs` `thumb`). It should ask the service for
  covers when the Books slice is done. Not touched here.
- The photos library keeps its own disk cache of square covers (`shell/photos/thumbs.rs`). It shares the RIMG format
  and `eviction_victims`, so it is the store the service uses. A later step moves `DiskCache` into `thumbs/` once Photos
  is reviewed.
- Files' in-app preview fallback (`apps/files/src/peek.rs`) still decodes a picture for the body of its preview. That is
  a preview, not a thumbnail, and it runs only when the shell cannot show the panel.
- Packages and archives: the card is the icon with a format badge; contents are not opened.
- The shell's `thumbnails` service answers `"no service"` on an older shell; Files then keeps its icons (no crash).

## Checks (by reading; the lead builds)

- Shell: `shell/services/thumbnails.rs` tests (flow, icons, undrawable file, bounded queue); `thumbs/mod.rs` tests
  (registry, cache, icon kind, folder, fallback); `preview_panel.rs` tests (kinds, waiting bodies, answers for the
  current item only); `quick_look.rs` tests.
- SDK: `sdk/src/thumbnails.rs` tests (reply and event decoding, the raw image loader).
- Files: no test change; `views/options.rs` lost its decoder tests.
- Midget: open a folder of pictures, text and a zip in icon view and the gallery; press Space on a picture and step
  with the arrow keys.
