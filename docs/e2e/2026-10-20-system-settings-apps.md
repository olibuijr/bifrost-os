# E2E on Midget: system UI, Settings and apps not tested before, 2026-10-20

Midget 0.10.8 (`integration` at `4592c90ae`), 2560x1440, Dark appearance with the Purple accent from the middle of
the run. Computer use through `tools/midget.py shot/click/type/key/drag`, about 35 minutes (01:13 to 01:48).
Reference images: `goldengate_refimages/27-Golden-Gate-*.png` (Apps, Spotlight-Search-v2, Calculator,
Settings-General, Settings-Appearance, TextEdit, Notes, About-This-Mac-Menu, Notification-Center).

Areas covered for the first time: the 0.10.8 asset update, Launchpad, Mission Control, Spotlight, Calculator,
Settings (search, Appearance, Sounds, About), Control Centre, the terminal bell and alert sounds, TextEdit, Notes,
Reminders, App Store updates, Music Radio, the system menu, Force Quit, Notification Center, window buttons and
screenshots. Rounds 82 and 83 covered Terminal, Quick Look, Activity Monitor, Calendar, the lock screen, Files and
Browser.

Evidence images are next to this file in `docs/e2e/2026-10-20-system-settings-apps/` (`sNN-*.png`, with
`-small.png` at 1280 wide). They are on disk only (`*.png` is gitignored).

## What passed

- 0.10.8 update: `phase=accepted`; `pacman -Qo` names springboard-shell as the owner of
  `/usr/share/springboard/system-assets`; the scriptlet removed 79 of the 87 unowned legacy files and logged each;
  `/var/lib/springboard/icons` holds exactly the 14 own icons (was 59, 34 of them vendor-named); a second prune run
  is a no-op.
- Unlock with `midget.py unlock`; the last session's windows come back after the reboot.
- Spotlight from the empty desktop: "calc" gives Calculator as the top hit and Enter opens it.
- Calculator: typed `12*3=` gives 36. Cmd+Q quits it and its Dock icon goes away.
- Settings: search ("dark" finds Dark and Appearance and opens the pane); Dark applies to the whole desktop with a
  fade; the Purple accent applies to the selection plates and sliders.
- Control Centre opens from the menu bar; its Sound slider sets the volume (0 to 0.716 in `state`).
- TextEdit: typing, and Save to `~/Documents/e2e-note.rtf` (valid RTF with the text).
- Notes: New Note, title and lines; the list shows the note after a refocus. Notification Center's Recent Notes
  widget shows it too.
- Reminders: add with a date, the counts and the Dock badge update, completing clears the badge.
- App Store: Updates lists Browser, Clock, Music and News with their notes; Update All installs all four
  (`installed.json` has the new versions).
- Notification Center opens from the clock with live widget data.
- Windows: drag by the title bar, minimise with the yellow button, green restores from full screen.

## Findings

Severity: **P1** breaks a basic task or leaves the screen broken, **P2** wrong behaviour or a clear visual difference
from the reference, **P3** polish or parity gap. Each finding is one fix brief.

