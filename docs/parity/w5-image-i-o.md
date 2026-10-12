# Wave 5: Image I/O (parity work)

Official reference: https://developer.apple.com/documentation/imageio (DocC index read with `tools/parity/docc.py imageio`).
REA evidence of the rendered landing page: `ev_b39a8b319e67f714c7c788866f9546cc1910d4815e4930299d342ca492aa1293`.

## What was built

1. **App-facing image source** (`springboard/sdk/src/image_source.rs`, `bifrost_app::image_source`).
   Mirrors the image-source part of the framework (`CGImageSource`, decode and thumbnail): `ImageSource::new` reads
   the format and pixel size from the PNG IHDR chunk or the JPEG frame header, before any pixel is decoded.
   Pictures wider or taller than the decode limit (`bifrost_render::imgdec::MAX_SIDE`, 8192) are refused from the
   header. `thumbnail_size` gives the fitted size from the header alone, `decode` returns premultiplied RGBA, and
   `thumbnail` decodes and scales down to a box without enlarging. Only PNG and JPEG are read; other data is
   `NotImage`.
2. **Header-only fit size** (`springboard/render/src/imgdec.rs`, `fit_dims`). The fit rule behind `fit_size` moved
   into `fit_dims(iw, ih, w, h)`, which takes dimensions instead of an `Image`. `fit_size` now calls it; its behaviour
   is unchanged.
3. **Tile grid for large pictures** (`springboard/render/src/image_tiles.rs`, `TileGrid` and `Tile`).
   Mirrors the tiled provider's preferred tile size. The grid gives the column and row counts (rounded up), a tile
   by position with edge tiles cut to the picture, row-major iteration, and `covering`, which lists the tiles that
   overlap a visible rectangle, clipped to the picture.

Nothing is wired into apps or the shell, and nothing was built or run (no cargo in this wave). The tests are written
by hand and not yet run on the host or on Midget.

## Official API names mirrored

- `CGImageSource` (image source: decode, size, thumbnail), `kCGImageSourceCreateThumbnail*` (thumbnail size request)
- Tiled image provider keys (`kCGImageProviderPreferredTileWidth`, `kCGImageProviderPreferredTileHeight`)
- Decode size limits: no direct equivalent in the reference; the header is read first, as the reference does.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/image-i-o.json`) | New status | BifrOSt path |
|---|---|---|---|
| Image I/O | Image source: decode from data or file | partial | springboard/sdk/src/image_source.rs |
| Image I/O | Downsampled and thumbnail decode | partial | springboard/sdk/src/image_source.rs, springboard/render/src/imgdec.rs (fit_dims) |
| Image I/O | Decode size limits | partial | springboard/sdk/src/image_source.rs (TooLarge before decode) |
| Image I/O | Tiled image providers | partial | springboard/render/src/image_tiles.rs |

Still open in this framework: image destination (encode and write, SDK API), format coverage beyond PNG and JPEG,
EXIF and GPS read in the SDK, XMP and IPTC, animation frames, colour profiles and HDR gain maps.

## Tests (hand-worked expected values)

- `image_source`: PNG header 300x200 gives Png and (300, 200); JPEG frame header 300x200 behind a 4-byte APP0
  segment gives Jpeg and (300, 200); `GIF89a` data is `NotImage`; a PNG cut to 20 bytes is `Truncated`; width 9000
  is `TooLarge` before decode; a 300x200 thumbnail box 150x150 gives (150, 100), 600x600 gives (300, 200), and a zero
  side is `EmptyBox`; a valid header with no pixel data gives `Undecodable`.
- `image_tiles`: 300x200 in 128 tiles gives 3 x 2 = 6; tile (2, 1) is (256, 128, 44 x 72); the order is row by row;
  the widths of row 0 sum to 300; a zero size gives no grid; a region at x 130 lies in tile (1, 1), which is
  (128, 128, 128 x 72); a region at x 120 to 140 covers columns 0 and 1; a region at x 290 to 390 clips to tile (2, 1);
  a region at x 400 is empty.

## Verification

- `python3 tools/check-names.py`: exit 0, no hits.
- No builds or tests run (wave rule).
