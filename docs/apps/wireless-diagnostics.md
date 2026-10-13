# Wireless Diagnostics

Wireless Diagnostics (`springboard/apps/wireless-diagnostics-desktop`, crate `bifrost-wireless-diagnostics`, version
0.1.0, id `is.olibuijr.wirelessdiagnostics.desktop`) shows the state of the Wi-Fi link and its radio environment in one
full-size window.

## What it shows

- **Connection card.** The network name, the access point (BSSID), channel, band, signal, noise floor,
  signal-to-noise ratio, a quality rating (Excellent, Good, Fair, Poor, Unknown), the receive and transmit rates, and
  the interface name. The rating follows the signal-to-noise ratio when the noise floor is known, and the signal alone
  when it is not.
- **Signal chart.** One column per reading for the last two minutes (120 readings, one per second), newest at the
  right, with guide lines at -40, -60 and -80 dBm. A poll that finds no signal leaves a gap.
- **Nearby networks.** The result of the last scan, merged by network name and sorted strongest first. Each row gives
  the channel, band, security and strongest signal. Security comes from the scan's RSN and WPA blocks: WPA3 (SAE),
  WPA2 (PSK), WPA2/WPA3 transition, WPA, WEP, enterprise (802.1X) or none.
- **Footer.** The result of the last action, else the reason the radio cannot be read, else the scan state, else the
  connection.

## Where the data comes from

- Every second a background poller reads `/proc/net/wireless` for the interfaces and the signal, noise and link
  quality, then runs `iw dev <interface> link` for the connection. The first wireless interface is used. When the kernel
  lists none, `wlan0` is tried and the failure is shown in the footer.
- Scan runs `iw dev <interface> scan` on a background thread. That needs the network privilege. When it is refused, the
  app falls back to `iw dev <interface> scan dump` (the cached results) and says so.
- The parsers live in `src/model.rs` and are pure: they take the text and return values, so they are unit tested
  without a radio.

## Toolbar

- **Scan** starts a scan. It is greyed while one runs.
- **Save Report** writes a plain-text report to `reports/wireless-diagnostics-<unix time>.txt` in the app's data folder.
  The report holds the connection, the quality, the signal history summary and the nearby networks.
- **Copy Summary** puts the same report text on the clipboard.

## Permissions and runtime

The manifest asks for `network` (the `iw` netlink calls need the host network namespace) and `storage` (reports).
The app keeps no unsaved state and enables sudden termination.

## Code layout

| File | Role |
|---|---|
| `src/model.rs` | Parsing of `/proc/net/wireless`, `iw link` and `iw scan`; rating; chart history; UTC stamp; report text |
| `src/source.rs` | Runs `iw` and reads `/proc`; the poller and scan threads |
| `src/state.rs` | Window state, toolbar actions, footer text, layout in points, report saving |
| `src/draw.rs` | Painting with the SDK kit (`widgets::toolbar`, semantic colours, text cache) |
| `src/main.rs` | The `App` implementation: touches, activation, tick, draw |

The scan parser uses `bifrost_app::wlan` (`Sample`, `merge`, `SecurityClass`, `channel_for_freq`), so the security
and channel rules match the rest of the system.

## Not in this version

- Choosing between several wireless interfaces (the first one is used).
- Charts for the rates, and a history beyond two minutes.
- Accessibility nodes for the cards (`App::controls`); the toolbar is the only published control.
- A compliance snapshot: `tools/gen-app-compliance-tests.py` should generate the `tests/compliance.rs` gate once the
  crate is in the workspace.
