# Parity sdk-complete-09: drawing, colour, media, store and MIDI rows

What this round adds, what is wired, and what still needs the lab machine. All new tests are headless
(`springboard/<crate>/tests/parity_sdk-complete-09.rs`); none opens a display or a device.

| Module | Mirrors | Wired | Needs the lab machine |
|---|---|---|---|
| `springboard/sdk/src/image_object.rs` (`ImageCatalog`, `tint_template`) | named images, template images | host model only | the kit's image widgets must use the tint, and the drawn result needs a check on Midget |
| `springboard/sdk/src/color_list.rs` (`ColorList::move_color`) | ordered named colour lists | host model only | a colour well or picker that uses the lists |
| `springboard/sdk/src/workspace.rs` (tests of `icon_for_file`) | file icons by content type | glyph names used by the kit | drawn per-type icons on Midget |
| `springboard/render/src/clip.rs` (`ClipStack`, `fill_clipped`) | rectangular clip of a drawing context | host model only | `gfx::Surface` draws take no clip yet |
| `springboard/render/src/affine.rs` (`Affine`, `TransformStack`) | affine transform and transform save and restore | host model only | apps apply the transform before drawing |
| `springboard/render/src/video_transfer.rs` (`transfer_i420`, `turn_for_degrees`) | planar pixel transfer and quarter-turn angles | shell video path | an app-facing transfer and rotation session |
| `springboard/layers/src/glass_group.rs` (`group_shapes`) | glass effect container grouping | host model only | the container blend on Midget |
| `springboard/src/bare/app_policy.rs` (`Policy::holds`, `Policy::held`) | entitlement queries | policy checks at launch | an SDK call that apps use to read the answers |
| `springboard/ui/src/promotion.rs` (`visible_in_display_order`) | promoted products that are not owned | host model only | the store page and the promotion view |
| `springboard/ui/src/midi_ble.rs` (`SysExAssembler`) | BLE MIDI system exclusive across packets | host model only | GATT discovery and a live Bluetooth peripheral |

Rows left as they were: view-level drawing (the retained view tree is not the draw cycle of the reference
framework), materials and vibrancy (Spotlight blur needs shader tuning on the lab machine), and semantic
system colours (the remaining names need the reference colour values, which are not in this repository).
