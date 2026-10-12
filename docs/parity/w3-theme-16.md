# Wave 3, theme 16: accessory manager

Theme: "Accessory manager" (size 19 rows, frameworks Accessory Access, External Accessory, CoreAudioKit, Core MIDI,
Core Bluetooth, Core Location, Matter, Game Controller). This pass builds two pure models and closes the rows they
cover honestly. Most of the theme is hardware or Apple services and stays open.

## What was built

1. `springboard/core/src/accessory_manager.rs` (module `bifrost_core::accessory_manager`)
   - `AccessoryManager`: attach and detach by registry id, connection ids that are never reused, detached records
     that keep their identity, listeners keyed by a caller number with matching criteria, queued `Event::Connected` /
     `Event::Disconnected` events, exclusive `open` and owner-only `close`, and `session` for a declared protocol.
   - `Accessory`: registry id, USB device and configuration descriptors (from `usb_descriptors`), manufacturer
     strings (name, manufacturer, model, serial, firmware, hardware), declared protocol strings, `connection_id()`
     that reads `None` once disconnected.
   - `Error`: `ListenerAlreadyRegistered`, `NotAccessible`, `InvalidState`, `Internal`.
   - Reuses the USB matching criteria in `springboard/core/src/usb_descriptors.rs` (`MatchingCriteria`).
   - Session data streams are not modelled.
2. `springboard/ui/src/controller_power.rs` (module `bifrost_ui::controller_power`)
   - `Battery` with `BatteryState` (unknown, discharging, charging, full), level 0 to 1 or unreported, percentage
     clamp and rounding.
   - `Color` with normalised components, clamping (NaN is 0), and 8-bit conversion with rounding.
   - `Light`, a colour holder for a controller light.
   - No HID report decoding: the caller passes in the decoded values.

Each module has a `#[cfg(test)] mod tests` with behaviour tests worked out by hand (9 in the core module, 6 in the UI
module).

## Apple API mirrored (official documentation, DocC)

- AccessoryAccess: `AAUSBAccessoryManager` (shared, registerListener with matching criteria, unregisterListener,
  events), `AAUSBAccessory` (open, close, registry id, descriptors), `AAUSBAccessoryListener` (usbAccessoryDidConnect,
  usbAccessoryDidDisconnect), `AAError` (accessoryListenerAlreadyRegistered, accessoryNotAccessible,
  invalidAccessoryState, internalError).
  Source: https://developer.apple.com/documentation/accessoryaccess
- External Accessory: `EAAccessoryManager` (connectedAccessories, connect and disconnect notifications),
  `EAAccessory` (name, manufacturer, modelNumber, serialNumber, firmwareRevision, hardwareRevision, protocolStrings,
  isConnected, connectionID), `EASession` (init(accessory:forProtocol:), protocolString, accessory).
  Source: https://developer.apple.com/documentation/externalaccessory
- Game Controller: `GCDeviceBattery` (batteryLevel, batteryState, State with unknown, discharging, charging, full),
  `GCDeviceLight` (color), `GCColor` (red, green, blue, normalised 0 to 1).
  Source: https://developer.apple.com/documentation/gamecontroller

Name mapping in the code: the framework's names are not used in code (owner rule). The mapping is: listener object
→ `key`, `AAError` → `Error`, connection ID → `Accessory::id` / `connection_id()`, protocol strings →
`protocol_strings`, `EASession` → `Session`, `GCColor` → `Color`.

## REA evidence (rendered official pages)

- `ev_14276a0883f89503696b23f9e2da9da2b4c241c5a326374d32cdab7aabf12199`: AAUSBAccessoryManager (listener delivery,
  exclusive access, the accessory manager's UI rule)
- `ev_807004d038c691e720e1076616d75a7d2122ced001a2f49e17ec677c27a1496d`: EAAccessoryManager
- `ev_108a6b8eb9e72ce7b82eb811b4b606ff09657408e5a4d09b12374e621bc22d28`: GCDeviceBattery

## Matrix rows closed (status partial)

| Framework slug | Feature (exactly as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| accessory-access | Accessory manager | partial | springboard/core/src/accessory_manager.rs (AccessoryManager) |
| accessory-access | Accessory object | partial | springboard/core/src/accessory_manager.rs (Accessory) |
| accessory-access | Listener events (attach and detach) | partial | springboard/core/src/accessory_manager.rs (register, take_events, Event) |
| accessory-access | Errors | partial | springboard/core/src/accessory_manager.rs (Error) |
| accessory-access | Permission and access grant for USB devices | partial | springboard/core/src/accessory_manager.rs (open, close, criteria on register) |
| external-accessory | Accessory manager and connect/disconnect notifications | partial | springboard/core/src/accessory_manager.rs (AccessoryManager, connected_accessories) |
| external-accessory | Accessory object (identity and state) | partial | springboard/core/src/accessory_manager.rs (Accessory identity fields, is_connected) |
| external-accessory | Protocol matching (declared protocols) | partial | springboard/core/src/accessory_manager.rs (session, supports_protocol) |
| game-controller | Battery and light on controllers | partial | springboard/ui/src/controller_power.rs (Battery, Light, Color) |

## Not closed, and why

- "Data session streams" (External Accessory): a session is modelled, but there are no input or output streams.
- "Matching criteria" and "Kernel USB enumeration and system visibility" (Accessory Access): the criteria type is
  already in `usb_descriptors.rs`, and kernel enumeration is outside a pure model.
- Core MIDI, CoreAudioKit, Core Bluetooth, Core Location, Matter rows: each needs a device, a service or a
  network stack. No pure model of these was built in this pass.
- The model is not wired into the shell, the SDK or any app. The USB hot-plug source, the app-facing SDK
  type and the controller battery read from HID reports are separate steps.
- `AAError.internalError` has no clear use in the model. `Internal` is returned for an unknown listener key, which
  the framework does not name, so this mapping is a guess.
