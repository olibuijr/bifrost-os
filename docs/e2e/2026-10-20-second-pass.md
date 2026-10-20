# E2E on Midget, second pass: 0.10.13, 2026-10-20

Midget 0.10.13 (`integration` at `c0fa07e8b`), fresh QA install from earlier the same day, 2560x1440, Dark, blue
accent. Computer use through `tools/midget.py shot/click/type/key/scroll` plus a held-modifier plan for Cmd+Tab
(`kd 125`, Tab, hold, `ku 125`), 13:17 to 13:40. Twenty-three apps installed from the store first (Viewer, Mail,
Messages, Contacts, Maps, Sticky Notes, Shortcuts, Podcasts, Voice Memos, Weather, Stocks, Books, Fonts, Clock,
Calendar, Notes, Reminders, Text Editor, Calculator, Activity Monitor, Console, Disk Utility, Photo Booth; Tips is not
in the store).

Reference images: `goldengate_refimages/27-Golden-Gate-*.png` (Apps, Spotlight-Search-v2, Notification-Center,
Mission-Control, Finder-App-Switcher, About-This-Mac, About-This-Mac-Menu, Force-Quit, Settings-*, Mail, Messages,
Maps, Stickies, Disk-Utility, Console, Font-Book, Preview, Window-Management). Evidence next to this file in
`docs/e2e/2026-10-20-second-pass/` (`eNN-*.png`, `-small.png` at 1280 wide, `-crop.png` for the Settings window; on
disk only, `*.png` is gitignored).

## Covered for the first time

The Cmd+Tab switcher while Cmd is held, every Settings pane (Network to Passcode, 26 panes), Viewer, Mail, Messages,
Contacts, Maps, Sticky Notes, Shortcuts, Podcasts, Voice Memos, Weather, Stocks, Books, Fonts, Clock, Calendar,
Activity Monitor, Console, Disk Utility and Photo Booth, the window-management menu on hover, minimise to the Dock.

## What passed

- Launchpad search filters as you type ("no" gives Sticky Notes and Notes); Esc closes Launchpad cleanly.
- Mission Control (F3) shows live tiles for all four windows above everything; F3 closes it.
- Spotlight opens over Calculator with a dark capsule field and closes with Esc, nothing left behind.
- Cmd+Tab switches in most-recent order and commits on release (Calculator to Notes with two Tabs).
- W3 fixed: hovering the green button shows Fill, Centre, Return to Previous Size, Move & Resize, Tile with Next
  Window, Enter Full Screen.
- D1 fixed: a minimised Notes window shows as a window thumbnail with the app badge at the right of the Dock.
- C1 fixed: the cube-root key shows its glyph.
- Clock (world clocks), Calendar (month, today marker), Activity Monitor (live processes), Weather (live Reykjavík
  forecast), Shortcuts, Podcasts, Contacts and Books open and draw their empty states.

## Findings

Severity as in the first pass: **P1** breaks a basic task, **P2** wrong behaviour or a clear difference from the
reference, **P3** polish. Each row is one fix brief; IDs carried over from the first pass keep their ID.

### System UI

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| CT1 | P1 | The Cmd+Tab strip is never visible: switching works and commits on release, but nothing is drawn while Cmd is held (4.5 s and 6 s into the hold). `draw_cmdtab` draws in the desk layer, under the app windows | A glass strip of app icons above every window while Cmd is held (ref `Finder-App-Switcher`) | e05, e07 |
| L2 | P2 | Launchpad is a full-screen dark page with a 9-column grid and a small search capsule | A floating glass Applications panel under a capsule header, category chips, 7 columns (ref `Apps`) | e02 |
| S3 | P2 | Spotlight: the field is 0.25 of the screen width (ref 0.37) and short; the results are opaque with section headers ("Top Hit", "Settings", "Shortcuts", "Search in Apps"); Settings results draw blank coloured squares instead of pane icons; shortcut rows have no icon; rows are one line | Wider, taller field, glass results with filter chips, real icons, two-line rows (ref `Spotlight-Search-v2`) | e09 |
| NC2 | P2 | Notification Center is 0.17 of the width (ref 0.245), one text-only Notes card, "No Notifications" as a bold heading, "Edit Widgets" as a full-width bar | "Notification Center" heading with a clear button, glass widgets with icons (weather, reminders, calendar), capsule Edit Widgets (ref `Notification-Center`) | e10 |
| CC2 | P3 | Control Centre still has only Wi-Fi, Focus and Sound | Also Bluetooth, Display, Now Playing, Screen Mirroring modules (ref `Finder-Control-Center`) | e11 |
| SM1 | P2 | System menu: no Force Quit... (Option+Cmd+Esc does nothing), no Log Out user... (Shift+Cmd+Q), Recent Items inline as rows | Force Quit window, Log Out, Recent Items submenu (ref `About-This-Mac-Menu`, `Force-Quit`) | e12 |
| AB1 | P2 | About This Computer opens the Settings About pane; the model is the type code `10FLS2WK00`; the processor is truncated | A small separate About window with the name, chip, memory, serial, version, More Info... (ref `About-This-Mac`) | e13 |
| MC3 | P3 | Mission Control's Desktop 1 thumbnail is a blank grey plate | A small picture of the desktop with its windows (ref `Mission-Control`) | e08 |
| MB1 | P3 | A running Notes still adds an "N" status item to the menu bar | No per-app letters | e05 |
| D4 | P3 | The Dock has no divider before the minimised window and the folder | A thin divider (ref `Desktop`) | e64 |
| W5 | P3 | The window-management menu lists halves and quarters as text rows | Tile glyphs in a row (ref `Window-Management`) | e63 |

