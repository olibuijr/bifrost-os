# SpringBoard

A macOS-style desktop shell written in Rust for x86_64 PCs. See
[the PC workflow](../docs/bifrost-pc-workflow.md) for build, publish and Midget
control. The built-in apps and controls below run in the shell.

## Terminal

The compact tab strip follows iOSTerm: add tabs with `+`, manage SSH connections
with **SSH**, inspect memory and processes with **SYS**, and open terminal actions
with the ellipsis or the keyboard’s **more** key. The menu contains search, text
size, themes, clipboard actions, scrollback export, and reset. SSH connections
and process lists open in centered panels; dismissing a panel restores the
keyboard’s previous visibility. SSH editing preserves unrelated configuration
lines, and deleting connections or signaling processes requires a second tap.

## Notes

Pin or unpin a note from its row or editor. Pinned notes appear in a separate
section at the top, including while searching. Pinning preserves the note text
and modification date, and the selection survives restarts.

## Photos

Use the Albums picker to browse All Photos, Favorites, or Recently Deleted.
The heart in the viewer adds or removes a photo from Favorites; navigating the
filtered album stays on the correct photo as its membership changes.

## Music: Radio

The Radio tab of the Music app lists the live stations of
[spilarinn.is](https://www.spilarinn.is) (the `temp*` placeholders are left out):
logo, name, frequency and, where the broadcaster reports it, what is on air.
Tap a station to listen; the heart on the right makes it a favourite, and
favourites lead the list.

- **Playing.** The list becomes the queue, so Previous and Next step through
  the stations. A live stream has no length: Now Playing, the lock-screen card
  and the Control Center tile show "LIVE" instead of a scrubber, and shuffle,
  repeat and seeking do nothing. Under the station name the stream's own title
  (ICY metadata) is shown, else the API's, else the frequency; "Buffering…" and
  "Reconnecting…" show the connection's state.
- **Streams.** MP3 over http or https (`media/stream.rs`). A network thread
  asks for ICY metadata, strips it from the audio, buffers about 30 s and
  reconnects with a growing delay (1 s up to 15 s) whenever the connection
  drops; the decoder resyncs on the next frame. Changing station or quitting
  abandons a blocked connect at once. A stream that is not MP3 stops with an
  error.
- **Data.** The list (`GET https://bakendi.spilarinn.is/api/stations/`) and the
  logos are fetched on worker threads when the tab opens and again, at most
  once a minute, while it is shown. Everything is cached under
  `<data dir>/Music/Radio` (`stations.json`, `logos/`, `favourites.txt`), so the
  list, the logos and the favourites show with no network.

`cargo run --example radio_preview -- <out dir>` renders the section to PNG on
the host: the list, a station playing under it, Now Playing and the lock card.
It fetches the real station list and logos once (cached under
`~/.local/share/paperclip-ops/radio-preview`) and plays a local fake Icecast
server, never a real stream.

## Clock

World Clock supports adding supported cities through a searchable picker and
removing cities in Edit mode. The selected cities persist separately from alarms.
Selecting a city in Spotlight opens its World Clock card.

## Spotlight

Pull down on the home screen to search apps, Settings, supported World Clock
cities, cached Deildu titles, and saved Contacts. Contact names, phone numbers,
and email addresses can open the corresponding contact directly; matching
supports Icelandic letters and accents.

## Contacts

Open Contacts from the home screen or Spotlight. Search by name, phone number,
email, or notes; add a contact with `+`, edit its details, or delete it after
confirmation. Cancel leaves the saved contact unchanged. Tap a phone number or
email in the detail view to copy it to the clipboard.

Contacts are stored locally in `Contacts/contacts.tsv` under the shell's data
directory. The default directory is `/data/local/springboard`. Saving replaces
the file atomically, and a failed write leaves the current contact unchanged.

## Focus

Toggle Do Not Disturb in **Settings → Focus** or the Focus tile in Control
Center. The choice persists across restarts. Shell notifications still appear
in Notification Center and retain their app badges; banners and notification
wakes are silenced. Clock alarms and timers continue to sound and wake the
screen.

## AssistiveTouch

Enable **Settings → Accessibility → AssistiveTouch**. Drag the floating button
to reposition it or tap it to open shortcuts for Home, App Switcher, Control
Center, Notification Center, Lock Screen, and Screenshot. Tap outside the menu
to dismiss it. The control hides while locked and dims after inactivity.

## Date & Time

Settings > General > Date & Time keeps the clock right on its own.
Set Automatically runs an SNTP client (`net/ntp.rs`, pool.ntp.org then
time.cloudflare.com) once Wi-Fi has an IP and every 6 h; the system clock is
set with `clock_settime` only when it is off by more than 2 s, and the page
shows the last result. The Time Zone row opens a searchable picker over the
tz database; `bare/timezone.rs` parses the TZif files (`/usr/share/zoneinfo`) and `sys::local_time` uses the chosen zone
(`tz` in `settings.ini`, default Atlantic/Reykjavik). `time_auto=0` turns
automatic sync off; 24-Hour Time is the existing `clock_24h` option.

## Power

Holding the power key for a second with the screen on opens a "slide to power
off" sheet with Restart below it; Cancel (or a short press
of the key) closes it. The same actions are in Settings › General (Restart,
Shut Down) behind confirmation alerts. Power off and restart sync the disks,
flush app data and settings, then power off or restart.

