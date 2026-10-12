# PhotoKit parity, wave 3: change details, media kinds and paired movies, change requests

Pure Rust models in `springboard/ui/src` (bifrost-ui, std only, no I/O). The shell and the SDK are not wired to them
yet; they are tested on the host only (`cargo test -p bifrost-ui photo_`, not run in this round).

## What was built

| Module | Mirrors (official docs) | What it does |
|---|---|---|
| `springboard/ui/src/photo_changes.rs` | `PHPhotoLibraryChangeObserver`, `PHChange` (`changeDetails(for:)` on a fetch result) | Diffs two fetched listings of asset identifiers with modification stamps into removed, inserted, moved and changed indexes. Order comes from `diff::diff`, so moves are minimal. Refuses duplicate identifiers. |
| `springboard/ui/src/photo_media.rs` | `PHAssetMediaType` (image, video, audio, unknown), `PHAssetMediaSubtype.photoLive`, `PHAssetResourceType.pairedVideo` | Kind of a file from its type (`uttype::type_for_extension` and `conforms`). Pairs a still with a movie that shares a content identifier, first with first, in still order. |
| `springboard/ui/src/photo_request.rs` | `PHAssetChangeRequest`, `PHAssetCreationRequest` (`forAsset()`, `creationRequestForAssetFromImage(atFileURL:)`) | Batch of create, favourite and trash operations. Plans the batch against the library listing: a name clash gets `-2`, `-3` before the extension, unsupported types and unknown or repeated assets fail the whole batch. Writes nothing. |

Existing code reused: `springboard/ui/src/diff.rs` (snapshot diff), `springboard/ui/src/uttype.rs` (file type and
conformance). Not duplicated: `contact_changes.rs` (a history store for contact records) and `contact_access.rs`
(the access prompt for contacts) were read and left as they are. The photo library access row was not closed: the
access model in `contact_access.rs` already has the same shape, and copying it under a photo name would duplicate it.

Each module is registered with one `pub mod` line at the end of `springboard/ui/src/lib.rs`.

## REA evidence

- Landing page `https://developer.apple.com/documentation/photokit`: `ev_21c4eecf662392c31122eef078375458e2daa387c163bcdbf731d09a264428bf`
  (the wave 2 matrix row uses `ev_c5daec50d0ec7872629a366160cc42fbfa15d118ce6fd1d0e4113eb6a3df1689`).
- Symbol pages read with `tools/parity/docc.py` (DocC JSON, not REA): `photos/phphotolibrary`, `photos/phchange`,
  `photos/phassetmediatype`, `photos/phassetmediasubtype`, `photos/phassetresourcetype`, `photos/phassetchangerequest`,
  `photos/phassetcreationrequest`, `photos/phphotolibrarychangeobserver`.

## Matrix rows closed (status gap to partial)

| Framework slug | Feature (as in `docs/parity-matrix/data/photokit.json`) | New status | BifrOSt path |
|---|---|---|---|
| photokit | Change observation of the library | partial | springboard/ui/src/photo_changes.rs |
| photokit | Video assets | partial | springboard/ui/src/photo_media.rs |
| photokit | Live Photos | partial | springboard/ui/src/photo_media.rs |
| photokit | Asset resources and creating assets | partial | springboard/ui/src/photo_request.rs |

Still open after this round: the shared library object (`PHPhotoLibrary`) and access levels, enhanced privacy and
limited access, and the editing and project extension points. Playback of Live Photos and video is not built.
Movies are not read from files; the caller supplies the content identifier.

## Tests

- `photo_changes`: identical listings, insert at the end, removal, a stamp change, a single move, insert plus remove
  plus change together, and a duplicate identifier.
- `photo_media`: kind by file type (including upper-case extensions and unknown types), still and movie pairing,
  unmatched identifiers, empty and missing identifiers, two stills without a movie, pairing order and extra stills.
- `photo_request`: unique names (free, taken, no extension, leading dot), clashes within a batch and with the library,
  unsupported type, bad names, unknown asset, repeated asset, and the order of favourites and trash.

Tests were not run: this round does not build.