### Settings

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| ST5 | P2 | Group headers are uppercase grey footnotes ("CONNECTIONS", "NOTIFICATION STYLE", "FOCUS MODES"), an iOS style; pane titles are centred in the toolbar; some panes open with a large hero (Notifications, Focus, Screen Time, Displays, Sharing, Lock Screen) and others do not | Title-case bold group headers above each card; the pane title left-aligned beside the back and forward buttons; one compact header card where the reference has one (ref `Settings-Network`, `Settings-Notifications`, `Settings-Focus`) | e14-e26 |
| ST6 | P2 | Choices are inline checklists: Desktop & Dock position and minimise effect, Screen Saver kind, move interval and clock size, Screen Time Day/Week, General Game Mode, Mouse and Trackpad secondary click, Privacy "Allow apps from", Assistant backend | Pop-up buttons for value choices and segmented controls for views (ref `Settings-Desktop-Dock`, `Settings-Screen-Time`) | e17, e18, e19, e23, e30-e32, e36, e38 |
| ST7 | P2 | Overview pane: "When Switching to an Application, Switch to a Desktop with Open Windows" runs under its switch ("...with Ope") | The label wraps or ends with an ellipsis before the control | e24 |
| ST8 | P2 | Generic gear tiles for Print & Scan, Keyboard Shortcuts, Assistant, Trackpad, Mouse, Wallpaper and Spotlight in the sidebar, and for the Ethernet and VPN rows in Network; General's About row has an unrelated glyph and Software Update a blank circle | A distinct symbol per pane and row (ref `Settings-General`, `Settings-Network`) | e19, e28, e14 |
| ST9 | P2 | Notifications lists apps as text rows (no icon, no "Badges, Sounds" subtitle); Focus modes have no icons | App icon and summary per row (ref `Settings-Notifications`, `Settings-Focus`) | e16, e17 |
| ST10 | P2 | Battery pane on Midget (a desktop with no battery): "Battery Level None", "Status Charging", "Last Charged Now" and an empty 24-hour graph | No Battery pane without a battery (the reference shows Energy on desktops) | e37 |
| ST11 | P2 | Print & Scan shows the raw error "Printing is not available: printing service unreachable: Connection refused (os error 111)"; `cups` is installed but neither `cups.service` nor `cups.socket` is enabled | The printing service runs (socket-activated) and a plain message if it cannot | e26 |
| ST12 | P3 | Touch wording on the desktop: "Swipe down from the top-right corner of the screen" (Control Centre), "Tap the mode" (Focus), "Touch the screen or press a key" (Screen Saver), "Tap a folder" (Spotlight), "Tap to find this place" (Weather); a "Passcode" pane with "Custom Numeric Code" | Click wording on the desktop form factor; the login password | e17, e20, e36, e39, e41, e51 |
| ST13 | P3 | Menu Bar pane says "Control Center", the sidebar says "Control Centre"; Keyboard Shortcuts lists "Home: Cmd" on the desktop | One spelling; no Home row on the desktop | e21, e29 |
| ST1 | P2 | (still open) No Wi-Fi row at the top of the sidebar; "BifrOSt Account" a text row with no icon; General carries Nearby Share and a Game Mode group | ref `Settings-General` | e13, e19 |

