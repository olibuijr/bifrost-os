# Core WLAN parity, wave 4

Official reference: https://developer.apple.com/documentation/corewlan (the framework page and its class pages).

## What was built

`springboard/sdk/src/wlan.rs` (`bifrost_app::wlan`), a pure std model of scan data for apps. It takes what the
system layer already reads from the supplicant's scan (frequency, signal, flag text, BSSID, SSID) and adds:

- `Band::from_freq` and `channel_for_freq`: the 2.4, 5 and 6 GHz bands and the channel number of a centre frequency.
  Channel width is not in the scan flags, so it is not decoded.
- `Phy::from_flags` and `Phy::label`: the highest PHY generation the flags name (ax, ac, n), otherwise a or b/g by band.
- `SecurityClass::from_flags`: Open, WEP, WPA personal, WPA2 personal, WPA3 personal, WPA2/WPA3 transition and
  enterprise. This adds the WEP and WPA-only classes that the shell's `Security` enum does not carry.
- `merge` and `Network`: access points grouped by SSID, hidden (empty SSID) ones dropped, strongest network first, with
  the strongest access point's channel, band, PHY and security.

No service call, no unsafe code and no new crate. The module is not wired to netd yet; apps have no client for it.

## Official API names mirrored

- CWChannel: `channelNumber`, `channelBand`, `channelWidth` (channel number and band are mirrored; width is not).
- CWNetwork: `ssid`, `bssid`, `rssiValue`, `wlanChannel`, `supportsSecurity(_:)`, `supportsPHYMode(_:)` (mirrored as
  fields and `SecurityClass` / `Phy` rather than method queries).
- CWMergeNetworks (CoreWLAN Functions collection): merging networks that share an SSID.
- CWSecurity and CWPHYMode enumerations: the classes and PHY modes listed above.

## REA evidence

- Framework page: `ev_67b7bcb1e3ec23f0c794af473e8dfce0a50b042800b5363f1e92702532814cf4`
- CWChannel: `ev_ab1915b0e45953922a999af15747ca3e5796abe22f3188cf2cde5a5f3ffe5c07`
- CWNetwork: `ev_ba6091cf7f507211ddd009a4a19703234de6ae9799c8dd0ef19bd80854e2d9e8`

## Matrix rows closed

| framework | feature | new status | BifrOSt path |
|---|---|---|---|
| Core WLAN | Channel, band, width and PHY mode (CWChannel, CWChannelBand, CWChannelWidth, CWPHYMode) | partial | springboard/sdk/src/wlan.rs (Band, channel_for_freq, Phy) |
| Core WLAN | Network merge and identity utilities (CWMergeNetworks, CWKeychainCopyEAPIdentityList) | partial | springboard/sdk/src/wlan.rs (merge, Network); no EAP identity list |
| Core WLAN | Security types (CWSecurity) | partial | springboard/sdk/src/wlan.rs (SecurityClass::from_flags) |

## Still open

- Channel width (CWChannelWidth) needs the HT/VHT/HE operation fields, which the scan flags do not carry.
- Keychain helpers for Wi-Fi passwords and EAP identities are not built.
- No app-facing client for the netd Status event, and no typed event delegate.
- Access-point and IBSS configuration are not covered.

## Verification

Not built or run, per the wave rules. The hand-worked expectations are in the `tests` module of `wlan.rs`.
