# iTunes Library parity: video track metadata

Framework: iTunes Library (https://developer.apple.com/documentation/ituneslibrary), matrix row in
`docs/parity-matrix/data/itunes-library.json`.

## What was built

`springboard/ui/src/video_info.rs` adds `VideoInfo`, the video track of a library item: frame width and height,
duration in seconds, frame rate and codec. It is a pure model in the `bifrost_ui` crate.

- `VideoInfo::new` validates each value. A frame needs a non-zero width and height, the duration must be finite and
  not negative, the frame rate must be positive and finite, and the codec name must not be blank.
- `resolution_label()` gives "4K" from 2160 lines up and "NNNp" below that. `is_hd()` is true from 720 lines.
- `duration_text()` gives `m:ss`, or `h:mm:ss` from one hour on. `aspect_ratio()` is width over height.

Six host unit tests cover the labels, the duration text, the aspect ratio and the validation. No library holds video
items yet, and the row is not verified on Midget.