### Keyboard and system UI

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| K1 | P1 | A Cmd chord the front app takes sends every window to the Dock. Settings search field, Cmd+A: `state` goes from `app:settings` to `home` and all windows minimise. Cmd+Space with the Browser in front does the same (S1). Likely cause: `hwkeys.rs` turns a lone Cmd press-and-release into `Shortcut::Home` (`meta_alone`, a tablet behaviour), and a chord the app owns never reaches `HwKeys::key`, so the Cmd release still counts as alone | On the desktop a lone Cmd does nothing, and a chord never triggers Home | s29, s11, s12 |
| S1 | P1 | Cmd+Space with an app in front does not open Spotlight (K1 sends everything home instead; `open_quicksearch_hw` also returns early unless the screen is Home or the desk is active). A 1 px line is left where the field would be | Spotlight opens over any app | s11, s13 |
| K2 | P2 | After K1, typing went to a cream monospace overlay "Applications: sound" in the middle of the desktop (not in the window list); Esc closes it | Typing on the desktop selects a desktop item; no overlay | s27, s28 |
| MC1 | P2 | Ctrl+Up and F3 do not open Mission Control; Ctrl+Up reached Files, which went to the enclosing folder | Mission Control (ref `Mission-Control`) | s03, s05 |
| SM1 | P2 | System menu: no Force Quit... (and Option+Cmd+Esc does nothing), Recent Items inlined as ten rows instead of a submenu, no Log Out user... (Shift+Cmd+Q), no item icons, opaque grey menu | ref `About-This-Mac-Menu` | s65, s67 |
| AB1 | P2 | About This Computer opens the Settings About pane. Model shows the type code `10FLS2WK00`; the processor is truncated | A small separate About window (picture, name, chip, memory, serial, version, More Info...), readable model name | s66 |
| NC2 | P2 | Notification Center covers the menu bar from y = 0 (status items and clock hidden); text-only grey cards; "No Notifications" as a bold header on the wallpaper; "Edit Widgets" is a plain link | Panel below the menu bar, glass widgets with icons, "Notification Center" header with clear, capsule Edit Widgets (ref `Notification-Center`) | s68 |
| CC2 | P3 | Control Centre has only Wi-Fi, Focus and Sound | Also Bluetooth, Nearby Share, Display, Now Playing, Screen Mirroring | s35 |
| MB1 | P3 | Each running app adds a one-letter status item to the menu bar (N for Notes, R for Reminders) | No per-app letters | s48, s51 |

### Overlays, Dock and windows

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| OV1 | P1 | Closed overlays leave their blurred backdrop on screen. Launchpad: a full-screen washed wallpaper stays until a second click. Control Centre and Notification Center: orange blurred ghost tiles stay at the top right through later screens | Closing an overlay removes its backdrop and blur layers | s04, s06, s37, s38, s69-s75 |
| L1 | P1 | Launchpad opens behind the app windows | Launchpad above every window | s02 |
| L3 | P1 | Launchpad does not close with Esc (Esc goes to the app behind) or with its Dock icon | Esc, the Dock icon and a click outside close it | s07, s08 |
| D3 | P1 | When the Dock reloads its icons (after the Dark switch, after Update All) slots draw the wrong images: two Settings, two Browser, no Camera or App Store; clicking the second "Settings" opens Terminal. A Dock click redraws it correctly | Each slot keeps its own icon | s24-dock-crop, s26, s59 |
| W2 | P1 | Green (full screen) on Music makes the window 2560x1440 but leaves it at its old origin (848,407), so it runs off the right and bottom; the menu bar hides and the Dock stays on top | A full-screen space at 0,0, menu bar and Dock auto-hidden | s74, s75 |
| D1 | P2 | Windows sent to the Dock show as extra app icons on the right, not window thumbnails | One icon per app; minimised windows as thumbnails | s12, s73 |
| D2 | P2 | After an app leaves the Dock the plate keeps its old width (an empty slab at an end); after Dark a light, later brown, slab sticks out at both ends | Plate fits the icons | s19, s24-dock-crop, s26 |
| L2 | P2 | Launchpad is a flat light-grey full-screen page | Floating glass Applications panel with category chips and a 7-column grid (ref `Apps`) | s02 |
| W3 | P2 | Hovering the green button shows no window-management menu | Fill, Centre, halves, quarters, Full Screen | s70 |
| S3 | P2 | Spotlight is opaque white, its field is 0.25 of the screen width (ref 0.37), result icons about 1% of the screen width (ref 2.6%), one row without an icon, section headers instead of the filter chips and two-line rows | ref `Spotlight-Search-v2` | s14 |