### Apps

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| PV1 | P1 | Viewer (Preview) never opens: the journal says `cannot start is.olibuijr.preview.desktop: Unsupported app permission: screen`; the user sees nothing | It opens (with an Open panel when there is no document); a failed launch tells the user why | e42, e61 |
| DU1 | P1 | Disk Utility: "No disks were found." with an empty sidebar on a machine with an NVMe disk; `udisks2` is installed but not running and nothing starts it | The disks and volumes in the sidebar (ref `Disk-Utility`) | e59 |
| DU2 | P2 | Disk Utility's first toolbar button ("Mount") is drawn under the traffic lights | Toolbar items start after the window controls | e59 |
| MP1 | P1 | Maps: the map is black (no tiles), though `tile.openstreetmap.org` answers 200 from Midget; four toolbar buttons show "..." instead of their symbols | Tiles and symbols (ref `Maps`) | e46 |
| MA1 | P2 | Mail: the title "Mail" is drawn under the traffic lights ("Ma" then the buttons); Refresh and Compose are text buttons; no mailbox sidebar | Title clear of the controls; symbol toolbar; Favorites and mailboxes (ref `Mail`) | e43 |
| MS1 | P2 | Messages: the sidebar search field sits under the traffic lights; "Loading saved accounts..." stays in red at the top right | Field below or beside the controls; red only for errors (ref `Messages`) | e44 |
| FD1 | P2 | Notes and Voice Memos list the filesystem folders Documents, Library and tmp as their folders (N4 in the first pass) | The app's own folders only (ref `Notes`, `Voice-Memos`) | e06, e50 |
| SK1 | P2 | Sticky Notes opens with no note; a small hint "Choose File, New Sticky Note to add a note." floats over other apps' windows | A first note opens (ref `Stickies`) | e47 |
| CO1 | P2 | Console's sidebar says "This Mac" (a platform name in the UI; the names guard did not catch it); Hang Diagnostics and Spin Reports have no icons | "This Computer"; an icon per row (ref `Console`) | e58 |
| FB1 | P2 | Fonts: a collection row named "Off" under All Fonts; the font list column is pure black, unlike the rest of the window | Collections only; the window's own background (ref `Font-Book`) | e54 |
| C3 | P3 | (still open) Calculator mode pill "1.5" and the small "Delete" text | ref `Calculator` | e05 |
| WE1 | P3 | Weather's toolbar has a "Stocks" button | Weather's own controls | e51 |

### Logs

- `cannot start is.olibuijr.preview.desktop: Unsupported app permission: screen` (PV1).
- The user manager logs "Closed/Listening on Disk Image Download Service Socket" about every 30 s while apps start.

## Not covered, for the next run

Stage Manager (no entry point found), Fast User Switching and Log Out (no menu item, SM1), Photo Booth with a camera,
Voice Memos recording (microphone permission "not answered yet" with no prompt), Mail and Messages with an account.

## Re-test on 0.10.14 (15:00 to 15:10)

Midget 0.10.14 (`integration` at `fd8dd7886`), the eleven changed apps updated from the store. The first OTA rolled back
because a manual reboot was sent during system probation (`update --restart` already reboots); the second OTA was
accepted after about 50 s. Evidence `fNN-*.png` and `e14`-`e25`/`e70`-`e78` `g-`/`app-` shots taken after 15:00.

| ID | Result | Notes |
|---|---|---|
| CT1 | Fixed | Holding Cmd after Tab shows the icon strip above the windows with the selection and its name (f04). |
| SM1 | Fixed | System menu: Recent Items submenu, Force Quit... with Option+Cmd+Esc, Log Out user... (f05). Option+Cmd+Esc opens Force Quit; Down three times and Return force-quits Calculator (f08). |
| AB1 | Fixed | A separate About window: ThinkCentre M900, Intel Core i7-6700T, 15.5 GB, serial, 0.10.14, More Info... (f07). New AB2 (P3): "8 cores" counts threads; the i7-6700T has 4 cores. |
| L2 | Fixed | Floating Applications panel under a capsule header, category chips, 7 columns (f02). The panel is opaque, not glass (P3). |
| S3 | Fixed | Wide glass field and results, filter chips, real pane and shortcut icons, two-line rows (f03). New S4 (P2): the results panel runs down into the Dock (no height limit). |
| NC2 | Partly | Wider panel, quiet "No Notifications", Calendar and Notes cards with icons, capsule Edit Widgets with close (f10). No Weather card although Weather has data. |
| CC2 | Fixed | Bluetooth (Unavailable) and Display (Dark) modules added (f11); closing leaves nothing behind (f12). |
| D4, MB1 | Fixed | Dock divider before the minimised windows and folders; no per-app letters in the menu bar. |
| ST1, ST5, ST7, ST8, ST12 | Fixed | Wi-Fi row, account icon, title-case group headers, titles beside back/forward, a compact Notifications header, the Overview label wraps, a distinct tile for every pane, click wording (e14-e19, f13, f20). |
| ST6 | Mostly | Pop-ups in General, Desktop & Dock and Screen Saver; Energy's sleep time and Power Mode are still checklists (P3). |
| ST9 | Mostly | App rows show icons and "Banners, Sounds, Badges"; Deildu, Photos and Files have no icon (P3). |
| ST10 | Fixed | Energy instead of Battery on Midget, with Low Power Mode and Power Mode. |
| ST11 | Partly | A plain "The printing service is not running." But printing still fails: `cups.socket` is active and listens on `/run/cups/cups.sock`, while `bifrost_ui::cups` connects over TCP to localhost:631 (Connection refused). |
| ST13 | Partly | "Control Center" everywhere in Settings, but the Focus footer still says "Notification Centre". |
| PV1 | Fixed | Viewer opens ("Open an image or a PDF to view it"). |
| DU1, DU2 | Fixed | Disk Utility lists the NVMe disk and its partitions; the toolbar starts after the window buttons. |
| MP1 | Fixed | Maps shows OpenStreetMap tiles around Reykjavík and its toolbar symbols. |
| SK1 | Fixed | Sticky Notes opens a yellow first note. |
| FB1 | Fixed | Fonts: "All Fonts" and "Disabled"; the list uses the window background. |
| CO1 | Fixed | Console says "This Computer". |
| MA1, MS1 | Fixed | Mail's title clears the window buttons, with Favorites/Mailboxes and a symbol toolbar; Messages' search field sits below the buttons. |
| FD1, WE1 | Fixed | Voice Memos and Notes list only their own folders; Weather has no Stocks button. |

