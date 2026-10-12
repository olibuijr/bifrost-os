# Wave 5, theme 19: Safari extensions, Wi-Fi accessory configuration, WebDriver, file-manager extension controller

Theme: "Safari web extensions and Safari app extensions" (`theme-19`, size 4). The theme's four matrix rows span four
frameworks. This round builds two of the gap rows as pure models and leaves two open, with reasons below. No builds or
tests were run in this round (`cargo` was not run).

## What was built

- `springboard/sdk/src/wifi_accessory_browser.rs` (new, `pub mod` line in `springboard/sdk/src/lib.rs`): an app-facing
  browser for unconfigured Wi-Fi accessories. It is the discovery stage in front of `accessory_hotspot`. Rules: a search
  and a configuration need the app's entitlement (a flag); a search clears the previous list; only matching accessories
  are kept, one row per MAC address; stopping a search keeps the list; one configuration runs at a time and only for a
  discovered accessory; a configured accessory leaves the list and a failed one stays. Includes `normalise_mac` (six hex
  pairs, stored lowercase). 7 behaviour tests.
- `springboard/ui/src/file_sync.rs`: `ExtensionEnablement`, a facade over the existing `extensions::Registry` under the
  point `file-manager-sync`. It reuses the registry's per-user choice store, so there is no second enablement store. It
  offers `declare`, `is_extension_enabled`, `set_extension_enabled` and `management_list` (enabled and disabled rows,
  sorted by title). Off until the user enables it; a choice made before a declaration applies when the declaration
  arrives; redeclaring keeps the choice. 5 behaviour tests in `enablement_tests`.

## Official API names mirrored

- `EAWiFiUnconfiguredAccessoryBrowser`: `init(delegate:queue:)`, `startSearchingForUnconfiguredAccessories(matching:)`,
  `stopSearchingForUnconfiguredAccessories()`, `unconfiguredAccessories`, `configureAccessory(_:withConfigurationUIOn:)`.
  Source: https://developer.apple.com/documentation/externalaccessory/eawifiunconfiguredaccessorybrowser (DocC via
  `tools/parity/docc.py externalaccessory/eawifiunconfiguredaccessorybrowser`).
- `EAWiFiUnconfiguredAccessory`: `name`, `manufacturer`, `model`, `ssid`, `macAddress`. The `properties` option set is
  not modelled. Source: `externalaccessory/eawifiunconfiguredaccessory` (DocC).
- `FIFinderSyncController`: `isExtensionEnabled`, `showExtensionManagementInterface()`. Source:
  https://developer.apple.com/documentation/findersync/fifindersynccontroller (DocC via `findersync/fifindersynccontroller`).
  The model keeps enablement per (app, identifier), not as one controller-wide flag, and `management_list` returns the rows
  a management sheet would show; it does not open a sheet.

## REA evidence

| Framework | Official page | REA evidence id |
|---|---|---|
| External Accessory | https://developer.apple.com/documentation/externalaccessory | `ev_ed909b84313c63aa183d32bc2171ee7fc3973824f2653482b62d6b3a824167dd` |
| Finder Sync | https://developer.apple.com/documentation/findersync | `ev_46698b73ebfd4600a08f78f1f5f38f4ab5d9681338eb257a8ae9732794aaeece` |
| WebKit (WebDriver section) | https://developer.apple.com/documentation/webkit | `ev_0bf0c764b4ab155fdc8d877ed004d1759c91e283cbb0bb63f660774c114041f2` |
| Safari Services | https://developer.apple.com/documentation/safariservices | `ev_bd7edc2b2a4706d152772948adb6a55e835b08b8aed87917c56d8c91e0de9e57` |

The External Accessory page states that the Wi-Fi accessory configuration entitlement and browser are a topic of that
framework. The Finder Sync page describes the controller as the bridge between the extension and the file manager.

## Matrix rows closed (new status `partial`)

| Framework slug | Feature (as written in `data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| external-accessory | Wi-Fi accessory configuration (unconfigured accessories) | partial | `springboard/sdk/src/wifi_accessory_browser.rs` |
| finder-sync | Extension controller and enablement | partial | `springboard/ui/src/file_sync.rs` (`ExtensionEnablement`) |

## Rows left open, and why

- `safari-services`, "Safari web extensions and Safari app extensions" (gap): this needs an extension host, page and tab
  proxies and extension messaging. Those are a larger piece than this round, so the row stays `gap`. The Safari
  extension class page was read (`safariservices/sfsafariextension`) but nothing was built from it.
- `webkit`, "WebDriver and automated testing" (gap): the Apple WebDriver pages (`webkit/testing-with-web-driver-in-safari`,
  `webkit/mac-os-web-driver-commands-for-safari-12-and-later`) returned HTTP 404 from `tools/parity/docc.py`, so no
  official command list could be read. No WebDriver command model was built, since that would rest on memory.
  REA evidence for the WebKit landing page was recorded, and it names the WebDriver section.

## Gaps that remain in the closed rows

- Accessory properties (the `EAWiFiUnconfiguredAccessoryProperties` bits) are not modelled.
- The configuration UI hand-off (`configureAccessory(_:withConfigurationUIOn:)`) is not modelled. `configure` only records
  the choice; there is no transport and no sheet.
- The entitlement is a constructor flag. It is not read from a manifest or the sandbox.
- Nothing in the shell or the Files app calls `ExtensionEnablement`. The management list is not shown anywhere.
- The modules are pure models, not wired into apps or the shell, and not tested on the VM or Midget.