### Audio

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| MU1 | P1 | Music Radio plays nothing (click, double-click, Controls > Play): the PCM stays closed, no now-playing bar, no error; the page later says "Radio (Offline)". Root cause: the media player service `is.olibuijr.audio` (`springboard/audiod`, socket `<root>/run/is.olibuijr.audio.sock`, started by bifrostd from `<root>/apps/<id>/current`, `kind = "service"`) is not installed on Midget. The store has `is.olibuijr.audio`, `is.olibuijr.net` and `is.olibuijr.update` 0.1.0 from 2026-10-05; neither the installer nor the OTA installs service packages. Music, Podcasts and Voice Memos depend on it | The system services ship with the system and update with it | s61-s64, s73 |
| AU1 | P1 | No alert sound reaches the output: the terminal bell (`tput bel`) and choosing an alert sound never open the PCM, even at volume 0.72; no visual bell either | The bell and alerts play (or flash when muted) | s39, s40 |
| SN1 | P2 | Choosing an alert sound in Sounds only saves it (`Pick::AlertSound`); no preview | Plays the sound on selection | s33, s34, s37 |
| AU2 | P2 | `bifrost-audio status` fails: cannot reach `/var/lib/springboard/run/is.olibuijr.audio.sock` (same cause as MU1) | Status of the audio service | |
| MU3 | P3 | Music Controls menu: no Stop or Go to Current Song; the crossfade times are inline rows | Submenu | s63 |

### Screenshots

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| SC1 | P1 | Shift+Cmd+5 > Screen saves only the shell's layers (menu bar and Dock on black): no wallpaper, no windows | The composed screen | s81-captured-file |
| SC2 | P1 | The file goes to `/Pictures/Screenshot-20261020-014807.png` at the filesystem root (no HOME in the capture path) | `~/Desktop` (the reference default) or `~/Pictures` | |
| SC3 | P2 | White rectangles stay at the bottom left for 9 s or more after the capture | A thumbnail at the bottom right that slides away | s79, s80 |
| SC4 | P2 | Shift+Cmd+3 and Shift+Cmd+4 are not bound (only Shift+Cmd+5 is); the capture toolbar sits on top of the Dock with text-only buttons | Both shortcuts; icon toolbar above the Dock | s77 |

### Settings

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| ST1 | P2 | Sidebar order and grouping differ from the reference (no Wi-Fi row at the top, "Sounds" for Sound, BifrOSt Account is a text row with no icon); General has Nearby Share and a Game Mode group the reference does not | ref `Settings-General` | s20 |
| ST3 | P2 | Appearance is two checklists (mode, accent rows) | Mode thumbnails, Liquid Glass preview, accent swatches in one row (Multicolour as a ring), highlight colour, icon and widget style, folder colour (ref `Settings-Appearance`) | s22, s24 |
| ST2 | P3 | Search results have no pane icons and both results show the selection plate at once | Icon rows, one selection | s22 |
| ST4 | P3 | Checkmarks and switches stay blue with the Purple accent (Settings, Reminders, the save panel) | They follow the accent | s24, s53, s46 |
| SN2 | P3 | The search field has no clear button; after clearing, the sidebar does not scroll to the selected pane | Clear button, selection kept in view | s31 |

### Apps

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| TE1 | P1 | TextEdit: clicks in the text never move the caret or select (double-click a word, type X: X goes to the end); Shift+Home does not select; B lights up but the text stays regular | Click and drag place and select; formatting applies to the selection | s42-crop, s44-word-bold-crop, s45-replace-crop |
| C2 | P2 | Calculator: square root after a result starts a new "√(" expression and drops 36 | Applies to the shown value (6) | s17 |
| C1 | P2 | Calculator Scientific: the cube-root key shows a missing-glyph box (U+221B) | The glyph | s15 |
| TE3 | P2 | TextEdit: no document title, also after saving; an extra text-button toolbar (New, Open, Save, Undo, Redo, Find, Print); the format bar uses text labels and is clipped ("Numbe"); the menu bar says "Text Editor" | Title with proxy icon; one format bar of icon controls and popups; name TextEdit (ref `TextEdit`) | s41 |
| N2 | P2 | Notes shows the Markdown marker in the title ("# Shopping list") | Title style, no "#" | s49, s50 |
| N4 | P2 | Notes lists the filesystem folders Documents, Library and tmp as note folders, in blue; "New Note" text button | A Notes folder (and Shared, Tags), yellow folder tint, compose button, the reference toolbar (ref `Notes`) | s48 |
| R2 | P2 | Reminders: "+" opens a full-pane form with Cancel and Done; Return in the title does not commit | An inline row at the end of the list; Return commits; details in a popover | s52 |
| AS2 | P3 | App Store notes show "(tools/make-icons.py)"; the Today card text is cut mid-sentence | User-facing notes; full text or an ellipsis | s56, s57 |
| C3 | P3 | Calculator mode is a segmented control plus an unlabelled "1.5" pill (decimal comma); "Delete" is small text under the display | Mode menu button; a backspace key | s15, s18 |
| N3 | P3 | The Notes list row only updates on refocus | Live | s49 |
| R3 | P3 | "Recently De..." tile label truncated | Fits | s51 |
| TE4 | P3 | Save panel: the base name is not selected; the panel floats instead of a sheet | Name selected; sheet | s46 |

