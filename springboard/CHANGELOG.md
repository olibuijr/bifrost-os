# BifrOSt changelog

## Unreleased

- Terminal: More and File menus gain Close Other Tabs (tabs running a job stay open), Paste Escaped Text, Open man page for selection, Edit Title (a per-tab name; empty restores the automatic title) and Terminal Help, a read-only list of the Terminal's shortcuts.
- Photos: the album picker can Delete Album, Sort Albums by name and set the slideshow slide length (2, 4, 6 or 10 seconds).
- Photos: the editor has a Tools page with Rotate Left, Flip Horizontal, Flip Vertical, Auto Enhance (a tone stretch) and a luminance histogram, all kept in the saved edit. Tap the selection title to Select All or Deselect All. The grid has four zoom levels (8, 5, 3 and 2 columns), remembered. The share sheet can Duplicate a photo (its edit and rating come along). The Info sheet rates a photo with stars and can Hide it into a Hidden album that is kept out of All Photos, Recents, Favorites and Screenshots.

## 0.7.25 — 2026-10-13

- Window server: labwc is gone. bifrost-wm is the only window server; after three quick exits it restarts in safe mode (GLES, no direct apps, no chrome clients), then recovery takes over. bifrost-wm has the global shortcuts (Spotlight on Super+Space, Super+A, Alt+Tab, Alt+F4, Ctrl+Super arrows to tile) and real screen capture (screencopy and ext-image-copy: the scene rendered offscreen and read back as RGBA, so screenshots show the wallpaper and the right colours). The shell's own live-backdrop blur, which only served labwc, is removed: the window server blurs the real scene behind every material.
- Parity sprints 1-8 (SDK and OS layer): TimeZone, FileManager atomic writes and standard folders, URLSession uploads and background downloads with resume, Accounts, SF Symbols drawing, sharing services, user notifications, named alert sounds, screen reader earcons, cursor art, Quick Look thumbnails, Settings panes (Mouse, Trackpad, Print & Scan, Sharing, Internet Accounts, Users & Groups), Vietnamese and Korean input methods, the power chime, Core Image distortion, stylize, halftone and tile filters with a catalogue and a GPU path for colour filters, Core Audio aggregate devices and process taps, the audio session with interruptions, Disk Arbitration volumes, Core Location, CFBundle, DispatchIO, Apple Events, predicate validation, Video Toolbox encoding, Speech, MIDI-CI, raw motion sensors, EventKit saving and recurrence, Keychain item queries, Local Authentication, pasteboard items and promises, the Network listener, Contacts multi-value fields and saving, PhotoKit, file versions, declared file types, AppKit printing, WebKit, Service Management, Core Spotlight, AVKit player view, log privacy formatting, Ed25519 signatures, UUID and Decimal. Open font families (Noto, Liberation, Croscore, STIX) with family substitution.
- Fixes: an app crash right after its socket closes keeps its exit code again (0.7.24 counted it as a clean quit); file version history survives safe saves; IMAP literal and response caps; bounded stream reader lines; the MCP server bounds pre-auth input with one deadline per request; netd and audiod survive poisoned locks.

## 0.7.24 — 2026-10-13

- An app whose socket closes before its process exits (a crash writing its core dump, or a slow quit) is reaped on a short-lived thread (up to 3 s, then SIGKILL) instead of being SIGKILLed at once, so a real crash keeps its status: SIGABRT in Files now logs, saves the report and posts the notification, where 0.7.22-0.7.23 saw only SIGKILL.

## 0.7.23 — 2026-10-13

- On the desktop an app crash posts a notification (report saved for Console) instead of the modal card, which the desktop drew under the app windows: unseen, unanswerable, and a full-screen repaint every frame.

## 0.7.22 — 2026-10-13

- An app stopped from outside (SIGHUP, SIGINT, SIGKILL, SIGTERM) no longer gets the "quit unexpectedly" alert. On the desktop that alert sat under the app windows, unseen, and forced a full-screen CPU repaint every frame: Browser scrolling at 165 Hz dropped to 17-29 ms frames after any app was closed that way.

## 0.7.21 — 2026-10-13

- Suspend attempts run on a short-lived thread each, so an idle shell holds no suspend thread (17 threads at idle).

## 0.7.20 — 2026-10-13

- An app launch no longer starts the app-resources writer; only the Cpu/Memory readout does (nothing else reads the file).

## 0.7.19 — 2026-10-13

- The app-resources writer exits when nobody has read its file for 15 s and starts again on demand, instead of parking a thread for the life of the shell.

## 0.7.18 — 2026-10-13

- Fixes 0.7.17 spinning a core: `sys-watch` polled the PSI trigger files for POLLIN, which a proc file always reports. Each watcher now has its own event mask (PSI waits on POLLPRI only); regression test `a_proc_file_polls_readable_but_not_priority`.

## 0.7.17 — 2026-10-13

- The memory-pressure (PSI) watcher runs on the shell's `sys-watch` thread (`purgeable::hosted_watcher`); `sys-watch` polls POLLPRI and drops a watcher whose fd fails instead of spinning.

## 0.7.16 — 2026-10-13

- Fewer shell threads: volume and sound share one audio worker; the SIGTERM and stall watchers share one `sys-watch` poll thread (`bare/sys_watch.rs`).

## 0.7.15 — 2026-10-13

- Unlock hands the lock screen's freed memory back to the system (`malloc_trim` when the unlock finishes): shell memory after a lock cycle 112 → 97 MiB on Midget.

## 0.7.14 — 2026-10-13

- Boot: the compositor starts without `--verbose` (dozens of debug lines into the journal on every boot; no measurable start-up change, less log noise).

## 0.7.13 — 2026-10-13

- Reverts 0.7.12's release of the window scratch buffers: freeing and re-allocating them raised the shell's memory after a lock cycle (97 to 113 MiB) instead of lowering it.

## 0.7.11 — 2026-10-13

- Boot: `video-arg` now writes the mode in use into a unified-kernel-image command line (it refused any line without `initrd=`), so the next boot starts at the panel's refresh and is not modeset twice; it follows the Settings refresh choice. The session keys its shell-version stamp on the binary (one stat) instead of globbing pacman's database, which took 0.65 s on a cold boot. Desktop first frame 6.64 to 6.37 s after kernel start, userspace 4.45 to 3.37 s on the lab PC.

## 0.7.10 — 2026-10-13

- Boot: the session starts the bus, the compositor and the portal as the account with `setpriv`, without a PAM session: `runuser`'s pam_systemd started the account's user manager synchronously first. The desktop's first frame comes 0.8 s sooner on the lab PC (7.60 to 6.77 s after the kernel starts).

## 0.7.9 — 2026-10-13

- Idle: the clock service reads the Clock app's alarm files once a second only while the app runs (once a minute otherwise), and the microphone probe looks every 10 s instead of 2 s.
- `SPRINGBOARD_WAKE_TRACE=1` logs which deadline wakes the shell (`Wake` records its call site).

## 0.7.8 — 2026-10-13

