# PDF engine (parity slice pdf-engine)

Owner: round 75. Matrix frameworks: PDFKit, Quartz (PDF parts). Branch `parity/pdf-engine`.
Matrix rows are not edited here; the lead closes them from the table below.

## Where the engine lives now

The PDF file reader, content interpreter and page renderer are in `springboard/render/src/cg/pdf/`
(crate `bifrost-render`, module `cg::pdf`). The Books app keeps only the reflow step that turns
extracted text lines and pictures into its `Book` model (`springboard/src/shell/books/engine/pdf/mod.rs`).

| File | Role |
|---|---|
| `doc.rs` | file structure: xref chain, repair scan, object streams, empty-password decryption (`crypt.rs`) |
| `obj.rs`, `filter.rs`, `inflate.rs` | objects and lexer; stream filters; DEFLATE (also used by the ZIP reader and the sdk) |
| `page.rs` | page tree with inherited attributes, page boxes and rotation, document info, destinations, outline tree, link annotations |
| `font.rs`, `type1.rs`, `tables.rs` | fonts: embedded TrueType, CFF, Type 1, Type 3, standard encodings; `from_resources` loads a page's font once |
| `image.rs`, `color.rs`, `function.rs`, `shading.rs` | images with masks, colour spaces, PDF functions, shadings types 1 to 7 |
| `extract.rs` | text extraction: lines with baseline and size, images, in reading order |
| `sink.rs` | `PdfSink` trait: the drawing interface the page renderer talks to, plus a recording sink for tests |
| `interpret.rs` (new) | content-stream interpreter: paths, clipping, colour, text and glyph outlines, forms, images (also inline), shadings, ExtGState, Type 3 procedures |
| `draw.rs` (new) | `ContextSink` paints `PdfSink` calls into a `cg::context::DrawContext`; `render_page`, `render_page_fit`, `fit_size`, `render_file_page` |

## Rows

| Matrix entry (PDFKit / Quartz) | Status after this slice | BifrOSt path | Still open |
|---|---|---|---|
| PDFPage: rendering and drawing | engine done: pages are rasterised through `cg::context` (`draw::render_page*`), with paths, clipping, text as glyph outlines, images, shadings and blend modes | `render/src/cg/pdf/interpret.rs`, `draw.rs`; Viewer `apps/preview-desktop/src/document.rs` (lazy page pictures, PNG export) | no app-facing SDK PDF page API yet; tiling patterns and soft masks are not drawn; not verified on Midget |
| PDFPage: page image and PDF creation | page image available via `render_page`; PDF creation untouched | `draw.rs` | PDF writing |
| PDFDocument loading (URL, data) | moved to `cg::pdf::doc` (`Doc::open`); Books and Viewer both load through it | `render/src/cg/pdf/doc.rs`; `src/shell/books/engine/pdf/mod.rs` | no app-facing document object |
| Document structure: xref repair, objects | moved unchanged to `cg::pdf::doc` / `obj` | `render/src/cg/pdf/doc.rs`, `obj.rs` | none new |
| Encrypted PDFs (empty user password) | already present in `cg::pdf::crypt` (RC4, AES-128, AES-256 with empty user password); a non-empty user password is refused with a message | `render/src/cg/pdf/crypt.rs` | password prompt and owner-password files |
| PDFPage: page tree and text extraction | moved to `cg::pdf::page` and `extract` | `render/src/cg/pdf/page.rs`, `extract.rs` | none new |
| Outline (table of contents) | engine outline tree (`page::outline`, flattened with levels) now feeds the Books contents; the Books loader no longer has its own copy | `render/src/cg/pdf/page.rs` | no outline tree object for apps |
| Document thumbnails in Viewer and Files | first page drawn by the engine for the Files pane; Viewer draws thumbnails at open | `src/files/mod.rs`, `apps/preview-desktop/src/document.rs` | Books library covers for PDFs |

## Migrations and deletions

- Books loader: `pages`, `walk`, `text_string`, `info`, `dest_page`, `named_dest`, `name_tree`, `outline` removed in favour of `cg::pdf::page`; `Doc`, `Extractor`, `Piece` come from `cg::pdf`.
- `springboard/src/shell/books/engine/inflate.rs` moved to `cg::pdf::inflate`; callers in `src/files/zip.rs`, `sdk/src/archive.rs`, `sdk/src/compression.rs` now import it from there.
- `src/thumbs/images.rs` decodes through `cg::imageio::decode` (the old `imgdec::decode` no longer exists).
- Font loading for the text extractor and the renderer share `font::from_resources`.
- Viewer: text reflow path (`Body::Text`, `chunk_text`, `TEXT_PAGE_CHARS`) deleted. Viewer 0.2.0.
- Files pane: a PDF preview is the first page drawn by the engine instead of the extracted text.

## Verification status

Written by reading only (no builds or tests run in this round, per the brief). Unit tests were added for:
the content interpreter (`interpret.rs` tests: fills, colour order, clipping, stroke state, unbalanced
saves, text, inline image key expansion); the renderer (`draw.rs` tests: warping and stencils, device box
clipping, axial and radial shadings, triangles, page render size and colour, file page errors); the Books
loader (existing tests kept). The lead should run `cargo test -p bifrost-render cg::pdf` and the Books and
Viewer tests, and check a PDF page on Midget before marking any row `parity`.

Reference evidence: the matrix's PDFKit and Quartz rows were the only reference used. No macOS reference
files were inspected for this slice. The PDF behaviour follows the PDF specification.