## Not covered, for the next run

Mission Control and the Cmd+Tab switcher overlay (the `key` tool releases Cmd at once, so the switcher cannot be
seen), Stage Manager, Preview, Mail, Messages, Contacts, Maps, Photos, Stickies, Shortcuts, Podcasts and Voice
Memos playback (blocked by MU1), Settings panes other than General, Appearance, Sounds and About, Fast User Switching
and Log Out.

## Lab state after the run

Midget: Dark, Purple accent, volume 0.716, Settings minimised, Music, Notes, Reminders, TextEdit, App Store and the
earlier windows open. Test data: `~/Documents/e2e-note.rtf`, a "Shopping list" note, a completed "Buy milk" reminder,
`/Pictures/Screenshot-20261020-014807.png`.

## Re-test on 0.10.9 (03:15 to 03:27)

Midget 0.10.9 (`integration` at `676cb6a82`), apps from the same round installed through the store. Evidence `rNN-*.png`
next to this file.

| ID | Result | Notes |
|---|---|---|
| K1 | Fixed | Cmd+A in the Settings search field selects the text; the state stays `app:settings` (r02). |
| S1 | Fixed | Cmd+Space opens Spotlight over Settings, Esc closes it with nothing left behind (r03, r04). |
| K2 | Fixed | The window server no longer binds Cmd+A, so no "Applications:" box. |
| L1, L3 | Partly | F4 opens Launchpad above the windows, Esc closes it (r05, r06). Clicking the Launchpad icon in the Dock no longer opens it (r07, r08): new finding L4. |
| MC1 | Fixed | F3 opens Mission Control above the windows with a tile for every window and the desktops strip (r14). Closing with F3 brought Notes to the front instead of Settings: new finding MC2. |
| OV1 | Partly | Control Centre closes cleanly (r11). Notification Center still leaves blurred tiles until a full redraw (r13). |
| NC2 | Fixed | Notification Center opens below the menu bar; the clock stays visible (r12). |
| D3, D2 | Fixed | After the switch to Light every Dock slot keeps its own icon, one plate (r09-dock-crop). |
| W2 | Fixed | Full screen fills 0,0 at 2560x1440, the Dock hides, Ctrl+Cmd+F restores the old frame (r18). A faint menu bar stays over the window's top edge and clicking green opens a one-item menu: new finding W4. |
| TE1 | Fixed | Double-click a word and type replaces it; B applies to the selection and the bar shows its state; title "Untitled – Edited" (r22-crop). |
| SC1, SC2, SC3 | Fixed | Shift+Cmd+3 saves `~/Desktop/Screenshot-….png` owned by the user, with the whole composed screen; the thumbnail sits at the bottom right and slides away (r23, r24). |
| ST4 | Fixed | Checkmarks, the selection plate and the text selection follow the Purple accent. |
| N2 | Fixed | The note title shows without "#" (r15). |
| MU1 | Not fixed | The media service now runs (as the user, socket 0600) but Music never connects to it; Radio plays nothing. |
| AU1 | Not fixed | The bell plays nothing; the shell logs `sound: default output device: UnknownObject`. |

New: the shell draws a 21-22 ms CPU frame about once a second while idle with zero damage (log `slow frame ... damage=0% path=cpu`).

Fix wave `fix/e2e3-followups`: L4, MC2, OV1 (Notification Center), W4, MU1, AU1 (default output device), the idle frame,
plus the 0.10.10 build breaks (hdiutil after the control socket change, two capture tests, the control socket test).

## Re-test on 0.10.10 (04:08 to 04:16)