Low battery: an offer of Low Power Mode at 20%, "Low Battery" alerts at 15%
and 5%, and a clean shutdown at 3% or less when not charging. Each alert shows
once per discharge and re-arms after the charge climbs three points above its
level; plugging in withdraws it. Settings › Battery has Low Power Mode, which
caps the backlight at 50%, locks the screen after at most 30 seconds and limits
drawing to 30 frames a second, and shows when the machine was last charged.
After every suspend the tap-to-wake gesture, Wi-Fi power save and the
wake-on-WLAN trigger are written again.

## Power profiles

Settings › Battery › Power Mode picks the CPU profile (`power_profile` in
`settings.ini`: auto, performance, balanced, battery). Performance runs the
`performance` governor on both clusters. Balanced is the stock `schedutil` with
the full range and the interaction boost (minimum clock to maximum for 500 ms
after a touch or while animating). Battery Saver caps the big cluster at 60% and
the little one at 70% of their cpuinfo range and boosts only to the middle for
300 ms. Automatic is Performance while charging at 20% or more, Battery Saver
unplugged at 30% or less and Balanced otherwise, with a three-point hysteresis;
Low Power Mode always forces Battery Saver. Frequencies are computed from each
policy's own limits. A clean exit restores `schedutil` and the
full range.

## Storage, Passcode and Developer

Settings > General > Storage shows used and free space on `/data` with a
per-category bar (Photos, Music, Files, Deildu, Terminal, Notes/Contacts/Other,
Logs), measured on a worker thread, and clears the disposable caches (Deildu
cache, thumbnails and artwork, `log.old`, `springboard.before-*` beyond the
newest two). Below 500 MB free the shell trims those caches itself and posts
"Storage Almost Full" (`shell/storage.rs`).

Settings > Passcode sets, changes or turns off a 6-digit (or 4 to 16 digit
custom) code kept as a salted, stretched SHA-256 in `DIR/passcode.ini`; the
lock screen asks for it after the unlock swipe, and wrong codes back off (1
minute after 5, then 5, 15 and 60) across restarts (`shell/passcode.rs`).

Settings > Developer has Rooted Debugging (off writes `service.adb.root=0` and
removes `/data/local.prop`; bare mode at boot needs it on) and Wi-Fi ADB
(`persist.adb.tcp.port` 5555 or 0), both read from the real properties.

## Verification and builds

Run commands from this directory:

```sh
cargo test
cargo run --example notes_pinned_preview
cargo run --example photos_favorites_preview
cargo run --example clock_preview
cargo run --example contacts_preview
cargo run --example radio_preview -- ~/Pictures/tab/radio-preview
cargo run --example assistive_touch_preview
```

The previews write images under `target/` (`radio_preview` under the directory you give it).

## Host run

The shell also runs on an x86_64 dev host: `cargo build --offline --profile fast --bin shell --features desktop-wayland`.
`SPRINGBOARD_VIRTUAL=1200x1920` selects the virtual panel and `SPRINGBOARD_CONTROL_PORT` the port. See `../docs/arch-x86-vm.md` for the VM.

## Updates and recovery

Releases go through the update service: `bifrost pc publish` publishes, and Settings › General ›
Software Update downloads it and restarts the shell. See `../docs/system-recovery.md` for
probation, rollback and the recovery states, and `../docs/bifrost-pc-workflow.md` for Midget.

## Audio routing

`media::route` switches the card between Speaker and Headphones without
a sound server. The headset input device (name containing
"headset" or "jack") reports plug state as switch events; plugging
switches the route, shows the volume HUD with the route name and lowers a
remembered headphone volume to the 85% "Headphone safety" cap until the
user raises it. Unplugging pauses music first, then restores the speaker.
Each route remembers its volume (`volume_speaker`, `volume_headphones`).
Output switches are found by name at run time and missing ones skipped.

## Audio status

Playback works through PCM device 3 (`pcmC0D3p`, FAST scene) with the writer polling the DMA
position itself; see the `tab-audio` skill.
