# Wave 5: Core Bluetooth (central role models)

Branch `parity/w5-core-bluetooth`. Pure Rust, std only, no hardware access, no transport. Nothing here is wired
into the shell or into apps yet, so every closed row is `partial`.

## What was built

- `springboard/sdk/src/central.rs` (`bifrost_app::central`): the central role as apps see it.
  - `ManagerState` (Unknown, Resetting, Unsupported, Unauthorized, PoweredOff, PoweredOn) and `AdapterPower`
    (Unavailable, Off, On, the shell's own power report). `ManagerState::from_adapter` maps one to the other.
  - `ScanOptions` (service filter, allow duplicates; default is every advertisement, one report per device) and
    `Scanner`. A scan starts only while the manager is powered on, reports `Discovery` values (address, RSSI, local
    name, listed service UUIDs) through `offer`, and stops when the manager leaves the powered-on state.
  - `Link` and `LinkState` (Disconnected, Connecting, Connected, Disconnecting) for one peripheral. Every transition
    refuses calls that do not apply in the current state. A failed attempt keeps its `BleError`; cancelling a pending
    attempt records `OperationCancelled`.
- `springboard/sdk/src/ble_error.rs` (`bifrost_app::ble_error`): typed errors for apps.
  - `BleError`: the failure cases of the transaction error reference, with `Display` text.
  - `AttCode`: the 17 failure codes of the remote attribute protocol, with `code()` and `from_code()`.
    Code 0 (success) and unassigned values are not errors and map to `None`.

Both use existing pure models rather than duplicating them: `bifrost_ui::advertising` (name and service UUID parsing),
`bifrost_ui::gatt::Uuid`, and `bifrost_core::bt_address::BdAddr`. The module names do not clash with any existing module.

The `AttCode` name is deliberate. `bifrost_ui::gatt_server::AttError` already exists and describes server-side
failures of the local attribute table; `AttCode` is the code a remote server sends back to a client.

## Official API names mirrored

Source: developer.apple.com DocC JSON, read with `tools/parity/docc.py`.

| Official name | BifrOSt |
|---|---|
| `CBManagerState` (`poweredOff`, `poweredOn`, `resetting`, `unauthorized`, `unknown`, `unsupported`) | `central::ManagerState` |
| `CBCentralManager.scanForPeripherals(withServices:options:)`, `stopScan()`, `isScanning` | `central::Scanner::start`, `stop`, `is_scanning` |
| `CBCentralManagerScanOptionAllowDuplicatesKey` | `central::ScanOptions::allow_duplicates` |
| `CBCentralManagerScanOptionSolicitedServiceUUIDsKey` (service filter) | `central::ScanOptions::services` |
| `CBCentralManager.connect(_:options:)`, `cancelPeripheralConnection(_:)` | `central::Link::connect`, `cancel` |
| `CBError` codes (`unknown` to `unknownDevice`, `encryptionTimedOut`, `peerRemovedPairingInformation`, `tooManyLEPairedDevices`, the two background notification limits, the two channel sounding failures) | `ble_error::BleError` |
| `CBATTError` codes (`invalidHandle` to `insufficientResources`) | `ble_error::AttCode` |

Not mirrored: the misspelled `unkownDevice` alias, `CBManagerState` raw values, `retrieveConnectedPeripherals`,
`retrievePeripherals`, `registerForConnectionEvents`, `supports(_:)`, and the Channel Sounding types.

Behaviour notes:
- The default scan options follow the scan-option reference: duplicates are not allowed unless asked for.
- The service filter passes an advertisement that lists at least one requested UUID. The DocC abstract for
  `scanForPeripherals(withServices:options:)` was retrieved, but its parameter text was not, so this rule is taken
  from the scan-option reference and is not separately confirmed.
- The `AttCode` numeric values are the attribute protocol error values of the Bluetooth core specification, not
  Apple's raw values. The case names and their meanings come from the official `CBATTError` reference.
- `Link::cancel` from Connecting records `BleError::OperationCancelled`, matching the meaning of that case in the
  reference (a canceled operation).

## Tests

- `ble_error.rs`: 5 tests (protocol values, success and unassigned values, round trip for 0x01 to 0x11, text, error text).
- `central.rs`: 9 tests (adapter mapping, start only when powered on, one report per device and a new scan forgets
  devices, duplicates allowed, service filter with 16-bit UUID advertisements, leaving powered-on stops the scan, link
  connect and disconnect order, failed attempt keeps its reason, cancel of a pending attempt).

Not run: no builds were made, as the task required.

## REA evidence

- Framework landing page `https://developer.apple.com/documentation/corebluetooth`:
  `ev_fc6015c23b361ae3611568eb1ac243d6723404dc47d6eff8270e108e8e2c1579` (rendered headings: Centrals, Peripherals,
  Data Transfer, Channel Sounding, Services, Supporting Types, Errors, Deprecated).

## Matrix rows closed

Framework slug `core-bluetooth`. Status changes from `os` to `partial`. Feature text is copied from
`docs/parity-matrix/data/core-bluetooth.json`.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-bluetooth | Central manager state and power | partial | springboard/sdk/src/central.rs (ManagerState, AdapterPower) |
| core-bluetooth | Scanning for peripherals | partial | springboard/sdk/src/central.rs (ScanOptions, Scanner) |
| core-bluetooth | Connecting and disconnecting peripherals | partial | springboard/sdk/src/central.rs (Link, LinkState) |

Rows not closed: ATT and Core Bluetooth errors (already `partial`; the typed errors in `ble_error.rs` are a
prerequisite for closing it, and that is left to the matrix lead). Channel Sounding and Background execution remain
`gap`: Channel Sounding needs hardware the repo does not have, and background policy needs the app sandbox work.
