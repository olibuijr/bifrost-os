# Settings window layout spec (measured)

Source: `goldengate_refimages/27-Golden-Gate-Settings-General.png` (1704x1604, 2x capture: 1 pt = 2 px).
Only geometry, spacing, colour and type are taken from it. No Apple names, icons or artwork are copied; every
glyph is our own and every label is ours.

Measured on the light appearance. A dark set must be derived from the same geometry with our own dark tokens.

## Window (pt)

| Item | Value | Measured px |
|---|---|---|
| Window | 740 x 690 | x112..1592, y76..1456 |
| Corner radius | 17 | 34 px, from the corner arc |
| Sidebar width | 232 | 112..576 |
| Sidebar fill | rgb(235,235,235) | |
| Detail fill | rgb(255,255,255) | |
| Card fill | rgb(247,247,247) | |
| Drop shadow | soft, large blur, offset down | outside the window bounds |

## Sidebar (pt, relative to the sidebar origin)

| Item | Value |
|---|---|
| Traffic lights | centres y=26, x=26/49/72 (23 pt pitch), diameter about 14 |
| Search field | x10..222 (212 wide), y61..89 (28 tall), pill radius, magnifier and "Search" placeholder |
| Account block | avatar 38 pt circle at x16, name 15 pt semibold, caption 12 pt secondary |
| Row pitch | 32 |
| Row inset | 10 each side, so a selected row is 212 x 30 |
| Selected row | fill rgb(0,112,245), white bold label, radius about 9 |
| Icon tile | 20 pt, x=19 (inside the 10 pt inset), label starts at x=46 |
| Group gap | about 18 pt between the Wi-Fi group and the General group |
| Label | 13 pt regular, rgb(38,38,38) |

## Detail pane (pt, relative to the pane origin at x=232)

| Item | Value |
|---|---|
| Side margin | 20 |
| Card width | 468 (pane 508 minus 2 x 20) |
| Toolbar pill (back/forward) | x8..81, y8..44, so 73 x 36, chevrons 14 pt |
| Header card | y64..230 (166 tall): 49 pt icon tile, 22 pt bold title, 13 pt secondary description, centred |
| Card gap | 11 |
| Row height | 49 |
| Row icon tile | 23 pt at x=20 inside the card, label at x=50 |
| Chevron | 8 pt wide, secondary, 8 pt from the card edge |
| Separator | 1 px, from the label x to the card edge, rgb(232,232,232) |
| Card radius | about 13 |

## Mapping to our code

| Spec | Today |
|---|---|
| `SIDEBAR_PT` 232 | `list.rs` `SIDEBAR_PT` = 320 |
| Row height 49 / pitch 32 | `list.rs` row height from `RADIUS_PT`/iPad metrics, 6 pt radius cap ("flat corners" rule) |
| Search field in sidebar | `settings_search::FIELD_PT`, drawn only when `!pane.sidebar` |
| Toolbar pill | no equivalent; the nav bar is `IPAD_NAV_PT` = 50 |
| Header card | none |
| Detail row icons | `icons::tile` is used in sidebar rows only |

Conflict to settle with the owner: the repo rule "every radius is at most 6" (`list.rs`, `ui.rs`) contradicts the
reference radii (card about 13, window about 26, selected row about 9). A pixel-matching recreation needs the radius
tokens raised; the rule has to change first.


## Shell chrome (measured from the Golden Gate desktop captures)

Sources in `goldengate_refimages/`: `Finder-Menu`, `Desktop`, `Spotlight-Search-v2`, `Finder-Control-Center`,
`Notification-Center`, `Finder-Empty-Trash` (alert), `Login-Window`, `Mission-Control`. The captures are 2816 x 1762 (2x, so
1408 x 881 pt); measurements were read off a 2000 px view and converted (pt = view px x 0.704), so treat them as +-1 pt.

| Surface | Measured (pt) |
|---|---|
| Menu bar | 28 tall, clear glass, 13 pt labels, active title in a rounded plate; status icons and a `Wed Sep 9  8:11 PM` clock at the trailing edge |
| Menu | 200 wide, radius 14, 5 pt inset; rows 24 tall (34 view px), separators add 11; row highlight radius 7, accent fill; text 13 pt, key equivalents right-aligned in secondary; a 14 pt gutter before the label for a leading glyph or check |
| Dock | glass bar 74 tall, radius 21, 4 pt above the screen edge; icons 45 pt on a 58 pt pitch; a divider before the trailing group; running dot under the icon |
| Alert | 259 x 233, radius 28, content left-aligned: 47 pt icon, bold 14 pt title, 13 pt message, two capsule buttons 110 x 28 pt with 8 pt between, the default one filled in the accent |
| Spotlight | search capsule 520 x 85 (fully rounded, dark glass, 22 pt text) 139 pt from the top; 12 pt below it a results panel 518 wide, radius 31, with a row of filter chips (25 tall, capsule) and result rows 62 tall (37 pt icon, 15 pt title, 14 pt secondary subtitle); the selected row is a plate with radius 18 |
| Control Centre | 292 wide column of glass modules 12 pt apart; a wide tile is 141 x 63 (capsule), a round button 65, a slider module 292 x 62 with a title above the track; an "Edit Controls" pill 94 x 23 under the stack |
| Notification Center | cards 343 wide, radius 23, 12 pt apart, a 15 pt semibold "Notification Center" heading and a close button above the list; widgets use the same width and radius; "Edit Widgets" pill below |
| Login | large light clock (about 110 pt digits) with the date above it, user list at the bottom centre: 45 pt avatar and a 14 pt semibold name per row, no card |
| Mission Control | a strip of desktops across the top (170 tall, 144 x 90 thumbnails with a label under each, a plus at the trailing end); windows below keep their traffic lights |

## Pane inventory against the reference (macOS 27)

Checked against the settings extensions and preference panes in the reference index (`ExtensionKit/Extensions/*.appex`,
`PreferencePanes/*.prefPane`). Names and structure only; nothing is copied.

| Reference pane | BifrOSt | Status |
|---|---|---|
| Displays, Wallpaper, Screen Saver, Appearance, Dock, Desktop, Expose | Display, Wallpaper, Screen Saver, Appearance, Home Screen & Dock, Overview | present |
| Network, Wi-Fi, Bluetooth, VPN, Sound, Notifications, Focus, Screen Time | present | present |
| Sharing (hub) | `PageId::Sharing`: Nearby Share and AirPlay & Handoff | added |
| Lock Screen | `PageId::LockScreen`: Passcode, Lock After Inactivity, Screen Saver | added (links only) |
| Software Update, Date & Time, Language & Region, Login Items, Default Apps, Fonts, Keyboard, Game Controllers, Storage, Backup | present | present |
| Privacy & Security, Passwords, Accessibility, Battery | present | present |
| Trackpad, Mouse | none | gap: needs pointer prefs (tracking speed, natural scroll, tap to click) in `Options` |
| Users & Groups | none | gap: needs the account and login options data |
| Printers & Scanners | none | gap: needs a print service |
| Spotlight, Siri | none | gap: needs search and assistant prefs |
| Startup Disk, Time Machine (as Backup), Internet Accounts, Wallet, Family, Profiles, Extensions | none or Backup only | gap: no backing service yet |

The Lock Screen and Sharing rows add no new state; they group controls that already have a page.
