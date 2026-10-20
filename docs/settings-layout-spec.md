# Settings window layout spec (measured)

Source: `goldengate_refimages/27-Golden-Gate-Settings-General.png` (1704x1604, 2x capture: 1 pt = 2 px).
Only geometry, spacing, colour and type are taken from it. No Apple names, icons or artwork are copied; every
glyph is our own and every label is ours.

Measured on the light appearance. A dark set must be derived from the same geometry with our own dark tokens (light and dark are both drawn from `Pair` tokens now).

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
The measurements are now constants in `springboard/ui/src/prefs_layout.rs` (`m::*`) and the Settings drawing in `springboard/src/shell/list.rs` and
`settings.rs` uses them (checked 2026-10-13; the table below is what it said before that work):

| Spec | Now |
|---|---|
| Sidebar width 232 | `m::SIDEBAR_W`; `list.rs` `SIDEBAR_PT` is that constant |
| Row height 49, sidebar pitch 32 and row 30, inset 10 | `m::ROW_H`, `SIDEBAR_ROW_PITCH`, `SIDEBAR_ROW_H`, `SIDEBAR_INSET` |
| Search field in the sidebar | drawn at `m::SEARCH_Y` when the pane has a sidebar (`list.rs`) |
| Toolbar pill (back/forward) | `m::TOOLBAR_*` in `list.rs`, drawn when the page has a back target |
| Header card | `m::HEADER_H` (`list.rs`) |
| Detail row icons | `m::ROW_ICON`, `ROW_ICON_X`, `ROW_LABEL_X` |
| Account block | `m::ACCOUNT_*` (avatar in `list.rs`) |
| Radii | `m::WINDOW_RADIUS` 17, `CARD_RADIUS` 13, `SELECTION_RADIUS` 9, `ICON_RADIUS` 5 |

Settled (owner decision 2026-10-09): the old rule "every radius is at most 6" is replaced by the radii above (window 17, card 13,
selection plate 9, icon tile 5, search fields are capsules). Small legacy controls keep 6 pt until restyled. The sidebar group
gap in code is 13 pt (`m::SIDEBAR_GROUP_GAP`), not the 18 pt first read off the capture.


## Pane style (E2E findings ST5, ST6, ST7; measured from `Settings-Notifications`, `-Desktop-Dock`, `-Focus`, `-General`, `-Screen-Time`, `-Lock-Screen`, `-Sound`)

Shared helpers in `springboard/src/shell/list.rs`; the tokens are in `prefs_layout::m` (and `prefs_layout::toolbar_title_x`).

| Item | Rule | Code |
|---|---|---|
| Pane title | left-aligned in the toolbar, `TOOLBAR_TITLE_GAP` (12 pt) right of the back and forward pill, title 3 (15 pt) semibold; never centred. A pane with the hero hides it until the hero scrolls away | `NavBar::pill(ui, on)`, `NavBar::draw` |
| Group header | title-case headline above its card: 13 pt semibold (`MAC_HEADLINE_PT`) in the label colour. Specs still write capitals ("DOCK POSITION"); `headline_case` turns them into "Dock Position" at render time (acronyms such as USB and IPv4 kept, small words lower) | `headline_case`, `ListPage::in_pane` |
| Group footer | unchanged: grey, under the card | |
| Header card, hero | only **General** (a category page whose rows are all links): 166 pt card, 49 pt icon, 22 pt bold title and description centred. The toolbar title fades in as it scrolls away | `header_for`, `m::HEADER_*` |
| Header card, compact | **Notifications** and **Screen Time** (both carry a description in the reference): one row, 27 pt icon tile 11 pt from the left, title and description in a column starting 48 pt from the card edge, 12 pt padding; the toolbar title stays | `m::HEADER_COMPACT_*` |
| No header | every other pane (Display, Sound, Focus, Lock Screen, Desktop & Dock, Control Centre, Sharing, ...) starts at its first group under the toolbar title | `header_for` returns `None` |
| Value choice | a pop-up button on the row: the label at the left, the chosen value and the up and down chevrons at the right; tapping opens a menu of the choices over the button. The stored pref keys and `Pick` actions are unchanged: the row's action carries the current index and `pages::chosen` swaps in the one picked (`Pick::with_choice`). Used for dock position and minimise effect, screen saver kind, move interval, clock size and start after, game mode, mouse and trackpad secondary click, allow apps from, assistant backend | `Item::popup`, `pages::chosen` |
| View switch | a segmented control across the row (`SEGMENT_H` 28 pt, `SEGMENT_INSET` 11 pt from the card edge, the chosen segment filled in the accent): Screen Time Day and Week. A tap picks the segment under it (`Hit::Segment`) | `Item::segmented`, `ListPage::draw_segments` |
| List of many choices | a page that is only a list of choices (Auto-Lock, Language, Region, Fonts, ...) keeps check rows; the pop-up is for one value among a few | |
| Row label | stops before the value and the trailing control (chevron, switch, check, detail button, network icons, badge) and wraps onto a second line; a third line is cut with an ellipsis. Rows stay 49 pt | `label_lines`, `accessory_room` |

Open: Alert Sound (Sound) and the Hot Corners lists are still check lists; the reference uses pop-up buttons for both.

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
| Trackpad, Mouse | `PageId::Trackpad`, `PageId::Mouse` | present (tracking and scroll speed, natural scroll, tap to click, secondary click, button swap, double-click speed in `Options`) |
| Users & Groups | `PageId::Users`, `UserDetail`, `Groups`, `GroupDetail` | present (privileged work through privd `user.*`, `group.*`, [privd-protocol.md](privd-protocol.md)) |
| Printers & Scanners | `PageId::PrintScan` | present as a page; printing itself is the Print Center app (`printers-desktop`) |
| Keyboard modifier keys, Hot Corners, Shortcuts, Game Controllers, Menu Bar, Login Items & Extensions | `ModifierKeys`, `HotCorners`, `Shortcuts`, `GameControllers`, `MenuBar`, `LoginItems` | present |
| Spotlight, Siri | none | gap: needs search and assistant prefs |
| Startup Disk, Time Machine (as Backup), Internet Accounts, Wallet, Family, Profiles, Extensions | none or Backup only | gap: no backing service yet |

The Lock Screen and Sharing rows add no new state; they group controls that already have a page.
