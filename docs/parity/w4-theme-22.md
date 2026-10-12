# Wave 4, theme-22: Custom Virtio drivers (Virtualization, Quartz)

Theme 22 covers six gap rows: five from the Virtualization framework and one from Quartz (the Quick Look embedded
preview). Everything built here is a pure Rust model with unit tests. Nothing talks to hardware, nothing is wired into
the shell or apps, and nothing has been built or run on the VM or Midget. Per the wave rules, no cargo build or test
was run; the tests are written but not executed.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/core/src/virtio.rs` | Virtualization: Custom Virtio drivers (VIRTIO 1.x driver initialization) | Status register bits (ACKNOWLEDGE, DRIVER, FEATURES_OK, DRIVER_OK, FAILED, DEVICE_NEEDS_RESET), the device-type id table, feature negotiation with unknown-bit and required-bit checks, and the ordered handshake. |
| `springboard/core/src/guest_clipboard.rs` | Virtualization: Clipboard sharing | Owner-based grab, request, deliver and release exchange between host and guest. A new grab bumps the generation and refuses late deliveries. Payload limit. |
| `springboard/core/src/guest_usb.rs` | Virtualization: USB Devices | Passthrough policy: first-match rules by vendor, product or class, default deny, hubs never passed through, per-guest attach limit, duplicate and detach checks. |
| `springboard/core/src/guest_audio.rs` | Virtualization: Audio | Guest PCM stream configuration (direction, sample format, rate from the VIRTIO sound list, channels 1 to 8, frame-aligned period and buffer), latency and periods per buffer. |
| `springboard/core/src/guest_display.rs` | Virtualization: VZVirtualMachineView (Displaying a guest in a window) | Aspect-fit and integer-scale placement of the guest picture, window-to-guest pixel mapping, keyboard capture with a release key and focus loss. |
| `springboard/sdk/src/peek.rs` (extended) | Quartz: QLPreviewView, QLPreviewViewStyle | `PreviewView` (embedded, not floating) and `ViewStyle` (normal, compact): item, display state with autostart, refresh count, close, close-with-window switch. |

Each core module is registered with one `pub mod` line in `springboard/core/src/lib.rs`. The SDK module needed no new
line; its tests are a second `#[cfg(test)]` module at the end of `peek.rs`, so the existing tests are untouched.

## Official API names mirrored

- Virtualization (`https://developer.apple.com/documentation/virtualization`): the topics "Custom Virtio drivers",
  "Clipboard sharing", "USB Devices", "Audio", "Graphics", and the class `VZVirtualMachineView`. The official page
  states that the framework supports the VIRTIO specification for network, socket, serial, storage, entropy and
  memory-balloon devices.
- Quick Look UI (`https://developer.apple.com/documentation/quicklookui/qlpreviewview`): `QLPreviewView`,
  `init(frame:style:)`, `previewItem`, `refreshPreviewItem()`, `displayState`, `autostarts`, `shouldCloseWithWindow`,
  `close()`. `QLPreviewViewStyle` has the cases `normal` and `compact` (from
  `https://developer.apple.com/documentation/quicklookui/qlpreviewviewstyle`).
- The VIRTIO status bits, the feature bit VIRTIO_F_VERSION_1 (bit 32) and the device ids come from the VIRTIO
  specification, not from the Apple pages. The sound rate list also comes from the VIRTIO sound specification.
- Limits marked as BifrOSt choices in the code: `MAX_PAYLOAD` (clipboard, 16 MiB), `MAX_ATTACHED` (USB, 8),
  `MAX_BUFFER_BYTES` (audio, 1 MiB). The official pages give none of these numbers.

## REA evidence

- Virtualization landing page: `ev_b468396900bf897e2abe9ceb4452bc2216a17f3f4e2e6f4b42ebbfd76d0ff518`
- Quick Look UI landing page: `ev_5f9e8024677624d4f77b1808b754874801fbec495121c70fe6cddb62f45bb22d`
- QLPreviewView page: `ev_65ae67479831e21d3feb79b4836d9231f5b4745a08249111e11befc7cdba725d`

## Matrix rows closed

These rows move from `gap` to `partial`. Each status is partial, not parity, because each module is a model with no
app-facing SDK call, no shell or service wiring, and no test on a guest.

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| virtualization | Custom Virtio drivers | partial | springboard/core/src/virtio.rs |
| virtualization | Displaying a guest in a window | partial | springboard/core/src/guest_display.rs |
| virtualization | Audio devices | partial | springboard/core/src/guest_audio.rs |
| virtualization | Clipboard sharing | partial | springboard/core/src/guest_clipboard.rs |
| virtualization | USB devices | partial | springboard/core/src/guest_usb.rs |
| quartz | QLPreviewView (embedded preview) | partial | springboard/sdk/src/peek.rs (PreviewView, ViewStyle) |

Note on the USB row: the matrix feature is written "USB devices" in lowercase in the data, and the Apple topic is
"USB Devices". The table uses the matrix spelling.

## Still missing

- Nothing starts a guest, so the virtio handshake, clipboard, USB policy, audio stream and display view have no caller.
- No driver touches a device; the handshake model is only a state machine.
- The clipboard model has no transport (no SPICE-agent or channel code) and the pasteboard SDK is not bridged.
- Display geometry is not drawn; no window in the shell uses `fit` or `Capture`.
- `PreviewView` is not drawn by the shell and no app adopts it.
- Tests are written but not run, per the wave rule.
