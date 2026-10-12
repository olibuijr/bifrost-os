# Wave 5: HIDDriverKit and Core HID parity (device switches and capability matching)

## What was built

Two pure models in `springboard/ui/src` (bifrost-ui, std only, no hardware access). Neither is wired into
the shell or the SDK yet; both have host unit tests only and have not been run on the VM or Midget.

- `springboard/ui/src/input_switches.rs`: the switch bitmap of an input device (lid, tablet mode, headphone
  and microphone insert). `SwitchState::update` takes a new bitmap and returns the switches that changed,
  in code order, as `SwitchEdge` values. Helpers: `lid_closed`, `headset_plugged`, `edge_for`.
- `springboard/ui/src/device_match.rs`: capability-based device matching. `Capabilities` records the event
  types, key codes and absolute axes of a device; `Criteria` requires types, keys and axes and excludes
  types. Presets: `Criteria::keyboard()`, `Criteria::pointer()`, `Criteria::touch()`. `Criteria::select`
  returns the matching positions.

Apple names mirrored (official documentation):

- Core HID framework, https://developer.apple.com/documentation/corehid
- `HIDDeviceManager.DeviceMatchingCriteria` (matching criteria used to filter HID devices), listed under
  https://developer.apple.com/documentation/corehid/hiddevicemanager
- `HIDDeviceManager.init()` and `monitorNotifications(matchingCriteria:)`, the match-and-watch pair.

Evidence: REA `ev_b2ae096e565df9be90179a02e80f2170be15eb4946bbcde1230df07ff39ad36d` (rendered Core HID
landing page, read through `tools/parity/rea-page.sh`). The DocC symbol page for `hiddevicemanager` was
read with `tools/parity/docc.py`; a REA run for the matching-criteria page was not made.

Not mirrored: HID usage pages, transport and localization codes, report elements and virtual devices.
The `*Criteria` types match by evdev event type and code, not by usage page, so the matching model is a
capability filter, not a Core HID-compatible criteria dictionary.

## Matrix rows closed

| Framework | Feature (as written in data/hiddriverkit.json) | New status | BifrOSt path |
|---|---|---|---|
| HIDDriverKit | Switches (lid, jack insert) | partial | springboard/ui/src/input_switches.rs |
| HIDDriverKit | Device discovery and matching by capability | partial | springboard/ui/src/device_match.rs |

Both rows stay partial: the models are not yet exposed to apps through `springboard/sdk/src`, and the
shell still reads the bitmaps itself (`springboard/src/bare/input.rs`, `springboard/src/media/route.rs`).

## Tests

`cargo` was not run (no-build rule for this wave). The tests are in each module and were checked by
hand against the rules the code implements, for example: a lid and microphone bitmap of `0b10001` changed
to `0b00100` gives edges on codes 0 (off), 2 (on) and 4 (off); the keyboard preset rejects a device with
only the A key; the touch preset needs both multitouch position axes.
