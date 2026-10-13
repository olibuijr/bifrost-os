# Settings: Notifications, Software Update, Sharing, Profiles, Print & Scan (slug settings-notif-update-sharing)

Status: code and unit tests written by reading only. Nothing here has been built or run on Midget. Every row below
still needs a Midget check before its matrix row can move to `parity`.

## Reference evidence

- Index: `~/Projects/macos-ref/extract/ref27-index.tsv`. It lists `PrinterScannerSettings.appex` (en `PrintAndScan.searchTerms`)
  and `Sharing.appex` (en `Sharing.searchTerms`).
- `~/Projects/macos-ref/extract/ref27/System/Library/ExtensionKit/Extensions/Sharing.appex/Contents/Resources/en.lproj/Sharing.searchTerms`:
  the "Hostname" entry indexes `computer name`, which backs the Computer Name row. Other entries name Internet sharing and
  the screen-sharing service.
- Not measured this round: the REA tools (`inspect_plist`, `decode_interface_builder`) were not run on these bundles. The
  lead extracts the resource tables. Pane layout below follows the existing BifrOSt Settings conventions, not a decoded nib.

## Per row

### Sharing (`SharingPref.prefPane` gap, `Sharing.appex` gap)

- BifrOSt path: `springboard/src/shell/sharing_page.rs` (new), wired from `pages.rs` `PageId::Sharing`, `settings.rs`
  `Action::Sharing`.
- Done (code): Computer Name row shows the pretty host name from `/etc/machine-info` (fallback `/etc/hostname`). Tapping it
  opens the existing Computer Name prompt (`forms::Prompt::computer_name`) and saves through privd `system.set_names`.
  Remote Login, File Sharing and Screen Sharing switches read their state with `systemctl is-enabled` and change it through
  privd `service.set`. Units come from `privd::accounts::service_unit` (sshd, smb, wayvnc). Nearby Share and AirPlay &
  Handoff stay below.
- Still needs Midget: privd must be running for the switches and the rename. The state read depends on the unit names
  existing on the image. Check that the Sharing footers read well at phone and desktop widths.

### Print & Scan (`PrintAndScan.prefPane` gap, `PrinterScannerSettings.appex` gap)

- BifrOSt path: `springboard/src/shell/print_scan_page.rs` (new), `PageId::PrintScan` in `pages.rs` (root row after Sharing),
  `Action::PrintScan`, and `forms::Purpose::PrinterAddress`.
- Done (code): the printer list comes from `bifrost_ui::cups::Client::local().printers()`. Each row shows its state (Ready,
  Printing, Stopped) and the default printer is checked. Tapping a row makes it the default (`set_default`). Add Printer
  opens an address prompt that accepts `ipp://`, `ipps://`, `socket://` and `usb://` addresses, then adds a driverless queue
  (`add_driverless_printer`). The queue name comes from the host, numbered if taken. An unreachable scheduler shows as the
  page footer, not an error.
- Gaps: scanning is not in this page yet (`ui::scan` is not wired). Remove and pause are not wired. Privileged writes
  (`set_default`, add) go to the scheduler as the shell's user. They need the scheduler to allow that user. Otherwise the
  change fails and the reason shows in an alert. A privd `printer.*` method would make this reliable. Not built this round.

### Notifications (`NotificationsSettings.appex` partial)

- Already present, not changed: per-app Allow Notifications switch, alert style (none, banners, alerts), Show Previews,
  Notification Grouping, Lock Screen and Notification Centre, Sounds and Badges, DND schedule (`notify_page.rs`).
- Not closed here: the Focus pane is its own page (`focus.rs`).

### Software Update (`SoftwareUpdate` partial, `SoftwareUpdateSettingsExtension` partial)

- Already present, not changed: check, download, install, rollback, Automatic Updates and Back Up Before Updates
  (`backup_page.rs`, `update.rs`, privd `update.*`).
- Not closed here: the `updated` daemon was not reviewed in this round.

### Profiles (`Profiles` gap, `ProfilesSettingsExt` gap)

- Not done. No configuration profile store exists in the shell. The terminal's `profiles.rs` is an unrelated colour theme.

## Shared-file edits the lead must merge with care

- `springboard/src/shell/pages.rs`: `PageId::PrintScan`, `Command::Notice(String)`, `Action::Sharing`, `Action::PrintScan`,
  the Print & Scan root row, the Sharing spec, the PrintScan spec arm, and tests. Root test indexes moved by one from
  Wallpaper onward because a row was added after Sharing.
- `springboard/src/shell/settings.rs`: `Action::Users`, `Action::Sharing` and `Action::PrintScan` arms in `activate`
  (`Action::Users` had no arm, so the shell as it stood at wave-base would not compile); `Command::Notice` in `Alert::new`
  and `run`; `join` takes `ui` and `t` and finishes prompt sheets; new `edit_result`; a reachability test row.
- `springboard/src/shell/mod.rs`: two `pub mod` lines.
- `springboard/src/shell/forms.rs`: `Purpose::PrinterAddress`, its validity rule, and `Prompt::printer_address`.

## Adopt and delete

- The Users & Groups edits already had `users_page::apply` and `finish` but nothing called them. Settings now calls them
  through `Action::Users` and `join`, so that page's prompts work.
- No superseded code was deleted this round.

## Matrix entries closed

None are closed yet. They need a Midget check first. Candidates, in order of readiness: `SharingPref` and `Sharing`
(Computer Name and the three services), then `PrintAndScan` and `PrinterScannerSettings` (list, default and add; scanning
still missing).
