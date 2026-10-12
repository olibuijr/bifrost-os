# Wave 4: Network Extension (hotspot configuration, accessory join)

Branch `parity/w4-network-extension`. Two pure Rust models in the SDK, std only, no unsafe, no hardware access.
Nothing is wired into the shell yet, and nothing was compiled or run in this wave (no builds were permitted).

## What was built

- `springboard/sdk/src/hotspot_configuration.rs`: the Wi-Fi network an app asks the system to remember.
  - `HotspotConfiguration` mirrors the configuration initializers and properties: `open(ssid)` (`init(ssid:)`),
    `with_passphrase(ssid, passphrase, is_wep)` (`init(ssid:passphrase:isWEP:)`), `ssid`, `hidden`, `join_once`,
    `lifetime_days`. `validate` applies the SSID rule (1 to 32 octets) and the WEP and WPA/WPA2 passphrase rules.
  - `HotspotManager` mirrors the configuration manager's add, list and remove calls: `apply(config, allowed_by_user)`
    (`apply(_:completionHandler:)`, the user's answer is passed in as a flag), `configured_ssids`
    (`getConfiguredSSIDs`), and `remove_configuration_for_ssid` (`removeConfiguration(forSSID:)`). A manager removes
    only the networks it holds.
  - `HotspotError` mirrors the error values `invalidSSID`, `invalidWPAPassphrase`, `invalidWEPPassphrase`,
    `userDenied`, `pending`.
- `springboard/sdk/src/accessory_hotspot.rs`: the accessory join, one at a time.
  - `AccessoryHotspotJoin::join` (`joinAccessoryHotspot(_:passphrase:completionHandler:)`) and
    `join_without_security` (`joinAccessoryHotspotWithoutSecurity(_:completionHandler:)`) check the request and move to
    `Joining`. A second join while one runs returns `Pending`.
  - `finish` records the transport's outcome as `Joined` or `Failed`. A failed join frees the accessory for a retry.
- `springboard/sdk/src/lib.rs`: two `pub mod` lines.

Decisions the official pages do not settle, recorded here so the shell can change them:
- The accessory is passed as a name (`&str`). The official call takes an accessory-setup object that BifrOSt does not have.
- `lifetime_days: None` means "until removed". The page does not state the default.
- `configured_ssids` lists only the networks this manager holds. The official page says the list is what the app
  "configured", so this follows it, but the system-wide list is not modelled.
- Hotspot 2.0 and EAP configurations are not modelled. The EAP settings and HS 2.0 settings are separate official classes.

## Official API names mirrored

- NEHotspotConfiguration (init(ssid:), init(ssid:passphrase:isWEP:), ssid, hidden, joinOnce, lifeTimeInDays):
  https://developer.apple.com/documentation/networkextension/nehotspotconfiguration
- NEHotspotConfigurationManager (apply, getConfiguredSSIDs, removeConfiguration(forSSID:), joinAccessoryHotspot,
  joinAccessoryHotspotWithoutSecurity): https://developer.apple.com/documentation/networkextension/nehotspotconfigurationmanager
- NEHotspotConfigurationError (values listed above): https://developer.apple.com/documentation/networkextension/nehotspotconfigurationerror
  (read through `docc.py` only; no REA evidence was recorded for this page).

## REA evidence

- NEHotspotConfiguration: `ev_13a97d5c69eccf2059e75ba28b3ee24a4c0bf547467ea2121297016dc2fcd0dc`
- NEHotspotConfigurationManager: `ev_264ab47142d28703d198b09479fcb6e9997aaf22a02004e0f2d0215177591cbe`

## Matrix rows closed

These are the rows for the lead to update in `docs/parity-matrix/data/network-extension.json`. The matrix is not edited
in this wave.

| framework slug | feature (as written in the matrix) | new status | BifrOSt path |
|---|---|---|---|
| network-extension | Wi-Fi configuration (persistent networks, join) | partial | springboard/sdk/src/hotspot_configuration.rs |
| network-extension | Configuring a Wi-Fi accessory to join a network | partial | springboard/sdk/src/accessory_hotspot.rs |

Still open in this framework: the shell does not yet call these models (`springboard/src/net/wifi_config.rs` keeps its
own shell-only copy), the hotspot helper, the DNS proxy, local push, and the app extension host.

## Not verified

- The code is not compiled and the tests have not been run. The expected values in the tests were worked out by hand
  from the validation rules.