## Mail with a real account, keyboard and clipboard (0.10.15, 15:50 to 16:00)

The owner's own mail account (IMAP 993 SSL, submission 587 STARTTLS on the domain's published mail host) was added
through Add Account; the inbox loaded with real mail in about 10 s, and messages open. Evidence `m01`-`m08`, `t01`-`t12`.

| ID | Sev | Finding | Expected |
|---|---|---|---|
| MA2 | P1 | Add Account asks for the incoming and outgoing servers and ports; it does not look them up | Autoconfiguration from the address: the domain's autoconfig file (`autoconfig.<domain>/mail/config-v1.1.xml`, `.well-known`), then the `_imaps._tcp` / `_submission._tcp` SRV records, then MX; the server fields only under a manual option |
| MA3 | P1 | Senders and subjects show raw MIME encoded words (`=?utf-8?B?…?=`, `=?UTF-8?Q?…?=`, `=?ISO-8859-1?Q?…?=`), some decoded and some not | RFC 2047 decoding of every header (B and Q, any charset, adjacent words joined, folded lines) |
| MA4 | P2 | The message body shows HTML entities as text (`&#8202;`, `&bull;`) | Named and numeric entities decoded |
| MA5 | P2 | Emoji in subjects draw as empty boxes | An emoji fallback font (openly licensed) |
| MA6 | P1 | Compose (the toolbar button) opens a window the shell lists (`ExternalWindow(18, 1)` 750x738) that is never drawn; the whole screen dims instead, and after Cmd+W closes it the desktop stays dark except a strip near the Dock | The compose window shows; closing it leaves the desktop as it was. Sending was not tested because of this |
| MA7 | P2 | Cmd+A does nothing in Mail's Add Account fields (the old text stayed when retyping) | Select all |
| KB1 | P1 | Text Editor: Cmd+A, Cmd+C, Cmd+X and Cmd+V do nothing (nothing selected, nothing cut, nothing pasted, Undo has nothing to undo); after Cmd+C the Wayland clipboard is empty ("Nothing is copied"). Cmd+Up/Down and the arrows work, so the Cmd+letter chords are lost on the way to the app or its copy never reaches the clipboard | Standard editing shortcuts in every text view and field; copy and paste within an app, between apps, between the shell and apps, and with other Wayland programs |
| KB2 | P2 | Option+Backspace does not delete a word; Cmd+Backspace does not delete to the line start | Both, plus the other standard text keys (Option+Fn+Delete, Ctrl+A/E/K, Shift+Cmd+arrows, Page Up/Down) |
| KB3 | — | Passed: Home, End, Shift+Home, Option+Left and Shift+Option+Left by word, Cmd+Up/Down, Shift+Right selection, typing over a selection | |
| TB1 | P2 | Printers (like Disk Utility before 0.10.14) draws its first toolbar button under the window buttons: the SDK toolbar helper has no traffic-light inset | The inset in the SDK, so every app gets it |
| TL1 | tool | `midget.py type` assumes a US layout; Midget uses the Icelandic one, so `@` came out as `"` or `^` | Type through the machine's layout (AltGr+Q is `@` on Icelandic) |
