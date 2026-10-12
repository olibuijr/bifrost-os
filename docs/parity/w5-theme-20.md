# Wave 5, theme-20: HDR per-frame metadata, tiled image providers, mesh buffer mapping

Theme `theme-20` (size 3) groups three Gap rows from three frameworks. Each one gets a pure model in the crate that
owns the feature. Nothing is wired into apps or the shell, and nothing has been tested on the VM or Midget. Host unit
tests are written by hand but not run in this round (no builds in this wave). All three rows move to `partial`.

## What was built

| Module | Crate | Framework row | What it does |
|---|---|---|---|
| `springboard/render/src/hdr_frame.rs` (new) | render | Video Toolbox | PQ (SMPTE ST 2084) and HLG (ARIB STD-B67) encode and decode, plus a `FrameSession` that takes per-sample nits for each frame and returns per-frame metadata: peak and average signal as 12-bit codes, a scene-change flag, and running MaxCLL and MaxFALL. The transfer comes from the existing `buffer_attach::Transfer`. |
| `springboard/render/src/imgtile.rs` (new) | render | Image I/O | `TileGrid` cuts a picture into tiles of a preferred size, clips edge tiles, and lists the tiles covering a rectangle. `TileCache` holds the most recently used tiles (LRU by counter) and fills the rest from a caller-supplied loader. `read_region` stitches a rectangle out of the tiles. |
| `springboard/ui/src/mesh.rs` (extended) | ui | Model I/O | Adds `BufferStore`: CPU-side bytes behind the existing `BufferAllocator` ranges. `map` and `map_mut` give access to exactly one range, `fill` writes at an offset inside a range, and every access is bounds-checked (including overflowing offsets). |

Both new modules are registered with one `pub mod` line each in `springboard/render/src/lib.rs`. `mesh.rs` already
was a module of `bifrost_ui`.

## Official API names mirrored

- Video Toolbox: `VTHDRPerFrameMetadataGenerationSession` (`init(framesPerSecond:hdrFormats:)`, `attachMetadata(to:sceneChange:)`).
  The model keeps the frames-per-second argument and the scene-change flag. It does not attach metadata to a
  pixel buffer, and it does not mirror `hdrFormats` (the official page lists `dolbyVision` only; no bitstream is written).
- Image I/O: `kCGImageProviderPreferredTileWidth` and `kCGImageProviderPreferredTileHeight`, mirrored as the preferred
  tile size (`TileGrid::new`). No image source or provider object is mirrored, and no file format reads tiles.
- Model I/O: `MDLMeshBuffer` (`fill`, `map`, `length`, `type` as `BufferKind`), `MDLMeshBufferAllocator` (allocate a
  buffer of a length; `BufferAllocator` in `mesh.rs`), `MDLMeshBufferData` (the bytes; `BufferStore`), and
  `MDLMeshBufferMap` (direct access to the bytes; `BufferStore::map` and `map_mut`).
  Not mirrored: `MDLMeshBufferZone` (zones are not built; the official page names them as a memory pool) and the
  allocator's buffer-from-data constructors.

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

- `ev_28a997738c043ffea2c4b9cb651eb5f95ad01a130a4b7f8dc7557ba8dfa31109`: https://developer.apple.com/documentation/videotoolbox/vthdrperframemetadatagenerationsession (macOS 15.0+, the session and its `HDRFormat`).
- `ev_e6c80beacf613db1d5262089175781e5171cbd914c15fba0d7de614e6302ae08`: https://developer.apple.com/documentation/imageio/kcgimageproviderpreferredtilewidth (macOS 26.0+).
- `ev_fc9e0cb301efed0a0705bd0c598f49131c0deb20c4d377abbc43dc5d8e8bf041`: https://developer.apple.com/documentation/modelio/mdlmeshbuffer (protocol; `fill`, `map`, `length`, `allocator`, `zone`, `type`).

DocC facts were read with `python3 -I tools/parity/docc.py` for the same pages and for the Model I/O buffer allocator, zone
and data pages.

## Tests (hand-worked, not run)

- `hdr_frame.rs`: PQ is exact at 10000 nits (1.0) and 0 decodes to 0; PQ round trip at 1, 100, 1000 and 4000 nits;
  HLG breakpoint at 1/12 (signal 0.5), peak at 1.0, and decode of 0.5 equals 1/12; luminance clamps negatives and rejects empty
  frames (average of 0, 100, 400, -5 is 125); sessions refuse SDR transfers and non-positive or NaN rates; running MaxCLL
  and MaxFALL (200/150, then 300/175); 12-bit codes at 4095 for a 10000-nit frame and 0 for black.
- `imgtile.rs`: a 1000 by 600 picture with 256 tiles has 4 by 3 tiles and a clipped edge tile of 232 by 88;
  missing or oversized preferred sizes give one tile; `covering` lists only touched tiles and nothing past the edge;
  LRU eviction order and reload counts; `read_region` stitches the values 0, 10, 1, 11 from four tiles; bad regions and
  loader failures (wrong pixel count) are refused.
- `mesh.rs` (`BufferStore`): fill then map reads back; `map_mut` writes are seen by `map`; ranges past the store and
  overflowing offsets are refused, and an empty range at the end is valid; fill stays inside its buffer and a
  refused fill writes nothing; allocations do not alias.

## Matrix rows closed (status `partial`; the lead updates the matrix)

| Framework slug | Feature (as in `data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| video-toolbox | HDR per-frame metadata | partial | springboard/render/src/hdr_frame.rs |
| image-i-o | Tiled image providers | partial | springboard/render/src/imgtile.rs |
| model-i-o | Mesh buffer allocation and mapping | partial | springboard/ui/src/mesh.rs (`BufferStore`, `BufferAllocator`) |

## Still missing (for the row notes)

- HDR: no HDR10+ or Dolby Vision bitstream; no attachment of metadata to a picture buffer; no compression session
  property for the log transfer function; no ColorSync HDR path (a separate row, not closed here); no display
  switching for HDR.
- Tiles: no provider or source object that an app can pass to a decoder; the JPEG and PNG decoders in
  `springboard/render/src/imgdec.rs` still decode whole pictures.
- Mesh buffers: no zones (`MDLMeshBufferZone`), no per-buffer type beyond vertex or element, and no SDK exposure: apps
  have no path to a `BufferStore`.
