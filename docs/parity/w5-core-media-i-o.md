# Wave 5: Core Media I/O (camera devices, streams, properties)

Branch `parity/w5-core-media-i-o`, on top of `parity/integration` (waves 1 to 4).

## What was built

The matrix rows for Core Media I/O are about camera devices, streams and properties. The Linux camera path
(`springboard/src/media/v4l2.rs`) can already find and stream capture nodes, but only the shell's Camera app
uses it. This wave adds the app-facing pure models that the system layer can fill in and that apps can read
through the SDK's `controls` re-export (`bifrost_app::controls`, which is `bifrost_ui`).

Two new pure modules, no hardware access, std only:

- `springboard/ui/src/video_device.rs`: capture node usability (the same capability-bit rule as
  `v4l2::usable`), pixel formats (YUYV, NV12, NV21, MJPG and any other four-character code), frame and row
  sizes for each format, the format choice for a stream (rank first, then the nearest size, then the larger
  frame), the device choice (lowest-numbered usable `/dev/videoN`), and `plan_stream`, which returns the
  node, the format, four buffers and the bytes per buffer.
- `springboard/ui/src/video_property.rs`: a property store for a camera provider, device or stream. Each
  property has a value and attributes (read-only, an inclusive range, or a list of valid integers). Writes
  are refused for unknown keys, read-only properties, a value of another kind and out-of-range values; a
  refused write keeps the old value.

Both modules are registered in `springboard/ui/src/lib.rs` (`pub mod video_device;`, `pub mod video_property;`).
Neither is wired into the shell or the apps yet. The bridge from `v4l2::probe` to `VideoDevice` values is not
written, so no app can see a real camera through these models yet.

## Official API names mirrored

Source: developer.apple.com DocC pages (`python3 -I tools/parity/docc.py coremediaio`, and the pages below).

- `CMIOExtensionDevice`: a device has a localized name, an identifier and streams. Mirrored as `VideoDevice` (path,
  card name, capability words, formats) and `select_device`.
- `CMIOExtensionStream` and `CMIOExtensionStreamFormat`: a stream has a format with a size and frame durations.
  Mirrored as `StreamFormat`, `choose_format` and `StreamPlan`. Frame durations are not modelled.
- `CMIOExtensionProperty`, `CMIOExtensionPropertyState`: a property is a key with a value and its state.
  Mirrored as `PropertyStore`, `PropertyState` and `Value`.
- `CMIOExtensionPropertyAttributes` (`init(minValue:maxValue:validValues:readOnly:)`, `isReadOnly`, `minValue`,
  `maxValue`, `validValues`): mirrored as `Attributes`, with the same four fields.

Not mirrored: providers and their source protocols, the device and stream source protocols, clients, the info
dictionary key and the mach service name key, and the DAL plug-in model. The matrix rows for those stay open.

## REA evidence

- Framework page, Core Media I/O: `ev_fc9e742bd72ae4a9dcb3e734b0905ac7056645e7864fe2a4049708ffa092f9af`
- Class page, CMIOExtensionDevice: `ev_84bbcb71fd8c5bbd9427734c24ed4dd7430b23a4f69519e1562bb676195d6242`
- Class page, CMIOExtensionStreamFormat: `ev_d05ee3df6f7f29945909709a6f61290d771ea140b1c5de521b2cc5b42b39348f`
- Class page, CMIOExtensionPropertyAttributes: `ev_de5f65b432698de1d05cdd4a1eb02e75e9d83e8e2e3ff3014114fdf92a4f7bbf`

## Matrix rows closed

Framework slug `core-media-i-o`. Feature names are copied from `docs/parity-matrix/data/core-media-i-o.json`.

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| core-media-i-o | Devices (device, device source, device properties) | partial | springboard/ui/src/video_device.rs (VideoDevice, select_device); discovery stays in springboard/src/media/v4l2.rs |
| core-media-i-o | Streams (stream, stream source, stream properties) | partial | springboard/ui/src/video_device.rs (StreamFormat, choose_format, plan_stream); streaming stays in springboard/src/media/v4l2.rs |
| core-media-i-o | Properties (property, state, attributes, info and mach service keys) | partial | springboard/ui/src/video_property.rs (PropertyStore, Attributes); info and mach service keys not modelled |

Rows not closed: Providers (gap), Clients and per-client access (partial, no change), USB video class override
(n/a), Legacy DAL plug-ins (n/a).

## Gaps still open

- No bridge from `v4l2::probe` and `Device::open` into `VideoDevice` values, and no SDK call that returns them.
- The app sandbox does not grant `/dev/video` nodes (`springboard/src/bare/app_policy.rs`).
- No virtual camera publishing (providers), and no per-client permission check.
- Frame durations and frame-rate choice are not modelled.

## Checks

- `python3 tools/check-names.py`: exit 0, no hits.
- Tests: `video_device.rs` has 7 tests and `video_property.rs` has 7 tests, all with expected values worked out
  by hand from the rules (for example YUYV at 640x480 is 614400 bytes, a fourcc of YUYV is 0x56595559).
  Per the task rules, nothing was built or run.
