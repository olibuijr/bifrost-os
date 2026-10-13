# Wi-Fi and Bluetooth live, Captive Network Assistant (slug `wifi-bt-captive`)

Round 75. Code and commits only, not built or run by the agent; the lead builds, tests on Midget and merges.

## Evidence used

- Reference index consulted for bundle names only: `~/Projects/macos-ref/extract/ref27-index.tsv`
  (`System/Library/CoreServices/BluetoothSetupAssistant.app`, `System/Applications/Utilities/Bluetooth File Exchange.app`,
  `System/Applications/Utilities/AirPort Utility.app`). No REA decode was run for this slice, so no REA evidence ids
  are recorded. Behaviour comes from the existing BifrOSt code and the Bluetooth management interface it already speaks.
- Existing code read: `src/net/wifi.rs` (Wi-Fi thread, `Cmd`, `Status`), `src/bare/bluetooth.rs` (mgmt socket, `Cmd`,
  `Device`, `Status`), `src/shell/desktop_bar.rs` (menu bar extras and menus), `src/shell/settings.rs` (Wi-Fi and
  Bluetooth panes, already wired to `Effect::Wifi` and `Effect::Bluetooth`), `src/bin/shell.rs` (info tick).

## Rows

### Wi-Fi join, select, forget (Settings panes and the Wi-Fi menu)

- BifrOSt path: `src/net/wifi.rs` (`Cmd::Join`, `Select`, `Forget`, `SetAutoJoin`, `Move`, `Enable`), Settings Wi-Fi
  pane in `src/shell/settings.rs` and `src/shell/wifi_page.rs`, desktop Wi-Fi menu in `src/shell/desktop_bar.rs`
  (`Menu::Network`).
- Done before this round: join, select, forget with confirmation sheet, auto-join, priority, enable. Settings and
  the Wi-Fi menu send these through `Effect::Wifi`.
- Still open: the Wi-Fi menu has no Forget row (the Settings pane confirms before forgetting, the menu does not
  offer it). Not done in this round.
- Still needs Midget check: join and forget from the menu on the desktop path (`desktop_network` through nmcli).

### Bluetooth pair, connect, disconnect (menu bar)

- BifrOSt path: `src/bare/bluetooth.rs` (`Cmd::Enable`, `Scan`, `Pair`, `Connect`, `Disconnect`, `Forget`), menu
  extra `Menu::Bluetooth` in `src/shell/desktop_bar.rs`.
- Done: a Bluetooth menu extra, shown once the radio is available. Rows: Turn Bluetooth On or Off; each paired or
  connected device (a checked row is connected; picking it disconnects, picking an unchecked one connects); a
  "Bluetooth Settings…" row that opens the Settings Bluetooth pane. Scanned, unpaired devices stay in the pane,
  where pairing lives. Busy devices show as a disabled label with an ellipsis.
- Test: `bluetooth_menu_lists_paired_devices_and_connects_or_disconnects_them` in `src/shell/tests.rs`.
- Still open: pairing from the menu (only the Settings pane pairs). Glyph is a small rune drawn with dots, not
  the reference symbol.
- Still needs Midget check: the glyph's look on the bar in Light and Dark, and a real connect and disconnect.

### Captive Network Assistant

- BifrOSt path: `src/net/captive.rs` (probe, classification, per-link bookkeeping), shell wiring in
  `src/bin/shell.rs` (info tick), `Info.captive_portal` in `src/shell/mod.rs`, Wi-Fi menu row "Sign In to <network>…"
  in `src/shell/desktop_bar.rs` (`Action::Portal`).
- Done:
  - After a link comes up with an address, one plain HTTP GET goes to `captive::PROBE_URL`
    (`http://connectivity.olibuijr.com/generate_204`), on its own thread, 4 s timeout.
  - 204 is open access. A redirect (3xx) or any 2xx other than 204 is a sign-in page; the redirect target (or the probe
    address) is the page. Errors and silence are offline and never open anything.
  - The sign-in page opens once per network in the browser (`desktop_open_url`). The Wi-Fi menu keeps a
    "Sign In to <network>…" row while the page is known. Going offline forgets it, and a return opens it again.
- Decision: the assistant is not a separate native window. It opens the browser view and offers the sign-in row in the
  Wi-Fi menu. A separate window would need its own drawing pass; this slice did not build one.
- Open: the probe host `connectivity.olibuijr.com` must be deployed and answer `204` for the probe to say "online";
  until then the probe reports offline and nothing opens (safe default). The probe is plain HTTP only; HTTPS
  probes are not attempted. No re-probe after a user signs in: the row goes away on the next link change.
- Tests: `src/net/captive.rs` (reply parsing, classification, redirect targets, URL splitting, assistant lifecycle).
- Still needs Midget check: a real portal network (open the page once, row stays), and an open network (no
  page opens).

### netd

- The shell still drives Wi-Fi in-process through `springboard::net::wifi::Wifi`, the same as before this round. The
  `netd` service (`netd/src/main.rs`) is not on the shell's path, so Wi-Fi commands from the menu and Settings do not
  go through it. Moving the shell onto netd is a larger change and was not part of this slice.

## Shared-file edits (merge with care)

- `springboard/src/net/mod.rs`: one line, `pub mod captive;`.
- `springboard/src/shell/mod.rs`: one field on `Info`, `captive_portal: Option<String>`. `Info` derives `Default`, so the
  examples that use `..Info::default()` still build.
- `springboard/src/bin/shell.rs`: the captive assistant is created after the Bluetooth thread; `captive_portal: None`
  in the `gather` literal; a probe and open block in the info tick before `shell.set_info(info)`.
- `springboard/src/shell/desktop_bar.rs`: `Menu::Bluetooth` and `Action::Bluetooth`/`Action::Portal`, a Bluetooth arm in
  `own_menu_rows`, the extra in `menu_buttons`, the glyph branch in `draw_status_glyph`, the Sign In row in
  `Menu::Network`, and two arms in `run_menu_action`.
- `springboard/src/shell/tests.rs`: one test appended.

## Migrations and deletions

- None. The Settings panes and the existing Wi-Fi menu already used `Effect::Wifi` and `Effect::Bluetooth`; they were
  left as they are.

## Matrix entries

- None closed by this slice. The matrix is not edited here; the lead closes entries from the list above.