Midget 0.10.10 (`integration` at `8fd4bcf32`), Music 1.13.4, Podcasts 1.13.4, Voice Memos 1.10.4, Video Player 0.2.6.

| ID | Result | Notes |
|---|---|---|
| Security | Fixed | `/run/bifrost/shell-control.sock` is 0600 (user), nothing listens on 7777, `nobody` gets "Permission denied". As the user on the normal Wayland socket, `grim` reports "compositor doesn't support the screen capture protocol"; root through `/run/bifrost/wm-privileged` captures. |
| Capture | Fixed | Mission Control shows live tiles for all 11 windows, other apps' windows included (r32). |
| MC2 | Fixed | Closing Mission Control with F3 keeps Music in front (r33). |
| OV1 (Notification Center) | Fixed | Closing it leaves nothing on the screen (r34). |
| L4 | Fixed | The Launchpad icon in the Dock opens and closes it (r35, r36). |
| AU1, MU1 | Partly | The audio server now picks a default output (`default-output is now alsa:PCH:0 (ALC233 Analog)`); Music reaches the media service, shows a now-playing bar and states the error ("Cannot play: audio output: the audio server is not reachable; retrying later", r31). The bell and the media service fail one step later: `sound: add the IO procedure: UnknownObject`. Fix in progress (`fix/audio-ioproc`). |
| Idle | Not fixed, P1 | With Music retrying playback in front, the shell used 98.6% of a core and Music 25.6%: the shell drew about 65 frames a second with zero damage (`why app(external(42)):dirty`). Quitting Music brought the shell to 7.6%. Without Music the shell still draws a 22 ms CPU frame about once a second. Fix in progress (`perf/idle-redraw`). |

## Re-test on 0.10.11 (05:06 to 05:10)

| ID | Result | Notes |
|---|---|---|
| AU1 | Fixed | `tput bel` in Term: the PCM goes RUNNING; `device alsa:PCH:0 started at 48000 Hz, quantum 256`, `audio.speaker: opened the SoundEffects device`. |
| MU1 | Fixed | Music Radio (Rás 2) plays: the PCM is RUNNING for the whole 15 s sample, decoded peaks 1134 to 14557, the played-frame count rises, Pause and the station show in the player and the menu bar (r41). |
| Idle | Fixed | With Music playing in front: springboard 0.8% CPU (was 98.6%), Music 3.4% (was 25.6%), bifrost-audiod 1.2%, the media service 0.8%; no shell frame and no slow frame in 10 s of trace. |

New: the speaker trace reports `underruns 768` and a slowest write of 30 ms during radio playback, and the audio server logged one device xrun when the first stream started. Check for audible glitches and the stream buffer size.

## QA install and 0.10.12 (05:25 to 05:50)

- QA install of the lab ISO `bifrost-lab-20261020-051025.iso` (0.10.11 bundled) through the installer screens: pass in
  about 3 minutes (Welcome, Language with the Icelandic keyboard, Account, Look, Disk `/dev/nvme0n1`, Confirm, Progress,
  Restart); the installed system came up 22 s after Restart with the session and MCP running. Shots in
  `docs/e2e/2026-10-20-qa-install/`. Two tool fixes on the way (`tools/midget.py`): taps go through the installer's input
  FIFO (a temporary uinput pointer is not seen by the running installer), and typing waits 1 s after a field tap
  (Verify lost its first keys).
- The fresh system: no `/usr/share/springboard/assets` at all, the package-owned `system-assets`, 14 own icons and no
  vendor-named ones, the system services' sockets (audio owned by the user), the default output chosen, the control
  socket private.
- The installer's first start on the live ISO exited with "No such file or directory" while the lab sync was replacing
  the assets, and systemd restarted it a second later. It should wait for its assets instead of exiting.
- OTA from the fresh install to 0.10.12: `phase=accepted`.
- Radio on 0.10.12: `underruns 0 event(s) / 0 frame(s)` in every interval and in total, no xrun lines, the ring full
  (the 30 ms slowest write is back-pressure: every write waited for room).
- New: a fresh install starts with the system volume at 0, so nothing is audible until the volume is raised. Fix:
  `fix/default-volume` (half volume when nothing is saved).
