# Changelog

## 0.1.0

- New: Wireless Diagnostics. The Connection card shows the network, BSSID, channel, band, signal, noise, signal-to-noise ratio, a quality rating, and the receive and transmit rates, read from the kernel (`/proc/net/wireless`) and `iw` every second.
- New: a live signal chart of the last two minutes, with a grid at the usual signal levels.
- New: Scan runs a Wi-Fi scan in the background (falls back to the cached results when a fresh scan is refused) and lists nearby networks by signal with channel, band and security.
- New: Save Report writes a plain-text diagnostics report to the app's data folder, and Copy Summary puts the same text on the clipboard.
- New: the pure parsing, quality rating, history and report code is tested.
