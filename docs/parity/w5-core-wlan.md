# Wave 5: Core WLAN (app-facing Wi-Fi events and secrets)

Framework: Core WLAN, https://developer.apple.com/documentation/corewlan (official doc, read through `tools/parity/docc.py corewlan`).
Branch `parity/w5-core-wlan`, on top of `parity/integration` (waves 1 to 4).

## What was built

Two pure modules in the SDK. Neither makes a service call, neither needs the shell running, and both use std only.

- `springboard/sdk/src/wlan_events.rs`: typed Wi-Fi change events. The shell sends one status snapshot on each change (`Evt::Status`). `diff` turns two snapshots into events in the order a delegate hears them (network name, link, signal, scan results). `Watch` keeps the last snapshot so an app can feed each new one in. Signal changes under `RSSI_STEP` (2 dBm) are not reported.
- `springboard/sdk/src/wlan_keychain.rs`: Wi-Fi secrets as generic keychain items. It builds the account names (`password:<hex ssid>`, `eap:<hex ssid>`), checks passphrases (8 to 63 printable ASCII, or a 64-digit hex key) and enterprise user names and passwords, encodes and parses the enterprise value (user name, line feed, password), and maintains the enterprise user name list. The caller stores the result with `springboard/sdk/src/keychain.rs` (`add`, `update`). Service name: `bifrost.wifi`.

Both modules are registered in `springboard/sdk/src/lib.rs` with one `pub mod` line each. Each has a `#[cfg(test)] mod tests` block with hand-worked expected values.

## Official API names mirrored

- CWWiFiClient event delegate: CWEventDelegate, ssidDidChange, linkDidChange, scanCacheUpdated (mirrored in the `Event` variants `SsidChanged`, `LinkChanged`, `ScanCacheUpdated`). The Apple delegate also reports bssid, power, mode, country code and link quality changes. Those have no data in the BifrOSt status snapshot, so they are not mirrored.
- Keychain helpers for Wi-Fi: CWKeychainSetWiFiPassword, CWKeychainFindWiFiPassword, CWKeychainSetWiFiEAPUsernameAndPassword, CWKeychainCopyEAPIdentityList (mirrored by `password_item`, `eap_item`, `parse_eap`, `identities`, `add_identity`).

## REA evidence

- `ev_296a757e840c0605e8bb1183bc903a57c3a6b669721fc714c43dd23958731193`: rendered official landing page https://developer.apple.com/documentation/corewlan (title "Core WLAN"). The page states that the client object is the entry point, that a delegate receives event notifications, and that interface objects vended from the client are sandbox safe.
- The wave 4 evidence `ev_e4c4cf24a0143cf25b9a47a6304490eb070f6f73d7ce1519a699dec2eab519d4` (recorded in the matrix) covers the same page.

## Matrix rows closed (status moves to partial)

| framework slug | feature (as written in `docs/parity-matrix/data/core-wlan.json`) | new status | BifrOSt path |
|---|---|---|---|
| core-wlan | Wi-Fi events (CWEventDelegate, CWEventType) | partial | springboard/sdk/src/wlan_events.rs |
| core-wlan | Keychain helpers for Wi-Fi passwords and EAP identities | partial | springboard/sdk/src/wlan_keychain.rs |

Both rows stay partial. The events module covers name, link, signal and scan, but not bssid, power, mode, country code or link quality. The keychain module builds and checks items, but nothing in the shell or the Settings page uses it yet. Passwords still live in `springboard.conf` (`springboard/src/net/wifi_config.rs`) until the shell migrates them.

## Not done

- The shell and netd do not call these modules. No app-facing subscription or call path exists for Wi-Fi yet (the `os` rows "Client and shared instance", "Interfaces", "Join, select and forget" stay as they are).
- No build, test run or Midget check was made in this round, per the wave rules. The modules are checked by reading the code and by hand-computed test values only.
