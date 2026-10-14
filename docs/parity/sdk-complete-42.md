# Parity sdk-complete-42: finished behaviour for partial rows

This round completes missing behaviour in eight partial rows. Every new model is pure and device-free, and its tests run
on the host. The lab machine is still needed for the wiring of each feature into the shell and apps.

## New modules

| Module | Mirrors | Wired | Needs the lab machine |
|---|---|---|---|
| `springboard/sdk/src/password_suggest.rs` (`bifrost_app::password_suggest`) | Password suggestions for a page, and the save decision after sign-in | Not yet; the passwords SDK and the keychain group are unchanged | Sign-in form and save prompt on Midget |
| `springboard/render/src/cg/pdf/outline_tree.rs` (`bifrost_render::cg::pdf::outline_tree`) | The outline (bookmark) tree of a PDF as nested entries | Not yet; Books uses its own copy of the outline | Books table of contents on Midget |
| `springboard/src/media/time.rs` (`springboard::media::time`) | Rational media time and time ranges | Not yet; the demuxer, sample table and player keep their units | None (host tests) |
| `springboard/src/media/pcm_channels.rs` (`springboard::media::pcm_channels`) | Planar PCM buffers, one per channel | Not yet | None (host tests) |
| `springboard/src/media/metadata.rs` (`springboard::media::metadata`) | Metadata key spaces, identifiers and typed tag collections | Not yet; the readers return their own fields | None (host tests) |

## Changed behaviour

- `springboard/src/shell/services/calendar.rs`: the `events` method expands repeating events to their occurrences in the
  requested range, and refuses ranges longer than `MAX_RANGE_DAYS`. The Calendar app's repeat rules are reused.
- `springboard/sdk/src/bluetooth_hid.rs` and `springboard/ui/src/view_camera.rs`: unchanged code; their tests are in the
  new `parity_sdk-complete-42.rs` files of the `sdk` and `ui` crates.

## Test files

- `springboard/sdk/tests/parity_sdk-complete-42.rs`: sign-in suggestions, save decisions, HID counts and the ignore list.
- `springboard/ui/tests/parity_sdk-complete-42.rs`: the 3D view camera model.
- `springboard/render/tests/parity_sdk-complete-42.rs`: the outline tree.
- The media models carry their tests inline in `time.rs`, `pcm_channels.rs` and `metadata.rs`.
