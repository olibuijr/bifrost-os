# Wi-Fi reliability (slice wifi-reliability)

Daily-use network behaviour on BifrOSt PC: reconnects, link problems in the menu bar, the captive probe address, and
scans that never block the UI. No matrix rows were closed by this slice; the matrix is the lead's to update.

## Done

- **Reconnect after resume** (`src/net/wifi.rs`, desktop tick): the wall clock runs through a suspend and the monotonic
  clock does not. A gap over `SLEEP_GAP` (5 s) between two ticks is a sleep (`wifi::slept`). On wake the manager asks the
  supplicant to `RECONNECT` and renews DHCP at once (`renew_due`), instead of waiting for the old lease to run out.
- **Reconnect on signal loss** (`src/net/wifi.rs`, poll): a link that was joined and has been down for `RECONNECT_AFTER`
  (10 s) gets a `RECONNECT`, at most every `RECONNECT_EVERY` (20 s) (`wifi::should_reconnect`). Short dips are left to
  the supplicant.
- **Link problem in the menu bar** (`src/shell/desktop_network.rs`): a connected Wi-Fi link with no IPv4 address shows
  "No IP Address"; one with no DNS server, or whose check name (`connectivitycheck.gstatic.com`) does not resolve, shows
  "DNS Not Responding". The DNS answer is cached 30 s. `LinkProblem::label` is the text; `src/bin/shell.rs` shows it as
  the Wi-Fi menu-bar text and as the Wi-Fi menu notice when no other error is set.
  Netd path (`wifi.rs`): a DHCP failure already sets the "No Internet Connection" note, which the menu bar shows.
- **Configurable captive probe** (`src/net/captive.rs`): the default is the generic
  `http://connectivitycheck.gstatic.com/generate_204` (`DEFAULT_PROBE_URL`, was a personal host). The `captive_url` key in
  settings.ini overrides it when it is a plain `http://` URL (`captive::probe_url`); it is read per probe.
- **Wi-Fi requests do not wait on the debounce** (`desktop_network.rs` worker): a signal-driven re-read used to sleep up
  to 8 s, delaying every join, select or scan request queued behind it. The debounce is now a deadline; requests are
  served at once.
- **Stalled clients on netd** (`netd/src/main.rs`): subscriber writes happen under the core lock, so the write timeout
  dropped from 500 ms to 50 ms; a stalled client is dropped instead of holding up every other client.

## Still a model or needs checking on the lab PC

- No Settings row edits `captive_url` yet; it is read from settings.ini only.
- On the NetworkManager desktop path, resume reconnects are NetworkManager's own autoconnect; the shell only reflects
  them through the `dbus-monitor` signals.
- The DNS check is a blocking lookup on the desktop worker thread; on a dead resolver it can take several seconds
  before the menu bar updates (the UI itself does not wait).
- DHCP on the netd path still runs on the Wi-Fi manager thread for up to 15 s; scan and join commands queue behind it.
- A wall-clock step from NTP looks like a sleep to the detector and triggers a harmless `RECONNECT` and DHCP renewal.
- Needs a real suspend/resume on the lab PC and a walk-out-of-range test to confirm the timings.

## Evidence

- Code read: `springboard/src/net/wifi.rs`, `springboard/src/net/captive.rs`, `springboard/src/shell/desktop_network.rs`,
  `springboard/netd/src/main.rs`, `springboard/src/bin/shell.rs` (wifi info and probe call sites).
- Existing text reused: `wifi_event::NO_INTERNET`, the wifi menu notice row in `src/shell/pages.rs`.
