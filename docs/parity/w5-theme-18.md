# Parity wave 5, theme 18: Wi-Fi keychain helpers and network byte totals

Theme: "Keychain helpers for Wi-Fi passwords and EAP identities" (5 frameworks, cohesion 0.875 to 0.925).
This round builds two pure models in `springboard/sdk/src`. Both are app-facing names only; no service
call, no hardware access, std only. Neither is wired into the shell or into `springboard/src/net/wifi_config.rs`
yet, and neither has been run on the VM or Midget.

## What was built

| Module | What it models | Official API it mirrors |
|---|---|---|
| `springboard/sdk/src/wifi_secrets.rs` | A store of Wi-Fi secrets per network name and keychain domain: the shared password, the 802.1X username and password, and an identity label. Rules: SSID 1 to 32 bytes, secret 1 to 63 bytes, domain must be User or System. Set replaces; find and delete answer ItemNotFound when nothing is stored; a rule break answers Param. An identity listing per domain, sorted by network name. | CoreWLAN keychain functions: `CWKeychainSetWiFiPassword`, `CWKeychainFindWiFiPassword`, `CWKeychainDeleteWiFiPassword`, `CWKeychainSetWiFiEAPUsernameAndPassword`, `CWKeychainFindWiFiEAPUsernameAndPassword`, `CWKeychainDeleteWiFiEAPUsernameAndPassword`, `CWKeychainSetWiFiEAPIdentity`, `CWKeychainCopyEAPIdentityList`; the domain type `CWKeychainDomain` (none, user, system). |
| `springboard/sdk/src/net_totals.rs` | Byte counters for Wi-Fi and cellular, each split into upload and download. Totals saturate at `u64::MAX`. `since()` gives the change between two readings; a slot that went down counts as zero (interface reset). | MetricKit network metrics: `TotalWiFiUploadMetric`, `TotalWiFiDownloadMetric`, `TotalCellularUploadMetric`, `TotalCellularDownloadMetric`. |

Registration: one `pub mod` line each in `springboard/sdk/src/lib.rs`, after `pub mod wlan;`.

Tests: `wifi_secrets` has 7 behaviour tests and `net_totals` has 5. Expected values are worked by hand from the
rules above (for example, a 33-byte SSID is Param, a 63-byte password is accepted, a 64-byte one is Param, and
`since` of 100 to 250 is 150). Per the wave rules, nothing was built or run.

## REA evidence (official pages as rendered)

| Page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/corewlan/corewlan-functions | ev_5ea76dfbd003c493c57e23eb51649453d0ed5f74c1cc38a3dc28475c4c3333c5 |
| https://developer.apple.com/documentation/corewlan/cwkeychaindomain | ev_b1cd3cc2c99518616997fcde092f4f6df4c3f731189138f7fa2bc406bd6b4fb3 |
| https://developer.apple.com/documentation/metrickit/totalwifiuploadmetric | ev_63b5557608f16f53cbd5dd3a143bec1c34752ba20f496da7cd04907aa44ab3ed |
| https://developer.apple.com/documentation/metrickit/totalcellulardownloadmetric | ev_37f3e3ce6e23e7a00aeb23ddbed806fbb365def44989fc073f1a53851bf48934 |

## Matrix rows closed (status partial)

| Framework slug | Feature (exact, from `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| core-wlan | Keychain helpers for Wi-Fi passwords and EAP identities | `springboard/sdk/src/wifi_secrets.rs` |
| metrickit | Network data metrics (Wi-Fi and cellular upload and download) | `springboard/sdk/src/net_totals.rs` |

## Not closed in this round

- **social**, "Social account credentials" (`SLRequest.account`): needs an account store and a request signer; not a pure model.
- **core-location**, "Location manager and live updates": needs a position source; no hardware access allowed.
- **pdfkit**, "Write operations (save, write options, permissions)": a PDF page writer is a larger job than this theme's budget.
- The Wi-Fi password itself is still kept in `springboard.conf` through `wifi_config.rs`. `wifi_secrets` is the model the shell can move to once the keychain backs it; the migration is not done.
- Official identifiers are not used in the `.rs` files (owner rule). `python3 tools/check-names.py` reports no hits.