- Idle: the SIGTERM watcher blocks on a self-pipe instead of polling every 100 ms (10 of the shell's 15 idle wake-ups a second on the lab PC).
- `tools/bench.py --midget`: unlocks with the account password (the login window ignores a bare Enter) and closes a window the session restored before timing that app's launch.

## 0.7.7 — 2026-10-13

- Displays: the fastest refresh at the native size is the default from the first frame (`display-refresh` runs in the session's autostart before the shell connects; a 2560x1440 panel that offers 165 Hz no longer starts at 60 Hz). Settings > Displays > Refresh Rate picks Highest Available or any rate the panel offers, applied live and kept across restarts (`/var/lib/springboard/display-refresh`). `refresh=firmware` in `/etc/bifrost/display.conf` still keeps the firmware mode.

## 0.7.6 — 2026-10-13

- Smoothness: the key-state fast path also takes an app frame that is still a few pixels larger than its window (the size the app first drew at), clipped to the body as the full draw clips it; such switches fell back to drawing the whole window.

## 0.7.5 — 2026-10-13

- Smoothness: a built-in window keeps its last body, so when only the key state moves (another window comes forward) and nothing in it changed since it was drawn, the body is reused and only the frame, the dim and the lights are drawn.

## 0.7.4 — 2026-10-13

- Smoothness: when only the key window changes (Cmd-Tab, a click on a back window), a full-size app's window copies its body again from the app's last frame, dimmed or not, and repaints its lights, instead of drawing the whole window afresh (the Browser switch was a 31 ms frame on the lab PC).

## 0.7.3 — 2026-10-13

- Smoothness: an outline's rows near its top and bottom edges also skip the empty stretch between the corner squares (still bit for bit). Host benches at 2560x1440: focus switch 19.9 to 16.3 ms, Settings repaint 9.2 to 7.8 ms (`bench_focus_switch`, `bench_settings_window_draw`).

## 0.7.2 — 2026-10-13

- Smoothness: a rounded-rectangle outline (window and card borders) only walks its edge bands instead of every pixel of its area; same pixels, bit for bit (test `stroke_rrect_matches_the_per_pixel_reference`). A Settings repaint at 2560x1440 went from 47 to 21 ms on the CPU path in the host bench `bench_settings_window_draw`.

## 0.7.1 — 2026-10-13

- Smoothness: a focused Settings window keeps its cached pixels until input, a change Settings reports or an animation makes them stale (it was redrawn in full on every desktop frame, about 23 ms a frame at 2560x1440 on the lab PC). It still refreshes at least once a second.

## 0.7.0 — 2026-10-13

Round 75: ten parity waves (macOS 27 reverse, daily-driver apps and settings, command-line tools, smoothness).


- Screen recording polish: a recording runs at the display's refresh rate (up to 60 frames a second) while the GPU path is
  up, and at 10 frames a second otherwise. The ffmpeg H.264 path and the built-in Motion JPEG AVI writer both stay. With
  ffmpeg, the Options menu's Microphone takes the audio service's microphone into the movie as AAC (the first time, Allow
  is asked for). A finished recording opens in the Video Player (`open-movie` intent). Stop Recording is in the menu bar's
  red item, as before.
- Screenshot toolbar Options menu: Save To (Desktop or Documents, for recordings), Timer (Off, 5 s, 10 s), Show Pointer
  (pictures) and Microphone (recordings). The choices last across sessions. Timer and Pointer buttons moved into the menu.
- Screen recording: the screenshot toolbar (Shift-Cmd-5) has Record Selection and Record Screen buttons. A recording is
  made of the frames the shell draws, encoded by the system `ffmpeg` (H.264 in a `.mov`) when it is on the PATH, else by a
  built-in Motion JPEG AVI writer. It is saved to a dated file on the Desktop. The menu bar shows a red stop item beside
  Control Center while it runs, with Stop Recording. The control port takes `rec start [screen|x,y,w,h] [file]`, `rec stop`
  and `rec status`, and `screencapture -v` records until stopped (or `-V seconds` for a set length).
- Services menu: the App menu lists the services that take the current selection (the pasteboard's text or files):
  Look Up in Dictionary, New Note With Selection, Make New Sticky Note, Open URL, Search With Quick Search and Show in Files.
  `bifrost_ui::services_menu` models which service takes what and where it goes; `shell/services_route.rs` carries it out
  (Dictionary's `look-up` and Notes' `create-note` intents over their control sockets, links in the browser, Files, Search).
  Ctrl+Cmd+D and Services > Look Up open the Look Up popover under the menu bar: the first installed definition, Quick Search
  results, Search and Open in Dictionary. Escape or a tap outside closes it (`shell/look_up_popover.rs`, model `bifrost_ui::look_up`).
  The StarDict reader moved to `bifrost_ui::stardict`, shared by the shell and Dictionary 0.1.1.
  Intents can take string parameters: `intents::run_with` and `request_line_with`.
- Dock: an app sets its own items in the Dock's menu (`dock` service `menu`, SDK `dock_tile::set_menu`). Items show under the
  app's Open, Keep in Dock and Quit rows; choosing one sends the app its id as `ContextAction`. Up to eight items.
- App launch: an app that fails to start, or exits before it says hello, posts an alert from the app ("X could not be opened") and its Dock launch bounce stops at once. A window closed and opened again in the session comes back at the size it had (a fixed manifest size still wins).
- SDK: `UserFonts::system_object`, `mono_object` and `library_object` return the user's Settings > Fonts choice (or a
  library match) as an app-facing `FontObject` at a size, so apps draw and measure with the shell's face.
- Dock: a Downloads stack sits left of the Trash (after the minimized windows). It shows the three newest visible files of
  Downloads fanned out on a blue folder, re-listed every 10 s (never per frame), and a click opens the folder in Files.
  `dock_stack.rs` holds the list, the fan and the drawing; the Trash uses the same account home helper.
- Dock magnification in Settings sets the dock's value (`Change::Magnification(on)`) instead of flipping it, so the switch
  and the dock cannot drift apart.
- Software Update: `softwareupdate --schedule` prints whether the background checks run; `--schedule on|off` switches them.
  Manual checks (`softwareupdate -l`, Settings > Check) still run. The switch is one file in the shell data folder that the
  update agent and the shell's own checker both read.
- Settings > General > Language & Region is real: preferred language (English or Íslenska, first one wins), Region
  (twelve regions), First Day of Week (Automatic, Sunday or Monday) and the formats they set (short date, currency).
  The choice is saved in `settings.ini` (`languages`, `region`, `first_weekday`, `currency`), publishes to the menu bar
  date (`bifrost_ui::region`, `short_date`), and every app reads it through the SDK `Locale::current()`, with
  `first_weekday()` and the currency overrides. The system locale follows through privd `locale.set` (`localectl
  set-locale`, which needs the locale generated, for example `is_IS`).
  Calendar's month grid still starts on Sunday; it does not read the first-day setting yet.

- Terminal: More menu row "Open link in selection" opens the first web link in the selected text in the browser
  (`terminal/links.rs`, reusing the URL detection in `profile_table.rs`).
- Input: a third input source, Japanese (Romaji), types the English keys and composes kana in the Search field: romaji
  becomes marked hiragana as it is typed, Space opens a hiragana and katakana list and a digit picks from it
  (`bifrost_ui::kana_method`). No kanji: no dictionary is shipped, and no CJK engine is installed.
- Input: holding a letter that has accents (in the Search field) opens its accent list instead of repeating it.
- Edit > Emoji & Symbols opens a character viewer: emoji in ten tabs, paged, shared with the keyboard's emoji page. A pick
  types into the open Search field, or else copies to the clipboard. Escape or Done closes it.
- Input Sources > Show Keyboard Viewer (the input menu, with two or more sources) draws the active source's keys, plain
  and with Shift, from the keymap tables, and lights the key held down.
- Help lists Input Source with Ctrl+Space.
- On-screen keyboard 1.1.1: its emoji category catalogue now comes from the shared emoji module (no behaviour change).

- Lock Screen > Require Password: the screen stays as it was for the chosen time (immediately, 5 seconds, 1 minute, 5 minutes, 15 minutes, 1 hour) after the display goes off, and a password is asked only after that. Login window: Login Options > Name and Password shows a name field and refuses another account name before the password is checked.
- Notification Center: Focus lets the apps you allow (Settings > Notifications > App > Allow During Focus) show banners and wake the screen; the Lock Screen, Notification Centre and Badges switches on each app now take effect.

- Disk images: one image model in `bifrost_core::disk_image` (the ui copy is gone). `hdiutil` (create, attach, detach, info, verify, convert) in the `cmd` crate; attach and detach go through privd `disk.loop_attach` and `disk.loop_detach` and the shell's `image-attach` and `image-detach` control lines. Files mounts an `.iso` or `.img` on double-click; Disk Utility shows loop devices that hold an image and detaches them (Image menu). See `docs/parity/disk-images.md`.
- SDK: text layout objects (`bifrost_ui::typeset`: typesetter, framesetter, frame, line, run) with Unicode line break and
  grapheme cluster rules, paragraph styles, tab stops, bidi line reordering, truncated and justified lines. Attributed
  strings gain font family and size, literal colours, baseline, superscript, kern, tracking, stroke, underline colour and
  style, paragraph styles and UTF-16 range methods. `RichTextView::draw` goes through the framesetter and draws the
  selection and caret; it has hit-testing. Removed `textfit` and the line layout manager of `textlayout`.

## 0.6.5 — 2026-10-12

Honest live audio numbers and apps rebuilt on the current SDK.

- Audio bench: the live MIDI latency measures on one clock and fails loudly on any sample outside 0..1000 ms; the
  audio round trip reports "no loopback" (skipped, never recorded) when no cable connects line out to line in.
- Apps: 21 desktop apps rebuilt on this SDK (an app built on the older shell/app protocol drew a black window).
- Window server: bifrost-wm and its chrome ship in the update (opt-in through /etc/bifrost/compositor); the shell
  asks it for live blur behind its windows.

## 0.6.4 — 2026-10-12

The audio server stops its devices when asked.

- Audio server: every wait of a device's IO thread is bounded and checks the stop request, a cycle target far in the
  future is resynced, and a device that does not stop in time has its stream aborted instead of being left running.
  A device thread is never detached: the server refuses to reopen the device until the old thread has ended. Before,
  a stuck stop left the sound card held by the server and every later open failed with "Busy".
- Boot: the module recorder rebuilds a stale initramfs drop-in after an update even when the module list is the same.
- Boot: the boot image carries microcode for this CPU only (it was in twice, for every CPU) and only the GPU firmware
  the driver asked for, compressed harder; the menu bar and Dock draw as soon as the compositor configures them, so the
  desktop no longer waits up to 4 s for its first full frame.

## 0.6.3 — 2026-10-12

One display mode from power-on to the desktop, and a desktop that cannot freeze on the audio server.

- Boot: Bootweave keeps the mode the firmware started in (the monitor's preferred mode) instead of switching to the
  fastest refresh, so the compositor's start is no longer a second switch back: two black gaps fewer. The fastest
  refresh stays opt-in (`refresh=max` in /etc/bifrost/display.conf). New installs keep the kernel console off the
  display (`fbcon=map:9`), so nothing can wipe the boot frame.
- Boot: the initramfs carries only storage, file system, virtio and display drivers instead of every module the
  machine ever loaded; the boot bench reads the loader's time correctly.
- Shell: no audio server call runs on the UI thread any more (microphone probe, volume, the speaker stream), and
  reconnects back off from 1 s to 30 s instead of five times a second. A server that accepts and never answers costs
  the desktop nothing.
- Audio server: clients that never say Hello or stop reading are dropped, a device stop no longer waits forever on its
  IO thread, and it stops within 5 s when asked.
- Session: a stall no longer counts toward the start-failure limit, the session waits for its bus, and the exit
  status of the shell is logged.
- Audio bench: a failed bench is reported and never recorded as zeros.

## 0.6.2 — 2026-10-12

The audio server and the after-desktop services reach installed machines.

- Packaging: the update package now carries bifrostd, the audio server (bifrost-audiod), its latency bench and the
  ALSA plug-in. 0.6.0 and 0.6.1 shipped without them, so the desktop-ready trigger failed and nothing that waits for
  the first desktop frame started (the agent access server among them). A package whose units name a missing binary
  now fails to build.
- Audio: the install script creates the audio user and enables bifrost-audiod; ALSA's default device goes through it.

## 0.6.1 — 2026-10-12

The real-time kernel by default, a loader built from source, and installs driven like a user.

- Kernel: new installs get linux-rt (PREEMPT_RT). Machines with an NVIDIA GPU, or a package source without linux-rt,
  get the stock kernel, and the installer logs why. The loader and the update guard follow whichever kernel is
  installed, so existing installs keep booting.
- Boot: every release builds the EFI loader from source (a stale prebuilt loader could not start the unified kernel
  image), and the live installer opens its display before it scans for disks.
- Installer: Return presses Restart on the Done screen, like the default button on every other screen.
- Lab: `midget.py qa-install` drives the installer's real screens. It picks the keyboard layout on the Language
  screen, types through the installer's own keymaps (dead keys and AltGr, so Ó and ú type), clears the
  pre-filled account name first, and detects the restart by the kernel's boot id.

## 0.6.0 — 2026-10-12

New graphics, audio and boot stacks, a full code audit and lab autonomy.

- Graphics: the window server's Vulkan scanout draws the real scene (Wayland and X11 windows) with real damage; the
  GLES path composes only when something changed; blur no longer forces a page flip every vblank; background windows
  keep their stacking and focus. GPU uploads are batched into the frame instead of a synchronous upload per glyph.
- Audio: the BifrOSt audio server (bifrost-audiod) with the HAL client, ALSA driver (mmap, 2-period ring, converter
  latency both ways), aggregates, MIDI server, PulseAudio, JACK and ALSA plug-in compatibility, and the
  lowest-latency path (16 and 32 frame quanta, CPU latency hold, sound IRQ thread priority, threaded IRQs).
  bifrost-audio-bench measures MIDI latency through a virtual Bench Instrument, audio round trip, xrun sweeps and
  MIDI throughput, live against the running server.
- Boot: Bootweave draws one boot frame from the loader to the desktop with a progress bar that learns from earlier
  boots, unified kernel images, a text-free kernel, Unlock is login, failure and rollback screens, the desktop
  revealed once, and a symmetric shutdown fade.
- Audit: 153 findings from a full read-only audit fixed or recorded. Fixes include an audio cycle that burned its
  whole budget waiting on a signal nothing sent, a hostile audio client reading out of bounds, a short app buffer
  that could crash the desktop, blocking app sockets that could freeze it, and the CPU desktop path's 40-90 ms frames.
- The workspace builds with no warnings; 12,165 tests pass.
- Lab: panics, oopses and lockups reboot the machine; Wake-on-LAN stays on; kernel messages stream to the build host.
  Real-time kernel benchmarks (worst wake-up 38 us versus 337 us, no underruns at 32 frames) decided linux-rt as the
  next default kernel.

## 0.5.44 — 2026-10-12

Parity wiring, wave 6 (30 agents).

- Screen reader: the cursor now walks the menu bar, an open menu and the front window (Settings rows or an app's
  elements), speaks the role before the name, Ctrl+Alt+Up and Down pick a rotor (Controls, Lists, Text ...), and
  Ctrl+Alt+Space presses the current element.
- Input: a built-in compose method (apostrophe, then a letter) opens a candidate window of accented and Icelandic
  letters; digits pick one.
- Menu bar: a Now Playing item appears while an app plays, with title, position and transport controls; the leading
  system menu lists Recent Items with Clear Menu.
- Notifications: the first notification from an app asks to Allow or Don't Allow.
- Settings: Game Controllers (battery, live button and stick test, button remapping), Passwords (reveal after the
  passcode, copy, edit, delete), Login Items start order, Bluetooth device type and Use for Sound Output.
- Window motion: closing a window fades and shrinks it, and the Dock icon bounces until the launched app's window maps.
- Photos: Lens in the Info sheet and Revert to Original for edited copies. App Store: a purchase sheet for paid apps.
  Camera: pick among connected cameras.
- Apps: Assistant 1.4 (tool calls shown inline), Books 1.6 (single and two-page views, select and copy a page),
  Browser 0.13 (Find, Zoom and Downloads in the menu bar), Calendar 1.11 (invitees and their responses), Console 1.4
  (hang diagnostics, daily summary), Contacts 1.7 (several phones, emails and addresses per contact), DAW 0.5 (Export CD
  as a 16-bit WAV), Files 1.21 (picture thumbnails, EXIF in Get Info, Open With), Freeform 1.6 (Pencil tool, soft
  shadows), Games 1.6 (Tic-Tac-Toe against the computer), Music 1.6 (BPM column), News 1.6 (article search), Podcasts
  1.7 (speed menu, 15 s back skips), Reminders 1.13 (#tags), Shortcuts 1.6 (result panel with a follow-on run), Voice
  Memos 1.6 (Split at Playhead, Skip Silence), Weather 1.5 (area fill under the hourly chart).

## 0.5.43 — 2026-10-12

Parity wiring, wave 5.

- File > Print opens a print sheet: CUPS printers or Save as PDF, paper size, orientation, scale, copies and a page
  preview.
- Settings > Accessibility > Screen Reader: Ctrl+Alt+Right and Left move a focus ring through the Settings page and
  announce each row.
- Ctrl+Space switches the input source; with two or more keyboards enabled the menu bar shows an input menu. Shell
  text fields underline text that is still being composed.
- Notification Center widgets have options in Edit Widgets, and tapping a widget opens its app at the widget's link.
- Settings > Network > VPN lists VPN configurations; Settings > Bluetooth has a Scan for Devices row.
- App Store: price and age rating on the product page, and a Purchased list under Updates.
- Default Apps has an Email group, and links open in the app that declares their scheme.
- Control Center's Now Playing shows playback progress; tap the bar to seek.
- Photos: Favourites, Recents and Screenshots are smart albums, and albums can be sorted by date.
- Apps: Contacts 1.6 (vCard import and export, last name first), Music 1.5 (Clear Up Next), Weather 1.4 (data
  attribution), Books 1.5 (Pages tab, notes on highlights), Freeform 1.5 (shape inspector, Export as PNG), Assistant
  1.3 (conversations sent as model sessions), Notes 1.13 (Replace, smart folders), Calculator 1.6 (statistics on the
  paper tape), News 1.5 (conditional feed loads, Share Article, text size), Clock 1.4 (Copy Laps, dark grey window).

## 0.5.42 — 2026-10-12

- Control Center and Notification Center open above app windows (they were drawn behind them), and Edit Widgets sits
  just under the last widget.
- Menus and search open right under the menu bar again (0.5.40 moved them down by the bar's height).

## 0.5.41 — 2026-10-12

Parity wiring, wave 4.

- Notification Center: Edit Widgets at the foot of the column opens a gallery to add or remove widgets and pick
  Small or Large.
- Accessibility: the accessibility tree reports each element's parent and its checked and focused state; apps can
  post announcements, shown as a banner; windows offer Zoom, Minimize and Close as custom actions.
- Apps can post and listen for named notifications through a new system service, and the system posts appearance,
  lock and network changes.
- Settings > Wi-Fi shows each known network's security type, and the network sheet has Details and Diagnostics.
- Photos: Exposure, Saturation and Warmth adjustments, Blur and Vignette effects, and a Sepia filter.
- Quick Look classifies pictures, text, PDF and audio and names the app that opens them.
- Dark appearance: Settings and the App Store use the dark grey content colour instead of black.
- The thin line that could stay under the menu bar after a menu closed is gone.
- Apps: Calendar 1.10 (File > New Event from Text; copy a phone number from the notes), Reminders 1.12 (Undo and
  Redo; smart list counts), Console 1.3 (saved searches; level: and message: search terms), Shortcuts 1.5 (intent
  titles in Add Action; run an intent and see its result), Browser 0.12 (History > Back and Forward lists), Files 1.20
  (permissions and owner in Get Info; writes wait for a folder in use), Voice Memos 1.5 (input meter, playback
  volume), Podcasts 1.6 (episode type, codec and length), Games 1.5 (2048 tiles swell and grow in), DAW 0.4 (Rack
  sliders, instrument presets, track format).

## 0.5.40 — 2026-10-12

- Window tiling now goes through the compositor: Ctrl+Cmd+Left and Right snap the front window to a half,
  Ctrl+Cmd+Up fills the screen, and Ctrl+Cmd+Down centres it, for every window. In 0.5.39 the Window menu's halves
  did not move app windows and left a copy of the menu on the wallpaper. The menu keeps Fill and Return to Previous
  Size, and shows the halves and Center with their shortcuts.
- Maximized and snapped windows stay below the menu bar and above the Dock.

## 0.5.39 — 2026-10-12

Parity wiring, wave 3.

- Desktop: the Window menu has Left Half, Right Half, Fill, Center, Return to Previous Size and Next Window, also on
  Ctrl+Cmd+arrow keys and Cmd+`; a window dropped on a screen edge or corner snaps to it.
- Screen saver: Settings > Screen Saver offers Clock, Colours or Black, starting after 1 to 30 minutes or never, with
  a preview. A key or click returns to the desktop or the lock screen.
- Permission prompts: when an app asks for location or media access, the system shows an Allow / Don't Allow sheet.
- Share: Compose Post is a share destination (a 280-character sheet; Post copies the text, nothing is sent online).
- Search: filters such as "pdf", "photos", "last week" or "past 3 days", with content type labels and thumbnails.
- Text: a system spelling service with learned words, and Settings > Keyboard > Text Replacements.
- Focus: apps can keep settings per Focus and hear when it changes. Widgets can say when they are most relevant,
  and Notification Center orders them by it.
- MIDI: .mid and .midi files play through a built-in synthesizer (Music plays them in the queue).
- Apps: Files 1.19 (Go > Connect to Server for ftp://), Browser 0.11 (View > Show Reader, File > Share), Notes 1.12
  (Insert Sketch), Books 1.4 (Read Aloud highlighting; no voice is installed yet), News 1.4 (Translate Article),
  Calendar 1.9 (an address in the notes can become the location), Reminders 1.11 (first-use tip), Calculator 1.5
  (Copy Tape), Contacts 1.5 (labelled detail rows).
- Dark appearance: the content area of 14 apps is dark grey instead of pure black.

## 0.5.38 — 2026-10-12

Parity wiring, wave 2: the models from the parity waves are now reachable in 13 apps and the shell.

- Desktop: the Calendar icon in the Dock shows today's weekday and date (it showed "Tue 1").
- Settings: Accessibility > Display has Dim Flashing Lights, and apps can read it. Privacy & Security lists Location
  access per app. A Bluetooth device's page has a Services row, and the network service can browse local services.
  Neither has a data source yet, so both lists stay empty.
- Files 1.18: the file area is white in Light and dark grey in Dark (it was pure black). Get Info describes disk
  images, and folders pick up changes made while they load.
- Contacts 1.4 (groups, Add People, sort order), Music 1.4 (rescans when files change, lists MIDI files),
  Notes 1.11 (detects flight numbers and addresses), Voice Memos 1.4 (Recording menu, memo notes),
  Podcasts 1.5 (feed refresh progress), Freeform 1.4 (first-use tip, Arrange), Weather 1.3 (map and chart),
  Games 1.4 (win confetti), Shortcuts 1.4 (entity queries, the open shortcut as context), Assistant 1.2
  (answer guardrails, request routing, Diagnostics), DAW 0.3 (MIDI file import, region export),
  Console 1.2 (Security Events, code requirement checks).

## 0.5.37 — 2026-10-12

Reference-SDK parity, wave 6: 20 agents worked through the 184 remaining gap rows. 62 are now built as pure models
with host tests (built share 82.0% -> 85.2%, coverage 46.6% -> 48.2%; `bifrost pc parity status`). The other 120
need hardware, a network service or a proprietary format; the reasons are in `docs/parity/w6-*.md`.

- New models include AES-GCM, authorization requests and an AutoFill credential provider, dispatch target queues,
  disk image properties, DNS-SD connection lifecycle, a multi-line text view (wrap, caret, selection, scroll), media
  trimming, 3D chart surfaces, a bag-of-words text classifier, widget search results and a compose sheet.
- Fixes found while integrating: a duplicate embedded preview model was dropped (the SDK's `PreviewView` stays).
- 10044 tests pass across the workspace. The models are not wired into the shell or apps yet, so the rows are Partial.

## 0.5.36 — 2026-10-12

Reference-SDK parity, the wiring and connect waves: the pure models from waves 1 to 5 now run in the shell and in the apps.

- 17 new shell system services over the named service channel (no wire protocol change), each with a typed SDK
  client: audio devices and volume, Bluetooth, cursor stack, Dock tile (badge, progress, bounce), game controllers,
  keychain access groups, login items (with an approval step in Settings), network path, scheduled notifications,
  Now Playing (Control Center's media tile drives the owning app), pasteboard (typed items, named boards, change
  events), power, privacy (camera and microphone ask once per app), Quick Look panel, screens, search index (ranked,
  language-aware Spotlight) and widget timelines with live activities in Notification Centre.
- 19 apps use the models: Books (find in book, highlights), Browser (bookmark import, ranged history clearing),
  Calculator (Convert mode), Calendar, Console (log positions), Contacts, DAW (MIDI events), Files (coordinated
  moves), Freeform (strokes and lasso), Games (state machines, randomization), Music and Podcasts (Now Playing),
  News (language, rich links, Reading List), Notes (find, data detectors, spelling), Reminders (shell-scheduled due
  alerts), Remote (service discovery, network path), Shortcuts, Voice Memos and Weather. Notes per branch in
  `docs/parity/wire-*.md`.
- Fixes found while integrating: language identification ignores links, email addresses and host names, so "com" no
  longer makes an English note Portuguese.
- The connect wave: 21 apps call the new services. Music and Podcasts drive Control Center's Now Playing; Files uses
  the preview panel, typed pasteboard, search index and Dock progress; Calendar, Clock, News, Notes and Reminders
  publish Notification Centre widgets; Shortcuts runs daily reminders and opens at login (after approval in Settings);
  Console shows thermal and battery state and copies log lines; Browser shows download progress on its Dock icon and an
  offline page; Assistant keeps the message when offline. Notes per app in `docs/parity/connect-*.md`.
- BifrOSt is for PC: Settings shows the machine's DMI product name (or "PC") instead of the old tablet model, the
  built-in web view reports an x86_64 desktop, and the Bluetooth and share names say BifrOSt.
- 9732 tests pass across the workspace.

## 0.5.35 — 2026-10-12

Reference-SDK parity, waves 3 to 5. The deploy gate is reached: 82.0% of the 1935 applicable feature rows are built
(Partial or Parity), and coverage is 46.6% (`bifrost pc parity status`). 0.5.34 was not published; this release
carries it too.

- 477 more std-only modules in `bifrost-ui`, `bifrost-core`, `bifrost-render` and the SDK (686 since the matrix
  started) answer 665 gap reports across 142 frameworks, as pure models with host tests (5811 tests in those crates).
  No framework is at Gap any more. The models are not wired into the shell or the apps yet, so their rows stay Partial.
- Fixes found while integrating: a pending purchase refuses a second pending result instead of finishing with it;
  bookmark import no longer wraps the file's root list in an untitled folder; an ended live activity reports the state
  it had before its end time; service-management errors carry the official texts; DNS-SD instance names drop the root
  dot; PDF page layout keeps f64 rectangles.
- Testing: the test pin moves to `bifrost_core::pin`, and the shell's wall clock honours it, so Calendar's snapshot no
  longer breaks at midnight; Files reads its first folder at once under the pin, so a busy machine never snapshots
  "Loading…".
- Tools: `bifrost pc parity` gains the built share (the 80% deploy gate) and `apply`; the embedding cache is written
  atomically.

## 0.5.34 — 2026-10-12 (not published; shipped in 0.5.35)

Reference-SDK parity, waves 1 and 2 (the matrix: `docs/apple-parity-matrix.md`, `bifrost pc parity`):

- A parity matrix of every framework the official documentation lists for macOS 27 (246 frameworks and parts, 2413
  feature rows), with coverage per SDK, work themes and a published database view. Coverage rose from 26.0% to 33.8%.
- 209 new std-only modules in `bifrost-ui`, `bifrost-core`, `bifrost-render` and the SDK answer 420 gap reports across
  127 frameworks as pure models with host tests (2340 tests in those crates): controllers and bindings, find and
  replace, list markers, Bezier paths and gradients, tracking areas and cursors, screens and Dock badges, vCard and
  contact groups, fetch requests, persistent history and migrations, index sets and progress, signposts, tensors and
  compute graphs, colour cubes and filters, sync-engine change tokens, recurrence, geofences and more. Each has a note in
  `docs/parity/`. None is wired into the shell or the apps yet, so the matrix marks them Partial.
- Fixes found while integrating: histogram equalization no longer underflows below the first occupied bin; screen
  rotation ignores non-finite gravity; the weather error shows its own text instead of the deprecated trait default.
- Tools: `bifrost pc parity` (status, coverage, show, gaps, search with Titan embeddings and rerank, related, themes,
  apply, check, build, db-export), the official-docs reader `tools/parity/docc.py` and REA page evidence.

## 0.5.33 — 2026-10-11

Five SDK and shell parity rounds (docs/bifrost-ui.md has a section for each, with the gaps that remain):

- State restoration: apps save and restore their state (`encode_restorable_state` / `restore_state`, `invalidate_restorable_state` coalesces writes, sudden termination waits for the write). The shell records open windows and reopens them at login; the Restart and Shut Down alerts carry "Reopen windows when logging back in", and General has "Close windows when quitting an application". Notes restores the open note and folder.
- Window animations: windows animate open, minimize to their Dock tile and back, and zoom (Scale effect; the Funnel choice falls back to Scale). Reduce Motion cross-fades. Apps can set `animation_behavior` (default, document, utility, alert, none). Close does not animate yet.
- Undo: named actions ("Undo Typing", "Undo Move"), nested and per-event grouping, levels of undo, per-target removal; the Edit menu shows the action names. Notes and Files adopt it.
- Accessibility text: label, title and help are separate; text fields report caret, selection and visible range over AT-SPI, with text-changed and caret events for screen readers; `announce` sends live announcements; secure fields expose only their length.
- Workspace: open links and documents with the default app, open with a chosen app, reveal in Files, move to Trash (Put Back works), icons by file or type, frontmost and running apps, and launch, terminate and activate events. Every file goes through the app's document grants.

## 0.5.32 — 2026-10-11

- Login: the desktop no longer stays black behind the menu bar and dock after logging in. At unlock the compositor still held both desktop buffers (the lock surface's last frame and the one queued behind it), so the first desktop frame was skipped, and the idle shell drew nothing more until the next minute's clock tick. A frame skipped for want of a buffer is now redrawn when the next one is released; the wallpaper is on screen about 0.4 s after the password is accepted.

## 0.5.31 — 2026-10-11

- Login: the first key typed after the screen locks goes into the password field. The login window opened on the next frame, and the shell may not draw after a lock, so the first key was dropped and every login after a lock failed once (`the_first_key_after_a_lock_reaches_the_password_field`). Clicks get the same fix.
- Boot: the loader's progress bar fills to the end when the desktop is ready (from the share of the boot already done, 300 ms ease-out), then the boot frame fades into the login window, the reference boot order. Before, the bar was a static track.
- Lab installs: the sync carries the whole packed asset set (fonts, font library, stills, icons, sounds), and the installer prunes fonts and stills the set no longer has; fresh lab installs had the live medium's old assets and blank dock icons.

## 0.5.30 — 2026-10-11

- Fonts: BifrOSt ships only open-source fonts. The defaults are open lookalikes of the reference system fonts: Inter (system), JetBrains Mono (monospaced) and Noto Sans Symbols 2 (symbols). `tools/pack.py` fetches them from one pinned google/fonts commit and checks their SHA-256; the proprietary symbol stub is no longer packed.
- Settings > General > Fonts: choose the system and monospaced fonts from BifrOSt's font library (Geist, IBM Plex Sans, Source Sans 3; Geist Mono, Fira Code, Source Code Pro, plus fonts apps install in `/var/lib/springboard/fonts`). The choice applies to the shell and every app after Restart BifrOSt; a font that fails to load falls back to the default.
- Wallpaper: a fresh install starts on a random Icelandic photo.

## 0.5.29 — 2026-10-11

- Login window (stage 1 of the login round, ported from branch `login-hardened`): boot and every later lock show the reference's login window instead of "Press Return to unlock". The date sits over a large clock at the top; the avatar, full name and a password field sit low in the middle; a power button at the right of the status icons opens Sleep, Restart… and Shut Down…. The password is checked by the new root helper `bifrost-privd` (PAM service `bifrost-login`: faillock, 5 tries then 5 minutes; an in-daemon delay; fail closed on PAM errors). `/etc/bifrost/no-login` is the explicit lab bypass.
- privd: the privileged call sites (power, sleep, timezone, updates, store installs) go through `privd_client`, which runs the methods in-process while the shell is root and uses the socket only when `SPRINGBOARD_PRIVD` is set (stage 2, the shell as the user).
- Lock clock: the short date ("Thu Sep 10") over a semibold clock.
- Installer: a Look step after the account: Light, Dark or Auto, the accent colour and the wallpaper on one page; the page itself follows the pick, and the choice is written to the new system's `settings.ini` (`appearance_mode`, `accent_color`, `wallpaper`). Unattended presets take optional `appearance`, `accent` and `wallpaper` keys.
- Lab: `midget.py unlock` types the lab password (never printed; only key codes cross ssh) and every Enter-unlock in the tools uses it; `midget.py login-bypass on|off`.

## 0.5.28 — 2026-10-11

- WoW, the built-in game-streaming app, is removed from the shell together with its `wowstream-client` dependency and the landscape-forcing rotation path that only it used. First step of moving components out of the shell so they update without a restart (`docs/shell-decomposition.md`).
- Unlock: the desktop surface is mapped again after unlock, which fixes the black wallpaper and the compositor disconnect (cbd807d7).
- Menu bar: the text turns dark over light wallpapers (7f1900e6, 72722230). Wallpaper ids use underscores.

## 0.5.27 — 2026-10-11

- Display: the desktop runs at the panel's fastest refresh at its native size (165 Hz on the 2560x1440 lab monitor) by default. `video-arg` had booted in the firmware mode (59.95 Hz) unless `refresh=max` was set in `/etc/bifrost/display.conf`; now the fastest mode is the default and `refresh=firmware` in that file keeps the firmware mode (the monitor retrains its link on each mode change).
- Display (bare KMS path, no compositor): an inherited loader mode stays only when it is already the selected size at the selected refresh or faster; the 0.5.26 change alone did not reach the labwc desktop, which picks its mode through `video=`.

## 0.5.26 — 2026-10-11

- Bare KMS: an inherited loader mode is no longer kept when a faster refresh exists at the same size (the desktop itself runs on labwc, see 0.5.27).

## 0.5.25 — 2026-10-11

- Transparency round: window sidebars and toolbars blur the windows behind their window, not only the wallpaper (the presenter builds a backdrop without the front window, at most 30 Hz, only when a lower window changes).
- Materials follow the window's active state: an inactive window's sidebar and toolbar are opaque (`.followsWindowActiveState`); `.active` and `.inactive` fix it.
- Glass: saturation 1.8 in Light and 1.7 in Dark (was 1.45), specular rim only on menus, the dock and panels (not on window backgrounds), lit from the top left with an angle falloff; dither against banding; window contents filtered into the live backdrop (no shimmer).
- Increase Contrast turns Reduce Transparency on and locks its switch.
- Every window whose materials look active gets its own backdrop of the windows under it (`.active` windows behind the front one too); a backdrop rebuild skipped by the 30 Hz cap now wakes the loop and draws, instead of waiting for the next damage; menus and panels on chrome layer surfaces redraw their glass when the backdrop changes.
- SDK: vibrant labels (`widgets::Vibrant`, `blit_vibrant`: plus-lighter in Dark, plus-darker in Light, the material's alpha kept), used by `source_list` for labels, headers, counts and subtitles off the selection plate.
- SDK: `widgets::visual_effect` with every `Material` (14), `Blending` (behind / within window) and `State`; protocol `MaterialKind` has 14 kinds plus `MaterialState`, gated by the new `appearance::MATERIALS` bit (older shells get the legacy kinds).

## 0.5.24 — 2026-10-11

- seed-data copies asset files the data dir does not have yet on every start (never overwriting), so renamed icons and fonts reach existing installs; the 0.5.23 icon id rename left four dock icons blank on Midget until the files were renamed.

## 0.5.23 — 2026-10-11

- Compositor (round 58): live materials (bar, dock and menu glass blur windows behind them), window shadow no longer darkens translucent window materials (black slab fix), per-surface transforms and `shell/window_fx.rs`, new `bifrost_render::motion`. See `docs/gpu-first.md`.

- Dock (Golden Gate round 58): pointer magnification with a cosine falloff (prefs `dock_magnification` on/off, `dock_magnification_size` in pt, default 80, clamp 48..128, stored in `desktop-dock.ini`; `Change::Magnification` and `Change::MagnifiedSize(delta)` toggle and resize it); hit-testing follows the magnified icons. Eased in and out with `motion::Kind::DockMagnify`.
- Dock: the hover label fades in and out (`Kind::CrossFade`) and sits beside the icon on a left or right dock; launch bounce (`Kind::DockBounce`) from a click until the app's first window maps (10 s cap, none under Reduce Motion); the running dot of a pinned native client shows only while it has a window.
- Dock: sections pinned | recents | minimized windows and Trash with a divider between sections (`dock_rects_seps`); a minimized window gets its own slot right of the last divider and a click restores it.
- Dock: Option+Cmd+D toggles Automatically Hide and Show (`Shell::dock_toggle_autohide`, called from `hw_chord`); the dock slides in and out (`Kind::PanelReveal`, ease in-out) instead of vanishing. `Shell::dock_animating` keeps `next_wake` ticking while any dock animation runs.
- New for the compositor: `Shell::dock_icon_rect(app, window) -> Option<Rect>` is the screen rect (px, resting size, any dock position, valid even while autohidden) the minimize animation should shrink a window into: the window's own minimized slot when `window = Some(index into the desk's windows)` has one, else the app's icon slot, else the free slot left of the Trash.
- Colours used: none new (`DOCK_TINT`, `MENU_TINT`, `CELL`, `SEPARATOR_C`, `WHITE`).

Track overview (Golden Gate round 58). Colours used are existing named constants (`CARD_BG`, `FIELD_BG`, `SIDEBAR_PLATE`, `LABEL*`, `SELECTION`, `TINT`); no new ones.

- Overview (F3, Ctrl+Up): the windows of the current desktop in a non-overlapping grid scaled from their last drawn frames (never enlarged), the title under the hovered or keyboard-selected tile, click to focus, Esc or a click on the background to leave, arrows / Tab / Enter on the keyboard. A strip of desktops sits at the top with a "+" to add one (up to 8); click a desktop to go there, drag a tile onto a desktop or the "+" to move the window. Opens and closes with `motion::Kind::OverviewEnter` (instant with Reduce Motion).
- Desktops: Ctrl+Left / Ctrl+Right switch with a sliding veil and a name pill (`Kind::PanelReveal`). A desktop is a tag on each window; windows of other desktops are minimized and their own minimized state is restored on return. A window brought forward from the dock joins the current desktop.
- Apps grid (F4, the dock's Apps icon): paged grid of every app, search field (type to filter, Backspace, Enter opens the first match or the selection), page dots (clickable), swipe or scroll between pages (`Shell::overview_scroll`), press and hold then drag to reorder (order saved in `appgrid.order`, page turns at the screen edge), Esc closes. Columns and rows derive from the screen size and scale.
- App switcher: Cmd+Tab walks the windows with the icon strip, Shift reverses, Esc cancels, arrows move; Cmd+` cycles the windows of the front app (`Shell::switcher_key`, `switcher_same_app`).
- Keys are not yet routed here: `bin/shell.rs` / `hwkeys.rs` must call `Shell::overview_key(code, down, KeyMods, t)`, `switcher_key`, `overview_type(text)` and `overview_scroll(dx, dy, t)` before the other shortcut handling.

- Windows: `Tile`, `tile_rect`, `snap_zone` and `Desk::tile` / `return_to_previous` in `shell/desk.rs` (halves, quarters, fill, centre, previous size) with a unit test; not yet wired to drag preview or the Window menu.
- Keys: Ctrl+Cmd+F zooms the front desktop window (`Shortcut::FullScreen`). Values needed later: snap preview fill = accent at 20 % alpha, band = `ui.pt(8.)`.

- Control Center (desktop): Now Playing module under Sound (shown while a player session exists), on the same glass plate, transport by thirds. Not done this round: anchoring to the status icon, PanelReveal/MenuAppear motion (motion module absent in this worktree), Notification Center groups/widgets, banner stacking, Search preview pane.

- Kit: first-class sheets. `bifrost_ui::sheet` (layout, hit-testing, Return = default button, Escape = cancel button, every key consumed) and `bifrost_app::widgets::sheet` (dims and blocks the parent, panel hangs from the title bar at slide `progress` from `motion::Kind::SheetSlide`); `Sheet::alert` for alert sheets.
- Kit: `segmented`, `stepper` (13 x 22) and `focus_ring` (3 pt) widgets with metrics and hit-testing in `bifrost_ui::sheet`; gallery shows them. Focus ring colour is systemBlue at 45% (premultiplied 4,59,115,115), no new named constant.
- Contract tests for every new API (`ui/src/sheet.rs`, `sdk/src/widgets.rs`).

### Golden Gate round 58: Settings (track settings-apps)

- Appearance: accent colour now has Multicolour (controls use blue) beside the eight accents; `CONTROL_ACCENT`, `LINK`, `SYSTEM_BLUE` and the `SELECTION` pair resolve to the chosen accent through `Pair::now` (render/src/appearance.rs). New Highlight colour (text selection, `ui::highlight_now()`), Sidebar icon size (`ui::sidebar_icon_scale()`: 0.85, 1.0, 1.2), Show scroll bars (`ui::scroll_bars_always()` / `scroll_bars_fade()`), and a Transparency slider (`glass::set_clarity`) with the Reduce Transparency switch. Not yet read by the source list and scroll bar code (other tracks): the accessors are in `render/src/ui.rs`.
- Desktop & Dock: Magnification (switch and size), Minimize Windows Using (Funnel or Scale), Show Suggested and Recent Apps, Hot Corners page (four corners: None, Overview, Apps Grid, Show Desktop, Lock Screen, Search). Prefs keys in settings.ini for the dock and compositor tracks to read: `dock_magnify` (0/1), `dock_magnify_size` (0..1), `dock_minimize_effect` (0 Genie, 1 Scale), `hot_corner_tl|tr|bl|br` (0..5 per `pages::CORNER_ACTIONS`).
- New Keyboard Shortcuts page (read-only list of the shell's shortcuts) and Overview page: `overview_group_by_app` and `overview_switch_desktop` (0/1, both default on) for the Overview track.
- `pages::DeskPrefs` loads these keys; the shell calls `DeskPrefs::load(..).apply()` from `appearance_tick`.
- Not done: the adoption tool (`tools/sdk-adoption.py`) reports 19 of 22 apps on the kit; the remaining ones (daw, keyboard, remote) are outside `apps/*-desktop`, and calculator, clock and freeform have no sidebar by design, so no app chrome was moved without a build to check it.

## 0.5.22 — 2026-10-11

- bifrost-mcp `browser`: favicon, bookmarks, history, storage, styles actions; new `term` tool (send text and keys, read the screen, wait for text, tabs, profiles).

- Term: the shell now runs as the logged-in desktop user (setgroups/setgid/setuid in the child, cwd = home, HOME/USER/LOGNAME/SHELL/PATH/TERM/COLORTERM set, the account's login shell), never root unless the user is root; a pump reads at most 1 MiB per frame so output bursts cannot starve the UI.
- Term performance: pty reads are capped at 256 KiB per frame so `yes` and `cat` of huge files keep the UI and Ctrl+C responsive; scrollback lines configurable with `BIFROST_TERM_HISTORY`; new `bench_term_*` benches.
- Term: wheel scrolling sends mouse reports (1000/1002/1003, SGR 1006) to programs that asked for them; `Terminal::report_mouse` helper. New `docs/term-compat.md` lists emulator status and gaps (OSC 8, OSC 52 consent, rectangular selection, emoji widths).
- Term: agent control. The shell control port takes `term <verb> key=value ...` (state, new_tab, select_tab, close_tab, send_text, send_keys, read, resize, set_profile, clear) and the MCP gains a `term` tool with `read_screen` and `wait_for` (docs/bifrost-mcp.md).
- Term: `shell/terminal.rs` split into `shell/terminal/` modules (window, tabs, session, view, input, find, profiles, menus, control, ssh, tests); no behaviour change.
- Term: Terminal.app profile table (Basic, Pro, Homebrew, Ocean, Novel, Silver Aerogel, Clear Dark/Light, Red Sands; Basic follows Light/Dark), URL detection and OSC 133 prompt-mark helpers in `terminal/profile_table.rs` (data and pure functions with tests; not yet wired to the UI).

## 0.5.21 — 2026-10-11

- bifrost-mcp `browser`: `inspect {x,y}` (element at a page point, highlighted), `downloads`, screenshot `scale`.
- SDK web view: tab discard and revive, parser hardening against garbled engine lines, byte caps.

## 0.5.20 — 2026-10-11

- Fonts: noto-fonts-cjk, noto-fonts-emoji and ttf-dejavu are installed, so the Browser draws CJK and emoji text.
- SDK web view: frames carry their view id (a late frame of a hidden tab is dropped), page dialogs, context and
  select menus, engine ping/pong watchdog, suspend/resume; control replies no longer block the UI thread.

## 0.5.19 — 2026-10-11

- bifrost-mcp: new `browser` tool for full control and debugging of BifrOSt Browser: tabs, navigate (wait for load),
  back/forward/reload, `eval`, console and network logs with cursors, DOM queries, page screenshots (viewport, rect,
  selector, full page) returned as images, wait-for conditions, viewport/zoom/find/user agent, page input.
- bifrost-mcp: app control and debugging completeness: image results from app methods, waits on app state,
  per-app screenshot, logs, crash and perf, structured error codes.
- SDK: async replies on app control sockets (`RpcError::pending`, `control::complete`) and token-correlated
  web view eval, screenshots and input (`webview_dev`).

## 0.5.18 — 2026-10-11

- Fix: unlocking after display-off could restart the session ("layer_surface has never been configured"). A
  configure for a destroyed chrome surface is now matched by object, not id, so a re-created overlay never
  attaches a buffer before its own configure.
- Bench: the after-lock phase presses Enter again while the screen is still locked.

## 0.5.17 — 2026-10-11

- Fix: the GPU pool trim deadline is armed only when the spare drawable is free. The compositor holds the
  on-screen buffer, so 0.5.16 re-armed a 1 ms timeout and the shell woke ~130 times a second.

## 0.5.16 — 2026-10-11 (Snow Leopard round 7, SDK: Observable, spell checking, Quick Look)

- Control: `lock` verb (same as Apple menu > Lock Screen); `tools/midget.py lock`; the Midget bench records shell
  PSS after a lock cycle (`midget_pss_shell_after_lock_mib`, budget 80 MiB).
- Memory: closing an external window drops its cached body/window pixel buffers, the icon cache is capped at 48
  entries, and the heap is trimmed afterwards.
- Settings: Energy page (from Battery) sets the system sleep timeout; applied without a restart.
- Idle: menu-bar CPU/RAM refresh every 5 s with no menu open (was 2 s); the GPU pool trim is a one-shot deadline.
- SDK: `Observable<T>` with `sink`, `map`, `remove_duplicates` and cancel-on-drop `Subscription` (Combine/Observation
  parity); `SpellChecker` (NSSpellChecker: check, correction, learn/ignore); Quick Look `PreviewPanel`,
  `PreviewItem`, `ThumbnailCache` and wire messages (panel not drawn yet).

## 0.5.15 — 2026-10-11 (Snow Leopard round 6)

- Memory: unlocking frees the dimmed and fitted lock wallpaper copies and the main loop trims the allocator
  afterwards, so a lock cycle no longer leaves the shell ~35 MiB larger.
- Presenter: the full-screen background starts with one GPU drawable, adds a second only while the compositor
  holds the first, and frees it after 5 s idle.
- Sleep: Lock Screen and display-off never suspend the PC (macOS parity). System sleep after display-off follows
  `system_sleep_minutes` (default never); Apple menu > Sleep suspends immediately.
- bifrost-portal: ignores inotify events for unrelated runtime-dir files (was ~2 wakeups/s idle).

## 0.5.14 — 2026-10-11 (Snow Leopard round 5, wallpaper memory)

- Memory: the shell owns its wallpaper images (no `'static` views). On the GPU presenter path the fitted home
  wallpaper is freed once it is uploaded as a texture and refitted only for the lock screen or Today; the lock
  wallpaper and its dimmed copy exist only while the lock screen shows. Shell PSS 95 → 71 MiB on Midget.
- labwc: the window-switcher setting uses the current `<windowSwitcher><osd show="no"/></windowSwitcher>` form.

## 0.5.13 — 2026-10-11 (Snow Leopard round 5; Apple SDK parity 2)

- Memory: the shell fixes glibc's mmap threshold (256 KiB), trim threshold and arenas at start
  (`bifrost_core::host::tune_allocator`, `mallopt`), so freed full-screen buffers stop fragmenting `[heap]`.
- Compositor: labwc indexes the small `hicolor` icon theme instead of Adwaita (its heap held the Adwaita index).
- Menus: an app menu titled File, View, Window or Help merges into the shell's menu of that name, its items first
  (`NSApp.mainMenu` parity); the gate rule CHR-01 now flags only a second Edit menu.
- SDK: printing (`bifrost_app::print`: `PrintInfo`, `PrintOperation`, CUPS printers, Save as PDF), `Locale` and
  formatters (number, date, relative date, byte count, measurement, list; en and is) and `localized!` with
  `Localizable.strings`, `SMAppService` login items and `NSStatusItem` menu-bar extras (Settings › General › Login
  Items), `AttributedString` with RTF/HTML and `RichTextView`, Core Spotlight indexing (`bifrost_app::spotlight`,
  Spotlight's Documents section), `NSFileCoordinator`/`NSFilePresenter`, cross-app `DistributedNotificationCenter`
  through the shell, table sort descriptors and column model, `FontPanel` and `ColorPanel` models.
- `NotificationCenter` delivers on the posting thread, as Apple's does.

## 0.5.12 — 2026-10-11 (Snow Leopard round 4; Apple SDK parity)

- Idle wakeups: the Wi-Fi manager ticks every 5 s once settled (700 ms only while scanning, joining or on DHCP);
  nm-worker blocks on requests and D-Bus signals (no 60 s poll) and backs off its re-read while nothing changes;
  input hot-plug uses inotify on `/dev/input` instead of a 2 s rescan.
- Memory: the idle trim also drops the chrome text cache and the menu-bar backdrop; the dock icon cache is bounded.
- Compositor: labwc ships no SVG button icons (no librsvg), no window-switcher OSD, two malloc arenas with early trim,
  and `WLR_RENDERER=gles2`.
- SDK: `URLSession` (`bifrost_app::url`: data and download tasks, `URLCache` with max-age and ETag),
  `FSEventStream` (`bifrost_app::fsevents`), `JSONEncoder`/`JSONDecoder`, `PropertyListEncoder`/`Decoder` and
  `codable!` with field defaults (`bifrost_app::coding`), `NotificationCenter`, `NSPasteboard::general()` with typed
  items shared between apps (ToShell/ToApp 0x190) and offered to Wayland clients with `wl-copy` (new dependency
  `wl-clipboard`).
- Bench: `tools/bench.py --midget --apps` times a cold launch of the first five installed apps and reads their PSS.

## 0.5.11 — 2026-10-11 (Snow Leopard round 3, memory)

- Memory: `bifrost_core::host::pressure_relief()` (`malloc_zone_pressure_relief` parity) returns freed heap to
  the kernel; the shell calls it once each time it goes idle after drawing. Shell PSS 108 → 90 MiB on Midget.
- Bench: the idle sampler settles 10 s so its own login's bus traffic is not counted (dbus-broker idles at 0/s).

## 0.5.10 — 2026-10-11 (Snow Leopard round 3)

- Idle: the Date & Time sync line reads systemd-timesyncd's state from `/run` and the unit links instead of
  spawning `timedatectl` twice every 15 s (each spawn opened a system-bus connection and started timedated;
  dbus-broker woke in bursts of ~25).
- Testing kit: pinned clock and random seed (`testing::pin`, `unix_secs`, `system_time`, `seed`); Books,
  Console, Games and Weather snapshots are deterministic and gated again.
- Compliance gate: APP-10, the app icon must be a 180x180 PNG.
- Bifrost MCP 0.5.0 ships in the package (`lifecycle` status, restart, netboot, reinstall).

## 0.5.9 — 2026-10-11 (Snow Leopard round 2)

- Fix: black app windows. The 1.5 s quit deadline used the whole-app cgroup kill, which also killed an instance
  reopened in the meantime; it now kills only its own instance, reopening ends the old one first, and a failed
  launch closes its window.
- Idle: the recovery-chord watcher waits on evdev/inotify instead of polling every 200 ms (65 of the shell's
  71 wakeups/s); Bluetooth and Wi-Fi managers wait for their next deadline; every shell thread is named.
- Presenter: no frame when only ambient info changed (uptime, storage, sync text); desktop damage is exact
  (unchanged pixels give empty damage).
- Host statistics and ProcessInfo (Foundation/Mach parity): `bifrost_core::host` (`host_statistics64`-style CPU
  load and VM statistics, per-CPU, `task_vm_info`, `os_proc_available_memory`), `bifrost_app::ProcessInfo`
  (physical memory, processor counts, uptime, thermal state, Low Power Mode, observers), `begin_activity` /
  `end_activity` (ToShell 0x161). The CPU and RAM menu extras read these counters on the main loop every 2 s
  (1 s with the menu open, 10 % tolerance, coalesced), nothing while the screen is off or locked; no sampler thread.
- SDK: dispatch pool width capped at the core count across QoS lanes; `async_after_leeway`;
  `DispatchSource::memory_pressure`; purgeable `bifrost_app::cache::Cache` (NSCache parity) evicted on PSI memory
  pressure; glyph and emoji caches moved onto it; `dispatch::drain_main` for tests.
- Footprint: wallpaper copies made lazily (lock-screen dim and Today frost only when shown).
- MCP `lifecycle` tool: `status`, `restart`, `netboot`, `reinstall` (guarded by lab hardware and a confirm phrase);
  unattended installer preset on the ESP; `tools/midget.py reinstall`.
- Apps: Podcasts 1.3.2 and Books 1.2.2 use dispatch queues instead of threads and polling.

## 0.5.8 — 2026-10-10 (Snow Leopard round 1: idle means idle)

- Idle: `bifrost-bootweave` blocks in `wait()` after the first frame (was 50 wakeups/s for the whole session);
  netd and audiod status tickers sleep until a client subscribes; updated ticks every 15 s unless checking or
  downloading; bifrostd supervises every 2 s; bifrost-portal waits on inotify.
- Shell: the CPU/RAM sampler and its 1 s deadline run only when a CPU or RAM menu extra or menu is shown; the
  app resource writer parks when unread; the stall watcher wakes every 5 s; Date & Time sync status
  (`timedatectl`, a new D-Bus connection per call) is cached for 15 s; the NetworkManager signal match drops
  AccessPoint noise. Debug log "idle: N wakeups, M frames" once a minute.
- SDK: `bifrost_app::dispatch` (Grand Central Dispatch parity): main, global (QoS) and serial queues,
  `async_after`, `sync`, `DispatchGroup`, `DispatchWorkItem`; main-queue work wakes the app loop by eventfd.
- Sudden termination (`ProcessInfo` parity): `ctx.enable/disable_sudden_termination()` (ToShell 0x160). The
  shell kills a terminable app at once and gives the others 1.5 s to quit without blocking the UI; power off,
  restart and OTA restart stop apps through one shared deadline. Calculator, Console and Weather opt in.
- Tools: `tools/midget-idle-measure.py` measures every process (CPU, wakeups, i915 flips); `bench.py --midget`
  records idle, memory, thread and size keys and checks `docs/budgets.toml`; `bifrost pc acceptance --midget`
  fails over budget. `midget.py` finds the lab key from a worktree.

## 0.5.7 — 2026-10-10

- Security, Unified Logging: a log subsystem belongs to the app whose manifest declared it (the registry records
  its `owner`). A second package cannot claim it, and an entry from another app (judged by journald's
  kernel-set `_EXE`) or an app's entry under the shell's subsystem loses its subsystem and category and is
  marked `[claimed subsystem …]`. Applied in `LogStore`, `bifrost-log` and Console.
- SDK: `UserDefaults::register` (`register(defaults:)`), a registration domain that is read when a key has no
  value and is never saved. `Ctx::testing` and `Ctx::take_sent` let tests check what an app sends to the shell.
- bifrost-mcp 0.4.1: `logs` filters by `subsystem` and `category` (exact journal fields).
- Tests: the menu-bar map test runs only on the desktop build (the workspace run has no menu bar). SDK coverage
  floor raised to 79 %.

## 0.5.6 — 2026-10-10

- Testing, SDK first (XCTest parity): `bifrost_app::testing` gives apps a headless `TestWindow` (click, key, type,
  resize, draw) and XCUIElement-style queries by role and label (`element`, `press`, `set_value`, `assert_element!`).
- Contract tests for the SDK API: UserDefaults, Keychain, UserNotifications, app/controller host, ScrollView, every
  control and form, intents, webview and runtime; `sdk/tests/kits.rs` now checks Apple behaviour through
  `bifrost_app::controls` instead of reachability. SDK API coverage 79 % (`tools/check-sdk-coverage.py`).
- Snapshot tests: ten kit controls in Light and Dark at 1x and 1.5625x against committed goldens
  (`BIFROST_BLESS=1` to accept changes).
- Update path tests (`updated`, shell `update.rs`/`ota.rs`), netd and audiod, and first tests for Weather, Contacts,
  Freeform, Games, Shortcuts, Voice Memos and Podcasts.
- `bifrost pc acceptance [--quick] [--midget]`: one command for workspace tests, coverage, tool tests, app smoke and
  benchmarks.

## 0.5.5 — 2026-10-10

- App Store apps update by themselves, like macOS "Install application updates from the App Store": with Settings ›
  Software Update › Automatic Updates on (the default), each check installs newer versions of apps that are not
  running. An open app keeps its offer and updates on a later check after it quits, so no window is restarted
  under the user. System packages still always update in the background.
- Update checks run every 15 minutes (the docs said 6 hours); the test is renamed to match.

## 0.5.4 — 2026-10-10

- Unified Logging (Apple `Logger`/`OSLogStore` parity): `bifrost_app::log::Logger(subsystem, category)` with debug,
  info, notice, error and fault, `private()` redaction, journald fields `BIFROST_SUBSYSTEM`/`BIFROST_CATEGORY`;
  subsystem registry from `[[log.subsystem]]` in manifests; `bifrost-log show|stream|config|subsystems`.
- Console 1.0.0, a system app: live log stream, Info/Debug toggles, search tokens, Crash Reports, /var/log files.
- SDK retained views: `ScrollView` (NSScrollView) and `resignFirstResponder` (pop-up menus close on an outside click).
- bifrost-mcp 0.4.0: `logs`, `crash`, `dev` (upload, install, restart), `perf`, screen-change waits and screenshot
  diffs; the HTTP endpoint needs the token on loopback too.

## 0.5.3 — 2026-10-10

- Cmd+Space opens Spotlight from anywhere, including an empty desktop or a native app in front: the compositor
  binds Super+Space and asks the shell over its control port (`spotlight`).
- `bifrost pc mcp call|bench|eval|push|token`.

## 0.5.2 — 2026-10-10

- Spotlight on the desktop: Cmd+Space opens it over any app (it only opened on the home screen), as an overlay
  above every window that takes the keyboard while open, so typing reaches it even over native apps; the windows
  stay drawn beneath it. The top hit's label follows Light mode.
- `bifrost-mcp`: agent access to the machine (MCP) as a system service, on by default: HTTP on port 7420 with a
  bearer token (`/var/lib/bifrost-mcp/token`) and `/run/bifrost-mcp.sock`. See `docs/bifrost-mcp.md`.

## 0.5.1 — 2026-10-10

- The running shell notices apps installed or updated from the command line (`--install-store-app`,
  `--install-package`) within a second, so they open from the Dock and Spotlight without a restart.
- Assistant 1.0.0 in the PC App Store: a chat app for Claude (Sonnet 5.5 by default).

## 0.5.0 — 2026-10-10

- App Store follows Light/Dark; AppKit-named semantic colours (`ui::system`, SDK `kit::colors`) and a guard test
  that fails on dark-only window or content fills.
- Third-party windows get macOS title bars from labwc (traffic lights on the left, centred title) that follow
  Light/Dark; external apps read the scheme from the Settings portal on a session bus.
- ArtCraft apps launch as native Wayland windows from the Dock, Spotlight and the App Store.
- MIT licence.

## 0.4.99 — 2026-10-10

- Apps follow a Light/Dark switch at once: the SDK repaints the whole window on a scheme change, background
  windows included (they kept the old scheme in the parts that had not changed). 17 apps rebuilt.
- Term follows Light/Dark (Basic profile) unless a theme was picked; DAW 0.2.4 and Remote 0.1.8 follow it too.
- PC App Store: 12 ArtCraft apps (PhotoCraft, VectorCraft, FilmCraft, EffectCraft, LightCraft, DesignCraft,
  PDFCraft, WordCraft, GridCraft, DeckCraft, CADCraft, SoundCraft) packaged from upstream releases
  (`upstream.toml`); a daily watcher publishes new upstream versions so they show up in Software Update.

## 0.4.98 — 2026-10-10

- App Store installs read only a manifest's top-level keys, so apps with `[[intent]]` tables (Notes 1.8.0,
  Reminders 1.9.0, Calculator 1.3.0) install instead of failing with "shortcut must be a string".

## 0.4.97 — 2026-10-10

- Software Update no longer reboots while pacman is still building the initramfs. `system-prepared` is set before
  pacman runs; `bifrost-update-guard apply` now waits until the system is staged (ESP `pending=0`, pacman lock
  released), so the new system does not boot a truncated initramfs and roll back (0.4.95 and 0.4.96 rolled back on Midget).

## 0.4.96 — 2026-10-10

Term, Terminal.app parity.
- Key equivalents while Term is in front: ⌘C copy selection (never ^C), ⌘A, ⌘F find, ⌘G / ⇧⌘G next/previous,
  ⌘K clear scrollback, ⌘= ⌘- ⌘0 zoom, ⌘T new tab, ⌘W close tab, ⇧⌘] / ⇧⌘[ and ⌘1-⌘9 switch tabs; shown in the menus.
- Find tints every visible match and shows "n of m".
- Double tap selects a word, triple tap a line; pasted control characters are dropped (bracketed paste stays intact).
- Tab captions name the running job; closing a tab with a running job asks first; Duplicate Tab opens in the same folder.
- A bell marks a background tab in orange; Clear screen sends Ctrl+L.

## 0.4.95 — 2026-10-10

Apple-parity SDK round: the AppKit layer the SDK was missing, plus the services around it.
- Retained views (`bifrost_app::view`): NSView/NSResponder/NSViewController counterparts with hit testing, first responder
  and key-view loop, responder-chain actions and menu validation, per-window undo, nested accessibility, partial redraw.
  Controls: Label, Button, Switch, Slider, Segmented, Checkbox, Stepper, ProgressIndicator, TextField, PopUpButton,
  Form/Section, Stack (with NSStackView distributions).
- Auto Layout: a Cassowary constraint solver with priorities, anchors, layout guides, intrinsic size, hugging and
  compression resistance.
- Core Animation parity: animated frame, opacity and background; implicit 0.25 s transactions, springs, completion,
  Reduce Motion.
- Text input client: marked text (IME composition), caret rect, surrounding text, return-key types and accessories; a
  grapheme-aware TextBuffer with emacs and macOS bindings; TextField uses it with pointer caret and drag selection.
- Apps publish their own main menus (NSMenu) with validation; the desktop bar shows them and routes key equivalents.
- Documents (NSDocument): edited state with the close-button dot and "Edited" title, autosave in place, untitled
  restore, Save As, Duplicate, Rename, Move To, Revert, the unsaved-changes alert and a standard File menu.
- Open and save panels: multiple selection, folders, New Folder, a places and Recents sidebar, last folder.
- Keychain: per-app encrypted secret storage brokered by the shell (`keychain` permission).
- User notifications with categories, actions and text replies; Dock badges; share sheet (NSSharingServicePicker);
  typed drag and drop between app windows.
- App Intents: typed intents and entities over the control socket; Spotlight lists app shortcuts.
- Accessibility: AT-SPI2 bridge (feature `atspi`) with hierarchy, actions and a ProgressIndicator role.

## 0.4.94 — 2026-10-10

- The CPU and RAM readouts are back in the menu bar, left of battery, network and Control Center.
- Settings › Menu Bar turns each menu extra on or off (CPU Usage, Memory Usage, Battery, Wi-Fi and Network,
  Control Center); the clock always shows.

## 0.4.93 — 2026-10-10

Golden Gate parity, round 2.
- System menu (the BifrOSt mark) before the app menu: About This Computer, System Settings…, App Store…, Sleep,
  Restart…, Shut Down…, Lock Screen. App menus are About, Settings…, Quit; Show Desktop is in Window.
- Menu extras like macOS: battery, network, Control Center and the clock (CPU/RAM readouts are gone). The clock opens
  Notification Center.
- Desktop Control Center and Notification Center as live glass panels under the menu bar.
- Spotlight: floating search capsule and a separate glass results card.
- Windows: one 17 pt radius, shadows measured from the reference, a 1 px rim, and vibrancy behind app sidebars and
  toolbar bands (GPU presenter).
- Reopening a running app raises it over native windows.
- Native apps follow Light/Dark through the freedesktop Settings portal (`bifrost-portal`).
- SDK: WebView (WKWebView parity), 59 glyphs, toolbar_layout, centred and pull-down toolbar items, two-line and
  avatar sidebar rows, collapsible sections, player capsule, popover, mini month, card grid, window size keys,
  destructive menu items; text fields follow Light/Dark.
- Lab: `springboard --install-package PATH`, `tools/pc-app.py pack`.

## 0.4.92 — 2026-10-10

Golden Gate parity, round 1 (measured against the reference captures; `docs/golden-gate-parity-audit.md`).
- Menu bar: transparent over the wallpaper with a soft text shadow; the open menu's title sits on a light glass capsule.
- Menus: 6 pt below the bar, systemBlue highlight capsule, hairline separators inset 12 pt, spaced shortcuts ("⌘ ,"),
  whiter material, and per-row symbols (Settings… has the outlined gear).
- Traffic lights 14 pt on a 23 pt pitch.
- Dock: lighter glass with a bright rim and a 24 pt radius, a divider, and Trash at the end (opens Files at Trash).
- SDK: 36 pt toolbar capsules, 20 pt toolbar and sidebar glyphs, `with_dot` sidebar rows, `folder_icon`, `search_button`.

## 0.4.91 — 2026-10-10

- Appearance changes live from the lab: `bifrost pc midget appearance light|dark|auto` goes through the shell's control
  socket, so open windows stay open (it used to restart the shell).
- New installs get Files from the App Store next to the Browser.

## 0.4.90 — 2026-10-10

- 0.37 s faster boot to the desktop (VM, first desktop frame 1.94 s to 1.56 s after kernel start): the command line
  carries an empty `systemd.tty.rows.console=`, so systemd no longer waits 333 ms for a console size reply that never
  comes. Existing installs get it from `video-arg` on their next session; the live ISO with its next build.

## 0.4.89 — 2026-10-10

- Boot benchmarks per step (`tools/boot-bench.py`): firmware and loader times from the BifrOSt loader, kernel, initrd,
  userspace and desktop milestones on one clock.
- One modeset: the loader sets the panel's native mode; the installer keeps the mode the driver inherited.
- The installer and the desktop start on the loader's frame and cross-fade into their first screen (450 ms).
- Faster installed boot (VM, kernel start to first desktop frame 4.67 s to 1.94 s): busybox initramfs that mounts the
  subvolumes before switch_root, the desktop and seatd start as soon as udev is up, session work moved off the path.
- Faster installer boot: the installer starts as soon as udev is up; lab sync no longer blocks it; no microcode in the
  live initramfs.

- The installer copies the BifrOSt EFI loader to `EFI/BIFROST/BOOTx64.efi` and registers that path as the firmware
  boot entry; `EFI/BOOT` stays as the removable-media fallback.
- Update recovery reads and restores the command line from `EFI/BIFROST/boot.cfg`, the file the loader boots first.
- The installer ISO boots UEFI only, through the BifrOSt loader alone: no systemd-boot, syslinux, memtest or EFI shell.

## 0.4.88 — 2026-10-10

- Replace Plymouth's boot handoff with the Rust Bootweave supervisor; the EFI/simpledrm frame remains the single visual
  bridge into the existing desktop session.
- Remove Plymouth from the installed and live package, initramfs, command line and splash-unit paths.
- Record compositor-ready and desktop-first-frame milestones for boot smoothness evaluation.

## 0.4.87 — 2026-10-10

- Start the desktop service without waiting for network/user-session ordering; BifrOSt can draw as soon as DRM and
  seat setup are ready.
- Disable unused LUKS, LVM, RAID and device-mapper initrd probes on the single BifrOSt root layout.

## 0.4.86 — 2026-10-10

- Rename the remaining legacy Android-era crate, service, environment and tooling identifiers to BifrOSt.
- Keep the Golden Gate Files shell adoption intact across Light and Dark appearance modes.

## 0.4.85 — 2026-10-10

- Expand semantic app-control roles for toggles, checkboxes, sliders, segmented controls, static text, menu items and windows. Existing role wire values remain stable; new roles are append-only.
- Keep the broader Apple-style SDK roadmap in `docs/bifrost-ui.md`: app menu transport, drawable standard controls, accessibility hierarchy and value setting, App Intents, share sheet, actionable notifications, typed drag and drop, protected secret storage, text composition, and complete document panels.

## 0.4.84 — 2026-10-10

- Golden Gate SDK kit (batch 1 of `docs/golden-gate-sdk-adoption.md`): every SDK widget follows Light and Dark; toolbar buttons are glyphs in glass capsules with groups and a selected segment; a search field (`NSSearchField`), a path bar (`NSPathControl`), source-list rows with glyphs and a neutral bold selection, and 28 line glyphs named after SF Symbols.
- Full-size app windows: an app with `window = "full-size"` in its manifest gets no title row. The traffic lights sit over its sidebar, and its toolbar band moves the window except over the lights and the app's own controls (on the Wayland compositor path too).
- `tools/sdk-adoption.py` reports how far each app has moved to the kit.

## 0.4.83 — 2026-10-10

- Light and dark across apps: the shell tells each app the scheme it shows (`ToApp::Appearance` with the `LIGHT` bit, and `Configure::dark` on start). The SDK sets its process scheme from that, so every `Pair` colour (`LABEL`, `CARD_BG`, `DETAIL_BG`, `SEPARATOR_C`, `SELECTION`, ...) an app draws follows Light or Dark, including Auto. Apps no longer see the shell's dark colours only.
- Files uses the scheme colours (label, card, detail background, separators, selection with white text on the blue row).
- Wallpaper grid: 16:9 landscape thumbnails (0.4.82).

## 0.4.82 — 2026-10-09

- Settings > Wallpaper shows the stills as landscape 16:9 thumbnails on a landscape screen. The grid cells and the enlarged preview take the screen's aspect (they were portrait), and each thumbnail is a centre cover-crop of its still, as the desktop backdrop is. Thumbnails are 320 x 180 (`pack.py` THUMB). The grid no longer stretches the portrait art.
- `midget appearance light|dark|auto` (tools/midget.py): sets Displays > Appearance on the Midget, restarts the shell, unlocks, and checks the stored mode.

## 0.4.81 — 2026-10-09

- Open and save panels for apps (`NSOpenPanel`/`NSSavePanel`): an app with the new `documents` permission calls `ctx.open_document` or `ctx.save_document`; the shell shows the panel over the desktop and the app receives the chosen file as an open descriptor, never its path. Saves go to a temporary file and replace the destination only on `commit_document`. Grants can be reopened later and notice a replaced file. The listing and every open run as the account, beneath the home, without symlinks.
- The panel follows the macOS panels: Documents first, alphabetical, filters by type (UTI conformance) or extension, Return/double click/Esc/arrows, a name field for saves, light and dark.
- A test-only keyboard field no longer warns in release builds.

## 0.4.80 — 2026-10-09

- Document grants, first half (`NSOpenPanel`/`NSSavePanel` groundwork, docs/file-portal-design.md): the shell's broker opens a file the user chose as the account, beneath the home, refusing symlinks and `..`, and saves through a temporary file with an atomic rename. Per-app grant records notice a replaced file. Wire messages `DocOpen`, `DocSave`, `DocCommit`, `DocAbort`, `DocReopen`, `DocRelease`, `DocGranted`, `DocError`. Not reachable from apps yet (no panel, no SDK call).
- Safety: `ioctl_ptr` and the GPU `cstr` helper are `unsafe fn`s (they read through caller pointers); `cargo clippy` has no errors.
- The PC shows no on-screen keyboard, in the shell or in apps (`keyboard::set_disabled`, set by the shell at startup): nothing is shown, reserved or spawned. The keyboard package and code stay for the day a touch screen needs them.

## 0.4.79 — 2026-10-09

- Fix: on a desktop (pacman updates) the scheduled check never reached the App Store, so system packages such as the keyboard were never installed or updated automatically. The store check now runs on every platform; only the OS-server lookup is skipped where pacman answers.

## 0.4.78 — 2026-10-09

- Agent API, closed loop: the element map reads Settings from the page's real source, so rows that depend on live state (Software Update's check/download/install, Storage, Wi-Fi, notifications) are listed. An open alert is `os/settings/alert` with its buttons, and other Settings presses are refused while it is open. `ax` with no prefix lists everything; an exact app address (with dots) resolves.
- `tools/midget.py wait NAME [--value V] [--gone] [--timeout S]` and `ota` (the full update by name).

## 0.4.77 — 2026-10-09

- The on-screen keyboard is a standalone system app, `is.olibuijr.keyboard` 1.1.0 (`kind = "system"`, `provides = ["keyboard"]`, crate `apps/keyboard`, binary `bifrost-keyboard`). No keyboard code remains in the shell or in any app: the 3,400 lines (layout, prediction, emoji, floating/split, trackpad) moved into the app, with their 25 tests.
- The shell keeps a thin client (`shell/keyboard.rs`): show/hide and the slide, touches in, the app's frame composited at the bottom edge, typed text back as `Out`. App processes (Notes, Contacts) get the same `Keyboard` type with an app backend: it asks the shell for the keyboard (`WantsKeyboard`) and reads its height from `safe_bottom`.
- Protocol and SDK first: `ToApp::KeyboardConfig`, `ToShell::KeyboardOut` (text, backspace, enter, hide, named keys, height, click, language), `App::keyboard_config`, `Ctx::keyboard_*`. Only the keyboard app's output is accepted by the shell.
- The shell owns Settings (input languages, caps lock, "." shortcut) and sends them in the config, so the sandboxed keyboard needs no access to the shell's files.
- A `system` package with an `exec` that `provides` something is spawned on demand (`shell::kinds`, `docs/apps.md`). `tools/pc-app.py publish keyboard` builds and packs it.
- Agent API: an element map of the OS and apps, generated from page specs, menus, the app list and app nodes (`bifrost_ui::ax`, `shell/axmap.rs`, docs/agent-api.md). `ax list [prefix]` and `ax press <name>` on the control socket; `tools/midget.py ax` and `press`. Apps on the kit are mapped automatically; others declare with `controls::ax::record`.
- Fix: `bifrost-notes` compiled again (`attach::latest_image` took only `&[&str]`).

## 0.4.76 — 2026-10-09

- Fix (0.4.75 regression): clicking the Network or Power status icon opened Control Centre's reveal instead of the menu; a press on a status button opens its menu again, and swipes that start elsewhere in the strip still reach Control Centre.
- The lab installer's update channel and `tools/pc-publish.py` serve the repo on Titan's home address (192.168.1.12:8079), where Midget on VLAN 20 can reach it.
- Tests: the 69 shell tests ignored in 0.4.75 run again (the test shell is the tablet presentation, `desktop_shell()` is the desktop); two stale assertions were updated.

## 0.4.75 — 2026-10-09

- A physical keyboard types with the installed layout (`/etc/bifrost/keyboard.env` or the session's `XKB_DEFAULT_*`), not always U.S.: Icelandic, German and the other installer layouts type their own letters.
- Right Alt (AltGr) types the third level (AltGr+E is € on German) while Left Alt stays a shortcut; dead keys (Icelandic acute) compose with the next letter.
- A swipe down from the right of the status strip opens Control Centre on the PC desktop; the menu bar no longer takes that touch.
- 69 shell tests that assume the tablet layout (fullscreen Files, tablet dock order) are `#[ignore]`d with a reason. They are to be ported to PC behaviour; the suite is green (2273 passed).

## 0.4.74 — 2026-10-09

Settings fixes and the rest of the light/dark work.

- Text drawn in the scheme's label colour was almost black in both schemes (tint maths divided by 255 twice); Settings text is now white in Dark and near-black in Light.
- Settings: Appearance gains an accent colour (eight swatches, used by the sidebar selection, switches and sliders) and a Clear/Tinted Liquid Glass style; the preference is `accent_color`.
- Switches and sliders follow the scheme (light tracks in Light) and use the macOS 38 x 22 pt switch; traffic lights are glossy spheres with a rim and light grey when the window is inactive.
- The sidebar no longer draws a ghost "Settings" title or a nav bar when scrolled, and scrolled rows pass under a clear traffic-light zone.
- Wi-Fi sheet, Time Zone, time picker, Wallpaper and Storage pages follow the scheme.
- About lists this computer (BifrOSt version, model, processor, memory, serial) instead of iPad media counts; "AirDrop" is now "Nearby Share"; "SpringBoard" in user-facing text is "BifrOSt"; removed the dead VPN, Ask to Join Networks, Location Services and Analytics rows.
- Software Update check works without `fakeroot` (a root shell refreshes the databases itself) and `fakeroot` is now a runtime package.
- Developer: `PREVIEW_DIR=/tmp PREVIEW_PAGES="Display>Appearance" cargo test preview_settings -- --ignored` renders Settings in both schemes to PNG.

## 0.4.73 — 2026-10-09

Golden Gate look for Settings and the desktop shell, with a system-wide Light, Dark and Auto appearance.

- Settings: rebuilt to the Golden Gate layout: borderless full-size window with the traffic lights in the sidebar, 232 pt sidebar with a search field and account block, back/forward pill, header card, 49 pt rows with drawn icons, 13 pt radii.
- Appearance: Light, Dark and Auto (light 07:00 to 19:00) under Settings > Appearance, with a 280 ms cross-fade; the menu bar, menus, dock and Settings follow it.
- Menus, dock, alerts and Spotlight: measured Golden Gate sizes (24 pt menu rows, 14 pt menu radius, 21 pt dock radius, capsule alert buttons, 520 pt Spotlight).
- Cmd/Alt/Ctrl+Tab on the desktop: a glass app strip that steps through windows while the modifier is held and raises the selection on release.
- The Android tablet code and docs are removed.

## 0.4.72 — 2026-10-09

Review fixes from a bug audit of the system daemons, the shell and the apps.

- audiod, netd, updated: subscriber sockets get a 500 ms write timeout, so one client that stops reading can no longer freeze the daemon while it holds the state lock.
- Shell: a replaced Browser child is reaped on a thread instead of left as a zombie; a window resized by the compositor keeps the scale-derived minimum size, not fixed pixels.
- Shell: the popup mask radius uses `MENU_RADIUS_PT`.
- Installer: a short `parted` line takes an empty disk model instead of the partition table name.
- Files: the Put Back origins file is written atomically.
- Browser: a malformed `/etc/passwd` line returns an error instead of panicking; a failed engine extraction or `.ready` write removes its staging folder.
- Docs: the `footer()` comment gives the footnote size (10 pt).

## 0.4.71 — 2026-10-09

Data detectors (`NSDataDetector`, `bifrost-ui::detect`) and tappable links in Notes.

- `detect::find` finds links, dates, street addresses and phone numbers in text, as byte ranges with overlap priority
  (link, date, address, phone). Apple parity: `address` is in; `transitInformation` is not (needs transit data).
- Notes: links in a note are underlined in the tint; tapping one opens it through the shell (`Ctx::open_url`) and the
  caret does not move. Contacts is unchanged (link detection is opt-in on `TextArea`).
- Tests: bifrost-ui 147 (detect 7), springboard lib 2379 passed, notes-desktop 55, SDK kits 3. Bench
  `ui_detect_scan_100kb` 8.07 ms (new corpus).

## 0.4.70 — 2026-10-09

Default applications, the way Launch Services does it (`docs/default-apps.md` has the macOS model and the sources).

- `bifrost-ui::launch`: `Registry`, `Handler`, `Rank` (`LSHandlerRank`). Default per type or URL scheme = the user's choice, else the most specific declaration, then rank, then install order; `Rank::None` is listed but never chosen. `apps_for_type` (all candidates), `open_with` (default first), `set_default_for_type/scheme`, and a one-line preference form. Lookups are cached (the 10k-lookup benchmark `bench_ui_launch_default_for_file_10k` went from 72 ms to 0.39 ms before the cache was kept; not recorded in the history, benchmarks are paused).
- `uttype::distance` (how far a type is from an ancestor), used for "most specific".
- Manifests: `handles = ["public.png:owner", "public.text"]` and `schemes = ["https"]`.
- Shell: `shell/defaultapps.rs` declares what the built-in and first-party desktop apps open and builds the registry; the choice is saved in the `default.apps` preference. `Shell::default_app_for_file` is the lookup for Files and the shell.
- Settings > General > Default Apps (desktop): Web Browser, Images, Audio, Video, Text, PDF Documents, Archives; the default is checked and listed first; choosing an app changes every type of the family it can open.
- Tests: 7 `launch`, 5 `defaultapps`, `distance`; every Settings page still reachable. Springboard lib 2378 passed, bifrost-ui 124, 0 failed.
- Not wired yet: Files and the shell opening files and links through the registry, an Open With menu.

## 0.4.69 — 2026-10-09

Round 13: UniformTypeIdentifiers (`UTType`) adopted. `bifrost-ui::uttype` had 25 types and no user; it is now the single table of file types and Files uses it.

1. `uttype` table grows to 58 types (every extension Files knew, plus gif, webp, pdf, wav, ogg, opus, mov, tar, 7z, rar, yaml, toml, source languages) with parents and MIME types.
2. `localizedDescription` per type ("PNG image", "Rust Source", "PDF document"): `uttype::description`.
3. `UTType(mimeType:)` and `tags[.filenameExtension]`: `type_for_mime`, `extensions`; plus `preferred_extension`.
4. Once-built FNV index for extension and identifier lookups (10k lookups 0.34-0.42 ms with 2.3x the types; the old scan was 0.19 ms with 25 types).
5. Files `kind_of` is conformance-based (`conforms(public.text)` and so on) instead of five extension lists. Only PNG and JPEG are Image (the viewer's decoders). Same coarse kinds as before for every extension tested.
6. The Files info sheet and the preview page's info rows show the type description in Kind ("Rust Source", not "Plain Text"); unknown extensions keep "Plain Text" for sniffed text and "Document".
7. Files Sort by Kind orders by that description, as Finder does.
8. The shell's wallpaper rotation picks Pictures through the same `kind_of`.
9. SDK parity guard `sdk/tests/kits.rs`: apps reach `controls::uttype`, `keys::KeyCombo` and `menu::Menu`.
10. Docs (`docs/bifrost-ui.md`, "Uniform type identifiers") and benchmark `ui_uttype_lookup_10k_names` in `tools/bench.py` history. Note: the history entry was taken while another agent ran local model evals on the machine, so the frame numbers (idle 28 ms) are inflated; compare with the 0.4.68 entry only for the uttype and menu lines.
- Tests: kind descriptions, mime and extension mapping, one-type-per-extension, description coverage, 2 SDK tests. Springboard lib 2373 (one timing test, `pause_freezes_and_resume_continues`, flaked once under the eval load and passes alone), bifrost-ui 117, 0 failed.

## 0.4.68 — 2026-10-09

Round 12: AppKit `NSMenu` / `NSMenuItem` key equivalents adopted in the shell (the `bifrost-ui` `keys`, `menu` kits were not used by it before).

1. Key equivalents column in the bar menus (⌘W, ⌘M, ⌘Q, ⌘Z, ⇧⌘Z, ⌘X, ⌘C, ⌘V, ⌘A, ⌘,), from `keys::KeyCombo`; the table is conflict-checked by a test.
2. Menus size to their content (at least 264 pt); the width is cached, so hover stays at 0.15 ms per move.
3. Help menu: no more duplicate shortcut lines; Spotlight ⌘Space, App Switcher ⌘Tab, Back ⎋.
4. Reduce Transparency draws bar menus opaque (`DESKTOP_MENU_SOLID`) on the GPU path.
5. Long window titles are cut to 300 pt in the Window menu.
6. Type-to-select in open menus (the `menu::Menu` model: letters jump, the same letter cycles).
7. Hover cost: the width cache above (measuring every title doubled the cost; benchmark in `docs/perf-history.jsonl`).
8. Cmd+, opens Settings (`Shortcut::Settings`); Ctrl+, stays the terminal's.
9. Docs: `docs/bifrost-ui.md` "Menus in the shell", Golden Gate note.
10. `tools/bench.py` history entry for this round (hover 0.31 ms per 2 moves, popup draw 1.27 ms, frames 15.1/15.8 ms).
- Tests: 4 new (key table, long titles, type-to-select, Cmd+,) and the chord test. Springboard lib: 2372 passed, 0 failed, 6 ignored.

## 0.4.67 — 2026-10-09

Round 11: menu structure (HIG "The menu bar": group items with separators; the Window menu lists every window).

- Separators in the App menu (About | Settings, Dock Settings | Show Desktop | Quit), the Edit menu (Undo, Redo | Cut ...) and the Window menu (before the window list). They are 9 pt high, never selectable, and the keyboard skips them. Row geometry now comes from one `menu_spans` function (draw, click, hover and popup height agree).
- Window menu lists native Linux windows (store apps with their own window, and Linux apps) after the built-in ones; the front one is checked, a row activates it.
- Hover no longer calls the menu code without `desktop-wayland` (the library builds without it again).
- Benchmarks: `bench_menu_popup` (hover 0.145 ms per move, key step under 1 µs, CPU popup draw 1.3 ms); `tools/bench.py` now runs the springboard suite with `desktop-wayland,testkit`, so the desktop frame benches are recorded too.
- Tests: separators and foreign window rows (2 new). Springboard lib: 2368 passed, 0 failed, 6 ignored. No more test-build warnings.

## 0.4.66 — 2026-10-09

Round 10: the menu bar works from the keyboard and under the pointer (HIG "The menu bar", Full Keyboard Access; checklist CTL-04).

- Open menu: Up/Down move the highlight over actionable rows (labels are skipped, it wraps), Left/Right switch between App, File, Edit, View, Window, Help, Return chooses, Esc closes. Other keys are swallowed while a menu is open.
- Pointer: the row under it is highlighted (systemBlue); moving over another bar title switches the open menu, as macOS does.
- Internals: the row actions moved out of the click handler into `run_menu_action`, shared by click and Return.
- Tools: `tools/test_midget.py` update tests count the status call the guard now makes (3 failures fixed). Removed the unused `OnceLock` warning in the test build.
- Tests: `desktop_menus_work_from_the_keyboard_and_highlight_under_the_pointer` (new). Springboard lib: 2366 passed, 0 failed, 5 ignored.

## 0.4.65 — 2026-10-09

Round 9: the menu bar follows the macOS structure (HIG, "The menu bar"), and the Terminal app has its own name.

- Menu bar: View and Window menus, in the macOS order (App, File, Edit, View, Window, Help). Before, the bar had no View or Window menu.
- View: Automatically Hide Dock (checked), Smaller Icons, Larger Icons. The same Dock settings stay in the Dock menu.
- Window: Minimize, Zoom, Bring All to Front (enabled only when a window is minimized), then every open window by name (alphabetical, the front one checked). Choosing a row brings that window forward, minimized or not. Native Linux windows are not listed yet.
- App menu: Quit <app> (Cmd+Q), disabled when no window is open.
- The Terminal app is named Term, not iOSTerm, in the Dock, Spotlight, window titles and notifications. The app id (is.olibuijr.iosterm) is unchanged, so installed copies keep working.
- Tests: `desktop_menu_bar_follows_the_macos_order_and_the_window_menu_focuses_a_window` (new). Springboard lib: 2365 passed, 0 failed, 5 ignored (with desktop-wayland and testkit).
- Known and not fixed here: `cargo check --all-targets` warns about two unused test helpers (`dock_centre`, `shell_sized`) and an unused `OnceLock` in `predict.rs`; they were already in 0.4.64.
- Not done: the menus cannot be driven from the keyboard yet (Full Keyboard Access), and window open/close animations are still missing.

## 0.4.64 — 2026-10-09

Round 8: your "Things to fix" note, plus two more AppKit kits.

- Performance HUD: it showed on no desktop screen (the desktop draw path never reached the overlay). Now drawn on the desktop and its box is part of the GPU damage regions. Checked on Midget.
- Dock: pressing an icon whose app has a window brings that window forward at once, not on release. Clicking the frontmost app's icon no longer minimizes it (macOS keeps it; minimize is the yellow light or Cmd+M).
- Dock drag: the neighbours slide aside with an ease while an icon is lifted (`bifrost-ui::dockdrag`, retargets without a jump); dropping forward now lands after the target, matching the preview (it used to land before it).
- Menu bar: status items sit closer (icon buttons 36 -> 30 pt, readout padding 12 -> 10 pt, the 8 pt reserve before the clock is 2 pt) and each glyph is centred on the bar like the text beside it (the plug, network and ethernet glyphs sat 1 to 1.5 pt low). Checked against Midget screenshots.
- Settings > Displays > Graphics: adapter name (from `pci.ids`), driver, clock, utilisation and video memory where the driver exposes them, and the connected displays with their modes. Reads sysfs, so it works for Intel (i915/xe), AMD and NVIDIA drivers; rows a driver does not expose are left out. Checked on Midget: Intel HD Graphics 530, i915, 367 of 1100 MHz, DP-2 2560 x 1440.
- Bifrost Browser: servoshell failed at launch with `libgstplay-1.0.so.0: cannot open shared object file`. `gst-plugins-base-libs` and `gst-plugins-bad-libs` are now runtime dependencies of the shell package (the update pulled 16 packages, about 71 MB net, with GStreamer's own dependencies). Checked on Midget: the browser starts and shows its new-tab page.
- `bifrost-ui::menu`: `NSMenu` / `NSPopUpButton` model (separators, disabled rows, checkmarks, arrow keys that skip what cannot be chosen, type-to-select with prefix and same-letter cycling, hit testing, the pop-up rule that the selected row covers the button). SDK `widgets::popup_button` and `widgets::menu` draw it (Popover material, accent highlight, check mark, shortcuts), with Show Borders honoured. No app uses them yet.
- Benchmarks: `ui_dock_drag_frame_x1000` 0.45 ms, `ui_menu_type_select_hit_x1000` 0.58 ms. Desktop frame 14.0 ms dirty, 13.4 ms idle (no change from 0.4.63).
- Tests: `bifrost-ui` 114, springboard lib 2364 (with `desktop-wayland`), SDK widgets 9; the new shell tests fail without their fixes (checked for the HUD).
- Not done: window open/close/minimize animations and their settings (windows are separate compositor toplevels, so a scale-and-fade needs the presenter to draw them), Remote main screen redesign, Reminders sidebar layout, replacing the shell's own dropdowns with the new widget.

## 0.4.63 — 2026-10-09

Round 7: ten Foundation/UIKit/AppKit-parity kits in `bifrost-ui` (reexported to apps as `bifrost_app::controls`), each a pure model with tests, five with benchmarks in `docs/perf-history.jsonl`.

- `diff`: `NSDiffableDataSourceSnapshot`-style list diff (remove / insert / move, longest-increasing-run so stable items never move). 10k items shuffled: 1.9 ms.
- `cache`: `NSCache`, cost-limited LRU with O(1) get/put and slot reuse. 100k mixed ops: 4.1 ms.
- `fuzzy`: command-palette matcher with match positions for highlighting. 5k items: 5.7 ms before the subsequence prefilter. (Spotlight keeps its own Icelandic-folding matcher.)
- `format`: `ByteCountFormatter`, `DateComponentsFormatter` (positional), `RelativeDateTimeFormatter`, grouped numbers.
- `gesture`: `Tap` (multi-tap), `LongPress`, `Pan` (slop, translation, release velocity) state machines.
- `focus`: Tab / Shift-Tab key-view loop in reading order and arrow-key nearest-view focus.
- `uttype`: `UTType` extension lookup, conformance tree, MIME fallback.
- `textfit`: head/middle/tail truncation and word wrap against a caller-supplied measure. 9k-char wrap: 0.43 ms.
- `observe`: `Observable<T>` (KVO / `@Published`), notifies only on a real change.
- `calendar`: exact Gregorian day arithmetic, weekday, `add_months` with clamping, Calendar.app month grid. 10k dates: 0.18 ms.
- Wired in: Settings > Storage and the App Store show sizes through `format::byte_count` (one rule, three significant digits). The store's "900 B" row now reads "900 bytes" instead of "1 KB". Deildu keeps its whole-MB download sizes.
- Tests: `bifrost-ui` 104 pass (was 65); 2344 springboard lib tests pass.
- Not done this round: the grey boxes along a window's bottom edge (still needs the GPU layer dump in `gpu/wl.rs`), cached large-title helper, the remaining 520 off-grid sizes and 84 colours.

## 0.4.62 — 2026-10-09

- Dock: an app whose process is still running with no window shows a grey dot under its icon (macOS shows the same for a quit app that is still running). A windowed app keeps the white dot.
- Apps: off-grid spacing snapped to the 4-pt scale on all desktop apps (296 literals of 24 pt or less, each moved at most 2 pt). Colour literals that match a named token in `render/src/ui.rs` use the token (34, values identical).
- Tooling: `tools/snap-spacing.py` and `tools/name-colours.py` make these changes value-checked; `tools/check-app-consistency.py` counts drop (spacing 839 -> 520, colours 113 -> 84), ratchet baseline lowered.
- Not done: 520 larger off-grid sizes (widths above 24 pt), 84 colours without a token, 26 hand-drawn large titles (LAY-07).
- Tests: 2343 shell tests pass (`--test-threads=1`). All desktop app test suites pass; `pc-app-smoke` passes for 16 apps.

## 0.4.61 — 2026-10-09

- Apps follow the system accessibility settings: the shell sends Increase Contrast, Reduce Transparency, Show Borders and Bold Text to every app (a new additive `ToApp::Appearance` message; apps built before it ignore it). Settings > Accessibility calls the old Button Shapes switch Show Borders, as macOS Golden Gate does. Controls drawn through the SDK outline themselves when it is on.
- Updating an app that is open now restarts it. Before, opening it only focused the old process (Reminders 1.6.0 kept running after 1.7.0 was installed and its window vanished).
- Menu bar: the status items sit next to the clock. They were placed from a fixed 188 pt reserve, which left about 90 pt of empty bar. The positions follow the measured clock and the CPU/RAM readouts drop out when the bar is too narrow, down to 640 px wide.
- The App Store shows the version the store has on every app row ("Productivity · v1.7.0 · 2.5 MB"), not only when an update is pending.
- Edit menu: Undo and Redo come first, as on macOS. Text fields keep a history (typing groups into one step, cut, paste and delete are their own steps); `Command::Undo` and `Command::Redo` are new, old apps ignore them.
- SDK: a standard window toolbar widget (unified bar, button plates, overflow menu), a source-list sidebar widget with the Golden Gate full-height shaded column, Show Borders outlines, and `bifrost-ui` models for layout, stacks, grids, split views, popovers, segmented control, stepper, slider, page control, progress, navigation stack, alert, responder chain, undo manager, key equivalents and animation curves.
- Performance: Bézier animation curves evaluate 14 times faster (Newton's method, checked against bisection on 150,000 points); a 2000-row sidebar steps in 5 µs. Benchmarks for the new code run in `tools/bench.py` and are recorded in `docs/perf-history.jsonl`.
- Known, not fixed: five grey rounded boxes along the bottom edge of some app windows (Notes, Music); the app's own frame is clean, so the shell's compositing draws them.

## 0.4.60 — 2026-10-09

- Dock badges: an app can show a short label on its dock icon (a red capsule, as `NSDockTile.badgeLabel` on macOS). SDK: `Ctx::set_badge`; it needs the `notifications` permission.
- The menu bar, menus, window titles and the dock label draw from a text cache: the desktop frame benchmark drops from 16.3 to 14.9 ms (host).
- Text caches clear themselves when the interface scale, Dynamic Type, Bold Text or Increase Contrast change, so cached labels are not left at the old size.
- Housekeeping: the tablet app packages are gone; the repository only holds the desktop apps (each now has its own source, no shared symlinks).
- Tests: the word predictor is per test thread, which removes another random failure in parallel runs.

## 0.4.59 — 2026-10-09

- Fixed: closing a menu left a ghost of its frosted glass on the desktop until something else redrew that area.
- Menus have a soft shadow like the dock. The shadows are drawn once per size and reused (the dock shadow costs 0.06 ms a frame instead of 8 ms).
- Windows that are not in front are slightly darker, so the active window stands out more, as in macOS Golden Gate.
- SDK: `widgets::material` draws a system material (`Sidebar`, `Titlebar`, `Popover`, `Hud`) over an app's own content, the counterpart of `NSVisualEffectView.Material`. It turns opaque with Reduce Transparency.
- The shell library now builds without the `desktop-wayland` feature (the SDK needed that).
- Tests: the shared clipboard is per test thread, which removes a random failure in parallel runs.

## 0.4.58 — 2026-10-09

- Liquid Glass clarity now works on the glass itself: Clear lets the sharp desktop show through the blur of the menu bar, menus, dock and Spotlight; Tinted is fully blurred.
- The dock has a soft shadow, so it lifts off the wallpaper.

## 0.4.57 — 2026-10-09

- Built-in app windows render on the GPU too, with a soft window shadow drawn by the GPU. The active window's shadow is deeper than the others, as in macOS Golden Gate. Clicks on the shadow reach whatever is below it.
- Liquid Glass: the glass now lifts the colour of what is behind it (vibrancy) and has a thin highlight along its edge. Settings › Displays › Liquid Glass sets it from clear to tinted, as in Golden Gate.
- Desktop frames only redraw the menu bar, menus and the dock on the CPU: about 1.4 ms per frame on Midget instead of 5 to 19 ms.
- Dock icons are scaled once and kept, not every frame.

## 0.4.56 — 2026-10-09

- GPU-first desktop: the desktop background, lock screen, menu bar, dock and menus render on the GPU into dma-buf buffers that go straight to the compositor (linux-dmabuf), with no CPU copy. The wallpaper is drawn by the GPU and its frosted-glass blur is computed once on the GPU (dual-Kawase), so the dock, menu bar, menus and Spotlight no longer blur on the CPU.
- Desktop frames redraw only the menu bar, open menus and the dock instead of the whole screen: about 15 % of the pixels, 0.5 ms to present on Midget instead of a full-screen copy.
- Falls back to the previous shared-memory path when the GPU cannot be used (`BIFROST_GPU=0` forces it).
- Traffic-light hover glyphs are bolder.

## 0.4.55 — 2026-10-09

- Dock: pointing at an app icon shows its name in a floating label above it, as in macOS.
- Pointer hover and context positions from the control socket and from Wayland pointer events go through the same orientation mapping as taps.

## 0.4.54 — 2026-10-09

- Faster frosted glass: the Spotlight card, dock, menu bar menus and lock backdrop blur a reduced-size copy and scale it back (a shared `blur_material`), and the glass panel skips the corner maths in its interior. Spotlight's card went from 12.7 to 4.7 ms per frame on the host benchmark.
- The dock no longer copies every icon each frame.
- Cmd+Q quits the app in the front window and Cmd+H hides it, as on macOS; both are listed in Help.

## 0.4.53 — 2026-10-09

- Fixes 0.4.52: the traffic-light glyphs now redraw when the pointer reaches them (the window cache ignored hover), and Settings sidebar names such as Network are no longer cut to "Net...".

## 0.4.52 — 2026-10-09

- Desktop windows: pointing at the traffic lights shows their close, minimise and zoom glyphs (Golden Gate behaviour); the lights are plain dots otherwise.
- Double-clicking a window's title bar zooms it, as in macOS.
- Settings sidebar: the whole column has its own shade, and rows no longer show status values ("Not Connected", "Off") that truncated names such as Network.

## 0.4.51 — 2026-10-08

- Fixes the 0.4.50 boot loop on Midget: 0.4.50 was packaged from a build without the desktop-wayland feature, so the shell opened the DRM card that the compositor already held and exited. The rebuilt shell starts as a Wayland client, as the session intends.

## 0.4.50 — 2026-10-08

- Desktop windows: the focused window's traffic lights carry a thin glass rim; background windows drop it, as in macOS Golden Gate.
- Window title-bar colours, inactive traffic lights, window and menu corner radii are named shared tokens (render/ui.rs).

## 0.4.49 — 2026-10-08

- A desktop window closes when its app exits. Before, the last frame of a crashed or killed app stayed on screen until it was closed by hand.

## 0.4.48 — 2026-10-08

- Installed App Store apps open from `open-app` and from Show in Files. They were reported as not installed before, because only built-in apps were looked up.

## 0.4.47 — 2026-10-08

- Show in Files: `bifrost-apps --open <folder>` opens the Files app at that folder. The shell takes an `open-folder <app id> <folder>` control request, writes it to the app's data folder, and opens the app; the folder may contain spaces.
- Files 1.10.0 (App Store) reads the request and opens there.

## 0.4.46 — 2026-10-08

- Cmd or Ctrl+Space on the desktop opens a macOS-style Spotlight: a 680 pt panel near the top, a field with a magnifier and no microphone, dense rows with small uppercase section titles, and the top hit in systemBlue. With nothing typed it shows no suggestions, and the on-screen keyboard stays down because the desktop types on its own keyboard. The tablet Spotlight is unchanged.
- Shared UI kit: named type scale, LINK (tint) and DESTRUCTIVE (red) colour roles, and footer() and large_title() helpers.

## 0.4.45 — 2026-10-08

- Escape reaches the app in front. Apps use it to close what is open: a note, an editor or a search field.
- Cmd or Ctrl+Backspace reaches the app in front as a chord, so apps can delete the selected item.

## 0.4.44 — 2026-10-08

- Desktop keyboard: a Ctrl or Cmd chord over a letter, digit or punctuation key that the shell does not use goes to the app in front. Ctrl+Z and Ctrl+Shift+Z reach external apps as undo and redo.
- The SDK sends chords as key codes with a flag bit, so existing apps keep working.

## 0.4.43 — 2026-10-08

- Lock screen on the desktop: the hint reads "Press Return to unlock" instead of "Press Enter".
- Desktop without a battery: Settings > Battery, and the Battery page, show "None" instead of a made-up 100%. The Today battery tile is hidden.

## 0.4.42 — 2026-10-08

- Software Update: "Roll Back to Previous System" works on the desktop. It asks first, then the rollback runs as its own system unit, so it finishes even though the shell stops during it.
- Keyboard: Cmd+W closes and Cmd+M minimizes the front desktop window. Ctrl+W and Ctrl+M still reach terminals as before. Both are listed in the Help menu.
- Desktop without a battery no longer records a made-up 100% battery sample in the battery log. Power decisions keep the mains fallback.

## 0.4.41 — 2026-10-08

- Status bar: the Wi-Fi glyph shows only for a joined Wi-Fi network. A wired link shows a LAN glyph; no adapter and no cable shows nothing.
- Status bar: a desktop without a battery no longer shows a made-up 100% charging battery.
- Notifications: Wi-Fi "Joined/Disconnected" notices follow real Wi-Fi state only; a cable coming up no longer posts "Joined No adapter".
- Wi-Fi widget: says "No Wi-Fi adapter" on machines without one and "Ethernet connected" on a wired link.
- Hardware detection: the shell now carries `wired` and `wifi_adapter` (from NetworkManager link kinds on the desktop, sysfs elsewhere).
- Cleanup: the build is warning-free (dead code, unused imports, clashing libc declarations, unreachable match arms).

## 0.4.40 — 2026-10-08

- Brand: new BifrOSt mark, an Icelandic-viking badge (basalt hexagon, four stone rainbow bands on piers, a Berkanan B-rune and an eight-armed runic star). Source SVGs in `packaging/brand/`; `tools/bifrost_brand.py` renders the installer logo, the Plymouth mark and the `splash.efi` pixels from it.

## 0.4.39 — 2026-10-08

- Boot: the desktop starts in the display's firmware mode and never changes it (every mode change makes the monitor retrain its link, a visible blink). The fastest refresh is opt-in with `refresh=max` in `/etc/bifrost/display.conf`; without it the learned `video=` argument is removed from `cmdline.txt`.
- Boot: Plymouth draws on the firmware framebuffer at once (`plymouth.use-simpledrm`) instead of waiting about 2 s for i915; `splash.efi` and the Plymouth theme share one resolution-independent layout.
- Boot: a stop requested by systemd (reboot, shutdown, restart) no longer counts toward the shell's crash-loop latch.

## 0.4.38 — 2026-10-08

- Boot: one display mode from the kernel to the desktop. labwc reuses the current output mode (`reuseOutputMode`), the session learns the monitor's fastest native mode and keeps it as `video=` in the ESP `cmdline.txt` (`/usr/lib/bifrost/video-arg`), and `session-env` skips the mode switch when it is already the fastest. No second modeset flash at start-up.

## 0.4.37 — 2026-10-08

- Boot: the splash handoff exits 0 when the splash was already released by the session (no failed unit on every boot).
- Boot: timing marks (uptime seconds) for session start, splash release, compositor ready, shell launch, shell ready and handoff go to `/var/log/bifrost-boot-timing.log`.
- Control socket: `version` and `ready` commands; `state` reports the shell version and ready flag.

## 0.4.36 — 2026-10-08

- Session: the boot splash is quit before labwc starts. Plymouth held DRM master, so labwc's output test failed ("Device or resource busy") and the shell start counted toward the crash guard, which rolled back 0.4.35.

## 0.4.35 — 2026-10-08

- Session: the crash-loop counter resets when the installed shell version changes, so a fixed build is not blocked by earlier failed starts.
- Installer: a fresh install starts with no crash-loop history; the target is released before partitioning; the installed kernel line disables i915 FBC and PSR (stripes on Skylake).
- Tools: the wipe refreshes the live installer from the lab sync and refuses to erase unless its hash matches the pushed build.

## 0.4.34 — 2026-10-08

- Installer: the shell, its units, session files and Plymouth theme are installed from the `springboard-shell` package, not copied from the live medium. Only the boot loader is still read from the medium.
- Package: ships the Plymouth theme and the splash handoff unit, so System Update delivers boot-time visuals too.

## 0.4.33 — 2026-10-08

- Boot: the Plymouth splash fades in after the first frame and fades out once the desktop has drawn; it no longer quits before the desktop exists.
- Boot: the shell writes `shell-ready` after its first committed frame; `bifrost-splash-handoff` releases the splash on that signal (60 s timeout).
- PC: getty on tty1 and the stock Plymouth quit units are masked, so no login prompt or text console is drawn.
- Tools: `tools/midget-wipe.py` wipes and reinstalls Midget behind a confirm token, with a dry-run and preflight checks.

## 0.4.32 — 2026-10-08

- Desktop: idle metrics redraw only when the shown values change; App Store download redraws throttled to 4 Hz.
- Desktop: App Store names and icons no longer leak on every registry rebuild; window scratch buffer reused.
- Desktop: NetworkManager signal filter narrowed to device, active-connection and access-point properties (1 s debounce).
- Desktop: Alt/Ctrl/Meta+Tab cycles desktop windows; Power menu gains Lock, Restart and Shut Down.
- Desktop: Network menu lists saved Wi-Fi networks; CPU and Memory menus show per-app usage.
- Desktop: General drops tablet-only rows; Displays drops placeholder rows; Desktop & Dock page shows dock position, size and auto-hide.
- Desktop: menu bar clock opens Calendar; Help lists keyboard shortcuts.
- App Store: Roll Back to the kept previous version; Cancel a download by tapping its progress ring (the .part file resumes later).
- Software Update: "Back Up Before Updates" switch (on by default; a failed backup blocks the install); disabled rollback row points to the terminal command.
- Desktop windows reopen after a shell restart.
- Download resume no longer re-hashes the partial file before fetching; the full checksum is still verified on completion.
- Wayland present: damage copy rewritten row by row, checked against the scalar reference in a unit test.

## 0.4.31 — 2026-10-08

- Require HTTPS throughout update, App Store and diagnostics redirects.
- Integrate 14 independently packaged PC app ports with the shared UI foundations.
- Support explicit app notification permission with rate-limited delivery.
- Fix desktop-feature isolation in non-Wayland builds and preserve physical-keyboard field focus.
- Correct source-package checkout discovery and create atomic-write temporaries exclusively.
- Keep the docked tablet keyboard hidden for desktop app windows while retaining keyboard input.

## 0.4.30 — 2026-10-08

- Extract shared raster graphics, SF UI typography, appearance, clipboard and text cache into bifrost-render.
- Consolidate JSON parsing, integrity and atomic storage in bifrost-core, and move scroll physics/input to bifrost-ui.
- Share Notes through a library; separate service environment from desktop/tablet presentation.
- Consolidate Arch package payloads, hooks and runtime dependencies; use bifrost as the development CLI and skill namespace.

## 0.4.29 — 2026-10-08

- Desktop: the menu-bar backdrop is blurred once per wallpaper and size instead of on every frame, cutting shell draw time.

## 0.4.28 — 2026-10-08

- Replace desktop network/power labels with icons; show live CPU and memory usage with detail menus.
- Track compositor maximize state and expose window restore from the File menu.
- Add persistent dock position, size, autohide, drag ordering and pin/unpin context controls.
- Add SDK context-menu requests/actions; route desktop web links to the sandboxed BifrOSt Browser.
- Install the default browser through App Store during fresh setup and expose shell apps in Applications.

## 0.4.27 — 2026-10-08

- Load Btrfs before touching the installation disk and explicitly mount its subvolumes.
- Include Btrfs in the installed initramfs for reliable first boot.
- Stage live asset directories without nesting when a file already created the destination.
- Register the system package outside the chroot temporary mounts and include native-launcher Python bindings.
- Keep debug boot diagnostics in the journal behind the graphical splash.

## 0.4.26 — 2026-10-08

- Snapshot complete Btrfs system generations with matching kernel/initramfs and persistent user data.
- Give pending updates one EFI boot attempt, then select previous kernel and root.
- Accept full-system updates only after a reboot and responsive desktop probation.

## 0.4.25 — 2026-10-08

- Add independent static Rust shell-package rollback and responsive-session probation.
- Route managed updates through recovery; block unrelated base upgrades without whole-root recovery.
- Default new installations to Btrfs with separate system, home, app-state, recovery, cache and log subvolumes.

## 0.4.24 — 2026-10-08

- Preserve SDK compositor focus metadata for accurate resource priority without duplicating dock entries.

## 0.4.23 — 2026-10-08

- Use compositor focus for both SDK and native app resource priorities.

## 0.4.22 — 2026-10-08

- Own SDK and native package process trees in per-app cgroups before launch.
- Enforce RAM-scaled memory pressure and hard limits, grouped OOM handling and process limits.
- Adjust SDK resource priority with lifecycle focus and expose per-app resource diagnostics.
- Close the complete SDK app process tree, including descendant processes.

## 0.4.21 — 2026-10-08

- Preserve native GPU acceleration with read-only hardware discovery metadata in the sandbox.

## 0.4.20 — 2026-10-08

- Expose read-only native session metadata for Browser compatibility inside the central sandbox.

## 0.4.19 — 2026-10-08

- Launch desktop SDK and native packages through central identity and fail-closed sandbox policy, without running app entry points as root.
- Isolate filesystems and processes, disable privilege escalation, enforce declared network/storage/audio access, and reject unsupported permissions.
- Add an authenticated, fixed Remote sharing broker operation; Remote UI no longer needs system privileges.

## 0.4.18 — 2026-10-08

- Route desktop menu input before the mobile notification-drawer gesture handler.

## 0.4.17 — 2026-10-08

- Add independent bifrost-ui control semantics, focus management, commands and Unicode text editing.
- Expose SDK control metadata to the shell and route enabled Edit commands directly to the focused app.
- Preserve compatibility with existing SDK applications through additive protocol messages.

## 0.4.16 — 2026-10-08

- Exclude wireless mouse and keyboard batteries from system power status; show mains power on desktops without a system battery.
- Enable the power-profile service in fresh installations and source-built package upgrades.

## 0.4.15 — 2026-10-08

- Invalidate the desktop only when foreign Wayland window metadata changes; stop compositor polling from repainting quiet windows.
- Add desktop app/File/Edit/Help, network and power menus on dedicated Wayland layers, preserving visibility above maximized apps.
- Show actual Ethernet and Wi-Fi state, adapter availability, and power-profile selection; install and enable the power-profile service.

## 0.4.14 — 2026-10-08

- Refresh app metadata after installs and updates without leaking registry snapshots; retain stable SDK slots and running-process ownership.
- Repaint only the performance overlay band during diagnostics updates, preserving quiet app windows.
- Patch SDK damage directly into retained window pixels and discard redundant body caches after updates.
- Monitor NetworkManager and power-profile D-Bus signals; wake the UI through a notification socket and retain a one-minute fallback refresh.
- Stream app downloads through bounded buffers, retain partial downloads, retry failures, and resume with validated byte ranges and final SHA-256 verification.

## 0.4.13 — 2026-10-08

- Allocate a second Wayland window buffer only when a dirty frame arrives while the compositor still owns the first.
- Recreate display buffers when window dimensions change, including equal-area reshapes with a different stride.

## 0.4.12 — 2026-10-08

- Share immutable window pixels between the shell cache and Wayland backend instead of retaining duplicate full-window copies.

## 0.4.11 — 2026-10-08

- Keep requested app dimensions separate from mapped buffer dimensions during resizing; prevent copying past the previous frame while replacement buffers are in flight.

## 0.4.10 — 2026-10-08

- Retain desktop app window content and propagate partial damage to Wayland buffers; quiet SDK windows stop repainting.
- Preserve independent app body buffers and copy only their changed regions.
- Refresh background built-in windows at most once per second and recover queued updates when compositor buffers are released.

## 0.4.9 — 2026-10-07

- Replace desktop Wi-Fi sidebar with Network, separating Ethernet details and Wi-Fi adapter status and controls through NetworkManager.
- Include runtime support for the independent Remote app.

## 0.4.8 — 2026-10-07

- Hand the existing desktop Browser dock position to the independently installed BifrOSt Browser and avoid duplicate icons.
- Include bubblewrap for native browser process isolation.

## 0.4.7 — 2026-10-07

- Support native Wayland App Store applications with their own GPU-rendered windows. Launch and focus them without creating an empty SDK window; use their packaged dock icon and running indicator.

## 0.4.6 — 2026-10-07

- Show running Linux applications, including Firefox, in the desktop dock using the compositor’s window list and installed app icons. Clicking restores and focuses a window; multiple windows share one icon, which disappears when the last window closes.

## 0.4.5 — 2026-10-07

- Keep tablet home icons out of all desktop overlay and transition backdrops, including the top clock drawer.
- Use the modern Adwaita cursor theme and explicitly restore arrow/resize cursors on Wayland shell surfaces instead of retaining a previous app’s hand pointer.

## 0.4.4 — 2026-10-07

- Open App Store packages in desktop windows and keep a single Files dock icon after installing the standalone app.

## 0.4.3 — 2026-10-07

- Add desktop App Store sidebar navigation and size its content to the actual window.
- Preserve desktop window geometry when initializing applications instead of forcing portrait dimensions.

## 0.4.2 — 2026-10-07

- Hand desktop Files over to its independent App Store package.
- Separate the PC app catalogue from Android packages and open documents through local desktop file associations.

## 0.4.1 — 2026-10-07

- Reveal the desktop throughout the unlock animation instead of briefly showing tablet home-screen icons.

## 0.4.0 — 2026-10-07

- Port PC desktop and installer display/input to native Wayland, with a shared session startup path.
- Give built-in desktop apps individual Wayland windows; run Linux apps through labwc and XWayland as the local account.
- Add the Applications dock button and Super+Space launcher for installed desktop entries.
- Lock through ext-session-lock, capture the entire compositor, preserve keyboard layouts and select the fastest native display mode.
- Register the installed shell with pacman so first Software Update works; ship session dependencies with PC updates and installer images.
- Align desktop Settings naming and enlarge its default window.

## 0.3.18 — 2026-10-07
- Desktop Settings has a dedicated Displays page with the active panel resolution and refresh rate, interface scaling, Night Shift and text controls. Monitors without a software backlight show monitor-control guidance instead of an ineffective brightness slider; tablet controls remain available on the tablet.
- Desktop System Update uses the BifrOSt product name. The first PC update adopts the installer-provided shell into the system package manager.

## 0.3.17 — 2026-10-06
- One binary now also runs as the OS shell of an x86_64 Arch Linux desktop (runtime Tablet/Desktop profile; the tablet is unchanged): evdev discovery for touchscreen, mouse and keyboard, a pointer cursor, key repeat and hardware-keyboard shortcuts, ALSA/VA-API media, logind sleep, pacman updates, adaptive layout for any panel size and scale, Mesa GPU path.
- Host emulators for development: a virtual panel (SPRINGBOARD_VIRTUAL) with vsync pacing, finger down/move/up, hold and double-tap control commands, and the Arch VM harness.
- `state` printed `today_off` without a trailing newline, which hid the orientation keys in remote inspection.

## 0.3.16 — 2026-10-06
- Fixes from a code review: tapping a home-screen folder no longer crashes; music and video keep their position after a pause and no longer hang at the end of the queue; Notes, Calendar and Voice Memos use local time; Calendar alerts now fire; Redo works; Backspace and the arrows move over whole emoji; Photos honour camera orientation; the clock digits no longer jitter in Bold Text; Auckland and Santiago use the right daylight-saving rules.
- Safer: crafted zip, MP4 and share files can no longer exhaust memory or storage; atomic saves survive a power cut; Bluetooth, binder and pty no longer leak descriptors; LAN updates need a signed manifest.
- Faster: Spotlight no longer reads the disk or re-folds text on every keystroke; Settings rebuilds only the visible page; Photos scrolling, thumbnails and the thumbnail cache do less work; Weather and the time picker stop redrawing from scratch.
- Flat corners everywhere: no radius above 6 px.

## 0.3.15 — 2026-10-06
- Deildu Sources: the duration chips and provider switch sit under the banner instead of on top of it.

## 0.3.14 — 2026-10-06
- Deildu Sources: every card on a page gets checked for a playable stream (not just the first 24), and the large banner shows its picture at full size.

## 0.3.13 — 2026-10-06
- Deildu gains a Sources tab with streaming providers (HeavyFetish, XAdultFlix, XNXX): 16:10 cards, web thumbnails, duration filter chips, provider switch, page cache with next-page prefetch, quality choice and preview clips on the detail screen, direct MP4 download-to-play, and a LAN server URL per playback so the TV (mpv/AirPlay) can show the same stream.

## 0.3.12 — 2026-10-06
- One scrolling feel everywhere, matching iPadOS: lists follow your finger without a jump when you start dragging, flings take the speed of your swipe, slow down like iOS and bounce back at the ends with the speed they hit them. Settings, the App Store, Photos, Files, Notification Center, Today and all apps share it.

## 0.3.11 — 2026-10-06
- The App Store checks for new versions every time you open it and when you tap Updates, so an update shows up right away.
- Update badges are checked every 15 minutes instead of every 6 hours.

## 0.3.10 — 2026-10-06
- Settings with the sidebar draws its pages straight to the screen: smoother scrolling and page changes, and 6 MB less memory.

## 0.3.9 — 2026-10-06
- Opening an app no longer leaves another app's keyboard on screen.

## 0.3.8 — 2026-10-06
- Apps scroll and animate in step with the display: each app draws one frame per screen refresh instead of racing ahead, so motion is even and apps use about half the work.
- The system reacts to a new app frame immediately instead of checking every 8 ms.

## 0.3.7 — 2026-10-06
- Smoother animations and scrolling: the next frame starts while the previous one is still being shown, so heavy screens no longer drop to 30 frames a second.
- Touches keep the time the screen sensed them, so flings and drags follow your finger more closely.
- Apps use less memory: they share the system font with the rest of the system instead of each loading their own copy (about 9 MB per open app).
- Night Shift and rotation give their extra screen buffer back when they're no longer in use.

## 0.3.6 — 2026-10-05
- Settings row icons are the real iPadOS ones, and Passcode has its own Touch ID icon.
- General follows the iPadOS layout, with new AirDrop, AirPlay & Handoff, Picture in Picture, Background App Refresh, Gestures, Fonts, Dictionary and VPN & Device Management pages.
- AirDrop: Receiving Off declines nearby files, and Contacts Only accepts files only from devices already seen nearby.
- Gestures: with Allow Finger to Swipe From Corner on, a swipe from the bottom-left corner takes a screenshot.
- Settings with the sidebar redraws less, so scrolling and page changes are a little smoother.
- Software Update no longer offers a build with the same version number, so installing it can't roll back a newer build.
- Hides background services from the home screen; the lock screen clock is thinner, and Control Centre is lighter.

## 0.3.5 — 2026-10-05
- Clock (alarms, timers, stopwatch, world clock) is now its own app, installed and updated from the App Store; alarms and timers still ring with the app closed, and your data moves over on first launch.
- Podcasts and Voice Memos are now installed apps (updated from the App Store) instead of being built into the shell. Podcasts episodes keep playing when you leave the app; your subscriptions, downloads and recordings are moved over on first launch.
- The audio service closes the microphone and saves the take if the app that was recording stops unexpectedly.
- Two more background services, a network service and an update agent, so Wi-Fi and software updates can keep running independently of the shell. Nothing changes on screen yet.

## 0.3.4 — 2026-10-05
- The Calendar icon's date digits are drawn with smooth curves instead of blocky squared strokes.

## 0.3.3 — 2026-10-05
- Music is now its own app, installed and updated from the App Store. It plays through the audio service, so playback keeps going when the app is closed, and the lock screen, Control Center and volume keys control it directly. Your library, queue, playlists and radio favourites are moved over on first launch.

## 0.3.2 — 2026-10-05
- Lock screen and Control Center Now Playing, the transport buttons and the volume keys now use the audio service when it is playing; the built-in Music player is the fallback until the apps move out.
- Apps get an audio client in the SDK.

## 0.3.1 — 2026-10-05
- Developer: the audio service client API (`bifrost_app::audio`) for apps to read playback state and control it. No change on screen yet.

## 0.3.0 — 2026-10-05
- New background service supervisor (bifrostd) and an audio service: the groundwork for music, podcasts and voice memos to keep playing independently of the shell. Nothing changes on screen yet.

## 0.2.2 — 2026-10-05
- Developer: added a benchmark and a written plan for zero-copy GPU output (still off by default; no change for users).

## 0.2.1 — 2026-10-05
- Sleep: when the kernel keeps a wakeup event pending, the tablet now still tries to suspend instead of giving up, and logs the blocking wakeup sources.

## 0.2.0 — 2026-10-05
- Ten stock apps (Games, Freeform, Shortcuts, Weather, News, Reminders, Contacts, Calendar, Notes, Books) are now their own apps, installed and updated from the App Store; their data moves over on first launch.
- Apps can use the system keyboard.
- Color emoji everywhere, from the Keyboard system package.
- Settings pages slide in smoothly.
- The App Store shows What's New and version history for every app and system component.
