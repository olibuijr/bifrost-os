# BifrOSt shared application controls

`springboard/ui` is the independent `bifrost-ui` Rust crate. It has no shell,
renderer, Linux, Wayland or third-party dependencies. Its first release provides:

- Semantic controls: stable IDs, roles, labels, values, bounds and enabled state.
- Keyboard focus traversal and press/release activation respecting disabled controls.
- A Unicode-safe single-line editor with caret, selection, deletion and clipboard commands.
- Shared Cut/Copy/Paste/Select All command IDs and availability.

The SDK reexports it as `bifrost_app::controls`. `App::controls` exposes control
metadata and `App::commands` exposes actions available in the current focus.
`Input::Command` and `Input::Activate` deliver semantic actions. Metadata and
command availability are sent only when changed. The wire protocol bounds control
packets to 16 KiB and 128 nodes; invalid roles/geometry are rejected. New tags are
additive: older SDK applications retain their existing behavior and expose no
commands. They are never sent unsupported commands.

The shell stores control metadata alongside the app host. `bifrost pc midget state`
reports each app's node count and command bitmask. The desktop Edit menu enables
only commands the focused SDK app advertises, and dispatches them over its socket.
Native Linux applications retain their existing command path.

`bifrost_app::widgets::text_field` paints the editor using shared SF UI, system
colors, selected text, caret and focus indication. Files 1.0.4 adopts this field
and semantic Back/Forward/View/Search controls. Tab traverses enabled controls;
Enter activates a focused button. Left/Right/Home/End/Delete edit search text.
Cut/Copy/Paste/Select All are available through the desktop Edit menu.

## Invisible text characters (system 0.10.22)

The shared renderer omits Unicode default-ignorable characters from visible glyph
placement and measurement, including zero-width spaces, formatting controls and
soft hyphens. This prevents missing-glyph boxes in plain-text message viewers.
Joiners remain in the shaping input where supported; font fallback does not split
runs for invisible characters. GPU glyph placement and SDK outline drawing also
skip them. The typesetter shows a soft hyphen only where a line ends at it.

Regression coverage: `default_ignorable_characters_take_no_room_and_draw_no_box`,
`placements_skip_default_ignorable_characters` and
`a_soft_hyphen_is_invisible_unless_the_line_breaks_at_it`.
The line-fitting step does not yet reserve a visible hyphen's width. This change
does not establish complete script-shaping parity. Apps statically link these
paths and need rebuilding to receive the fix; Mail 0.17.1 includes it.

## System services (named service channel)

One generic channel lets an app use a shell feature without a new wire message. The app calls
`bifrost_app::service::call(ctx, "dock", "badge", &params)` (JSON params) and gets the answer in
`App::service_reply(ctx, request, &result)` (or polls `service::take_reply(request)`); a service can push
`App::service_event(ctx, service, event, &body)` to one app or to all. Wire: `ToShell::ServiceCall` (0x1d2),
`ToApp::ServiceReply` (0x1d2), `ToApp::ServiceEvent` (0x1d3); bodies are JSON text up to 64 KiB.

In the shell each service is one file in `springboard/src/shell/services/` implementing `SystemService` (`name`,
`call(shell, app_id, method, params, t)`, optional `tick`), registered with one line in `services::all()`. A call gets
the whole `Shell`, must answer at once, and returns `Ok(Json)` or an error message. The built-in `system` service
answers `ping` (echo) and `services` (the registered names). Parity wiring adds services here instead of new
protocol messages. About 35 are registered in `services::all()` (as of shell 0.7.25), among them `dock`, `pasteboard`,
`keychain`, `contacts`, `calendar`, `location`, `photos`, `local_auth`, `quick_look`, `now_playing`, `notify_bus`, `launch`,
`thumbnails`, `disks`, `screens`, `cursor`, `network_path`, `bluetooth` and `gamepad`; the list in that function is the truth.

The `screens` service reports the displays: `list` answers with each screen's frame, visible frame, scale and main
flag, and `changed` is pushed when the geometry changes. SDK client: `bifrost_app::screens` (`list`, `from_reply`,
`changed`). Since system 0.10.23, the visible frame reserves the Dock's selected edge
(Bottom, Left or Right); an auto-hidden Dock reserves no space. Filled windows and
this service use the same work area, independently of magnification and label padding.
Since 0.10.24, Settings changes apply after window-local input handling returns,
so filled-window reflow uses the full output dimensions.
Gaps (refresh rate, several displays) are in `docs/parity/wire-svc-screens.md`.

## Thumbnail generator (`QLThumbnailGenerator`, `bifrost_app::thumbnails`)

`GeneratorRequest::new(path, size, scale, types)` mirrors `QLThumbnailGenerator.Request`: `RepresentationType` is `Icon`, `LowQualityThumbnail` or `Thumbnail`, and an empty list means all. `plan()` gives the service requests in delivery order (icon, half-size picture, full picture). `Generation::accept` takes each reply or event and returns the `Representation` for the update handler, never one worse than already delivered. This is a pure model with host tests; PDF page rendering is not done yet.

## Reference SDK parity matrix

Every framework the official documentation lists for macOS 27 is tracked in `docs/apple-parity-matrix.md` (generated by
`bifrost pc parity build` from `docs/parity-matrix/data/`), with coverage per SDK and the remaining gaps. `python3 tools/bifrost.py pc parity status` prints the current numbers (at the last check: 246 frameworks assessed, 165 partial, 0 parity, coverage 71.0% of 1965 applicable feature rows; no row is `parity` until checked on Midget). The pure models
added by the parity waves have one note each in `docs/parity/` (API, the official names they mirror, REA evidence, gaps).
New SDK work updates its rows there in the same round (skill `bifrost-parity-matrix`).

## Golden Gate kit (batch 1, system 0.4.84, Files 1.13.0)

Plan and progress: `docs/golden-gate-sdk-adoption.md`. Every widget below follows Light and Dark (`Pair` tokens resolved
with `.now()`); `bifrost pc gallery [DIR]` (the `preview_sdk_gallery` test) renders them all in both schemes.

| API | Apple counterpart | Notes |
|---|---|---|
| `widgets::toolbar_band(s, ui, bounds)` + `widgets::toolbar(s, ui, bounds, &Toolbar, pressed)` | `NSToolbar` in a unified window | Glyph buttons (`ToolbarItem::icon(id, label, "symbol")`) in light glass capsules; `in_group(g)` joins buttons into one capsule (`NSToolbarItemGroup`); `selected(true)` marks a segment; `ToolbarItem::custom(id, label, width)` reserves a slot the app draws (title, search). `Toolbar::layout_with` keeps a group whole when it overflows behind `»`. Returns `ToolbarLayout` with `hit` and `rect_of`. **Breaking:** `toolbar` now takes a `Rect`, not `(&Layout, top)`; no app used the old form. |
| `widgets::source_list` with `SidebarItem::row(..).with_symbol("house")` | Source-list `NSOutlineView` | Accent glyph per row, neutral selection plate with a bold label (Finder, Notes, Reminders references), 9 pt plate radius (`prefs_layout::m::SELECTION_RADIUS`). `SourceList::set_selected(None)` when the shown place is not in the list. |
| `widgets::search_field` + `controls::search::{layout, hits_clear}` | `NSSearchField` | Capsule, magnifier, placeholder, clear button only with text, focus ring, white selected text on the selection. |
| `widgets::path_bar` + `controls::pathbar::{layout, hit}` | `NSPathControl`, Finder's path bar | Middle labels shrink to their glyph first, then the first, then the last label is cut. |
| `glyphs::glyph(s, ui, "name", rect, colour)`, `glyphs::NAMES` | `NSImage(systemSymbolName:)` | 28 line glyphs named after SF Symbols, drawn from lines and arcs; no symbol font is needed (the device has none). Unknown names draw nothing and return false. |
| `glyph_draw::paint(s, box, colour, dist)` with the shapes `disc`, `ring`, `segment`, `arc`, `ellipse`, combined with `either`; named icons `globe`, `mic`, `dismiss_keyboard`, `alarm_clock`, `stopwatch`, `timer` | No direct counterpart; the shape fill behind a symbol (`CGContext` fill of a path) | Anti-aliased signed-distance painters into a `Surface`: every pixel in `box` whose distance is within half a pixel of the shape is filled. The code lives in `bifrost_render::sdf_glyphs` (render owns it, because the shell and the SDK both depend on render); `springboard::shell::glyphs` and `bifrost_app::glyph_draw` re-export the same functions, so the shell and apps draw identical pixels. Name lookup stays in `glyphs::glyph`. |
| `widgets::capsule` | Liquid Glass button group plate | Soft ring, plate, one-pixel edge. |
| `controls::split::SplitView::toggle` | `NSSplitViewController.toggleSidebar(_:)` | Files binds it to the sidebar button and ⌃⌘S. |
| Manifest `window = "full-size"`; `layout::UNIFIED_BAR_PT`, `layout::traffic_lights_trailing()` | `NSWindowStyleMaskFullSizeContentView` | No title row. The shell draws the traffic lights at the Settings positions over the app; the top 52 pt band moves the window except over the lights and the controls the app publishes with `App::controls` (so publish every toolbar button). Needs system 0.4.84 (`min_shell`). |
| Manifest `window = "borderless"` | `NSWindowStyleMaskBorderless` with a clear background (`isOpaque = NO`, `ignoresMouseEvents` per pixel) | A full-size window with no shell chrome (no traffic lights, title band, rim, shadow, inactive dim or corner clip) that covers the work area. Where the app draws nothing (alpha 0) the pointer reaches the window below: the shell sets the toplevel's input region from the frame's alpha (`desk::alpha_shape`, `WindowFrame::input`) and `Desk::pick` skips the transparent part. Sticky Notes uses it for its notes (W2-22). Gap: the notes still share one window, so they raise and lower together; one window per note needs an app without a main window, which the SDK does not have yet. |

Moving an app to the kit:

1. Replace fixed dark tokens (`WHITE` text on neutral fills, `SECONDARY`, `FILL`, `SIDEBAR_FILL`, `SEPARATOR`, ...) with
   the `Pair`s (`LABEL`, `LABEL_2`, `LABEL_3`, `DETAIL_BG`, `CARD_BG`, `SIDEBAR_BG`, `SEPARATOR_C`, `FIELD_BG`) and `.now()`.
   `WHITE` stays right for text on the accent.
2. Draw the sidebar with `source_list` (symbols per row) and hit-test with `SourceList::hit`.
3. Build the toolbar as a `Toolbar`, draw it with `toolbar_band` + `toolbar`, hit-test with `ToolbarLayout::hit`, and
   publish every visible button as a control (`Node` with the same rectangle).
4. Use `search_field` for search and `path_bar` for a location footer.
5. Add `window = "full-size"` and `min_shell = "0.4.84"` to the manifest; keep the leading 87 pt of the top band free
   for the traffic lights (`traffic_lights_trailing()`).
6. Check with `bifrost pc adoption` (fixed colours left, widgets used, full-size) and a host preview in both schemes:
   `bifrost pc smoke APP --sizes 1200x780 --scale 1.5625` and the same with `--light`. `bifrost pc gallery` renders
   every kit widget after an SDK change.
7. On Midget, press toolbar buttons with `bifrost pc midget tap-window "External(N)" X Y` (window-local pixels from the
   screenshot; a full-size window has no title bar to add).

Golden Gate parity round 1 (`docs/golden-gate-parity-audit.md`, Files 1.14.0): toolbar capsules are 36 pt
(`CAPSULE_PT`), toolbar and sidebar glyphs 20 pt. Additions:

| API | Apple counterpart | Notes |
|---|---|---|
| `SidebarItem::row(..).with_dot(colour)` | Finder sidebar Tags (`NSOutlineView` row with a tag colour image) | 12 pt filled dot in the glyph's place. |
| `widgets::folder_icon(s, ui, rect, emblem)` | Finder's folder icon (`NSWorkspace.icon(forFile:)`) | Drawn at any size: back tab, paper edge, gradient body, embossed symbol (`Some("doc")`). Same art in Light and Dark. Apple uses bitmap icon sets; ours is procedural and has no custom folder icons yet. |
| `widgets::search_button(s, ui, origin, pressed)` | Golden Gate toolbar search item (collapsed `NSSearchToolbarItem`) | Round 36 pt glass capsule with a magnifier; the app swaps it for `search_field` when clicked and back on Esc or empty blur. |

Files look pass (E2E 2026-10-19, finding F7; `docs/e2e/2026-10-19-finder-browser.md`):

| API | Apple counterpart | Notes |
|---|---|---|
| `SidebarItem::row(..).with_symbol(..).neutral()` | Finder sidebar Locations rows (secondary-colour symbols) | The glyph is drawn in `LABEL_2` instead of the accent `TINT`; a disabled row stays `LABEL_3`. Files uses it for the whole Locations section (home, computer, trash, volumes); Favorites and Recents keep the accent. Default off, so no other app changes. |
| `widgets::folder_icon` emblem | Finder's embossed folder glyphs | The special-folder glyph is `FOLDER_EMBLEM` (a darker blue, about 45 below the body in luminance, as in the reference) over a `FOLDER_EMBLEM_HILITE` line a hair lower: an emboss that reads on the body in both schemes. Both tokens are in `render/src/ui.rs`. Folders without an emblem are unchanged. |
| Toolbar without `toolbar_band` | A full-size content window whose toolbar shares the content background | `toolbar_band` is the unified titlebar material with a hairline. An app whose toolbar floats over its content (Files, as in the Home reference) skips it: fill the window with `CONTENT_BG`, then call `widgets::toolbar` alone, and the groups draw as glass capsules over the content colour. Other apps keep their band. |
| `ToolbarItem::menu(id, label, Some("symbol"))` with `ToolbarLayout::menu_anchor(id, gap)` | `NSMenuToolbarItem` | Files's Group and Sort pop-up and its action menu ("...") open their menu under the button through `Ctx::context_menu` (window pixels, so points times `ui.s`); the choice returns as `Input::ContextAction(id)`. An app that lays the bar out itself must give a menu item `CAPSULE_PT + TOOLBAR_MENU_CHEVRON_PT` of width, as `widgets::toolbar_layout` does. Share is `share::show_picker` with `Payload::Url("file://...")` under the Share button; it takes the anchor in window pixels, like the context menu. |

Text Editor look and input pass (E2E 2026-10-20, findings TE1, TE3, TE4; Text Editor 1.12.0):

| API | Apple counterpart | Notes |
|---|---|---|
| `glyphs::NAMES` now 76: `tray`, `paperplane`, `archivebox`, `xmark.bin`, `arrowshape.turn.up.left`, `arrowshape.turn.up.left.2` | `NSImage(systemSymbolName:)` | Mail's mailbox and message-action symbols (inbox, sent, archive, junk, reply, reply all). Same line style as the rest; no hierarchical or palette rendering. |
| `glyphs::NAMES` now 70: `bold`, `italic`, `underline`, `strikethrough`, `text.aligncenter`, `text.alignright`, `text.justify`, `list.number` | `NSImage(systemSymbolName:)` | The format bar's character styles, alignments and numbered list. `bold` and `strikethrough` draw their letter with the UI font; the others are lines. Same line style as the rest. |
| Manifest `edit_keys = true` (`apphost::Manifest::edit_keys`, `HwKeys::nav_mods`, `Shell::hw_nav`) | `NSTextView` key bindings (`moveLeftAndModifySelection:`, `moveToBeginningOfLine:`) | A text editor in front gets the arrows, Home and End with Shift, Alt, Ctrl or Cmd (and Home and End alone) as chords on the evdev key: `KeyInput::chord(105, CHORD_SHIFT)` is Shift+Left, `chord(102, 0)` is Home. Without the key every app gets plain arrow codes with no modifiers, as before. The shell path has no key repeat for these chords yet, and the direct Wayland path (`wl/input.rs`, which deliberately sends chords on named keys as plain codes) does not send them, so a direct text editor does not get Shift+arrows yet. |
| Save panel sheet: the base name selected, the window under it dimmed (`docpanel::Panel::name_sel`, `Shell::draw_doc_dim`) | `NSSavePanel` as a sheet (`beginSheetModal`), the name field with the base name selected | Typing replaces the name and keeps the extension; Backspace deletes the selection. The panel still draws as a popup with rounded top corners at the top of its window; the sheet slide and square top corners are not done. |

Text Editor 1.12.0 uses the kit for its chrome: the title row (document icon and name, "Edited" while dirty) is drawn by the app because a full-size
window has no title row of the shell's; the format bar is one `widgets::toolbar` (a `ToolbarItem` group of B I U S, a group of the four
alignments, a `ToolbarItem::menu` list pull-down, and `ToolbarItem::custom` slots for the colour well, `widgets::popup_button` for font,
style and size, and a `widgets::stepper` for the line spacing). Menus open with `Ctx::context_menu` and come back as `Input::ContextAction` with
the menu id of the action; what does not fit moves behind the kit's `>>` button, whose menu the app builds from the hidden items. Pointer
selection is by click count (`select::Clicks`): a caret, a word, a paragraph, and a drag grows by that unit. Apple parity gaps: no Shift-click
to extend (the touch event carries no modifiers), no drag of selected text, and no line spacing stepper step smaller than a menu entry
(`SPACINGS`).

Files icon view: the default icon is 128 pt (`GRID_ICON`), the reference's folder width of 3.7 sidebar rows (about 118 pt
drawn); the cell is the icon plus 40 x 60 pt and the name sits 10 pt under it. View Options > Icon size still steps by
16 pt, from 32 to 160. Not matched yet: Network in the sidebar (Files has no network browsing view, only Go > Connect
to Server), and the reference's group pop-up glyph (a dotted grid with a chevron; the glyph set has only
`arrow.up.arrow.down`).

Apple parity gaps recorded in batch 1:

- **App menu items:** an app cannot add items to the menu bar yet, so View > Show/Hide Sidebar is the ⌃⌘S chord and
  the toolbar button only. Needs a protocol message for app menus.
- **Symbols:** our glyphs are a fixed set of drawings (56 after kit gaps round 2), not the SF Symbols catalogue, and have one weight. Asking for
  an unknown name draws nothing.
- **Search field:** no recent-searches menu and no search tokens (`NSSearchField.recentSearches`, `NSSearchToken`).
- **Path bar:** segments open their folder; there is no per-segment context menu or drag.
- **Sort:** the Finder group/sort pull-down was a glyph button that cycled the order; Files now opens a pull-down
  menu from its toolbar button (see "Files look pass" above); other apps keep their buttons.

Kit gaps round 2 (found by the app agents moving apps to the kit; additive, existing calls unchanged):

| API | Apple counterpart | Notes |
|---|---|---|
| `glyphs::NAMES` now 56: `chevron.down`, `backward.fill`, `forward.fill`, `play.fill`, `pause.fill`, `gobackward.15`, `goforward.30`, `person`, `person.crop.circle`, `star`, `star.fill`, `envelope`, `phone`, `bookmark`, `textformat`, `pencil`, `square.and.pencil`, `antenna.radiowaves.left.and.right`, `mic`, `music.note.list`, `opticaldisc`, `radio`, `calculator`, `cloud.sun`, `ellipsis.circle`, `info.circle`, `flag`, `calendar` | `NSImage(systemSymbolName:)` | Same procedural line style as the first 28 (filled ones use `icons::polygon`); `gobackward.15`/`goforward.30` draw their numerals with the UI font. Still one weight and no hierarchical/palette rendering. |
| `widgets::toolbar_layout(ui, bounds, &Toolbar) -> ToolbarLayout` | `NSToolbarItem` frames (`view.frame` of each item) | The exact widths `toolbar` uses (`toolbar` calls it). Apps hit-test without copying the width rule. A bar that starts left of `widgets::toolbar_lead_pt()` (traffic lights plus `TOOLBAR_GAP_PT`) in the lights' rows is moved right of them (`clear_of_traffic_lights`), so a bar laid out from x = 0 never draws under the window controls; a content toolbar beside a sidebar is left alone. A bar in a pane with its own coordinates uses `toolbar_in` / `toolbar_layout_in` with the pane's x offset. An app that calls `Toolbar::layout_with` itself starts its bounds at `toolbar_lead_pt()`. |
| `ToolbarItem::centered()` | `NSToolbar.centeredItemIdentifiers` | Centred items leave the flow; the rest is laid out around a hole at the first flexible space and the centred run sits mid-bar, pushed aside only as far as its neighbours need. A centred group stays one capsule. |
| `ToolbarItem::menu(id, label, symbol)` + `ToolbarLayout::menu_anchor(id, gap)` | `NSMenuToolbarItem` (pull-down toolbar button) | Drawn with a trailing chevron (`TOOLBAR_MENU_CHEVRON_PT` wider). On click the app draws `widgets::menu` at `menu_anchor`. The menu is still drawn inside the app window: an app menu cannot leave its window yet. |
| `SidebarItem::with_subtitle`, `with_trailing`, `with_avatar(initials, colour)`; `SourceList::two_line_h` | `NSTableCellView` in a source list (two-line rows, trailing text field, image view) | Subtitle rows are `two_line_h` (`row_h + 16`) tall; trailing text sits before the count; the avatar is a monogram disc. Apple shows a photo: draw it yourself over the leading 24 pt of the row. |
| `SidebarItem::section(id, label, collapsed)`, `SourceList::{hit_section, toggle_section, set_collapsed, is_visible}` | `NSOutlineView` group rows with disclosure (`isItemExpanded`) | The app owns the state. Rows under a collapsed section have zero height, do not hit, and are skipped by the arrows. The chevron always shows (no hover on touch). |
| `widgets::player(s, ui, bounds, &Player, pressed) -> PlayerLayout`, `PlayerLayout::new`, `PLAYER_PT` | Music/Podcasts mini player (`MPNowPlayingInfoCenter` data, `MPRemoteCommandCenter` buttons) | Glass capsule: artwork slot (the app draws into `artwork`), title and subtitle, progress line (`progress` rect for seeking), skip buttons from `skips` (`backward.fill`/`forward.fill` or `gobackward.15`/`goforward.30`), play/pause. |
| `widgets::pane(s, ui, rect) -> Surface`, `widgets::pane_origin_px` | `NSSplitViewController` child view | A clipped sub-surface whose origin is the pane's top-left, so a detail pane draws as its own view. Damage stays in full-surface pixels (add `pane_origin_px`). |
| Manifest `window_size = "WxH"` / `"fixed:WxH"`, `min_size = "WxH"`, `max_size = "WxH"` (points, content size); `apphost::Manifest::size_limits` | `NSWindow.contentMinSize` / `contentMaxSize`, a window without `NSWindowStyleMaskResizable` | The shell opens the window at `window_size`, clamps edge resizing to min/max, and a `fixed:` window has no resize edges and does not zoom. Not yet applied to native Wayland windows' own resize requests. |
| `glyphs::NAMES` now 59: `checklist`, `arrow.clockwise`, `newspaper` | `NSImage(systemSymbolName:)` | Same line style. |
| `glyphs::NAMES` gains `chevron.up` (the Shortcuts reorder arrows) | `NSImage(systemSymbolName:)` | The turned-over `chevron.down`; the array length is the count of names the glyph function draws. |
| `glyphs::NAMES` now 62: `text.alignleft`, `square.on.square` | `NSImage(systemSymbolName:)` | The Browser's page menu icon (lines of text, the last one short) and tab overview button (a square in front of another). Same line style; added for the Browser toolbar (E2E finding B7). |
| `MenuItem::destructive(true)` | `UIMenuElement.Attributes.destructive` (red Delete items) | `widgets::menu` draws the title in `DESTRUCTIVE` (systemRed) unless highlighted or disabled. |
| `MenuItem::swatch(colour)` | `NSMenuItem.image` (the colour dot of a calendar or tag pop-up) | `widgets::menu` draws a round `MENU_SWATCH_PT` swatch after the check mark column and moves every title over by it when any item has one. Calendar's Calendar pop-ups (quick-event popover and event form) open this menu (2026-10-23). Only a solid colour, not an arbitrary image yet. |
| `widgets::popover(s, ui, anchor, size, bounds, preferred_edge) -> Rect` | `NSPopover.show(relativeTo:of:preferredEdge:)` | Popover material panel with a hairline and an arrow at the anchor; flips via `split::place_popover`. Returns the content rect. The arrow is the material's opaque colour (no blur under it); the app handles transient dismissal. |
| `Pair::tinted(accent)`, `Pair::tinted_by(accent, dark, light)` | A calendar colour's fill (`NSColor` with alpha over the window background) | Opaque fill: 18% accent over white in Light, 32% over dark grey in Dark. For calendar event blocks and list colour plates. |
| `controls::calendar::MiniMonth::new(bounds, y, m, first_weekday)` + `widgets::mini_month(...)` | `NSDatePicker` graphical style (Calendar sidebar month) | Title row with previous/next, weekday initials, always 6 week rows; today red, selection a neutral plate, `marked` days get a dot. The previous and next months' days fill the six rows in secondary grey (`outside`, 2026-10-23, as the reference). `hit_day`, `hit_date` (also the greyed days), `prev`, `next` for clicks. |
| `calendar_store::EventStore` (`save_event`, `remove_event`, `save_reminder`, `remove_reminder`, `commit`, `reset`, `fetch_reminders`, `events_between`), `Event`, `Reminder`, `Span`, `ReminderPredicate` | `EKEventStore` write API, `EKEvent`, `EKReminder`, `EKSpan` | Pure in-memory model gated by `calendar_access::Access` (writes need a write grant, reads full access; else `EventStoreNotAuthorized`). `commit: false` queues until `commit()`; `reset()` drops the queue; each applying commit posts one `StoreChangeHub` change. `remove_event` with `.thisEvent` skips one occurrence of a repeat-every-N-days event, `.futureEvents` ends the series before it (or deletes it from the first occurrence). Saving replaces the whole item whatever the span. No attendees, no recurrence editing, not wired to the shell store; host tests only, not run on Midget. |
| `controls::collection::CardGrid` (`columns`, `frames`, `content_height`, `hit`, `with_hero`, `with_max_columns`) | `NSCollectionViewCompositionalLayout` (adaptive columns, a full-width group first) | News Top Stories: a hero card, then columns no narrower than `min_item_w`. Fixed row height; no self-sizing cells yet. |
| Shell `TextField` / `TextArea` text follows Light and Dark | `NSTextField`/`NSTextView` `textColor = .labelColor` | Text rendered white is drawn with `blit_colored(LABEL.now())`, so an appearance switch needs no rebuild and undo survives. Spotlight's field is fixed by this. Placeholder grey, caret and selection blue already read in both schemes. `TextArea` keeps its `color` parameter (the `NSTextView.textColor` equivalent): pass `WHITE` to follow the label colour, any other colour is drawn as is. |

`widgets::text_field` already covers single-line editors in both schemes (plate, text, placeholder, selection with
white selected text, caret; all `Pair::now()`), so apps on the legacy shell `TextField` drawing can move to it as is.
There is still no multi-line `text_view` widget in the SDK (`NSTextView`); none was added this round.

Round 26 (Files 1.12.1): `widgets::text_field` follows Light and Dark. The plate is `FIELD_BG`, text `LABEL`,
placeholder `LABEL_2`, the selection `SELECTION` with white selected characters (`Pair::now()`). Apple parity: the
NSSearchField look is a light capsule in light mode and a dark one in dark mode. Shell and apps that draw their own
fields still use fixed colours.

`bifrost_app::widgets::material(s, ui, bounds, radius, Material::…)` draws a system material
(`Sidebar`, `Titlebar`, `Popover`, `Hud`), the counterpart of `NSVisualEffectView.Material`. An app
cannot see the desktop through its window, so the material blurs the app's own content behind the
rectangle (draw the scrolling content first, then the material over it) and tints it. Reduce
Transparency turns it into the opaque `Material::solid()` colour. It is built on `Surface::glass_rrect`.

`Ctx::set_badge(label)` sets the app's dock badge, the counterpart of `NSDockTile.badgeLabel`: a short
label (control characters removed, at most four characters, empty clears it) shown as a red capsule on
the dock icon. The app needs the `notifications` permission; an older shell rejects the message, so apps
that use it set `min_shell = "0.4.60"`. Reminders uses it for the number of reminders due today or overdue.

The current drawing adapter still uses the existing shell drawing toolkit, and
Files still uses shell filesystem utilities. Extracting those dependencies is
separate work. This release establishes semantic controls and command transport;
it does not implement an AT-SPI screen-reader bridge, a complete layout engine,
IME preedit, undo history, shift-selection, or migrate every existing application.
Avoid claiming those capabilities or measured CPU/RAM savings.

Build/test the core with `cargo test --manifest-path springboard/ui/Cargo.toml`.
Publish shell through `bifrost pc publish`, then Files through
`bifrost pc app publish files`. Install shell before Files (minimum shell 0.4.17).

Verified on Midget: shell 0.4.18 and Files 1.0.4 installed, semantic controls
received, search filtered, and Edit Select All/Copy/Cut/Paste completed.
Core/protocol/Files/shell focused tests passed. Captures live in
`~/Pictures/tab/bifrost-ui-final.png` and `bifrost-ui-menu-verified.png`.

## AppKit drawing and print layout (`bifrost_app::drawing_model`)

`BezierPath` (NSBezierPath: move/line/curve, close, appendRect/Oval/RoundedRect/Arc, `bounds`, `contains` under
`WindingRule` NonZero or EvenOdd, `set_line_dash`, `line_width`, cap and join styles, `fill`/`stroke`/`add_clip` on a
`GraphicsContext`); `Gradient` (NSGradient colors at locations, interpolated colour, linear angle and radial
position); `ShadowStyle` (NSShadow offset, blur, colour, covered extent); `PageLayout` (NSPrintInfo scaling:
content size, page grid, page count, per-page document rect for A4/Letter, either orientation).

Wave 15 (2026-10-19): `BezierPath::stroke` now renders the path's `line_cap_style`, `line_join_style`, `miter_limit` (new field,
default 10) and dash pattern through the graphics context (they were stored and ignored) and restores the context's own line
settings. `Gradient::draw_in_path`, `draw_in_rect` (linear, at an angle, same direction as `linear_t`) and `draw_radial_in_path`
(relative centre -1..1, last colour at the farthest corner of the bounds) fill a path under its winding rule. `BezierPath::fill_with_shadow`
and `stroke_with_shadow` cast a `ShadowStyle` (`ShadowStyle::effect`) under the shape. `ColorList::system(dark)` is the built-in
System list of the 13 named system colours, `ColorLists::standard(dark)` starts a registry with it, `swatches()` and
`apply_to_panel(key, panel)` feed a colour well and the colour panel, and `to_text` / `from_text` keep a list in a text file.
Gaps: NSImage representations per codec, NSColorSpace pickers, an eyedropper, NSPrintOperation.runModal and NSPageLayout in the
shell are not built yet. Host tests only; not yet checked on Midget.

## Screen savers in the shell (`springboard/src/shell/screensaver.rs`)

Wave 15 (2026-10-19). Each shell saver (Clock, Colours, Black) has a descriptor parsed by `SaverManifest` (`screensaver::manifest`):
its name, frame interval and options. The options are the saver's configuration sheet: `ConfigureSheet` gained `text_of`,
`set_from_text`, `choices`, `int_range`, `is_bool`, `bool_or` and `choice_of` so a sheet is saved and restored as text. Settings >
Screen Saver shows the chosen saver's options under Preview (a switch per on or off option, a list of checks per choice option), and
each change is saved under `ScreenSaver.<module>.<key>` (the key space of `ScreenSaverDefaults`) and read by the shell
(`load_sheet`, `save_option`). Clock options: Show Date, Move Every (30 seconds, 1 minute, 5 minutes), Clock Size (small, medium,
large); Colours: Colour Change (slow, normal, fast). The Clock draws its position from `SaverRandom` (`clock_spot`), so one move number
always gives the same spot. The saver's frame interval comes from its descriptor (Clock one frame a second, Colours ten).

Hot corners (`springboard/src/shell/hot_corners.rs`): Settings > Desktop & Dock > Hot Corners now acts. The pointer resting in a
corner square (4 pt) for 350 ms runs the corner's action once (Overview, Apps Grid, Show Desktop, Lock Screen, Search, or the new
Start Screen Saver); leaving the corner re-arms it. Gaps: no `.saver` bundle host (savers are built in), integer options have no
Settings control, and the password-on-wake rule is the existing lock path (the saver ends on input and the lock screen shows as usual).

## UIKit and AppKit counterparts (`bifrost-ui`, pure models with tests)

Layout (`geometry`, `stack`, `split`): `EdgeInsets` (UIEdgeInsets); `Rect::inset`, `divide` (CGRectDivide),
`intersection`, `union`, `centered`; `stack` (UIStackView / NSStackView: fixed and flexible items, spacing,
shortfall shrinks flexible items first and nothing leaves the bounds); `grid` and `adaptive_columns`
(NSGridView, compositional layout); `SplitView` (NSSplitView: draggable divider, both minimums, collapse);
`place_popover` (NSPopover: preferred edge, flips when it does not fit, stays on screen); `visible_rows`
(UITableView cell reuse window with overscan).

Controls (`values`): `Segmented`, `Stepper`, `Slider`, `PageControl`, `Progress` (NSSegmentedControl,
NSStepper, NSSlider, UIPageControl, NSProgressIndicator). They hold state and map pointer positions to values;
drawing stays in the renderer adapter.

Navigation and presentation (`nav`): `NavigationStack` (back title is the previous screen's title, the root
cannot be popped), `Toolbar` (validation like `validateToolbarItem:`, flexible space, overflow menu when narrow),
`Alert` (default action answers Return, cancel answers Escape, macOS button order).

Behaviour: `ResponderChain` (an action goes to the first responder that handles it, menu items are enabled by
the same walk), `UndoManager` (grouping, bounded history, redo cleared by a new action), `KeyCombo` and
`Shortcuts` (parse "cmd+shift+z", show "⇧⌘Z", refuse two owners of one combination), `anim` (Bézier curves,
springs with a damping ratio, retargeting without a jump).

Foundation and UIKit kits (round 7): `diff` (`NSDiffableDataSourceSnapshot`), `cache` (`NSCache`, cost-limited LRU),
`fuzzy` (match with highlight positions), `format` (byte counts, positional durations, relative times, grouped
numbers), `gesture` (`Tap`, `LongPress`, `Pan` with velocity), `focus` (key-view loop and arrow focus), `uttype`
(`UTType` conformance; `uttype_declarations`: app-declared types, below), `typeset` (typesetter, framesetter, frame, line and run objects with UAX 14/29 breaking and paragraph styles; any font through a `GlyphSource`), `observe`
(`Observable<T>`), `calendar` (Gregorian arithmetic and month grids). Costs are in `docs/perf-history.jsonl`
(`ui_diff_*`, `ui_cache_*`, `ui_fuzzy_*`, `ui_typeset_*`, `ui_calendar_*`, `ui_focus_*`).

Round 8: `dockdrag` (`slot_at`, `preview_order`, `Slide`: eased per-icon offsets for reordering a row of icons, used by the
Dock) and `menu` (`NSMenu` / `NSPopUpButton`: `Menu`, `MenuItem`, keyboard stepping, type-to-select, hit testing,
`popup_origin`). The SDK draws them with `widgets::popup_button` and `widgets::menu`.

The SDK reexports the crate as `bifrost_app::controls`. None of these are wired into the shell or the apps yet:
this release adds the models, tested on the host (`cargo test -p bifrost-ui`, 114 tests).

### Semantic control roles (system 0.4.85 groundwork)

`bifrost-ui::Role` and the app-control wire protocol retain the original values (`Button=0`, `TextField=1`, `List=2`,
`Row=3`) and append `Toggle`, `Checkbox`, `Slider`, `SegmentedControl`, `StaticText`, `MenuItem`, and `Window`.
The agent map names them `toggle`, `checkbox`, `slider`, `segmented-control`, `text`, `menu-item`, and `window`.
Focusable roles now include the interactive types. This gives SDK widgets and apps a semantic vocabulary that matches
the controls they draw; hierarchy, `ax set` and the AT-SPI bridge follow in "Accessibility bridge (AT-SPI2)".

### Screen reader earcons (Accessibility)

`springboard/src/shell/accessibility/earcons.rs` synthesises one short cue per reader event family (`Earcon`: Focus,
Boundary, Wrap, EnterGroup, ExitGroup, ItemSelected, Error, ModeOn, ModeOff, PageTurn, Announcement, TextSelected), each
mono i16 PCM at `kb_clicks::RATE` and under 400 ms. `Earcon::for_event(ReaderEvent)` is total, and
`ReaderEvent::Posted(Notification)` maps the `NSAccessibility.Notification` set (focusedUIElementChanged, layoutChanged,
announcementRequested, selectedTextChanged, rowExpanded, rowCollapsed). `Reader` queues the cue on a focus move, wrap,
empty page, rotor change, press and unavailable press (`cue`, `take_earcons`; `set_sounds(false)` is the sound option).
Status: partial. The cues are synthesised stand-ins, not the original sounds, and the sound player does not drain
`take_earcons` yet (it lives in `bin/shell.rs`). The parity matrix holds no earcon rows to flip.

### Screen reader speech, reader roles and Spoken Content

Accessibility round, partial. What is built:

- **Speech settings.** Settings > Accessibility > Vision > Speech opens a page with three pop-up choices: Speaking Rate
  (Slow 140, Normal 180, Fast 240 words a minute), Pitch (Low 30, Normal 50, High 70) and Voice (System Default,
  English (US) `en-us`, English (UK) `en-gb`, Icelandic `is`). They are saved as `a11y_speech_rate`,
  `a11y_speech_pitch` and `a11y_speech_voice` (`accessibility::SPEECH_RATES`, `SPEECH_PITCHES`, `SPEECH_VOICES`). Loading
  clamps the rate to 80 to 450 and the pitch to 0 to 99, and drops a voice name that is not a plain word.
- **Speaking.** `accessibility/speech.rs` `args(text, &Config)` builds the espeak-ng arguments `-s rate -p pitch [-v voice]
  -- text`. `Shell::speech_config` reads the open Settings window's values, else the saved ones, and the banner's announcement
  path passes them to `Speaker::configure` before each utterance. The reader and the announcements share this speaker, so
  the speech service (`services/speech.rs`, for apps) keeps its own rate and voice.
- **Reader roles.** A Settings pop-up row (`Accessory::Popup`) is a `popup` element: spoken "pop-up button, Speech, Icelandic",
  in the Controls rotor. `walk::spoken_kind` has `sheet`, `panel` and `dialog` words. Extra windows of the front app are
  `window` elements already (`walk::window`). The AT-SPI role map (`axmap::item_role`) is unchanged, so Orca still sees the
  pop-up row as a choice.
- **Not built.** Help viewer, assistant panel and permission sheets are not walked for their content (only their windows).
  Speak Selection (Option+Esc) and Speak Items Under the Pointer need a selection source and a pointer-hover option; neither
  exists. Speech on the Midget is not verified.
- **Display options.** Reduce Motion, Reduce Transparency and Increase Contrast are already read by the shell's motion
  (`anim_ms`, desk, switcher, jiggle), material (`vibrancy.rs`, `glass.rs`, Control Centre, menu bar, Spotlight) and colour
  (`ui.rs` separators and material borders, `appearance.rs` contrast) code. Nothing new was wired.
- **Zoom.** Full Screen and Picture in Picture (`accessibility/zoom.rs`) are a presenter transform (`bare/wayland.rs`
  `set_zoom`); the rows in `docs/parity-matrix/data/accessibility.json` were corrected to say so.
- Tests: `accessibility/speech.rs` (args, clamp, configure), `accessibility/mod.rs` (speech prefs round trip and fallback,
  choices inside the engine range), `accessibility/walk.rs` (pop-up, sheet, panel, dialog and window words),
  `accessibility/reader.rs` (a pop-up row through `page_elements`), `shell/pages.rs` (speech voice row).

### Accessibility actions, rotors and display options

`bifrost_ui::ax::AccessibilityElement` has the rest of the action set: `accessibility_perform_show_menu`, `_cancel`,
`_confirm` and `_pick` (all decline by default), plus `accessibility_custom_actions` (`AxCustomAction`, listed by name and
run with `perform_custom_action`, which posts `layoutChanged`) and `accessibility_custom_rotors` (`AxCustomRotor`, whose
`step(from, forward)` walks its element ids and wraps). Apps read all display options with
`sdk::accessibility_display::options` and `options_changed`: `DisplayOptions` holds `increase_contrast`,
`reduce_transparency`, `differentiate_without_color` (Button Shapes), `invert_colors` (always false), `reduce_motion` and
`dim_flashing_lights`. The shell pushes `options_changed` once per change.

### Accessibility bridge (AT-SPI2)

Screen readers (Orca) read BifrOSt apps through the AT-SPI2 bus. Three layers:

- **Wire** (`springboard/proto`). `Node` is unchanged (id, role, label, value, bounds, enabled), so every app compiles as
  before. Hierarchy and state ride in a sidecar, `ToShell::ControlsTree(Vec<NodeExt>)` (tag `0x117`, at most 128 entries):
  `NodeExt { id, parent: Option<u64>, focused, checked: Option<bool> }`, each 17 bytes (id lo/hi, parent lo/hi, flags:
  bit0 has parent, bit1 focused, bit2 checkable, bit3 checked). An app that never sends it, or a node without an entry, is a
  direct child of the window, so old apps read as a flat list. The shell drops parents that are missing, self or cyclic
  (`atspi::model::build`). The other direction is `ToApp::AxSet { id, action }` (tag `18`) with `AxAction::{Press,
  SetValue(String), Focus, Increment, Decrement}`.
- **SDK** (`bifrost_app::App`). `controls_tree(&self, ctx)` (default empty, sent only on change) and
  `ax_set(&mut self, ctx, id, action)`. The default maps `Press` onto `Input::Activate(id)` (the same path as a tap on
  the control) and ignores `SetValue` and `Focus`; apps with editable nodes override it.
- **Shell** (`src/shell/atspi/`). `model.rs` is pure and always built: role map (Button=PUSH_BUTTON, TextField=ENTRY,
  List=LIST, Row=LIST_ITEM, Toggle=TOGGLE_BUTTON, Checkbox=CHECK_BOX, Slider=SLIDER, SegmentedControl=PAGE_TAB_LIST,
  StaticText=LABEL, MenuItem=MENU_ITEM, Window=FRAME), state bits, object paths
  (`/org/a11y/atspi/accessible/root[/app<slot>[/n<id>]]`). `bridge.rs` (cargo feature `atspi`, `dep:zbus`, already vendored
  for the portal and the desktop build) asks `org.a11y.Bus.GetAddress` on the session bus, connects to the a11y bus,
  exports `Accessible`, `Component` (extents, hit test, grab focus), `Action` (one action, `press`), `Value` (sliders),
  `Text` and `EditableText` (text fields) per node, and registers the root with `org.a11y.atspi.Socket.Embed`. It emits
  `StateChanged` (focused, enabled, checked), `Focus`, `PropertyChange(accessible-value)` and `ChildrenChanged`. The
  tree is published from `ext_tick` at most four times a second and only when it changed; the connection is made on a
  helper thread; with no `org.a11y.Bus` the bridge turns itself off. Screen reader requests queue to the shell thread
  and leave as `ToApp::AxSet`. No hand-written D-Bus marshalling was needed because zbus is available offline.

Closed in the text round (system 0.4.9x groundwork, AT-SPI side untested on a live Orca):

- **Label, title, help.** `Node::label` stays `accessibilityLabel`; `ToShell::AxMeta` (0x211, `bifrost_ui::ax::AxMeta`) carries
  `accessibilityTitle` and `accessibilityHelp`. The bridge's `Name` is the label (else the title), `Description` the help
  (else the role name), and `GetAttributes` lists `title` and `help`. SDK: `App::ax_meta`, `View::ax_meta`.
- **Text attributes.** `ToShell::AxText` (0x210, `bifrost_ui::ax::AxText`): insertion point, selected range (so
  `AXSelectedText` is `AxText::selected_text`) and visible range, as char offsets. `Text` exports `GetText`, `GetCaretOffset`,
  `CaretOffset`, `GetNSelections`, `GetSelection`; the visible range is the `visible-range` attribute. SDK: `App::ax_text`,
  `View::ax_text`, implemented by `TextField`.
- **Text events.** The bridge diffs each text field's value and attributes on publish and emits `TextChanged`
  (`insert`/`delete` with offset, length and text), `TextCaretMoved` and `TextSelectionChanged`, so Orca follows typing.
- **Announcements.** `Ctx::announce(text, priority)` sends `ToShell::Announce` (0x212). The closest AT-SPI equivalent is
  `Object:Announcement` (AT-SPI 2.46, `atspi_accessible_announce`), emitted on the app's frame with the politeness in `detail1`
  (low and medium polite, high assertive) and the text as `any_data`. The shell keeps at most 8 pending per app.
- **Secure fields.** A secure field is `PASSWORD_TEXT`, its value is replaced by bullets in `atspi::model::build_ax`, and
  `AXSelectedText` is always empty; only the length crosses the wire. Wired for the shell `TextField::ax_text` and the sign-in
  sheet (`deildu::login::Login::ax_fields`).

Parity gaps against `NSAccessibility` (recorded, not hidden):

- Hierarchy from the kit: `ax::begin_group(role, label, bounds)` / `ax::end_group()` make every node recorded in between a
  child of the group (`ax::take_links`, `parent_of`, `children_of`, `index_in_parent`); the SDK runtime merges the links into
  `ControlsTree` (`runtime::merge_tree`, the app's own entries win), so apps get the tree without changes. `ViewApp` already
  sent the view tree. Only `source_list` groups its rows so far (a `List` named "Sidebar"); the other immediate-mode helpers
  still record flat nodes.
- Shell chrome: `atspi::model::chrome_snap` publishes the menu bar (menus and rows nested), the dock apps (open ones marked) and a
  Spotlight entry as the application "BifrOSt Shell" (slot 250). It is read-only: a press on a chrome node is not routed back
  to the shell yet, and Spotlight carries no live text. Settings pages stay in the agent map only.
- `NSAccessibility` vocabulary in `bifrost_ui::ax`: `AxRole` / `AxSubrole` (the `AX*` constants, `From<Role>`), `AccessibilityElement`
  (`accessibilityParent`, `Children`, `Frame`, `Role`, `Value`, `setAccessibilityValue`, `isAccessibilityFocused`,
  `setAccessibilityFocused`, `PerformPress`, `PerformIncrement`, `PerformDecrement`), `AxElement` (`NSAccessibilityElement`),
  `AxCustomContent`, and `ax::post(element, AxNotification)` with `FocusedUIElementChanged`, `ValueChanged`, `LayoutChanged`.
  Posted notifications are queued (`take_notifications`); forwarding them as AT-SPI signals is not wired yet, so the
  bridge still derives its events from snapshot changes.
- `AxAction` gained `Increment` and `Decrement` (wire tags 3 and 4): `ViewApp::ax_set` steps a slider with the arrow keys
  (`Window::ax_adjust`), and the bridge lists `increment` and `decrement` actions on numeric elements.
- Extents are window-local points for both coordinate types: the bridge is not told window positions or the scale.
- No custom actions beyond press, no `AXRelations` (title-for, labelled-by), no rotor equivalent.
- The text attributes cover single-line fields only: no multi-line `AXInsertionPointLineNumber`, no `AXStringForRange`
  attributed runs, no `SetSelection` or `SetCaretOffset` from a screen reader (the bridge answers false; there is no
  `ToApp` message for it yet).
- The SDK `TextField` has no secure mode (no bullets drawn, no `AxText { secure }`); only the shell's own `TextField`
  and the sign-in sheet report masked fields. The sheet's `Login::ax_fields` feeds `atspi::model::build_ax`, but the
  shell does not yet publish its own chrome (sheets, menus) as an AT-SPI application, so Orca cannot reach it until it does.
- `accessibilityTitle` and `accessibilityHelp` are set on the SDK `TextField` only (`with_ax_title`, `with_ax_help`); other
  controls report no title or help yet.
- `Value` is parsed from the node's value text ("0.4", "40%"); a typed min/max/step needs a node field.
- `bifrost_ui::ax::Settable` is the default role-keyed handler for `setAccessibilityValue(_:)`, `setAccessibilityFocused(_:)`,
  `accessibilityPerformIncrement()`/`Decrement()` on `.slider` (clamped to min/max, stepped) and `.textField` (replaces text), posting
  `Notification::ValueChanged` and `FocusedUIElementChanged`; other roles are unchanged. Routing it from every app's `ax_set` and a
  live Orca check are still open.
- `SetValue`, `Focus`, `Increment` and `Decrement` reach the shared view kit through `ViewApp::ax_set` (sliders, text fields,
  checkboxes, pop-up buttons); a plain `App` that does not override `ax_set` still handles only `Press`.
- Rows are not selectable (`Selection` interface) and lists are not `Table`s.
- Untested on a live Orca: unit tests cover the wire round trips and role/state/path mapping only.

### Remaining Apple-style SDK roadmap

Priority order based on existing app needs and current implementation seams:

1. (Done, see "App-published menus") Wire app-owned `NSMenu` items and validation into the desktop bar; before, only shell menus and app context menus crossed
   the protocol.
2. Add drawable Golden Gate adapters for standard controls already modeled in `bifrost-ui::values` (`NSSwitch`,
   `NSSegmentedControl`, `NSSlider`, `NSStepper`, `NSProgressIndicator`) plus grouped form rows, and record the matching
   semantic nodes automatically.
3. Accessibility (`NSAccessibility`): parent/child relations (`ToShell::ControlsTree`), `ax set` (`ToApp::AxSet`) and
   the Linux AT-SPI2 bridge are in (feature `atspi`, see "Accessibility bridge (AT-SPI2)"). Text ranges, caret events and live announcements are in. Still open: window actions,
   routing presses on shell chrome nodes, and grouping in the immediate-mode helpers other than `source_list`.
4. Add typed App Intents (`AppIntent`, `AppEntity`, `AppShortcut`) alongside app-control RPC methods.
5. (5a-5c done, see "User notifications", "Share sheet" and "Typed drag and drop") Add the brokered share sheet (`NSSharingServicePicker`), actionable/reply notifications (`UNUserNotificationCenter`),
   and typed drag and drop (`NSDraggingDestination`/`NSDraggingSource`) using `uttype`.
6. Permissioned Keychain-style secret storage: done (see "Keychain" below); secrets must not be stored in app-readable defaults files.
7. Complete the text input contract (`UITextInput`/`NSTextView`) with marked text, selection/composition, return-key
   variants and input accessories.
8. Complete document panels with multiple selection, folder selection, places, recent locations, tags and New Folder.

These are staged API and protocol changes, not claims that the Apple counterparts are already implemented. The shared
kit also needs adoption beyond Files; current baseline is in `docs/golden-gate-sdk-adoption.md`.

## NSColor semantic colours (`ui::system`, SDK `bifrost_app::kit::colors`)

Apple parity: AppKit apps never pick a dark or light value; they ask for `NSColor.windowBackgroundColor`,
`labelColor` and so on, which resolve against `NSAppearance` at draw time. Ours are `Pair`s in
`render/src/ui.rs` `mod system`; call `.now()` while drawing. A plain constant (`BLACK`, `CELL`, `WHITE`,
a literal `pack(...)`) never follows Light/Dark: do not use one for a window or content background.
The guard test `appearance_guard::window_and_content_fills_follow_light_dark` fails on `fill_rect`/`fill_rrect`
with `BLACK` or `CELL` in `src/shell` and `apps/*/src` unless the file is in its allowlist (video, viewfinders,
swatches, letterbox, blur bases, plus the legacy gaps below).

| AppKit `NSColor` | `ui::system` | Existing pair |
|---|---|---|
| windowBackgroundColor, controlBackgroundColor, textBackgroundColor | `WINDOW_BACKGROUND`, `CONTROL_BACKGROUND`, `TEXT_BACKGROUND` | `DETAIL_BG` |
| underPageBackgroundColor | `UNDER_PAGE_BACKGROUND` | `SIDEBAR_BG` |
| labelColor, textColor | `LABEL`, `TEXT` | `LABEL` |
| secondaryLabelColor | `SECONDARY_LABEL` | `LABEL_2` |
| tertiaryLabelColor, placeholderTextColor | `TERTIARY_LABEL`, `PLACEHOLDER_TEXT` | `LABEL_3` |
| quaternaryLabelColor | `QUATERNARY_LABEL` | new |
| separatorColor | `SEPARATOR` | `SEPARATOR_C` |
| gridColor | `GRID` | new |
| selectedContentBackgroundColor | `SELECTED_CONTENT_BACKGROUND` | `SELECTION` |
| unemphasizedSelectedContentBackgroundColor | `UNEMPHASIZED_SELECTED_CONTENT_BACKGROUND` | `SIDEBAR_PLATE` |
| controlAccentColor | `CONTROL_ACCENT` | new (Dark 10,132,255; Light 0,122,255); follows Settings > Appearance > Accent colour (`Pair::following_accent`) |
| linkColor, systemBlue | `LINK`, `SYSTEM_BLUE` | Dark 10,132,255; Light 0,122,255; fixed, the accent does not change links |
| The accent reaching an app process | `ToApp::Appearance` flags, bits 8 to 11 (`appearance::ACCENT_SHIFT`, `ACCENT_MASK`; `Accent::to_wire`) | The shell sends the accent with the other appearance flags and the SDK runtime calls `appearance::set_accent`, so `CONTROL_ACCENT.now()`, `appearance::accent_now()` and `SELECTION` follow the user's accent in apps too (0 from an older shell keeps blue). Controls tinted by the accent use `accent_now()`: list checkmarks, switches, the default button of sheets and the save panel, focus rings. Text links keep `LINK`. Apps that draw their own switch use `CONTROL_ACCENT.now()` (Reminders does); other hand-drawn blue controls are not converted yet. |
| selectedTextBackgroundColor (Highlight Colour), sidebar icon size, `NSScroller.preferredScrollerStyle` (Show Scroll Bars) | `ui::highlight_now()`, `ui::sidebar_icon_scale()`, `ui::scroll_bars_always()`; `ToApp::Appearance` bits 12 to 15, 16 to 17 and 18 to 19 (`appearance::HIGHLIGHT_*`, `SIDEBAR_ICON_*`, `SCROLL_BARS_*`, value plus one, 0 = not sent) | Highlight Colour paints text selections (kit text fields, `TextLayout`, TextEdit, the shell's fields and text areas); "Accent Colour" is the `SELECTION` plate. Sidebar Icon Size scales `SourceList` row heights (drawing and hit-testing), the kit's row glyphs and the shell's Settings sidebar (small 0.85, large 1.2). Show Scroll Bars "Always" keeps the kit scroll views' knobs drawn at rest; Apple's legacy scroller with a track is not drawn yet (knob only). The SDK runtime applies the three on every appearance push, so apps follow a change live. |
| systemRed ... systemGray (13 colours incl. mint, cyan, indigo, brown) | `SYSTEM_RED` ... `SYSTEM_GRAY` | Apple's Light and Dark values |
| windowFrameColor, findHighlightColor, shadowColor | `WINDOW_FRAME`, `FIND_HIGHLIGHT`, `SHADOW` | new (findHighlight is a design approximation, confirm on Midget) |

Gaps (Apple parity rule):
- `windowBackgroundColor` and `controlBackgroundColor` share `DETAIL_BG` (Dark pure black, per the Golden Gate reference); Apple's Dark values differ slightly.
- Other `NSColor` names are not provided.
- The old `u32` constants (`SYSTEM_RED` ...) remain for fixed-colour uses; migrating callers is ongoing.
- Still dark-only (allowlisted in the guard): Browser chrome, Clock full-screen, the power alert, `apps/remote`.
  Fixed in 0.4.100 work: the built-in App Store (all backgrounds, text, separators, tab bar) and it now redraws during the Light/Dark fade.

## Appearance for external apps (third-party Wayland, GTK, Qt)

On macOS an app follows Light/Dark without opting in. Third-party Linux apps follow it only through standard channels:
- `org.freedesktop.portal.Settings`, namespace `org.freedesktop.appearance`: `color-scheme` (u: 0 no preference, 1 dark, 2 light)
  and `accent-color` ((ddd), 0..1). Apps (GTK 4/libadwaita, Qt 6.5+, Firefox, Chromium, Electron) read it and subscribe to
  `SettingChanged`. This is served by `bifrost-portal` (see "Native apps follow the appearance"), so the channel exists.
- Env hints for toolkits that do not use the portal at launch: `GTK_THEME=Adwaita:dark` (or `:light`), `QT_QPA_PLATFORMTHEME`
  and `QT_STYLE_OVERRIDE`, `COLOR_SCHEME_PREFERENCE`-style variables for some apps. These are read once at start.
- Gaps: the launcher does not yet set these env hints per scheme, so GTK3 and older Qt apps stay on their default until
  restarted with them; `contrast` ignores Increase Contrast; `xdg-desktop-portal` frontends (file chooser, screenshot)
  are not implemented; a `gsettings`/dconf `color-scheme` mirror for GTK apps that ignore the portal is not written.
  Not implemented in this change.

## System fonts (`NSFont.systemFont`, `monospacedSystemFont`; Settings > Fonts)

- **Defaults.** The packed `SFUI.ttf`, `SFUIMono.ttf` and `Symbols.ttf` are open-source lookalikes (Inter,
  JetBrains Mono, Noto Sans Symbols 2), fetched and checksummed by `tools/pack.py`; see the README "Fonts".
- **Override.** `bifrost_render::fontprefs`: `FontRole::{System, Mono}`, `map_font(dir, role)` maps the font
  saved in `settings.ini` (`font_system`, `font_mono`) or the default when it is unset or does not parse. The shell
  (`bin/shell.rs`) and the SDK runtime (`run_on`) both load through it, so every app draws with the user's choice.
  It applies from the next start (Settings offers Restart BifrOSt).
- **Font library.** `fontprefs::library(dir)` is `<data dir>/fonts`; `installed(&[dir])` lists one face per family
  (variable upright first, then Regular), flags fixed-pitch faces (`post.isFixedPitch` or PANOSE) and reads only
  the table directory and the `name`, `post` and `OS/2` headers. Host and distribution fonts are never offered.
- **Apple parity, decided.** On the reference desktop the system font is fixed and Font Book manages installed fonts.
  BifrOSt deliberately allows the override because its defaults are lookalikes. Not matched: Font Book (install,
  validate, collections, disable) and live font switching without a restart. `FontPanel` families can come from
  `fontprefs::installed`.
- **System font library (OS assets, Core Text).** The runtime image carries open equivalents for the reference
  system fonts: `noto-fonts` and `noto-fonts-extra` (script coverage, so `CTFontCreateForString` /
  `CTFontCopyDefaultCascadeListForLanguages` fallback finds a face for most scripts), `ttf-liberation` and
  `ttf-croscore` (metric-compatible sans, serif and mono), `ttf-dejavu` (monospace, symbols), `otf-stix` (math),
  `gsfonts` (symbol and dingbats), `unifont` (Braille and last-resort glyphs).
  `packaging/arch/fontconfig/60-bifrost-aliases.conf` maps the compatibility family names (Arial, Helvetica,
  Times New Roman, Courier New, Menlo, Monaco, Symbol, Zapf Dingbats, STIX, LastResort) to them with
  `binding="same"`; `springboard/tests/font_packages.rs` guards the package list, the alias targets and the
  installer. `61-bifrost-cjk-fallback.conf` is a fontconfig scan rule for the Browser engine's fixed fallback list (CJK
  punctuation; see "Web view", "Where Servo 0.7.0 limits this"). The matching `docs/parity-matrix/os/assets` rows are `partial` ("open equivalent X, glyph coverage not
  metric-identical") until a Midget check. Not matched: Latin display, SF variant and CJK decorative families
  (substitution task), and Apple's exact glyph shapes outside the Liberation/Croscore metrics.

### Family substitution (Core Text descriptor matching, `bifrost_app::font_substitution`)

- **Substitution.** `UserFonts::resolve(shell_dir, &FontSpec)` first matches the descriptor against the font library.
  When nothing matches, the named family is looked up in a static table (`class_of_family`: grotesque, geometric and
  humanist sans, didone and transitional serif, slab, script, monospaced, rounded, CJK gothic/mincho/kai/rounded) and
  the open equivalents of that class are tried in order (`substitute`, `candidates`). An unknown name falls back by
  traits (`class_for_spec`: fixed pitch is monospaced, else the system grotesque sans). Weight, size and the other
  attributes carry over.
- **System design.** `UserFonts::with_design(spec, SystemDesign::{Default, Rounded, Serif, Monospaced})` mirrors
  `withDesign(_:)`: the family becomes the design's first open equivalent, monospaced also sets fixed pitch.
- **Cascade list.** `UserFonts::cascade_list(&["ja", "zh-Hant", "ar", ...])` mirrors
  `CTFontCopyDefaultCascadeListForLanguages`: ordered script families per language tag, repeats dropped; feed it to
  `FontSpec::with_cascade`.
- **Decided / not matched.** No original face ships, so a substitute is another design of the same class. The listed
  open families (Inter, Outfit, Noto Serif, Noto Sans CJK, ...) must be in the font library to resolve. Matching
  returns `Installed` records, not descriptor objects with mandatory-key sets. Parity rows stay partial until a
  Midget check.

## One stylesheet for apps

Apps take text sizes from the `MAC_*_PT` and `DISPLAY_*_PT` tokens, spacing from `SPACE_*` (4-pt grid), and the
top inset from `layout::top_inset()` (the shell's safe area; 0 on the desktop). `tools/check-app-consistency.py
--ratchet` fails when an app gains a literal size, on-grid spacing literal or status-bar inset;
`tools/fix-type-scale.py`, `fix-spacing.py` and `fix-status-inset.py` rewrite them. Bifrost scales to any screen,
so layouts derive from `layout::Layout` (window size in points) and never from a fixed resolution.

## Menus in the shell (AppKit `NSMenu`, 0.4.68)

The desktop bar menus use the kits instead of private code: `keys::KeyCombo` is the key equivalent shown in the
right column (`MENU_KEYS` in `shell/desktop_bar.rs`, conflict-checked by `keys::Shortcuts` in a test), `menu::Menu`
does type-to-select, and `menu_spans` is the one row-geometry function (rows 24 pt, separators 11 pt, 5 pt padding at
both ends). Menus size to
their content (at least 264 pt; the width is cached per row set), long titles are cut with `ui.fit`, and Reduce
Transparency draws them opaque (`DESKTOP_MENU_SOLID`). Keys with a menu open: Up/Down, Left/Right, Return, Esc,
letters. Cmd+, opens Settings.

### App-published menus (`NSApp.mainMenu`, roadmap item 1)

An SDK app overrides `App::main_menu(&self, ctx) -> Vec<AppMenu>` (`AppMenu { title, items: Vec<AppMenuItem> }`;
`AppMenuItem::new(id, title).key("cmd+n").enabled(..).checked(..)`, `::separator()`, `::submenu(title)` followed by its
children at `.depth(1)`). The runtime calls it after every event and frame and sends `ToShell::AppMenus` (tag 0x117)
only when the result differs, so recomputing enabled and checked state there is `validateMenuItem:`. The desktop bar
opens them with the same menu drawing and sends the chosen item back as `ToApp::MenuAction(id)` (tag 18), delivered to
`App::menu_action(&mut self, ctx, id) -> bool`. Bar order (Golden Gate; `desktop_bar::bar_after_app`): the app name,
File, Edit, View, then the app's own menus in the order it publishes them (the Browser's History, Bookmarks, Develop),
Window, Help; a Format, Font, Text or Insert menu sits right after Edit instead. A menu titled File, Edit, View, Window
or Help merges into the shell's menu of that name. Key equivalents (`KeyCombo::parse` notation) of enabled items are
matched in the shell before a chord reaches the app (`performKeyEquivalent:`); the app gets `MenuAction` instead of the
key. They also win over the shell's window chords: while the app in front publishes an enabled item on Cmd+W (a tabbed
app's Close Tab) or Cmd+M, the shell does not close or minimize the window with it (`Shell::app_owns_chord`, checked in
`bin/shell.rs` before `hw_shortcut`). Publish the item disabled and without a key when it has nothing to do (the
Browser's last tab) and the shell closes the window again. Shift+Cmd+W always closes the window, and the File menu's
Close Window shows it while the app owns Cmd+W. Cmd+Y is not Redo, so an app can use it. In the shell's own fields Ctrl+Y redoes; in an SDK app the shell leaves it to the app, where the text engine reads it as the yank (Ctrl+K then Ctrl+Y puts the killed text back) and redo is Shift+Cmd+Z. Limits: 8 menus; 512 items
in each menu level, counted apart for the menu's own items and for each submenu's children (`AppMenu::kept_items`), and
1024 in one menu with its submenus; titles cut to 64 characters. A set that does not fit one 16 KiB packet goes as
`AppMenus` and then `AppMenusMore` packets (tag 0x251, `proto::app_menu_packets`), whole menus each; anything still left
out is logged by the SDK (and menus past 8 by the shell), never dropped silently. A submenu header is a row with a
chevron whose submenu cascades beside it as `NSMenu` does (`Menu::AppSub`, as Recent Items and Services): to the right
of the row, overlapping the menu's edge by 3 pt with its first row level with the header, or to the left when the right
has no room (a cascade that turned left keeps going left, `desktop_bar::flyout_origin`). The parent stays open with the
header highlighted (`Shell::menu_chain`, `menu_level_rects`). Resting on a header for 200 ms opens it
(`FLYOUT_OPEN_MS`); a click on it opens it at once. While the pointer heads for the open submenu across other rows of
the parent (the safe triangle from the pointer's last position to the submenu's near corners, `in_safe_triangle`) the
submenu stays, and a row there takes over only after a 300 ms rest (`FLYOUT_SAFE_MS`). Right opens the highlighted
header's submenu with its first item highlighted, Left and Esc close one level back to the header, and a click on an
item of any level reaches the app. Each submenu level is an overlay surface of its own under the window server (chrome
ids from 17, `FLYOUT_CHROME_ID`; the menu is id 1), the parent's surface left clear where the submenu's covers it, and
the pointer Leave sent when the pointer crosses from one level's surface to the next closes nothing. Submenus nest to
`MAX_APP_MENU_DEPTH`. A menu taller than the screen is held to the room below the
bar and scrolls between arrows at its top and bottom edge (resting on an arrow, the wheel, or the keyboard).
Gaps against AppKit, recorded: context menus (`ContextItem`) have no submenus yet; with a submenu open by hover, Up and
Down move in the submenu rather than the parent; a checked
item shows a check mark glyph in its title, there are no item icons, tooltips, `representedObject`, alternate
(Option) items or `NSMenuDelegate.menuNeedsUpdate` before opening; state is pushed on change instead. Menu items with
a modifier-less key equivalent are ignored by the chord path.

**The app's own Print (`AppMenuItem::print`, 2026-10-23).** The shell ends every SDK app's File menu with its own Print…
(Cmd+P), which prints a snapshot of the window. An app that prints its documents itself (Apple parity: the app owns
`NSDocument.printDocument:` / `print:` behind File > Print…) declares that item with `AppMenuItem::print(id)` (title "Print…",
key Cmd+P, the `prints` flag, bit 4 of the item flags on the wire) and handles it in `menu_action` like any item. The shell
then shows the app's item in place of its own row, at the end of File, whichever app menu declares it
(`desktop_bar::app_print_row`; `app_menu_rows` leaves it out elsewhere), so File has one Print…; `app_menu_chord` sends Cmd+P to
it. Publish it disabled when there is nothing to print; the shell's snapshot does not come back while the app declares one.
Contacts uses it (File > Print… prints the contact list; Print Address Labels… stays a separate item). Tests: proto
`the_apps_own_print_item_keeps_its_flag_over_the_wire`, shell `an_apps_own_print_replaces_the_shells_print_row`, Contacts
`group_actions_lead_the_detail_bar_and_file_has_one_print`. Not matched yet: there is no app-side print panel hook
(`printOperation(withSettings:)` handing the shell's print sheet the app's pages); the app runs its own `PrintOperation`.

### App context menus (`Ctx::context_menu`, `NSMenu.popUpContextMenu`)

A right click arrives as `Input::Context { x, y }` (window-local px). The app answers with
`ctx.context_menu(x, y, items)`; the shell draws the rows with the bar menus' drawing (`menu_spans` geometry, the menu
material, rounded corners and shadow, hover highlight, Up/Down/Return/Esc) and sends the choice back as
`Input::ContextAction(id)`. Build the rows with `ContextItem::new(id, title)`, `.key("cmd+i")` (a key equivalent in
`KeyCombo::parse` notation, shown right-aligned; the app still handles the chord itself), `.enabled(false)` (secondary
ink, not a choice) and `ContextItem::separator()` between groups; a separator at either end or doubled is dropped by the
shell. At most 32 rows, separators included (the rest are cut).

Placement (`desktop_bar::context_menu_origin`): the menu's top-left corner is the pointer. A menu that would pass the
right edge opens to the left of the pointer and one that would pass the bottom edge opens above it; the area is the
screen below the menu bar and above the Dock (`Desk::area`, the windows' work area), so a menu never covers the Dock.
A menu larger than the area is held at its top-left. The surface is exactly as tall as its rows and separators; before
this round it was sized from a fixed 36 pt per item (the rows were drawn 24 pt tall), so it was far taller than its items,
reached over the Dock and opened above the pointer (Files E2E finding F4).

Wire compatibility: `ContextItem` keeps its three fields (`id`, `label`, `enabled`). A separator is id 0 with an empty
label; a key equivalent follows the title after a tab inside `label` (`CONTEXT_KEY_SEPARATOR`). Apps that fill the struct
by hand keep working unchanged; they just show no keys and no separators. Gaps against AppKit, recorded: no submenus,
no checked-state column (a check mark goes in the title), no icons, no `NSMenuDelegate` and no alternate (Option)
items in context menus.

## Uniform type identifiers (`UniformTypeIdentifiers`, 0.4.69)

`uttype` is now the one table of file types: 58 types with identifier, parents, extensions, MIME type and
`localizedDescription` ("PNG image", "Rust Source"). API: `type_for_extension`, `type_for_mime`, `extensions`,
`description`, `preferred_extension`, `mime`, `conforms`. Lookups use a once-built FNV index.
Files derives its coarse `Kind` from conformance (`fs::kind_of`) and shows `fs::kind_description` in the info sheet and
for Sort by Kind, as Finder's Kind column does. Only PNG and JPEG count as Image there (the viewer's decoders); other
images still say "GIF image" and so on. Apps reach all of it as `bifrost_app::controls::uttype` (guarded by
`sdk/tests/kits.rs`). Benchmark: `ui_uttype_lookup_10k_names`.

## Data detectors (`NSDataDetector`, 0.4.71 in progress)

`detect` finds links, dates, street addresses and phone numbers in plain text, the `NSTextCheckingType` subset Bifrost uses. `find(text,
kinds) -> Vec<Match { kind, range }>` returns byte ranges (always char boundaries), sorted, without overlaps. Priority
when ranges collide: link, then date, then address, then phone, so "2026-10-09" is a date, not a phone number.

- Link: `http://`, `https://` and `www.`, at a word boundary. It stops at whitespace or a quote or bracket; sentence
  punctuation and unbalanced `)`/`]` are not part of it. The host needs a dot.
- Date: `yyyy-mm-dd` or `d.m.yyyy`, real calendar dates only (leap years checked, years from 1000).
- Address: a street name with a house number, English or Icelandic: "Laugavegur 12", "Hverfisgötu 4a", "Main Street 12",
  "12 Main Street". A bare street name is not an address, and a lower-case word is not a name.
- Phone: 7 to 15 digits (E.164), an optional leading `+`, and at least one separator (space, dash, brackets). A bare
  digit run is an id or a version, so it is not reported.
- Not yet: `transitInformation` (needs transit data Bifrost does not have), city and postcode after the street,
  locale-specific month names. Apple's type list is date, address, link, phoneNumber, transitInformation.

Apps reach it as `bifrost_app::controls::detect` (guarded by `sdk/tests/kits.rs`). Benchmark:
`ui_detect_scan_100kb` (8.07 ms for 100 KB of mixed text, median of 21 runs, Titan, `fast` profile). The corpus
gained addresses this round, so 2.97 ms (round 15) is not comparable. `addresses` allocates per token; it is the next
candidate to optimise.
Consumer: Notes. `TextArea::set_detect_links(true)` (opt-in; Contacts is unchanged) underlines links in the tint and
`link_at` reports the one under a point. A tap on a link queues it (`Notes::take_link`) and `main.rs` sends
`Ctx::open_url`; the caret does not move. Decision: a tap opens the link (as in Apple Notes' reading view), so editing
a link means tapping beside it. Not verified on Midget yet (no screenshot).

## App preferences, pasteboard and appearance (SDK, 0.4.72 development)

Apple parity for the three most-used system services an app reaches for. All are on `Ctx` (the SDK's one import, `bifrost_app`):

- **`UserDefaults`** (`NSUserDefaults.standard`): `ctx.defaults()` reads and `ctx.defaults_mut()` writes typed values
  (`string`, `bool`, `int`, `double`, `data`; `set_*`, `remove`, `contains`, `keys`). A missing or mistyped key reads as
  the zero value, as in Foundation. Each change is written at once to `<app data>/defaults.txt` with an atomic replace.
  The format is one `kind<TAB>key<TAB>value` line per key, with escapes. Decision: a synchronous write, because the
  store is small and an app that quits right after a change must keep it. Round trip checked with a scratch test
  (escaped keys and values, all five types, removal); no test file is kept in the repo.
- **Property lists** (`UserDefaults` with `Plist`): `ctx.defaults().plist(key)` returns `Option<Plist>` for any stored value,
  arrays and dictionaries included, and `ctx.defaults_mut().set_plist(key, plist)` stores one. Scalars still go through the typed
  getters. A key that holds `data` reads back through `data(key)` (hex in the file). Plist values are stored in `defaults.txt` in the
  same one-line format as the scalars.
- **Pasteboard** (`NSPasteboard.general` strings): `ctx.pasteboard_string()` and `ctx.set_pasteboard_string(text)`. The
  shell's protocol already carried the clipboard text; this exposes it. Decision: only plain text, because the protocol
  sends text (`SetClipboard`). Gap: pasteboard types, images, the change count, and private named pasteboards.
- **Appearance** (`NSWorkspace` accessibility display options): `ctx.appearance()` returns text scale, display zoom,
  touch hold time, bold text, Reduce Motion, Increase Contrast, Reduce Transparency and button shapes. Read it when
  `configure` runs; the shell sends changes as `ToApp::Appearance`.

Done since:
the Keychain, the share sheet, notification actions and typed drag and drop are done (see their sections). The app loop's timers are the
`tick`/`next_wake` pair on `App`, which already matches `Timer` and `DispatchQueue.asyncAfter`.

## Keychain (`SecItem*`, `kSecClassGenericPassword`)

Secrets an app must not keep in `defaults.txt` or anywhere in its own (app-readable) data directory. `bifrost_app::keychain`:
`add`, `get`, `update`, `delete`, each `(ctx, service, account[, secret])`.

- **Query API** (`bifrost_app::keychain_query`): `ItemClass` (generic and internet password), `Attributes` (service, account,
  server, protocol, port, label, comment, creation and modification date, access group), `Query` with `MatchLimit::{One, All}`,
  `return_data` / `return_attributes`, `Status` (OSStatus values), and `add_item`, `copy_matching`, `update_items`,
  `delete_items`. Attributes are stored in the value as a `SIQ1` record, so the shell store is unchanged and legacy items
  still read as secret-only generic passwords. Limitation: `MatchLimit::All` needs a shell list operation; until then
  `select` filters records the app already has.
- **Asynchronous**, like the document panels: each call returns a request id and the answer arrives in
  `App::keychain(ctx, keychain::Reply { request, result: Result<Vec<u8>, keychain::Error> })`. Decision: the app loop is
  single-threaded and the shell owns the store, so a blocking call would stall frames; Apple's calls are synchronous.
- **Errors mirror Apple's**: `DuplicateItem` (errSecDuplicateItem -25299, `add` on an existing item), `ItemNotFound`
  (errSecItemNotFound -25300, `get`/`update`/`delete` on a missing one), `Param` (errSecParam -50), `NotPermitted`,
  `Io`. `Error::code()` gives the `OSStatus`.
- **Permission**: the manifest needs `permissions = ["keychain"]` (`app_policy.rs`, `tools/aap.py`); without it every
  call answers `NotPermitted`. It binds nothing else.
- **Identity**: the shell stores items under the app id it launched (`AppHost::id`); the wire message carries no id, so an
  app cannot read or write another app's items.
- **Broker, not a daemon**: `ToShell::Keychain` / `ToApp::KeychainReply` on the existing app channel (`proto/src/lib.rs`),
  handled in `src/shell/apphost.rs`, store in `src/keychain.rs`. The shell runs as root.
- **At rest**: `<root>/keychain/<app id>.kc` (directory 0700, files 0600, atomic replace). The file is
  `BKC1 | nonce | ChaCha20(items) | HMAC-SHA256` (encrypt then MAC, per-write random nonce); per-app subkeys come from
  a random 32-byte master key in `<root>/keychain/.master` (0600), created on first use. Limits: names 255 bytes, secrets
  8 KiB, 256 items per app.
- **Limitation, explicit**: the master key sits on the same disk as the items. This keeps secrets from apps, other users
  and backups of app data; it does not protect against root or an attacker holding the whole disk. There is no
  TPM or login-password key source yet; `Store::with_master` is where one plugs in.
- Tests (written, not yet run): `src/keychain.rs` (HMAC vector, round trip, duplicate/not-found, per-app isolation,
  tamper detection, modes), `proto` `keychain_messages_round_trip`, `sdk/src/keychain.rs`.

Access groups (`kSecAttrAccessGroup`) are wired through the `keychain-groups` service (`docs/parity/wire-svc-keychain.md`).
An app lists `keychain_groups = ["is.example.group"]` in its manifest, with the `keychain` permission, and then
`bifrost_app::keychain_groups` (`add`, `get`, `update`, `delete`, each naming the group) reads and writes items that
every app in the group shares. Members get a `changed` event after another member writes. A group is one file under
`keychain/groups/`, and a call never searches across groups, unlike Apple's default search.

Parity gaps, not matched yet: no access-group search when the group is omitted (every call names one), no
biometric or user-presence access control (`SecAccessControl`, Touch ID), no `kSecAttrAccessible` classes (items are
available whenever the shell runs), only `kSecClassGenericPassword` (no internet passwords, keys or certificates), no
attributes beyond service and account (labels, comments, dates), no `kSecMatchLimitAll` listing, no iCloud sync, and
`update` replaces the value only (no attribute update).

## Accounts (`ACAccountStore`, `ACAccount`, `ACAccountType`, `ACAccountCredential`; `springboard/sdk/src/accounts.rs`)

Pure model, host-tested. Parity row: `docs/parity-matrix/data/accounts.json` (status stays partial until a Midget check).

- **Store**: `AccountStore::open(path)` keeps the store in a file; `AccountStore::shared_path()` is `$XDG_DATA_HOME/bifrost/accounts.ini`
  (or `~/.local/share/bifrost/accounts.ini`). A missing file is an empty store; a malformed file is an `InvalidData` error.
  `AccountStore::new()` lives only in the process (no file).
- **Shared list**: every change (`save`, `remove`, `set_access`, `register_type`, a renewal) first reads the file's current contents,
  applies the change, then writes the file atomically (`bifrost_core::fsutil::write_atomic`). A refused change writes nothing.
  Another handle's changes appear after `reload()`, which returns whether anything changed.
- **Change notification** (`ACAccountStoreDidChange`): `observe_did_change(FnMut)` returns a token; `unobserve(token)` removes it.
  Observers run once per change that alters the state (a refused or no-op change runs none). `change_count()` counts them.
- **Format**: `[store]`, `[type]` and `[account]` sections with `key = value` lines; values escape `\\`, newline and carriage return.
  Unknown sections and keys are skipped. Identifiers `account-N` are never reused, even after a removal and a reopen.
- **Credentials are not written to the file.** A handle keeps the credential it holds in memory across its own saves and reloads.
  After a reopen the app sets the credential again from its keychain item (`Credential::encode` / `decode` give the bytes).
- **Access grants persist**: `accessGranted` is kept in the file, so a grant survives a restart. There is still no user prompt
  that grants a type, so `set_access` is called by the app's own grant path until one exists.
- **Gaps**: no app reads the shared store yet, and the Settings Internet Accounts pane keeps its own list in
  `internet-accounts.ini` (a different schema: mail, CalDAV and CardDAV kinds), so Mail, Calendar and Contacts still see separate
  lists. Merging the two lists needs a decision on the schema, so it is not done here. Not yet tested on the VM or Midget.

## Website logins and secure notes (`bifrost_app::passwords`, credential exchange)

Website logins, secure notes and certificates that an app saves once and reads later. Every item is one entry of the shared
passwords group (`passwords::GROUP`, `is.bifrost.passwords`), which the Settings Passwords page and the Passwords app both list.
The item model lives in the shell's `credentials` module, shared with the SDK.

- **Calls**: `passwords::save(ctx, site, account, password)`, `update`, `fill(ctx, site, account)`, `remove`, `list(ctx)`,
  `save_note(ctx, title, body)` and `save_certificate(ctx, label, pem)`. Each returns a request id. The answer arrives in
  `App::service_reply`. `decode_password(result)` gives the password for `fill`, and `decode_list(result)` gives the rows for `list`.
- **Identity**: a login's service is the site and its account is the user name, so a form filler reads back what a browser saved.
  Notes and certificates are kept under a kind prefix. `kind_of` and `name_of` tell them apart in a listing.
- **Duplicates**: `save` answers `Error::DuplicateItem` when the login exists. Call `update` instead.
- **Permission**: the manifest lists `is.bifrost.passwords` under `keychain_groups` and asks for the `keychain` permission. Without
  both, every call answers `NotPermitted`.
- **Credential exchange**: `bifrost_app::credential_exchange` re-exports the import and export model from `bifrost_ui`:
  `export`, `import`, `PasswordRecord`, `ImportReport`, `ExchangeError`, `escape`, `unescape` and `HEADER`. The format is defined once,
  so the shell and apps read the same file.

Gaps: the Passwords app shows a password without the passcode that Settings asks for (the shell has no app-side passcode prompt).
Creating a secure note from an app needs a text editor and is still missing. Midget check: save, fill and remove one login from a test app.

## notifyutil (command line for the notify bus, OS-layer row `notifyutil`)

Apple's `notifyutil` posts, watches and sets the state of named notifications. BifrOSt has no Darwin notify daemon: the
bus is the shell's `notify_bus` service (`springboard/src/shell/services/notify_bus.rs`, 0.5.41), which the command line
reaches over the control socket as `notify <verb> <args>`. Verbs: `post NAME`, `set NAME VALUE`, `get NAME`,
`register NAME` (a poll token owned by `control`, which the shell does not prune), `check TOKEN`, `cancel TOKEN`.
The client is `springboard/cmd/src/notifyutil.rs` (`bifrost_cmd::notifyutil`) and `springboard/cmd/src/bin/notifyutil.rs`:
`-p name`, `-w [count] name`, `-1 name`, `-g name`, `-s name value`, `-v`, `-z msec`.

- **Gap**: names are BifrOSt names, not the Darwin system names (only the shell's `system.*` names are posted by the shell).
- **Gap**: `-w` and `-1` poll the token every 50 ms; a post is seen up to that late. The bus pushes only to apps.
- **Gap**: names cannot hold white space; state is a whole number up to 2^53 (the JSON number limit on the bus).
- **Gap**: a killed `-w` leaves its token behind (cap 64 for the control owner).
- Row status: `partial`. Not built or run on Midget yet (uncompiled in this round); host tests are in
  `springboard/src/bare/control.rs` (`notify_requests_keep_their_words`), `springboard/src/shell/services/notify_bus.rs`
  (`a_command_line_poll_token_survives_prune_and_sees_one_post`, `set_and_get_keep_a_whole_number_state_per_name`) and
  `springboard/cmd/src/notifyutil.rs`.

## User notifications (`UNUserNotificationCenter`, roadmap item 5a)

`bifrost_app::usernotif` (SDK) over `ToShell::Notify*` (tags 0x140 to 0x143) and `ToApp::Notification*` (tags 50 to 52); types in
`proto/src/rich.rs`, shell state in `src/shell/usernotif.rs`.

- **Categories**: `set_categories(ctx, &[NotifCategory])` (`setNotificationCategories`). An action is a button, a destructive
  button (`.destructive`), a foreground action (`.foreground`) or a text-reply input (`UNTextInputNotificationAction`, with
  `button_title` and `placeholder`). Up to 16 categories of 10 actions.
- **Requests**: `add(ctx, Request::new(id, title).subtitle().body().thread().category().after_ms().badge())` returns a request id;
  `App::notification` gets `Event::Status` (`Ok`, `NotPermitted`, `Param` for an empty id or content or an unknown category, `Full`
  past 64 pending). The same identifier replaces the earlier pending or delivered notification. Needs the `notifications` permission.
- **Delivery**: through the normal shell notification UI (banner, lock screen, Notification Center, Focus rules). The subtitle is
  shown above the body. `after_ms` is a `UNTimeIntervalNotificationTrigger`; due requests are delivered from the shell tick.
- **Remove and query**: `remove_delivered`, `remove_pending` (no ids = all), `delivered`, `pending` (answered as `Event::List`).
  Delivered requests the user cleared from Notification Center drop out of the list.
- **Responses**: `Event::Response { id, action, text, thread, category }` (`didReceive`). Opening the app from its newest notification
  answers `NOTIF_DEFAULT_ACTION`; the shell answers an action button through `Shell::usernotif_respond`, which refuses an action the
  category does not have and only passes `text` to a text-input action. If the app is not running the shell starts it and delivers
  the response as soon as it is up (up to 32 are held).
- **Dock badge** (`NSDockTile.badgeLabel`): `Request::badge(n)` sets it (0 clears); `Ctx::set_badge` still sets any short label.
- **Content** (`UNMutableNotificationContent`): `Request::sound(&Sound)` (default, named, critical, critical with volume),
  `Request::user_info(&[(key, value)])` (text values, at most 16 entries of 512 characters, otherwise an error),
  `Request::interruption_level(InterruptionLevel)` (passive < active < time sensitive < critical) and
  `Request::relevance_score(f32)` (clamped to 0...1). The notification-schedule service takes the same as `sound`, `userInfo`,
  `interruptionLevel` and `relevanceScore`. Delivery does not yet play the sound or return `userInfo` in `Event::Response`.
- **Triggers** (`bifrost_app::notification_schedule`, the `notification-schedule` service): `Schedule::interval(id, title, seconds,
  repeats)` (`UNTimeIntervalNotificationTrigger(timeInterval:repeats:)`) and `Schedule::calendar(id, title, hour, minute,
  weekdays, repeats)` (`UNCalendarNotificationTrigger(dateMatching:repeats:)`, weekdays 1 = Sunday .. 7 = Saturday, empty = every
  day). `Schedule::checked_interval` and `Schedule::checked_calendar` return `Result<Schedule, TriggerError>`: a repeating interval
  under `MIN_REPEAT_SECS` (60), a zero interval, an hour above 23, a minute above 59 or a weekday outside 1..=7 is refused.
  `Schedule::validate()` checks a built request. `Trigger::next_trigger_date(now_unix, offset_secs)` is
  `UNCalendarNotificationTrigger.nextTriggerDate()` and `UNTimeIntervalNotificationTrigger.nextTriggerDate()`: an interval
  answers `now + seconds`; a calendar answers the next local time strictly after `now` on a listed weekday (a time exactly
  now has passed). `offset_secs` is the zone's offset east of UTC at that moment; `None` for an invalid trigger.
  Gap: Apple's `nextTriggerDate()` for a non-repeating calendar trigger with a fixed date is `nil` once it has passed; these
  triggers carry no date, so a one-shot answers its next occurrence, as the shell delivers it.

Gaps (not matched yet): action buttons, the destructive colour and the reply field are not drawn on banners or in Notification Center
(the model and the response path are done; only the drawing and hit testing remain), so only the default action is reachable by tap;
`NOTIF_DISMISS_ACTION` is defined but never sent (`customDismissAction`); a tap answers for the app's newest notification, not the one
tapped (the banner and center rows do not carry the request id); delivered requests and categories are not persisted across a shell
restart; no `UNNotificationSound`, attachments, `userInfo`, interruption levels, location triggers, the `usernotif` (`after_ms`) path's repeating triggers (use `notification_schedule`),
(`willPresent` and `requestAuthorization` are wired now; see "Notifications: authorization and presentation (update)" under the SDK modules of 2026-10-17.)

### Notification stacks, persistence and Show Previews (parity round)

- **Stacks by thread**: a request's `thread` is kept on the notification (`thread=` in `notifications.ini`). The store
  groups by (app, thread), newest stack first (`springboard/src/shell/notify/store.rs`, `Store::groups`). Each stack has its own
  Clear (`Store::clear_thread`, `Shell::clear_group`), the desk column's close control and a whole-stack swipe on the lock
  screen; Mute, Show Less and the expand tap still act on the app. Tests: `store.rs` (`notifications_stack_per_app_and_thread`,
  `clear_thread_removes_only_that_stack`), `notify/mod.rs` (`clearing_a_stack_removes_only_its_thread_of_the_app`).
  Gap: the lock-screen expansion is per app, so two stacks of one app expand together. Clear All and the 50-entry limit are unchanged.
- **Pending requests across a restart**: `notification-schedule` saves interval and calendar requests to
  `pending-notifications.ini` beside `notifications.ini` (`services/notification_schedule.rs`, `encode`/`decode`, `restore`,
  `persist`). Each saved trigger is a Unix time in milliseconds; on the first tick after the data directory is set, a
  request is put back at the same moment on the new shell clock, so one that passed while the shell was off fires at once.
  Tests use two shells with a fake wall clock: `pending_requests_survive_a_restart_and_keep_their_trigger_times`,
  `a_request_whose_time_passed_while_the_shell_was_off_fires_on_the_first_tick`, `a_cleared_request_is_not_saved_again`.
  Gap: sound, `userInfo` and interruption level are not saved, and delivered requests still are not persisted.
- **Show Previews** (`Settings > Notifications > <app> > Show Previews`): Always (the default; the passcode still hides text
  on the lock screen), When Unlocked (hidden on the lock screen, shown once unlocked) and Never (hidden everywhere). A
  banner's text follows the same rule, since banners only show when unlocked (`Shell::previews_hidden`, `start_banner`).
  Test: `show_previews_decides_what_banners_and_lists_reveal`, `previews_hidden_per_app_hide_only_that_app_text` (`list.rs`).
- **Calendar triggers** still carry only hour, minute and weekday over the wire.
- **Not verified on Midget**: the stacks, the restart, the Show Previews settings and the Focus modes are host-tested only.

### Focus modes (Focus round)

- **Modes** (`springboard/src/shell/notify/modes.rs`): Do Not Disturb, Work, Personal and Sleep, plus custom modes (up to
  12, named with a unique slug id). Each mode has allowed apps (app keys), allowed people (matched by exact name), a
  time-sensitive allowance and an optional schedule. A schedule is a set of weekdays and a window in local minutes; a
  window that crosses midnight belongs to the weekday it starts on (Friday 22:00 to 07:00 covers Friday night and early
  Saturday). One mode is on by hand (`Modes::manual`, a manual choice wins) or by its schedule.
- **Delivery**: `modes::decide` returns `Show` or `Quiet`. With no mode on everything shows. A notification shows when its
  app is allowed, or its sender is allowed, or it is time-sensitive and the mode allows that. `Notify::delivers` uses it;
  a `Quiet` notification is stored and badged, with no banner or wake (Notification Center only).
- **Do Not Disturb** keeps its allowed apps and schedule in `notifications.ini` (`Store::focus_allowed`, `Store::schedule`),
  so existing data keeps working. `Notify::as_stored` reads them into the mode. Other modes live in `focus_modes.ini`,
  saved next to the notification store when the shell loads its data directory (`Notify::load_modes`, `save_modes`).
- **API**: `Shell::set_focus_mode(Option<&str>)` (saves; the Settings Do Not Disturb switch follows only Do Not Disturb),
  `Shell::set_dnd` (a wrapper for Do Not Disturb), `Notify::set_mode`, `Notify::active_mode`, `Notify::dnd` (any mode is on).
  Turning a mode off inside its schedule's window holds until the window ends, as Do Not Disturb always did.
- **Focus filters**: `services/focus_filter.rs` names the Focus by the active mode, so apps read and get the `changed`
  event for the values they stored under that mode's name. Apps declare filter fields as `[[focus_filter]]` tables in
  `manifest.toml` (`id`, `title`, `params = ["folder", "room"]`); `parse_declarations` checks them and drops bad tables.
- **Shortcuts**: `When Focus Turns On` and `When Focus Turns Off` (`springboard/src/shortcuts.rs`, `Automation::FocusOn`,
  `Automation::FocusOff`, `Event::FocusChanged`). Nothing calls `shortcuts::due` in the shell yet, so they do not run.
- **Control Centre**: the Focus tile still toggles Do Not Disturb; its subtitle names the mode on.
- **Tests**: `notify/modes.rs` (schedule across midnight and weekdays, weekdays from the Unix epoch, manual over schedule,
  delivery decisions, custom modes, the file format, junk, persistence, and the `Notify` clock tests), `shell/focus.rs`
  (`a_mode_chosen_by_hand_survives_a_restart_and_the_settings_switch_follows_do_not_disturb_only`),
  `services/focus_filter.rs` (`a_work_focus_hands_the_apps_its_work_values_and_the_change_is_pushed`,
  `filter_declarations_come_from_the_manifest_and_bad_tables_are_dropped`), `tests/parity_app-shortcuts.rs` (Focus
  automations). Not run: the lead runs them.
- **Not built**: the menu bar Focus glyph; the Control Centre list of modes; a Settings page for the modes, their people
  and the per-mode filter values; the sender on a notification (so allowed people never match on device); the
  time-sensitive level from the app protocol; the Shortcuts dispatcher that raises `FocusChanged`.
- **Not verified on Midget**: all of the above.

### User notifications model (sprint 9)

`bifrost_app::usernotif_model` (re-exported from `usernotif`) is the pure, unit-tested model; the shell wire-up is still to do.

- **Authorization**: `AuthorizationStore::request(app, AuthorizationOptions, Decision)` (`requestAuthorization(options:)`) moves
  `AuthorizationStatus` from `NotDetermined` to `Authorized`, `Denied` or `Provisional` (no prompt) and never asks twice;
  `settings(app)` is the `UNNotificationSettings` snapshot (`Setting` per alert, sound, badge, critical alert). `to_text`/`from_text` persist it.
- **Triggers**: `CalendarTrigger { DateComponents, repeats }::next_trigger_date(after, offset)` matches year, month, day, weekday
  (1 = Sunday), hour, minute and second (omitted = any, except second = 0), follows DST (a spring-gap time fires after the jump, a
  repeated autumn hour the first time). `IntervalTrigger::new(seconds, repeats)` refuses 0 and a repeat under 60 s; the
  `notification-schedule` service enforces the same minimum.
- **willPresent**: `foreground_presentation(delegate options, settings)` with `PresentationOptions` (banner, list, sound, badge); no delegate shows nothing.
- **Sound and attachments**: `Sound::{Default, DefaultCritical, DefaultCriticalVolume, Named}` (critical needs the critical-alert grant);
  `Attachment::new(id, file name, size)` accepts image, audio and video types within 10, 5 and 50 MB.

Gaps: none of this is on the wire yet (`NotifRequest` has no sound, attachments or calendar fields); the delegate round trip and the
authorization prompt are wired since 2026-10-17. Remote push and service extensions are not built. Midget check pending.

## Share sheet (`NSSharingServicePicker`, roadmap item 5b)

`bifrost_app::share` (SDK) over `ToShell::Share` (0x144) and `ToApp::ShareDone` (53); shell side `src/shell/sharing.rs`.

- **Asking**: `share::show_picker(ctx, &[Payload::Text | Url | File { grant, name }], anchor)` where `anchor` is the rect of the
  button in window points (`NSSharingServicePicker.show(relativeTo:)`, preferred edge below). At most 8 items. A file is named by
  the document grant the app already holds (`open_document` and friends), never by a path.
- **Targets**: the shell shows a menu under the anchor with every installed non-system app other than the sender whose manifest
  accepts *every* item, sorted by name, then **Copy**. Acceptance is `uttype::conforms(item type, handled type)` over `handles`
  (`public.png:owner`), a web URL by `schemes`, a `file://` URL by its extension's type. Text is `public.utf8-plain-text`, which
  now exists in `uttype` with `public.url` and `public.file-url`.
- **Receiving**: a URL arrives as `ToApp::Open { url }` (`Input::Open`). Text arrives as an `Open` with a `data:text/plain` URL
  that `share::text_from_url` decodes. A file arrives as a `DocGranted` (a `DocBatch` first for several) whose `request` has
  `share::INBOUND` (bit 31) set, with a fresh **read-only** grant the broker made from the sender's grant
  (`Broker::transfer`), so the receiver never learns the sender's path and the sender's write access is not passed on.
  The target window comes forward (`desk_open`); it is started if it was not running.
- **Result**: `App::share_done(request, Some(app id | "copy"))`, or `None` when the menu was dismissed. Copy puts text and URLs
  on the clipboard, one per line, and a file as its name.

- **Service objects** (`NativeSharingService`, roadmap parity "Social"): `share::SharingService::sharing_services_for_items(&items,
  &installed)` returns the services that take the items, in the menu's order (apps by name, Compose Post, Copy). `installed` is a
  `share::Candidate` list the caller builds from the manifests it may offer (its own app left out). `SharingService::named(COPY |
  COMPOSE)` builds the built-in services. Each service has `name()` (app id, `COPY` or `COMPOSE`) and `title()`, `can_perform(&items)`
  (every item must match the service's `handles`/`schemes`; Copy and Compose take every kind; nothing to share is never performable)
  and `perform(ctx, &items)`, which for Copy puts `copy_text` on the clipboard and returns false for the rest.
- **Picker and delegates** (`NativeSharingServicePicker` with its delegate): `SharingServicePicker::new(&items, &installed)` offers
  `services()`; `show(ctx, anchor)` shows the menu and returns the request. The answer goes to `share_done(request, target, &mut picker,
  &mut sharer)`, which returns false for a request that is not the menu on screen. A chosen service calls
  `SharingServicePickerDelegate::did_choose(Some(&service))` then `SharingServiceDelegate::did_share_items(&items)`; a dismissed menu
  calls `did_choose(None)`; a target the picker never offered calls `did_choose(None)` and
  `did_fail_to_share_items(&items, ShareError::NotOffered(id))`. The shell cannot tell a failed delivery from a dismissal (both
  answer an empty target), so a failed delivery reads as a dismissal.

Gaps (not matched yet): only apps declaring `handles`/`schemes` appear (the system services such as Mail, Messages, AirDrop, Add to
Reading List and Notes are not offered); no `image` on a service; `perform` for an app or Compose Post does not hand the items over
directly (the shell has no message for one named target), so the menu does it; no
`sharingServicePicker(_:sharingServicesForItems:proposedSharingServices:)` hook to add or remove services; no `NSItemProvider` / rich
items (images as data, attributed strings, several representations), no preferred edge, no share-sheet extensions installed by apps,
the menu is a plain context menu (no icons, no "More..."), and Copy of a file copies only its name.

## Typed drag and drop (`NSDraggingSource` / `NSDraggingDestination`, roadmap item 5c)

`bifrost_app::dnd` (SDK) over `ToShell::Drag*` (0x145 to 0x149) and `ToApp::DragDest` / `DragEnded` (54, 55); shell side
`src/shell/dnd.rs` (the pure state machine `Session` plus `Shell::dnd_*`).

- **Destination**: `dnd::register_types(ctx, &["public.image", "public.text"])` (`registerForDraggedTypes`). A window takes a
  session when some type of some item conforms to a registered type (`uttype::conforms`, so `public.image` takes PNG and JPEG
  and `public.data` takes everything). Windows that did not register never hear about a drag. `App::drag` gets
  `Event::Dest { session, phase, x, y, allowed, items }` with `Entered`, `Updated`, `Exited` and `Perform`; `x`, `y` are
  window pixels, the space of `Touch`. Answer `Entered` and `Updated` with `dnd::reply(op)`, one of `dragop::COPY | LINK | MOVE`
  (0 refuses); a reply the source did not allow counts as 0. The operation resets when the pointer enters another window.
- **Source**: `dnd::begin(ctx, allowed, items, preview)` (`beginDraggingSession`) with `DragItem { types, payload }`
  (`Payload::Text | Url | File { grant, name }`; `DragItem::new` offers an item as its natural type) and a preview from
  `dnd::preview_from_rgba` (nearest-sampled to at most 32 x 32). The session id has bit 31 set. While it runs the shell owns the pointer
  (the source gets no more touches) and draws the preview under it. `Event::Ended { op }` closes it (`op` 0: nobody took the drop;
  also the immediate answer when the shell refuses: another session runs, the app is not in front, no items).
- **Perform**: on release over a window that replied with an operation the shell first hands the files over, each as a fresh
  **read-only** grant made from the source's grant (`Broker::transfer`, a `DocBatch` and `DocGranted` with `request == session`),
  then sends `Perform` with the items; file items carry the new grant ids and a file that could not be handed over is dropped.
  The destination answers `dnd::performed(accepted)` and the source gets `Ended` with the agreed operation, or 0 when refused.
  Text and URLs travel inline (512 characters at most).
- **Same window** (W4-24): a release over the source's own window always ends in `Perform` there, whatever its last reply said
  (the reply to the last `Updated` can still be in flight when the button goes up); the app decides and answers `performed`.
  Files moves the dragged items into the folder row or icon under the drop (the target is tinted while hovered, list and icon
  view) and copies with Option.
- **Modifier keys** (`draggingSourceOperationMask`): the shell narrows the source's mask by the keys held at each pointer step
  (`dnd::modifier_mask`: Option copy, Command move, both link; a wanted operation the source does not allow leaves 0) and
  sends the narrowed mask as `allowed`; a reply is negotiated against it when the drop happens.
- **Cleanup**: an app that exits loses its registration and abandons a session it takes part in. Wire sizes are capped (8 items,
  4 types of 128 bytes per item) so every packet fits `MAX_PACKET`; a full packet is asserted in the proto tests.

Gaps (not matched yet): destinations are app windows only; the desktop background, the Dock (drop a file on an icon to open it)
and Files' own desktop icons are not destinations, and shell chrome (Spotlight, menus) cannot be dragged from. No Escape to
cancel, modifier changes while the pointer stands still reach the destination only with the next move, no auto-scroll, no
slide-back animation, one drag image (no per-item image components, no `draggingFormation`, no image swap on `Entered`).
Promised data lives in-process (see "Promised pasteboard data"); the drag wire still has no promised-file payload kind. Drags between an SDK window and a native Wayland
or X11 window (the `text/uri-list` portal) are not bridged. The preview is drawn only where the shell composites on the CPU
path; on the retained GPU window path it may not show until the next full repaint.

### Promised pasteboard data (AppKit)

`bifrost_app::pasteboard_promise` mirrors `NSPasteboardItemDataProvider`: `PasteboardItem::set_data_provider(provider, types)`
lists promised types without bytes, the first `data_for_type` calls `provide_data` once and keeps the answer, and a provider
that returns `None` gives `PromiseError::NoData`. `NativePasteboard::declare_types(ctx, types, owner)` takes a
`PasteboardTypeOwner`; clearing the board (or an item dropping its last use of a provider) fires
`finished_with_data_provider` / `changed_owner` once. `FilePromiseProvider` +
`FilePromiseProviderDelegate` (`file_name_for_type`, `write_promise_to`) write a promised file into a destination dir.
Gap: promises are not carried over the wire to other apps yet. (This replaces the separate `PromiseBoard`: a board is
a `NativePasteboard`, so promises and named boards share one implementation.)

## Open and save panels (`NSOpenPanel` / `NSSavePanel`, 0.4.81)

An app with the `documents` permission asks the shell for a file; the shell shows the panel and hands back an open file,
never a path (design and security model: `docs/file-portal-design.md`).

| Apple | Bifrost |
|---|---|
| `NSOpenPanel.begin(completionHandler:)` | `ctx.open_document(mode, title, filters) -> request` (one file) or `ctx.open_panel(&OpenOptions)`; the answer is `App::document(ctx, DocumentEvent::Opened(doc))` |
| `allowedContentTypes` | `filters`: UTIs (`"public.text"`, conformance through `uttype`) or extensions; other files are dimmed |
| `NSSavePanel` + write | `ctx.save_document(name, filters)` -> `SaveTo(doc)`: write `doc.file`, then `ctx.commit_document(request)` -> `Saved(doc)` |
| `.cancel` response | `Failed { code: Cancelled }`; refusals are `Denied`, `NotFound`, `Stale`, `Unsupported` |
| security-scoped bookmark | `doc.grant` with `ctx.reopen_document(grant)` / `release_document(grant)`; `Stale` when the file was replaced |

Panel options (`NSOpenPanel`, wire `ToShell::DocOpenEx` with `proto::docopt` bits; `DocOpen` and every existing caller are unchanged):

| Apple | Bifrost |
|---|---|
| `allowsMultipleSelection` | `OpenOptions::multiple`; the answer is `DocumentEvent::OpenedMany { request, docs }` (`NSOpenPanel.urls`), always, even for one file. On the wire: `ToApp::DocBatch { count }` then `count` `DocGranted` (at most 256) |
| `canChooseFiles` / `canChooseDirectories` | `OpenOptions::files` / `directories`; a chosen folder is granted as its read-only directory descriptor and `reopen_document` reopens it. Files are dimmed in a folders-only panel; with no selection a folders panel chooses the folder shown |
| `canCreateDirectories` | `OpenOptions::create_directories` (a save panel always has it): the New Folder button opens a name field (`untitled folder`), Return makes the folder as the account and selects it, Esc or Cancel closes the field |
| `allowedContentTypes` | `OpenOptions::filters`, matched by `uttype::conforms` or extension |
| sidebar | Recents (the app's recent grants, newest first, gone files left out), Home, and Desktop, Documents, Downloads when they exist; the active place has a 9 pt plate |
| last folder used | the shell remembers each app's last folder; the next panel starts there, else Documents |

`ctx.open_panel(&OpenOptions { multiple: true, directories: true, ..Default::default() })` shows it. Code: `src/shell/docpanel.rs`
(`Panel`, `Options`, `Broker::finish_many`), `src/bare/docbroker.rs` (`open_dir`, `open_any`, `make_dir`), `sdk/src/runtime.rs`.

The panel follows the macOS panels: it lists alphabetically, Return or a double click is the default button (a double click
opens a folder), Esc cancels, Left goes up, a click on a file in a save panel copies its name. Not matched yet:
Shift/Cmd-click ranges (no modifier keys reach the panel, so in a multiple-selection panel a click toggles a row), tags and the
tags sidebar, iCloud Drive, mounted volumes and other users' folders (the panel is confined to the account's home, so the
sidebar has no Volumes section), `allowsOtherFileTypes`, the file-type popup (`accessoryView`), previews, and a
column or icon view. First user: Notes 1.6.0 (Export, Import with Cmd+O).

## Documents (`NSDocument` / `NSDocumentController`)

`bifrost_app::document` (code `springboard/sdk/src/document.rs`) is the document layer on top of the open and save panels
and the docbroker grants. The app implements `Document` (`read(bytes, uttype)`, `write(uttype) -> bytes`, `untitled()`,
`readable_types()`, `default_type()`) and owns a `DocumentController<D>`; the controller talks to the shell through
`DocumentHost` (implemented for `Ctx`, so it is testable without a shell).

| Apple | Bifrost |
|---|---|
| `NSDocumentController` | `DocumentController`: `new_document`, `open_panel` (several files), `open_recent`, `recents`, `clear_recents`, `ids`, `get`, `get_mut`, `active`, `has_unsaved` |
| untitled names | `display_name`: "Untitled", "Untitled 2" (the smallest free number) |
| `isDocumentEdited`, `updateChangeCount` | `is_edited`; `note_change(id, undo.change_count())` (new `bifrost_ui::undo::ChangeCount`: equal when the content is equal, so undoing back to the saved state clears it) or `mark_edited(id)` |
| window dirty dot, "Edited" title | new wire `ToShell::DocumentState { title, edited, represented }` (tag 0x180, `Ctx::set_document_state`, sent by `sync`/`tick`/`handle` when it changes). The desk draws a dark dot in the close light and "- Edited" after the title; `AppHost::doc_title/doc_edited/doc_file` hold it |
| `save(_:)` | `save`: in place for a saved document, the save panel for an untitled one |
| `saveAs`, `moveTo`, `rename` | `save_as`, `move_to`, `rename`, through the save panel; the document then lives at the chosen file (old grant released, `Notice::Saved(id, op)`) |
| `duplicate` | `duplicate`: a new edited untitled copy (write, then read) |
| `revert(toContentsOf:)` | `revert`: reopens the grant and replaces the model; `Notice::Reverted` (reset your undo manager) |
| `autosavesInPlace` | `tick(host, t)` after `autosave_delay` (5 s) from the first unsaved change, and `suspend`. Saved documents save in place through new `ToShell::DocSaveGrant` (tag 0x181, `Ctx::save_document_in_place`): the shell makes a temporary file next to the original, the app writes it, `commit_document` renames it over the file (crash-safe) and the grant keeps its id. A failing autosave is not retried until the next change |
| autosave of Untitled / `restoreWindow` | untitled documents are written to `<data_dir>/autosave/untitled-N.autosave` (temp + rename); `restore` reopens them at launch as edited documents with their old number |
| `canClose(withDelegate:)` | `close`: a clean document closes; an edited one raises `alert()`, a `nav::Alert` with Save (default), Cancel, Don't Save (macOS order via `button_order`); the app shows it and calls `answer_alert(choice)` with `alert::SAVE`, `CANCEL` or `DONT_SAVE`. Save then closes after the commit |
| File menu | `file_menu() -> AppMenu` (New, Open..., Open Recent with Clear Menu, Close, Save, Save As..., Duplicate, Rename..., Move To..., Revert To Saved, enabled for the active document) and `menu_action(host, id)`; ids in `document::item`, `is_menu_id` |
| recent documents | the grants the panel already keeps (the panel's Recents place lists them); the controller keeps its own ordered list (10, newest first) in the app defaults key `document.recents` and releases grants that leave it. A recent whose file is gone or replaced is dropped |

Wiring: `App::document` calls `controller.handle(ctx, ev)` (`None` = not the controller's request), `main_menu` returns
`controller.file_menu()`, `menu_action` forwards ids for which `is_menu_id` is true, `tick`/`next_wake`/`suspend` forward, and after
each edit the app calls `note_change`. `handle` answers with `Notice` values (`Created`, `Opened`, `Saved`, `Reverted`, `Closed`, `Failed`).

Gaps (not matched yet): no versions browser (`Revert To > Browse All Versions`, no version history at all), no iCloud documents,
no `NSFileCoordinator` or file-presenter watching (an external change to an open file is noticed only as `Stale` on the next
save), document packages only in part (see "Folder packages" below), no multiple windows per document, no `NSDocument` subclass hooks for printing or sharing. Rename
and Move To copy to the new name and leave the old file in place, because the broker has no rename or delete request yet;
Open requests write access, so a read-only file does not open. Untitled documents are autosaved as private copies, not into a
Documents folder as autosave-in-place does on macOS. First user: none yet.

### Recents and a command-line file without the controller (Console 1.9.0)

An app that keeps its documents in its own model (Console shows logs under Opened Files) can still have Open Recent across
launches. `document::format_recents` and `document::parse_recents` are public (the text of the `document.recents` store: one
`grant<TAB>name` line each, the grant in hex; at most `MAX_RECENTS`), `Ctx::reopen_document(grant)` opens a remembered file
again (`Failed { code: Stale | NotFound | Revoked }` when it changed or went away), and `Ctx::release_document` gives a grant
back. Console keeps its own list in the key `recentLogs` (`console-desktop/src/recents.rs`: `noted`, `without`): a log opened
with the panel, handed over by the shell or saved by Capture Archive is noted newest first, one that falls off the end or leaves
through Clear Menu or Move to Trash is released unless an open log still holds the grant, and a reopen that fails drops the
entry with a status note. Open Recent lists the logs open now, then the remembered ones (`Console::recent_items`, `Recent`).

A file named on the command line (`std::env::args`, as the Files, Browser and Video Player apps read theirs) opens in the same
way: a path inside the home folder is asked for with `Ctx::grant_path` (answered `Opened` or `Failed`), so Reveal in Files
works on it, and a failed or impossible grant (any path outside the home folder, which `DocGrantPath` refuses) falls back to
reading the file by its path with no grant. Matches `NSDocumentController.noteNewRecentDocumentURL` and opening a file named in
`application(_:openFile:)`. Not matched: a grant for system log paths (`/var/log`) for apps with the `log-store` entitlement,
which needs a second root in the shell's grant store; Console's Reveal and Move to Trash do not reach those files.

## Folder packages (`bifrost_app::document_package`, TextEdit RTFD)

A folder package is a document whose on-disk form is a directory that the file panels and Files treat as one file (Apple's
package document type, here `public.rtfd` for RTFD). Code: `springboard/sdk/src/document_package.rs`.

| Piece | Bifrost |
|---|---|
| package types | `PACKAGE_TYPES`: extension to uniform type identifier (`rtfd` to `public.rtfd`); `package_type_for_name` and `is_package_name` decide by the extension alone |
| manifest | `package_types = ["rtfd"]` in `manifest.toml` (shell `apphost::Manifest::package_types`); declares the packages an app reads and writes |
| read | `read_package_dir(dir)`: regular files at the top level, sorted by name; subfolders and links skipped; refused past `MAX_PACKAGE_BYTES` (64 MiB) |
| write | `write_package_dir(dir, files)`: files written into a temporary sibling folder, then the package is renamed into place; the old package is moved aside and removed only after the new one is in place, and restored if that rename fails. Names with a separator or `..` are refused before anything is written |

The document controller opens a package folder that a grant hands over: `Document::read_package(files, uttype)` reads
the files, and the entry remembers the folder (read from `/proc/self/fd`). A save in place of such a document writes the
package folder with `document_package::write_package_dir` (`Document::package_files` gives the files) and discards the
temporary file the shell made for the save. Save As and Rename of a package, and a package chosen in the open panel, are
not wired: the panel offers files only (`OpenOptions::directories` is not set for packages).

Gaps: no package-aware icon, no open-panel folder choice for packages, no Save As to a package folder, no package in Recents
until a package is opened, and an app that opens a package from a path (Files) has no controller entry for it yet.

## On-screen keyboard API (`UIKeyInput`, `UIReturnKeyType`, 0.4.77)
The keyboard becomes a system app (`docs/keyboard-system-app.md`). The SDK API exists first: `App::keyboard_config`,
`Ctx::keyboard_insert_text / keyboard_delete_backward / keyboard_height`, wire types `KeyboardConfig`, `ReturnKey`,
`KeyboardLayout`, `KeyboardOut`. Marked text, selection, the full `UIReturnKeyType` set and `inputAccessoryView` are now on the wire: see "Text input client" below.

## Event modifier flags (`NSEvent.modifierFlags`, E2E-3)
Every press, drag and release an app gets carries the modifier keys held at the time, so a click can be a Shift-click
(extend a selection from its anchor) or a Cmd-click (toggle one item), as `NSEvent.modifierFlags` on a mouse event.

- Wire: `bifrost_proto::ModifierFlags`, re-exported as `bifrost_app::ModifierFlags`. The bits are the reference's
  `NSEventModifierFlags` raw values (`CAPS_LOCK` 1<<16, `SHIFT` 1<<17, `CONTROL` 1<<18, `OPTION` 1<<19, `COMMAND` 1<<20);
  unknown bits are dropped on decode. `ToApp::Touch` and `ToApp::WindowTouch` end with the flags as a `u32`; a packet
  from a sender built before them decodes as none held, and a decoder built before them stops at `t_ms`.
- Shell: `HwKeys::modifier_flags` (Shift, Ctrl, Option without AltGr, Cmd, and Caps Lock while it is on) is copied to
  the shell on every key event (`Shell::set_key_mods`) and stamped on each press it forwards to an external app
  (`ext_touch`, `ext_window_touch`). Presses the shell synthesizes (accessibility actions, the keyboard app) carry none.
- SDK: `Input::Touch { kind, id, x, y, t, mods }`, on the main window and on extra windows (`App::window_event`). The
  direct Wayland path (`sdk::wl::input`) fills `mods` from `wl_keyboard.modifiers`.
- Gaps: there is no `flagsChanged:` event and no class-level `NSEvent.modifierFlags` (the state at any moment, outside
  an event); an app reads the flags from the press only. Key events carry modifiers only as chords (`CHORD_*`): the
  arrows with a modifier reach an app as chords when its manifest declares `edit_keys` or a text field is focused
  (Mail declares it for Shift-Up and Shift-Down). Touch frames (`App::touches`) do not carry the flags yet.

## Text input client (`NSTextInputClient`, `UITextInput`, `UITextInputTraits`)

Apple parity: an IME composes in the field (marked text), the candidate window follows the caret, and predictive text sees
the text around it. Ours has a wire part and an editing model.

Wire (ToApp 60 to 63, ToShell 0x150 to 0x153; `springboard/proto/src/lib.rs`, SDK `Ctx` and `App::text_input`):

| Apple | BifrOSt |
|---|---|
| `setMarkedText:selectedRange:replacementRange:` | `ToApp::SetMarkedText { text, sel_start, sel_len }` (60): from Wayland `text-input-v3` `preedit_string` + `cursor_begin/end` or the keyboard app |
| `unmarkText` | `ToApp::UnmarkText { commit }` (61): commit keeps the text, otherwise the composition is dropped |
| `deleteSurroundingText` (text-input-v3) | `ToApp::DeleteSurrounding { before, after }` (62), bytes |
| `inputAccessoryView` button | `ToApp::AccessoryAction(id)` (63); the row is `ctx.input_accessory(Vec<AccessoryItem>)` (0x153, at most 8) |
| `selectedTextRange`, `markedTextRange`, `textInRange:` | `ctx.text_state(TextState { text, cursor, anchor, marked })` (0x150): a window of at most 4000 bytes around the caret, UTF-8 byte offsets |
| `firstRectForCharacterRange:` / `caretRect(for:)` | `ctx.caret_rect(x, y, w, h)` (0x151), window points |
| `UIReturnKeyType` (all 12), `UIKeyboardType`, auto-correct, secure entry | `ctx.input_traits(InputTraits { return_key: ReturnKeyType, content: TextContentKind, flags: tiflags::* })` (0x152); `ReturnKeyType::basic()` is the nearest `ReturnKey` label the keyboard app can show today |
| `hasMarkedText` | `TextBuffer::has_marked_text` |

Offsets are UTF-8 bytes (Wayland's unit); `text::byte_to_utf16` and `utf16_to_byte` convert to `NSRange`. The decoders reject
a selection that cuts a character, a marked range outside the text and a non-finite caret rectangle.

Model (`springboard/ui/src/text.rs`, `bifrost_ui::text`, no dependencies):

- `TextBuffer`: text, selection (`anchor`, `caret`), marked range. Movement by grapheme (`move_char`, collapses a
  selection like AppKit), word (`move_word`), line (`move_line`), document and vertical (`move_vertical`, keeps the column).
  Editing: `insert_text` (replaces the marked text or the selection), `delete_backward / forward`, `delete_word_backward
  / forward`, `delete_to_line_start`, `kill_line` (ctrl-K, into a kill buffer), `yank`, `transpose`, `open_line`,
  `delete_surrounding`. IME: `set_marked_text(text, sel_start, sel_len, replacement)`, `unmark_text(commit)`,
  `cancel_marked`. Every edit returns an `Edit` that `revert` / `reapply` undo and redo.
- `TextEditor`: a buffer plus an `UndoManager`. Typing is coalesced into one step until the caret moves, a newline is
  typed or another edit happens; a whole composition is one step when it commits; `delete_surrounding` is one step.
  `perform(EditCommand, extend)` and `handle_key(KeyCombo)` use `text::binding`: the emacs set every Cocoa field has
  (ctrl-A / E line start / end, F / B / N / P, D, H, K, Y, T, O), option-arrows by word, cmd-arrows by line / document,
  option-backspace, cmd-backspace, cmd-A / Z / shift-Z.
- The standard text keys (`text::binding`, `text::key_action`; Shift extends the selection of every movement; the tests in
  `ui/src/text.rs` run the whole table over the engine, the field and the Text Editor's own page):

  | Keys | Does |
  |---|---|
  | Left, Right, Up, Down | by character; by line (a single line goes to its start or end) |
  | Home, End, Cmd+Left, Cmd+Right, Ctrl+A, Ctrl+E | line start, line end |
  | Cmd+Up, Cmd+Down, Cmd+Home, Cmd+End | document start, document end |
  | Option+Left, Option+Right, Ctrl+Left, Ctrl+Right | word back, word forward |
  | Page Up, Page Down | a page of lines up or down (`TextView::perform` knows its height; a field goes to its ends) |
  | Backspace, Ctrl+H; Delete (Fn+Delete), Ctrl+D | delete back; delete forward |
  | Option+Backspace; Option+Delete | delete the word back; the word forward |
  | Cmd+Backspace; Ctrl+K | delete to the line start; kill to the end of the paragraph |
  | Cmd+A, Cmd+C, Cmd+X, Cmd+V, Cmd+Z, Shift+Cmd+Z | select all, copy, cut, paste (own undo step), undo, redo; Ctrl+C / X / V / Z are the same |
  | double click, triple click, drag | select a word, select the paragraph, grow the selection from the press by that unit |

  How a key gets to a field: Cmd+A / C / X / V / Z are shell shortcuts; for an SDK app in front the shell sends them as
  `Input::Command` (the app's own enabled menu item on the chord first, the chord itself when the app did not report the
  command in `App::commands`), after a sheet or panel of the shell has had its say (`Shell::hw_shortcut` -> `hw_type` ->
  `ext_type_out`). Ctrl+A reaches the app as a chord, because it is the line start there. The movement and delete keys with
  a modifier, and Home, End, Page Up, Page Down and forward Delete, are chords too (`HwKeys::nav_mods`, `chord_mods`); they
  go to an app that declares `edit_keys` and to any SDK app with a text field focused (it has reported a caret). The shell's
  own fields (`shell/textfield.rs`) are separate and read the same keys as `Out::Named` names from `HwKeys::field_edit`
  (`Home`, `End`, `WordLeft`, `WordRight`, `Shift...` to extend, `DeleteWord`, `DeleteWordForward`, `DeleteLine`,
  `DeleteForward`) next to Cmd+A, C, X, V, Z; a secure field treats its text as one word. Shift+click needs the modifiers on
  the pointer events, which the app protocol does not carry yet, so it is not in the SDK field or the Text Editor.
- Terminals send keys, they do not edit: `terminal_keys` (`springboard/ui/src/terminal_keys.rs`, `bifrost_ui::terminal_keys`,
  no dependencies) maps chords to the bytes the pty expects, so the hardware keyboard, the on-screen Ctrl key and
  `term send_keys` agree. `control_byte` (Ctrl+A 1 through Ctrl+Z 26, Space/@ NUL, brackets, `^`, `_`), `alt_bytes`
  (ESC plus the key), `alt_word_bytes` (Alt+Left/Right word jumps), `ctrl_bytes` / `alt_combo_bytes` / `alt_arrows`
  (the same over `KeyCombo`, leaving shell shortcuts out), and `CONTROL_BINDINGS` (every byte with its readline action).
- A view feeds the shell: after each change `ctx.text_state(...)` from `TextBuffer::surrounding(4000)`, and
  `ctx.caret_rect(...)` from its own layout (the model has no geometry).

Gaps, decided:

- Done: SDK `view::TextField` is backed by `TextEditor` (emacs and macOS bindings by `text::binding`, click and drag
  placement from the caret marks `draw` records, marked text drawn underlined, the IME clause heavier). While first
  responder it implements `View::text_input_info` / `apply_text_input`; `ViewApp::sync_text_input` (run after `event`,
  `text_input`, `draw`) sends `TextState`, `CaretRect` and `InputTraits` only when they changed, and an empty
  `TextState` when focus leaves text. An app forwards `App::text_input` to `ViewApp::text_input`. Traits are set with
  `TextField::with_traits`. The shell stores state, caret (`AppHost::text_state/caret_rect/input_traits`), feeds
  `ReturnKeyType::basic()` to the keyboard app, and forwards keyboard-app composition (`KeyboardOut::Preedit` /
  `Unmark`, sub-codes 8 and 9 of the existing tag) as `SetMarkedText` / `UnmarkText`. `textinput::popup_origin` places
  a candidate popup from the caret (`Shell::ext_caret_screen`).
- Wayland side: `bifrost-wm` serves `zwp_text_input_v3` / `zwp_input_method_v2` (`docs/window-server.md`), and the shell
  is the input method: it binds `zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1` on its seat
  (`src/bare/wayland.rs`), raises the keyboard on activate, and types into the focused client's field with
  `commit_string` / `set_preedit_string` / `delete_surrounding_text` and `commit`, named keys through the virtual keyboard.
  The mapping is in `src/shell/textinput.rs`. The keyboard does not yet use the content kind of the field. The keyboard app (`apps/keyboard`) does not yet send `Preedit`, draw the other return
  keys, or use secure/content traits. An installed input method draws its own candidate list (see "InputMethodKit: the input
  method host"); the keyboard app and Wayland clients do not get one.
  `AccessoryItem`s are stored in the host (`AppHost::accessory`) but not drawn; `AccessoryAction` is not sent.
- Only `view::TextField` is a text input client; other SDK text widgets (`widgets::text_field` over
  `bifrost_ui::TextField`, apps with their own fields) do not use `TextBuffer`, and the field does not mask secure
  entry or handle double-click word selection (pointer events carry no click count).
- Grapheme clusters are approximated without the Unicode tables: combining marks, variation selectors, emoji modifiers,
  ZWJ sequences, flags, CRLF and Hangul jamo are right; Indic conjuncts, precomposed Hangul LV / LVT with trailing jamo
  and rarer scripts can split. Word boundaries are alphanumeric runs, not UAX #29 (no dictionary segmentation for CJK).
- `move_vertical` works on paragraphs; a soft-wrapped view must move by its own visual lines.
- A `replacementRange` is honoured only when nothing is marked yet; `attributedSubstring`, marked-text attributes
  (underline styles) and `selectedRange` inside the field's surrounding window beyond 4000 bytes are not carried.
- Not matched: `NSTextInputContext` input source switching, `characterIndexForPoint:`, `UITextInteraction` handles,
  dictation, `UITextInputDelegate` (`selectionWillChange`) and bidirectional caret movement (movement is logical).

## Compose method (apostrophe accents, Icelandic letters, kana, `bifrost_ui::compose_method`)

The built-in input method that every text input runs with no provider process. Round 75 (2026-10-13) moved the flow out of the
shell's Search field into this module, so the shell and every SDK text field share one implementation.

- **Keys**: a typed apostrophe arms the method. The next letter that has accented forms opens the candidate list for it (`'a` offers
  a, a-acute, a-grave, ...). Digits 1 to 9 pick a candidate (`MAX_SHOWN` is 9). Any other key ends the composition and keeps the
  letter. The table includes the Icelandic letters (acute vowels, o-umlaut, ae, eth, thorn).
- **Japanese (Romaji) source**: `compose_method::set_japanese_source(on)`. The runtime sets it when the shell sends
  `ToApp::InputSource { japanese }`, so every SDK app follows the shell's input source. Romaji becomes marked hiragana as it is typed,
  Space opens the hiragana and katakana list, and a digit picks from it.
- **State**: `Composer` is what one text input keeps (the armed apostrophe, the kana being composed, the open list). It exposes
  `is_armed`, `is_composing`, `is_open` and `candidates()`.
- **Flow**: `type_text(c, target, text)`, `type_text_as(.., japanese)`, `backspace`, `end_for_control` and `accent_hold(c, target, letter)`
  run over a `ComposeTarget` (the field's marked text, commit and plain insert). The model draws nothing. The caller paints the list
  (`widgets::candidate_list`).
- **SDK fields**: `bifrost_ui::text::TextEditor` owns a `Composer`. Its `type_text` is the typed-key entry point, so an app's own
  field composes apostrophe lists, kana and the candidate list. A key hold sends `ToApp::AccentHold { letter }`, and `apply_text_input`
  hands the letter to `TextEditor::accent_hold`.

Gaps: the shell cannot see an app's compose state, so a hold while an app has an apostrophe armed is ignored by the app and the letter
repeats. `RichTextView` (`sdk/src/attributed.rs`) has no marked-text model, so accent hold is not wired there. Text replacements and
the spelling underline in SDK fields are not wired. Midget checks: kana in an app field, and accent hold on a key hold.

## Native apps follow the appearance (`NSApp.effectiveAppearance`, freedesktop Settings portal)

Apple parity: every AppKit app sees `NSApp.effectiveAppearance` and the accent without asking. Native Linux apps
(GTK, Qt, Firefox, Servo via winit/egui) get the same through the standard `org.freedesktop.portal.Settings`
interface, served by our own `bifrost-portal` (no xdg-desktop-portal or GNOME stack).

- The shell (`Shell::appearance_tick`) writes `$XDG_RUNTIME_DIR/bifrost-appearance` (runtime dir from
  `/run/bifrost/user/session.json`) atomically, only when the effective scheme (Auto resolved) or accent changes,
  and once at startup; chowned to the account when the shell runs as root. Format: `color-scheme=1|2`
  (1 dark, 2 light) and `accent=R G B` (0..1). Code: `springboard/src/appearance_portal.rs`.
- `bifrost-portal` (`springboard/src/bin/portal.rs`) owns `org.freedesktop.portal.Desktop` at
  `/org/freedesktop/portal/desktop`, Settings version 2: `ReadAll`, `Read` (deprecated, double variant),
  `ReadOne`, and `SettingChanged`. Namespace `org.freedesktop.appearance`: `color-scheme` (u), `accent-color`
  ((ddd)), `contrast` (u, always 0). Other keys return `org.freedesktop.portal.Error.NotFound`. It polls the
  file every 500 ms. D-Bus activated by `/usr/share/dbus-1/services/org.freedesktop.portal.Desktop.service`.
- Session bus: `packaging/arch/wayland/session` starts `dbus-daemon --session` as the account at
  `$XDG_RUNTIME_DIR/bus` (exported as `DBUS_SESSION_BUS_ADDRESS`, so `session.json` and `bifrost-apps` carry it);
  the session script starts `bifrost-portal`. The native-app sandbox (`bare/app_policy.rs`) binds that socket and sets
  the variable. Launch-time env such as `GTK_THEME` cannot follow later switches, so the portal is the only live path.
- Not yet matched: `contrast` does not follow Increase Contrast, and Auto switches reach apps up to 500 ms late.

## Window vibrancy (`NSVisualEffectView`: material, blendingMode, state)

Apple parity: a Finder-style source list and a unified toolbar are translucent and show the blurred desktop, and
the windows behind the window. Ours, end to end:

- SDK: `widgets::visual_effect(s, ui, bounds, radius, Material, Blending, State)` is `NSVisualEffectView`.
  `Material` has every current `NSVisualEffectView.Material` case (`Sidebar`, `Titlebar`, `HeaderView`, `Menu`,
  `Popover`, `Sheet`, `Hud` = `.hudWindow`, `FullScreenUi`, `ToolTip`, `Selection`, `WindowBackground`,
  `ContentBackground`, `UnderWindowBackground`, `UnderPageBackground`; `Material::ALL`), each with `tint()`,
  `solid()` (Reduce Transparency), `blur_pt()` and `kind()`. `Blending::BehindWindow` (default) writes the
  premultiplied tint inside the rounded rect (corners anti-aliased against the content) and declares the region to
  the shell; `Blending::WithinWindow` blurs the app's own pixels (`widgets::material`). `State` is
  `NSVisualEffectView.State` (`FollowsWindowActiveState` default, `Active`, `Inactive`). Where the shell offers no
  vibrancy (CPU path, older shell, Reduce Transparency) a behind-window effect draws `solid()`.
  `source_list` and `toolbar_band` use it for `.sidebar` and `.titlebar`.
- Protocol: `ToShell::Materials(Vec<Material>)` (`kind: MaterialKind` (14 cases), `rect` in window points, `radius`,
  `state: MaterialState`; the state rides in the top two bits of the kind byte), at most `MAX_MATERIALS` (16), sent
  only when the list changes. Shell to app: `appearance::VIBRANCY` (GPU presenter, no Reduce Transparency) and
  `appearance::MATERIALS` (the shell decodes every kind and state). Without `MATERIALS` the SDK sends
  `MaterialKind::legacy()` kinds (`Sidebar`, else `Titlebar`) with the default state, which every shell decodes.
- Window server (Apple parity: `NSVisualEffectView` behind-window sidebar material): under bifrost-wm the shell
  sends each External window's material rects (`expand_material`, window radius growth on inner sides) as the
  `ext-background-effect` blur region, exactly as for built-in windows; apps need no extra call beyond
  `visual_effect` / `source_list`. Direct Wayland-surface apps do not send a region yet (gap).
- Version guard: an app whose SDK protocol differs from the shell's (`Hello`, or packets that do not decode) is
  stopped, logged, and its window shows "This app needs an update" instead of black (`AppHost::incompatible`).
- Shell: `AppHost::materials` holds the list; `Shell::take_wayland_materials` turns it into window px with a
  `glass::Kind` (`Sidebar`, `Titlebar` for title bar and header, `Content` for the background materials, `Menu` for
  the floating ones) and the fixed state, and adds the Settings sidebar column.
- Presenter (`src/gpu/wl.rs`): the front window's materials blur a backdrop of the wallpaper and the windows under
  it (`Presenter::under`, rebuilt at most 30 Hz when a lower window changes, not when the front window itself
  repaints). A material that follows the active state is an opaque plate (`glass::Kind::inactive`) while its
  window is inactive. Saturation follows the scheme (1.8 light, 1.7 dark, `glass::saturation`: Apple's
  image-effects sample uses `saturationDeltaFactor` 1.8; a cross-platform toolkit fits 2.0 / 1.7 to the simulator).
  Window backgrounds have no specular rim; menus and the shell's chrome do. A per-pixel dither hides banding.
- Increase Contrast turns Reduce Transparency on and locks its switch, as macOS does
  (`appearance::reduce_transparency`).
- Vibrant labels (`NSAppearance.vibrantDark/Light` text): `widgets::Vibrant` (`Label`, `Secondary`, `Tertiary`,
  `Quaternary`; strengths are the label colours' alphas, 0.85 / 0.55 (0.50 light) / 0.25 (0.26) / 0.10) and
  `widgets::blit_vibrant(s, img, x, y, level)` blend white-rendered text into the material: plus-lighter in Dark
  (light is added), plus-darker in Light (light is removed), alpha untouched. The compositor's premultiplied blend
  then gives backdrop + label (Dark) or backdrop + label - 1 (Light), the published definitions of the two blend
  modes. `widgets::vibrant_text()` says when to use it (vibrancy on, no Reduce Transparency). `source_list` draws
  labels, headers, counts and subtitles this way off the selection plate (the plate is opaque, so its text is plain).
  Apple's exact per-level vibrant values are private; the strengths above are the documented label alphas.
- Vibrancy model (`bifrost_ui::vibrancy`, pixel math in `bifrost_render::vibrancy`, tables `ui::VIBRANT_LABEL` /
  `VIBRANT_FILL`): `Role` (label or fill) x `Level` (primary .. quaternary, SwiftUI hierarchical `.primary` ..
  `.quaternary`) gives a strength; `blend(light, on_material, reduce_transparency)` gives plus-lighter (Dark),
  plus-darker (Light) or `Normal`. Labels use the label alphas (0.85 / 0.55 / 0.25 / 0.10; Light 0.50, 0.26 for
  secondary and tertiary); fills use the system fill alphas (Dark 0.36 / 0.32 / 0.24 / 0.18, Light 0.20 / 0.16 /
  0.12 / 0.08). `render::vibrancy::blit_image` (text, symbols) and `fill_rect` (shapes) draw on the CPU surface;
  alpha is kept so the GPU glass path composites backdrop plus or minus the amount. `glass::Kind::allows_vibrancy`
  is false for shell chrome (Liquid Glass) and under Reduce Transparency, where both draw the opaque colour at
  the level's alpha. Mapping: `allowsVibrancy` = `Kind::allows_vibrancy`, `vibrantLight/vibrantDark` = the scheme
  position, `foregroundStyle` hierarchy = `Level`. Apple's exact vibrant values are private; these are the
  documented label and fill alphas. Not yet wired: `widgets::Vibrant` (SDK) still carries its own copy of the
  label table and the symbol/fill call sites in `source_list` use plain colours.
- Backdrops: the shell keeps no live backdrop. Client windows send materials to the window server, which blurs the
  real scene; without the material protocol they draw `Kind::inactive()` plates. Shell chrome blurs the wallpaper.
- Model: `bifrost_ui::visual_effect` (`MaterialKind`, `BlendingMode`, `MaterialState::looks_active`,
  `BackgroundStyle` = `interiorBackgroundStyle`, `Mask` = `maskImage` (rounded rect with per-corner `Corners`, or an
  alpha mask), and `MaterialRequest`, the per-view request the window server consumes). The SDK draws one with
  `widgets::visual_effect_request`. Tints per material and scheme are `MATERIAL_*` in `render/src/ui.rs`; the new ones
  (menu, sheet, tool tip, header, content, under-window, under-page, full-screen) are derived from the measured
  neighbours, not yet measured. `isEmphasized` changes the `.selection` tint (full accent); an inactive selection is grey.
  Decision (2026-10-12, reconciliation): there is one model. `visual_effect::{MaterialKind, BlendingMode, MaterialState,
  Corners, MaterialRequest}` is used by the shell (`ShellSurface::request`, `material_request.rs`, which now only names the
  16 shell surfaces and their kinds), the wire (`proto` re-exports the same enums; `proto::Material` carries kind, rect,
  uniform `radius`, state, `blending`, `emphasized` and optional per-corner `corners`, flags byte after the radius: bit 0
  emphasized, bit 1 within-window, bit 2 four corner f32 follow), the SDK (`widgets::visual_effect_request` declares the
  full request; the canvas still writes the tint with the largest corner) and the window server (`bifrost-wm`
  `MaterialKind::from_model`). `SDK_VERSION` is 2: a version-1 app fails `Hello` and shows "This app needs an update".
  SwiftUI `Material` maps onto kinds plus `MaterialRequest::strength` (see Glass and materials for apps).
- Not matched yet: alpha masks on the wire (rounded rect only; an alpha mask is sent as its bounding rectangle), `strength`
  and `glass` (Liquid Glass) on the wire, and vibrant fills (`vibrancy-fill` levels) as separate API; the
  within-window `widgets::material` blurs on the CPU (apps draw on the CPU; the presenter never sees the pixels
  under a within-window material).

## Main window size from the app (`NSWindow.setContentSize`, `contentMinSize`; `bifrost_app::windows::set_content_size`)

Apple parity gap closed (E2E follow-up): an SDK app could not resize its own main window. Golden Gate Calculator
resizes its window per keypad (27-Golden-Gate-Calculator.png is the 230-point Basic window), which needs it.

- Protocol: `ToShell::SetContentSize { w, h }` (tag 0x252, two f32 points) and `ToShell::SetContentLimits { min, max }`
  (tag 0x253, four f32; a 0 x 0 pair is `None`). The decoder refuses sizes that are not finite and positive or are over
  `MAX_CONTENT_PT` (16384) on a side.
- SDK: `windows::set_content_size(ctx, w, h)` and `windows::set_content_limits(ctx, min, max)`. A bad size is not sent.
  The preferred initial content size stays the manifest's `window_size`; `min_size` / `max_size` are the defaults that
  `set_content_limits(.., None, None)` falls back to.
- Shell: `apphost` queues the size and applies it after its message loop (`Shell::desk_set_content_size`), keeps the
  limits per running app (`desk::set_content_limits`, merged in `size_limits` with `SizeLimits::with_content_limits`,
  dropped when the app exits). `Desk::set_content_size` keeps the top-left corner, holds the size to the limits and the
  work area, and moves the window only as far as the work area needs; the change runs the zoom effect
  (`desk_frame_changed`) and the app hears the size as a configure. A zoomed or full-screen window keeps its frame and
  takes the size as the one it goes back to (Apple sets the frame even then; ours leaves the window server's zoom
  state alone). A `fixed:` window takes the app's size as asked: only the person cannot resize it.
- Not yet: the direct route (`BIFROST_DIRECT=1`, the app's own Wayland toplevel) does not resize its toplevel on this
  request; there is no `ToApp` answer beyond the configure (`windowDidResize`), and no `setFrameOrigin` equivalent.
- Calculator uses it: Basic 230 x 512, Scientific 560 x 522, Programmer 560 x 586, Convert 640 x 660 points, each
  also the window's minimum; a window too narrow for the four keypad segments shows one keypad pull-down instead.

## Window level (`NSWindow.level`, floating windows; `bifrost_app::windows::set_level`)

Apple parity: an app can ask for its window to float above the normal windows (Calculator's Always on Top, Stickies'
Float on Top, the floating CPU window of Activity Monitor). Ours, end to end:

- Protocol: `ToShell::WindowLevel(WindowLevel)`, tag 0x165, one byte (`Normal` = 0, `Floating` = 1); an unknown byte is
  a bad value. `springboard/proto/src/lib.rs`.
- SDK: `windows::set_level(ctx, Level::Floating)` (`Level` is `proto::WindowLevel`). It sends the message on every call;
  the app keeps its own checked state. There is no standard Window menu in the SDK yet, so an app builds its own
  item with `AppMenuItem::new(..).checked(on)` (Calculator and Stickies do).
- Shell: `AppHost.window_level` keeps the level the app asked for while the app runs (`Normal` until it asks). The app
  host's window is the desk window `AppId::External(slot)`, drawn as the wayland toplevel whose frame id is
  `win_id(app)` (`apps()` position plus one). `Desk.floating` lists the floating apps, refreshed by
  `take_wayland_windows` (`sync_levels`); `Desk::band_top` puts a raised or opened window at the top of its band, and
  the frames of floating windows go last.
- Shell to window server: the private protocol `bifrost_window_level_v1` (`wm/protocol/bifrost-window-level-v1.xml`,
  client `springboard/src/bare/window_level_client.rs`, server `springboard/wm/src/window_level.rs`). `set_level(surface,
  level)` stores the level on the surface; the window server applies it at once to a mapped toplevel and at its map.
  Version 5 adds the output's white point (Night Shift on the whole screen, `set_white_point` and the
  `white_point_mode` event; `docs/gpu-first.md`, "Night Shift on the whole output"). Apple's Night Shift is a display
  property (CoreBrightness), not a per-window one, so the window server owns it: it tints every app's window, the
  pointer included, on the CRTC gamma ramp or a final GLES pass, and the shell stops tinting its own surfaces.
- Window model: `springboard/wm/src/wm.rs` `Level` (`Normal`, `Floating`). The stack is sorted by level: a floating
  window is above every normal window, a raised or re-levelled window goes to the top of its own band, and raising a
  normal window does not pass a floating one. Hit testing uses the same order. The key window is the one last raised
  (or the topmost shown window when it is minimized or closed). Menus, panels and the menu bar are above both bands
  and are not modelled here.
- Each window has its own level. The main `WindowLevel` lifts only the main window, and a floating extra window does not
  lift the main window or its siblings. Stacking follows the window's own level in `Desk::floating_apps`.
- Focus and stacking: the window server's focus raises through the model and then restores the space order from
  the model, so a floating window stays above the normal ones.
- Extra windows (`app_windows`): `windows::set_level` levels only the main window. An extra window floats with
  `windows::set_window_level(ctx, id, Level::Floating)` (it takes the extra window's `WindowId`), which
  sends `ToShell::WindowLevelOf { id, level }` (tag 0x24c, id then one byte). The shell keeps the flag in the window table
  (`app_window_table.rs` `set_floating`, `floating_ids`; id 0 is refused) and `Desk::floating_apps` lists the floating
  extra windows with the floating apps, so they stack last. Activity Monitor's CPU windows use it (activity-desktop 0.8.0),
  and its GPU History window joined them in 0.10.0 (Window > GPU History; Keep CPU and GPU Windows on Top levels all three).
  Built and reviewed by reading only; no Midget run yet. Activity Monitor 0.10.0 also turns its Network tab into a
  per-process table from the kernel's socket counters (`sample_net.rs`, `ss -tinpH`: TCP only, the user's own processes,
  no packet counts yet) and adds the Real Mem (proportional set size), Purgeable (`MADV_FREE` pages) and Kind (ELF machine)
  columns, each read only while its column or tab is shown.
- Not matched yet: built and reviewed by reading only (no build, no tests, no Midget run). Notes still share one
  window (Stickies' Float on Top applies to the board window). Apple's `NSWindow.Level` names (`.floating`, `.normal`,
  `.statusBar`, `.modalPanel`) beyond these two are not exposed; the menu bar and panels keep their own layer.

## Web view (`WKWebView`, `WKProcessPool`, `WKNavigationDelegate`; Browser 0.3.0)

`bifrost_app::webview` puts web content in an SDK app. Like WebKit, pages run out of process: a `WebProcessPool`
spawns the engine `bifrost-webview` (Servo 0.7.0 embedded with libservo and a software GL context, source in
`springboard/apps/browser/engine/`, built by `tools/build-webview-engine.sh`) and holds any number of web views
(`WebViewId`), one shown in a rectangle of the window.

| Apple | BifrOSt |
|---|---|
| `WKWebView init` + `loadRequest:` | `pool.create(url)`, `pool.load(id, url)` |
| `goBack` / `goForward` / `reload` / `stopLoading` | `go_back`, `go_forward`, `reload`, `stop_loading` |
| `title`, `URL`, `isLoading`, `estimatedProgress`, `canGoBack`, `canGoForward` | `pool.state(id)` (`WebViewState`) |
| `WKNavigationDelegate` start / commit / finish | `WebViewEvent::LoadStarted`, `LoadCommitted`, `LoadFinished` from `pool.poll()` |
| Develop > Reload From Origin | `pool.reload_ignoring_cache(id)` (clears the HTTP cache, then reloads; no per-origin cache flag) |
| `customUserAgent` | `pool.set_user_agent(Option<&str>)` (`Servo::set_preference`; subsequent requests) |
| Responsive Design Mode | `pool.set_viewport_override(area, w, h, dpr)` letterboxes the view |
| `WKUIDelegate createWebViewWith`, `webViewDidClose` | `WebViewEvent::CreateWebView`, `Closed` |
| `webViewWebContentProcessDidTerminate` | `WebViewEvent::ContentProcessTerminated`, `EngineExited` |
| view frame | `pool.set_frame(x, y, w, h, scale)` (window pixels) |
| appearance (`prefers-color-scheme`) | `pool.set_dark` |

Drawing: each frame the engine presents is copied once into shared memory (the `Surface` pixel format) and
`pool.draw(surface)` blits it. Zero-copy (a Wayland subsurface or dma-buf from a GPU context) is the follow-up.
Input: `pool.touch` retains tablet touch behavior; `pool.mouse` sends desktop press/drag/release/hover,
`pool.mouse_leave` clears the native hover target, `pool.scroll` forwards wheel deltas, and `pool.key` forwards text,
named keys and Ctrl/Cmd chords. `pool.set_clipboard` updates the native clipboard delegate; copied text returns as
`WebViewEvent::ClipboardChanged`. Favicons return as bounded native `FaviconChanged` RGBA images. The event loop wakes on the engine through the new
`App::wake_fd` (the runtime polls it with the shell channel); call `poll` from `tick`. `url_for_input` is Safari's
smart address field (URL, bare host or web search).

`stopLoading` remains a no-op because Servo 0.7.0 has no public cancel-load API; the browser toolbar therefore
exposes Reload. Native embedder controls handle context menus, select popups, script dialogs, file inputs and color
inputs. `FilePicker` events carry extension filters and multiple-selection support; `file_picker_reply` supplies
the selected paths or cancels. Browser `uploads.rs` obtains granted descriptors with `Ctx::open_panel`, streams
private staging copies off the UI thread, and removes them when the document retires. Color pickers use shared
RGB controls; `color_picker_reply` supplies the chosen channels or cancels. `UnloadRequested` and `unload_reply`
surface Servo's navigation confirmation with a safe initial Stay action; direct tab close/quit remains outside
that confirmation path. `WebViewState::status_text` and `StatusTextChanged` carry native hovered-link destinations.
These integrations have not been checked on Midget and are not marked full parity.
Engine-owned cookie/local/session storage and the HTTP cache can be cleared through
`pool.clear_website_data`; full browser data clearing falls back to a stopped-engine profile reset when
IndexedDB, CacheStorage or HTTP authentication data exists, preserving content rules.
`pool.clear_session_history(id)` calls Servo's native API and keeps the current page. The engine defaults to hardware
GL with an explicit CPU fallback. Frame transport still uses readback and shared-memory copies; full WebView
parity remains incomplete.

### Find, zoom, session (Browser, round D2)

- `WebProcessPool::set_zoom` is `WKWebView.pageZoom` (clamped 30% to 500%). The Browser steps 25% with Cmd+Plus, Cmd+Minus, Cmd+0 and remembers the factor per origin in `~/.local/share/bifrost-browser/session.txt`.
- `WebProcessPool::find(id, query, FindDirection)` and `find_clear` follow `find(_:configuration:)`; the count is `WebViewState::find_matches`. Gap: Servo 0.7.0 has no find API, so the engine uses standard DOM traversal, Range and Selection APIs (case-insensitive, wrapping, bounded main-document search). No highlight-all, no match index, only the total; the case and whole-word options of `WKFindConfiguration` are not exposed.
- Session restore (tabs, titles, selected tab) and Cmd+Shift+T (10-entry stack) are app-level; per-tab back/forward history is not restored (only the last URL).

### Configuration, policy, data store, messages and panels (`web_config`, `web_nav`, `web_data`)

Host-tested state for embedding apps. The engine glue that feeds it is not wired yet, so these rows stay partial until checked on Midget.

| Apple | BifrOSt |
|---|---|
| `WebViewConfiguration`, `Preferences` | `WebViewConfiguration`, `WebPreferences` (`javascript_enabled`, `minimum_font_size`, ...) |
| `customUserAgent`, `applicationNameForUserAgent` | `WebViewConfiguration::effective_user_agent(default)`, applied when the engine starts |
| `UserContentController`, `UserScript` | `UserContentController`, `UserScript`, `InjectionTime` |
| `add(_:name:)`, `ScriptMessageHandler`, `ScriptMessage` | `UserContentController::add(handler, name)`, `ScriptMessageHandler`, `ScriptMessage`; pages post with `window.bifrost.messageHandlers.<name>.postMessage(body)` |
| `decidePolicyFor navigationAction` / `navigationResponse` | `NavigationDelegate`, `NavigationActionPolicy` (Allow, Cancel, Download); `PolicyBroker` resolves an unanswered decision to Allow after `DECISION_TIMEOUT_MS` |
| `BackForwardList` (`backItem`, `forwardItem`, `item(at:)`, `go(to:)`) | `BackForwardList` (`back_item`, `forward_item`, `item_at(offset)`, `go_to(offset)`) |
| `HTTPCookieStore` (`getAllCookies`, `setCookie`, `delete`) | `CookieStore` (`get_all_cookies`, `set_cookie`, `delete`), persistent cookies saved to `cookies.tsv` |
| `WebsiteDataStore` (`fetchDataRecords`, `removeData`) | `WebsiteDataStore` over the profile directory (`fetch_data_records`, `remove_data`, `remove_all`) |
| `WebUIDelegate` alert, confirm, text input, `runOpenPanelWith` | `UiDelegate`; `answer_script_dialog` returns the `(ok, text)` that `dialog_reply` takes |
| `FindConfiguration`, `FindResult` | `FindConfiguration` (`backwards`, `case_sensitive`, `wraps`), `FindResult`, `FindSession`, `find_matches` |

Gaps: the open panel has no engine event; a user agent change on a running view is not applied (Servo reads it at start-up).

### Engine wiring, content blockers, page PDF and the JavaScript context (2026-10-19, `web_bridge`, `content_rule_lists`, `web_print`, `js_context`)

The state in `web_config`, `web_nav` and `web_data` now reaches the engine (`apps/browser/engine`) through `bifrost_app::web_bridge`. The verbs are in the engine's header comment; apps use these calls:

| Apple | BifrOSt |
|---|---|
| `WKUserContentController.addUserScript`, `WKWebViewConfiguration` | `pool.install_configuration(&config)` keeps the configuration's scripts (the message bridge first, then the app's, each in its own function scope, `AtDocumentEnd` after `DOMContentLoaded`, main frame only when asked) as one document-start script slot in every view; pages loaded afterwards run it. Scripts over 120 KB are refused |
| `WKScriptMessageHandler.userContentController(_:didReceive:)` | page `window.bifrost.messageHandlers.<name>.postMessage(value)` writes a console line with the prefix `@@bifrost-msg`; the engine turns it into `scriptmsg` (never console text, whatever `devtools` says) and the pool reports `WebViewEvent::ScriptMessage { id, message }`; `route_script_messages(&mut controller, &events)` delivers it to the handler registered under the name. Bodies are JSON text up to 60 KB |
| `decidePolicyFor navigationAction` | `pool.set_navigation_policy_enabled(true)`, then `WebViewEvent::NavigationRequested { id, token, url }` for every navigation (engine `request_navigation`); answer with `pool.navigation_reply(id, token, policy)`, or `pool.answer_navigation_requests(&mut delegate, &events)` for a synchronous `NavigationDelegate`, or `NavigationRouter` for answers that come later (`DECISION_TIMEOUT_MS`, then allow). `Download` cancels the navigation and reports `WebViewEvent::DownloadStarted` with a file name from the URL |
| `HTTPCookieStore.getAllCookies` | `pool.request_cookies(id, token)` then `WebViewEvent::Cookies { id, token, cookies }` (the engine's jar for the page URL, HttpOnly cookies included); `reconcile_cookies(&mut store, url, &cookies, now)` makes a `CookieStore` agree with it |
| `HTTPCookieStore.setCookie` | `pool.set_page_cookie(id, &cookie, now)` through the page's own `document.cookie` (refused for `HttpOnly`); `pool.clear_engine_cookies()` deletes every cookie |
| `WKContentRuleList`, `WKContentRuleListStore` | `ContentRuleList::compile(id, json)` (the reference's JSON: `block`, `block-cookies`, `css-display-none`, `ignore-previous-rules`, `make-https`, `notify`; triggers `url-filter` with the regular-expression subset in `rule_match`, `url-filter-is-case-sensitive`, `if-domain`, `unless-domain`, `resource-type`, `load-type`, `if-top-url`, `unless-top-url`), `ContentRuleListStore` (`compile`, `look_up`, `remove`, `available_identifiers`, files in `<profile>/content-rules/`), `ContentRuleLists` (the lists a view uses), `pool.set_content_rules(&lists)` |
| `WKWebView.createPDF(configuration:)`, print operation page layout | `web_print::pdf_from_rgb(rgb, w, h, title, &PrintLayout::letter() / a4())`: the full-page snapshot (`ShotMode::Full`) is scaled to the paper width and cut into pages with a title and "Page n of m" in the margins |
| `JSContext`, `JSValue`, `JSExport` (functions), `exceptionHandler` | `js_context::JsContext` (`evaluate_script`, `set_value`, `set_function`, `set_exception_handler`, `exception`, `take_console`) over a fresh Boa realm; `JsOut` reads undefined, null, booleans, numbers (NaN and infinities too), strings, bigints, symbols, functions (source text), arrays and objects; `js_context::evaluate` / `set_global` convert to and from the SDK `JsValue` |

Browser (the system counterpart): the control socket has `browser.content_rules` (`op` list, compile, remove, clear, test), `browser.messages` (what pages posted to the `control` handler; `since_seq`, `clear`) and `browser.screenshot` with `format: "pdf"` (`paper` letter or a4, `margin`, `header_footer`; prints the whole page unless `selector` or `rect` is given).

Where Servo 0.7.0 limits this, and Apple behaviour that is not matched yet:

- Content rules: `block` is enforced for every web resource load and navigation (engine `load_web_resource`, `request_navigation`), `make-https` for navigations, `css-display-none` by a page script. `block-cookies` and `notify` have no hook in the engine; they are compiled and reported by `ContentRuleLists::evaluate` for apps that judge URLs themselves, but not applied to page loads. `load-type` compares sites by their last two labels (three for `co.uk` style suffixes) because there is no public suffix list. A pattern is matched on the first 2048 characters of the URL with a step budget.
- Navigation policy: the engine reports neither the frame nor the cause of a navigation, so every request reaches the delegate as a main-frame navigation of unknown type. A navigation response policy (`decidePolicyFor navigationResponse`) has no engine hook; downloads are still decided from the URL or the Browser's response probe.
- Cookies: the engine answers for one page URL at a time, and the engine cookie jar stays the source of truth; there is no engine call to set an HttpOnly cookie.
- Scripts: a script slot cannot target one view, and there are no content worlds. A change applies to documents loaded afterwards.
- Page PDF: the PDF holds the page as pictures (no selectable text, no CSS print rules, no page breaks at element boundaries); `WKPDFConfiguration.rect` is `ShotMode::Rect` on the snapshot. Printing to a device goes through `print_panel` with that PDF.
- JavaScript context: values are copied out as JSON-shaped trees, so object identity and `JSManagedValue` are not matched; there is no shared `JSVirtualMachine`, no timers and no `import`; a Rust type cannot be exported as a class with methods (functions only); a promise that needs a timer or I/O to settle is an exception. `JSContext` C API (`JSEvaluateScript` and friends) is not provided.
- Back and forward: the engine keeps the documents of the three history entries on each side of the current one alive
  (`engine/src/memory.rs`), so `go_back` and `go_forward` to those entries restore the page and its scroll offset like
  `WKBackForwardList` does. A farther entry, and an entry of a discarded tab, reloads at the top: Servo 0.7.0 has no
  scroll restoration (`history.scrollRestoration` is not implemented).
- Page rendering: CSS `backdrop-filter` is not implemented in Servo 0.7.0 (its style system parses it only under the
  off-by-default `layout.unimplemented` preference and layout never emits it), so a translucent sticky header shows the
  page behind it unblurred; `filter` works. The gap and what closing it needs are in `apps/browser/docs/servo-review.md`
  ("Remaining gaps"); the engine is not patched. `@supports (backdrop-filter: blur(1px))` is false, so a site can fall back.
- Fonts: the engine's fallback list is fixed in Servo, so CJK punctuation (U+3000 to U+303F and the other Common-script CJK
  blocks) relies on `packaging/arch/fontconfig/61-bifrost-cjk-fallback.conf`, which makes the installed Noto Sans CJK SC
  answer the list's last family name; the package scriptlet rebuilds the font cache for it (Browser README, "Web compatibility").

### Downloads (`web_download`)

- `navigation_response_policy(content_disposition, mime)` is `WKNavigationResponsePolicy.download` for attachments and MIME types the view cannot render; `WebViewEvent::DownloadStarted` is `didBecome download`.
- `WebDownload` is `WKDownload` (`original_request`, `web_view`, `progress`, `cancel()` returning `ResumeData` with offset and ETag). `WebDownloadDelegate` mirrors `WKDownloadDelegate` (`decide_destination`, `download_did_finish`, `download_did_fail` with resume data, `will_perform_redirection`); return values stand for completion handlers. `DownloadDriver` feeds transfer events to the delegate.
- `unique_destination` picks `name (1).ext` in the chosen folder; `resume_headers` builds the `Range` and `If-Range` headers.
- The engine emits `DownloadStarted` when the app answers a `NavigationRequested` with `Download` (see the section above); it does not yet choose downloads by response headers, and the Browser still uses its own model.

### Developer tools (`webview_dev`, Browser round 53)

`bifrost_app::webview_dev` extends `WebProcessPool` for Safari-style developer tooling (`WKWebView.evaluateJavaScript`
with a completion handler, `takeSnapshot(with:)`, `WKUserScript`). The base pool is untouched apart from three hooks
(`dev ...` lines are kept apart from `WebViewEvent`s, `send_raw`, `device_scale`). Requests carry a caller-chosen token
and the answer repeats it:

| Apple | BifrOSt |
|---|---|
| `evaluateJavaScript:completionHandler:` | `pool.dev_eval(id, token, js)` then `DevEvent::Eval { token, result: Ok(EvalValue { ty, json }) / Err(message) }` from `pool.dev_events()`; errors carry the thrown message with file and line |
| `takeSnapshotWithConfiguration:` | `pool.dev_screenshot(id, token, &ShotMode::View / Rect / Full, max_pixels, path)` writes a PNG, answered by `DevEvent::Shot` |
| `WKUserScript` at document start | `pool.dev_collector(js)` (used for the Browser's network log) |
| synthesized input | `dev_pointer`, `dev_wheel`, `dev_text`, `dev_key`, `dev_chord` in page CSS pixels |

Gaps: Servo 0.7.0's `evaluate_javascript` does not await promises; `take_screenshot` reads the rendering context, so
`Full` resizes it for a moment; there are no request hooks, so the network log is injected script (see the Browser
README, "Web Inspector"); `scale` of a snapshot and right or middle clicks are not supported.

The control socket got async replies for this (`bifrost_app::control`): an `App::rpc` handler returns
`Err(RpcError::pending(token, timeout_ms))` (`control::new_token()`), keeps the token, and later calls
`control::complete(token, result)` from any thread; the SDK holds the connection (64 at most) and answers on the next
loop pass, or with -32000 after the timeout (60 s at most). `RpcError::with_reason` adds `error.data.reason`. The event
loop polls every 50 ms while a call is pending. The Browser's methods are in `apps/browser/README.md`.

## Retained views (NSView / NSResponder / NSViewController)

`bifrost_app::view` (`springboard/sdk/src/view/`) is an optional retained layer over the immediate-mode `App` trait.
Existing apps are untouched; an app opts in by holding a `ViewApp<S>` and delegating `configure`, `event`, `controls`,
`commands` and `draw` to it (see the doc example on `ViewApp`). It reuses the bifrost-ui models instead of duplicating
them: `Responder` (`responder.rs`), `UndoManager` (`undo.rs`), `focus::next` (`focus.rs`), `values::Slider` and
`values::Segmented`, `stack::stack`, `Rect`, and `Node`/`Role` (`ax.rs`).

| Apple | BifrOSt |
|---|---|
| `NSView` (`frame`, `setFrameSize`, `needsDisplay`, `isHidden`, `hitTest:`, `intrinsicContentSize`) | `View` trait with a `ViewBase` (`frame`, `dirty`, `hidden`, `enabled`, `focused`); `set_frame`, `layout`, `draw`, `hit_test`, `intrinsic`, `needs_display` |
| `subviews` | `children()` / `children_mut()` (`Vec<Box<dyn View>>`) |
| `NSResponder` (`acceptsFirstResponder`, `mouseDown:`, `keyDown:`, `doCommandBySelector:`, `validateMenuItem:`) | `accepts_first_responder`, `handle(&ViewEvent)`, `perform(action) -> bool`, `validate(action) -> Option<bool>` (same shape as `Responder`) |
| `NSWindow` (`firstResponder`, `makeFirstResponder:`, `keyViewLoop`, `undoManager`) | `Window<S>` (alias `ViewTree`): `first_responder`, `make_first_responder`, `key_view_loop`, `tab`, `undo: UndoManager<S>` |
| `NSControl.sendAction:to:from:` (nil target) | `ViewBase::send_action(&str)`; the window sends it sender, ancestors, window (Undo, Redo), then the model `S: Responder` |
| Edit menu / key equivalents | `Window::send(action, &mut S)` from the first responder, `is_enabled(action, &S)`, `commands(&S)`; `action_for(Command)` gives `cut copy paste selectAll undo redo` |
| `NSViewController` (`loadView`, `viewDidLoad`, `viewDidLayout`) | `ViewController` + `ControllerHost` (puts the controller in the responder chain above its view) |
| `NSStackView` | `Stack` (`vstack` / `hstack`, `spacing`, `insets`, `Align`) over `stack::stack`; children with an `intrinsic` length keep it, others share the spare space |
| `NSTextField` label, `NSButton`, `NSSwitch`, `NSSlider`, `NSSegmentedControl` | `Label`, `Button`, `Switch`, `Slider`, `Segmented` (models from `values.rs`) |
| accessibility hierarchy | `View::ax_node`; `Window::ax_nodes()` (tree order, for `App::controls`) and `ax_tree()` (adds depth and parent id) |
| `setNeedsDisplayInRect:` / dirty rects | `Window::draw` returns the merged `Damage` of dirty views |
| `NSButton` checkbox, `NSStepper`, `NSProgressIndicator`, editable `NSTextField`, `NSPopUpButton` | `Checkbox`, `Stepper` (`values::Stepper`), `ProgressIndicator` (`bar`, `indeterminate_bar`, `spinner`; animated by `Window::tick(t)`), `TextField` (`bifrost_ui::TextField` model, `cut copy paste selectAll undo redo` via `perform`/`validate`), `PopUpButton` (`menu::Menu` + `widgets::popup_button`/`menu`, menu drawn as an overlay) |
| inset grouped `List` / `Form` | `Section` (13 pt card, `m::CARD_RADIUS`, hairline separators, footnote header and footer) inside `Form` (`m::DETAIL_MARGIN` inset, stacked sections) |
| `NSAccessibility` bridge | `ViewApp::controls_tree()` (parent, focused, checked from `Window::ax_ext`), `ViewApp::ax_set(id, AxAction)`: Press sends Return to the view, SetValue calls `View::ax_set_value` (Checkbox/Switch "on"/"off", Slider/Stepper number, TextField text, PopUpButton item title), Focus is `make_first_responder` |
| `NSResponder.resignFirstResponder` | `View::resign_first_responder() -> bool` (false refuses) and `did_resign_first_responder()`; `Window::make_first_responder` asks the old responder first and notifies it after the change. A pointer Down on a view that takes no focus, or on empty background, resigns the current responder (`makeFirstResponder(nil)`); a refusing responder keeps the focus. `PopUpButton` closes its menu on resign |
| `NSScrollView` / `NSClipView` (`documentView`, `scroll(to:)`) | `ScrollView::new(document)`, `content_offset()`, `max_offset()`, `scroll_to(y)` (clamped), `document_view::<T>()`. `View::child_offset()` and `clips_children()` let any container scroll its children: draw, hit test, overlay pick, focus geometry and pointer events (converted to content coordinates for `handle`) all apply the accumulated offset. Dragging pans through `bifrost_ui::scroll::Scroller` (slop, fling, rubber band; `tick` drives it), a thin overlay knob (`LABEL_2`) shows while moving |
| `NSMenu` / `validateMenuItem:` | `ViewApp::set_menus(Vec<(String, Vec<MenuEntry>)>)` (`MenuEntry::edit_menu()` is the standard Edit menu); `main_menu()` sets each item's `enabled` from `Window::is_enabled(action)`; `menu_action(id)` calls `Window::send` from the first responder. Ids are 1.. over non-separator items in order |

Behaviour. Frames are in window points (no per-view bounds, flipping or transform), so `draw` needs no origin. A view's
`draw` paints only itself; the window paints parents first. Pointer events go to the deepest visible view under the
pointer, which becomes first responder if it accepts, and bubble to ancestors until one returns true; Move, Up and
Cancel then go to the view that took the Down (implicit grab). Keys go to the first responder and outwards; unhandled
Tab (`Code(9)`) and Shift-Tab (chord on evdev key 15) walk the key-view loop in reading order. Touch positions are
converted from pixels with `Ctx::ui.s`. After `layout` the whole window repaints; afterwards each dirty view's frame is
cleared to `DETAIL_BG` and the view and its subtree are drawn again, so siblings must not overlap and a view that
paints its own backdrop must be marked dirty as a whole. Read a control's value after an action with
`window.view::<Switch>(id)` and `take_sent()`.

Clipping and dirty rectangles. `Surface` has no clip rectangle, so partial redraw is made safe differently: a dirty view
clears its own frame to `DETAIL_BG` and repaints its subtree, except that a view with `opaque_backdrop()` (`Section`)
is repainted whole when any descendant is dirty (the nearest opaque ancestor), so a toggled row never leaves a hole in
its card. A view that paints outside its frame (`dirty_rect()` differs from `frame()`, an open or just-closed pop-up
menu) forces a full repaint, and `draw_overlay` runs after the tree so menus are on top; an open overlay also wins the
hit test. Spinners and indeterminate bars call `set_needs_display` from `View::tick(t)`; the app calls
`ViewApp::draw_at(ctx, s, t)` from `App::draw` (or `Window::tick` then `Window::draw`).

Text field notes. The field owns its undo history (the `bifrost_ui::TextField` snapshots, grouped typing, 100 levels)
like `NSTextView.undoManager`; "undo"/"redo" reach the window's `UndoManager<S>` only when the field has nothing to
undo. Not done: caret placement and drag selection by pointer (a click only focuses), word and line movement, secure
entry, formatters. A pop-up menu closes by choosing, Escape, Tab, clicking the button again or clicking elsewhere
(`did_resign_first_responder`). `ProgressIndicator` has no accessibility role of its own (`Role` has no
progress case), so it is reported as static text with its percentage.

Apple parity gaps (recorded per the standing rule): no Auto Layout (constraints, priorities, `layoutSubtreeIfNeeded`
passes); layout is a top-down `layout(ui, bounds)` with `Stack` and `intrinsic`, and no content hugging or compression
resistance. No layer-backed views (animation: see "Animation (Core Animation parity)"), so no `wantsLayer`, shadows, masks or `NSAnimationContext`

Apple parity gaps (recorded per the standing rule): Auto Layout is opt-in, see "Auto Layout (constraints)"; the default layout is the top-down
`layout(ui, bounds)` with `Stack` and `intrinsic`. No layer-backed views, so no implicit animation, `wantsLayer`, shadows, masks or `NSAnimationContext`
(use `bifrost_ui::anim` by hand and mark the view dirty each frame). `ScrollView` is vertical only and a view has no
`bounds` origin or transform (scrolling shifts frames while drawing). `Surface` still has no clip rectangle: `ScrollView`
confines its children to its frame by rows (`Surface::rows`) and relies on the document being as wide as the view, so
horizontal scrolling and horizontal clipping are not done. Scroll wheel and trackpad scrolling are not available: the proto's
input is only `TouchKind` Down, Move, Up and Cancel, so a `ToApp` wheel variant is needed (also for hover and tracking
areas, which stay missing). `ViewEvent` has no timestamp, so drag velocity uses the frame time of the last `Window::tick`
(a fling needs ticks between the Moves). A document child that handles Down (a button) takes the grab, so a drag that
starts on it does not pan. No tracking areas or hover, no `nextKeyView` overrides (the loop is geometric), no
drag and drop, no `NSTextView`-style first responder for text (use `text_field` inside a custom view), and the
accessibility `Node` is flat, so nesting is reported only by `ax_tree()`.

### Framework views: security panels, `SceneView`, `SpriteView` (wiring wave 14)

Three groups of retained views host models that existed before only as tested logic. All are in `bifrost_app::view`; each
sends action strings through the responder chain and reads back through `Window::view`.

- **Security panels** (`view/security_views.rs`, over `framework_kits::security_interface`): `CertificatePanelView` shows the
  chain leaf first with a validity dot and word on each row, a Details disclosure and the selected certificate's subject,
  issuer, serial number and status; `TrustEditorView` shows one Default, Always or Never control for each certificate, the
  verdict of the chain and Reset to Defaults; `IdentityChooserView` lists the identities that passed the policy with Choose
  and Cancel. The shell does not use these views (it cannot depend on the SDK); it draws its own copy of the certificate
  panel in `shell/trust_sheet.rs` when the Browser's TLS client refuses a certificate. `deildu::http::Tls` wraps the webpki
  verifier, keeps the chain it refused (`take_rejection`), and the Browser takes it when the page fails. The sheet offers
  Cancel and Show Certificate and never loads the page anyway: there is no per-site trust exception yet, so the
  trust editor is an app view only.
- **`SceneWidget`** (`view/scene_widget.rs`, `SceneView`): renders a `scene3d::Scene` through its camera with the CPU
  rasteriser (`raster3d`), at most 640 by 480 pixels, only when the scene, light, camera or size changed. A
  `SceneDrawable` ties a node to `primitives3d` geometry and a material. Drag orbits (never past the poles), `+`, `-` and
  `scroll()` zoom, arrow keys orbit, and a click picks the nearest bounding sphere (`ACTION_PICK`). Gaps: no wheel event
  reaches views (see above), triangles that cross the near plane are dropped, picking is per sphere, no textures.
- **`SpriteWidget`** (`view/sprite_widget.rs`, `SpriteView`): hosts a `sk_node::NodeTree` with `Scene2d` scaling and a
  `SceneLoop`. `NodeTree::draw_quads` gives each node's rotated box; rectangles and images draw rotated, circles as discs,
  labels upright. `on_step` receives every frame phase, node actions advance in the loop and their reports come from
  `take_reports()`, and a click picks the topmost node by its own shape (`ACTION_TOUCH`). Gaps: shader nodes, rotated
  labels, the physics world (the app steps `physics` in `on_step`), and per-node colour lives in the view, not the model.

## Animation (Core Animation parity)

Code: `springboard/sdk/src/view/animation.rs`, curves and springs from `springboard/ui/src/anim.rs`.

- `ViewBase` gained `opacity` (`alphaValue`, default 1) and `background: Option<u32>` (premultiplied `layer.backgroundColor`, painted by the window under the view). With `frame` these are the animatable properties (`Prop`: X, Y, Width, Height, Opacity, Background).
- `Window::animate(Timing, |w| ...)` (`UIView.animate`, `CATransaction`): the window diffs frame, opacity and background of every view before and after the closure and interpolates each change from the value on screen. Anything that sets them animates, `layout` included. `Timing::DEFAULT` is 0.25 s ease-in-out (the `CATransaction` default); `Timing::new(secs, Curve)`; `Timing::spring(damping_ratio, response)` (`CASpringAnimation`, lasts until settled).
- `Window::transaction(Transaction::new(timing).completion(|finished| ..).action("name"), |w| ..)` (`NSAnimationContext.runAnimationGroup`): the completion runs once after the last track, `finished` false when it was replaced or interrupted. The action is queued only when finished; `ViewApp::draw_at` sends it down the responder chain (`flush_animation_actions`). A transaction that changes nothing completes on the next tick.
- `Window::disable_actions(|w| ..)` (`setDisableActions`) applies at once and cancels running tracks of the touched properties; inside an `animate` the outer transaction does not animate those changes.
- Retargeting: a new change on a running property starts from the value shown now (no jump). Setting a property outside any transaction while it animates cancels the track and takes the value.
- Clock: tracks start at the first `Window::tick` after creation, so an animation started in an event handler never begins in the past. `Window::next_wake(t_seconds)` returns `Some(t + 1/60)` while anything animates and `None` when idle; `ViewApp::next_wake(t_ms)` does the same in milliseconds for `App::next_wake`. `Window::is_animating`, `target_frame(id)` (model value) and `animation_damage()` (union of old and new rects since the last draw) help apps and tests.
- Damage: opacity and background only dirty the view; a moving or resizing frame schedules a full repaint, because `Surface` has no clip and the vacated area must be cleared (cost: one window repaint per frame while a frame animates).
- Opacity below 1 is group opacity: the window saves the pixels under the frame, draws the view and its subtree normally, then blends back (`scale(saved, 255-a) + scale(drawn, a)`). Cost per faded view: one copy and one blend of its frame, no full layer. It assumes the view paints opaquely over its backdrop. Opacity 0 skips drawing.
- Reduce Motion (`appearance::reduce_motion`, override with `Window::set_reduce_motion`): frame changes are instant, opacity and background cross-fade over 0.12 s.
- Units: `Window::tick`, `Timing` and `next_wake` are seconds; `ViewApp::next_wake` is milliseconds like `App`.

Key-path animations (below) cover the animation objects; `Window::animate` keeps the typed frame/opacity/background diff, and a retargeted spring now carries its velocity. Gaps, decided: no `CALayer` tree, transforms, corner radius, shadow or masks (views have no layers); reading `frame` mid-flight returns the presentation value, use `target_frame` for the model value; hit testing uses the presentation frame and ignores opacity; overlays (`draw_overlay`) ignore opacity; no `CADisplayLink` (the window ticks per frame while `next_wake` is `Some`); the background cross-fades in premultiplied RGBA, not a perceptual space; colour is a plain `u32`, not a `Pair`, so a Light/Dark switch mid-animation does not re-resolve it.

### Key-path animations

Code: `springboard/ui/src/{keypath_anim,anim_value,anim_transaction}.rs` (engine, pure), `springboard/sdk/src/view/keyanim.rs` (views). Details and the row-by-row status are in `docs/parity/ca-animation.md`.

- `KeyAnimation` (basic from/to/by, keyframes with key times, timing functions and calculation modes, path animations with rotation mode, springs with mass, stiffness, damping and initial velocity, groups) on string key paths; `AnimValue` is scalar, point, size, rect, colour or transform; sub-paths such as `position.x`, `frame.size.width`, `transform.rotation.z` resolve through the compound value.
- `Window::add_animation(view, anim, key)`, `remove_animation`, `animation_keys`, `presentation_value` and `model_value`. Animations never change the model; `AnimationDelegate` hears start and stop(finished).
- Transactions nest: `begin_transaction`, `commit_transaction`, `flush_transactions`, `set_animation_duration`, `set_animation_timing_function`, `set_transaction_disable_actions`, `set_transaction_completion`, `set_transaction_value`. `Window::animate` and `disable_actions` open the same scopes, and `set_value_animated(view, key, value)` animates under them (a running spring on the key is retargeted with its velocity).
- Gap: do not animate one property with both `animate` diffs and key animations.

## Auto Layout (constraints)

Apple reference: `NSLayoutConstraint`, `NSLayoutAnchor`, `NSLayoutGuide`, `intrinsicContentSize`, content hugging and
compression resistance, `NSStackView.Distribution`. Two layers:

- `bifrost_ui::constraint` (pure, no dependencies): an incremental Cassowary solver. `Solver::var()`, linear `Expr`
  (`+`, `-`, `* f64`), `Solver::add(expr, Rel::{Le,Eq,Ge}, Strength) -> ConstraintId` meaning `expr rel 0`,
  `remove(id)`, `add_edit(var, strength)` + `suggest(var, value)` (incremental re-solve, the window resize path),
  `value(var)`. `Strength::{REQUIRED, STRONG, MEDIUM, WEAK}` or `Strength::from_priority(1..=1000)` (`UILayoutPriority`;
  1000 is required, 999 and below always lose to required). A weighted sum of violations is minimised, so a higher
  priority wins a conflict. A required constraint that conflicts with earlier required ones returns
  `SolverError::Unsatisfiable(id)` and leaves the solver unchanged.
- SDK `bifrost_app::view` (sdk/src/view/autolayout.rs): `anchors(view_id).leading()/trailing()/top()/bottom()/width()/
  height()/center_x()/center_y()` give an `Attr`; `Spec::eq/le/ge(a, b).multiplier(m).constant(c).priority(p)` is
  `a rel m * b + c`, `Spec::fixed(a, rel, c)` has no second attribute. `Constraints` collects them with helpers `fill`,
  `center`, `size`, `equal_widths`, `chain_h`, `chain_v`, `guide()` (an `NSLayoutGuide`), `hugging(id, axis, p)` and
  `compression(id, axis, p)`. `Window::constrain(cs)` adds them, `remove_constraints()` clears, and `Window::layout`
  solves after the normal top-down pass. Built-in guides: `Item::Window`, `Item::SafeArea` (below
  `layout::top_inset()`), `Item::Margins` (inset by `prefs_layout::m::DETAIL_MARGIN`).
- Intrinsic size: a constrained view adds `w <= intrinsic` at its hugging priority (`View::hugging(axis)`, default 250)
  and `w >= intrinsic` at its compression resistance (`View::compression(axis)`, default 750), per axis, when the
  intrinsic length is above 0. Its current frame is a priority-1 preference, so an axis nothing constrains stays where
  the manual layout put it. Views no constraint mentions are untouched.
- `Stack` gains `distribution` (`Fill`, `FillEqually`, `FillProportionally`, `EqualSpacing`, `EqualCentering`; pure
  `bifrost_ui::stack::distribute`) and `Align::Trailing` (`TOP` and `BOTTOM` for horizontal stacks). In `Fill`, when every
  child has an intrinsic length, the children with a strictly lower hugging priority than the rest stretch (ties keep
  intrinsic sizes).
- Bench: `bench_ui_constraint_solve_200_views`, `bench_ui_constraint_resize_x100` (`tools/bench.py`).

Gaps (recorded per the Apple parity rule): no visual format language (`constraints(withVisualFormat:)`), no ambiguity
debugging (`hasAmbiguousLayout`, `exerciseAmbiguityInLayout`, the constraint-breaking log and the debug view UI;
conflicting required constraints are only listed in `Window::broken_constraints`), no `activate`/`isActive` toggling of
single constraints after the fact (rebuild with `remove_constraints` then `constrain`), no `NSLayoutConstraint.constant`
animation, no baseline anchors, no `systemSpacing(after:)`, no `NSStackView` gravity areas, `detachesHiddenViews` or
custom spacing per view, no baseline alignment, no `NSLayoutGuide` owned by a view (guides are window-wide), no
per-view `translatesAutoresizingMaskIntoConstraints` (all frames are window coordinates), and no `updateConstraints` /
`setNeedsLayout` pass: constraints are re-solved on every `Window::layout`, which is also where a resize arrives. The
solver is re-created per layout (an edit-variable resize through `suggest` is available on `Solver` but the window does not
keep a persistent solver yet). `Fill` shrinks stretchable children first and does not rank compression resistance among
fixed children (use constraints for that). Not compiled or run when written; the lead verifies the tests.

## App Intents (`AppIntent`, `AppEntity`, `EntityQuery`, `AppShortcutsProvider`)

SDK module `bifrost_app::intents`, built on the control socket (`App::rpc`), so nothing new crosses the shell protocol.

| Apple | Ours |
|---|---|
| `AppIntent` (`title`, `description`, `@Parameter`) | `Intent::new(id, title, description).param(Param::new(name, title, ParamType::..))`, `.shortcut()` |
| Parameter types | `ParamType::{String, Int, Bool, Date (ISO 8601 text), Url, Entity("Type"), Enum(cases), Double, Currency, Placemark, Measurement, File}`, `Param::optional()`. `People` is `Entity("Person")` |
| `IndexedEntity`, `hideInSpotlight` | `indexed_entity::IndexedEntity::new(entity).keyword(..).hide_in_spotlight()`; `indexed_entity::index(ctx, &[..])` writes Core Search items in the `entity.<type>` domain (`springboard/sdk/src/indexed_entity.rs`) |
| `AppEntity` | `Entity::new(type, id, title).subtitle(..)` |
| Parameter editor model | `bifrost_app::intent_form::IntentForm`: declaration-order fields, typed values, missing-required checks and JSON arguments |
| `EntityQuery`, `suggestedEntities`, `entities(for:)`, `entities(matching:)` | `EntityQuery::{Suggested, Ids, String}` |
| `perform()` returning `IntentResult` (`ProvidesValue` + `ProvidesDialog`) | `IntentResult::value(json).with_dialog("..")` |
| `AppShortcutsProvider` | `[[intent]]` tables in `manifest.toml` with `shortcut = true` |

Wire format (JSON-RPC over the app's control socket): `intent.list`; `intent.<id>` with an object of parameters, reply
`{"value": .., "dialog": ".."}`; `entity.query` with `{"type": "Note", "string": "milk"}` (or `"ids": [..]`, or neither for
suggested), reply a list of `{id, type, title, subtitle?}`. The SDK validates parameters against the declaration (missing
required, wrong type, unknown name give `INVALID_PARAMS`) before the app sees them. Try it:
`bifrost pc control call APP intent.list` and `bifrost pc control call APP intent.new-note '{"text":"milk"}'`.

Adopting it in an app (three steps):

1. Implement `App::intents()` (the declarations), `App::perform_intent(ctx, data_dir, id, params)` (match on `id`; read
   parameters with `intents::str_param` or `params.get`; return `IntentResult`) and, for entity parameters,
   `App::entities(data_dir, type, &EntityQuery)`. Data-only intents can work from `data_dir` without a `Ctx`.
2. Copy `Intent::manifest_toml()` of each intent that should appear in Spotlight into `manifest.toml` (a unit test that
   compares `intents()` with the parsed manifest keeps them in step). Keys: `id`, `title`, `description`, `shortcut`,
   `params = ["name:type", "other:date?"]` (`?` marks optional; `entity(Type)` for entities). An `[[intent]]` table ends at
   the next `[...]` header, so put app-level keys above it.
3. Add the intent to the app's changelog and its `rpc` documentation.

The shell parses `[[intent]]` (`apphost::Manifest::intents`, `shell/intents.rs`). Spotlight's "Shortcuts" section lists
intents with `shortcut = true` and no required parameter, matched on title and app name without launching the app.
Choosing one opens the app, then calls `intent.<id>` on `$XDG_RUNTIME_DIR/bifrost/apps/<app id>.sock`, retrying for up to
8 s while the app starts.

Gaps (Apple behaviour not matched yet):

- No Siri or voice: `dialog` is returned to the caller but Spotlight neither shows nor speaks it (it is logged on failure).
- No parameter resolution UI: Spotlight runs only parameterless shortcuts. Required parameters, disambiguation
  (`needsValueError`, `requestDisambiguation`) and confirmation (`requestConfirmation`) are not modelled; callers pass JSON.
- `entity.query` is for callers and scripts; nothing in the shell shows entity pickers yet. `OpenIntent` as a type of its
  own and `ParameterSummary` are not implemented (`IntentResult::opens` covers the follow-on case).
- Present in the SDK but not wired to the shell: `Intent::foreground` and `Intent::authentication` (checked by
  `Intent::may_run` and `check_result`; the shell does not gate runs on the session), `IntentResult::with_snippet` (the
  shell does not draw the snippet), `donation` and `donations` (stores; Search does not read them), `focus_filter`
  (client of the shell's Focus service; the Focus name is the active Focus mode's name; there is no `SetFocusFilterIntent` model).
- Parameter prompts: `IntentForm` is only a typed data model; `controls::intent_resolve` plans ask, choose, confirm and
  block (`Plan`), but the control socket replies with a plain error and the shell does not draw a form or prompt for a value.
- Not started: intents driving widgets, controls and Live Activities.
- Shortcuts lists and runs its own intents only; intents of other apps (`intent.list` over their sockets) are not listed.
- The Shortcuts app (`shortcuts-desktop`) does not list or run intents yet; it could read `intent.list` of running apps and
  the manifests of installed ones and offer them as actions, passing results between steps as the `value` JSON.
- Intents are served on the control socket, so the app must be running; the shell launches it first.
- `Date` is validated as ISO 8601 text only; no natural-language dates or durations. Plural parameters (`[IntentFile]`,
  `Doubles`, `[Placemark]`) are not modelled; `Placemark`, `Measurement` and `File` take one object each.
drag and drop, no multi-line text view, and the
accessibility `Node` is flat, so nesting is reported by `ax_tree()` and sent as `NodeExt` parents.

## App services (Services menu, `NSServices`, `bifrost_ui::app_services`)

Apps add entries to the App menu's Services submenu. Each entry is a `[[service]]` table in `manifest.toml`:

```toml
[[service]]
title = "Open Images in Viewer"
send_types = ["public.image", "public.pdf"]
return_types = []
method = "open_images"
```

- `title` is the menu row; `send_types` are the types of the selection the service takes (any one matches);
  `return_types` is non-empty when the service answers with a text value; `method` names the control call
  `service.<method>` (`[a-z0-9_-]`).
- The selection's types come from the pasteboard (`bifrost_ui::app_services::selection_types`): every file selection
  carries `public.file-url`, plus `public.image` and `public.pdf` when a file has that extension; text carries
  `public.utf8-plain-text`, plus `bifrost.file-path-text` (the shell's own type, not an Apple UTI) when the text is one
  absolute path to a file. The App menu lists the built-in services, then every installed app's matching service in
  install order (`services_route::app_services_for`).
- Choosing a row opens the provider (or activates it) and sends `service.<method>` over its control socket:
  `{"type": <uti>, "text": ...}` for text, `{"type": <uti>, "paths": [...]}` for files and path text. The app answers
  with `ServiceCall::from_params` (SDK `bifrost_app::services`); a `return_types` service answers
  `{"value": text}` (`value_result`), and the shell puts that text on the pasteboard and the clipboard.
- Apps implement `rpc_methods` and `rpc` as for any control method. Viewer opens the first picture or PDF of the
  selection (`service.open_images`). TextEdit adds New Window Containing Selection (`service.new_window_with_text`) and
  Open Selected File (`service.open_selected_file`); the latter opens a named file as an untitled document, since the
  document controller only opens files through a panel grant.
- Decisions and gaps:
  - The shell reads the selection from the pasteboard, which apps fill on copy and selection. It does not ask the
    frontmost app for its selection types over the control socket: that call would block the menu on the UI thread.
  - The frontmost app's own services are listed too (the reference model lists every provider).
  - The returned text is put on the pasteboard; the shell does not type it into the frontmost app.
  - A service whose app is not running gets the call after the app opens (retried for 8 seconds).
  - Not verified on Midget yet: the rows, the open, and the return path.
- Tests: `app_services` (manifest keys, matching, payloads, the request line, reply parsing) and
  `apphost::service_manifest_tests`.

## Assistant (`springboard/apps/assistant-desktop`)

A chat client for the Anthropic Messages API, built on the kit only (`source_list`, `toolbar`, system `Pair` colours with `.now()`).

- Streaming: `POST https://api.anthropic.com/v1/messages` with `stream: true` on a background thread, through `bifrost_app::url::URLSession::bytes` (rustls + graviola underneath). `api::SseParser` reads `content_block_delta` / `text_delta`, `error` and `message_stop`.
- The API key is kept with the SDK keychain (`bifrost_app::keychain`, permission `keychain`); conversations and the model are JSON in the data dir.
- Apple parity gaps, decided here: the composer is an end-of-text editor (no caret movement, selection or drag; `TextArea` is built around notes layout); there is no monospace face in the shell, so code blocks use the system font on a secondary-fill plate; Shift+Return depends on the shell delivering a newline as `KeyInput::Text`, because `KeyInput::Code(13)` carries no modifiers.

## URLSession (`bifrost_app::url`)

Code `springboard/sdk/src/url.rs`, contract test `sdk/tests/url.rs` (local `TcpListener` server). Transport is the shell's
rustls/graviola stack (`springboard::deildu::http::Tls`); plain `http://` works too. Work runs on a `UserInitiated`
global queue; completions run on `DispatchQueue::main`.

| Apple | Here |
|---|---|
| `URLSession.shared` | `URLSession::shared()` (16 MiB memory `URLCache`) |
| `URLRequest` (`url`, `httpMethod`, headers, `httpBody`, `timeoutInterval`) | `URLRequest { url, method, headers, body, timeout }`, `new`, `post`, `set_header` |
| `dataTask(with:completionHandler:)` | `data_task(req, completion)`, completion gets `Result<(HTTPURLResponse, Vec<u8>), URLError>`; non-2xx is a response, not an error |
| `downloadTask(with:)` + progress | `download_task(req, dest, progress, completion)`: writes `dest.part`, renames on 2xx |
| `URLSession.bytes(for:)` | `bytes(req, cancel, on_response, on_chunk)`: blocking, streaming (the assistant's SSE) |
| `URLSessionTask.cancel()` | `URLSessionTask::cancel()` |
| `uploadTask(with:from:completionHandler:)` | `upload_task(req, body, progress, completion)`: `Content-Length` from the body; `progress(bytes_in_chunk, total_sent, total_expected)` is `didSendBodyData` and runs on the main queue |
| `uploadTask(with:fromFile:completionHandler:)` | `upload_task_from_file(req, path, progress, completion)`: streams the file in 64 KiB chunks; a missing file completes with `FileError` and sends no request |
| `URLSessionTask.countOfBytesSent` / `countOfBytesExpectedToSend` | `count_of_bytes_sent()` (u64) and `count_of_bytes_expected_to_send()` (i64, -1 until known, like `NSURLSessionTransferSizeUnknown`); cancel stops between 64 KiB chunks with `Cancelled` |
| `URLCache` | `URLCache::new(capacity)`: GET 200 by URL; `Cache-Control: max-age` serves without a request, `ETag` revalidates with `If-None-Match` and a 304 serves the cached body; `no-store` and `no-cache` respected |

Gaps, decided here: no `URLSessionConfiguration`, delegates (progress comes in as closures instead), background sessions
or HTTP/2; no disk cache (`URLCache` is memory only); no cookie storage or `URLCredential`s (the Deildu `Client` keeps
its own jar); no `Vary`, `Expires` or `Age` handling; one connection per request (`Connection: close`); `URLError` has
the few codes used here. Upload progress is the same `didSendBodyData` shape as the delegate. A 307/308 redirect of an
upload re-sends the body, so its counts keep growing past the expected length: a known limit. The assistant app moved
onto `URLSession::bytes`.
### Background sessions and download resume (`bifrost_app::url_background`)

Code `springboard/sdk/src/url_background.rs`. Downloads that survive app quit; tasks are saved to `<dir>/<identifier>.tasks`.

| Apple | Here |
|---|---|
| `URLSessionConfiguration.background(withIdentifier:)`, `isDiscretionary`, `sessionSendsLaunchEvents` | `BackgroundConfiguration::background(id)` and its fields |
| `NSURLSessionDownloadTaskResumeData` | `URL_SESSION_DOWNLOAD_TASK_RESUME_DATA`, `ResumeData { url, etag, bytes_received }` `encode`/`decode` |
| `downloadTask(with:)` / `downloadTask(withResumeData:)` | `BackgroundSession::download_task`, `download_task_with_resume_data` (a resumed run sends `Range` and `If-Range`; a `200` restarts from zero) |
| `cancel(byProducingResumeData:)` / `cancel()` | `cancel_by_producing_resume_data(id)` / `cancel(id)` |
| `getTasksWithCompletionHandler` | `BackgroundSession::open` restores by identifier; `pending_tasks()` |
| `URLSessionDownloadDelegate` | `DownloadDelegate::{did_write_data, did_finish_downloading_to, did_complete}`, driven by `run(id, delegate)` |

Gaps, decided here: no `URLSessionConfiguration` (other than background), delegates (other than downloads), app relaunch events, upload tasks or HTTP/2; no disk cache
(`URLCache` is memory only); no cookie storage or `URLCredential`s (the Deildu `Client` keeps its own jar); no
`Vary`, `Expires` or `Age` handling; one connection per request (`Connection: close`); `URLError` has the few codes
used here. The assistant app moved onto `URLSession::bytes`.
### URLSession completion (2026-10-13)

Host tests only (`sdk/tests/url_session.rs`, loopback servers); a row becomes `parity` after a Midget check.

| Apple | Here |
|---|---|
| `URLSessionConfiguration.default / .ephemeral / .background(withIdentifier:)` | `url_background::URLSessionConfiguration::{default_configuration, ephemeral, background}`, `identifier()`; fields for cache policy, timeout, cookie, credential and cache stores |
| `URLSession(configuration:delegate:)` | `ConfiguredSession::new(config, session, delegate)`, blocking `data` / `data_at(req, now)` |
| `urlSession(_:didReceive:completionHandler:)`, `URLAuthenticationChallenge` | `URLSessionTaskDelegate::did_receive(&AuthChallenge) -> AuthDisposition`; a 401 Basic challenge is retried once, the credential is stored per `URLProtectionSpace` unless its persistence is `None` |
| `HTTPCookieStorage` in a session | cookies attached and stored per request (`http_should_set_cookies`), accept policy honoured |
| `URLCache`, `CachedURLResponse`, `URLRequest.CachePolicy` | `url_cache::{URLCache, CachedURLResponse, CachePolicy}`: directory store, memory/disk capacity, `Cache-Control`/`ETag`/304, oldest-first eviction |
| `URLSessionTaskMetrics` | `ConfiguredSession::last_metrics()`: per-transaction url, status, duration, bytes, `FetchType` |
| background session | `BackgroundSession::open(id, dir)`, `enqueue_download`, `run_pending`: the queue is a file, so a rebuilt session resumes unfinished downloads |
| `URLSessionWebSocketTask` | `websocket::URLSessionWebSocketTask`: `connect`, `send`, `receive`, `send_ping`, `cancel(code, reason)` over `ws://` |
| `URLSessionStreamTask` | `url_stream_task::URLSessionStreamTask`: `connect`, `write`, `write_line`, `read_data`, `read_line`, `close_write` |
| `NetService`, `NetServiceBrowser` | `net_service`: publish/stop, TXT, browser with `did_find` / `did_remove`, over `dns_service` |

Decided gaps: Digest, client-certificate and server-trust challenges; `AcceptPolicy.onlyFromMainDocumentDomain`; `wss://`
and secure stream upgrade; upload tasks and OS-level background transfer; an mDNS responder (the browser is fed by a
transport); no `Vary`, `Expires` or `Age` handling; `URLError` has the few codes used here. The shared session's
`url::URLCache` stays memory only.

### Networking and privacy (2026-10-23)

Rule: a request to a third-party service carries nothing that tells one person's requests apart. Both HTTP stacks
are privacy-first by default, and a caller opts in only where a sign-in genuinely needs it.

- **No cookies by default.** `deildu::http::Client::new` keeps no cookies and sends none (`with_cookies()` opts in,
  `keeps_cookies()` says which). `URLSession` never adds a `Cookie` header on its own. A `ConfiguredSession` sends
  cookies from its configuration's `http_cookie_storage` when `http_should_set_cookies` is on (the reference default);
  no app builds one today, and an app that does for a third-party service turns `http_should_set_cookies` off.
- **Generic `User-Agent`.** Both send `Mozilla/5.0` (`deildu::http::USER_AGENT`, `url::DEFAULT_USER_AGENT`): no
  product, version or device. A request that names its own `User-Agent` replaces it; `Client::user_agent(..)` sets
  another default for one client.
- **Who opts in, and why.**
  | Caller | Cookies | User-Agent |
  |---|---|---|
  | Browser page loader (`shell/browser/fetch.rs`) | kept (sites sign the person in) | `BROWSER_USER_AGENT` (browser-style, no product name) |
  | Deildu site (`deildu/mod.rs`) | kept (the sign-in is a cookie session, saved to disk) | `BROWSER_USER_AGENT` |
  | Maps tiles, geocoding, routing, transit (`maptiles::net::Live`) | none | the app's identifying agent only for `*.openstreetmap.org` (the tile and Nominatim usage policies require it), generic elsewhere (`net::agent_for`) |
  | Weather forecasts and place search, Podcasts directories (fyyd, gpodder) and feeds, Music radio directory, News pictures (`shell::music::radio::Http`) | none | generic |
  | Stocks and the Weather watchlist (`market_quotes::QuoteHttp`) | none | generic |
  | Radio streams (`media/stream.rs`), the captive-portal probe (`net/captive.rs`) | none | generic |
  | Mail autoconfig, Messages (Matrix, bearer token), DAW AI (API key) | none | generic |
  | First party: App Store and updates (`shell/store/net.rs`), BifrOSt Account (`account_page.rs`, session token sent explicitly from the keychain) | none | generic (a product agent may be set with `user_agent(..)` if the server ever needs the version; nothing does today) |
  | `URLSession` apps (Assistant, Calculator rates, News feeds, Calendar subscriptions, Locate) | none | generic |
- **Apple parity, decided divergence.** `URLSession.shared` on the reference platform uses `HTTPCookieStorage.shared`
  and a `User-Agent` naming the app and the OS version. Ours does neither by default; the parity API is still there
  (`attach_cookies`, `store_cookies`, `ConfiguredSession` with a cookie store, a caller-set `User-Agent`).

## Unified Logging (`os.Logger`, `OSLogStore`, `log` CLI; milestone `docs/milestone-unified-logging.md`, Phase 1)

Code: `springboard/core/src/log.rs` (shared with the shell, which cannot depend on the SDK), re-exported as
`bifrost_app::log`. CLI: `bifrost-log` (`sdk/src/bin/bifrost-log.rs`, packaged to `/usr/bin` by the PKGBUILD).

```rust
let log = bifrost_app::log::Logger::new("is.olibuijr.notes", "sync");
log.notice("synced");
log.error(format!("cannot open {}", bifrost_app::log::private(path.display())));   // stored as "cannot open <private>"
```

Manifest: `[[log.subsystem]] id, categories = [...], level = "notice"`. The installer rebuilds
`/var/lib/bifrost/log/registry.toml`; `/etc/bifrost/log.toml` overrides it (`bifrost-log config --subsystem X --level debug
--private on`). A `Logger` re-checks both files' mtime at most once a second, so a disabled message costs one atomic compare.

Per-argument privacy and formatting (`core/src/log_format.rs`, `OSLogPrivacy`, `OSLogIntegerFormatting`):

```rust
use bifrost_app::log_format::{LogArg, Privacy, Mask, IntFormat, Align};
log.info(format!("user {} id {}", LogArg::string(name).privacy(Privacy::Private(Mask::Hash)).render(),
    LogArg::int(7).format(IntFormat::Hex { include_prefix: true, uppercase: false }).align(Align::Right(6)).render()));
```

`Auto` treats strings as private and numbers as public; `Mask::Hash` stores a stable `<mask.hash: 'base64'>`.

| Apple | BifrOSt |
|---|---|
| `Logger(subsystem:category:)` | `Logger::new(subsystem, category)` |
| `.debug .info .notice .error .fault` | same names; syslog priority 7, 6, 5, 3, 2 |
| `<private>` / `privacy: .public` | `private(v)` is `<private>` unless the subsystem has `private = true` |
| logd | journald (native socket datagram; multi-line values use the binary length form); stderr when the socket is unreachable |
| `log config --subsystem X --mode level:debug` | `bifrost-log config --subsystem X --level debug` |
| `log show --predicate ... --last 1h` / `log stream` | `bifrost-log show|stream` (`--predicate 'subsystem == "x" AND level >= error AND message CONTAINS "y"'`, `--last`, `--limit`) |
| `OSLogStore(scope: .currentProcessIdentifier)` | `LogStore::current_process()` (in-process ring of 2000 entries, works in the sandbox) |
| `OSLogStore.local()` + entitlement | `LogStore::local()`: `kind = "system-app"` and `entitlements = ["log-store"]`; the shell (`Policy::with_entitlements`) then mounts `/var/log/journal` and `/run/log/journal` read-only and sets `BIFROST_ENTITLEMENTS`. Other apps get `PermissionDenied` and have no journal mount. `LogStore::system()` is for unsandboxed tools |
| `OSLogEntryLog` | `LogEntry { date, process, pid, subsystem, category, level, message }`; `Query` (subsystem, category, process, min/exact level, contains, last, limit); `LogStore::entries(&Query)`, `LogStore::stream(&Query, FnMut(LogEntry) -> bool)` (blocking, use a thread) |

Gaps, stated explicitly: no info-in-memory-only tier (info is written when the level allows it); no `OSSignposter`
(Phase 3); no format specifiers or per-argument `.public`/`.private(mask:)` (the whole value is marked); no
`OSLogStore.getEntries(at: position)` or persistent position tokens; the journal reader shells out to `journalctl` and the
predicate language is a subset (`==`, `>=`, `CONTAINS`, joined by `AND`); `private` is per subsystem, not per value.
Apps in the bwrap sandbox reach `/run/systemd/journal/socket` through a read-only bind added to every app (`app_policy.rs`;
the datagram socket is world-writable). Unverified: the app account reading journal files needs `systemd-journal` group
access (`drop_identity` copies the account's supplementary groups); if that is not the case on Midget, Phase 2 adds a shell
broker over the app protocol (reserve ToApp/ToShell tags by the per-feature range rule from HANDOFF round 39 first).

### Subsystem ownership (security, 0.5.7)

Apple ties a log subsystem to the process that writes it. Here the registry records the `owner` of each
subsystem (the id of the package whose manifest declared it; the first package wins, a second one cannot take it
over). Readers (`LogStore`, `bifrost-log`, Console) check journald's `_EXE`, which the kernel sets and the app
cannot change. An entry from a store app (`/var/lib/springboard/apps/<id>/…`) under a subsystem another app owns,
or under `is.olibuijr.springboard`, keeps its message, loses the subsystem and category, and is marked
`[claimed subsystem …]`. System binaries may use any subsystem. `bifrost_core::log::may_log_as` is the rule.
Gap: apps still write the datagram themselves, so the check is on read; Console reads the registry once at launch.

## Background App Refresh (`BGTaskScheduler` parity)

Apps run deferred work while no window is open. Status: wired end to end on the host, not yet verified on Midget
(`docs/parity-matrix/ipados/data/background-tasks.json`, status `partial`).

- Manifest: `[background] tasks = ["com.x.app.refresh"]` (`springboard/src/shell/apphost.rs`, `Manifest.background_tasks`;
  parsed by `shell::services::background::parse_tasks`). Identifiers are ASCII letters, digits, `.`, `-`, `_`, at most 128
  bytes, at most 16 per manifest. Requests for any other identifier are refused.
- SDK (`bifrost_app::background_tasks`, `springboard/sdk/src/background_tasks.rs`): `submit(ctx, &Request)` with
  `Request::refresh(id)` or `Request::processing(id)`, `.earliest(epoch_secs)`, `.network()`, `.external_power()`;
  `cancel`; `completed(ctx, id)` when the task is done; `task_event(service, event, body)` reads the shell's events.
  All of it goes over the `background` system service (`ToShell::ServiceCall`, `ServiceEvent`), so no new wire tags.
- Model (`bifrost_core::background_tasks`, moved from the SDK so the shell can use it): `Request`, `Scheduler` (pending
  queue, due rule for time, network and external power) and the JSON codec.
- Shell (`springboard/src/shell/services/background.rs`): the queue is saved to `background-tasks.txt` under the shell root.
  A run starts when a request is due and `may_start` passes: system switch (`bg_refresh`) on, the app not switched off
  (pref `background.refresh.off`), not in Low Power Mode, no task running for the app, runs left in the budget.
  Limits: refresh 30 s, processing 600 s, warning `taskExpiring` 5 s before the limit. The budget is 8 runs per app per
  UTC day. Power: a machine with no battery counts as on external power, otherwise a mains or USB supply must be
  `online`. Network: any interface other than `lo` with `operstate` up.
- Launch: an app not running is spawned without a window (`AppHost::spawn_with`, no session restore) and gets
  `taskStarted` `{identifier, limit}`; a running app gets the same event. When the task completes or its limit passes,
  the shell ends the app it launched (an app the user had open is left running).
- Settings: General > Background App Refresh has the system switch and an APPS group with a switch per app that declares
  tasks (`login_items_page::refresh_group`, `Action::RefreshToggle`).
- Gaps: the repeating `ActivitySchedule` is not connected; a hidden launch skips the notification and extension hooks;
  an app the user opens during a task is ended when the task completes; the Settings per-app switch state comes from
  the page's model, which is refreshed on touch (not verified); continued processing, background GPU access and the
  development hook are missing.
- Tests (run by the lead): `bifrost-core` `background_tasks` (queue, codec), `shell::services::background` (manifest
  parsing, power and network rules, the gate order, budget and day reset, phase timing with a fake clock, one task per app,
  low power and switches, persistence to disk and broken store lines), `bifrost_app::background_tasks` (event parsing).

## Process assertions (RunningBoard) and the SMB client

`bifrost_app::running_board` (`springboard/sdk/src/running_board.rs`): `take(ctx, reason, limit_secs)` asks the shell to keep the app
running while nothing shows it; `release(ctx, id)`, `held(ctx)`. The model is `bifrost_core::process_assertion` (reason of 1 to 64
characters, 60 s default and 1 h at most, 8 per app, lapses by itself) and the service is `runningboard`
(`springboard/src/shell/services/running_board.rs`); `shell::background::Suspender` reads it on every tick, so an app that holds one is not
stopped (`Event::Assertion`) and a quit app loses them. Gaps: assertion domains and memory or CPU limits per process.

`bifrost_app::smb` (`springboard/sdk/src/smb.rs`, SMBClient parity): `SmbUrl::parse("smb://[domain;][user@]host[:port][/share/...]")` refuses a
password in the address; `list_shares`, `list_dir` and `get` run Samba's `smbclient` with the password in `PASSWD`, the session held to SMB 2.02
or newer with encryption when offered, names checked against `;`, quotes, wildcards and `..`, and `NT_STATUS_*` mapped to `Error`. Gaps: the
Files Connect to Server sheet still lists `ftp://` only (no password prompt there yet) and shares are not mounted into the file tree.

## File system events (`FSEventStream`, `NSFilePresenter` gap)

`bifrost_app::fswatch::WatchStream::new(paths, latency, flags)` (Apple's `FSEventStream`) watches folders recursively over inotify (new
subfolders are added as they appear) with `start()` / `stop()`; drop stops. Events are `WatchEvent { path, flags }` with
`WatchEventFlags::CREATED | REMOVED | RENAMED | MODIFIED | ITEM_IS_DIR | MUST_SCAN_SUB_DIRS`, coalesced per path over `latency`.
There is no thread and no timer: return `stream.fd()` from `App::wake_fd`, call `stream.poll(Instant::now())` from
`tick` (events arrive on the main thread) and `next_deadline()` from `next_wake` while a batch matures.
`NO_DEFER` delivers the first event after a quiet period immediately; `FILE_EVENTS` is accepted (events are always
per file). Queue overflow reports `MUST_SCAN_SUB_DIRS` on the roots. `EventJournal` (a bounded ring of numbered events) answers `replay_since(id)` for `sinceWhen` replays, with `SINCE_NOW` for live-only streams. Gaps: no
rename pairing (both halves are `RENAMED`), and no `NSFilePresenter` coordination yet. Files uses it (its own
inotify FFI is gone); Notes, DAW and Music still poll and should migrate.

`WatchStreamFlags::SHALLOW` is a BifrOSt extension with no Apple counterpart: it watches only the roots, not their
subfolders. Apple's FSEvents is recursive for free because the kernel journals every volume; over inotify a recursive
stream walks the whole tree when it starts, so Files watches the open folder with `SHALLOW` (Computer `/` and the home
folder open without a full-tree walk). Test: `fswatch::tests::shallow_watches_only_the_roots`.

## Dispatch (GCD parity)

`bifrost_app::dispatch` is Grand Central Dispatch: one lazily started process-wide pool (per-QoS lanes; all lanes together run at most
`available_parallelism` jobs, higher QoS first, a lane borrowing the width higher lanes leave idle, Background always
keeping one slot when it has work), threads named `dispatch-<qos>`, idle workers blocked on a `Condvar` (they exit after 30 s
idle; nothing spins). Utility and Background threads call `springboard::gfx::affinity::background_thread()` (nice +10).
Closures are `FnOnce() + Send + 'static`; a panic is caught and logged.

| Apple | BifrOSt |
|---|---|
| `DispatchQueue.main` | `DispatchQueue::main()`: runs on the app loop thread. `async_` pushes and writes an eventfd; the runtime loop polls it, drains the queue before `tick` and draw, and marks a frame needed |
| `DispatchQueue.global(qos:)` | `DispatchQueue::global(Qos)`, concurrent |
| `DispatchQueue(label:)` | `DispatchQueue::serial(label)` / `serial_qos(label, qos)`, FIFO |
| `async` / `asyncAfter` / `sync` | `async_(f)` / `async_after(Duration, f)` / `sync(f) -> T` (concurrent queues run it on the caller; `main().sync` from the main thread panics; a serial queue syncing onto itself deadlocks, as in GCD) |
| `DispatchGroup` | `DispatchGroup::{enter, leave, notify(&queue, f), wait(Option<Duration>) -> bool}`, `queue.async_group(&g, f)` |
| `DispatchWorkItem` | `DispatchWorkItem::{new, cancel, is_cancelled, perform, wait}`, `queue.async_item(&item)`; cancel stops an item that has not started |
| `DispatchQoS` | `Qos::{UserInteractive, UserInitiated, Utility, Background}` |

Gaps (stated, not skipped): no `DispatchSource` (timers, fd or signal sources), no barriers (`.barrier`), no
`concurrentPerform`, no target queues, no `DispatchSemaphore` (use `std::sync` or a group), no QoS inheritance. The width cap
is on running jobs; blocked lane threads may exist beyond it (they hold no work). Priority is lane order plus nice; a
running low-QoS job is never preempted.

Updating UI from a worker: the main-queue closure is `FnOnce() + Send`; it gets no `&mut App`. Pattern: the worker
computes, then `DispatchQueue::main().async_(move || *result.lock().unwrap() = Some(v))` into an `Arc<Mutex<_>>` (or an
`mpsc` sender) the app owns; the main-queue wake makes the loop mark a frame, and `App::tick` reads the slot.

Migrating `thread::spawn`:
- Fire-and-forget background work (scans, fetches, photo loads): `DispatchQueue::global(Qos::Utility).async_(f)` replaces
  `thread::spawn` plus a manual `affinity::background_thread()` call.
- Work that must not interleave (one file, one database): a `DispatchQueue::serial("com.app.store")` instead of a thread
  with a channel loop.
- Join patterns (`handle.join()`): a `DispatchGroup` with `async_group` and `wait`.
- Delayed or repeating `thread::sleep` loops: `async_after`, re-armed by the closure; do not park a thread to sleep.
- Keep a long-lived thread only for a blocking loop that owns a descriptor (accept loops, a media decoder).

## Cache (NSCache parity)

`bifrost_app::cache::Cache<K, V>` (code `springboard/render/src/purgeable.rs`): a cost-bounded LRU that gives memory
back under pressure. `V: Clone` (store `Arc<T>` for large values); a hit returns a clone and becomes most recent.

| Apple | Here |
|---|---|
| `NSCache.countLimit` / `totalCostLimit` | `new(count_limit, total_cost_limit)`, `set_count_limit`, `set_total_cost_limit` (0 = unlimited) |
| `setObject(_:forKey:cost:)` / `object(forKey:)` | `set(k, v, cost)` / `get(&k)` |
| `removeObject(forKey:)` / `removeAllObjects()` | `remove(&k)` / `remove_all()` |
| Eviction on memory warning (`NSPurgeableData`, `didReceiveMemoryWarning`) | one lazily started `cache-pressure` thread blocks in `poll()` on a PSI trigger (`/proc/pressure/memory`, `some 150000 2000000`, POLLPRI); on pressure every live cache drops half its cost, oldest first (`purge_all(0.5)`) |

`purge(fraction)` is the hook the watcher calls; `purge_all(fraction)` reaches every registered cache. Without PSI (old
kernel, no permission) no thread is started and the caches are plain LRUs. Moved onto it: the glyph coverage cache
(`render/src/font.rs`, 4 MiB cost) and the emoji image cache (`render/src/emoji.rs`, 256 entries, cost in bytes).

Gaps: no `NSCacheDelegate.willEvictObject`, no `evictsObjectsWithDiscardedContent`/`NSDiscardableContent` begin/end
access counting, no per-entry purge priority; the unprivileged PSI window must be a multiple of 2 s, so the trigger is
2 s, not the 1 s of the spec. `TextCache` (a `&mut` borrow API) and app-local image caches are not moved yet.

## Testing kit (XCTest parity)

`Ctx::testing(ui, cfg)` builds a context with no shell (memory-only defaults); `ctx.take_sent()` returns what the
app queued for the shell (`ToShell` messages), so SDK calls such as `open_url`, `notify` and `set_badge` are tested
directly. `UserDefaults::register([(key, Value)])` matches `register(defaults:)`: fallbacks read when a key has no
stored value, never written to disk; `contains` ignores them, like the persistent domain.

Gap: apps had no way to drive their own UI in a test short of a running shell and Midget. Apple has XCTest and
XCUITest (`XCUIApplication`, `XCUIElement`, `XCTAssert*`). Decision: `bifrost_app::testing` is a thin headless layer over
the existing view `Window`, its responder chain and the accessibility tree (`ax_nodes`, `ax_press`, `ax_set_value`), so a
plain `cargo test` exercises real hit testing, focus and key handling.

| Apple | BifrOSt |
|---|---|
| `XCUIApplication()` | `TestWindow::new(root, model, w, h)` (loads `assets/SFUI.ttf`, scale 1); `.model` is yours to assert on |
| `app.buttons["Save"]` | `win.element(Role::Button, "Save") -> Option<Element>` (role and label match) |
| `element.tap()` | `Element::click()` (centre of the frame through the hit test), `TestWindow::click(x, y)` |
| `AXPress` | `Element::press()` |
| `typeText` / key events | `TestWindow::type_text(&str)`, `TestWindow::key(KeyInput)` (to the first responder) |
| `element.value`, `.frame`, `.exists`, `.isEnabled` | `.value()`, `.frame()`, `.exists()`, `.is_enabled()`; `win.exists(role, label)` |
| setting `AXValue` | `Element::set_value(&str)` |
| `XCTAssertTrue(el.exists)` | `assert_element!(win, Role::Button, "Save")` |
| window resize | `TestWindow::resize(w, h)` |
| `XCTAttachment(screenshot:)` | `TestWindow::draw() -> Vec<u32>` (packed pixels; compare or write out) |

Example: `springboard/sdk/tests/testing.rs`. Gaps, stated explicitly: no `XCTestCase` lifecycle or `measure {}` (use
`#[test]` and `tools/bench.py`); no `XCUIElementQuery` chaining, `descendants` or predicates (first match by role and
label only); no wait-for-existence or `XCTestExpectation` (the window is synchronous; call `Window::tick` for
animations); no gestures beyond click and keys (no drag, scroll or multi-touch); `Element` is a snapshot, so query again
after a change to read a new value; the harness drives one window, not the shell, menus or other processes (use
`bifrost pc control` or the MCP tools for those).

## Attributed strings and rich text (Foundation / AppKit parity)

`bifrost_app::attributed` mirrors `NSAttributedString` / `NSMutableAttributedString` and `NSTextView` rich mode.

- `AttributedString`: merged runs of `Attrs` (bold, italic, `SizeClass`, `ColorToken`, underline, strikethrough, link).
  `attributes_at`, `substring`, `replace_characters` (new text inherits the preceding character's attributes, as typing does),
  `replace_attributed`, `add_attribute`, `remove_attribute`, `enumerate_attribute` (maximal ranges per key), `append`.
- Sizes are the `MAC_*_PT` text styles (`SizeClass`), colours are named tokens resolved with `Pair::now()` (Light and Dark);
  no literal sizes or colours.
- RTF (`to_rtf`/`from_rtf`): `\b \i \ul \strike \fsN`, colour table, hyperlink fields, `\par`, `\'hh`, `\uN` incl. surrogates.
  RTF colour-table bytes preserve RGB order on import/export. HTML (`to_html`/`from_html`): `b strong i em u s del a h1-h3 small br p`,
  colour as `span data-color`, entities.
- `RichTextView`: `RichText` (content, selection, typing attributes) with `insert_text`, `delete_backward`, `toggle(Trait)`,
  `key_command(char, shift)` for Cmd+B/I/U and Cmd+Z / Cmd+Shift+Z, every edit one step on an `UndoManager`. `draw` renders
  the content through the framesetter (`richlayout::TextLayout`) with selection and caret; `layout(ui, width)` gives pixel
  queries (`index_at_px`, `caret_px`, `selection_px`), `index_at_point` and `drag_selection` do mouse hit-testing.
- Extended `Attrs`: font family and size, literal foreground and background colours, baseline offset, superscript, kern,
  tracking, stroke width and colour, underline colour and style, and a paragraph style (alignment, indents, tab stops,
  line break mode, line height multiple and limits, line and paragraph spacing, base direction). `*_utf16` range
  methods take UTF-16 ranges. See `docs/parity/ct-layout.md`.
- Contract tests live in `sdk/src/attributed.rs` (pure model, no shell calls, so no `Ctx::testing`/`take_sent`).

Gaps (not matched yet): the classic range methods count characters (use the `*_utf16` ones for UTF-16); italic is stored and exported but drawn upright (no italic
face in the system font); `Caption` and `Footnote` share 10 pt so Caption reads back as Footnote from RTF; a font family is
stored and laid out but drawn with the system face; attachments, lists and tables are missing; the Notes adoption is
open. The `TextArea` control stays plain text.

## Sudden termination (ProcessInfo parity)

`ctx.enable_sudden_termination()` / `ctx.disable_sudden_termination()` mirror `ProcessInfo`. Counted like Apple's: an app
starts disabled (count 1) and opts in with one `enable`; pairs of `disable`/`enable` bracket work that must finish. The
state goes to the shell (`ToShell::SuddenTermination { enabled }`, tag 0x160) only when it flips. Counter:
`bifrost_app::process_info::SuddenTermination`.

Shell: a suddenly terminable app is SIGKILLed at once on close, logout, shutdown and restart. Any other app gets `Quit`
and 1.5 s (`QUIT_GRACE_MS`) to save and exit, tracked as a pending kill (`Shell::dying`, reaped in `ext_tick`, included
in `next_wake`), so the UI thread never waits. `Shell::stop_apps` does the same for every app at shutdown with one shared
deadline. The decision is the pure `apphost::quit_plan`. First-party opt-ins: Calculator, Console, Weather.

Gap: Apple also disables termination automatically while a document save is in flight; ours does not yet (apps with
documents simply never enable it, so a save is always followed by a Quit round trip).

## Codable (Foundation parity)

`bifrost_app::coding`: `JSONEncoder` (`pretty_printed`, `sorted_keys`, `date_encoding_strategy` seconds or ISO 8601),
`JSONDecoder`, `PropertyListEncoder`/`PropertyListDecoder` (XML plist), traits `Encodable`/`Decodable` over a `Value`
tree, impls for integers, floats, bool, `String`, `Date`, `Option`, `Vec`, `HashMap`/`BTreeMap<String, _>`. Plain structs
use `codable!(Name { field: Type, .. })` (macro_rules, no proc-macro). serde is in the lockfile only through other
crates, so it is not used: the SDK stays lean and offline-buildable. `Option` fields are omitted when `None` and
decode as `None` when missing. The Assistant's `model.rs` uses it. Gaps: no `CodingKeys` renaming (hand-write the impl,
see `Message` in the Assistant), no key/date/data strategies beyond those above, `Data` unsupported. Binary plists:
`PropertyListEncoder::encode_binary` and `PropertyListDecoder::decode_binary` (see "Files and data persistence").
Tests: `sdk/tests/foundation.rs`. `Uuid` and `Decimal` encode as JSON strings.

### UUID and Decimal (Foundation parity)

`value_types::Uuid`: `new_random`, `parse`/`FromStr` (either case), `uuid_string` (upper case), `bytes`, `from_bytes`, `version`, `Ord`/`Hash`.
`decimal::Decimal` (`i128` mantissa and base-10 exponent): `parse`, `from_string(text, &locale)`, `adding`/`subtracting`/`multiplying`/`dividing`
(checked: `DecimalError::Overflow`/`DivideByZero`; `+ - *` operators panic on overflow), `rounded(scale, RoundingMode::{Plain, Down, Up, Bankers})`
(`NSDecimalRound`), `Display` (`description`). 0.1 + 0.2 == 0.3. Division keeps 18 fractional digits. Gaps: no `NSDecimalNumber` object, no `power`, no NaN.

## Files and data persistence (Foundation parity)

All host-tested only (`sdk/tests/files_persistence.rs`); rows in `docs/parity-matrix/data/foundation-files-persistence.json`
stay `partial` until a Midget check.

- `binary_plist`: `BValue` (null, bool, int, real, date as seconds since 2001, data, string, `Uid`, array, dict), `read`,
  `write`. Smallest widths, ASCII or UTF-16 strings, a bounded reader (depth limit, checked offsets and lengths).
  `from_value`/`to_value` bridge to the Codable `Value`.
- `keyed_archive`: `Archive::to_binary` / `from_binary` write the keyed-archive layout (`$archiver`, `$version` 100000, `$top`,
  `$objects` with `$null` first, `$class` uids and `$classname`/`$classes` descriptions). Plain value slots written by other
  writers are read as values. Secure coding: `KeyedUnarchiver::new` requires it; `decode_object_of(archive, owner, classes, key)`
  is `decodeObject(of:forKey:)`; `without_secure_coding()` is `requiresSecureCoding = false`. The archiver name is our own.
- `file_wrapper`: `FileWrapper` regular file, directory, symbolic link. `read_from`, `add_file_wrapper` (unique names
  "name 2.ext"), `add_regular_file`, `remove_file_wrapper`, `matches_contents_of`, `write_to(path, WritingOptions { atomic,
  with_name_updating }, original_contents)`. Atomic writes stage next to the target and swap in with `replace_item_at`;
  unchanged members of `original_contents` are hard linked.
- `bookmark`: `bookmark_data(path, options, keys, relative_to)`, `resolve_bookmark(data, options, relative_to)` with a stale flag
  (inode first in its directory, then ancestors on the same volume), `resource_values_from_bookmark`, security scope
  (`SecurityScope::start_accessing`/`stop_accessing`, read-only scopes) enforced by `ResolvedBookmark::read`/`write`.
- `file_manager`: `replace_item_at(original, with, backup_name, ReplaceOptions)` keeps the original's permissions and xattrs and
  can keep a backup; `enumerator(dir, keys, DirectoryEnumerationOptions)` with `skip_descendants` and prefetched `ResourceValues`;
  `ResourceKey` includes `FileResourceIdentifier`, `IsExcludedFromBackup`, `VolumeAvailableCapacityForImportantUsage` (equal to
  free space: nothing is purgeable yet).
- `file_metadata`: extended attributes (`xattr`, `set_xattr`, `remove_xattr`, `list_xattrs`, `copy_xattrs`) and backup exclusion:
  `set_excluded_from_backup` stores the `user.bifrost.exclude-from-backup` attribute; `backup_items(root)` skips excluded items
  with their contents.
- `metadata_query`: `MetadataQuery` with `predicate`, `search_scopes`, `start_query`, `poll(now)` delivering `DidStartGathering`,
  `GatheringProgress`, `DidFinishGathering`, then `DidUpdate { added, changed, removed }` from the watch stream;
  `MetadataItem::value(MetadataAttribute)`. Separate from the Core Spotlight index (`quicksearch`) and from `ui/src/metaquery.rs`.
- `encrypted_archive`: `bifrost_app::encrypted_archive::{seal, open}` wraps caller-provided archive bytes in the custom
  versioned BFEA format using ChaCha20-Poly1305. The header is authenticated and tests cover corruption and wrong keys.
  This is not Apple's encrypted archive profile; compression and key derivation/storage remain caller responsibilities.

Gaps: no iCloud or ubiquity containers, no file versions, no on-disk purgeable data, no metadata query sort descriptors, grouping or
predicate text parser, no network volumes in bookmarks.

## NotificationCenter (Foundation parity)

`NotificationCenter::default()`: `add_observer(name, object: Option<u64>, f) -> ObserverToken` (drop removes),
`post(name, object, user_info)`. Observers run on the main thread: a main-thread post delivers synchronously, any other
thread queues on `DispatchQueue::main()`. `user_info` is
`HashMap<String, String>` (not `Any`) so posts can cross threads. Tests: `sdk/tests/foundation.rs`.

## DistributedNotificationCenter (Foundation parity)

`DistributedNotificationCenter.add_observer(name, f)` and `post_with(ctx, name, user_info)`. A post is delivered to the
sender's own observers at once and sent as `ToShell::DistPost` (tag 0x1d0); the shell forwards it as
`ToApp::DistNotify` (tag 0x1d1) to every other running app, whose runtime delivers it to observers on the main thread.
Names are a separate namespace from `NotificationCenter`. Apple decision: the broadcast model and string-keyed
`userInfo` match; the shell relays to all running apps and the SDK filters by observer name (no per-name subscription
table). Gaps: `user_info` is string to string, at most 32 entries and 1024 characters a value (a post over the caps is
truncated on encode); no `suspensionBehavior` (an app that is not running misses the post, like `.drop`); no
`deliverImmediately` flag; plain `post` (no ctx) stays process-local. Tests: `notification::tests`, `runtime::tests`
(`distributed_post_goes_to_the_shell_and_local_observers`), proto round trips in `rich.rs`.

## NSFileCoordinator and NSFilePresenter (Foundation parity)

`bifrost_app::file_coordination`: `NSFileCoordinator.coordinate_reading(path, f)` (shared) and
`coordinate_writing(path, f)` (exclusive) run `f` under a bounded advisory `flock` (5 s, then `Timeout`) on the
item's parent directory, so renames cannot slip past the lock. `NSFileCoordinator::add_presenter(Box<dyn FilePresenter>)`
registers a presenter (token drop removes it) with `presented_item_url`, `presented_item_did_change` and
`accommodate_deletion`. The change feed is the app's `fswatch::WatchStream`: pass each `poll()` batch to
`file_coordination::deliver_events`. A path just written through `coordinate_writing` is not reported back to this
process once. Gaps: no relinquish or `savePresentedItemChanges` negotiation, no will-move callbacks (a rename is a
change on both paths), no versions or iCloud coordination, advisory only (uncoordinated processes are not blocked),
presenters only learn of changes inside folders the app's `WatchStream` watches. Tests: `file_coordination::tests`.

## FileManager (Foundation file system, `bifrost_app::file_manager`)

`NativeFileManager` (`springboard/sdk/src/file_manager.rs`) is the SDK's FileManager. Item operations: `item_kind`,
`file_exists`, `attributes_of_item`, `contents_of_directory`, `subpaths_of_directory`, `create_directory`, `copy_item`
(refuses an existing destination), `move_item` (renames, or copies and removes across file systems), `remove_item`
(a link removes only the link). Documents:
- `write_atomically(path, bytes)` is `Data.write(to:options: .atomic)`. It delegates to `bifrost_core::fsutil::write_atomic`
  (temporary sibling, sync, rename). Unlike `fsutil`, it does not create a missing parent directory; it returns
  `NotFound`, as `Data.write` does.
- `replace_item_at(original, with_item, backup_item_name, ReplaceOptions)` is
  `replaceItemAt(_:withItemAt:backupItemName:options:)`. A file replacing a file is one rename; otherwise the old item is
  renamed aside first, the new one moved in, and the old one put back if the move fails. With a backup name the old item
  is kept beside it, and deleted again after success unless `without_deleting_backup_item`; without a name it is deleted.
  `using_new_metadata_only` keeps the replacement's own permissions and xattrs instead of the original's. It returns the
  placed path.
- `url_for(SearchPathDirectory, appropriate_for, create)` is `url(for:in:appropriateFor:create:)`, and `urls_for` is
  `urls(for:in:)`. The directories are `Desktop`, `Downloads`, `Documents`, `ApplicationSupport` (`$XDG_DATA_HOME`
  or `~/.local/share`), `Caches` (`$XDG_CACHE_HOME` or `~/.cache`) and `ItemReplacement`, which needs `appropriate_for`
  and is placed beside that item so the replacement stays a rename. The base is `$HOME`; the XDG values are used only
  when absolute.
- `temporary_directory()` is `temporaryDirectory` (`env::temp_dir()`).
- `enumerator(at, keys, DirectoryEnumerationOptions)` is `enumerator(at:includingPropertiesForKeys:options:)`. The options
  are `skips_hidden_files` (dot names and everything below them) and `skips_subdirectory_descendants`. Items are
  `EnumeratedItem { path, values }` (full path plus the prefetched `ResourceValues`), depth first, names sorted, links never
  entered; the walk is lazy, an unreadable root is an error and an unreadable subdirectory is skipped.
  `skip_descendants()` and `level()` work as on Foundation's enumerator.

Gaps: no `~/Library` folders (Linux XDG locations are used). `FileWrapper` and bookmarks are in their own modules. Tests:
`file_manager::tests` (run with `cargo test -p bifrost-app file_manager`).

## NSFileVersion (Foundation parity)

`bifrost_app::file_version`: `FileVersionStore::new(root)` is the per-app version store, keyed by file identity (device
and inode) so a renamed file keeps its history. `current_version_of_item(url)`, `other_versions_of_item(url)` (newest
first), `add_of_item(url, contents, AddingOptions { discardable })`, `remove_other_versions_of_item(url)` and
`replace_item(&version, url, ReplacingOptions { by_moving })` (keeps the current contents as a version first). A
`FileVersion` exposes `url()`, `modification_date()`, `localized_name()`, `is_resolved()`, `is_discardable()`,
`set_localized_name()` and `remove()`. Copies are atomic (`fsutil::write_atomic`); discardable versions are pruned by
`max_discardable` and `max_discardable_age` on each add. Gaps: no conflict or iCloud versions. Tests: `file_version::tests`.

## Host statistics and ProcessInfo (Foundation/Mach parity)

One implementation, `bifrost_core::host` (re-exported as `bifrost_app::host`), serves the shell and the SDK. Every call
is one `open` + `read` of a procfs file into a buffer the `HostSampler` reuses; there is no sampler thread and no cache:
the caller picks the cadence, as on macOS where `host_statistics64` is a cheap Mach call.

| Apple | BifrOSt | Source and mapping |
|---|---|---|
| `host_statistics64(HOST_CPU_LOAD_INFO)` -> `cpu_ticks[user,system,idle,nice]` | `HostSampler::host_cpu_load_info() -> HostCpuLoadInfo` | `/proc/stat` aggregate. system = system+irq+softirq+steal, idle = idle+iowait |
| `host_processor_info(PROCESSOR_CPU_LOAD_INFO)` | `HostSampler::processor_info()` | the `cpuN` lines of the same read |
| Activity Monitor CPU % | `HostCpuLoadInfo::usage_percent(prev, now)` | busy share of the tick delta |
| `host_statistics64(HOST_VM_INFO64)` -> `vm_statistics64` | `HostSampler::memory() -> MemorySample { vm: VmStatistics64, .. }` | free=MemFree, active=Active, inactive=Inactive, wire=Unevictable+KernelStack+PageTables+SUnreclaim, compressor=Zswap or zram `mm_stat` mem_used_total, external=Cached, pageins/pageouts=vmstat pgpgin/pgpgout (KiB to pages), speculative=0 |
| `vm_kernel_page_size` | `host::vm_kernel_page_size()` | `sysconf(_SC_PAGESIZE)` |
| `task_info(TASK_VM_INFO).phys_footprint` | `HostSampler::task_vm_info()` | `/proc/self/smaps_rollup` `Pss_Anon + Pss_Shmem` (memory the process is charged for, no clean file mappings, like the footprint); falls back to `Private_Dirty` |
| `os_proc_available_memory` | `HostSampler::os_proc_available_memory()` | own cgroup `memory.max - memory.current` when limited (capped by MemAvailable), else MemAvailable |
| `ProcessInfo.physicalMemory / processorCount / activeProcessorCount / systemUptime` | `ProcessInfo::physical_memory() / processor_count() / active_processor_count() / system_uptime()` | meminfo MemTotal, `sysconf`, `/proc/uptime` |
| `ProcessInfo.thermalState` | `ProcessInfo::thermal_state() -> ThermalState` | worst `/sys/class/thermal/thermal_zone*`: `passive` trip = Fair, `hot` = Serious, 5 C under `critical` = Critical; zones with no trips: 75/85/95 C |
| `thermalStateDidChangeNotification` | `ProcessInfo::observe_thermal_state(queue, handler)` | runs on the given queue (pass `DispatchQueue::main()`) |
| `isLowPowerModeEnabled`, `powerStateDidChangeNotification` | `ProcessInfo::is_low_power_mode_enabled()`, `observe_low_power_mode` | the shell writes its Low Power Mode setting to `/run/bifrost/power-state` when it changes |
| `beginActivity(options:reason:)` / `endActivity` | `ctx.begin_activity(ActivityOptions, reason) -> Activity`, `ctx.end_activity(a)` | `ToShell::Activity { options }` (0x161), sent when the union of live options changes. User-initiated or latency-critical: the shell does not suspend the app when it leaves the foreground |
| `DispatchSource.makeMemoryPressureSource` | `DispatchSource::memory_pressure(&queue, handler)` | shared PSI watcher in `render/src/purgeable.rs`: warning = 150 ms "some" stall, critical = 100 ms "full" stall in 2 s; `Normal` after 4 quiet seconds. The `Cache` purge uses the same thread |
| timer leeway / `Timer.tolerance` | `DispatchQueue::async_after_leeway(delay, leeway, f)` | fires in `[delay, delay+leeway]`; the one timer thread wakes at the earliest allowed time of all pending timers and fires every timer already due |

Notifications are not pushed by the kernel: only when someone subscribes, one dispatch timer re-reads every 10 s with 5 s
leeway (so it coalesces with other timers), and stops when nobody is subscribed.

### Menu extras (CPU, RAM)

Both stay enabled by default. The shell reads `host` at a deadline on its main loop (`Shell::metrics_tick`, scheduled by
`next_wake`): every 2 s with 10 % tolerance, 1 s while a CPU or RAM menu is open (Activity Monitor "Very often").
The deadline snaps to the next whole second when that is within the tolerance and joins any wake already planned in its
window (`desktop_metrics::deadline`, `coalesce`). Nothing is read or scheduled while the screen is off, the shell is
locked, the desktop is not active, or no readout or menu is showing; the CPU baseline is dropped on resume. The bar
repaints only when the shown text changes. RAM % is macOS's "Memory Used": (active + wired + compressed) / physical
memory, not total - available (which would count reclaimable cache as free; Linux `Active` still includes some
file-backed pages, so the number runs above macOS on cache-heavy loads).

Tests: parsing fixtures and deltas in `core/src/host.rs`, the schedule in `src/shell/desktop_metrics.rs`, counters and
activities in `sdk/src/process_info.rs`.

Gaps: no per-process energy impact; thermal and Low Power notifications poll every 10 s (not event driven) and the shell
does not yet broadcast them; memory pressure handlers cannot be cancelled; `vm_statistics64` has no purgeable, throttled
or zero-fill counters; `Active` includes file-backed pages (no `Active(anon)` split); activities only prevent
suspension, not CPU throttling or system sleep (`IDLE_SYSTEM_SLEEP_DISABLED` is recorded but not acted on).

### Pinned clock and seed (deterministic snapshots)

Gap: an app that read the wall clock, the system zone, a random seed, the journal or the network drew a different frame
on every run, so Books, Console, Games and Weather carried `KNOWN_GAPS = ["snapshot"]`. XCTest has no single equivalent
(apps inject a `Clock` or seed themselves); decision: the SDK owns the pin.

| API (`bifrost_app::testing`) | Meaning |
|---|---|
| `pin()` | Sets `BIFROST_TEST_PIN=1` for this process and the app binaries it spawns. `tools/templates/app-compliance.rs` calls it before `AppHost::spawn`. |
| `pinned()` | True when the pin is on. Apps branch on it only to swap an external source for a fixture. |
| `unix_secs()` / `system_time()` | The wall clock; `PINNED_UNIX_SECS` (2025-10-10 12:00:00 UTC) when pinned. Use instead of `SystemTime::now()`. |
| `seed()` | A non-zero RNG seed from the clock; `PINNED_SEED` when pinned. Use for xorshift and shuffles. |

Under the pin, Games deals from `PINNED_SEED`, Console shows a fixed three-row journal in UTC with no follow and no
`/var/log` files, Weather uses an offline fetcher, and Books scans its own empty data dir instead of `$HOME`. Monotonic
`now_ms()` (animation time) is not pinned. The four apps' goldens were regenerated once with `BIFROST_BLESS=1` in 0.5.10, and their
`KNOWN_GAPS` lists are empty again.

The pin lives in `bifrost_core::pin` (the SDK re-exports it), so the shell crate honours it too: `clock::Wall::sample`
returns the pinned instant in UTC under the pin, which keeps Calendar's month and today marker fixed (it followed the real
date before and broke its snapshot at midnight). Files reads its first folder at once under the pin, so a busy machine
never snapshots "Loading…".

### NSPasteboard rich types (Apple parity)

`bifrost_app::pasteboard::NSPasteboard::general()` mirrors `NSPasteboard.general`: `clear_contents(ctx) -> change_count`,
`set_data(ctx, bytes, uti)`, `set_string`, `data(uti)`, `string()`, `types()`, `change_count()` and `write_objects(ctx, urls)`
(`public.file-url`). Types are UTType ids (`public.utf8-plain-text`, `public.html`, `public.rtf`, `public.png`,
`public.file-url`); `springboard::shell::pasteboard::mime_for` maps them to MIME (`text/plain;charset=utf-8`, `text/html`,
`text/rtf`, `image/png`, `text/uri-list`). Wire: `ToShell::PasteboardSet` (0x190) and `ToApp::Pasteboard` (0x190), at most
16 items. `Ctx::pasteboard_string` / `set_pasteboard_string` keep working. The shell pushes `ToApp::Pasteboard` (the typed
items) and `ToApp::Clipboard` (the text, which the apps' text fields read) only to the app in front, when it has not seen the
board's change count (`AppHost::sync_pasteboard`, so also when an app comes to the front); every other app gets
`ToApp::PasteboardChanged { change_count }` (0x192) and reads the board as empty until it is in front (the SDK then asks with
`ToShell::PasteboardRead`, 0x193, if the push was lost; `NativePasteboard::request_contents`). The writer is not sent its own
write, text-only writes included. Writes (`SetClipboard`, `PasteboardSet`) are taken from the app in front and for 1.5 s after
it left; an app in the background cannot replace the clipboard. Gap: the contents still reach the app in front without a
paste gesture (old apps expect the push); per-app write rate limits are not enforced beyond one effective write per pass.

**Large items** do not fit the 16 KiB packet and are never cut or dropped silently: they travel in a memfd
(`proto/src/pbblob.rs`, `ToShell::PasteboardBlob` / `ToApp::PasteboardBlob`, tag 0x191, the descriptor as SCM_RIGHTS, the
message holds each item's type and length; 32 MiB per item, 64 MiB per write, at most 16 items). The SDK picks the form: a
`PasteboardSet` or `SetClipboard` that does not fit goes as a blob; one over the limits is not sent, so the clipboard keeps
its contents. The shell does the same for the app in front. Apps built before the blob messages ignore `ToApp::PasteboardBlob`
and `ToApp::PasteboardChanged` (unknown tags are skipped); a shell without them refuses an app that sends one (needs a
matching `min_shell`).

**Concealed copies**: an item that carries the marker type `org.nspasteboard.ConcealedType` (`pasteboard::CONCEALED`,
`NativePasteboard::write_concealed_string`) holds a secret. The shell strips the marker, keeps the text in memory only
(`clipboard::set_text_volatile`, the old `clipboard.txt` is removed), offers it to Wayland programs together with
`x-kde-passwordManagerHint`, and clears pasteboard, clipboard and selection 45 seconds after the copy when nothing else was
copied (`pasteboard::expired`, `Board::pass_at`). `org.nspasteboard.TransientType` is the same without the timer. The Passwords
app copies this way and no longer touches the plain-text clipboard. A selection another program sets with the password-manager
hint is imported as concealed (usable in the shell's fields, kept off disk, cleared on the timer). When the program that
set the selection clears it, and the pasteboard still holds what it set, the pasteboard is cleared too.

**One clipboard** (`src/shell/pasteboard_sync.rs`, run from `Shell::ext_tick`): the shell-wide text clipboard
(`clipboard::set_text`, what the shell's own fields copy), the typed pasteboard and the window server's selection are kept
equal. A copy in a shell field (Settings search, Search, Apps search, file names in Files), an app's `SetClipboard` or
`PasteboardSet`, and a selection another Wayland program sets all end up on the pasteboard, in the apps and in the selection.
The shell owns the selection through `zwlr_data_control_v1` (`pasteboard_wl.rs`, a thread with its own privileged connection;
`wm/src/data_control.rs` offers the global to the shell and to unsandboxed programs of the session's user: a client in another mount namespace, which a sandboxed native app always is, is refused and uses the ordinary focused data device) and
offers text as `text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING`, `TEXT` and `STRING`, beside `text/html`, `text/rtf`
(and `application/rtf`), `image/png`, `image/jpeg` and `text/uri-list`. From another program's selection it reads one text
type and each rich type (32 MiB at most per type and 48 MiB together, all types read at once under one 1.5 s deadline;
selections that follow each other faster than 250 ms are coalesced). The connection is restarted with a backoff (0.5 s to 30 s)
after an error or a panic, and the newest offer is made again on the new connection. Gaps: `STRING` carries UTF-8 (X clients that expect
Latin-1 see the bytes unconverted); the primary (middle-click) selection is not shared; a type with no MIME form (a private
UTType) stays inside BifrOSt; images are not imported into the shell's own text fields.

#### Pasteboard items and named boards

`bifrost_app::pasteboard::NativePasteboard::named(name)` mirrors `NSPasteboard(name:)`; `general()`, `drag()` and `find()` use
the Apple names. A board holds a list of `bifrost_app::pasteboard_item::PasteboardItem` (`set_data`,
`set_string`, `data`, `string`, `types`, `set_data_provider`). Board calls: `write_items(ctx, &[item])` (appends; `clear_contents` first to replace),
`replace_items(ctx, items)` (clear and write as one change step), `items()` (`pasteboardItems`), `read_items(&[uti])` (items with a conforming type), `available_type(&[uti])` (first requested type
the board conforms to, by UTType conformance), `can_read_item(&[uti])`, and `change_count()` (bumped by each `clear_contents`).
The older `set_data`, `set_string`, `data`, `types` work on the first item, so a legacy copy stays a single item. Gap: the shell
wire (`PasteboardSet`) is a flat (type, data) list, so the general board sends its items merged (the first item holding a type
wins, at most 16 items) and promised data nobody has read yet is not sent; named boards are local to the app (cross-app named
boards are `pasteboard_types::Board`).

### Core Transferable and the multi-type pasteboard (Sprint 10)

`bifrost_app::transfer` mirrors Core Transferable. A model type implements `Transferable::transfer_representations()` and
returns a `Representations<Self>` builder; declaration order is the preference order:

- `.data(uti, visibility, export, import)` (`DataRepresentation`), `.codable(JSON, ...)` (`CodableRepresentation`),
  `.proxy(uti, visibility, export)` (export only) and `.proxy_of::<U>(visibility, to_proxy, from_proxy)` (forwards `U`'s
  bytes representations, with the stricter visibility).
- `.file(uti, visibility, export -> SentTransferredFile, import(&ReceivedTransferredFile))` (`FileRepresentation`).
  `SentTransferredFile::new(path).allow_open_in_place(bool)`; `receive(dest_dir)` copies the file (receiver gets
  `is_original_file == false`) unless opening in place was allowed.
- `Visibility::{All, Team, OwnProcess}` against `Viewer::{Other, Team, OwnProcess}`; `types_for`, `exported_content_type`,
  `imported_content_types`, `export`, `export_file`, `import`, `import_file`, `best_import`, `kind_of`.

Bridges: `pasteboard_item` / `from_pasteboard_item`, `drag_item` / `begin_drag` / `drop_values`.

`NativePasteboard` gains `write_transferable(ctx, &value)` (every representation as one item, one write, one change count
step), `declare_types(ctx, types, owner)` (`declareTypes(_:owner:)` with a lazy `PasteboardTypeOwner`, resolved once on first
read; the owner hears `changed_owner` when the board is cleared or another app writes it), `read_objects::<T>()` (one value
per item) and `can_read_object::<T>()`. There is one `PasteboardItem` (`bifrost_app::pasteboard_item`): each type is data or a
promise (`set_data_provider`, `is_pending`, `data_for_type`, `to_item`, `from_item`), served by a
`bifrost_app::pasteboard_promise::PasteboardItemDataProvider` (`provide_data(&self, uti)`, `finished_with_data_provider`).
`bifrost_app::item_provider::ItemProvider` registers data or lazy representations (`registerDataRepresentation`),
`with_transferable(&value)`, and `load_transferable::<T>()` (synchronous; errors are `ItemProviderError`).

Gaps: the shell (`springboard/src/shell/pasteboard.rs`) still publishes one type to the Wayland selection and keeps items
flat (the general board sends its items merged, the first item holding a type wins), so multi-type
copies stay inside BifrOSt until multi-MIME sources land; file representations travel as bytes on a pasteboard item;
`proxy_of` does not forward file representations. Host tests only (`sdk/tests/transferable.rs`); verify on Midget.

## Printing (NSPrintInfo / NSPrintOperation parity)

`bifrost_app::print` (`springboard/ui/src/print.rs`, re-exported by the SDK as `pub use bifrost_ui::print`). Updated in round 75
(2026-10-13): submission moved to CUPS IPP and plug-in panels were added.

- **`PrintInfo`**: paper size, orientation, margins, copies and page range. `paper_size()` and `imageable()` are `paperSize` and
  `imageablePageBounds`. `attributes()` gives the job-template pairs (`media`, `orientation-requested`, `copies`, `page-ranges`).
- **`PrintOperation`** has three sources: `PrintOperation::new(file)` (a PDF, or an image the scheduler converts),
  `PrintOperation::pages(job_name, pages, fonts)` (pages the app built, written as a PDF when the job runs) and
  `PrintOperation::text(job_name, text)` (text laid out to the imageable area by `text_pages`).
- **Chain**: `.with_info(PrintInfo)`, `.with_options(Vec<(key, value)>)` (plug-in panel values, below), `.to(Destination::Printer(name))`
  or `.to(Destination::Pdf(path))`, then `.run()`. `Printer(None)` is the scheduler's default printer. `run` returns the job as
  `printer-id`, or a `PrintError`. `document()` gives the format and bytes without submitting.
- **Submission**: `cups::Client::print_document_with` over IPP (not `lp`). Copies, media, orientation and page range are
  job-template attributes. Margins are not, because the document is already laid out inside them. `Destination::Pdf` is Save as PDF
  and works without a scheduler.
  The client reaches the scheduler through `cups::Endpoint::detect()`: `$CUPS_SERVER` (a socket path or `host[:port]`), else the
  socket `/run/cups/cups.sock` when it exists (what `cups.socket` activation provides), else `localhost:631`.
- **Errors**: `PrintError::NoCups` (the scheduler cannot be reached, any I/O or HTTP failure), `NoPrinter` (no name and no default),
  `Failed(String)`. `print::available()` is a 200 ms connect to the local scheduler port.
- **Printer capabilities** (NSPrinter): `Client::get_printer_attributes(name)` returns `printer_caps::PrinterCapabilities`: `papers` (PWG self-describing names with sizes, `PaperSize::ppd_name()` = PMPaperGetPPDPaperName), `default_paper`, `duplex`, `color_modes`, `status` (state, reasons, message; `statusForTable`), `page_size_for_paper(name)` (`pageSize(forPaper:)`). Printers omitting attributes fall back to Letter/A4, one-sided, auto colour. Not yet done: `deviceDescription`, `isKey(_:inTable:)`, `languageLevel`, the print panel using the lists.
- **Print sheet rows** (wave 15, 2026-10-19): the shell sheet (`springboard/src/shell/print_sheet.rs`) has Printer, Paper, Orientation, Scale, Copies, Pages, Sides (one-sided, two-sided, two-sided short edge) and Colour (colour, black and white). Sides and Colour fill `PrintInfo.sides` and `PrintInfo.color`, so the job carries `sides` and `print-color-mode`; black and white also greys the pages (`to_grey`) so Save as PDF is monochrome.
- **Printers**: `bifrost_app::cups` (`Client::printers()`, with the default flagged). The old `print::printers()` (`lpstat`) is gone.

**Plug-in panels** (`bifrost_app::print_panel`, the accessory views of the print and page setup dialogs):

- `PlugIn::new(id, name).panel(PrintPanel::new(id, title, Placement::PrintDialog | Placement::PageLayout).option(..))`. Options are
  `PanelOption::choice(key, label, choices, default)`, `toggle(key, label, default)` and `number(key, label, min, max, default)`.
- `PlugIns::new()` then `register(plugin)`. Registration refuses duplicate panel ids and option keys, keys outside `a-z A-Z 0-9 - _`,
  and the `RESERVED_KEYS` that `PrintInfo` already writes (`media`, `orientation-requested`, `copies`, `page-ranges`).
- Read and write values by key with `get` and `set(key, Value)`, list panels with `panels(placement)`, and send `job_options()` with
  `PrintOperation::with_options`.
- The shell does not draw either dialog yet. The app draws the panels with its own kit controls.

**Printing a view** (`bifrost_app::print_pages`, host tests only, not yet checked on Midget):

- `PrintableView` has `bounds()` (y down, origin top left), `knows_page_range()`, `rect_for_page(n)` and `draw(rect, &mut dyn DrawOps)`.
  Without a page range, `paginate` cuts the bounds into tiles of the printable area divided by `PrintInfo::scaling()`, down each column
  and then across. With one, each page shows the rect the view returns.
- `DrawOps` is the shared set of drawing calls (fill and stroke rects, paths, text, pictures). `PdfContext` writes them as PDF content:
  `begin_pdf_page` / `end_pdf_page` / `finish` give the pages, and `begin_view_page` maps a y-down view tile onto the printable area.
  Text is Helvetica, font `/F0`; write the pages with the font list `["Helvetica"]`.
- `PrintOperation::print_operation(job_name, &view, info)` is `printOperation(with:printInfo:)`;
  `PrintOperation::pdf_operation(&view, rect, path)` is `pdfOperation(with:inside:to:)` and shows no panel.
  `run_modal(panel, did_run)` asks `panel` to edit the `PrintInfo` when `shows_print_panel` is set, runs the job, and reports through
  `did_run`; a cancelled panel returns `Ok(None)`. `shows_progress_panel` is a flag for the app, nothing draws it.
- `PrintInfo` gained `scaling_factor` (read through `scaling()`, limited to 0.1 to 10.0), `job_disposition` (`Spool`, `Save`, `Preview`)
  and `saving_path`. `Save` writes the PDF to `saving_path`; `Preview` writes it to the temporary directory and returns the path.
- Dialog models (`bifrost_app::print_panel`): `PanelOptions` (which standard controls show) with `apply(&mut PrintInfo, &PanelRequest)`,
  which ignores hidden controls and invalid page ranges and names them in its result, and `PageSetup` (paper, orientation, margins,
  scale; `from_info`, `set_margins`, `set_scaling`, `apply_to`).
- Printer: `PrinterCapabilities::paper_size_in_points(name)` and `imageable_rect(name)` (origin at the lower left, hardware margins
  from `media-*-margin-supported`).
- Bitmaps: `image_source::representation(&Image, FileType::Png | Jpeg, &RepProperties)` encodes a decoded picture
  (`representation(using:properties:)`); `compression_factor` applies to JPEG.
- Not done: the shell-drawn print and page layout sheets, text styles other than Helvetica, clip and transform calls on `DrawOps`.

**Print settings, ranges and presets** (this round, host tests only, not yet checked on Midget):
- `PrintInfo` has `sides` (`Sides`: one-sided, two-sided long or short edge) and `color` (`ColorMode`). Non-default values go
  into the job template as `sides` and `print-color-mode`; the defaults send nothing extra.
- `PaperSize::from_media(name)` reads a paper back from its saved name (`PaperSize::ALL` is the sheet's list).
- `render_pages` draws only the pages of `PrintInfo::pages`, so a view job's PDF holds the range (one page per number,
  none past the last page). `print_operation` then drops the range from the operation's info so CUPS does not cut it again.
- `print_ranges::parse_page_ranges(text, total)`: "1-3, 5, 7-", "-4", "all" or empty. Sorted, merged, last page cut to the
  document, first page past the end refused. `PageSelection::apply_to` writes one range to `PrintInfo::pages` and several
  to `PrintInfo::page_list`; the job sends one `page-ranges` attribute with a value per range (`cups::merge_attributes`).
  Not yet checked on CUPS or Midget.
- `print_presets::PresetStore`: named `Preset`s (paper, orientation, margins, copies, scale, sides, colour). The page range,
  printer and save path stay with the job. `to_text`/`from_text` is the storage form; reading skips bad names and repeated
  sections, ignores unknown keys and keeps defaults for values that do not parse. An app keeps the text in its defaults.
- Not done in this round: the shell print sheet does not yet show the two-sided, colour, presets, PDF popup (Save as PDF,
  Open in Preview, Mail PDF) or Page Setup sheet rows, and TextEdit, Notes and Preview still use their own print calls.
  Mail PDF needs `mail_compose` with the PDF as a grant. These are the next steps before the rows can move.

Decisions and gaps:
- No `run_modal(window)` print sheet yet: the kit print sheet needs a shell-drawn sheet (proto tags 0x1a0-0x1af stay reserved for
  it). Apps choose printer and options with their own controls and call `run`.
- Print Center's Add Printer reads the clipboard, because the SDK has no text prompt sheet yet.
- The `PrintInfo` page range is not used by the shell sheet, which prints only the previewed page when Pages is set. The range
  parser (`print_ranges`) is ready for the sheet's Pages field.
- View printing is limited to Helvetica text, fills, paths and pictures (see below). Pages are drawn when the operation is made.
- Without a scheduler, `available()` is false and `run` returns `PrintError::NoCups`. Whether the Midget image ships CUPS is unchecked.
- Books: File > Print (cmd+P) prints the open PDF to the default printer; EPUB has no print rendering. wordcraft and pdfcraft are
  upstream packages with no source here, so they are not wired.

## Locale and localization (Foundation parity)

`bifrost_app::locale` mirrors Foundation: `Locale::current()` (`LC_ALL`, `LC_MESSAGES`, `LANG`; the shell's region
preference arrives as `BIFROST_REGION` and replaces the region), `NumberFormatter` (decimal, percent, currency, grouping,
fraction digits), `DateFormatter` (`date_style`/`time_style`, `set_date_format`, `set_localized_date_format_from_template`),
`relative_date_time` (`RelativeDateTimeFormatter`), `byte_count` (`ByteCountFormatter`), `Measurement`/`Unit`/`measurement`
(length, mass, temperature) and `list` (`ListFormatter`). `NSLocalizedString` is `localized!("key", "comment")`: `Bundle::load(dir, &locale)`
reads `<app>/<lang>.lproj/Localizable.strings` (comments, escapes) with `en` then the key itself as fallback; call
`set_main_bundle` once at launch. Built-in data covers `en` and `is`; other languages format as `en`. Contract tests live in the module.

Gaps: no other languages, no `.stringsdict` plural rules, no time-zone names beyond English (`tz_offset` is explicit seconds; see "Time zones" below),
Gregorian calendar only, no unit-system auto-conversion in the measurement formatter, no binary `ByteCountFormatter`
style. The shell does not yet publish a region key itself, so `BIFROST_REGION` is read only if set. `bifrost_ui::format`
stays English-only for shell code and its callers are not migrated: the shell has no `Locale` plumbing yet, so moving
them would change nothing visible.

## Time zones (Foundation `TimeZone` parity)

`bifrost_app::time_zone::TimeZone` mirrors Foundation's `TimeZone`. `TimeZone::new(identifier)` (`TimeZone(identifier:)`)
returns None for a name the tz database lacks; `TimeZone::current()` is the system zone, `TimeZone::auto_updating_current()`
reads the system zone on every query, and `TimeZone::from_seconds_from_gmt(s)` is a fixed offset (-18 to +18 hours,
identifier `GMT+0100`). Queries take a `coding::Date` (seconds since 1970): `seconds_from_gmt`, `abbreviation`,
`is_daylight_saving_time`, `next_daylight_saving_time_transition` (the first changing second within four years, None
for a fixed offset). `TimeZone::known_time_zone_identifiers()` lists the database sorted. `localized_name(NameStyle, &Locale)`
returns English names: short forms are the abbreviation in force, long forms come from a built-in table of common
abbreviations (GMT, BST, CET, EST, PST, JST, AEST, NZST, ...) and are None for anything else. The data is the shell's
`springboard/src/bare/timezone.rs` (TZif v1/v2 files, POSIX footer rules, abbreviations); `SYSTEM_TIME_ZONE_DID_CHANGE`
names the zone-change notification, which nothing publishes yet.

Gaps: Gregorian instants only; no names in languages other than English; the World Clock still uses its fixed city table
(`shell/clock/tz.rs`) rather than `TimeZone`; apps have not moved to the SDK type. Tests live in `sdk/src/time_zone.rs`
and need `/usr/share/zoneinfo`. Not yet checked on Midget.

### Preferences domains, time zones, calendars and more formatters (Core Foundation / Foundation parity)

- `bifrost_app::pref_domains::PrefStore` (CFPreferences, `UserDefaults(suiteName:)`): one file per domain under a shared root
  (`open`, `open_host`, `suite`). `copy_app_value` reads the app domain then the global domain; `copy_value` reads one
  domain (`ANY_APPLICATION`, `CURRENT_APPLICATION`, `ANY_USER`/`CURRENT_USER`, `ANY_HOST`/`CURRENT_HOST`); `set_app_value`
  and `set_value` (None removes) stay in memory until `app_synchronize`, which writes and makes later reads reload the
  file, so another process's changes show up. `observe` is `UserDefaults.didChangeNotification` (`DID_CHANGE_NOTIFICATION`)
  and fires only on a real change. Not yet behind `defaults.rs` (`UserDefaults` stays per app).
- `bifrost_app::time_zone::TimeZone` (see "Time zones" above): one type, reading the shell's TZif reader
  (`bare::timezone::Zone`). The calendar work added `utc()`, `from_posix_rule(identifier, rule)` (a zone from an `Mm.w.d`
  POSIX rule, no database file) and `daylight_saving_time_offset(date)` (extra seconds in force, 1800 for Lord Howe).
  `Calendar` converts its Unix seconds to `coding::Date` for the zone queries.
- `bifrost_app::calendar_system::Calendar` (identifiers `gregorian`, `buddhist`, `japanese`, `islamic-civil` (tabular),
  `hebrew`, `iso8601`): `date_components`, `date(&DateComponents)`, `date_by_adding` (months clamp, days keep wall time over
  DST, hours are elapsed), `range(of:in:for:)` (day/month, day/year, month/year, hour/minute/second), `start_of_day`,
  `first_weekday`. Hebrew months are numbered Tishri = 1, Adar I = 6 (leap years only). Japanese eras Meiji..Reiwa (232..236).
  Islamic is the tabular calendar, not Umm al-Qura, so days may differ by one from the observed calendar.
- `bifrost_app::formatters`: re-exports the `locale` formatters and adds `number` (spell-out English, ordinal, one
  entry point over the five styles), `relative::named_days` ("yesterday", "today", "tomorrow") and `date_components`
  (`DateComponentsFormatter`: positional, abbreviated, short, full; English). German (`de`) now uses comma-decimal
  separators and the "12 %" / "1.234,50 EUR" shapes in `NumberFormatter`.

Gaps: `Locale` has no `region`/`calendar`/`measurementSystem` accessors yet, `DateFormatter` still takes an explicit
`tz_offset` and a Gregorian calendar, week-of-year and weekday-ordinal components and `DateInterval` are missing, CLDR
data from `/usr/share` is not read (small built-in table only), and spell-out beyond English is not there. Host tests
only; not checked on Midget.

## Login items and menu-bar extras (ServiceManagement and NSStatusItem parity)

Modules `bifrost_app::service_management` and `bifrost_app::status_item`; wire tags `0x1b0..=0x1bf` (`proto/src/extras.rs`);
shell side `src/shell/extras.rs`, the bar in `desktop_bar.rs`, the page in `pages.rs`.

**SMAppService.** `SMAppService::main_app()` gives `register(ctx)`, `unregister(ctx)`, `refresh(ctx)` and `status()`.
The shell keeps the login-items list in the `login.items` pref (comma separated app ids), starts those apps once per
session when the desktop is up and unlocked, and lists them in Settings > General > Login Items, where choosing a row
removes it. `status()` is synchronous like Apple's and returns the last status the shell reported (it reports once at app
start and after each call); the call results also arrive in `App::login_item_status(ctx, request, status)`.
Apple parity decisions: the status enum has all four cases (`NotRegistered`, `Enabled`, `RequiresApproval`, `NotFound`).
Gaps that cannot be matched yet: `SMAppService.loginItem(identifier:)`, `.agent(plistName:)` and `.daemon(plistName:)` do
not exist (BifrOSt apps have no embedded helpers, launch agents or daemons), `RequiresApproval` is never produced
(there is no approval step).

**Errors, unregister completion and the settings link.** `ServiceError` carries the `kSMError*` values of macOS 27 (`InternalFailure` 1, `InvalidSignature` 2, `AuthorizationFailure` 3, `ToolNotValid` 4, `JobNotFound` 5, `ServiceUnavailable` 6, `JobPlistNotFound` 7, `JobMustBeEnabled` 8, `InvalidPlist` 9, `LaunchDeniedByUser` 10, `AlreadyRegistered` 11) in `ERROR_DOMAIN` (`SMAppServiceErrorDomain`); `to_error()` gives the `AppError`. `try_register` returns `AlreadyRegistered` when the last status is `Enabled` or `RequiresApproval`; `try_unregister` returns `JobNotFound` when the item is not registered; an agent or daemon name that is not a `.plist` name is `InvalidPlist`. `unregister_with(ctx, done)` is `unregister(completionHandler:)`: `done(None)` once the shell reports `NotRegistered`, `done(Some(error))` otherwise, called at once when the call fails before reaching the shell. `error_for_reply(LoginOp::Register, status)` maps a refused request (`NotRegistered` after register) to `LaunchDeniedByUser`. `service_management::open_system_settings_login_items(ctx)` sends `LoginOp::OpenSettings` (wire value 3) and the shell opens Settings on Login Items & Extensions. The `login-items` system service adds `error` and `code` to a failed reply and has an `open_settings` method.

**NSStatusItem.** `StatusBar::system().status_item(StatusItemLength::Variable)` returns a `StatusItem` with
`set_title`, `set_symbol`, `set_menu(&[MenuItem])`, `set_visible` and `remove`; every setter sends the whole item. A
`MenuItem` is `MenuItem::new(id, title)` with `.enabled(bool)`, `.checked(bool)` and `MenuItem::separator()`. The shell
draws the title in the menu bar's extras area, left of the system extras (an empty title shows the app's initial), opens
the menu as a bar menu and sends the choice back as `App::status_item_action(ctx, item, id)`. An app may hold 4 items
and a menu 32 entries; titles are cut at 64 characters; the items go when the app quits.
Apple parity decisions: items need no permission (the menu bar is the user's to arrange); one-level menus only.
Gaps that cannot be matched yet: `symbol` is carried but not drawn (the bar has no symbol renderer for app-named
symbols; the title or the app initial is drawn), `StatusItemLength::Square` is drawn as `Variable`, there are no
submenus, key equivalents or custom `NSView` buttons, no `button.action` for a plain click (a click always opens the
menu), and the user cannot reorder or hide a third-party extra in Settings > Menu Bar yet.

Contract tests: `service_management::tests`, `status_item::tests` (with `Ctx::testing` and `take_sent`),
`proto::extras::tests` (round trips and limits) and `shell::extras::tests` (registry).

## Core Spotlight (`CSSearchableIndex`, `bifrost_app::quicksearch`)

Apple's CoreSpotlight lets an app index its own content so system search finds it without launching the app. Ours:

- `CSSearchableItemAttributeSet::new(title).content_description(..).keywords(..).thumbnail_path(..).content_url(..)`,
  `CSSearchableItem::new(unique_identifier, domain_identifier, attributes)`.
- `CSSearchableIndex::default_index()` with `index_searchable_items(ctx, &items)`, `delete_searchable_items(ctx, &ids)`,
  `delete_searchable_items_with_domain_identifiers(ctx, &domains)` and `delete_all_searchable_items(ctx)`. Indexing an
  existing identifier replaces the item. Calls are fire-and-forget messages to the shell (no completion handler:
  `ToShell::QuickSearchIndex` 0x1c0, `QuickSearchDelete` 0x1c1; a message carries at most 8 items, the SDK splits larger calls).
- The shell keeps the index per app in `<data>/quicksearch-index/<app id>.json`, bounded to 1000 items per app (oldest
  dropped first) and 64 apps, with long text cut by the wire codec. The search panel (Super+Space, `QuickSearch`) lists matches in a Documents
  section (title match before keyword before description match).
- Opening a result starts the app and sends `ToApp::ContinueActivity` (0x1c2). The app implements
  `App::continue_activity(ctx, UserActivity)`; `activity_type == quicksearch::ACTION_TYPE` (`CSSearchableItemActionType`,
  `"bifrost.quicksearch.item"`) and `unique_identifier` is the item's (`CSSearchableItemActivityIdentifier`). The message
  may arrive before the first `configure`; the runtime holds it until the app has a context.
- Adopted in Notes 1.8.6: every note outside Recently Deleted is indexed (title, first lines, folder as keyword) and
  `continue_activity` opens it with `Notes::show_note`.
- Adopted in Reminders 1.17.0: each titled reminder is indexed by its title and the first note lines (`search.rs`, the
  item id is the reminder id). Edits re-index, deletes and cleared titles leave the index; `continue_activity` selects the
  reminder (`App::show_item`). Checked every 3 s against the store generation.
- Adopted in Mail 0.14.0: the newest 200 messages of the mailbox on screen are indexed by subject and sender (`index.rs`,
  id `account/mailbox/uid`, never bodies). Mail Settings "Show in Search" (default on) turns it off and removes the items.
  `continue_activity` shows the account and mailbox and opens the message.
- Contacts already indexes its contacts (`services::SearchIndex`); Notes, Reminders, Mail and Contacts are the adopters.
- Shell per-app grouping: the Search panel shows indexed results in one ranked pass and sections them by the owning app's
  name (`quicksearch::indexed_groups`, `app_section_title`). Items of an uninstalled app are dropped when the installed
  list changes (`spotindex::forget_missing`, called from `installed_changed`); an empty list (registry not read yet) keeps
  every item.
- Settings > Search store: `<data>/search-prefs.txt` (`shell/search_prefs.rs`) holds the sources the panel does not query
  (`off <section>`, e.g. `off Files`), the apps whose indexed items are hidden (`app <id>`) and the folders file search
  leaves out (`exclude <folder>`, Privacy). The panel applies them (`sources_of`, `indexed_groups`, file walk). The
  Settings page that edits the store is NOT built yet: until it is, the file is edited by hand.

Tests: `bifrost_proto::quicksearch` (wire round trip, packet bound), `bifrost_app::quicksearch` (`Ctx::testing` + `take_sent`),
`springboard::shell::spotindex` (ranking, replace, delete, bounds, disk round trip).

Query side (`bifrost_ui::quicksearch_query`, pure model, host-tested; `bifrost_app::quicksearch::search_items` runs it over an app's own items):
`SearchQuery::new(query_string, QueryContext{fetch_attributes, filter_queries, source_options})` with `start(items, now,
found_items, completion)` (batches of 8, `cancel()` / `CancelHandle`, `found_item_count`); language `attr == "text"`
(`!=`, `<` `>` `<=` `>=` for numbers and times), `*` wildcard, modifiers `c` `d` `w`, `&&` over `||`, parentheses,
`$time.now` / `$time.now(-3600)`. `UserQuery` + `UserQueryContext` rank results (title > keywords > description) and give
`Suggestion`s. `SearchableIndex::new(name, protection_class, capacity)` has acknowledged `index_searchable_items` /
`delete_searchable_items` (`Error::RemoteConnection` when offline, oldest evicted past capacity, `take_evicted`),
`begin_batch` / `end_batch(client_state)` / `fetch_last_client_state`, and `IndexDelegate` (`reindex_all_searchable_items`,
`reindex_searchable_items`) invoked by `index_lost` / `items_corrupt`. Gaps: shell store still the per-app JSON file,
`sourceOptions` has no effect, the search-tool bridge is a gap; not tested on Midget.

Gaps, recorded: the panel does not draw the stored thumbnail yet, and items of an app are searched only while it is installed
(their stored items are dropped once the installed list no longer has the app). Search of an app's own index is available as `quicksearch::search_items` over the pure
query model; `CSSearchableIndexDelegate` re-index requests are `reindex_all_with` / `reindex_identifiers_with`.

### Completion, errors, batches and extended attributes

`bifrost_app::search_index_batch::SearchIndexLedger` is the app's model of its index (the shell index does not acknowledge
calls). It validates like the shell and answers with `Result<(), CSIndexError>`; only accepted calls are sent.
`CSIndexErrorCode`: `IndexUnavailableError` (ledger `close()`), `InvalidItemError` (empty or over-long identifier or domain, or
more than `MAX_ATTRIBUTE_BYTES` of text; `item_position` names the item; nothing from that call is indexed),
`InvalidClientStateError` (no open batch, nested `begin_batch`, state over 250 bytes), `RemoteConnectionError` (declared; the
shell never reports it yet) and `QuotaExceeded` (a call would pass `MAX_INDEXED_ITEMS` = 1000; no silent eviction).
`CSSearchableIndex` gets `index_searchable_items_with_completion`, `delete_searchable_items_with_completion`,
`delete_all_searchable_items_with_completion`, `end_batch_with_client_state` and `fetch_last_client_state`; `begin_batch()` is on
the ledger. A batch holds calls back and commits all or none; `fetch_last_client_state` returns the last committed state.

`CSSearchableItemAttributeSet` also has `content_type`, `content_creation_date` and `content_modification_date` (Unix seconds),
`author_names`, `display_name` (shown instead of `title`; a localized title still wins) and `ranking_hint`.
`CSCustomAttributeKey::new(name, searchable)` with `custom_value(&key, value)`: searchable values are searched, others are kept
but not. Type, authors, `YYYY-MM-DD` dates and searchable custom values are folded into the shell keywords, so `search_items`
matches them. The wire format is unchanged. Gap: the shell does not use `ranking_hint` yet, and `RemoteConnectionError` is never raised.

### Core Spotlight stores, expiry and user queries

- `CSSearchableItem::expiration_date(secs)` and `is_expired(now)` mirror `expirationDate`.
- `bifrost_ui::searchstore` (re-exported from `quicksearch` as `StoreSet`, `IndexStore`, `StoredItem`): one store per app and
  optional index name (`CSSearchableIndex(name:protectionClass:)`). It supports delete by identifier, by domain (a domain
  also removes its dotted sub-domains) and all, `purge_expired(now)`, oldest-first eviction at 1000 items, and
  `begin_batch` / `end_batch(state)` / `client_state` (state up to 250 bytes). `StoreSet::search(requester, owner, name,
  query, now)` returns every live item to the owner and only public items to other apps.
- `bifrost_ui::usersearch` (`run_user_query`, `CSUserQueryContext`, `CSSuggestion`): the items of a `CSSearchQuery` plus
  completions of the last word typed, ranked by how many records hold the word.
- Gaps: the stores are in memory and not yet wired to the shell service or the on-disk files; the attribute query grammar,
  streamed found-items replies and metadata queries over app containers are not built; the assistant search tool stays a gap.

### System search ranking (`bifrost_ui::search_rank`; OS rows HybridDatabaseTokenizer, HybridSearchAdapter, HybridSearchResultRanker, HybridSearchRuntime)

Decision (2026-10-17): the Search panel ranks by one hybrid ranker instead of per-string fuzzy scores, and the runtime
is written as plain threads with a deadline so a slow source never holds up the panel. Pure data, no I/O.

- Tokenizer: `tokenize` splits words close to UAX #29 (letters and digits; an apostrophe between letters joins), folds
  case and diacritics (`Café` is `cafe`, `Straße` is `strasse`), and cuts CJK runs into overlapping pairs (a lone
  character stays itself). `index_terms` adds the single characters of CJK pairs and the prefixes of each Latin word from
  two characters up, so `mai` finds `mail` while typing; queries use `tokenize` only.
- Ranker: `Bm25` (k1 1.2, b 0.75) over weighted fields (title 3, keywords and description 1) with a coverage factor (a
  document holding every query word beats one holding a single word). `rank` adds, with fixed weights: title match
  (exact 1, prefix 0.6, word start 0.3), kind prior for one- or two-word queries (app 1, settings 0.8, contact 0.6,
  document 0.5, file 0.4, other 0.2), recency of indexing (e^(-age/14 days)), local open count on a log scale, and the
  semantic score. `order` puts every item in ranked order, keeping unmatched items after the ranked ones.
- Semantic hook: `SemanticHook` returns an optional 0..=1 similarity. The shell passes `NoSemantic`: `bifrost_ui::embedding`
  is a caller-fed vocabulary and no local text model is bundled, so nothing is called remotely and the term adds zero.
- Usage: `Usage` counts opens per item (`app/unique id`), stored locally as `search-usage.txt` beside `quicksearch.ini`.
  It is never sent anywhere.
- Runtime: `run(query, sources, deadline, per_section, usage, now, semantic)` queries each `Source` on its own thread and
  waits at most the deadline. Items from the sources that answered are ranked together; the result has the top hit, the
  sections in source order (each capped), and the names of sources that were late or failed (`missing`).
- Shell: the Documents section (`springboard/src/shell/quicksearch.rs`, `indexed_groups`) keeps the index's matching
  and re-orders the top candidates with `order`; an open of a Documents result bumps its count.
- Panel: the sources (apps, Settings pages, clock, Deildu titles, contacts, shortcuts, files and calendar events) are
  `Table` sources built per query (`build_tables`); `Query` runs them on threads with a 300 ms deadline and a 12 ms grace
  before the first frame. Late sections stream in, the top hit freezes once shown, and groups already on screen keep their
  order. Opens of apps and Settings pages count alongside indexed results.
- Gaps: notes is not a source (no shell-side store); the semantic hook has no model.

## Table columns and shared panels (NSTableView, NSFontPanel, NSColorPanel parity)

Code: `springboard/ui/src/table.rs`, `springboard/ui/src/panels.rs` (SDK: `bifrost_app::controls::table`, `controls::panels`). Pure models with unit tests.

- **Sortable columns.** `SortDescriptor` (key, ascending), `Column` (`sort_key` = `sortDescriptorPrototype`), `TableColumns::click_header` (first click ascending, second descending; the key becomes primary, earlier keys stay secondary), `indicator(i)` for the header arrow, `sort_rows` (stable multi-key sort). Resize (`resize`, divider grab zone `RESIZE_GRAB`, min and max width), reorder (`reorder`, `drop_index`), and `save`/`restore` for `autosaveName`.
- **Font panel.** `FontPanel`: family, style, size (1 to 288, list `FONT_SIZES`), `on_change` as the `NSFontManager` target action. Families come from the fonts the renderer has, supplied by the caller.
- **Color panel.** `ColorPanel`: RGB, HSB wheel hit-test (`wheel_pick`), hex field (`parse_hex`), alpha, `on_change`. Named system colours are the `render::ui::SYSTEM_*` tokens.
- **Gaps.** No drawing yet: the shell does not render the two panels or a clickable header row, and Files was not adopted (it keeps its toolbar sort). Column hiding (header context menu), `NSColorPanel` crayons/sliders modes and eyedropper, font collections and typography effects are not matched. Sort descriptors compare through a caller closure (no key-path reflection). Contract tests are model-level (`Ctx::testing` not needed: nothing is sent to the shell yet).

## Quick Look (`QLPreviewPanel`, `QLPreviewItem`, `QLThumbnailGenerator`)

Code: `springboard/sdk/src/quick_look.rs` (`bifrost_app::quick_look`), wire types `springboard/proto/src/peek.rs` (tags 0x1e0 `ToShell::Preview`, 0x1e1 `ToShell::PreviewClose`).

- **Panel.** `PreviewPanel`: `set_items` (`dataSource`, at most 16 items per message, URL cut to 256 bytes, title to 96), `current_preview_item_index` (always clamped), `step` (arrows, no wrap), `reload_data`, `make_key_and_order_front`, `close`, `toggle` (Space). `PreviewItem` is `previewItemURL` + `previewItemTitle`.
- **Thumbnails.** `thumbnail_path(url, size, scale)` names a PNG under `$XDG_CACHE_HOME/bifrost-quicklook`; `ThumbnailCache::best_representation` returns the cached path or asks the caller to render it, and caches it (`cache::Cache`).
- **Shell panel.** The shell draws the floating panel (`springboard/src/shell/preview_panel.rs`, `Panel`, `Kind`): images, text, PDF and audio (through the thumbnails service and the registry's providers), and as of this round video (`public.movie`, info line), fonts (`public.font`, info line) and folders (`folder_summary`: item count and file bytes, at most 10000 entries, then "More than"). Keys: Left and Right step without wrapping, Esc and Space close. PDFs are rendered page by page with the render crate's PDF renderer (`cg::pdf::draw::render_page_fit`, pages fitted to the body width and kept only while visible) and scrolled with Up, Down, Page Up and Page Down (`PdfView`, `Scroll`: a pure layout and offset model, clamped at both ends; the strip under the body shows "Page n of N"). A font item sets three sample lines in its own face (`Font::parse` on the file's bytes, read once per path and leaked for the run; a file that does not parse is an info line). Audio and movies play on the shell's media player (`media::player::Player`, paused on open) behind a transport strip: play or pause in `LINK`, a seek track that jumps on a click, and `m:ss / m:ss` time (`Transport`, pure, tested). The card resizes: drag its right or bottom edge (or the corner) and it follows the pointer at twice its distance from the screen centre, held between 240 x 200 points and the screen less a margin (`size_for_drag`, `grip_at`); the size is kept for the next item and opening. The full-screen button (top right) fills the screen; a full-screen card has no edges to drag.
- **Entry from apps.** `bifrost_app::quick_look::preview(ctx, &[path], index)` turns absolute paths into `file://` items titled by file name (`file_items`: relative and control-character paths are dropped and the index follows the kept items); it sends nothing when no path is usable. Files' Space key and the context menu's Preview call the panel.
- **App preview extensions (model).** `ui::preview_manifest` parses `[[preview]]` blocks from the app manifest: `id`, `types` (type identifiers for `Registry::register`), `generator` (the control method the shell calls with `{"file", "width", "height"}`; the app answers `{"picture": "<absolute PNG path>"}`), and `timeout_ms` (default 3000, clamped to 1..10000). `Wait` decides once: an answer in time is used, while a late, missing, error or relative-path answer falls back to the generic preview. Wired: when the panel shows a file of a declared type (`Generators::for_file`, through the preview registry, so the most specific type wins), the shell starts a worker thread that calls the app's generator over its control socket (`intents::exchange`, behind the `Exchanger` trait so tests use a fake) and waits with `Wait` for the declared timeout (`preview_gen.rs`, `Requests::poll`). The PNG the app names is drawn in the panel; a late, missing, error or unreadable answer falls back to the built-in preview of the file (the thumbnails service for pictures, the kind's own body otherwise). The declarations are read from each installed app's `manifest.toml` on every opening. **First provider:** Notes declares `[[preview]]` for Markdown (`net.daringfireball.markdown`, `generator = "preview.render"`) and answers with a PNG of the note's first lines in its own face (`apps/notes-desktop/src/notes/preview.rs`, written under the app's data directory and named by a content key).
- **File menu.** Files declares an app menu titled File with a Quick Look item (`main_menu`, `menu_action` in `springboard/apps/files/src/main.rs`); the shell merges an app menu with a shell menu's title into that menu. The item needs a selection and opens the same panel as the context menu's Preview.
- **Gaps.** Still open: the panel has no Markup button (Files keeps its own) and no `QLPreviewView`. Only a PNG path comes back from an app's generator: a `QLPreviewProvider` reply with HTML, a PDF or a file does not reach the shell yet. Only Notes declares a generator. Nothing of this is checked on Midget yet (drag, full screen, key delivery, the PDF and transport in a real session). No app is adopted yet (Files, PDFCraft and PhotoCraft are next). No built-in renderers: the app renders thumbnails itself (`QLThumbnailGenerator` generates from any file type). No `QLPreviewView`, `previewItemDisplayState`, transition source frames or full-screen mode, and no `QLThumbnailGenerator` update handlers (single result only).

### Quick Look: archives, animated pictures and picture information (2026-10-19, wave 15)

Code: `springboard/src/shell/preview_info.rs` (pure helpers), drawn by `springboard/src/shell/preview_panel.rs`.

- **Archives.** `Kind::Archive` (provider `desk.archive` for `public.archive`): a ZIP is listed from its central directory (`files::zip::Archive`), a plain tar from its 512-byte headers (`preview_info::tar_entries`; long-name and extended-header records are skipped, a damaged header ends the walk with what came before). The panel shows a summary line ("12 items, 3.1 MB when unpacked"), then one line per entry (name and size, folders with a trailing slash), at most 400 entries and a closing "and N more" line. A format with no listing (gzip, xz, 7-Zip, RAR) says so; an unreadable ZIP says "This archive cannot be read." Names lose control characters and are cut to 100 characters.
- **Animated pictures.** A GIF or animated PNG (up to 16 MB, 300 frames, 32 million pixels in all) opens as `Body::Animation`: `preview_info::load_animation` decodes the frames and `PictureAnim::tick` picks the frame from the clock (`frame_index`: delays of 10 ms or less show for 100 ms, the loop count is honoured, the last frame stays after the last pass). A still picture keeps waiting for the thumbnails service.
- **Picture information.** Under a picture the caption shows one line read from the first 256 KB of the file (`picture_info_for`): size in pixels, format, file size, frame count for an animation, camera (the make is not repeated when the model already starts with it), capture date, and the exposure as `1/250 s  f/2.8  ISO 200  35 mm`. Parts the file does not have are left out. The location is deliberately not shown.
- **Preview panel model follows the shell.** `bifrost_app::peek::PreviewPanel::apply_event` and `handle_service_event` apply the shell's `closed` and `index` events to the model (so Space after the user closed the panel opens it again instead of sending a close), and an optional `PreviewPanelDelegate` hears `panel_did_change_item` and `panel_did_close`. Apple's key-event and source-frame delegate callbacks are not matched: the shell handles the panel's keys itself and the card opens at its own place.
- **Gaps.** No scrolling in a long listing (the card shows what fits), no WebP animation (the WebP codec decodes the first frame), no Markup button for apps, and no extension host for out-of-process HTML or PDF replies.

## Image I/O (`CGImageSource`, `CGImageDestination`, `bifrost_render::cg::imageio`, SDK `image_source`, `image_metadata`, `image_animation`, `picture_writer`)

Decoders and encoders are in `springboard/render/src/cg/imageio` (one module per format); apps use the SDK wrappers. Wave 15 (2026-10-19) adds:

- **Animated PNG.** `png::decode` composites an APNG into one full-canvas frame per animation frame (`png::composite`: blend source or over, dispose none, background or previous, the first frame treating "previous" as "background", rectangles clipped to the canvas), with the delay (`delay_num / delay_den`) and the loop count (`acTL`). A file whose default image is not part of the animation skips it. `ImageSource::count` reports the frame count. `encode::encode_apng` writes full-canvas frames with delays and a loop count, and `ImageDestination` / `PictureWriter` write an APNG when a PNG gets several frames of one size (the first frame is also what a reader without animation shows). Colour-managed decode already applies to every frame.
- **More metadata.** The EXIF and GPS tables name `LensMake`, `OffsetTimeOriginal`, GPS `Speed` and `SpeedRef`, `Track` and `TrackRef`, `ImgDirection` and `ImgDirectionRef`. `image_metadata::PictureInfo` adds `lens_make`, `exposure_bias`, `gps_time` (`GpsTime`: UTC time of day with the date stamp when present), `gps_speed_kmh` (converted from km/h, mph or knots) and `gps_direction_deg`.
- **More metadata written.** `PictureWriter` writes `exposure` (shutter time, f-number, ISO, focal length), `exposure_bias`, `lens` (make and model), `gps_time`, `speed_kmh` and `direction`; bad values are ignored. They round-trip through JPEG, PNG and TIFF.
- **Tiles from a picture.** `render::image_tiles::TiledPicture` decodes a picture once and serves tiles through the LRU `TileCache` (`region`, `tile`, `crop_tile`); a viewer holds at most `capacity` tiles in drawable form.
- **Gaps.** The decoders still decode a whole picture before scaling (no DCT-scaled JPEG decode, so the tile cache bounds memory held for drawing, not the decode), no APNG region-only frames are kept (frames are composited to the full canvas), no HDR gain maps, XMP or IPTC, and the picture's own drag payload does not carry image data yet.

## Speech recognition requests and tasks (`SFSpeechRecognizer`, `SFSpeechRecognitionRequest`, `SFSpeechRecognitionTask`)

`ui::recognition_task`, SDK `bifrost_app::speech_recognition` (alias of `ui::recognition_task`). Pure models, no I/O.

- Authorization: `AuthorizationStatus` (`NotDetermined`, `Denied`, `Restricted`, `Authorized`) with one answer per
  device. A second answer is refused and `restrict` overrides any answer, as `authorizationStatus` does.
- Request: `Request` for a `Source` (`Buffer` for live audio, `Url` for a recorded file), with partial results on by
  default, `TaskHint`, contextual strings, on-device-only recognition and an interaction identifier.
- Task: `Recognizer::start` returns a `Task` in `Starting`, refused when not authorized or when on-device recognition
  is required and not installed. States `Running`, `Finishing`, `Canceling`, `Completed`, as
  `SFSpeechRecognitionTaskState`. Results reuse `recognition::Transcription` and `Session`.
- Decisions: the recognizer that produces readings is not in this module. Partial results are dropped by the task when
  the request turns them off. A canceled task completes with `TaskError::Canceled`.
- Gaps: no live microphone feed from an app yet (the audio service records WAV only), no on-device model download
  (see `speech_models`), and no result-level time codes beyond `Transcription::range`. Not tested on Midget.

## Spell checking (`NSSpellChecker`, `NSTextCheckingResult`)

`bifrost_app::spellcheck`: `words(text)` tokenizer (byte ranges, keeps inner apostrophes), `Dictionary` trait (hunspell or other backends plug in), `WordList` (in-memory, edit-distance-1 suggestions), `SpellChecker` with `check_spelling`, `correction`, `guesses`, `learn_word`/`unlearn_word` (persisted in `UserDefaults` key `NSSpellChecker.learnedWords`), `ignore_word` (checker lifetime), `available_languages`, `automatically_corrects_spelling` (flag only), and `CheckingType`/`CheckingTypes` (OptionSet) with `CheckingResult`.
Gaps: no grammar checking (no data); `check` yields spelling only, link/date/address/phone stay in `controls::detect` and are not merged; ignore is not scoped by document tag; no bundled dictionary; no app adopts it yet (Notes and Wordcraft are next).

## Observable values (Combine / Observation parity, `bifrost_app::observable`)

`Observable<T>` mirrors `CurrentValueSubject` / `@Published`: `get`, `set`, `update` (one notification per change), `sink` (delivers the current value, then changes), `subscribe` (changes only), `map`, `remove_duplicates`, `set_if_changed`. `Subscription` is `AnyCancellable`: dropping it cancels; `Subscription::store(&mut Cancellables)` is `store(in:)`. Pure and single-threaded (`Rc`), no shell dependency.

Gaps: no `ObservableObject.objectWillChange` aggregate (use one `Observable` per property or a struct value); no `withObservationTracking` automatic dependency capture; no Send/threaded publishers, schedulers, `debounce` or `combineLatest`; a `set` from inside a callback of the same observable updates the value but notifies no one (the subscriber list is checked out while callbacks run). App adoption (reminders-desktop `store.rs`, podcasts) is deferred: those stores are file based, not observable state.

### WebProcessPool crash recovery (browser S1)
Safari parity: `webViewWebContentProcessDidTerminate` maps to `ContentProcessTerminated` (the browser shows "This webpage was reloaded because a problem occurred", then reloads). `EngineExited` is also raised on read errors; the browser restores tabs and restarts the engine (max 3 per minute). `close` ignores unknown ids. WebKit has no equivalent of a whole-engine restart; this is a Bifrost addition.

## Browser history, bookmarks and address suggestions (SDK gaps)

The browser keeps history (capped at 5000, deduped, frecency ranked) and bookmarks in `~/.local/share/bifrost-browser/*.tsv`
and shows Safari-style Top Hits under the address field. Gaps against AppKit/Safari, decided for now:
- No menu-bar API for app-defined History and Bookmarks menus (Safari: `NSMenu` items). Cmd+D toggles a bookmark and Cmd+Y opens
  the address field with bookmarks listed; a menu and a full History window wait for the SDK menu API.
- No popup list control (`NSPopUpButton`/`NSTableView` popover for the suggestion list); the browser draws the dropdown itself with
  `CARD_BG`, `SEPARATOR_C` and `TINT`.
- Start Page (`startpage.rs`, `starttab.rs`): a blank tab (`about:blank`) shows Favorites (the bookmarks as tiles), Frequently Visited
  (one tile per site from history: frecency order, at least two visits, not already a favorite) and Recently Viewed (history, newest first)
  in a centred column under title 2 headings, Light and Dark through the kit pairs. Decision: it is drawn natively over the page area (like the
  tab overview) and not served to the engine as a page, so the tab stays on `about:blank` and its address, history and session handling are
  unchanged, and Back from a site returns to it. A tile shows the site icon `favicon.rs` already cached on disk, else a monogram on one of eight
  system colours picked from the host (FNV-1a, stable across releases); nothing is fetched. A tap opens the tile in the same tab, the wheel and a
  drag scroll, tiles are published as accessibility buttons, and the address field stays focused and empty after Cmd+T (an empty field on the
  Start Page lists no suggestions, the Favorites are on the page). A private window shows a notice instead of the lists. Gaps against the
  reference, decided for now: no Privacy Report (the Browser counts no blocked trackers yet), no Edit mode or Favorites folders (bookmarks have
  no folders), icons are the 32 px favicons rather than touch icons, no topics from other devices (needs account sync).
- The private-mode flag exists in the store and a private window uses it (`--private`).
- Unit tests live in `suggest.rs` (a binary crate cannot be imported from `tests/compliance.rs`).

## Golden Gate kit: sheets and small controls (round 58)

- `bifrost_ui::sheet`: `Sheet::new` / `Sheet::alert`, `layout(parent, titlebar_h, progress)`, `hit`, `key` (Return = default button, Escape = cancel button, all keys consumed while shown), `segments` / `segment_hit` / `segment_key`, `stepper_*`, `focus_ring_rect`. Drawing: `widgets::sheet`, `segmented`, `stepper`, `focus_ring`, `popover` (arrow on the Popover material).
- A sheet slides from the title bar (`motion::Kind::SheetSlide`; progress is 1 at once under Reduce Motion), dims the parent (28% black) and swallows clicks on it (`Hit::Blocked`).
- Metrics: segmented 24 pt, stepper 13 x 22 pt, pop-up button 22 pt, focus ring 3 pt, sheet radius 10 pt, alert width 260 pt.
- Gaps: sheet buttons size by character count (no text measuring); the top corners of the panel are hidden by extending it upward inside the parent, not clipped.

## Login window (`shell/login.rs`)

Login window parity with the reference (`goldengate_refimages/27-Golden-Gate-Login-Window.png`): the date over a large semibold clock at the top, the avatar disc (initials; the account picture via `account::avatar_path` is not drawn yet), full name and a capsule password field with a dot mask and a return arrow low in the middle (62 % of the height), a message line, and a power button at the right of the status icons whose menu holds Sleep, Restart… and Shut Down… (the last two confirm in a sheet). The wallpaper stays clear behind it (a light veil only). Host preview: `PREVIEW_DIR=DIR cargo test preview_login -- --ignored`. Colours are `Pair`s resolved with `.now()`; sizes use `ui.pt()`. Authentication is `privd` `login.authenticate` on a worker thread (`Backend` trait, stubbed in tests); lockout countdown and the 450 ms shake follow the privd reply; a dead helper shows "Login service unavailable" while the power buttons keep working. Gaps: Caps Lock indicator needs a key-state source (`Login.caps` is wired but nothing sets it); no biometric or paired-device unlock; no user switching (single account); no keyboard-layout status item. Stage 1 (0.5.29): the shell still runs as root and asks privd over its socket; stage 2 moves the shell to the user (docs/privd-protocol.md).


## Undo (NSUndoManager parity)

`bifrost_ui::undo::UndoManager<T>` (`springboard/ui/src/undo.rs`) is the SDK's `NSUndoManager`. Window-based apps get one
per window (`Window::undo`); Edit > Undo and Redo reach it as the `undo` and `redo` actions.

API (Apple name in brackets):

| Method | Meaning |
|---|---|
| `register(name, undo, redo)`, `register_for(target, ...)` | `registerUndoWithTarget:`; a pair of closures instead of an invocation |
| `set_action_name(name)` | `setActionName:`, names the open group, else the newest step |
| `undo_menu_item_title()`, `redo_menu_item_title()` | `undoMenuItemTitle`: "Undo Typing", plain "Undo" when empty or unnamed |
| `begin_undo_grouping()`, `end_undo_grouping()`, `grouping_level()` | nestable; only the outermost close makes a step |
| `set_groups_by_event(on)`, `end_event()` | `groupsByEvent`: edits within one event are one step; the app calls `end_event` at the end of each event |
| `register_coalesced(key, now_ms, gap_ms, ...)` | typing: pairs with the same key within `gap_ms` join the newest step |
| `can_undo()`, `can_redo()`, `is_undoing()`, `is_redoing()` | same names |
| `levels_of_undo()`, `set_levels_of_undo(n)` | `levelsOfUndo`; 0 is unlimited, shrinking drops the oldest steps at once |
| `remove_all_actions(target)`, `clear()` | `removeAllActionsWithTarget:` and `removeAllActions` |

Menu titles: `App::undo_titles(ctx) -> (undo, redo)` reports the titles; the runtime sends `ToShell::UndoTitles`
(tag 0x220) when they change and the shell's Edit menu shows them ("Undo Move", "Redo Typing"). An empty string keeps
the plain word. The row stays disabled unless the app's `commands` reports Undo or Redo, so a window app gets this for
free through `Window::commands` and `Window::undo_titles`. Cmd+Z and Cmd+Shift+Z are unchanged.

Decisions: a step holds its own inverse and repeat closures, so undoing a step moves it to the redo stack and redoing
moves it back (Apple's "registrations made while undoing become the redo"); a new action clears the redo stack
(`ChangeCount` keeps edited-state tracking exact). Adopted: Notes (typing grouped per word, named "Typing", titles from
the editor) and Files (named Move, Rename, Trash, New Folder; Undo only).

Gaps: no `prepareWithInvocationTarget:` or selector registration (closures replace it), so registering from inside an
undo handler is not supported; `remove_all_actions(target)` drops a whole group when any of its parts matches;
`groupsByEvent` is off by default and needs the app to call `end_event`; `actionIsUndoable` and `undoNestedGroup` are
not provided; Files has no Redo (its stack is a plain list of reversible operations); the control-socket undo
(`undo` RPC) is separate from the menu titles.



## Window animations (NSWindow.AnimationBehavior parity)

Reference: `NSWindow.AnimationBehavior` (default, none, documentWindow, utilityWindow, alertPanel) and the Dock's
minimize effects (Genie, Scale).

Done:
- `shell/window_fx.rs` is wired. `Shell::desk_fx_start` begins an effect; `Desk::fx` (`FxTracker`) holds the running
  ones and is empty at rest. `draw_desk` stamps each `WindowFrame::fx`; `wayland.rs` hands it to `Presenter::set_fx`.
- Open: scale 0.9 to 1 plus fade (220 ms). Minimize: scale and slide toward the app's Dock tile (420 ms); the window
  stays mapped until the effect ends. Restore: the reverse. Zoom: a short settle pulse (light click, Window > Zoom, F11).
- Reduce Motion: every effect becomes a 120 ms cross-fade in place (no scale, no slide); Zoom does not animate.
- Behaviour: manifest key `animation_behavior = "default" | "document" | "utility" | "alert" | "none"`
  (`apphost::Manifest::animation_behavior`). `none` never animates; `utility` and `alert` play open and close only.
  Built-in windows use `default`.
- No frames at idle: `fx_tick` drops finished effects (one last identity frame), `next_wake` asks for frames only
  while `FxTracker::active`; the cached-frame fast path (`retain_wayland_windows`) is off while any effect runs.
- The minimize effect pref (`dock_minimize_effect`) is read (`MinimizeEffect::from_index`).

Gaps (not matched yet):
- Funnel (Genie) needs a mesh warp. The presenter draws a window as one quad inside its fixed drawable, so Funnel
  falls back to Scale. A true warp needs a screen-size overlay surface; follow-up.
- The slide is clamped to the window's shadow margin (the drawable is fixed), so the window does not reach a far Dock
  tile before it fades; zoom can only shrink for the same reason.
- Close does not animate yet: `desk_close_app` removes the window at once (the close effect math exists). It needs a
  closing ghost kept until the effect ends.
- Minimize from Show Desktop and Overview, and foreign (native Wayland) windows, do not animate.
- Hot corners and the title-bar double-click prefs are unchanged (not adjacent).
- Nothing here was built or run when written; the unit tests in `window_fx.rs` and `apphost.rs` cover the math.


## Disk Arbitration (DASession parity, `springboard/core/src/disk_session.rs`)

Decisions, recorded for the disk-arbitration matrix rows (`docs/parity-matrix/data/disk-arbitration.json`):

- A `Session` holds the last snapshot of block devices. The caller feeds it snapshots (a udev monitor is the source on
  BifrOSt); the session diffs them with `disk_events::diff` and calls the appeared, disappeared and description-changed
  callbacks in registration order. This matches `DARegisterDiskAppearedCallback`, `DARegisterDiskDisappearedCallback`
  and `DARegisterDiskDescriptionChangedCallback` in shape, and `observe` returns the events it sent.
- Gap, decided: there is no run loop or dispatch queue on the session (`DASessionScheduleWithRunLoop`,
  `DASessionSetDispatchQueue`). Callbacks run on the thread that calls `observe`. The shell or an app schedules the
  snapshots on its own loop. Matching a run loop needs the shared dispatch work, so it waits for that.
- Gap, decided: the peek callback (`DARegisterDiskPeekCallback`) is not implemented.
- Approvals and claims live on the session. `unmount` and `eject` plan commands through `disk_commands`, which checks
  the claim, then the approval chain, then the device state. An unknown device name returns `Refusal::NoDisk`.
- Sprint 3, wired: the shell service `disks` (`springboard/src/shell/services/disks.rs`) scans `/sys/block` and
  `/proc/mounts` every 2 s (`disk_scan`), keeps a `Session`, and pushes `disk_appeared`, `disk_disappeared` and
  `disk_description_changed` to every app (the wire shape is `bifrost_core::disk_wire`). The first scan is silent.
  Calls: `list`, `describe`, `plan_unmount`, `plan_eject`, `claim`, `unclaim`. The plans are validated and returned,
  not run: running them needs the privileged helper, to be tried with a real USB disk on Midget.
- SDK client `bifrost_app::disk_arbitration::DiskSession`: `create` (`DASessionCreate`), `set_dispatch_queue`
  (`DASessionSetDispatchQueue`), `schedule_with_run_loop` (the main queue; there is no run loop object),
  `register_disk_appeared_callback`, `register_disk_disappeared_callback`,
  `register_disk_description_changed_callback`, `list` + `on_list_reply` (disks already connected are reported as
  appeared, as in the native framework), `handle_event`, `disk_from_bsd_name`, `disk_from_volume_path`,
  `description` with the `DESCRIPTION_*_KEY` names (volume path, media size, removable, writable, model, BSD name),
  and `plan_unmount` / `plan_eject` with `plan_from_reply`. Gaps: no volume name or UUID key (the scan does not read
  labels yet), events are service events rather than notify-bus names, and the Files sidebar does not show volumes yet.
- Status stays `partial` until tested on Midget with a real USB disk.

## Workspace (NSWorkspace parity, `bifrost_app::workspace`)

Reference: AppKit `NSWorkspace` (`open(_:)`, `open(_:withApplicationAt:configuration:)`, `openApplication(at:configuration:completionHandler:)`, `activateFileViewerSelecting(_:)`,
`recycle(_:completionHandler:)`, `urlForApplication(withBundleIdentifier:)`, `icon(forFile:)`, `icon(for:)`, `frontmostApplication`,
`runningApplications`, and the workspace `notificationCenter`).

API (all take a `Ctx`; answers arrive in a completion closure on the main thread):
- `open(ctx, url, done(bool))`: web links open in the default browser, another scheme in the app that declares it (`schemes` in the manifest).
- `open_file(ctx, item, done(bool))`: opens with the default app for the file's type (Settings > Default Apps, `shell/defaultapps.rs`, `launch::Registry`).
- `open_with(ctx, items, app_id, done(Status))`, `activate_file_viewer_selecting(ctx, items, done)`, `recycle(ctx, items, done(Status, moved))`.
- `open_application(ctx, app_id, done(Status))` (`openApplication(at:configuration:completionHandler:)`): launches the installed app, or brings it
  to the front when it runs, the way the Dock opens it (`Shell::desk_open_or_start`). `NotFound` for an unknown id, a service or a system package.
  Files' Applications place opens its rows with it (double-click, Return, File > Open); a refusal shows a notice in the status bar.
- Files opens documents through the workspace (W4-18; `apps/files/src/system.rs` `Effect::OpenFiles`): double-click, Cmd+O (each file of a
  selection), the context menu's Open and a provider download ask for a read grant per file (`Ctx::grant_path`, allowed because Files holds
  `storage`), then `open_file` (default app) or `open_with` (a row of the Open With panel, which lists only the apps whose manifest declares the
  type, the default first and marked). The sandboxed app never runs `/usr/bin/bifrost-apps`. A refused grant or open shows "could not be
  opened" in the footer; the sender's grant goes back 10 minutes after the shell took the file (the malware card may hold the open that long).
- Document types live in the manifests (`handles`): Preview owns PNG, JPEG, GIF, WebP and PDF (`public.image` default), Text Editor owns plain
  text, RTF and RTFD, Music `public.audio`, Video Player `public.movie`, Files `public.folder` only. The built-in Photos library is `default`
  rank for PNG/JPEG, so Preview wins as in the reference.
- `url_for_application(ctx, id, done(Option<id>))`, `frontmost_application`, `running_applications`.
- `icon_for_content_type(uti)` and `icon_for_file(name)`: pure, return a glyph name from the type tree (`uttype`).
- `observe(ctx, kind, closure) -> ObserverToken`: did-launch, did-terminate, did-activate application (`user_info` `id`, `name`), will-sleep, did-wake,
  screens-did-sleep, screens-did-wake. They are posted on `NotificationCenter` under the `workspace.*` names; `observe` subscribes the app
  with the shell (`ToShell::WorkspaceSubscribe`, a mask). An app hears nothing it did not subscribe to.

Decisions:
- A file is a document grant (`workspace::Item = GrantId`), never a path. The shell resolves the grant against the grants the app owns (`Denied` for
  anyone else's or an unknown one, same answer, so grants cannot be probed). The app needs the `documents` permission. `recycle` needs a write grant,
  and never moves the home folder or the trash itself. Open and open-with hand the file to the target as a fresh read-only grant (the share-sheet path).
- Nothing launches a binary. `open_with` and `open_application` take an installed app id whose package kind is `app` or `system-app`; services,
  system packages, removed packages, and an id with a `/` or a control character are refused (`workspace::application_to_open`). Apple's
  `openApplication` takes an app URL; ours takes the id, since an app has no path to another app's package. Like Apple's, it needs no
  entitlement: launching an installed app is what the Dock and Launchpad already let any person do.
  Links with `file`, `javascript`, `data`, `blob`, `vbscript` or `about` schemes, or with control characters, are refused.
- Recycle uses the same Recently Deleted folder and `.origins` record as Files, so Put Back works. Reveal opens Files at the file's folder.
- Wire: `ToShell::WorkspaceRequest` 0x230 (op 8 is `OpenApplication`, the id in `arg`), `WorkspaceSubscribe` 0x231, `ToApp::WorkspaceReply` 100,
  `WorkspaceEvent` 101 (`proto/src/workspace.rs`).
  Events are a message of their own, not `DistNotify`, so no app can forge a workspace event.
- Contract tests: `open_uses_the_default_app_for_the_type`, `recycle_moves_items_to_the_trash`, `apps_hear_did_launch_and_did_terminate` (SDK and shell side),
  `open_application_names_the_app_by_id_and_reports_the_status` (SDK), `open_application_launches_only_installed_apps_by_exact_id` (shell),
  `opening_an_app_row_launches_the_app_by_id` (Files), `open_asks_for_a_grant_then_hands_it_to_the_workspace_never_a_helper` (Files),
  `files_opens_documents_in_the_app_that_declares_them` (shell, against the shipped manifests).

Gaps (explicit):
- Sleep and wake events: the shell has `Shell::workspace_system_event(kind)` but nothing calls it yet (no suspend/lid hook located); launch, exit and
  activation are wired in `apphost.rs`.
- Reveal opens one Files window, at the first file's folder, and does not select the files. `open_file` for a built-in app (no process) raises the app
  but delivers no file. `icon_for_*` return glyph names, not images. No `open(_:configuration:)` or `openApplication` configuration options (activates, hides, new instance, arguments; an app is
  always activated), no
  `runningApplications` details beyond ids, no `urlForApplication` path (apps have no URL an app may use), no `selectFile(_:inFileViewerRootedAtPath:)`.
- Not verified on a device or built; the lead runs the tests.


## State restoration (NSWindowRestoration parity)

An app saves what it needs to put its window back, and the desktop puts the windows back, after a relaunch, a restart,
a shut down, an update or a crash.

**API** (`bifrost_app::restoration`, hooks on `App`, `Ctx`):

| Apple | BifrOSt |
|---|---|
| `NSWindow.restorationIdentifier`, `isRestorable` | `App::restoration_identifier() -> Option<&str>` (`None`, the default, is not restorable), `App::is_restorable()` |
| `encodeRestorableState(with:)` | `App::encode_restorable_state(ctx, &mut RestorableState)`; `state.encode_for(key, &value)` takes anything `Encodable` (see Codable) |
| `restoreState(with:)` | `App::restore_state(ctx, &RestorableState) -> bool`; `state.decode_for::<T>(key) -> Option<T>`, `contains` |
| `invalidateRestorableState()` | `ctx.invalidate_restorable_state()` |
| automatic window frame | the window size is saved with the state (`restoration::load` returns it); the position and order are the shell's (below) |

Decisions:
- The state is one JSON file, `restorable-state.json`, in the app's own data dir, written with an atomic replace. It holds a
  format version, the window identifier, the window size and the keyed state. A missing, corrupt, foreign (other
  identifier) or newer-format file is ignored and removed, like a stale restoration archive.
- `invalidate_restorable_state` is a debounce, not a write: the first call of a burst arms one write 750 ms later
  (`DEBOUNCE_MS`), the rest join it. The runtime checks the deadline in its loop (it also sets the wait timeout), so
  there is no write per frame. Suspend and quit write at once. A write whose encoded state equals the file is skipped.
- Sudden termination (see above) is held between the first invalidation and the write: the call does a
  `disable_sudden_termination` and the write the matching `enable`. An app that opted in is therefore killed silently on
  logout, restart or shutdown only when its state is already on disk, which is the point of being restorable.
- The shell starts every app with `BIFROST_RESTORE=1` (`restoration::restore_requested()`) when it should restore, else `0`.
  The runtime calls `restore_state` once, after the first `configure`.
- Reference adoption: Notes (`apps/notes-desktop`) saves the list view (All, a folder, Recently Deleted) and the open note,
  and calls `invalidate_restorable_state` when either changes.

**Shell** (`src/shell/session.rs`):
- `session.txt` in the shell's data dir lists the open windows back to front: app id, frame, zoomed, minimized. Frames are
  stored as parts of the screen (10000 = the whole width or height), so a layout saved at one resolution or scale reopens
  sensibly on another; `Desk::place` keeps a restored frame inside the work area and the app's size limits. The file is
  rewritten while the desktop runs (at most every 2 s, only when it changed), so a crash leaves the last layout, and a last
  time before a restart, shut down or update restart (`Shell::session_save_for_exit`, called by the binary before
  `stop_apps`).
- On the next login, once the desktop is up and unlocked, the windows are reopened in their old order and places and their
  apps start with `BIFROST_RESTORE=1`. Apps that are no longer installed are dropped; if every saved window was minimized,
  the last one is shown.
- The restart, shut down and update-restart confirmations (Settings > General, and the Restart row of the shell) carry the
  checkbox "Reopen windows when logging back in" (pref `reopen_windows_on_login`, default on). Unchecking it saves an empty
  session. The checkbox is `AlertCard::with_checkbox`, a reusable alert accessory (Apple: `showsSuppressionButton`).
- Settings > General > "Close windows when quitting an application" (pref `close_windows_on_quit`, default off, the inverse of
  `NSQuitAlwaysKeepsWindows`). Off: any launch of an app restores its saved state. On: only windows reopened at login do;
  an app the user opens by hand starts fresh.

**Gaps (stated, not hidden):**
- No `NSWindowRestoration` class / completion-handler protocol and no `NSCoder` archive format: the keyed state is a typed
  JSON dictionary, and each app has one restorable window (an app with several windows would need a per-window file and
  identifier lookup).
- The window position is the shell's, not the app's: the app only gets its size back through `restoration::load`; apps that
  want to react to a different restored size use `configure`.
- The Restart, Shut Down and Log Out items of the desktop menu bar still act at once (their ellipsis has no dialog behind it
  yet), so they use the saved "Reopen windows" preference without asking; only the Settings confirmations show the
  checkbox. There is no Log Out (single user session) yet.
- A crash restores the layout and each app's last debounced state, but an unchecked "Reopen windows" is only honoured on
  an orderly restart or shut down; Apple's rule that a crash after a user-initiated quit does not reopen is not modelled.
- "Close windows when quitting" does not delete a saved file; it only stops the next launch reading it.
- Native Linux windows (`window = "native"` apps) keep the older `open-native-windows.json` list (app ids only, no frame).
- Only Notes has adopted the hooks; Files (folder and view) is the next candidate.
- Not exercised on a device: the replay of `session.txt` at login and the alert checkbox drawing were written without a
  build and need a Midget run.

Tests: `restoration::tests` (`restorable_state_round_trips_through_a_relaunch`, `invalidate_coalesces_writes`,
`an_unchanged_state_costs_no_write`, `a_corrupt_or_foreign_file_is_ignored_and_removed`,
`invalidating_holds_sudden_termination_until_the_write`, `is_restorable_follows_the_identifier`,
`the_window_size_is_kept_automatically`), `session::tests`, `desk::tests::a_restored_frame_is_kept_inside_the_work_area`,
`settings::tests::the_restart_alert_offers_reopen_windows_and_remembers_the_choice`,
`settings::tests::close_windows_when_quitting_is_a_general_setting`, and Notes'
`the_open_note_and_the_folder_view_come_back_after_a_relaunch`.

## App runtime frame loop (audit 2026-10-12, SDK-1 to SDK-7, SDK-16, SDK-18 to SDK-20)

- `Configure` updates the live `Ctx` in place (`reconfigure`): the font is parsed once, Bold Text, the glyph cache,
  the defaults and the queued `ctx.out` messages survive a resize. New memfds are made (and a `Buffers` with the next
  generation sent) only when the size really changes. Contract C1: `Buffers.generation` counts the sets (wrapping,
  from 1) and the runtime ignores a `Release` whose generation is not the current one.
- Copy-up: an app whose last frame was `Damage::is_all()` is drawn first and only `stale minus damage` is copied
  afterwards (`copy_up`, `copy_rect_except`), so a full repaint pays no memcpy. `full_redraw()` still skips it.
- Apps with `draws_canvas() == true` are drawn through `draw_canvas`; consecutive `Move`s of one touch in a drain
  batch are delivered as the last one; `DistNotify`, workspace replies and events only ask for a frame when someone
  listens; the controls/menus/titles comparison runs only on turns where something happened.
- Identical repaints are not sent (`paint_slot`): an app that reports its whole surface as damage (it keeps no damage of
  its own) is compared with the buffer the shell shows, and only the rows that differ go out as the frame's damage;
  none that differ, no `Frame` message at all, so the shell draws nothing for a repaint that changed no pixel. Apps that
  report partial damage are trusted as before. Measured so far on the host only (`paint_slot` tests); the device number is
  the shell's frame count with the app idle (`docs/build-performance.md`, "Idle redraw on the device").
- `BIFROST_FRAME_STATS=1` turns on the once-a-second "app frames" line in the shell log (with the count of repaints that
  were identical to the screen and not sent).
- `View::wants_tick` (scroll fling, indeterminate progress) feeds `Window::next_wake` (SDK-6).
- Not done yet: SDK-8 to SDK-13, SDK-15, SDK-17 (the retained view kit and the direct path are kept for the
  layer-backed rebuild in `docs/graphics-architecture.md`; none of them has an app caller today).

## Glass and materials for apps

Module `springboard/ui/src/material_style.rs`. Views are imperative, so each SwiftUI modifier is a builder that yields
`StyleRequest`s (rect, per-corner radii, fill, optional merge group), one per shape; `StyleRequest::to_material_request`
makes the shared `visual_effect::MaterialRequest` the window server consumes. Mapping: ultraThin -> UnderWindowBackground,
thin -> ContentBackground, regular -> Popover, thick -> Sheet, ultraThick -> HudWindow, bar -> HeaderView, each with
`strength` = the level 0..4; glass sets `glass` (Regular or Clear), which the window server draws as GlassRegular or GlassClear.

| SwiftUI | Ours |
|---|---|
| `Material` (ultraThin, thin, regular, thick, ultraThick, bar) | `Material`; `level()` 0..4 is the blur rank, `Bar` is the chrome material |
| `.background(material, in: shape)` | `background(material, MatShape, rect)` |
| `glassEffect(_:in:)`, `Glass.regular/clear/tint/interactive` | `glass_effect(Glass, MatShape, rect)`, `Glass::REGULAR/CLEAR`, `.tint()`, `.interactive()` |
| `GlassEffectContainer(spacing:)` | `GlassContainer`: shapes within `spacing` merge, transitively; regular and clear do not merge by proximity; `requests()` tags merged shapes with a group |
| `glassEffectUnion(id:)` | `GlassItem.union`: the same id always merges, near or far |
| `glassEffectID` and morphing | `GlassIds::frame` starts a `Morph` (spring, response 0.35 s, damping 0.8, the shell's panel-reveal feel) when an id changes geometry; an interrupted morph retargets without a jump |
| `backgroundExtensionEffect()` | `background_extension(content, bounds)` returns the edge strips to mirror and blur |
| `.glass`, `.glassProminent` | `GlassButtonStyle`: interactive capsule; prominent is tinted with the accent |

Mapping note: agent 1's shared material request was not on `integration` when this landed, so `Material` and `Fill`
are minimal compatible definitions; `Fill::Blur(Material)` converts onto the shared type by level. Morphs interpolate a
uniform corner radius (the largest corner), so uneven corners snap to it while moving. Not built or tested (no builds
in this wave); not tried on Midget.

## Image blur filters (Core Image blur category, SwiftUI `.blur`)

`bifrost_render::blur_filters` (SDK: `bifrost_app::image_blur`) holds the blur filters with the reference parameters:
`gaussian_blur` (`inputRadius` is the standard deviation, kernel truncated at 3 sigma), `box_blur` (`2 * round(r) + 1`),
`motion_blur` (radius, angle counter-clockwise from +x), `zoom_blur` (centre, amount = shift in px at the farthest corner),
`disc_blur`, `bokeh_blur` (radius, ring amount, ring size, softness; defaults 20, 0, 0.1, 1) and `masked_variable_blur`
(radius times the mask's alpha, five interpolated gaussian levels). `Edge::Clamp` is `clampedToExtent`, `Edge::Transparent`
fades the border. `BlurFilter::apply(&Image, Edge)` is the app entry point for an `ImageSource::decode()` result;
`ViewBlur::new(radius_pt).opaque(bool).apply(&rendered, scale)` is `.blur(radius:opaque:)`. Gaussian and box are separable
f32 row passes (vectorisable); disc, bokeh, motion and zoom gather taps. Bench: `glass_cost_blur_filters` (picked up by `tools/bench.py`).
Gaps, stated rather than skipped: the output keeps the input size (the reference grows the extent); the Canvas has no offscreen
layer, so a retained `View` cannot be blurred as a group yet (the app renders the view to an `Image` first); the filters are CPU
only (no GPU path); the masked blur reads alpha rather than luminance. Not run on Midget (no builds in this wave).

## Core Image filters for app images (composite, colour matrix, gradient)

Code: `bifrost_render::effect_chain` (named colour filters and `Chain`), `bifrost_render::composite_filters` (blend
filters, `ColorMatrix`, `gamma`, `linear_gradient`), SDK: `bifrost_app::image_filter`. The shell and apps share the
render code; the SDK re-exports it, so there is one API.

| Apple | BifrOSt |
|---|---|
| `CIFilter(name:)` with input keys, `CIFilter` chaining | `image_filter::Filter::named` + `FilterChain::push` / `apply(&Image)`; inputs by key with `set_clamped` and `input_range` |
| `CIMultiplyBlendMode` and the blend filters, `CISourceOverCompositing` | `image_filter::blend(fg, bg, Blend::Multiply)`; `Blend::named` takes `sourceOver`, `multiply`, `screen`, `overlay`, `darken`, `lighten`, `colorDodge`, `colorBurn`, `hardLight`, `softLight`, `difference`, `exclusion` |
| `CIColorMatrix` (rVector to aVector, rBias to aBias) | `image_filter::color_matrix(&Image, &ColorMatrix)`; straight RGBA in, clamped to 0..1 |
| `CIGammaAdjust` (`inputPower`, default 1) | `image_filter::gamma(&Image, power)`; powers at or below zero act as 1 |
| `CILinearGradient` (point0, point1, color0, color1) | `image_filter::linear_gradient(w, h, p0, p1, color0, color1)`; end colours held outside the segment |
| `CIExposureAdjust`, `CIHueAdjust`, `CIColorControls`, `CISepiaTone`, `CIVignette`, `CIColorInvert`, `CIColorPosterize` | the named filters `exposure`, `hueRotate`, `saturation`, `contrast`, `sepia`, `vignette`, `invert`, `posterize` |
| `filterNames(inCategory:)` | `image_filter::filter_names()`, `filters_in(Category)`, `search(query)`, `blend_names()` |

Decisions: composite filters take premultiplied pixels (as `blend::composite` does); colour filters take straight RGBA
(as `colorfx` does, and as Core Image's colour kernels unpremultiply first). Output keeps the input size. Blend math is
the W3C separable set in `render/src/blend.rs`; the `Normal` blend is `sourceOver`.
Gaps, stated rather than skipped: no lazy `CIImage` graph (each call runs eagerly on CPU pixels); no `CIContext`, and the
render destination below is CPU only; `CIColorMatrix` has no per-channel bias beyond the five-vector form; no `CIColorMap` or `CIColorCube`; no
`CIRadialGradient`, checkerboard or stripes generators; the filters are CPU only, with no GPU path. Not built or run on
Midget in this round (unit tests written, not run). The parity matrix row for "Composite operations and blend modes" and
"Colour adjustment" can move to partial; parity needs a Midget test.

#### Render destinations, accumulator, perspective and video filtering

Code: `springboard/render/src/render_destination.rs`, `springboard/render/src/perspective.rs` (pure CPU models); app API
`bifrost_app::image_render` (`springboard/sdk/src/image_render.rs`).

| Apple | BifrOSt |
|---|---|
| `CIRenderDestination` (format, colour space, alpha mode, region) | `Destination::new(w, h, PixelFormat)` with `Rgba8Premultiplied`, `Rgba8Straight` or `RgbaF32Premultiplied`, the `ColorSpace::Srgb` tag, and `render_into(src, Region, &mut dest, origin)`, which clips both rectangles and returns the pixels written; `render_to_destination(img, format)` for a whole image |
| `CIImageAccumulator` (state across frames, `setImage`, `setImage(_:dirtyRect:)`) | `Accumulator::new(w, h)`, `feed(&frame, retain)` (source-over of the frame over the retained state, `retain` clamped to 0..1), `snapshot()`, `clear()`; a size mismatch is refused |
| `CIPerspectiveCorrection` | `perspective_correct(img, quad, out_w, out_h)`: a homography from the four corners (`homography`, `map_point`) with bilinear inverse mapping and transparent edges |
| Running a `CIFilter` chain on each frame of a video | `VideoFilter::new(FilterChain).with_feedback(retain)` and `process(&frame)`, which runs the chain and then the optional accumulator; `reset()` starts a new trail. The app passes the frames it gets from its own player |

Decisions: destinations are tightly packed rows; the 8-bit straight conversion rounds `c * 255 / a`, and a zero alpha gives
black. Pixel input is the software rasteriser's premultiplied u32 (R in the low byte). Tests (written, not run) hold these
to hand-computed images: clipping at negative origins and past the edges, straight-alpha unpremultiply (128 over 128 gives
255), accumulator fades (retain 0.5 gives 128, then 64), homography identity and 2x scale, a sub-quad crop, and degenerate
quads.

Gaps: no GPU destination (`CIContext.render(_:to:)` has no GPU counterpart; the GL dispatch is not wired); only sRGB is
produced, with no colour-managed conversion; `VideoFilter` is not wired to the shell media player (`take_frame` in
`springboard/src/media/player.rs` is not exposed to apps) and Photo Booth and Photos video do not use it yet; no
`CIPerspectiveTransform` (free quad-to-quad) and no perspective tile. Nothing here is verified on Midget.

#### Core Image GPU path (colour kernels)

Code: `springboard/src/gpu/filter_kernels.rs` (pure, host-testable).

| Apple | BifrOSt |
|---|---|
| `CIColorKernel` running `CIColorMatrix` and `CIGammaAdjust` | `ColorKernel { stages: [Stage::Matrix(ColorMatrix), Stage::Gamma(p)] }`: `fragment_source()` (GLSL ES 3.00, unpremultiply, stages clamped one by one, premultiply), `uniforms()` (`u_m<i>` column-major `mat4`, `u_b<i>` bias, `u_g<i>` power) |
| `CISourceOverCompositing` | `source_over_fragment_source()`: `fg + bg * (1 - fg.a)` on premultiplied texels |
| `CIContext(options: [.useSoftwareRenderer: false])` | `Backend::choose(has_gl_context, ContextOptions { use_software_renderer })`: GPU only with a context and no software request, else CPU |

`ColorKernel::eval_pixel` and `source_over_eval` are the CPU reference of the generated expressions; tests hold them within
1/255 of `ColorMatrix::apply_pixel`, `gamma` and `composite`. Gaps: the GL program build and draw and
`CIContext.render(_:to:)` are not wired yet; no app API; written but not run (no builds this round), not tested on Midget.

### Accessibility display options (Reduce Transparency, Increase Contrast)

Settings > Accessibility > Display holds the toggles (`a11y_reduce_transparency`, `a11y_contrast`, `a11y_button_shapes` in
`settings.ini`; Increase Contrast turns Reduce Transparency on and locks it). `AccessibilitySettings::publish` mirrors them into
`bifrost_render::appearance`, sends them to apps in `ToApp::Appearance` flags, and writes `/run/bifrost/user/accessibility`
for the window server (`docs/window-server.md`).

Apps read them through `kit::appearance`, the mirror of NSWorkspace:

| Apple | BifrOSt |
|---|---|
| `accessibilityDisplayShouldReduceTransparency` | `appearance::reduce_transparency()` |
| `accessibilityDisplayShouldIncreaseContrast` | `appearance::increase_contrast()` |
| `accessibilityDisplayShouldDifferentiateWithoutColor` | `appearance::differentiate_without_color()` (Button Shapes) |
| `accessibilityDisplayOptionsDidChangeNotification` | `appearance::observe_display_options(f)` / `remove_observer(token)`; fires only when a value moves |
| SwiftUI `accessibilityReduceTransparency`, `colorSchemeContrast` | `reduce_transparency()`, `color_scheme_contrast()` |

`appearance::display_options()` returns all three as a `DisplayOptions`. Increase Contrast adds a material outline
(`ui::MATERIAL_BORDER_CONTRAST`, `ui::material_border()`) and stronger separators (`SEPARATOR_CONTRAST`, `separator_now()`).
Gap: the shell's materials do not yet all draw `material_border()`; adoption is per surface. Not built or run on Midget.

## View effects and Canvas (SwiftUI drawing and Core Animation shadow parity)

Code: `springboard/render/src/effects.rs` (planes, layers, shadow, mask, group opacity, blend) and
`springboard/sdk/src/graphics_context.rs` (`Canvas`, `GraphicsContext`, `Path`). CPU, premultiplied RGBA8.

| Apple | BifrOSt |
|---|---|
| `shadow(color:radius:x:y:)`, `shadowRadius`, `shadowOffset`, `shadowOpacity` | `effects::Shadow` + `shadow_layer(shape, Shadow)`; Gaussian, sigma = radius / 2, offset rounded to whole pixels; colour alpha is the opacity |
| shadow spread / `shadowPath` | `Shadow.spread` (round dilate/erode); any shape plane is the path |
| `cornerCurve = .continuous` | `CornerCurve::Continuous` in `rrect_cover` / `rrect_plane` (superellipse n=5, reach 1.28 r); `Circular` for the plain curve |
| `mask(alignment:_:)`, `clipShape` | `Layer::mask(&Plane)` multiplies alpha by anti-aliased coverage; `GraphicsContext::clip(path)` |
| `compositingGroup()`, `opacity` | `Layer::draw_group(src, opacity, mode, dx, dy)` flattens the group first; `GraphicsContext::draw_layer` |
| `blendMode` | `blend::Mode` (separable modes) via `composite` |
| `Canvas`, `GraphicsContext` fill, stroke, draw image, draw text, transforms, clip, opacity, blend, layers, shadow | `Canvas::draw`, `GraphicsContext::{fill, stroke, draw_image, draw_text, translate, scale, rotate, clip, set_opacity, set_blend_mode, add_shadow, draw_layer, save, restore}` |

Gaps, decided: no hue/saturation/brightness/colour-matrix filters on views, no variable `blur` modifier in this module (glass blur lives in `render/src/glass.rs`); strokes have round joins and caps only, no dashes or miter limit; `draw_image` and `draw_text` take axis-aligned transforms only; no gradients/shading in `GraphicsContext` yet (use `render::gradient`); no resolved symbols or child views; view-tree integration (`.shadow()` style modifiers on widgets) is not wired because `sdk/src/widgets.rs` is owned by another wave. Not built or run in this round; tests are unrun golden-pixel tests (`effects.rs`, `graphics_context.rs`).

## CallKit: incoming-call card and default calling app (`CXCallUpdate`, `CXAnswerCallAction`, default calling app)

Pure models in `springboard/ui/src/call_ui.rs` and `springboard/ui/src/call_apps.rs`, re-exported by the SDK as
`bifrost_app::calling_ui` and `bifrost_app::calling_apps` (the call controller itself is `bifrost_app::calling`, from
`call.rs`). Host unit tests only; not wired into the shell and not yet run on the VM or Midget, so the matrix rows stay `partial`.

- Incoming-call card: `CallUpdate` mirrors `CXCallUpdate` (handle, caller name, video, and hold, group and DTMF support, all
  defaulting to supported). A blank caller name falls back to the handle value. `IncomingCard::for_call` builds the card for a
  ringing call only; the presentation is a banner while the screen is in use and full screen while it is locked.
  Decline sends `CallAction::End` and Answer sends `CallAction::Answer`, the same requests `CXEndCallAction` and
  `CXAnswerCallAction` produce.
- Decided gaps: no PushKit-style VoIP push wake-up and no ringtone or vibration (the shell and audio service own them); the
  presentation depends on a caller-supplied `screen_locked` flag until the shell reports lock state to the model.
- Default calling app: `CallingApps` keeps the apps that registered for calling, in registration order, and one default.
  Only a registered app can be chosen. Unregistering the default clears it. The Settings page that lets the user pick the
  default is not built yet (the decision is recorded here; the page belongs to the shell).

## Core MIDI (`bifrost_app::midi`)

The MIDI models are shared by the shell, the MIDI daemon and apps. `bifrost_midi` owns them; `sdk/src/midi.rs` re-exports
`clock`, `endpoints`, `events`, `hub`, `thru` and `ump` so an app uses the same `Hub`, `Registry`, `Ump` and `MidiEvent` types as
the server. Apple parity: the vocabulary follows MIDI Services (clients, virtual sources and destinations, source connections,
received packets with host-clock timestamps). Decided: the SDK exposes the pure models only. An app does not open a device here;
the daemon owns the hub and delivers to apps.

Gaps, decided: the pure models stay as they are; the client of the running daemon is `bifrost_app::midi_client`
(below), added 2026-10-19. Not built or run in this round; the tests in `sdk/src/midi.rs` are unrun.

MIDI-CI (`bifrost_app::midi_ci`, Apple `MIDICIDiscoveryManager`, `MIDICIDevice`, `MIDICIProfile`): `Message::encode` and
`Message::decode` convert MIDI-CI 1.1 Universal SysEx (discovery inquiry and reply, invalidate MUID, profile inquiry, profile
inquiry reply, profile set on, profile enabled report) with 28-bit `Muid`s in 7-bit bytes. `DiscoveryManager` builds the
broadcast inquiry, answers inquiries for the local device and records `Device`s and their `Profile`s from replies. Decided: no
property exchange or process inquiry, no transport; the caller moves the bytes. Tests in `sdk/src/midi_ci.rs` are unrun.

## Core MIDI: the MIDI server client (`bifrost_app::midi_client`, `bifrost-midi`)

Apple reference: MIDI Services (`MIDIClientCreate`, `MIDIObjectGetIntegerProperty`, `MIDISourceCreate`,
`MIDIDestinationCreate`, `MIDIPortConnectSource`, `MIDISend`, `MIDIReceived`, `MIDIThruConnectionCreate`, the setup-changed
notification). Added 2026-10-19; written without a build, so the tests are unrun and nothing is verified on Midget yet.

The daemon `bifrost-midid` now listens on a unix socket (`/run/bifrost/audio/is.olibuijr.midi.sock`, mode 0666, or
`BIFROST_MIDI_SOCK`). Its frames are `bifrost_midi::wire` (a `u32` length and a little-endian payload, tags below 0x100
for requests and above for replies and events); the server logic is `bifrost_midi::broker::Broker`, which owns the
`Hub` and one small state per connection and is tested without a socket; `bifrost-midid/src/server.rs` only moves bytes.
`MidiClient` is the app side:

- `MidiClient::connect(name)` says hello. `sources()`, `destinations()`, `devices()` and `device_of(endpoint)` read the
  setup (devices, entities, endpoints with `connected` and `owner`); the lists re-read themselves when the server says
  `MidiEvent::SetupChanged`, which it does on hardware hotplug (the daemon rescans `/dev/snd` every two seconds) and when
  any client creates or disposes an endpoint.
- `connect_source(id)` and `disconnect_source(id)` (a client has one input port and hears every source it connected, in
  host-time order); `create_source(name)` and `create_destination(name)` for virtual endpoints; `dispose`, `set_ref_con`.
- `send(destination, host_ns, &[Ump])` and `send_bytes` (MIDI 1.0 bytes, running status and system exclusive handled)
  queue packets for a destination at a host-clock time (0 or the past: now, a future time waits in the hub). A destination
  that is hardware is written to its rawmidi port by the daemon (new: rawmidi output); `host_now_ns()` reads the server's clock (the
  `Pong` carries it) so an app can stamp a packet a few milliseconds ahead; a virtual destination delivers to
  its owner as `MidiEvent::Delivered`. `inject` and `inject_bytes` send from a virtual source the client owns to everyone
  connected to it. A refused `send` or `inject` comes back as `MidiEvent::SendRefused`, a separate message so it is never
  taken for the reply to a later request.
- `create_thru(source, destination, ThruSpec)` runs `bifrost_midi::thru` inside the server (channel filter, note and
  velocity value maps, control rewrites; MIDI 1.0 and MIDI 2.0 packets keep their type, other messages pass), so play-
  through needs no round trip to the app; it ends when disposed or when the client disconnects.
- `SysexCollector` joins system exclusive packets into whole `F0 .. F7` messages per sender, and `MidiCiLink` runs the
  MIDI-CI `DiscoveryManager` over the server (`discover`, `inquire_profiles`, `process` answers inquiries), so MIDI-CI has
  a transport now (still no property exchange subscriptions).
- Limits: 128 virtual endpoints and 32 thru connections per client, 64 connections, 8192 packets per message,
  256 KiB per frame; a client that sends a malformed frame or stops reading is dropped and its endpoints go with it.
- `bifrost-midi list | monitor [SECONDS] | send DEST BYTES | note DEST KEY` (`cmd/src/midictl.rs`) is the check from a
  shell: `list` prints the devices and endpoints, `monitor` connects every source and prints
  `host_ns source-name words`, `send 5 90 3C 64` writes a note-on.
- Packaging: `packaging/arch/bifrost-midid.service` (user `audio`, `/dev/snd` only) is installed with the binary and
  enabled by the package's post-install step.

Decided gaps: one input port per client (no `MIDIInputPortCreate` ports of their own), timestamps are host-clock
nanoseconds rather than `MIDITimeStamp` ticks, no ALSA sequencer output (hardware output is rawmidi only), no kernel UMP
endpoints, no BLE GATT link (the Bluetooth service has to feed `BleIngress`), no RTP-MIDI sockets or Bonjour.

Network sessions (`bifrost_app::midi_network`, Apple `MIDINetworkSession`, `MIDINetworkHost`, `MIDINetworkConnection`):
`NetworkSession` holds enabled, port (default 5004), names, `ConnectionPolicy`, contacts and connections, and drives the
RTP-MIDI session handshake with `SessionPacket` (`IN`, `OK`, `NO`, `BY`; big endian, protocol 2). Decided: no sockets, no
Bonjour discovery and no RTP data stream yet. Tests in `sdk/src/midi_network.rs` are unrun.

## Core Motion raw sensors (`bifrost_app::motion_raw`)

`RawMotion` is the `CMMotionManager` raw-sensor facade over the kernel sensor tree: `open(root)`, `start_accelerometer_updates`,
`start_gyro_updates` (and `_with` handler forms), `accelerometer_data`, `gyro_data`, `set_*_update_interval`, `tick(now)`.
Accelerometer values are in g and gyroscope rates in rad/s. An interval snaps to the nearest rate listed in the sensor's
`sampling_frequency_available` and the choice is written to `sampling_frequency`; a sensor without those files keeps the
requested interval. Decided: no mounting matrix, no hardware on the lab machine, so only a fixture sysfs tree is tested; not
yet checked on Midget. Tests in `sdk/src/motion_raw.rs` are unrun.

## Audio Unit: plug-ins, sequencing and the generic view (`bifrost_app::audio_engine::units`, `midi_player`, `audio_unit_view`)

Apple reference: Audio Unit hosting (`AudioComponentFindNext`, `AudioComponentInstanceNew`, `AUAudioUnit`: parameter tree,
factory presets, `render`, MIDI events), `AVAudioSequencer` and `AVMIDIPlayer`, and CoreAudioKit's generic view. Added
2026-10-19, unbuilt and unrun. `audio/units` is the unit model the engine hosts; the SDK reaches it through
`bifrost_app::audio_engine::units` (the engine re-exports the crate).

- Plug-in loader (`audio/units/src/plugin.rs`): a third-party unit is a shared library exporting `bifrost_unit_entry`, which
  returns a `#[repr(C)]` `UnitVTable` (ABI version 1: `describe`, `param`, `create`, `destroy`, `initialize`,
  `uninitialize`, `reset`, `render`, `latency`, `tail`, all plain C functions). `PluginLibrary::open(path)` loads it with
  `dlopen`, reads its description (a four-character component code triple, kind, channels) and its parameters, and
  `instantiate()` makes a `PluginUnit`, an ordinary `AudioUnit`: it goes into a `UnitHost`, an engine node, or the
  out-of-process `Worker`. The host owns the parameter tree, so ramps, clamping, read-only flags and presets work as for
  built-in units; each block the plug-in gets every parameter's value at the first frame and one frame after the last
  (`param_start`, `param_end`) and interpolates. MIDI arrives as `CMidi` messages at frame offsets. `render` allocates
  nothing; output that is not finite is silenced. `register_into` adds the plug-in to a `ComponentRegistry` (which now takes
  boxed, fallible factories), `scan_directory` loads every `*.so` in a directory (`/usr/lib/bifrost/units`,
  `~/.local/share/bifrost/units`), reporting each failure. `audio/units` moved from `forbid(unsafe_code)` to
  `deny(unsafe_code)` with `plugin` the one module allowed `unsafe` (the loader and the calls through the table).
- Sequencer (`sequencer.rs`): `SequencerUnit` wraps an instrument and a `Sequence` and puts each message on its exact
  frame from inside the render callback, at any block size; `SequencerHandle` (atomics) has `play`, `pause`, `stop`,
  `seek`, `set_loop`, `set_rate` (0.25 to 4), `position_seconds`, `is_finished`. Pause, seek, loop wrap and the end release
  held notes; live MIDI is merged by offset. `Sequence::from_ticks` applies a tempo map.
- Built-in instrument (`synth.rs`): `GmSynth`, 32 voices of an additive tone chosen by the General MIDI program, drums on
  channel 10, channel volume, pan, expression, sustain pedal, pitch bend, all notes off. No sound bank: it is simpler than a
  sampled instrument. Registered as `aumu/gmsy/bfst`.
- `MidiPlayer` (`sdk/src/midi_player.rs`): a Standard MIDI File (or any `Sequence`) through `GmSynth` or any instrument unit
  in an `Engine`; `control()` is the transport, `render(frames)` pulls audio, `render_to_wav(tail)` renders the whole file,
  `engine_mut()` reaches the engine (taps, effects, `start_on_device`).
- Presets: `PresetStore::export_user` and `import_user` keep user presets in a text file (one per line).
- `UnitView` (`sdk/src/audio_unit_view.rs`, the generic view): a row per parameter with the control that suits it
  (log-taper slider for frequencies, switch, pop-up, read-out), value text with units, sections by group, and input
  (`set_position`, `toggle`, `select`, `nudge`, `reset`, `apply_preset`) that sets the unit through its `ParamHandle`.
  It is a model: an app draws the rows with its kit widgets.

Decided gaps: no extension process for plug-ins (a loaded plug-in runs in the host's process; the shm `Worker` can host one
but nothing launches it), no plug-in supplied view (only the generic one), no AU v2 or v3 binary compatibility (the ABI is
ours), no state chunk beyond the parameters and presets.

## AVFAudio: AVAudioPlayer shape and the node graph (`bifrost_app::audio_player`, `bifrost_app::audio_engine`)

Apple reference: `AVAudioPlayer` (volume, `setVolume(_:fadeDuration:)`, `pan`, `rate`, `numberOfLoops`, `currentTime`,
`duration`) and `AVAudioEngine` (attach, connect, taps). Decisions, made in this round:

- `ui/src/audio_player.rs` (`AudioPlayer`) is the pure model: play, pause, stop, volume with a linear fade, stereo pan,
  rate 0.5 to 2 by linear interpolation, loops (0 once, N extra, -1 until stopped), seek and `render` of stereo blocks.
  Pan uses the equal-power law of `panner.rs`. A mono source is placed by the pan; a stereo source keeps both channels
  and the pan scales each side (a balance). Rate keeps the source sample rate and does not change `duration`. Pause keeps
  the time; stop keeps the time too; a player that finishes stops and rewinds to 0. Out-of-range values are refused,
  not clamped. The `enableRate` switch of AVAudioPlayer is not modelled: the rate always applies.
- `audio/engine` (`bifrost-audio-engine`, `Engine`) is the node graph: players, mixer, units, connections, detach and
  taps (`attach_tap`), offline and device rendering. The SDK now re-exports it as `bifrost_app::audio_engine`. The older
  pure model in `ui/src/audio_engine.rs` is superseded by it; the shell may use either until it is removed.
- Playback rate reaches the audio service (protocol change, Voice Memos 1.10.0). `Cmd::SetRate(f32)` (wire tag 18,
  0.5 to 2.0, clamped, NaN is 1) is in `springboard/proto/src/svc.rs`; `audiod` applies it to its one player
  (`springboard/src/media/audioplayer.rs`, `set_rate`). The rate is time-stretched, not resampled: `springboard/src/media/timestretch.rs`
  is a WSOLA stretcher (1024-frame Hann windows, 512-frame hop, a search of 256 frames), so the pitch stays put and
  the output is `input / rate` long. The stretcher holds back about 27 ms and drops that tail on a song change and a seek.
  The rate stays set across songs. The `State` event gained a trailing `rate: f32` (both ends ship together). The SDK
  sends it through `bifrost_app::audio::Command::rate` (`springboard/sdk/src/system_audio.rs`) and the shell client
  `springboard/src/svc.rs` `Client::set_rate`. Voice Memos stores the speed per recording (`.<stem>.rate` sidecar) and
  shows a Slower/Faster stepper at 0.5, 1, 1.5 and 2. Tested headless (encode/decode, length, dominant frequency by a
  small DFT); not verified on Midget.
- Pan and fades reach the speaker (2026-10-19, unbuilt and unrun): `Cmd::SetPan(f32)` (tag 20) and
  `Cmd::FadeVolume { target, ms }` (tag 21) are in `springboard/proto/src/svc.rs`; `audiod` applies them in
  `springboard/src/media/audioplayer.rs` through `media/player_fx.rs` (pan is a balance, the far channel falls to silence;
  a fade ramps the gain across each block so it has no steps and the snapshot volume follows it; `SetVolume` cancels it).
  The shell client has `set_pan` and `fade_volume` and the SDK has `Command::pan` and `Command::fade_volume`. Rate was
  already there. The player is still one shared player (apps play through `bifrost_app::audio`); a second source needs
  `bifrost_app::audio_engine`. Session categories, interruptions and speech stay gaps. Verify the audio path on Midget
  before any row is `parity`.

Tests: unit tests in `ui/src/audio_player.rs` (not run in this round).

## AVFAudio: audio sessions and interruptions (`bifrost_app::audio_session`)

Apple reference: `AVAudioSession` (`setCategory(_:mode:options:)`, `setActive(_:)`), `interruptionNotification` and
`routeChangeNotification`. `SessionHub` is the pure model: `register`, `set_category` (Category, Mode, Options with
`mix_with_others` and `duck_others`), `set_active`, `set_route`, `take_events` per client. Decisions:

- Nothing is active until `set_active(client, true)`; apps never take audio on launch or focus.
- Ambient, `mix_with_others` and `duck_others` sessions mix: they never interrupt and are never interrupted. Ducking
  only flags the others (`is_ducked`).
- Activating a non-mixing session interrupts the other active non-mixing sessions (`Began`). Deactivating it, or
  `unregister` on quit, sends `Ended { should_resume: true }`; the app reactivates itself. Chains resume in order.
- `set_route` posts `RouteChange` (raw reasons as in the framework) with the previous route to every client; a category
  change posts `CategoryChange` to that client.
- Route changes are wired (2026-10-19, unbuilt and unrun): `Shell::set_route` tells `audiod` with `Cmd::SetRoute { headphones,
  reason }` (tag 22; reason 1 a device appeared, 2 it went away; the shell also reports the route after every service
  reconnect, reason 0), `audiod` keeps it, sends `Evt::Route` (tag 0x105) to every subscriber and to each new subscriber, and
  `bifrost_app::audio::Client::take_route_changes` plus `audio_session::apply_service_routes(hub, client)` turn the reports
  into `SessionHub::apply_service_route`, so every registered session gets `RouteChange` with the system's reason.
- Not done: the interruption arbitration is not in `audiod` and has no wire messages (apps' `SessionHub`s are local), and the
  other modes and options (`interruptSpokenAudioAndMixWithOthers`, `allowBluetooth`) are not modelled. Rows stay
  partial until a Midget check.

Routes, inputs and service resets (added in sprint 11):

- `current_route(client)` returns a `RouteDescription` (`inputs`, `outputs` of `PortDescription` with `port_type`,
  `port_name`, `uid`). The output follows the hub route unless the client forced the speaker.
- `available_inputs`, `set_available_inputs`, `set_preferred_input(Some(uid))` and `preferred_input`. A preferred
  input must be listed; it is cleared when the device goes away.
- `override_output_port(client, PortOverride::Speaker)` posts an `Override` route change to that client only;
  `PortOverride::None` posts `RouteConfigurationChange`. Repeating the same override is not a change.
- `media_services_were_reset` posts `SessionEvent::MediaServicesWereReset` to every client and deactivates all
  sessions; apps rebuild their audio objects and activate again.
- `RouteChange` still carries the previous `Route` only, not a full previous `RouteDescription`.

Tests: unit tests in `sdk/src/audio_session.rs` (not run in this round).

## AVFAudio: recorder and per-source mixing (`bifrost_ui::audio_recorder`, `bifrost_ui::playback_arbiter`)

Apple reference: `AVAudioRecorder` and `AVAudioMixing`. `AudioRecorder` takes `Settings` (`AVFormatIDKey` linear PCM
only, `AVSampleRateKey`, `AVNumberOfChannelsKey`) and is fed capture blocks with `push`. Its device clock advances with
the input, so `record_at(time)` starts mid-block when the clock reaches `time`, and `record_for(seconds)` stops and
truncates at the duration. `pause` drops blocks until `record` resumes; `wav_bytes` always returns one continuous
16-bit WAV. Input is converted with `pcm_convert` (channel remap, linear resampler). Metering
(`set_metering_enabled`, `average_power`, `peak_power`) is per output channel over the latest block, in decibels with
0 at full scale and -160 for silence.

`Arbiter` keeps `volume` (0 to 1) and `pan` (-1 to 1) per participant and `mix` sums stereo sources with an
equal-power pan that is unity at the centre, then clamps to -1..1. Players already have rate, pan and fade
(`audio_player`).

Not done: no capture device feeds the recorder, the arbiter mix is not in `audiod`, the shell does not call
`set_route` or `media_services_were_reset`, and the engine output node is not tied to the real output path. Rows stay
partial until a Midget check. Tests: unit tests in both files (not run in this round).

## Image analysis: subject lifting and Visual Look Up (`bifrost_app::image_analysis`)

Models: `springboard/ui/src/subject_lift.rs` (subjects, hit test, highlight, cutout) and
`springboard/ui/src/visual_lookup.rs` (`AnalysisTypes`, `look_up`). The SDK re-exports both from
`springboard/sdk/src/image_analysis.rs`. The shape follows `ImageAnalysisInteraction` (`subjects`,
`highlightedSubjects`, `analysisTypes`, the Look Up action).

Decisions:

- A subject is an id, an 8-bit alpha mask in image pixels, and its tight bounds. A pixel belongs to the
  subject at alpha 128 or more (`SUBJECT_THRESHOLD`). A subject with no such pixel is refused.
- Hit testing is topmost-first: a later subject is on top of an earlier one. Duplicate ids are dropped.
- A cutout keeps soft edges: alpha = mask alpha times image alpha, cropped to the subject bounds.
- `look_up` builds a request only when `AnalysisTypes` includes `VISUAL_LOOK_UP`. It returns the id,
  bounds and cutout. The lookup service itself is not in the repo.

Gaps still open: no segmentation model (the masks come from the caller), no lookup service, no image view
that draws the subject highlight or the Look Up button, and no text recognition. Matrix rows
"Subject lifting and background removal" and "Visual Look Up" are `partial` at best until the model runs
on a device with a segmenter.

## AVKit: route picker button and display management (`AVRoutePickerView`, display criteria)

Pure models in `springboard/ui/src/route_picker_view.rs` and `springboard/ui/src/display_manager.rs`, re-exported by the SDK
through `bifrost_app::controls` (the SDK re-exports the whole `bifrost_ui` crate). Host unit tests only; not wired into the
shell or the video path, and not yet run on the VM or Midget, so the matrix rows stay `partial` (never `parity`).

- Route picker button: `RoutePickerView` mirrors the button's style (`System`, `Inactive`, `Active`), its highlight rule, and the
  will-begin and did-end delegate events for the routing menu. The rows of the menu come from `route_picker.rs`, which already
  exists; this model does not choose routes. Decided: the button is unbordered unless an app sets `bordered`.
- Receiver discovery: `bifrost_app::route_discovery::discovery_point(owner)` declares a namespaced point; `ReceiverFeed`
  checks reports, emits `Found`/`Lost`, expires them by TTL and produces authorized picker rows. It opens no sockets and
  does not run an extension or feed the shell picker, so discovery remains partial.
- Display management: `DisplayManager` takes the criteria a playing app asks for (refresh rate and dynamic range, the shape of
  the display criteria) and picks the supported mode with the same dynamic range whose rate is closest to the content rate
  (ties go to the higher rate, so content is not slowed down). Rates within 0.05 Hz are the same rate. A switch is pending
  until `finish_switch`; requests that match the current mode return `Keep`.
- Decided gaps: nothing switches the panel yet (the video path owns it); HDR is only a dynamic range flag, with no HDR metadata
  or tone mapping; there is no user setting that turns mode switching off; route selection still lacks an AirPlay and nearby
  receiver source on the shell side.

## AVKit: player view (`AVPlayerView`, `VideoPlayer`)

Pure model in `springboard/ui/src/player_view.rs` (re-exported through `bifrost_app::controls`). Host unit tests only, unrun and
not checked on Midget, so the matrix row stays `partial`.

- `PlayerView`: `controls_style` (`ControlsStyle::{None, Inline, Floating, Minimal}`), `shows_full_screen_toggle_button`,
  `allows_picture_in_picture_playback`, `updates_now_playing_info_center`. `visible_chrome()`, `layout(bounds)` and
  `hit_test(bounds, x, y, duration)` give the chrome; `tap(...)` drives any `PlayerHandle` (play, pause, seek).
- `VideoPlayer<O>`: SwiftUI `VideoPlayer(player:videoOverlay:)` as a player view with inline controls plus an optional overlay.
- Helpers: `scrub_time` (x to `MediaTime`), `format_time` (`m:ss`, `h:mm:ss`), `format_remaining`.
- Gaps: nothing is drawn or decoded here; Picture in Picture is a flag only (see `pip_window.rs`); no delegate.

Video audio output is separately exposed by `bifrost_app::video_playback::open(path, VideoAudio::{Speaker, Silent})`.
It opens the system media player with a speaker or clocked silent output and distinguishes a missing file from other
open failures. This does not add a network route picker, an app-facing video window, or shell integration; the feature
stays partial until the playback path is checked on Midget.

## StoreKit parity round (product kinds and promoted purchases)

Built as pure models in `springboard/ui/src/` (shared by the shell and the SDK through `bifrost_app::controls`). Not run or
built in this round; the unit tests in `product.rs`, `promotion.rs` and `springboard/src/shell/store/api.rs` are unrun.
Status stays partial until a test on Midget.

- `product::ProductType` (Product kinds: consumable, non-consumable, non-renewing, auto-renewable). The store catalogue
  reads `product_type` (`StoreApp::product_type`); an unknown or missing kind is `None`. Decision: consumables are never
  restorable and only consumables repeat.
- `promotion::PurchaseIntents` (purchase intents from a promoted product). Decision: first in, first out, no duplicate
  product while it waits, at most 16 waiting (`MAX_PENDING`). `promotion::in_display_order` gives the promoted list order.
  Gap: the store does not send intents yet, so the queue has no producer and no SDK call that reads it.
- Not decided here: sync of transactions and subscription management (App Store interactions) stay gaps; they need a
  store account service that BifrOSt does not have yet.

## Drawing canvas models (reference pencil-canvas parity, `bifrost_app::drawing`, `tool_picker`, `lasso`)

Shared models in `springboard/ui/src/drawing.rs`, `tool_picker.rs` and `lasso.rs`, re-exported by the SDK as
`bifrost_app::drawing`, `bifrost_app::tool_picker` and `bifrost_app::lasso`. Pure state and geometry; the shell and
apps draw and capture touch.

- **Vector eraser.** `Drawing::erase_strokes_at(x, y, radius)` removes each stroke with a segment inside the radius, as
  a whole, and returns the count (the bitmap eraser is below).
- **Image render.** `Drawing::image(rect, scale)` returns a `Raster` of 0xAARRGGBB pixels, painted in stroke order with
  source-over blending. Colour alpha, per-point opacity, pressure width and pencil tilt width are rendered; marker blend
  modes are not. Empty, non-finite or over `2^26` pixel requests give an empty raster.
- **Tool picker.** `ToolPicker::new()` gives the default items (pen, marker, pencil, bitmap and vector eraser, lasso);
  `with_items` takes a custom set. `current_ink` gives the ink for a new stroke. Widths clamp to `width_range`.
  Decision: observers are not modelled; callers read `selected()` after each event.
- **Bitmap eraser.** `Drawing::erase_points_at(x, y, radius)` cuts the parts of strokes within the radius (segments are
  subdivided to half the radius first, capped at 1024 pieces a segment): a stroke the eraser crosses splits in two, one
  it grazes is shortened, one it covers goes. This is the `Bitmap` eraser of the canvas view; the picker still only
  records the choice.
- **Live stroke and tilt.** `Drawing::image_with(overlay, rect, scale)` paints strokes that are not yet in the drawing
  (the stroke being drawn). `tilt_width_factor(kind, altitude)` widens a pencil mark up to 2.5 times as the pen leans
  (altitude 0 is "no reading" and keeps the width); pens and markers ignore tilt. Marker blend modes are still not
  rendered.
- **Gaps.** No handwriting recognition or Scribble routing; no ruler. Parity rows stay partial until a Midget test
  passes.

### Canvas view (`bifrost_app::canvas_view::CanvasView`, `PKCanvasView`)

`springboard/sdk/src/canvas_view.rs`. The app-facing canvas that joins the models above.

- **Input.** `touches(&TouchEvent)` takes the app's touch frames in view points (`TouchEvent::in_view`): a mouse or
  trackpad is one `Indirect` touch, a stylus a `Pencil` touch with force. `stylus(&StylusReport, &StylusProfile, now_ms)`
  takes a pen digitizer's raw reports (pressure and tilt become each point's `force`, `altitude` and `azimuth`; the
  eraser end erases whole strokes whatever tool is selected; a report with the tip up ends the stroke). Only the first
  accepted touch draws; a cancel drops the stroke or puts an erase or move back.
- **Policy** (`drawingPolicy`): `DrawingPolicy::{Default, AnyInput, PencilOnly}`. On the desktop form factor
  (`bifrost_ui::form_factor`) the default accepts every input and a pen alone never turns fingers off; a mouse or
  trackpad is never refused. On a tablet the default stops a finger from drawing once a pencil has been seen.
- **Tools.** The canvas owns a `ToolPicker` (`tool_picker_mut`): pen, marker, pencil (each stroke records the ink's kind,
  colour and width and each point's time, force and tilt), vector and bitmap eraser (radius `set_eraser_radius`, a fast
  move still erases what it crossed), and the lasso (selects enclosed strokes, a press inside the selection drags it).
  Selection edits: `delete_selection`, `move_selection`, `scale_selection`, `select_all`.
- **Undo** (`undoManager`): `undo`, `redo`, `can_undo`, `can_redo`, `undo_titles` ("Undo Draw", "Undo Erase", "Undo Move",
  "Undo Delete", "Undo Scale"), `change_count`; 100 levels; an erase or move that changes nothing leaves no step.
- **Save and load** (`PKDrawing.dataRepresentation`, `PKDrawing(data:)`): `data_representation()` is the drawing's text
  form as bytes; `set_drawing_data` / `drawing_from_data` refuse over 32 MiB, non-text and malformed data (the canvas is
  unchanged on an error). A stroke keeps at most 20000 points.
- **Rendering.** `render(rect, scale)` and `render_view(width, height, scale)` return a `Raster` of the strokes and the
  stroke in progress; `zoom_scale` and `content_offset` map view points to drawing points.
- **Gaps.** The compositor does not route tablet-tool events (pressure, tilt, eraser end) to apps yet, so on the PC a
  pen arrives as a pointer without pressure; the stylus path is exercised by tests with synthetic reports only. No app
  uses the canvas view yet (Notes' sketch pad, Freeform and the shell's markup editor keep their own stroke code). No
  handwriting recogniser, Scribble routing, marker blend or scroll view.

Markup has a separate `bifrost_ui::markup_ink::InkCanvas`: pointer gestures make pen, marker and pencil strokes, the
eraser removes touched strokes, and committed edits have bounded undo. A gesture retains the tool it started with and
an eraser pass that removes nothing does not consume history. The shell markup editor still owns its own stroke list;
this model is not wired into that editor and does not model pressure input.

## Calendar access (`bifrost_app::calendar`, the `calendar` service, the `calendars` permission)

Decision (privacy first): `calendars` is a consent-gated permission, like `contacts`. The manifest declaration only lets
an app ask (`aap.py`, `app_policy::ENTITLEMENT_NAMES` and the SDK entitlement keys all list it); the person's answer
decides. `access` reports `not_determined`, `restricted`, `denied`, `write_only` or `authorized`; `request` queues the
Allow / Don't Allow sheet (read and write). A first `save_event` by an undecided app queues the add-only sheet and fails
until it is answered, so an app that only adds events (Mail's "Add to Calendar") never gets to read them: `events`,
`update_event` and `delete_event` need full access. Settings > Privacy & Security > App Permissions has a CALENDARS
group (on is full access, off is denied); answers are saved in `privacy-grants.tsv`. Model `bifrost_ui::calendar_access`,
service `springboard/src/shell/services/calendar.rs`. Gaps: reminders have no service yet, and no Midget test.

## Contacts access, fetch and save (`bifrost_app::contacts`, the `contacts` service)

Apps read and write the person's contacts only after the person allows it. `access` and `request` report the status
(`not_determined`, `restricted`, `denied`, `authorized`, `limited`); `fetch` returns the named fields (`given_name`,
`family_name`, `organization`, `phones`, `emails`, `address`, `note`) of each readable contact, sorted by name, with an
optional text query. `save` takes `add`, `update` and `delete` lists and applies them as one call: the reply lists the
ids the new contacts got, and a failing item changes nothing. Only full access writes; a limited grant is read-only.
The SDK side is `springboard/sdk/src/contacts.rs` (`SaveRequest`, `save`, `decode_saved`). The access model is
`bifrost_ui::contact_access`, the partial records are `bifrost_ui::contact_fetch`, and the shell side is
`springboard/src/shell/services/contacts.rs` (`apply_save` is pure; the service stores the book only on success).
Gaps: no access sheet or Settings row yet, grants reset on restart, no contact picker for apps, and one phone, one
email and one address per contact (a list of more is refused). See `docs/parity/sdk-app-services.md`.
Apps read the person's contacts only after the person allows it. `access` and `request` report the status (`not_determined`,
`restricted`, `denied`, `authorized`, `limited`); `fetch` returns the named fields (`given_name`, `family_name`,
`organization`, `phones`, `emails`, `address`, `note`) of each readable contact, sorted by name, with an optional text
query. The access model is `bifrost_ui::contact_access`, the partial records are `bifrost_ui::contact_fetch`, and the
shell side is `springboard/src/shell/services/contacts.rs`. `phones`, `emails` and `urls` come back as lists of `LabeledValue { label, value, identifier }` (labels `LABEL_HOME`,
`LABEL_WORK`, `LABEL_OTHER`, `LABEL_PHONE_MOBILE`, `LABEL_PHONE_MAIN`); `birthday` is `DateComponents { year?, month, day }` and
`dates` is a list of labeled `DateComponents`. A reply with plain strings still decodes, into one-entry lists with no label.
The first phone and email come from the contacts file, the rest from the sidecar `contacts-extra.tsv`
(`springboard/src/shell/contactextras.rs`). Gaps: no access sheet or Settings row yet, grants reset on
restart, no writes, no contact picker for apps. See `docs/parity/sdk-app-services.md`.

Errors: `contacts::ContactsError` is the typed failure of a contacts call (`NotGranted`, `ReadOnly`, `Restricted`,
`NothingPicked`, `AlreadyDecided`, or `Other` with the shell's message). `decode_error(&reply)` returns it for a failed
reply, and `ContactsError::from(contact_access::Error)` maps the access model's errors. The variants are matched by
name; Apple's `CNErrorCode` raw values are not mapped yet because they are not verified against the reference.

### Contacts: vCard import and export, and containers (2026-10-19, wave 15)

vCard through the service. `contacts::vcard::export(ctx, ids, version)` asks for the readable contacts as vCard 3.0 or 4.0
text (`decode_export` gives the text and the count); `contacts::vcard::import(ctx, text)` adds the people in the text
(`decode_import` gives `added`, `skipped` and `duplicates`). Export needs a read grant (a limited grant exports only its
contacts, and an id outside the grant is left out without an error); import needs full access and applies the whole text or
nothing. The shell side is `springboard/src/shell/contactcard.rs` (pure: `export`, `plan_import`) and the `export_vcard` and
`import_vcard` methods of `services/contacts.rs`. A card maps onto the store's one phone, email and address plus the extras
sidecar (more phones and emails, URLs, birthday, dates) and the picture store; a person already in the book (same name and the
same email or phone) is a duplicate and is not added again, a card with nothing to name it by is skipped and counted, and new
contacts get the next free ids and a new UID. Each import is an `added` event per contact in the change log.
Gaps: a name prefix, nickname (used only when there is no name), department, job title, relations, social profiles and instant
messages are read from a card but not stored, because the shell store has no field for them; the address is one line.

Containers (`CNContainer`). `contacts::containers` lists containers, their members, and the container of one contact, and with
full access adds, renames, removes and assigns. The local container (id 1, "On My Computer") always exists and cannot be
removed; every contact is in exactly one container, the local one unless assigned elsewhere; removing a container moves its
contacts home and deletes none. The store is the sidecar `contact-containers.tsv` beside the contacts file
(`springboard/src/shell/contactcontainers.rs`, service methods `containers`, `container_members`, `container_of`,
`container_add`, `container_rename`, `container_remove`, `container_assign`); deleting a contact removes it from its container,
and moving one is an `updated` event in the change log. Gaps: the groups are not scoped to a container (they stay book-wide),
and no account sync fills an `account` container yet, so only the local container is non-empty by default.

System search finds a contact by name, phone, email and company (notes and address stay out of Search by design).

## InputMethodKit: composing input methods (`springboard/src/shell/input_methods`)

Provider side of text input, pure Rust, no fcitx. `InputController` mirrors the reference controller:
`input_text(string, client)` (`inputText(_:client:)`), `handle(Key, client)` (`handle(_:client:)`),
`commit_composition(client)`, `composed_string()`, `cancel_composition(client)`. `TextClient` is the text field
side (`set_marked_text(text, selection, replacement)`, `insert_text(text, replacement)`); an empty marked string
clears the marking. `ComposingController<C: Composer>` turns a small state machine into a controller:
a non-composing key (space, digit, punctuation) commits the marked text and is then inserted; when idle,
`input_text` returns false so the client handles the key.

Sources (`sources()`, `new_controller(id)`): `bifrost.inputmethod.Korean.2SetKorean` (`HangulComposer`),
`bifrost.inputmethod.VietnameseIM.VietnameseTelex` and `...VietnameseVNI` (`VietnameseComposer`).
Hangul: 2-set keys incl. shifted Q W E R T O P, compound vowels and finals, the final moves on a following vowel,
backspace decomposes. Vietnamese: marks (circumflex, horn, breve, bar), five tones with modern placement,
tone removal (`z` or `0`), repeated keys revert, backspace replays the remaining keys.

Gaps: not wired into `kb_input.rs` (follow-up: look a controller up by source id and route keys), ranges count
characters rather than UTF-16 units, no candidates window, no Hanja conversion, no ancient jamo, no Simple Telex
or VIQR, no modifier/command events. Host tests only; parity rows stay partial until a Midget check.

## Cursors (`NSCursor`, `ui::cursor`, `ui::cursor_art`)

`Kind` now covers the AppKit set: `ResizeNorthEastSouthWest`, `ResizeNorthWestSouthEast`, `ColumnResize`, `RowResize`,
`Move`, `Rotate`, `ScreenshotWindow` next to the existing shapes (service names `resize_north_east_south_west`, `move`, ...).
Constructors mirror AppKit: `Kind::frame_resize(FrameResizePosition, FrameResizeDirections)` (`NSCursor.frameResize(position:directions:)`;
one-way edges give the one-sided arrow, corners the diagonal shape), `Kind::column_resize(ColumnResizeDirections)` and
`Kind::row_resize(RowResizeDirections)`. `ZoomIn`, `ZoomOut`, `DisappearingItem` and `OperationNotAllowed` already existed.

`cursor_art::art(kind, scale)` draws each shape from vector paths into two non-overlapping alpha masks (`fill`, `outline`) at 1x or 2x,
with the hotspot in pixels. `CursorArt::compose(outline_rgba, fill_rgba)` recolours the layers separately, which the pointer colour
setting (Accessibility > Display > Pointer) needs; `PointerColors::default()` is a white outline over a black fill.
Not matched yet: the original PDF art (ours is code-drawn), per-corner rotate variants, vertical I-beam, and wiring the art into the compositor.
Parity rows stay `partial` until checked on Midget.

## QuickLookThumbnailing: per-type providers

The `thumbnails` service (`shell/services/thumbnails.rs`) draws by content type with the providers in `springboard/src/thumbs/`: text, rich text, web (title and text), image, font, office (embedded picture), contact, calendar, clippings, audio/video and archives. Unknown types answer with their icon.

- **PDF** (`thumbs::pdf`): the first page, drawn by the PDF renderer (`cg::pdf::draw::render_file_page`).
- **Audio and video** (`thumbs::media`): the embedded cover, else a video's poster frame (first key frame, decoded by `media::hwdec`), else a waveform card (`thumb_providers::waveform_card`). Each carries the running time as a badge in the bottom right corner (`thumbs::duration`: `duration_secs` reads the MP4 movie header or the audio decoder's declared length, `format_duration` writes `m:ss` or `h:mm:ss`, `badge` draws the pill; none under 48 px).
- **Archives** (`thumbs::cards::PackageProvider`): zip and plain tar get a card with the format badge and the entry count (`thumb_providers::archive_entries`); other formats keep the icon with a badge.
- **Representation types** (`QLThumbnailRepresentation.RepresentationType`): one model, `bifrost_ui::thumb_service::RepresentationType` (`Icon`, `LowQualityThumbnail`, `Thumbnail`), re-exported by `bifrost_app::thumbnails` and `shell::services::thumb_providers`. The tiers are planned by the client: `GeneratorRequest::plan` turns each wanted tier into one `thumbnails` service request (the icon kind, a low quality request at a quarter of the edge with a 16 px floor, and the full size), so the service needs no per-tier parameter.
- **Generator with update handler** (`bifrost_app::thumbnail_generator::ThumbnailGenerator`, `QLThumbnailGenerator.generateRepresentations(for:update:)`): runs several generations at once (`generate`), routes each service reply or `ready`/`failed` event to the generation that asked for that file and edge (`accept`), reports `Update::Representation` cheapest tier first and never a worse one after a better one, `Update::Failed` with a `ThumbnailError` code, and supports `cancel`. At most 256 generations run.
- **Provider extensions** (`QLThumbnailProvider`, `QLFileThumbnailRequest`, `QLThumbnailReply`): an app declares `[[preview]]` for its file types in `manifest.toml` and answers the control call `preview.render` `{file, width, height}` with `{"picture": "<abs PNG path>"}`; `thumbnail_generator::FileThumbnailRequest::parse`, `ThumbnailReply::picture` and `answer_provider_call` are the app side. The `thumbnails` service asks the app for Files and Spotlight thumbnails too (not only the Space panel). An app whose declared type is at least as specific as the built-in provider's (`Registry::distance_for`, `preview_gen::Generator::distance`) is asked first; a broad type such as `public.data` never takes over the system's picture, text or PDF thumbnails. When the app gives no picture in time the built-in provider draws the file, and a type nothing draws shows its icon. Not built: replies that carry data or a drawing callback instead of a PNG path.
- **Where thumbnails show**: Files (icon, list, column and gallery views), the Space preview panel, and now Spotlight file rows (`shell::search_thumbs::FileThumbs`: asked while the rows are laid out, drawn on two worker threads through the shared disk cache, painted in place of the Files icon; folders and types without a picture keep the icon). Searchable items an app indexed keep drawing their own `thumbnail` PNG.
- **Gaps**: Live Photo and USDZ thumbnails (no such files on the PC); the system does not wait on an app generator for icon-kind requests; error codes are BifrOSt's own. Parity rows stay `partial` until checked on Midget; Office stays as the embedded-picture case only.

## System Settings: Mouse pane (`springboard/src/shell/mouse_page.rs`)

The Mouse pane is its own page (`PageId::Mouse`, `x-apple.systempreferences:com.apple.Mouse-Settings.extension`), separate from Trackpad.
Rows: Tracking Speed, Pointer Acceleration, Scrolling Speed, Double-Click Speed, Natural Scrolling and Secondary Click
(Click on Right Side / Click on Left Side). Constants (`DOUBLE_CLICK_RANGE_MS`, `SECONDARY_BUTTON`) and the slider mapping live in
`mouse_page.rs`; `Options` stores `mouse_speed`, `mouse_scroll`, `double_click_ms`, `mouse_accel` and `swap_buttons` (settings keys
`mouse_speed`, `mouse_scroll_speed`, `double_click_ms`, `mouse_acceleration`, `swap_buttons`). Tap to Click and the two-finger secondary
click stay on Trackpad only. Gaps: Pointer Acceleration and Double-Click Speed are stored but not yet read by the input layer or the
shell's double-click detection (`pointer_prefs::Tuning` has no field for them); Natural Scrolling is one setting shared with Trackpad,
where Apple keeps one per device. Audit of the other panes (Trackpad, Print & Scan, Sharing, Internet Accounts, Users & Groups): the
pages exist (`pages.rs`, `print_scan_page.rs`, `sharing_page.rs`, `internet_accounts_page.rs`, `users_page.rs`); their parity rows are
`partial` with the missing parts listed, none `parity` until checked on Midget.

## System Settings: shared list rows (`springboard/src/shell/list.rs`)

Every Settings list page draws through `list::Item`. Two accessories were added for the per-app lists. Existing rows
are unchanged unless they ask for them.

- **Leading app icon.** `Item::app_icon(app_id)` draws the installed app's icon at the detail row's icon token
  (`ROW_ICON`, 23 pt, at `ROW_ICON_X`), through `apphost::icon_image`, which calls `bifrost_render::icon_doc::app_icon`
  (see "Layered app icons"). The label moves to `ROW_LABEL_X`, as for a row with a symbol tile. A built-in app has no
  package, so its slot is blank and the label still moves in. Sidebar rows do not take the app icon.
- **Trailing pop-up button.** `Item::popup(choices, selected, action)` sets `Accessory::Popup`. The button shows the
  chosen label in the label colour, then up and down chevrons at the row's right edge (`popup_layout`). Tapping the
  row opens a menu (`ListPage::open_popup`) built on the kit's menu model (`bifrost_ui::menu::Menu::popup`) and placed
  by `popup_origin`: the selected item sits over the button and the menu stays below the bar. The menu is a translucent
  card (`MATERIAL_MENU`, 14 pt radius) with a checkmark on the chosen item and the touched item filled with
  `MENU_HIGHLIGHT`. Its metrics are the desktop menus' (24 pt rows, 11 pt separators, 5 pt padding).
- **Choosing.** A touch on a menu row is `Hit::Choice(k)`; a touch elsewhere is `Hit::Dismiss` and closes the menu. A
  touch on the menu's padding or separators does nothing. `Settings::activate` closes the menu and sends
  `pages::chosen(&row.action, k)`, the row's action with the choice in its place. Only the location access edit maps a
  choice today (`Edit::Access`, index into `LocationAccess::ALL`); other actions map to None and the touch does nothing.
  A restricted app's access popup is dimmed (`Tint::Dim`) and does not open.
- **Per-app lists.** Location Services (`location_page.rs`): each app's row is its icon and an access popup (Never,
  Ask Next Time, While Using the App), replacing the pushed checkmark page (`PageId::LocationAccess` is gone). The
  Precise Location switch and "Used in the last 24 hours" rows stay. App Permissions (Camera, Microphone, Screen
  Recording, Location) and Background App Refresh show each app's icon on its switch row. They keep switches: the store
  records only allowed or not allowed for them, with no Ask state, so a two-item popup would be a switch in disguise.
  Screen Time's allowed-apps list is not converted.

Gaps: icons come from the installed package only (no fallback glyph for a built-in app), and the popup has no keyboard
path (arrow keys and type-to-select of the kit menu are not wired to the list). Host-tested only: the layout and hit
tests are in `list.rs`, the choice mapping in `pages.rs` and `location_page.rs`. Parity rows stay `partial` until
checked on Midget.

## Launch agents and daemons (OS daemons, role map)

No new API. The shell performs the roles of the reference launchd agents (Dock, Finder desktop, controlcenter,
notificationcenterui, usernotificationsd, WindowManager, WindowServer, Spotlight, corespotlightd, screencaptureui,
wallpaper, loginwindow, TextInputSwitcher, battery menu extra, login item registration, shared file list) inside one
process, so these rows in `docs/parity-matrix/os/daemons/` are `partial` with the filling module in `bifrost` and the
missing parts in the note. The table is in the `services/mod.rs` doc comment; `springboard/tests/daemon_map.rs`
checks that every partial row cites an existing path and has a note. Siri, family controls, iCloud, search-party
and slot-notification agents have no counterpart and stay `gap`. The Screen Time agent (`com.apple.FamilyControlsAgent`)
is `partial`: the shell fills its on-device role (see "Screen Time" below). Parity needs a Midget check.

## AppKit: named alert sounds (`bifrost_sdk::system_sound`, `shell::alert_sound`)

`SystemSounds::named(name)` mirrors `NSSound(named:)` for the system alert names: Basso, Blow, Bottle, Frog, Funk, Glass, Hero, Morse, Ping, Pop, Purr, Sosumi, Submarine and Tink (plus Bell, Chime, Pulse, Tick). It returns a `SoundId` for a listed name and `None` for any other (`ALERT_NAMES` lists them). The id plays like any created sound. The shell synthesises each tone from a note recipe (additive sine notes and noise bursts, under 1.5 s, no clipping), so the sound is an equivalent, not the original recording. Settings > Sound > Alert Sound lists all of them (`options::ALERT_SOUNDS`). Status: partial until checked on Midget.

## Core Location (`bifrost_app::core_location`, the `location` service)

`LocationManager` mirrors `CLLocationManager`: `authorization_status`, `request_when_in_use_authorization`,
`start_updating_location` / `stop_updating_location`, `request_location`, `desired_accuracy` (`accuracy::*`),
`distance_filter`, and a `Delegate` with `did_update_locations`, `did_fail_with_error` and `did_change_authorization`.
`Location` mirrors `CLLocation` (coordinate, horizontal accuracy, timestamp, `distance_from`); `reverse_geocode` returns a
city-level `Placemark`. Authorization is the privacy grant (Settings > Privacy > App Permissions > Location); errors are
`bifrost_ui::location_auth::LocationError`. The shell side is `springboard/src/shell/services/location.rs`.

Privacy model (owner decision, follows the reference platform):
- **Global switch**: `location.services.enabled` (default `1`); `0` denies every app whatever its grant
  (`services::location::effective_status`).
- **Precise Location per app**: `location.precise.<app id>` (default off = approximate). An approximate app gets its
  coordinates moved to the centre of a 0.027 degree grid (about 3 km, `location_auth::approximate`) and an accuracy of
  3 km (`location_auth::APPROXIMATE_ACCURACY`). Precise apps get the fixed fix unchanged.
- **Purpose string**: the app's manifest `[privacy] location = "..."` (`Manifest.location_purpose`). A `location_request`
  without it is refused by `privacy_permissions::location_purpose_gate` (built on `location_auth::request_purpose`), so no
  prompt shows without a reason. The request sheet does not show the text yet.
- **Retention**: the last fix lives in memory only. The service records when each app last got a fix
  (`services::location::record_use` / `last_use_of`, a thread-local map), in memory, never on disk and never a location
  history. It drives "Used in the last 24 hours" on the Location Services page.
- **No third-party lookup**: the IP-address estimate (`location.ip.*`) was removed. bifrost-cloud has no geo endpoint
  (checked `~/Projects/bifrost-cloud`: `device_locations` is encrypted sharing data, not IP geolocation), so there is no
  IP source. GeoClue2 is not built (no D-Bus client for it is in use; `zbus` exists only for AT-SPI and Wayland).

- **Menu bar indicator state**: `Location::in_use(t, ACTIVE_WINDOW)` lists the apps with a live watch or a fix in the
  last 5 s, sorted. The menu bar arrow (springboard/src/shell/desktop_bar.rs) shows while that list is not empty; its
  menu lists those apps and ends with "Location Services Settings…", which opens the page below.
- **Settings > Privacy & Security > Location Services** (`springboard/src/shell/location_page.rs`): the global switch
  (footer: turned off, every app is denied, and a watching app gets `didFailWithError` on the next tick); System
  location (Set Location… asks for latitude then longitude in two one-field sheets, validated to -90..90 and
  -180..180, a decimal comma is accepted; Clear removes it; no network geocoding, so the place has no city or
  country); and one row group per app that has an answer in the privacy store (`privacy_permissions::location_asked`):
  an access pop-up on the row with Never, Ask Next Time or While Using the App (Ask Next Time forgets the answer so the
  next request shows the sheet again; see "System Settings: shared list rows"), a Precise Location switch
  (`location.precise.<id>`), and "Used in the last 24 hours" (a check while the last fix is within 24 h). Pref writes
  go through `Effect::Location`, which the shell saves to `settings.ini`. Apps that have only been asked, and not
  answered, are not listed: the store keeps no entry for them. Gaps: the access popup has no Allow Once choice, and the
  page is host-tested only until checked on Midget.

Sources: one, the fixed location from the shell prefs (`location.manual.*`, 100 m), set in Settings > Privacy & Security
> Location Services. Gaps still open: the city and country of the fixed place (no geocoding, so they stay empty), the
Allow Once choice on the request sheet (the sheet is two buttons: Don't Allow and Allow; Allow Once needs a third
button and a session-only grant; the Location Services popup has no Allow Once either), the purpose text on the sheet,
and the Weather "My Location" wiring (its LOCATION constant is still `NotDetermined`, because the weather app does not
run with an SDK `Ctx`).

Altitude is always absent; reverse geocoding knows only the fixed place within 50 km, not a worldwide gazetteer;
no heading, regions or background updates. Host-tested only (not run in this round); the rows stay partial until
checked on Midget.

## Core Image: distortion, stylize, halftone, tile and the filter catalogue (`bifrost_render`)

CPU filters on `gfx::Image` (premultiplied RGBA in a `u32`), host-tested, not yet verified on Midget, so the
`core-image.json` rows stay `partial`. Parameter names follow the reference input keys.

- `distortion_filters`: `twirl`, `vortex`, `pinch`, `bump` (center, radius, angle or scale) and `glass`
  (texture-slope displacement). Backwards warp with bilinear sampling; zero strength is the identity.
- `stylize_filters`: `unsharp_mask`, `sharpen_luminance`, `bloom`, `gloom`, `pixellate`, `crystallize`,
  `pointillize`.
- `halftone_tile_filters`: `screen` (`Screen::Dot|Line|Circular|Hatched`, `ScreenParams` center, angle, width,
  sharpness), `cmyk_halftone`, `affine_tile`, `op_tile`, `kaleidoscope`.
- `filter_catalogue`: `filter_names(category)`, `attributes(name)`, `default_value(name, key)` for every
  filter built, with the `CICategory...` constants.

Decision and gaps: `CIDetector` stays a gap (the only QR decoder is in the shell crate, no face, rectangle or
text detection exists). `CIRenderDestination` and depth effects stay gaps (need the CIImage/CIContext model).
Perspective and triangle tiles, displacement, circle splash and torus lens are not built. Falloffs are linear
in the distance from the centre, which may differ from the reference curve; check on Midget before promoting.

## Foundation and Core Foundation small gaps (`predicate_validating`, `cf_bundle`, `dispatch_io`, `app_events`)

- **Foundation: `bifrost_app::predicate_validating`** (`NSPredicateValidating`). `Predicate` (comparison, `And`, `Or`, `Not`),
  `PredicateVisitor`, `ValidationPolicy::new().allow("title", &[Operator::Contains]).allow_key("date")` and the
  `PredicateValidating::validate` trait method. Unknown keys give `DisallowedKey`, an operator outside the list gives
  `DisallowedOperator`, an empty compound gives `Empty`. There is no predicate parser or evaluator yet.
- **Core Foundation: `bifrost_app::cf_bundle`**. `CfBundle::create(dir)` (`CFBundleCreate`) reads the top-level keys of
  `manifest.toml`; `value_for_info_dictionary_key("CFBundleName")` maps the CF names to manifest keys (`id`, `name`,
  `version`, `exec`, `icon`, `category`, `min_shell`) and also takes the manifest spelling; `resource_url` defers to
  `bundle_resources`. `PlugInRegistry` holds `CFPlugIn` factories (`register_factory`, `register_type`,
  `create_instance`, `factories_for_type`, `unregister_factory`). Plug-ins are in-process; nothing is loaded dynamically.
- **Dispatch: `bifrost_app::dispatch_io`** (`DispatchIO`). `DispatchIO::new(StreamType::Stream | RandomAccess, file)`
  over a file or pipe end. `read(offset, length, queue, handler)` calls `handler(done, data, error)` per chunk, then once
  with `done == true`; `write(offset, data, queue, handler)` hands back the unwritten remainder. `set_limit` is the
  high-water chunk size, `set_low_water` batches chunks, `close` ends operations with `ECANCELED` (125).
- **Core Services: `bifrost_app::app_events`** (`AEDesc`, `AESendMessage`, `AEInstallEventHandler`). `AppEvent::new(class,
  id, record)` travels as the control-socket method `ae.<class>.<id>`; `send_message(socket, &event, timeout)` returns an
  `AppEventReply` (`error_number` 0 on success, `-1708` not handled, `-1712` timeout, `-609` unreachable).
  `AppEventHandlers::install(class, id, handler)` and `rpc(method, params)` serve events from `Application::rpc`.

All four are host-tested only; rows stay `partial` until checked on Midget.

## Video Toolbox: compression session (`bifrost_sdk::video_compression`)

- `CompressionSession::create(w, h, Codec, callback)` mirrors `VTCompressionSessionCreate`; `set_property` / `copy_property` check keys (`key::*`, the `kVTCompressionPropertyKey_*` names), types, ranges and read-only keys; `prepare_to_encode_frames`, `encode_frame(frame, pts, duration)`, `complete_frames`, `invalidate`. Time stamps must increase; output order is submission order.
- The only encoder is lossless intra-only (one PNG per frame, all key frames). `Codec::H264` and `Codec::Hevc` fail with `Status::CouldNotFindVideoEncoder`.
- Multi-pass: `FrameSilo` (`add_sample`, `for_each_sample`), `MultiPassStorage` (`set`, `get`, `close`, `to_bytes` / `from_bytes`), and `set_multi_pass`, `begin_pass`, `end_pass` (true when another pass is wanted).
- Frame processing: `FrameProcessor` trait, `FrameProcessorSession` (start, process, end) and `SuperResolutionScaler` (integer upscale); `set_frame_processor` runs it before encoding.

## Compression: algorithm selection (`bifrost_app::compression`, `bifrost_app::compression::lz4`, `bifrost_app::compression::lzma`)

- `Algorithm::Lzma` works in `encode_buffer` / `decode_buffer` / `decode_buffer_limited`, `Stream` and the input and output filters. Like LZ4 it has no incremental codec here: the stream or filter buffers its input and runs the one-shot codec when it ends. The buffer form is a whole xz stream (`compression/lzma.rs`): the stream header and footer, one block whose filter is LZMA2 (dictionary property 22, the 8 MiB of preset 6) with a CRC64 check, and the index. The level and the thread count are ignored.
- The LZMA decoder takes streams with a CRC32, CRC64, SHA-256 or no check, and any number of blocks. It accepts every LZMA2 chunk kind: uncompressed chunks with or without a dictionary reset, and LZMA chunks with no reset, a state reset, new properties or a dictionary reset. It checks the stream flags, the block headers, the block checks, the index records and CRC, and the footer, and it refuses output past the limit with `Error::TooBig`. Its dictionary is the output so far, so a distance is checked against the bytes produced. `lzma::crc32` (IEEE, reflected) and `lzma::crc64` (ECMA-182, reflected, the xz check) are public; SHA-256 comes from `digest::sha256`.
- The LZMA encoder is a greedy matcher with hash chains over 3-byte prefixes and a range encoder, at lc=3, lp=0, pb=2. It writes literals and simple matches only (no rep matches). Chunks are 64 KiB; a chunk that does not shrink is written as an uncompressed LZMA2 chunk, and the model starts again after it. Correctness is the goal, not ratio.
- LZMA gap: no xz stream made by an external tool (xz-utils) is in the repository, so there is no external test vector and nothing here is checked against another xz implementation. The tests (`springboard/sdk/tests/compression_lzma.rs`, inline tests in `lzma.rs`) cover round trips (empty, small, repetitive, pseudo-random, past 1 MiB), the CRC32 and CRC64 check values, refusals (bad magic, bad CRC, bad footer, every truncation) and the output limit. Do not treat the bytes as xz-compatible until an external tool reads them.
- `Algorithm::Lz4` works in `encode_buffer` / `decode_buffer` / `decode_buffer_limited`, `Stream` and the input and output filters, the same way Zlib and Deflate do. The buffer form is the `bv41` (compressed, with decoded and encoded sizes), `bv4-` (stored) and `bv4$` (end marker) framing, in 64 KiB chunks.
- `compression::lz4::encode_frame` / `decode_frame` are the LZ4 frame format (magic, FLG and BD, xxHash32 header checksum, block and content checksums). `xxh32` is exposed too.
- The encoder is a greedy hash-table matcher with a 64 KiB window, minimum match 4, the last 5 bytes as literals, and the last match starting at least 12 bytes before the end. The decoder is bounds-checked and rejects zero or out-of-range offsets, truncation, and output over the limit with `Error::TooBig`.
- Still `Unsupported`: LZFSE, LZBITMAP, Brotli. `Algorithm::is_supported` is the source of truth.
- Status: host unit tests and `springboard/sdk/tests/compression_lz4.rs` written, and `springboard/sdk/tests/compression_lzma.rs` with the LZMA unit tests, not yet run (the build round is pending). Not yet exercised on Midget. The parity row stays `partial`.

## Core Audio: aggregate devices, process objects, process taps (`bifrost_app::audio_aggregate`, `bifrost_app::audio_process`)

Apple reference: `AudioHardwareCreateAggregateDevice` and the `kAudioAggregateDevice*Key` dictionary,
`kAudioDevicePropertyClockDomain`, `kAudioHardwarePropertyProcessObjectList`, the audio process properties and
`CATapDescription` with `AudioHardwareCreateProcessTap`. Pure models, tested without hardware:

- `audio_aggregate::AggregateDescription` (name, uid, sub-devices with drift flag, main sub-device, clock device,
  private, stacked) validates and serialises with Apple's keys (`name`, `uid`, `subdevices`, `master`, `clock`,
  `private`, `stacked`, per sub-device `uid` and `drift`). `ClockDomain` and `drift_compensation_needed` say which
  sub-devices sit in another clock domain than the clock source.
- `audio_process::ProcessObject` (pid, bundle id, running input and output) with `list_request` and
  `decode_processes`; `TapDescription` has the four `CATapDescription` initialisers, `captures(pid)`, mute behavior
  (`Unmuted`, `Muted`, `MutedWhenTapped`) and a wire shape.

Gaps, stated honestly: audiod has no command for any of this yet, so nothing creates an aggregate device, reports
processes or captures audio; drift compensation is a flag only; driver plug-ins and boxes stay gap. Rows in
`core-audio.json` are partial until a Midget check.

## Speech recognizer (`bifrost_app::speech_recognizer`)

`SFSpeechRecognizer` mirrors Apple's legacy recognizer: `request_authorization` records the person's answer once
(`NotDetermined`, `Denied`, `Restricted`, `Authorized`), `is_available` asks the backend, and `recognition_task` /
`recognition_task_url` start a task only when authorized and available. Audio requests take `AnalyzerInput` buffers
through `append` and `end_audio`; the caller drives the task with `pump`, which delivers partial results (when
`should_report_partial_results`) and one final `SFSpeechRecognitionResult` with `best_transcription` and timed
`SFTranscriptionSegment`s. `cancel` reports `Canceled` once. Gaps: no recognition engine ships, so a `RecognitionBackend`
must be supplied; no alternatives beyond the best transcription; no dictation entry point on the keyboard. Rows stay
partial until a Midget check.

## Speech service (`bifrost_app::speech`, the `speech` service)

Text to speech for apps, the BifrOSt counterpart of the reference speech synthesizer. The shell owns the engine:
`espeak-ng`, else `espeak`, a system package. The shell runs it as a child process and feeds it the text on standard
input, written from a thread so the shell never blocks on a long text.

- Calls: `speak {text, voice?, rate?}` replies `{id}`; `stop` replies `{stopped}`; `pause` and `continue` (SIGSTOP and
  SIGCONT to the engine) reply `{paused}`; `is_speaking` replies `{speaking, paused}`; `voices` replies
  `{voices: [{name, language}], loading}`. The voice listing is read once in a thread on the first call, so apps ask
  again while `loading` is true.
- One utterance per app. A new `speak` replaces the current one, `stop` ends it, and the shell ends it when the app
  quits. An engine that has exited stops counting as speaking.
- Limits: text is at most 16 KiB, the rate is a whole number from 80 to 450 words per minute, and a voice name may not
  start with a dash or contain spaces. No engine installed fails `speak` and `voices` with "no speech engine is
  installed (install espeak-ng)".
- Gaps against the reference synthesizer: no word or sentence events, no pitch or volume, no delegate callbacks, no
  per-utterance language (the voice carries it), and no pause of an engine the shell did not start.
- Code: `springboard/src/shell/services/speech.rs` (service: the state machine behind a `Spawner` trait, so tests run
  with a fake engine), `springboard/core/src/speech_engine.rs` (engine programs, `engine_args`, `parse_voices`, shared
  with the `say` tool in `springboard/cmd/src/say.rs`), `springboard/sdk/src/speech.rs` (client).
- Parity: `docs/parity-matrix/data/avfaudio.json` (speech synthesis row, partial). Notes and TextEdit have Speech menus
  (Note > Speech, Text > Speech) rather than Edit > Speech, because the shell's Edit menu takes no app items.
- Not verified on Midget: the menus and the engine path need espeak-ng installed on the device.
- Viewer (Preview) has Preview > Speech > Start Speaking, which speaks the PDF selection or else the page text
  (`apps/preview-desktop/src/find.rs` `speech_text`).

## Screenshot service (`bifrost_app::screenshot`, the `screenshot` service)

Pictures of the screen for any app (no manifest permission: the shell takes the picture, and the person's screen-recording consent is asked on the first request). The shell takes the picture the way the screenshot toolbar
does and saves it to Photos. Methods: `selection()` (drag a rectangle), `window()` (the window pick, with its shadow),
`screen()` (the whole screen now). Each replies `{started: true}`, or an error for a locked screen, a missing permission
or a capture already on screen. When the file has landed, the shell sends the event `captured` with body `{path}`; an app
opens the path itself. A cancelled selection or window sends nothing; a file that does not land within 10 s is dropped.
One request at a time.

- Gaps: the file is in the Photos folder, not the app's data folder; no delay timer, no pointer option, no recording
  (the toolbar keeps those); the event arrives on the shell tick after the file exists, not at the save.
- Code: `springboard/src/shell/services/screenshot.rs` (`Shots` is the pure state machine; the service calls
  `Shell::capture_request` and `take_screenshot`, and reads `Shell::shot_saved`), `springboard/sdk/src/screenshot.rs`
  (client: `take`, `started_from_reply`, `captured_path`).
- Parity: Viewer's File > Take Screenshot items (`apps/preview-desktop/src/main.rs`), row "Take Screenshot
  (selection/window/screen)" in `docs/parity-matrix/apps/preview.json`, partial. Not verified on Midget.

## Local Authentication (`bifrost_app::local_auth`, the `local_auth` service)

`LAContext` mirrors Apple's type: `can_evaluate_policy` (canEvaluatePolicy), `evaluate_policy` (evaluatePolicy with a
reason), `invalidate`, `localized_fallback_title`, `localized_cancel_title`, `touch_id_authentication_allowable_reuse_duration`
(a success inside the window skips the next prompt) and `biometry_type` (always `None`). `LAPolicy` has
`DeviceOwnerAuthentication` and `DeviceOwnerAuthenticationWithBiometrics`; `LAError` carries `AuthenticationFailed`,
`UserCancel`, `UserFallback`, `SystemCancel`, `PasscodeNotSet`, `BiometryNotAvailable`, `BiometryNotEnrolled`,
`BiometryLockout`, `AppCancel`, `InvalidContext` and `NotInteractive`. The shell service
(`springboard/src/shell/services/local_auth.rs`) has `can_evaluate` and `evaluate`; it checks the account password with
the login backend (read-only) and locks an app out after five failures. Gaps: the password sheet is not drawn, so
`evaluate` without a `password` returns `not_interactive` with `prompt: true`; no biometry (the biometrics policy is
`biometry_not_available`); the fallback and cancel titles are held by the context but not yet shown; grants and the
failure count reset on shell restart. Rows stay partial until a Midget check.

## Network: NWListener (`bifrost_app::network_listener`)

`Listener::new(parameters, port)` mirrors `NWListener(using:on:)`; port 0 lets the system choose and `port()` returns the
bound port once ready. `start()` binds with `std::net` (TCP or UDP, all IPv4 addresses) and moves `State` from `Setup`
to `Ready`, or to `Failed(reason)` when the bind fails; `cancel()` closes the socket and moves to `Cancelled`. Handlers
(`set_state_update_handler`, `set_new_connection_handler` giving `Accepted { stream, remote }`, and
`set_new_datagram_handler` giving `Datagram { from, data }` for UDP) fire from `poll()`, which the app calls from its run
loop and which never blocks. `set_new_connection_limit(Some(n))` is `newConnectionLimit`: extra peers are closed at once.
`Service::new(name, kind, domain, txt)` is `NWListener.Service`; `advertiser(host, addresses)` returns the
`service_discovery::ServiceAdvertiser` for the bound port, and the app answers mDNS queries with it. Gaps: TLS listeners
are refused (`ListenerError::TlsUnsupported`), no IPv6 or interface selection, no automatic Bonjour publishing, no
`NWConnection` wrapper around `Accepted` yet (the app owns the `TcpStream`). Host tests only; not yet checked on Midget.

## Bundle Resources: URL types (`bifrost_app::bundle_resources`, the `launch` service)

- `URLType::parse_all(&Json)` reads `CFBundleURLTypes` into `URLType { url_name, url_schemes, type_role: BundleTypeRole, handler_rank: HandlerRank }`. Entries without schemes are dropped and schemes are lowercased.
- `url_scheme(url)` gives the scheme of `myapp://x`.
- The `launch` service method `application_for_url {url}` answers `{app, apps}`: `urlForApplication(toOpen:)` (the user's default, else the first declaring app) and every declaring app, default first. Web schemes still go to the browser. No universal links.

## Progress tree: cancel, pause and handlers (`bifrost_ui::progress`, Foundation `Progress`)

- `ProgressTree::cancel` reaches every descendant (cancellable or not), flagging each and firing its
  `set_cancellation_handler` once; `pause` and `resume` do the same with `set_pausing_handler` and
  `set_resuming_handler`. The root still needs `set_cancellable` / `set_pausable`, as in Foundation.
- `add_child(parent, child, pending)` (`addChild(_:withPendingUnitCount:)`) attaches a root; cycles and re-parenting are refused.
- `become_current(id, pending)`, `current()`, `implicit_child(total)` and `resign_current(id)` mirror
  `becomeCurrent(withPendingUnitCount:)`, `Progress.current()` and `resignCurrent()`: the first implicit child takes the pending
  units, otherwise resigning completes them on the object.
- Gap: publishing progress to other processes. Row status stays partial until checked on Midget.

## URL bookmarks and FileWrapper (Foundation parity)

One implementation of each, described under "Files and data persistence" above: `bifrost_app::bookmark`
(`bookmark_data(path, options, keys, relative_to)`, `resolve_bookmark(data, options, relative_to)`, security scopes now
supported) and `bifrost_app::file_wrapper::FileWrapper` (`read_from`, `write_to(path, WritingOptions, original_contents)`,
`matches_contents_of`, `needs_to_be_updated`).

## Window server session parity: key bindings and frame capture (`bifrost-wm`)

Mirrors WindowServer global hotkeys and ScreenCaptureKit / `CGWindowListCreateImage` single-frame capture. Status stays partial until checked on Midget.

- `keybindings::action(keysym, Mods) -> Option<Action>` (pure): Super+Space `QuickSearch` (sends `quicksearch` to the shell control socket `/run/bifrost/shell-control.sock`), Ctrl+Up and F3 `MissionControl` (send `overview` to the control socket; Super+A is unbound so Cmd+A reaches the app), Alt+Tab `NextWindow`, Alt+F4 `CloseWindow` (xdg close request to the focused toplevel), Ctrl+Super+Left/Right/Up/Down `Tile(Arrow)` (half, half, maximize, centre via `tiling.rs`). Modifiers match exactly. `WmState::key` takes these chords before any client sees them; the action runs on the press and the release is swallowed too.
- Screencopy: `session_protocols::write_shm` now writes into the client's wl_shm buffer. The `capture` hook returns RGBA rows (R, G, B, A in memory); `rgba_to_shm` converts to XRGB8888 (alpha forced to 0xff) or ARGB8888 (B, G, R, A in memory, the fix for swapped red and blue), honouring the buffer stride; `crop_rgba` clips a region out of a full-output frame. Other formats are refused.
- Capture backends: `wm/src/capture.rs` implements the `capture` hook for the DRM and headless GLES backends (the composed scene as RGBA rows, with the pointer when asked); the Vulkan presenter does not serve it. Not done: cursor capture sessions and ScreenCaptureKit streams. Not checked on Midget.

## Recurring events (EventKit: `EKRecurrenceRule`, `EKEventStore.events(matching:)`)

`bifrost_app::event_recurrence` (`springboard/sdk/src/event_recurrence.rs`) is a pure model with host tests; status
partial (not wired into the shell calendar store or the Calendar app, not yet checked on Midget).

- `RecurrenceRule` (`frequency`, `interval`, `days_of_the_week: Vec<RecurrenceDayOfWeek>`, `days_of_the_month`,
  `months_of_the_year`, `set_positions`, `recurrence_end: Option<RecurrenceEnd>`), `parse`/`format` for the RFC 5545
  RRULE subset FREQ, INTERVAL, COUNT, UNTIL, BYDAY, BYMONTHDAY, BYMONTH, BYSETPOS. `RecurrenceEnd::EndDate(date)` and
  `OccurrenceCount(n)`. `RecurrenceDayOfWeek { day_of_the_week (Sunday 1), week_number }`.
- `EventStore::predicate_for_events(start, end, calendars)` and `events_matching(&predicate)` return one `Occurrence`
  per repeat overlapping the window, with `occurrence_date` (the original start) and `is_detached`.
- `Event.excluded_dates` skip repeats (they still count toward `OccurrenceCount`); `EventStore::detach` replaces one
  repeat with its own time, length and title. `EventStore::remove(id, occurrence_date, Span::ThisEvent | FutureEvents)`.
- Times are floating (`DateTime` = date + minute of day), so a 09:00 event stays at 09:00 across a clock change.
  Weeks start on Monday. The first occurrence is always the start. `RecurrenceRule::from_repeat_choice` maps the
  shell store's 6 stored repeat choices.
- Differences from Apple: no time zones (floating only), no `EKRecurrenceRule` week-of-year or day-of-year parts, the
  shell store still saves only the six repeat choices.

## PhotoKit: photo library for apps (`bifrost_app::photo_library`, the `photos` service)

Apps fetch photos from the person's library (`~/Pictures`, as the Photos app scans it) with the PhotoKit shapes:
`PHPhotoLibrary::authorization_status` / `request_authorization`, `PHAsset::fetch_assets` with `PHFetchOptions`
(`sort_descriptors` on creation date, a `PHPredicate` on media type and creation date, `fetch_limit`), `PHFetchResult`
(`count`, `object_at`), `PHAsset` (`local_identifier`, `media_type`, `pixel_width`, `pixel_height`, `creation_date`,
`is_favorite`) and `PHImageManager` (`request_image_data` for the file, `image_from_bytes` for a thumbnail that fits or
covers the target size, built on `image_source`). The app needs the `photos` permission in its manifest; without it the
status is `denied` and a fetch is an error. The first `request_authorization` of an app with the permission is allowed,
later ones return the state they hold; limited access shows only the granted ids. A `localIdentifier` hashes the file
path, so it stays the same across rescans. Models: `bifrost_ui::photo_fetch`, `photo_access`, `photo_media`; shell side
`springboard/src/shell/services/photos.rs`. Decision: an unauthorized fetch is an error (PhotoKit returns an empty
result). Gaps: no access sheet, asset creation, collections, change observation or `isFavorite` (the Photos app keeps
the marks); rows stay partial until a Midget check.


### Uniform Type Identifiers: app-declared types (`uttype_declarations`)

`UTExportedTypeDeclarations` and `UTImportedTypeDeclarations` for apps. A manifest lists `[[exported_types]]` and
`[[imported_types]]` blocks with `id`, `conforms_to`, `extensions`, `mime_types` and `description`;
`parse_manifest` reads them and `register_manifest` (or `register_plist` for `UTTypeIdentifier`,
`UTTypeConformsTo`, `UTTypeTagSpecification` shaped entries) installs them, so `uttype::type_for_extension`,
`type_for_mime`, `conforms`, `distance`, `description`, `extensions` and `mime` see the app types. Rules: an
exported type beats an imported one with the same id; a type with no parent conforms to `public.data`; cycles,
`public.`/`dyn.` ids and unknown parents are refused and the previous set stays; an app cannot take a built-in
extension or MIME type; only the first MIME type of a declaration is indexed.

`UtType` mirrors `UTType`: `new`, `exported_as`, `imported_as`, `filename_extension`, `mime_type`,
`tag(tag, UtTagClass, conforming_to)`, `tags`, `conforms_to`, `is_declared`, `is_dynamic` (`dyn.*`: no one declared
the tag; it conforms to the requested type or `public.data`) and `is_public` (system-declared). `UtTagClass` takes
the two Apple classes or any declared class string. Not done: the shell does not call `register_manifest` at app
launch yet, and nothing is checked on Midget (row stays `partial`).

`bifrost_ui::content_sniff::sniff(bytes)` reads at most 512 leading bytes and maps common binary signatures, markup,
scripts and UTF-8 text to built-in type identifiers. Shared containers such as ZIP report their container type for the
caller to refine. This is not a `magic.mgc`-compatible database; the reference ContentKit API was not confirmed, and no
picker or preview consumer calls the sniffer yet.

## CryptoKit signatures (`Curve25519.Signing`, `bifrost_ui::ed25519`)

Ed25519 (RFC 8032) public-key signatures, a pure model with host unit tests. Field
arithmetic is shared with `x25519`; the hash is `bifrost_core::sha512`.

- `PrivateKey::generate()` (`init()`), `from_raw_representation` (32-byte seed),
  `raw_representation`, `public_key`, `signature_for(data)` (64 bytes).
- `PublicKey::from_raw_representation`, `raw_representation`,
  `is_valid_signature(signature, data)`; it rejects bad lengths, non-canonical S and
  keys that are not curve points.
- `CryptoKitError::IncorrectKeySize` for keys that are not 32 bytes.

Still missing: P256, P384, P521. Status is partial until checked on Midget.

## Post-quantum KEM and signatures (`MLKEM768`, `MLKEM1024`, `MLDSA65`, `MLDSA87`)

ML-KEM (FIPS 203) for ML-KEM-768 and ML-KEM-1024, pure Rust in `bifrost_core::mlkem`; the hash for the matrix is the
SHAKE128 and SHAKE256 XOF added to `bifrost_core::sha3` (`Shake`). The SDK module `bifrost_app::kem` wraps it:

- `DecapsulationKey::generate(variant)` (seeds `d` and `z` from the kernel pool), `from_seeds`, `from_bytes`,
  `bytes`, `encapsulation_key`, `decapsulate(ciphertext)`.
- `EncapsulationKey::from_bytes` (length and the FIPS 203 modulus check), `bytes`, `encapsulate()` (fresh `m` from the
  pool) returning `Encapsulation { shared_secret, ciphertext }`.
- `Variant` is `Params`: `MlKem768` (ek 1184, dk 2400, ct 1088) and `MlKem1024` (ek 1568, dk 3168, ct 1568).
- `KemError`: `IncorrectKeySize`, `IncorrectCiphertextSize`, `InvalidEncapsulationKey`, `InvalidDecapsulationKey`,
  `Unavailable`. A tampered ciphertext is not an error: decapsulation returns the implicit-rejection secret.

Decisions: the seeds are the caller's or the pool's, so the core draws no randomness; `DecapsulationKey` `Debug`
shows the variant only. Known gap: the official FIPS 203 known-answer vectors are not in the repo, so the tests check
lengths, the NTT round trip, NTT multiplication against a schoolbook product, byte and compress codecs, round trips
and implicit rejection, and not interoperability. Out of scope: the `KEM`/`KEMPublicKey` type shapes. Parity:
`Post-quantum KEM and signatures` row in `docs/parity-matrix/data/apple-cryptokit.json` (partial until the vectors pass
and it runs on Midget).

### X-Wing (`XWingMLKEM768X25519`, `bifrost_app::kem::xwing`)

The hybrid KEM of ML-KEM-768 and X25519, from the IETF draft `draft-connolly-cfrg-xwing-kem` (clean-room from the
public draft). The arithmetic is in `bifrost_ui::xwing`, which can reach both `bifrost_core::mlkem` and
`bifrost_ui::x25519`; the SDK module draws the entropy and keeps the object shape of `kem`:

- `DecapsulationKey` is a 32-byte seed. SHAKE256 of the seed gives 96 bytes: `d` and `z` for ML-KEM-768 key generation,
  then the X25519 secret. `from_seed`, `generate` (seed from the pool), `from_bytes` (32 bytes), `bytes`,
  `encapsulation_key`, `decapsulate(ciphertext)`.
- `EncapsulationKey` is `pk_M || pk_X`, 1216 bytes. `from_bytes` checks the length and the ML-KEM modulus, `bytes`,
  `encapsulate()` (64 bytes of randomness from the pool: `m`, then the X25519 ephemeral scalar) returning
  `Encapsulation { shared_secret, ciphertext }`, where the ciphertext is `ct_M || ct_X`, 1120 bytes.
- The shared secret is SHA3-256 of `ss_M || ss_X || ct_X || pk_X || label`, with the six-byte label `\./` + `/^\`.
- `KemError::LowOrderPoint` is new: an all-zero X25519 agreement (a small-order peer key) is refused. The draft's
  handling of that case is not checked here; this is a choice to confirm against the draft.

Known gap: the X-Wing draft test vectors are not in the repo, so the tests check sizes, deterministic round trips from
fixed seeds, that tampering `ct_M` or `ct_X` changes the secret, the label bytes and the combiner input order. Nothing
is checked against another X-Wing implementation. Still missing: the Apple-named `XWingMLKEM768X25519` type and the
`KEM`/`KEMPublicKey` shapes.

### ML-DSA signatures (`MLDSA65`, `MLDSA87`, `bifrost_app::signature_pq`)

ML-DSA (FIPS 204) for ML-DSA-65 and ML-DSA-87, pure Rust in `bifrost_core::mldsa`, on the same `Shake` XOF and the
NTT over Z_q with q = 8380417 (zeta 1753). The SDK module `bifrost_app::signature_pq` wraps it:

- `PrivateKey::generate(variant)` (the seed `xi` from the kernel pool), `from_seed`, `from_bytes` (recomputes the public
  half from the secret bytes), `bytes`, `public_key()`, `sign(message, context)` (hedged: fresh signing randomness),
  `sign_deterministic(message, context)` (zero randomness, reproducible).
- `PublicKey::from_bytes` (length check), `bytes`, `is_valid_signature(signature, message, context) -> bool`. A bad
  signature, a wrong message or context, and a context over 255 bytes all give `false`.
- `Variant` is `Params`: `MlDsa65` (pk 1952, sk 4032, sig 3309) and `MlDsa87` (pk 2592, sk 4896, sig 4627).
- `SignatureError`: `IncorrectKeySize`, `ContextTooLong`, `Unavailable`.

Core: `keygen`, `sign`, `verify` (the pure interface with the context prefix `0 || |ctx| || ctx || M`),
`public_key_from_secret_key`, and the internal algorithms with the rejection loop checks (`||z||inf`, `||r0||inf`,
`||ct0||inf` and the hint count). The FIPS 204 bit packings and HintBitPack are in the same module.

Decisions: the signing randomness is the caller's, so the core stays deterministic for a given input; the SDK fills it
from the pool for `sign`. `PrivateKey` `Debug` shows the variant only. Known gap: the official FIPS 204 known-answer
vectors are not in the repo, so the tests check sizes, the NTT round trip and multiplication, the sampling and
packing round trips, sign and verify for both sets, the failure cases and reproducible deterministic signing; they do
not check interoperability. Not done: ML-DSA-44 and the X-Wing hybrid KEM.

## Preference panes: pane links (`bifrost_ui::pane_link`, `NSPreferencePane` deep links)

A System Settings pane is opened by an `x-apple.systempreferences:` link: the pane identifier, and an optional anchor
after `?` for a row inside the pane (for example `…security?Privacy_Camera`). `pane_url(identifier, anchor)` builds the
link and returns `None` unless both parts are plain tokens (letters, digits, `.`, `-`, `_`). `parse_pane_url(url)`
reads a link back into a `PaneLink { identifier, anchor }`, splitting at the first `?`, and refuses other schemes, an
empty identifier and an empty or malformed anchor. The pane lifecycle (`NSPreferencePane` will and did select, should
and will and did unselect) stays in `bifrost_ui::pane`; links do not change it.

Gaps: the shell has no link router, so no link opens the Settings app or a pane yet. Settings pages have no identifier
table: the macOS 27 pane identifiers are not in `docs/parity-matrix/os/settings`, and only `com.apple.preference.displays`
is evidenced (as a LaunchAgent name in `docs/parity-matrix/os/daemons/a-c.json`). Add identifiers with their evidence
before wiring them. The scheme name is allowlisted in `tools/check-names-allow.txt`. Parity: `Preference Panes`,
"Pane links" row in `docs/parity-matrix/data/preference-panes.json` (partial).

### Contacts data model, store and vCard (`bifrost_ui::contact_book`, `bifrost_ui::contact_vcard`)

Mirrors the Contacts framework type by type, pure and host-tested; the shell store and the Contacts app still use the
single-field `contactstore.rs` until wired (`from_legacy` migrates one old row). `Contact` (CNMutableContact) holds
labelled multi-values (`LabeledValue`, `label::*` for the CNLabel constants): phones, emails, postal addresses, URLs,
relations, social profiles, instant messages, dates, plus birthday and image data. `ContactStore` offers
`contacts_matching(Predicate, keys)` (name, email, phone, identifiers, in group, in container; email and phone use
indexes), `groups_matching`, `containers_matching`, `default_container_identifier`, and atomic `SaveRequest`s
(add/update/delete contact and group, add/remove member). A fetch returns `FetchedContact`, whose getters fail with
`Error::PropertyNotFetched(key)` for keys the fetch did not name. `format_name` is the formatter's full-name style.
`contact_vcard::serialize(contacts, Version::V3|V4)` and `contacts(text)` read and write vCards (folding at 75 bytes,
escapes, TYPE, custom labels via item groups, PHOTO base64). Gaps: no on-disk format for the new model, no change
history, no thumbnail in vCards, no postal address formatter, no SDK service surface for the new keys.

## PhotoKit: change requests, albums, change observation (`bifrost_ui::photo_library`)

`PhotoLibrary` mirrors `PHPhotoLibrary`. `perform_changes(|r| ...)` (`performChanges`) runs the closure on a copy of the
library: an `Err` throws the copy away (atomic), an `Ok` keeps it and tells every observer once with a `Change`.
`ChangeRequests` holds the requests: `creation_request_for_asset_from_image`, `creation_request_for_asset_from_video`
(duration from `video_duration`, which reads `mvhd` from MP4 or QuickTime bytes), `creation_request_with_resources` and
`add_resource` (`PHAssetCreationRequest.addResource`), `set_favorite`, `set_hidden`, `delete_assets`, and for albums
`create_album`, `rename_album`, `delete_album`, `add_assets`, `remove_assets`, plus `create_collection_list` and
`add_child_album` (`PHCollectionList`).
Fetching: `fetch_assets` and `fetch_assets_in` take `FetchOptions` (`predicate`, `sort_descriptors`,
`include_hidden_assets`, `fetch_limit`) and return a `FetchResult`. `Change::change_details(&fetch)` returns
`FetchResultChangeDetails` (`removed_indexes`, `inserted_indexes`, `changed_indexes`, `moves`) or `None` when the
result is unaffected. Access: `AccessLevel::AddOnly` may create but not read or edit; `Status::Limited` (from
`photo_access`) hides unchosen assets, and assets the app creates join the selection.
Gaps: not wired to the shell or an app yet, no `requestAVAsset` / `requestPlayerItem`, no editing or project
extensions, trash is not exposed as a change request. Host tests only; a Midget check is still needed before the
matrix rows can become parity.

Photo projects (`bifrost_ui::photo_project`) are a separate pure model: an ordered set of sections containing ordered
asset identifiers. `Project::apply(&[Change])` is all-or-nothing and supports section add/rename/remove/move and asset
add/remove; text is trimmed and bounded and projects hold at most 200 sections. It does not create or own photo assets,
persist projects, or provide the PhotoKit extension UI. See `docs/parity/photokit-projects.md`.

## Rendering requirement and patch-graph models (`bifrost_ui::gl_requirements`, `patch_graph`)

`GlVersion::parse` reads the API family, major/minor version and desktop core-profile marker from a reported
`GL_VERSION`; `GlRequirement` checks OpenGL 3.2 Core and OpenGL ES 2.0/3.0 floors. `PatchGraph` validates typed port
connections, single-source inputs and cycles, then evaluates patches in deterministic dependency order. Both are pure
and headless-tested. Neither creates a GL context or draws a composition: apps cannot access the internal EGL presenter,
and the graph has no Quartz Composer file format, editor, renderer or plug-in host.

## AppKit user interactions: wheel, tracking, drag sessions (models)

- `ui::wheel` (`NSEvent` scrollWheel): `Sequencer::feed(Raw)` gives a `ScrollEvent` with `scrolling_delta_x/y`,
  `has_precise_scrolling_deltas`, `phase` and `momentum_phase` (Began, Changed, Ended; momentum follows the lift). A notched
  wheel is imprecise, `LINE_HEIGHT` points per line, no phases.
- `ui::tracking` (`NSTrackingArea`): options `ENTERED_EXITED`, `MOVED`, `CURSOR_UPDATE`, `ACTIVE_IN_KEY_WINDOW`,
  `IN_VISIBLE_RECT`; `Tracker::set_visible_rect`, `set_key_window`, event `CursorUpdate`.
- `ui::drag_session` (`NSDraggingSession`, source and destination): `Session::begin(mask, items)`, `moved`, `drop_on`,
  `cancel`; callbacks logged in order Entered, Updated, Exited, Perform, Conclude; one operation chosen from the source mask
  and the destination answer (copy, then link, then move); `DraggingItem` carries pasteboard types and an
  `NSFilePromiseProvider`-style `PromisedFile` written on drop.
- Magnify and rotate models exist in `ui::gesture`; `ui::trackpad_gesture::TwoFinger` runs both on one two-finger touch and
  returns Began, Changed and Ended events (`lift` ends them). Needs a Midget check before parity.
- Update 2026-10-19: wheel, hover and the two-finger events now reach apps, and the view kit has drag and drop; see the next
  two sections. What is still open: trackpad scroll phases and momentum, trackpad magnify and rotate from the compositor, and a
  Midget check of all of it.

### Pointer events: wheel, hover, magnify, rotate (`ToApp::Pointer`)

Apple parity: `scrollWheel(with:)`, `mouseMoved`/`mouseExited` through tracking areas, `magnify(with:)` and `rotate(with:)`.
An app that does not ask keeps the touch stream it had (the wheel as a drag, no hover, no gestures).

- **Model** (`bifrost_ui::pointer_event`, SDK `bifrost_app::pointer_events`): `PointerEvent` is `Wheel { x, y, dx, dy, precise,
  phase, momentum }` (`dx`/`dy` are how far the content moves in pixels, positive `dy` moves it down, `precise` is
  `hasPreciseScrollingDeltas`), `Hover { x, y }`, `HoverExit`, `Magnify { x, y, magnification, phase }` and `Rotate { x, y,
  radians, phase }`. `to_points(scale)` converts pixels to points. `PointerRouter` keeps the app's tracking areas
  (`set_areas`, `set_key_window`) and `route(&event)` returns `Delivered`: tracking Entered, Exited, Moved from the hover,
  `Scroll` with a `wheel::ScrollEvent`, `Magnify`, `Rotate`. `from_gesture` turns a `trackpad_gesture::Event` into a
  `PointerEvent` at the midpoint of the fingers.
- **Opt in**: `App::pointer_interest()` returns `pointer_events::WHEEL | HOVER | GESTURES` bits (default 0) and
  `App::pointer(ctx, &event)` handles the events (positions in surface pixels, like touches). The runtime sends
  `ToShell::PointerInterest { mask }` when the mask changes; the shell sends only the kinds an app asked for
  (`AppHost::pointer`).
- **Wire** (`proto/src/pointer.rs`): `ToApp::Pointer` and `ToShell::PointerInterest`, both tag `0x1f1` in their own tag spaces;
  a non-finite number, an unknown kind or phase byte is refused on decode, unknown interest bits are dropped.
- **Shell** (`src/shell/pointer_events.rs`): the Wayland pointer now also pushes `input::Event::Wheel` ahead of the drag it
  emulates for an axis event (the drag is unchanged for apps that did not ask). `Shell::pointer_wheel` finds the external app
  window content under the pointer as a click does (`Desk::pick`, content hits only), sends the wheel at the position in its
  window and then drops the next three plain touches for that app (`AppHost::swallow_wheel_drag`, 250 ms), so the app does not
  scroll twice. `Shell::pointer_hover` (called from `desktop_pointer`) sends `Hover` over the content of an app that asked
  and `HoverExit` when the pointer leaves it or moves to another app. `Shell::pinch_contact` feeds the first two touchscreen
  contacts of the app in front to `TwoFinger` and sends `Magnify` and `Rotate`.
- **Extra windows** (2026-10-23, E2E): the wheel and hover go to the window under the pointer, extra windows included. Before,
  `ext_content_at` matched only an app's main window, so an extra window (Calendar's event editor, Mail's viewer and compose
  windows) never got the wheel and the app had no window id for it. Now an extra window gets `ToApp::WindowInput { window,
  input: WindowInput::Pointer(event) }` (kind 6, the `ToApp::Pointer` bytes after it) in window points, like its
  `WindowTouch`; the main window keeps `ToApp::Pointer` in surface pixels. The emulated drag is dropped for an extra window that
  took the wheel too (`ext_window_touch` checks `swallows_touch`); an app that did not ask keeps the drag in every window. SDK:
  `App::window_pointer(ctx, window, &event)` (default: the main window goes to `pointer`, extra windows drop it), reached
  through `default_window_input`; an app that overrides `window_input` passes `WindowInput::Pointer` on to it (Activity and
  Dictionary do). Scroll views take the wheel: `View::scroll_wheel(dx, dy)` (default none), `ScrollView` scrolls by the wheel's
  travel within its range and passes it on when the document fits, `Window::wheel(x, y, dx, dy)` offers it to the deepest view
  under the pointer and then its ancestors, and `ViewApp::pointer` (pixels) / `pointer_points` (an extra window's points) feed it.
  Hand-drawn lists use `Scroller::wheel(dy)` (`bifrost_ui::scroll`). Tests: proto
  `a_window_pointer_event_has_the_pointer_layout_after_kind_six`, apphost `the_wheel_over_an_extra_window_carries_the_window_id`,
  pointer_events `an_extra_window_gets_its_pointer_events_in_points_and_the_main_window_in_pixels`, app_windows
  `the_wheel_reaches_the_extra_window_it_is_over_with_its_id`, view scroll `the_window_hands_the_wheel_to_the_scroll_view_under_the_pointer`,
  ui scroll `the_wheel_moves_the_offset_by_its_travel_within_the_range`. The pinch and rotate gestures still go to the main
  window of the app in front only.
- **Adopters** (2026-10-23, E2E follow-up): asking for the wheel drops the emulated drag in *every* window of the app, so an
  app that adopts it handles `pointer` for its main window as well as `window_pointer`. Mail scrolls its list and reader
  (main window), a viewer's text and a compose window's message (whole lines back from the end; typing returns to the end).
  An extra window's `WindowTouch` and wheel are window points: Mail's viewer and compose sheet scale them to the pixels they
  draw in (the viewer used to scroll at 1/scale speed and compose clicks missed at a scale other than 1). TextEdit scrolls
  its page and the Fonts panel's family column (its emulated drag had placed the caret and selected text). The shell's
  open and save panel takes the wheel over its list and icon grid while it is up (`Shell::doc_wheel`, modal: the wheel
  never reaches the window under it) and drops the release of the emulated drag (`Panel::swallows`), which had clicked
  the row under the pointer. Tests: mail `the_wheel_scrolls_the_list_a_viewer_and_a_compose_message`, textedit
  `the_wheel_scrolls_the_page_and_over_the_fonts_panel_its_families`, docpanel
  `the_wheel_over_the_panel_never_clicks_and_never_reaches_the_window_under_it`.
- **Not built, said plainly**: the Wayland layer reads only the vertical axis of a notched wheel (no `axis_source`, no phases,
  no momentum, no horizontal scroll), binds no pointer-gestures protocol (a touchpad pinch does not arrive; only touchscreen
  fingers do). Apple's trackpad momentum and phase behaviour cannot be
  matched until the compositor forwards `axis_source`/`axis_stop`.
- Midget check: give an SDK app `fn pointer_interest(&self) -> u32 { pointer_events::WHEEL | pointer_events::HOVER }` and an
  `App::pointer` that logs, install it, scroll the wheel and move the pointer over its window, and read `bifrost pc log`.

### Drag and drop in the view kit (`View::dnd_source`, `dnd_types`)

Apple parity: `NSDraggingSource`, `NSDraggingDestination`, `registerForDraggedTypes`. Code: `sdk/src/view/drag.rs`.

- A view is a **source** when `dnd_source()` returns a `DragSpec` (items, allowed operations as `dragop` bits, a drag image).
  The window records the press (`Window::event`); a move past `DRAG_SLOP` (4 pt) cancels the control that held the press
  (so a button does not fire), and `ViewApp::event` starts the session (`dnd::begin`) from `Window::take_drag_request`.
  `dnd_ended(op)` tells the source how it ended (0 when nobody took it).
- A view is a **destination** when `dnd_types()` lists types. A type takes every type that conforms to it
  (`uttype::conforms`: `public.image` takes `public.png`); every item of the drag must be taken. `ViewApp` registers the union
  with the shell (`sync_drop_types`, on configure and after every event), and `ViewApp::drag` (call it from `App::drag`) finds
  the deepest accepting view under the pointer (or its nearest accepting ancestor) and calls `dnd_over(entered, x, y, items,
  allowed)`, `dnd_exited()` and `dnd_perform(items)`. The window answers with one operation both sides allow
  (`pick_operation`: copy, then link, then move). A drop ends the highlight with `dnd_exited`.
- Items are the shell's `DragItem`s (text, links, granted files); the view reads a file through the grant, bytes do not travel.
- Not built: no shipped view is a source or a destination yet (Files and Notes still use their own glue), no drag image
  from a view's pixels, no spring-loading and no `draggingFormation`.
- Midget check: see the `Drag and Drop` row of `docs/parity-matrix/data/appkit-user-interactions.json`.

## InputMethodKit: language methods, input sources, client protocol (sprint 9)

New pure models in `springboard/ui/src/` (host unit tests only; not wired into the shell, not yet checked on Midget):

- `hangul_method::HangulMethod`: two-set keys to jamo (`jamo_for_key`), joined into syllable blocks by the Unicode block
  formula, compound vowels and finals, a vowel after a final moves it to the next block. `push`, `backspace`, `marked`,
  `take`, like `kana_method::KanaMethod`. No hanja conversion.
- `telex_method::TelexMethod`: Vietnamese Telex typing. `aa ee oo aw ow uw dd` shape marks, `s f r x j` tones (`z`
  clears), tone placed by the orthography rules (marked vowel first, `oa oe uy` open, closed pairs on the second vowel,
  `qu` and `gi` consonants). Lower-case keys only are interpreted.
- `input_source::InputSources` (text input source services): `install`, `list(include_all_installed)`,
  `for_language`, `current`, `enable`, `disable`, `select`, `select_next`, and `save`/`restore` as the persisted form.
  A palette source is enabled but never selected; the selected and the last keyboard source cannot be disabled.
- `imk_client::TextInputClient` (insertText(_:replacementRange:), setMarkedText(_:selectionRange:replacementRange:),
  selectedRange, markedRange, attributes(forCharacterIndex:lineHeightRectangle:)) with `update_composition` and
  `apply(outcome, client)` so a text view only implements the trait; a mock client drives the controller tests.

Gaps: no pinyin (no syllable-to-hanzi data is shipped), no Transliteration or Dictation methods, no Wayland text-input
provider. (The sources and the other models above are now used by the input method host below.)

## InputMethodKit: the input method host (`bifrost_ui::imk_host`, `shell/input_providers.rs`)

Added 2026-10-19. Joins the models above into one flow the shell runs; it is in one process, with no provider socket.

- **Registering**: `Host::register(bundle, &info, languages, engine)` parses an Info dictionary (`imk_info`), starts a
  `Server` for it and installs one input source per mode, id `<bundle>.<mode>` (`input_source`). `Host::install_layout`
  adds keyboard layouts to the same source list; install them first, because the first selectable source installed is
  the selected one. The shell scans `<springboard root>/input-methods/*/Info.plist` at start
  (`Providers::load_dir`). The two BifrOSt keys are `BifrostEngine` (`kana` or `hangul`) and `BifrostLanguage`; a bundle
  without an engine starts no composition, so every key passes through to the client.
- **Sessions**: `Host::connect(client)` gives a client one `InputController` in every method; the Search field is client 0
  and an external app in slot `n` is client `n + 1`. `disconnect` returns the composition still held.
- **Switching**: `select_source(id)` makes a method's mode the active one (all sessions change mode, the previous method's
  compositions are committed) or, for a layout, no method. It returns `PendingCommit`s that the shell inserts
  (`Shell::apply_provider_commits`). The menu bar input menu lists the enabled modes under the layouts
  (`Action::InputMethod`), the bar label is the first character of the mode name, and choosing a layout or pressing
  Ctrl+Space ends the method (`Shell::input_layout_selected`).
- **Events**: `Host::key(client, text, combo, &mut dyn TextInputClient)` returns `KeyResult`: `Consumed`, `PassThrough`, or
  `CommittedThenPass` (the composition is in the client; the client now types the key). Text keys go through the `Engine`
  with the whole keystroke string (`Engine::convert(mode, keystrokes) -> Option<Conversion>`, so Backspace is a replay of the
  shorter string; `None` ends the composition); Space and Return pick the candidate (the selected one, Space takes the first
  when none is selected), digits 1 to 9 pick by number, Up and Down move the selection, Left and Right the composition caret,
  Escape restores the typed keystrokes, a shortcut or any key without text commits and goes on. `Host::command` and
  `command_by_name` carry editing commands (`move_left`, `delete_backward`, `select_all`, ...); `Host::mouse` handles a click
  during a composition (inside the marked text it moves the caret, outside it commits).
- **Queries and the window**: `composed_string`, `original_string`, `candidates`, `composition_caret`, `candidate_view`
  (items, selected, location hint, opacity, panel type) and `candidate_origin` (`candidates::place`). The shell draws the
  window in `compose_window::draw_compose`: a numbered single-column list with the selected row in `MENU_HIGHLIGHT`, over the
  Search card, or beside the caret of an external app (`Shell::provider_candidates`: the caret rectangle in window points
  plus the window body origin, through `textinput::popup_origin`).
- **Clients**: `TextInputClient` is implemented by `bifrost_ui::text::TextEditor` (one undo step per composition),
  by the shell's Search field (`FieldClient`, characters to bytes) and by an external app's field (`AppClient`: marked text and
  commits as `SetMarkedText`/`UnmarkText`, the caret and marked range from the app's last `TextState`). Keys reach the
  host before the field in `Shell::compose_search_key` and `ext_type_out`, so the on-screen keyboard's keys are composed too.
- **Engines**: `KanaEngine` (romaji to hiragana, candidates hiragana and katakana) and `HangulEngine` (two-set). The older
  composers in `shell/input_methods` (Telex, VNI, pinyin tables and others) are a separate trait and are not hosted yet.
- **Not built, said plainly**: no provider process (a method is a folder plus a built-in engine; nothing speaks the window
  server's input method protocol to a third party), no dictionary conversion, no mouse forwarding (a click on the candidate
  window or into marked text is not delivered to `Host::mouse`/`pick_candidate`), editing commands from the menu do not go
  through `Host::command`, the selected mode is not saved across a restart, `attributes_for_character_index` returns
  `None` everywhere, and the keyboard app still emits no `Preedit` and does not draw the candidates.
- Midget check: the recipe in the `Input method server and connections` row of
  `docs/parity-matrix/data/inputmethodkit.json` (a Kana bundle under `/var/lib/springboard/input-methods`, choose Romaji in the
  input menu, type `ka` in Search).

## Text view and TextKit 2 layout (AppKit Text, `bifrost-ui`)

Pure models with host tests (`springboard/ui/tests/text_view.rs`). Nothing is drawn by a shell widget yet, and none of it is
checked on Midget, so the `appkit-text` rows stay `partial`.

| Apple | BifrOSt (`bifrost_ui::...`) | Notes |
|---|---|---|
| `NSTextContentStorage` | `text_layout_manager::TextContentStorage` | Text plus a paragraph style and optional list item per paragraph (attribute runs, so they shift with edits) and attachments at `U+FFFC` characters. `set_paragraph_attrs`, `paragraphs`, `list_markers`, `insert_attachment`. The last empty paragraph cannot hold attributes. |
| `NSTextLayoutManager`, `NSTextLayoutFragment`, `NSTextLineFragment` | `TextLayoutManager`, `TextLayoutFragment`, `TextLineFragment` | `ensure_layout` caches per paragraph; `relaid_paragraphs` lists what an edit laid out again. `x_of`, `offset_at_x`, `offset_at_point` for carets and hit tests. |
| `NSTextContainer` | `textlayout::TextContainer` | Width and `line_fragment_padding` drive the wrap; exclusion paths are not applied by the layout manager yet. |
| `NSParagraphStyle` | `paragraph::ParagraphStyle` | Honoured by layout: alignment, `first_line_head_indent`, `head_indent`, `tail_indent`, `line_spacing`, `paragraph_spacing(_before)`, line height multiple and limits, `tab_stops` (left-aligned only, default interval past the last stop), `line_break_mode` (word and character wrap; clip and truncation modes keep one line). Justification and base direction are not laid out. |
| `NSTextList` | `textlist::List`, `text_layout_manager::ListItem` | Markers renumber on edit, restart after a plain paragraph, nest by level. |
| `NSTextTable`, `NSTextTableBlock` | `text_table::TextTable`, `TextTableBlock` | Absolute, percentage and auto columns, row and column spans, padding; the caller supplies each cell's content height. Borders and cell backgrounds are not modelled. |
| `NSTextAttachment` | `text_attachment::TextAttachment` | Glyph size from `bounds` or the image, shrunk to the container width; the line grows to the tallest attachment. |
| `NSTextSelection`, `NSTextSelectionNavigation` | `TextSelection::moved` with `Direction` and `Granularity` | Character, word, line (visual) and paragraph; up and down keep the x position (`goal_x`); at the first or last line a vertical move goes to the text edge. |
| `NSTextView` | `textview::TextView` | Adds `is_rich_text`, `allows_undo`, `uses_find_bar`, `show_find_bar`, `set_find_query` (incremental), `find_next`, `find_previous`, `replace_current`, `replace_all` (one undo step), `set_spell_checker` and `misspelled_ranges`. |
| `NSTextFinder` | `textfind::TextFinder` | Case and whole-word options; wraps in both directions. No regular expressions. |
| `NSSpellChecker` | `textview::SpellHook` | A closure returning misspelled ranges; the shell's `text` service is the intended provider. |

Typing coalesces into one undo step (`TextEditor`); a newline or a caret move ends the step. `TextEditor::set_allows_undo(false)`
applies edits without recording them, `replace_range` and `replace_ranges` record find-bar replacements.

Open: draw the view in the SDK (caret, selection, find bar, list markers, attachments), exclusion paths in the layout manager,
right and centre tab stops, justification, bidirectional text, carry paragraph styles in `bifrost_app::attributed`, and Midget checks.

## AVFoundation playback model (`bifrost_ui::av_player`, `av_queue_player`, `av_asset`)

Pure-Rust, caller-clock playback model (microseconds); types drop the framework prefix.

- `Asset` (`from_wav_bytes`, `load_status`, `duration_us`, `tracks`), `Pcm` (WAV in/out, `slice`, `sine`).
- `Player`/`PlayerItem`: `status`, `play`, `pause`, `set_rate`, `default_rate`, `time_control_status`, `seek(to, before, after)`, periodic and boundary time observers, `DidPlayToEndTime`, `action_at_item_end`, `volume`, `is_muted`, timed metadata groups. Nothing plays until `play()`; `advance(dt)` drives the clock.
- `QueuePlayer`: `insert`, `remove`, `advance_to_next_item`, `QueuePlayer::looper(asset)`.
- `MutableComposition` and `insert_time_range`, `MutableAudioMixInputParameters` volume ramps, `export_wav` (trimmed WAV via `ExportSession`).

Gap: not connected to the audio engine or apps; video tracks and real metadata output are not modelled. Needs a Midget check.

## Shader library and image kernels (Metal library, compute pipeline, Core Image kernels, MPS image kernels)

Code: `ui/src/shader_library.rs` (names and stages, `from_source`, `RenderPipelineDescriptor`), `ui/src/shaders/mod.rs`
(built-in library, function constants, compute pipeline state, Core Image routing), `ui/src/image_kernels.rs` (CPU
reference kernels), `ui/src/compute_dispatch.rs` (the grid walk the kernels run on). Tests: `ui/tests/shader_kernels.rs`.

| Apple | BifrOSt |
|---|---|
| `MTLDevice.makeLibrary(source:options:)` | `ShaderLibrary::from_source(&str)`; scans `kernel`, `vertex` and `fragment` declarations (nothing is compiled) |
| `MTLLibrary.makeFunction(name:)` | `ShaderLibrary::make_function`; `LibraryError::NotFound` for a missing name |
| `MTLFunctionConstantValues` | `shaders::FunctionConstantValues` (`set`, `set_bool`, `f32` values); unset constants take the Core Image default |
| `MTLComputePipelineState` | `shaders::make_compute_pipeline_state(lib, name, &constants)` then `run(&Image)`; `thread_execution_width`, `max_total_threads_per_threadgroup` |
| `MTLRenderPipelineDescriptor` | `RenderPipelineDescriptor { vertex_function, fragment_function, raster_sample_count }` with `validate(&lib)` |
| `CIKernel`, `CIColorKernel`, `CIWarpKernel` `(functionName:fromMetalLibraryData:)` | `shaders::CiKernel::new(name, &lib, KernelKind)`; a class mismatch is `KernelError::KindMismatch` |
| `CIGaussianBlur`, `CIColorControls`, `CISepiaTone`, `CIVignette`, `CILanczosScaleTransform`, `CIAffineTransform` | `shaders::ci_filter(name, &[("inputRadius", 2.0)])` |
| `CIColorMatrix`, `CISourceOverCompositing` | `shaders::ci_color_matrix(r, g, b, a, bias)`, `image_kernels::source_over(fg, bg)` |
| `MPSImageConvolution`, `MPSImageHistogram` | `image_kernels::convolve(img, w, h, weights, bias)`, `image_kernels::histogram(img)` |

Decisions: pixels are the premultiplied `Image` format; colour kernels unpremultiply first. Convolution is a correlation with
clamped edges and needs an odd size. The affine kernel samples nearest and leaves outside pixels transparent. The blur
reuses `bifrost_render::blur_filters`.
Gaps, stated rather than skipped: there is no GPU compute back end (the workspace has no GPU compute crate, so no adapter
test exists yet; the CPU path is the reference a GPU path must match); no shader compiler, the library is a name table
over built-in kernels, so app-supplied shader bodies are not run; no `MPSImageGaussianBlur` class (the blur kernel is
`CIGaussianBlur`'s), no per-kernel `MPS` texture descriptors; `CIColorMatrix` and convolution weights are set by value, not
by `CIFilter` input keys. Written without building or running (owner rule); parity needs a Midget check.

## GPU kits (MetalKit, MetalFX, Metal Performance Shaders): app-facing view, textures, meshes, scalers, image kernels, graphs

Wave 15 (2026-10-19). The GPU path is Vulkan/GLES inside the shell; apps get these modules, all CPU reference code (no
proprietary shaders), so the same calls run headless in tests and a GPU back end is later checked against them. Code is in
`springboard/sdk/src/`; the models underneath stay in `bifrost_ui` and `bifrost_render`.

| Apple | BifrOSt |
|---|---|
| `MTKView`, `MTKViewDelegate` (`drawableSizeWillChange`, `draw(in:)`) | `gpu_view::GpuView` and `GpuViewDelegate` (`drawable_size_will_change`, `draw(&mut Frame)`): bounds, scale, timed or on-demand loop, pause on resign, clear colour, a pool of 2 or 3 CPU drawables (`take_presented`, `image`, `release`); `draw()` forces a pass |
| `MTKView.device`, `colorPixelFormat`, `isOpaque` | `GpuView::set_preferred_device`, `set_opaque`, `drawable_request()` (size, `FORMAT_OPAQUE` or `FORMAT_ALPHA`, render node checked); drawables above 16384 on a side or 2^26 pixels are refused |
| `MTKTextureLoader` (`newTexture(data:)`, `(URL:)`, `(name:...)`, async batch) | `gpu_texture::TextureLoader` (`from_bytes`, `from_file`, `from_catalog` with scale variants, `from_image`, `load_async` with `PendingTextures::poll` and `wait`); options are `texture::LoadOptions` (sRGB, mipmap allocate or generate, origin, usage) |
| `MTLTexture` (`replace(region:)`, `mipmapLevelCount`, `label`) | `gpu_texture::Texture` (`replace_region`, `mip_level_count`, `loaded_levels`, `pixel`, `sample_nearest`, `label`) |
| (no MetalKit class: a 2D sprite pass) | `gpu_sprites::draw_batch(target, &SpriteBatch, &[&Texture], scale)` draws the texture runs of a `SpriteBatch` into a view drawable (nearest texel, vertex tint, source-over) |
| `MTKMesh`, `MTKSubmesh`, `MTKMeshBuffer`, `MTKMeshBufferAllocator` | `gpu_mesh::{box_asset, plane_asset, sphere_asset, load_mesh_file, mesh_from_primitive}` give a `render::mesh::Mesh` (interleaved position, normal, uv; one triangle submesh, 16-bit or 32-bit indices) from a `BufferAllocator` with a byte budget; OBJ, STL and ASCII PLY files |
| `MTLFXSpatialScaler` (+ `colorProcessingMode`) | `image_scaler::SpatialScaler` over `Image`: bicubic, alpha-weighted, `ColorProcessing::{Linear, Perceptual, Hdr}` |
| `MTLFXTemporalScaler`, `MTLFXTemporalDenoisedScaler` | `image_scaler::TemporalImageScaler` (`next_jitter`, `reset`, `resolve(color, depth, motion)`; `denoised` flag) |
| `MTLFXFrameInterpolator` | `image_scaler::FrameInterpolator` (`interpolate(a, b, motion, t)`, `intermediate_frames`), blending in linear light |
| `MPSImageConvolution`, `MPSImageTent`, `MPSImageLanczosScale`, `MPSImageBilinearScale` | `image_ops::{convolve, tent_blur, scale}` on all four channels (`ScaleMode::{Nearest, Bilinear, Lanczos}`) |
| `MPSImageStatisticsMeanAndVariance`, `...MinAndMax`, `MPSImageIntegral(OfSquares)`, `MPSImageReduceRowMax` and kin | `image_ops::{statistics, luma_statistics, integral, reduce_rows, reduce_columns}` |
| `MPSImageAdd`, `Subtract`, `Multiply`, `Divide`, `MPSImageConversion`, `MPSImageTranspose` | `image_ops::{combine, invert, convert_transfer, transpose}` |
| `MPSImageGaussianPyramid`, `MPSImageLaplacianPyramid(Add/Subtract)` | `image_ops::{gaussian_pyramid, laplacian_pyramid, collapse_laplacian}` |
| `MPSImageEuclideanDistanceTransform`, `MPSImageGuidedFilter`, `MPSImageFindKeypoints` | `image_ops::{distance_transform, guided_filter, find_keypoints}` |
| `MPSKernel` with `NSKeyedArchiver` | `image_ops::KernelSpec` (an `ImageKernel`, so `apply_chain` runs it), `encode_kernels` and `decode_kernels` (text archive, hostile-input checked), `run_chain` |
| `MPSNNGraph` with convolution, pooling, fully connected, relu, add, concat, softmax nodes | `nn_graph::Graph` (`add_input`, `add`, `set_output`, `run`), model file `to_bytes` and `from_bytes` |
| `MPSRNNImageInferenceLayer` over a sequence, `MPSRNNMatrixInferenceLayer`, `MPSCNNFullyConnected` over a batch | `nn_graph::{lstm_sequence, gru_sequence, matrix_inference}` |

Decisions: pixels are the premultiplied `Image`; colour that is filtered or blended is unpremultiplied (or weighted by alpha)
first. The view's drawables are CPU images so the model is testable without a device; the shell presenter uploads a presented
image. The kernel archive and the graph file are our own formats (text and binary), not Apple's keyed archive, and their readers
cap every length. `KernelSpec` arguments are capped (tent radius 1024, window side 255, sigma 64) rather than allocated.
`render_view.rs` gained `set_pause_on_resign`, `set_resume_on_active`, `app_resigned_active`, `app_became_active` and
`frames_displayed` so a view can pause with the app.
Gaps, stated rather than skipped: no GPU dispatch for any kernel and no GPU device or texture handle for apps (a view's drawable
is a CPU image until the shell attaches it to the presenter); Metal command-buffer encoding and the `MTL4FX` protocols cannot
exist on this system; the temporal denoiser is a 3x3 box filter, not a learned one, and the spatial scaler is bicubic, not
edge-directed; frames passed to the temporal scaler are taken as opaque; no ray intersector or acceleration structures (deprecated
upstream); no learned-model import (a graph is built in code or read from our file); no batch-normalisation or user-defined
graph node; no tangents, Model I/O vertex-layout conversion or mesh formats beyond OBJ, STL and ASCII PLY; `sample_nearest`
is the only texture sampler (no bilinear or mip selection on `Texture`); a texture has no colour-space tag on the raster.
Written without building or running (owner rule); parity needs the Midget checks listed in the matrix rows.

## Run-time assets: BifrOSt's own and openly licensed only

BifrOSt loads only its own assets and openly licensed ones at run time. Nothing from the reference system is read: no
reference-asset root (the old `bifrost_core::ref_assets` resolver, its installed root, per-user folder and environment
override are removed), no reference font and no reference symbol font. `springboard/core/tests/no_reference_assets.rs`
scans every Rust source and fails if one names that root, brings the resolver back or lists the vendor symbol font.
What the packages contain is a separate check (the licence guard for packaging); this section is about what the code opens.

What each former caller uses now:

| Caller | Source | Notes |
|---|---|---|
| Named sounds (`SystemSounds::create_named(name)`, `Sound(named:)`) | BifrOSt's own alert tones, `system_sound::ALERT_NAMES` | The name is matched ignoring case against the 18 tones the shell synthesises from notes (`shell/alert_sound.rs`); Bell, Chime, Pulse and Tick are the original four, and each other name is a synthesised tone with that name's role. `None` for any other name, and the caller keeps its built-in sound. Interface cues (`shell/ui_sounds.rs`) and the key clicks (`shell/kb_clicks.rs`) are synthesised too, and the cues reach apps through `PlaySystemSound`. No recording is read. |
| App icons (`shell/apps.rs` `term_icon`) | `res/icons/terminal`, BifrOSt's layered icon document | Drawn by `bifrost_render::icon_doc`; the other built-in apps use `assets/icons/<id>.raw` or a drawn icon (`AppDef::icon_image`). |
| Font descriptors (`UserFonts::resolve`, sdk `font_descriptor.rs`) | The font library only: `<assets>/fonts` (seeded from `/usr/share/springboard/assets/fonts`) and the fonts apps install | A family that is not in the library falls to the open equivalents of its class (`font_substitution`). No other folder is scanned. |
| Symbols (`bifrost_render::symbols`) | `FONT_FILES = ["Symbols.ttf"]`, Noto Sans Symbols 2 (SIL OFL 1.1), credited in `CREDITS.md` | The vendor symbol font is never loaded, even when a file with its name sits in the assets folder (`a_vendor_symbol_font_in_the_assets_folder_is_never_loaded`); `only_openly_licensed_symbol_fonts_are_listed` fails when a file is added to the list without being credited. |

Symbol sources after the vendor font left the list. The font held eight glyph slots at private-use code points, and none of
the 22 `Symbol` values names one of them (it matched neither by glyph name nor by Unicode), so no symbol lost its source:
18 symbols come from `Symbols.ttf` by Unicode, and Wi-Fi, battery, the gear and the outlined gear were already drawn by hand
(`shell/icons.rs` and `sdf_glyphs`) because no open font has them. Toolbar, source-list and search-field symbols of the SDK kit
come from the SDK glyph set (`sdk/src/glyphs.rs`), which needs no font. The outlined gear in the app menu row
(`Settings…`) has no hand-drawn fallback yet: with no font glyph the row shows no symbol (unchanged from before; the
open fonts have no U+2699 either).

Tests that need a CFF face use BifrOSt's own `springboard/render/tests/fixtures/test-cff.otf` (a box, a disc and a ring drawn by
`make_test_cff.py` beside it); no packed font is CFF.

## Form factor and on-screen keyboard policy (`bifrost_ui::form_factor`, `bifrost_app::FormFactor`)

One shell and one SDK serve a desktop and a touch tablet. `FormFactor` is `Desktop` or `Tablet`, read once from
`BIFROST_FORM` or the plain-word file `/etc/bifrost/form`. With neither set, `FormFactor::current` detects it once: a
tablet or convertible SMBIOS chassis (codes 30 to 32) is a tablet, and without SMBIOS a known tablet device-tree board
(`samsung,gta8wifi`) or a direct-touch input device (`B: PROP` bit 1 in `/proc/bus/input/devices`) is a tablet. Anything
else is `Desktop`. `FormFactor::resolve` is the pure form of the lookup, with the machine facts passed in as `Facts`. The shell passes `BIFROST_FORM` to sandboxed apps, and `bifrost_app::is_touch_tablet()` answers whether the
app runs on a touch tablet.

Keyboard policy (`FormFactor::allows_onscreen_keyboard`, one gate in `Shell::set_keyboard`): the on-screen keyboard never
appears by itself on a desktop, whatever raises it (a focused text field, Terminal, an external app or Wayland client,
the lock screen, a sheet, Search), and a hidden one reserves no inset, `kb_top` offset or window resize. The desktop also
sets `keyboard::set_disabled(true)` (and a process that never calls it defaults to the form factor), so views that own
a `Keyboard` stay down too. The only way up is the user's own choice, Input menu > Show On-Screen Keyboard
(`Shell::keyboard_open_explicit`): it stays until the user closes it (the same row, `keyboard_close_explicit`, or the
keyboard's hide key), and no automatic hide lowers it. A tablet raises it for text fields and keeps it down while a
hardware keyboard is attached (`Shell::set_hardware_keyboard`); tapping outside a field dismisses it as before. Apps
get `BIFROST_HARDWARE_KEYBOARD=1` only on a desktop. Not built: an Accessibility Keyboard switch in Settings; the Input
menu shows only with several input sources (or while the keyboard is open).

## Touch gesture recognizers (`bifrost_ui::recognizer`)

One set of recognizers for the shell and apps, on a touchscreen and with a pointer. Input is a `TouchSample`
(touch id, phase, position, time in seconds); `PointerAdapter` turns button and motion events into samples of one
touch, so a mouse drives the same state machines. The recognizers never read a clock: callers pass sample times and
call `tick` (or `GestureArbiter::tick`) from a timer; `deadline()` and `GestureArbiter::next_deadline()` say when a
tick matters.

- States: `Possible`, `Began`, `Changed`, `Ended`, `Cancelled`, `Failed`, `Recognized`. Discrete gestures end in
  `Recognized` or `Failed`; continuous ones go `Began`, `Changed*`, `Ended` or `Cancelled`.
- `Tap` (taps and touches required, movement, hold and gap limits), `LongPress` (duration, allowable movement,
  touches), `Pan` (min and max touches, translation that does not jump when a finger is added or lifted, mean
  velocity), `Pinch` (scale, scale velocity, focal point), `Rotation` (accumulated radians, angular velocity),
  `Swipe` (direction mask, touches, distance, speed, off-axis ratio) and `ScreenEdgePan` (a `Pan` that must start
  within a margin of an allowed edge and move away from it).
- Velocity uses the smoothing of `shell/gestures.rs` (0.6 new, 0.4 old, zero after a 120 ms pause), per second.
- `GestureArbiter` runs a set: `require_failure(a, b)` holds back what `a` publishes until `b` fails (and fails `a` if
  `b` is recognized); `allow_simultaneous(a, b)` lets two run together; otherwise the first to begin fails or cancels
  the others. It returns `GestureEvent`s in the order the observer must see them.

`ui::gesture` (single pointer `Tap`, `LongPress`, `Pan`, `Magnify`, `Rotate`) stays for the existing callers; new code that
takes touch input uses `recognizer`, which tracks several fingers by id.

Not done yet: the shell's own system gestures still use `src/shell/gestures.rs` directly; moving them onto
recognizers is a separate step.

## Additional OS-shared policy models

`bifrost_ui::account_policy` holds per-account app blocks and feature restrictions; `PolicyTable` checks requests but
does not persist policies or enforce them in the launcher. `bifrost_ui::ambient_light` converts IIO illuminance text,
smooths lux readings and maps light to a brightness fraction, but no sensor reader or display control consumes it.
`bifrost_ui::route_prediction::suggest` ranks known audio destinations by use count and recency; the picker has no
persisted history or integration yet. These remain partial; see `docs/parity/os-gaps-03.md`.

## Game Mode (GamePolicy, GamePolicyFoundation, GamePolicyServices, GamePolicyAgent)

`springboard/src/shell/gamemode.rs` is the shell's Game Mode: a pure state machine (`Model`) with a 3 s leave
hysteresis, `plan` for the effects of a change, and `GamePref` (Settings > General > Game Mode: Automatic or Off,
stored as `game_mode` in settings.ini). A game is an installed app in the Games category or with `game = true` in its
manifest (`apphost::Manifest::game`). It counts while it is the app in front and full screen: a video's full-screen
flag, or the front window taken full screen. The shell checks four times a second (`bin/shell.rs`); entering takes
effect at once.

**Full screen for app windows.** The green light of an external window toggles full screen (`Desk::set_full_screen`):
the window takes the whole screen (its own space), has no title bar, and the menu bar is hidden until the pointer
reaches the top edge (`desk::bar_hidden`). An app asks for it with `ToShell::SetFullScreen { on }` (SDK:
`Ctx::set_full_screen`, tag 0x24d); the shell answers with `ToApp::FullScreen { on }` (tag 0x24c), which the SDK
accepts and ignores for now. Fixed-size windows do not go full screen.

**What Game Mode does while on:**
- CPU profile Performance (`bare::boost::set_profile` writes the governor and clock window), except under Low Power Mode.
- The game's nice and uclamp minimum are raised through bifrostd (`game_priority.rs` on the shell side,
  `bifrostd/src/priority.rs` on the daemon side). The line protocol on `/var/lib/springboard/run/priority.sock` is
  `priority <pid> <nice> <util_min>`, nice -5..0, util_min 0..1024, replied `ok` or `err <reason>`. bifrostd accepts
  only a child of the caller, of the same user, whose executable is under `apps/`, and applies to every thread. Leaving
  (or Turn Off Game Mode) sends nice 0 and util_min 0 for the pid the shell remembered.
- The background task scheduler starts no new tasks (`Settings::paused` in `services/background.rs`); a running task
  still ends on time.
- Menu bar: a "Game Mode" extra shows while it is on; its one row, "Turn Off Game Mode", holds it off for the game in
  front until that game leaves the front (`Model::turn_off`).
- Notice: "Game Mode is on" is posted the first time it turns on, and `game_mode_notice_shown` in settings.ini keeps it
  from repeating.

**Not built:** the file indexing pause (no indexer runs in the shell, so nothing is reachable to pause), and the
Bluetooth sniff interval change for controllers and headsets (the shell's mgmt socket has no connection parameter
update). Both are named in `gamemode::SKIPPED`. No framework API exists for apps to read the Game Mode state. The
priority request reaches only installed apps under the system data root; a user-installed app is refused.

Tests cover the transitions with a fake clock, the turn-off hold, the effect plan, the category and flag rule, the
preference round trip, the background pause, the desk full-screen state, the menu-bar reveal rule, the SDK message, the
proto codec, and the bifrostd line parser, child and install checks. **None of it was compiled or run** in the session
that wrote it; the lead runs the tests. The GamePolicy rows are `partial`.

## Unary file system module (FSKit `FSUnaryFileSystem`, `bifrost_app::fs_module`, `bifrost_app::fuse`)

FSKit lets an app extension ship a file system module that runs in user space and is mounted by the system. Linux has
FUSE in the kernel, so the module side can be built without a C dependency: `bifrost_app::fuse` speaks the kernel
protocol (`include/uapi/linux/fuse.h`, 7.31 and later) over `/dev/fuse` as pure encode and decode, and
`bifrost_app::fuse_mount` mounts the session through the `fusermount3` helper (the helper opens `/dev/fuse` and passes
it back over a socket pair with `SCM_RIGHTS`, as libfuse does), then serves it on the calling thread.

Decisions:
- The app implements `fs_module::UnaryFileSystem` for one volume: `lookup`, `attributes`, `read_directory`, `read`,
  `parent`, optional `statistics`. Item identifiers are the FUSE node ids (the root is `ROOT_ITEM` = 1). This is the
  FSVolume and FSItem model, named in BifrOSt style.
- `fs_module::Dispatcher` answers the kernel's requests for that volume: `handle(bytes) -> Option<reply bytes>`, with
  no reply for FORGET and INTERRUPT. It adds "." and ".." to READDIR and resumes a listing by entry position.
- Read only. Opens for writing get EROFS; unknown opcodes get ENOSYS. Attribute and lookup answers are cached for one
  second by the kernel.
- The wire structs are `#[repr(C)]` with the uapi layout; `const` asserts pin their sizes (88-byte attributes,
  128-byte entries, 40-byte READ arguments, and so on).

Gaps, stated rather than skipped:
- Read only. No write, create, rename, or attribute change operations (WRITE, SETATTR, MKDIR, UNLINK and the rest).
- No shell mount service and no grant check: nothing mounts an app's module on the user's behalf, and the grant broker
  (`docbroker.rs`) does not yet cover a module's resource. Mounted volumes do not appear in the file panel sidebar.
- No `mount(2)` path (needs `CAP_SYS_ADMIN`); no optional FUSE features are negotiated, so no splice, writeback cache
  or async reads.
- No probe-then-mount step and no resource kinds beyond the path the app hands over (no block device, no generic URL).
- Written without building or running (owner rule). The protocol module has host tests from byte buffers
  (`fuse::tests`, `fs_module::tests`); `fuse_mount` has no test because it needs a mount. Parity needs a Midget check
  with a real mount before the row can be `parity`.

## Assistant system service (`springboard/src/shell/assistant.rs`, OS daemon `com.apple.Siri.agent`)

Status: partial (`com.apple.Siri.agent` is `partial`). Built and wired, but not compiled or run on Midget yet: the pure logic is in `assistant.rs`, the HTTPS client in `springboard/assistant-client`, the panel in `assistant_panel.rs`, and the Settings page in `assistant_page.rs`.

- Settings (`assistant.*` prefs): `enabled` (off), `shortcut` (`meta-shift-a`, Cmd+Shift+A; Cmd+Space is Search, Ctrl+Space the input source, Ctrl+Option+Space reader activation, so none is reused), `backend` (`off`; the only other choice is `anthropic`, the API `assistant-desktop` already speaks), `history` (off: earlier turns are neither kept nor sent), and `assistant.app.<id>` per-app access to intents (off for every app).
- Catalogue: `assistant::build` takes the installed apps' `[[intent]]` tables and keeps only allowed apps. Ids must be safe (letters, digits, `-`, `_`), a title is required, parameter specs must be `name:type` (`?` optional), descriptions are cut at 300 characters. Only these entries are sent.
- Protocol: the request carries the catalogue in the system prompt and asks for one JSON object: `{"answer":...}` or `{"app","intent","params","summary"}`. `parse_reply` returns `Reply::Answer` (plain text is an answer too) or `Reply::Intent`, and refuses unknown apps, intents or parameter names, and missing required parameters.
- Gate: an intent is only proposed as a card ("Create note 'Groceries' in Notes?", `card_text`). `Gate::confirm` hands the call out once; `execute` runs it through `intents::run_with`, the same path as Search's Shortcuts. Nothing runs on proposal.
- Speech: `speech_engine` looks for `whisper-cli` or `vosk-transcriber` on `PATH`. Without one the mic button is hidden. No engine is installed on the image yet, so the mic is hidden.

- Shortcut: `Shortcut::Assistant` (Cmd+Shift+A) in `hwkeys.rs`, switchable as `BINDINGS[11]` in `kb_input.rs`. With the assistant off it opens Settings > Assistant instead; with the chord set to Off it does nothing.
- Panel (`assistant_panel.rs`): a field, a wrapped answer area that scrolls with the arrows, and a card with Run and Cancel for one intent call. It draws with Search's material (`quicksearch::mac_glass`) and, on the desktop, as overlay surface id 6 with exclusive keyboard focus (like Search's id 3). Requests run on a worker thread through `assistant::ask` with the key from the keychain item `is.olibuijr.assistant` / `anthropic-api-key`; replies come back on a channel that `tick` polls. Nothing is sent while the backend is Off, the assistant is off, or the field is empty. Run goes through `Gate::confirm` and `assistant::execute` on another worker. History is kept only when it is on.
- Settings > Assistant (`assistant_page.rs`): the switch, the shortcut (value only; switched in Keyboard Shortcuts), the backend (Off or Anthropic API), history (off), and an access switch per installed app that declares intents. Each row saves on tap.
- Not built: speech input (no engine on the image), the assistant service registration, and the panel on non-desktop screens (the overlay exists only in the desktop path). Not yet compiled or run on Midget.

## Module index: modules without their own section

Every module below exists in the crate named and had no mention in this document. Each line is the first sentence of the
module's own doc comment; read the file for the API. Most are pure models with host tests; none counts as `parity` in the matrix
until checked on Midget (see `python3 tools/bifrost.py pc parity status`). The sections above stay the place for flows and shell
wiring.

### `bifrost_app` (`springboard/sdk/src`)

- `accessibility_display`: The Dim Flashing Lights setting, for apps, over the shell's `accessibility_display` service
- `accessory_hotspot`: Accessory hotspot joins for apps
- `actor`: Actor isolation
- `app_transaction`: The app transaction (`AppTransaction`)
- `async_publisher`: `AsyncPublisher` and `AsyncThrowingPublisher`
- `audio_devices`: Typed client for the `audio-devices` shell service
- `audio_format`: Audio stream formats for apps
- `auth_events`: Authorization events
- `ble_error`: Typed errors for the low energy central role
- `ble_midi`: MIDI over Bluetooth Low Energy packets, for apps that talk to a BLE MIDI peripheral.
- `bluetooth_devices`: Remote Bluetooth devices as an app sees them
- `bluetooth_events`: Bluetooth device events
- `bluetooth_hid`: Human interface devices (keyboards, pointers and other peripherals) as an app sees them.
- `bluetooth_inquiry`: Device inquiry (scan) as an app drives it
- `bluetooth_picker`: Bluetooth accessory picker for apps
- `bluetooth_service`: The shell's `bluetooth` service as an app uses it
- `call_stack_tree`: Call stacks captured with a diagnostic report, as a pure model.
- `central`: Central role for apps
- `core_ai`: Core AI for apps: load a model once, keep its specialized plan in the cache, and call it as a typed inference function (`InferenceSession`, `InferenceModel`); CPU dense, relu and softmax chains only, no GPU or Neural Engine (Apple parity: Core AI inference functions; not matched yet: other model formats and GPU kernels)
- `crash_report`: Crash reporter extension (the crash-report hook and the crashed-process view)
- `csv_writer`: Tabular data CSV writing (`CSVWritingError` and the writer that reports it)
- `daily_metric_report`: Daily metric reports as a pure model.
- `data_scanner`: Barcode and QR scanning for apps (`DataScannerViewController`, `RecognizedItem` barcode items, and the
- `dav_client`: CalDAV and CardDAV client for the apps that sync Internet Accounts (Calendar and Contacts).
- `dav_sync`: Internet Accounts sync for the apps that read them
- `device_registry`: Device registry for apps
- `digest`: Security transforms for apps
- `dispatch_data`: Dispatch data (`DispatchData`)
- `dispatch_time`: Dispatch time values
- `display_link`: Display timing (Core Video's `CVDisplayLink`, `CVTimeStamp` and `CVDisplayLinkGetActualOutputVideoRefreshPeriod`).
- `distributed`: Distributed parity
- `dock_tile`: The app's Dock tile through the `dock` system service (`springboard/src/shell/services/dock_tile.rs`)
- `donations`: Donations and discovery (`IntentDonationManager`, `RelevantIntentManager`, `RelevantIntent`).
- `encryption_preference`: Encryption preference for a nearby peer session
- `entropy`: Randomization services (`SecRandomCopyBytes`)
- `event_client`: Monitoring client lifecycle (parity row "Client creation and teardown").
- `exception_handler`: Failure monitoring for apps
- `external_accessory`: Accessory connections for apps
- `file_provider`: File provider domains, provider errors and enumerator signals (`domain manager`, `provider domain` and
- `file_stat`: File status records for apps
- `focus_filter`: Typed client for the `focus_filter` shell service
- `font_descriptor`: The user's font choice (`UserFonts` is the app-facing side of Settings > Fonts).
- `ftp`: FTP model for apps
- `gamepad_service`: Typed client for the `gamepad` shell service
- `gpu_layer`: The app-facing drawable layer
- `gss_buffer`: GSS-API buffer management (`gss_buffer_desc`, `gss_buffer_set_desc`, `gss_create_empty_buffer_set`,
- `gss_context`: GSS-API context services (`gss_init_sec_context`, `gss_accept_sec_context`, `gss_context_time`,
- `gss_credential`: GSS-API credential management (`gss_acquire_cred`, `gss_inquire_cred`, `GSSCredentialGetLifetime`,
- `gss_krb5`: The Kerberos 5 settings of the GSS-API (`gss_krb5_ccache_name`, `gss_krb5_set_allowable_enctypes`,
- `gss_mechanism`: GSS-API security mechanisms (`gss_indicate_mechs`, `gss_indicate_mechs_by_attrs`, `gss_inquire_attrs_for_mech`,
- `gss_message`: GSS-API message protection (`gss_get_mic`, `gss_verify_mic`, `gss_wrap`, `gss_unwrap`, `gss_wrap_size_limit`)
- `gss_name`: GSS-API name handling (`GSSCreateName`, `gss_import_name`, `gss_display_name`, `gss_compare_name`,
- `gss_oid`: GSS-API object identifiers (`gss_OID`, `gss_OID_set`, `gss_oid_to_str`, `gss_oid_equal`,
- `gss_status`: GSS-API function status (`GSS_S_COMPLETE`, `GSS_S_FAILURE`, `GSS_C_MECH_CODE`, `gss_display_status`,
- `gss_token`: GSS-API token management (`gss_encapsulate_token`, `gss_decapsulate_token`)
- `hid_devices`: HID devices for apps
- `hitch_ratio`: Animation hitch measurement as a pure model.
- `host_addressing`: Host name addressing
- `hotspot_configuration`: Hotspot configuration for apps
- `http_auth`: `URLCredential`, `URLCredentialStorage`, `URLProtectionSpace` and HTTP Basic challenge parity for the URL loading
- `http_cookies`: `HTTPCookie` and `HTTPCookieStorage` parity for the URL loading system
- `identity_events`: User and group ID change events
- `inference`: Typed inference signatures
- `input_events`: Input events for apps, over the shell's `events` service (`springboard/src/shell/services/event_services.rs`)
- `intent_resolve`: Parameter resolution for App Intents
- `launch_services`: Open With and default apps, over the shell's `launch` service (`springboard/src/shell/services/launch_services.rs`).
- `login_items_approval`: Typed client for the shell's `login-items` service
- `markup`: Markup canvases for apps (`PaperMarkup`, the insertion menu of `MarkupEditViewController`, `FeatureSet` and
- `memory_events`: Memory mapping and protection events
- `messaging`: Communication notifications and messaging requests
- `multipart`: `addMultipartData(_:withName:type:filename:)` parity
- `music_understanding`: `MusicUnderstanding` for apps (see `docs/parity/music-understanding.md`)
- `ndarray`: Multidimensional arrays
- `nearby_service`: Nearby service type
- `net_diagnostic`: Connection diagnosis
- `net_totals`: Network byte totals for apps
- `network_browser`: Local service discovery parity
- `network_connection`: Connection and listener parity
- `network_path`: Network path for apps
- `notification_queue`: `NotificationQueue` parity
- `notify_bus`: Named notifications between apps, over the shell's `notify_bus` service (`springboard/src/shell/services/notify_bus.rs`).
- `now_playing`: Now Playing for apps
- `object_graph`: Object graph and persistent store
- `observable_object`: `ObservableObject` parity (Combine) `derived_state` adds the computed-property rule the macro lacks: a `DerivedState` mirrors the `Observable`s it reads (the fields of an `observable_model!` among them) as inputs of `bifrost_ui::attribute_graph::Graph`, and a `Derived` value recomputes only when something it read changed, so a field write costs one recompute of what depends on it. Not built: the macro does not declare derived fields itself (build them with `DerivedState::derive`); the shell's view bodies are not computed through the graph.
- `passkey`: Passkey request inputs
- `passkey_client_data`: Client data for a passkey ceremony
- `password_gen`: Password generation rules (a generate-password request for a sign-up form)
- `peer_identity`: Peer identity
- `peer_mesh`: Peer mesh
- `peer_session`: Peer sessions
- `permission_kit`: Permission requests for apps (the app side of the parent or guardian approval flow).
- `pixel_buffer`: Pixel buffers and their pool (Core Video's `CVPixelBuffer`, `CVPixelBufferLockBaseAddress`, `CVPixelBufferGetBaseAddress`,
- `post_delivery`: Suspension behaviours for cross-app broadcast posts
- `power_service`: Typed client for the shell's `power` service (`springboard/src/shell/services/power_service.rs`).
- `power_sources`: Power sources for apps
- `privacy_permissions`: Typed client for the shell's `privacy` service
- `process_events`: Process event records and the subscription set that filters them
- `reachability`: Reachability for apps
- `review`: Review prompts (`RequestReviewAction`)
- `route_events`: Route events (`AVCustomRoutingEvent` parity, see `docs/parity/w3-theme-30.md`)
- `save_sync`: Synced save directories (`GameSave` parity)
- `screen_sample`: Screen pixel sampling for apps that show what is under the pointer (the Color Meter).
- `screen_saver_defaults`: `ScreenSaverDefaults`
- `screen_time`: Screen Time configuration for apps
- `script_export`: Native object export to scripts
- `script_syntax`: Script syntax check parity
- `script_value`: Script value parity
- `search_ranking`: Ranked search over the items this app indexed with Core Search, item expiry, and language identification.
- `serial_ports`: Serial ports for apps
- `sign_in`: Sign-in for apps
- `signpost`: Signposts for apps
- `synchronization`: Synchronization parity
- `system_prefs`: System preferences for apps
- `text_services`: Typed client for the shell's `text` service
- `timer_source`: Timer dispatch source (`DispatchSourceTimer`) as a pure state model, with its lifecycle (`DispatchSourceProtocol`)
- `translation`: `TranslationSession` parity
- `url_components`: `URLComponents` and `URLQueryItem` parity
- `usb_printer`: USB printers for apps
- `visual_search`: Visual search for apps
- `vpn_configuration`: Personal VPN configuration for apps
- `web_auth`: Shared-login authentication sessions for apps, shaped like the system web-session API.
- `web_extension`: Browser-style web extensions for apps
- `widgets_timeline`: Client for the shell's `widgets_timeline` service (`springboard/src/shell/services/widgets_timeline.rs`)
- `wifi_accessory_browser`: Wi-Fi accessory browser for apps
- `wifi_secrets`: Wi-Fi network secrets for apps, kept per network name (SSID) and keychain domain
- `wifi_status`: Wi-Fi connection status for apps
- `wireless_accessory_browser`: Wi-Fi setup for unconfigured accessories
- `wlan`: Wireless LAN scan data for apps
- `wlan_events`: Wi-Fi change events for apps
- `wlan_keychain`: Wi-Fi secrets for apps, kept as generic keychain items (see `keychain`)

### `bifrost_ui` (`springboard/ui/src`)

- `access_control`: Access control for stored secrets and authorization rights.
- `accessible_look`: The accessible-appearance decisions an app makes from the user's display settings
- `acroform`: PDF form fields (widget annotations) as a pure model
- `actionrequest`: The request behind one chosen file action
- `advertising`: Advertising data
- `analyzer_input`: Time-coded audio input for a speech analyzer.
- `animated_values`: Keyframed values over time (`NativeAnimatedValue`, `NativeAnimatedScalar`, `NativeAnimatedVector3`,
- `annotation`: Page annotations as data
- `assetmock`: A local test source for asset packs
- `assetpack`: Asset packs (`AssetPackManager`, `AssetPackManifest`, `AssetPack`)
- `audio_render`: Audio render cycle
- `auth_view`: Authentication view model (task theme-03)
- `authorization_request`: Authorization requests (parity
- `ax_audit`: Accessibility audit (the checks of the Accessibility Inspector developer tool)
- `bidi`: Writing direction and bidirectional text
- `blit`: Buffer fill and copy passes
- `bookmark_import`: Reads a bookmark export in the HTML form that browsers write
- `braille`: Braille translation for a Braille display
- `browser_settings`: System-wide browser settings
- `browser_sheet`: A standard in-app browser sheet as a pure model.
- `bt_chooser`: Device chooser
- `bt_passkey`: Passkey display
- `calendar_conference`: Virtual conference rooms for calendar events, as a provider app holds them
- `calendar_error`: Error codes for calendar and reminder store operations
- `calendar_participant`: A person, group, room or resource invited to a calendar event
- `calendar_store_changes`: Change notices for a shared calendar store.
- `call_directory`: Caller identification and blocking
- `camera_device`: Camera device model
- `camera_region`: Stabilised camera region of interest
- `captions`: Caption preferences
- `capture_access`: Capture permission for one app
- `capture_config`: Stream configuration
- `capture_content`: Shareable content
- `capture_error`: Stream errors
- `capture_event`: Capture events from the hardware capture buttons of a device, as pure state.
- `capture_filter`: Content filters
- `capture_picker`: System content-sharing picker
- `capture_shot`: Single-frame screenshot settings
- `capture_sync`: Synchronized capture
- `certificate_panel`: Certificate panel
- `certificate_trust`: Certificate trust editor
- `channel_layout`: Channel layouts now live in `bifrost-audio-types` (docs/audio-architecture.md)
- `chart`: Chart geometry for data-driven charts
- `chart_ax`: Accessible chart semantics
- `chart_content`: A chart as a list of marks.
- `chart_marks`: Mark geometry for data charts
- `chart_scroll`: Scroll targets for charts that scroll along a value axis.
- `chart_surface`: Surface plots for 3D charts
- `chart_vector`: Batched plots for large data sets.
- `clip_buffer`: Rolling clip buffer (`ClipBuffer`)
- `cnn_layers`: Eager layers for small neural networks
- `column_summary`: Statistical summaries of one column of a table (the `NumericSummary` and `CategoricalSummary` model)
- `command_buffer`: The life of a GPU command buffer as a pure state machine
- `conference`: Participants and virtual conferences of a calendar event.
- `contact_card`: Contact card model
- `contact_changes`: Batched saves and change history for a set of records (contacts, or any record type).
- `contact_display`: Contact display defaults
- `contact_groups`: Contact groups with subgroups
- `contact_photo`: Contact pictures
- `contact_picker`: Contact picker model
- `content_proposal`: Next-item proposals as pure state
- `content_rules`: Content-blocking rules
- `context_element`: What a right click landed on, and which menu to show for it (the reference SDK's context menu element info).
- `context_window`: Context window budget
- `controller_motion`: Motion profile of a controller
- `controller_power`: Battery and light of a game controller
- `coordinate_space3d`: A tree of 3D coordinate spaces
- `counter_sample`: A fixed-size buffer of timestamped counter samples, as a pure model for measuring work between two points.
- `credential_identity`: Credential identities (parity
- `data_store`: A pluggable record store
- `default_device`: Default audio devices
- `device_browser`: Device browser
- `device_match`: Device matching by capability
- `directory_node`: Directory sessions and nodes
- `directory_query`: Directory records and queries
- `display_info`: Display information
- `dock_tile`: The app's Dock tile (`NativeDockTile`)
- `docscan`: Document scanning geometry
- `dom_snapshot`: A snapshot of a page's document tree
- `downloadqueue`: Scheduled asset downloads (a download manager queue)
- `drawable_layer`: A drawable surface an app renders into and presents to the window server.
- `echo_cancel`: Echo cancellation for voice capture
- `ecs`: Entities and components
- `embedding`: Text embeddings
- `emitter`: Particle emitter model
- `environment`: Environment values propagated through a view hierarchy (`EnvironmentValues`)
- `evaluation`: Model-quality evaluation
- `event_distance`: Distance models and directivity for a sound source
- `event_envelope`: Envelopes
- `event_tap`: Low-level input events and event taps.
- `export_session`: Export session model
- `face_group`: Face grouping for `FaceGroupAnalyzer`
- `features`: Typed model inputs and outputs (`MLFeatureValue`, `MLFeatureProvider`, `MLDictionaryFeatureProvider`,
- `file_provider`: Provided items of a file provider domain (`provided item` parity)
- `file_sync`: Sync-status state a file provider keeps for the Files app
- `fileaction`: Actions a provider adds to the context menu of the system document browser
- `filebadge`: Badges that a file-manager extension puts on files and folders
- `flashing_lights`: Flashing light detection and dimming as a pure model.
- `focus_filter`: Focus filters
- `format_readers`: Runtime registry of media format readers (the registration of professional video workflow readers,
- `fourier`: Fourier and cosine transforms over `f64` slices
- `frame_clock`: The frame clock
- `frame_grab`: Frame grab model
- `frame_rate`: Frame-rate ranges an app asks for (`CAFrameRateRange`, `CADisplayLink.preferredFrameRateRange`) as a pure model.
- `fs_maintenance`: Client interaction and maintenance (`maintenance client` parity)
- `fs_probe`: Containers and probing (`probe and container` parity)
- `fs_volume`: Volumes and items (`volume and item` parity)
- `gamepad`: Gamepad input model
- `gatt`: GATT client model for the central role
- `gatt_server`: GATT server model for the peripheral role
- `generation_schema`: Structured output
- `geo`: Map coordinates on a Web Mercator projection
- `geofence`: Circular geographic regions and an enter and exit monitor for them.
- `grid_graph`: Grid navigation
- `guardrail`: Output guardrail for generated text
- `hfp`: Hands-free profile session model
- `hid_match`: HID device matching
- `hid_report`: HID report descriptors
- `highlight_rank`: Highlight segments for `HighlightAnalysisRequest`
- `identity_chooser`: Choose identity panel
- `imroute`: Where a key or mouse event goes while a text input session is active.
- `imsession`: A text input session on the provider side
- `imstate`: The state of an input method as the shell keeps it
- `index_client`: Index maintenance
- `indexset`: Sets of integers (`IndexSet`)
- `indirect_command`: A fixed-size list of reusable GPU commands that are encoded once and executed later, as a pure model.
- `input_picker`: Audio input picker as pure state
- `input_switches`: Switch state of an input device
- `intent_access`: Consent for voice-assistant and directions requests.
- `intent_resolve`: Parameter resolution for typed actions
- `intent_vocab`: Custom vocabulary for typed and spoken requests
- `internet_accounts`: Internet Accounts
- `ipp`: IPP/2.0 binary codec (RFC 8010 framing, RFC 8011 attributes), the subset the printer client needs.
- `item_attrs`: Per-item attributes that a file provider can set for an item and the Files app can read back
- `itemattrs`: Per-item tags and last-used dates that a file-manager extension keeps with each file
- `jws`: The structure of a signed transaction in JWS Compact Serialization (`VerificationResult`, `VerificationError`)
- `keychain_save`: Keychain save form
- `keychain_settings`: Keychain settings
- `keyframe_pick`: Representative frame selection for `KeyFrameAnalysisRequest`
- `layer_bridge`: Renders a [`Layer`] tree through the styled-layer rasteriser.
- `layer_layout`: Layer delegates, layout managers and the constraint layout manager.
- `layer_render`: CPU rendering of a styled layer
- `layer_style`: Layer appearance rules
- `layer_tree`: The retained layer tree
- `layout_protocol`: Custom layout protocol
- `lexical_tags`: Lexical tagging of text
- `linalg`: Vectors, 4x4 matrices and unit quaternions for 3D content (`NativeVector3`, `NativeVector4`, `NativeMatrix4`, `NativeQuaternion`).
- `linkmeta`: Rich links as pure models
- `live_activity`: Live activities
- `location_reminder`: Location-based reminders
- `lu`: Dense linear systems
- `map_item`: A place shown in a detail sheet
- `map_overlay`: Map overlays as pure data
- `maps_link`: Map links
- `mapshape`: Map overlays
- `mapview`: Map camera, zoom levels, scale bar and user-tracking state for a map view.
- `markup`: Markup documents (`NativeMarkupDocument`, `NativeMarkupOrderedSet`, `NativeMarkupID`, the element kinds and
- `markup_drawing`: Drawings
- `match_pattern`: Match patterns
- `media_artwork`: Artwork for a media item as a pure model
- `media_block`: Block buffers and a bounded memory pool for media samples.
- `media_changes`: Change records for a media library
- `media_content`: Typed now-playing content
- `media_feedback`: Feedback and rating for the playing item as a pure model.
- `media_playlist`: Playlists as a pure model
- `media_query`: Media library queries
- `media_request`: Media playback requests
- `media_search`: Media search requests
- `media_selection`: Language options and accessibility track choice as a pure model.
- `media_session`: Local Now Playing sessions as a pure model
- `media_sources`: Media sources and the load options that choose them, as a pure model.
- `media_time`: Rational media time
- `media_timebase`: Timebases
- `media_trim`: Trimming session for a media player view as a pure model.
- `media_type_names`: Localized names of media types and media subtypes (`MTCopyLocalizedNameForMediaType`,
- `menu_extras`: System menu extras of the menu bar
- `mesh_file`: Mesh files as plain data
- `mesh_primitives`: Built-in 3D shapes as vertex data
- `mesh_process`: Mesh processing on `PrimitiveMesh` data
- `mic_consent`: Microphone consent and ownership
- `midi_ble`: BLE MIDI packets
- `midi_ble_picker`: Bluetooth MIDI picker and advertiser
- `midi_events`: MIDI 1.0 channel messages as app-facing events
- `midi_thru`: MIDI thru connections
- `ml_model`: A model that runs
- `ml_state`: Named state buffers that a stateful model keeps from one prediction to the next.
- `ml_tensor`: Multi-dimensional numeric arrays for model inputs and outputs (`MLTensor`)
- `ml_update`: On-device model updates
- `model_assets`: Models that arrive at runtime, tracked as named assets.
- `model_attachment`: Image attachments for a prompt.
- `model_history`: Change history with transaction tokens (`HistoryDescriptor`, `HistoryTransaction`, `HistoryChange`).
- `model_provider`: The model behind a session.
- `model_foundation`: Model ids (`name[:tag]`), `ModelInfo` capabilities, `Request` with `GenerationOptions` and its
  `check` against a model, `Response` with estimated `Usage` and `FinishReason`, and a `ModelRegistry` that resolves a
  name. Pure types; no runtime answers yet.
- `model_schema`: Attribute options for the fields of a model entity
- `model_session`: A model session with profiles, dynamic instructions and typed session properties.
- `model_specialization`: A cache of specialized model artifacts, keyed by model name and specialization options.
- `model_store`: An in-memory record store with a model-context style API
- `model_tool`: Tools a model may call while it answers.
- `morph_skin`: Shape morphing and skeletal skinning as pure models (`NativeMorpher`, `NativeSkinner`).
- `nav_policy`: Navigation policy for web content
- `nn`: Output sizes of windowed operators (convolution and pooling) and the scale factor of a Fourier transform.
- `node_constraint`: Node constraints
- `notifattach`: Notification attachments
- `notifcomm`: Communication notifications
- `notifdelivery`: How a notification is presented
- `notifmedia`: Play and pause on a notification card
- `notifservice`: Notification content changes before delivery
- `object_push`: Object push transfer panel
- `obstacle`: Impassable areas that steering code routes around
- `one_time_code`: One-time codes (task theme-28)
- `onscreen_entity`: Onscreen context for app entities
- `param_view`: Generic parameter view
- `particle_system3d`: A 3D particle system
- `passkey_display`: Passkey display for pairing
- `pcm_convert`: Linear PCM conversion
- `pdf_access`: PDF access permissions and write options as a pure model.
- `pdf_overlay`: PDF page overlays and markup mode as a pure model.
- `pdfannot`: PDF annotations as a pure model
- `pdfform`: PDF form widgets as a pure model
- `pdfpage`: PDF page geometry as a pure model
- `pdfselect`: Text selection and find over the text of a PDF document, as a pure model.
- `pdfview`: PDF document view layout as a pure model
- `peripheral`: Peripheral role
- `permission_question`: Permission questions an app sends to a guardian, and the answers that come back.
- `photo_caching`: Preparing thumbnails ahead of need.
- `photo_changes`: Change details for a photo library (the change record a library observer receives).
- `photo_request`: A batch of changes to a photo library, planned before anything is written
- `photo_smart_albums`: Smart albums of a photo library
- `physics`: Rigid-body physics world
- `physics3d`: Three-dimensional rigid-body world
- `playback_arbiter`: Playback arbitration between participants
- `playback_rate`: Playback rate commands
- `plugin_host`: Plugin host model.
- `plugin_view`: Plugin view model
- `poi_filter`: Point of interest categories and the filter that includes or excludes them.
- `pose`: Rotations, Euler angles, and poses (a position with a rotation, optionally scaled) for 3D content.
- `postal_address`: Structured postal addresses and their mailing layout.
- `precipitation`: Precipitation (`Precipitation`)
- `pressure_trend`: Pressure trend (`PressureTrend`)
- `primitives3d`: Built-in 3D shapes as plain vertex data
- `procedural_texture`: Procedural textures for material inputs
- `quadrature`: Numerical definite integration
- `quadtree`: Spatial partitioning
- `racing_wheel`: Racing wheel input model
- `reading_list`: A reading list
- `record_asset`: Assets attached to records
- `record_share`: Record sharing
- `record_store`: Record database
- `record_sync`: Sync queue, conflicts and error codes for the record database in `record_store`.
- `recordsearch`: Record search with compound elements
- `regex`: Regex parity (`Regex`, `Regex.Match`)
- `relevance`: Relevance ranking for a stack of glanceable content
- `remote_command`: Remote command models for the playback rate, rating and feedback commands of the media player framework.
- `remote_view`: A view that another process draws and the host shows in its own window.
- `render_loop`: A render loop as a pure model
- `render_synchronizer`: Render synchronizer model
- `reverse_lookup`: Nearest-place lookup
- `route_action`: Custom action items for a route picker.
- `route_auth`: Per-app authorization of third-party media routes, and the active state of each route.
- `ruby`: Ruby annotations
- `rule_system`: Rule systems for game logic
- `run_delegate`: Run delegates and adaptive images for styled text.
- `sample_attach`: Per-sample attachments and data-readiness notifications for a run of media samples.
- `sample_attachments`: Sample attachments
- `save_sync`: Synced save directories
- `scan_job`: Scanner device model
- `scanner`: Scanners over the SANE command line (`scanimage`)
- `scene2d`: Two-dimensional scenes as a pure model
- `scene3d`: Scene tree for 3D content
- `scene_action`: Actions for scene nodes
- `scene_audio`: Positional audio players for a 3D scene
- `scene_light`: Light sources for 3D scenes (`NativeLight`, `NativeLightType`, `NativePhysicallyPlausibleLight`).
- `scene_loop`: The lifecycle and frame cycle of a game scene as a pure model.
- `scene_material`: Surface materials for 3D objects (`NativeMaterial`, `NativeMaterialProperty`, `NativeMaterialSemantic`,
- `scene_object`: Object hierarchy for 3D assets (`NativeObjectContainer`, `NativeObject`, `NativeNamed`, `NativeTransform`).
- `scene_source`: Scene asset import
- `scene_view`: A view that shows a `Scene` through one of its cameras, as a pure model of the reference framework's scene
- `scheduled_region`: Scheduled playback on a sample timeline.
- `screensaver`: Screen saver module model (`ScreenSaverView`) and its utilities (`SSRandomIntBetween`, `SSRandomFloatBetween`,
- `scripting`: Scripting object model
- `searchindex`: Named search indexes and their descriptions (`CSSearchableIndex(name:protectionClass:)` and
- `searchquery`: App-side search queries
- `services_menu`: The Services menu
- `shared_list`: Shared file list
- `skip_interval`: Interval skips
- `smartcard_slot`: Smart card reader slots
- `sound_analyzer`: Sound classification requests and the analyzer that runs them over audio input.
- `sound_classify`: Sound classification
- `sound_distance`: Distance attenuation and directional emission for sound sources.
- `sound_envelope`: Breakpoint envelopes for audio parameters over a linear input (time, distance or a game value).
- `sound_event`: Sound event trees
- `sound_group`: Sound groups and ducking.
- `sound_result`: Sound classification results.
- `sound_room`: Room reverberation and the direct to reverberant balance of a sound.
- `sound_space`: Sound in a 3D scene
- `spatial`: Points, sizes, boxes, axes, rays and spherical coordinates in 3D space, built on `linalg::Vector3`.
- `speech_assets`: Speech model assets
- `stardict`: Dictionaries for the Dictionary app and the Look Up popover, in the StarDict layout (an `.ifo` description, an `.idx`
- `state_machine`: Finite state machines for game logic
- `state_report`: State reporting
- `store_test`: A store test session for purchase flows, the counterpart of the StoreKit testing harness (`StoreKit Test`).
- `strategist`: Turn-based game AI as pure search over a game model.
- `stylus`: Digitizer stylus input as a pure model.
- `surface_attach`: Attachments
- `surface_colour`: Colour tagging for a shared buffer
- `surface_layout`: Pixel formats and plane layout for a shared buffer
- `surface_lock`: CPU lock state for a shared buffer.
- `surface_use`: Use counts for a shared buffer.
- `switch_scan`: Switch Control item scanning (`AccessibilityTechnology.switchControl`)
- `symbol_effect`: Symbol effects for system symbol images (`SymbolEffect`, `DiscreteSymbolEffect`, `IndefiniteSymbolEffect`,
- `symbol_layers`: Symbol layers
- `tensor`: Tensor types for compute graphs
- `text_classifier`: A text classifier that runs
- `text_marker`: Text markers and marker ranges over one text element
- `text_markup`: Text markup for legible media
- `textbreak`: Text segmentation
- `textmetrics`: Where text widths come from.
- `texture_descriptor`: Texture descriptors for GPU resources
- `thumb_service`: The system thumbnail model
- `tilemap`: Tile map model
- `timeline`: Timelines for glanceable content (`Timeline`, `TimelineEntry`, `TimelineReloadPolicy`)
- `tips`: Contextual tips
- `token_objects`: Objects a cryptographic token stores (keys, certificates and identities), read as keychain-like items.
- `token_registration`: Smart card sign-in
- `token_registry`: Registration of smart cards by token ID, and the policy that says when a card is needed to log in.
- `token_watch`: Cryptographic tokens
- `tool_calls`: Tool calls
- `transcript`: A session transcript
- `transform3d`: 3D transforms
- `translation`: `Translation` framework parity models
- `ubiquitous_store`: Small key-value store that keeps a set of settings in step across a person's devices.
- `url_filter`: URL filter
- `utdecl`: App-declared file types
- `value_query`: The result of an intent value query
- `vcard`: vCard 3.0 cards for contacts
- `vector_ops`: Vector arithmetic over slices of `f32`
- `verification_codes`: One-time codes that arrive in messages and mail.
- `vertex_descriptor`: Vertex layout
- `vertical_text`: Vertical text layout
- `video_device`: Video devices and stream formats
- `video_property`: Extension properties
- `video_session`: Video encoder session properties and the catalogue of encoders.
- `video_settings`: Encoder settings for a video output
- `video_transition`: Opacity over time for each video track of a composition
- `view_camera`: Camera for 3D views (`NativeCamera`, `NativeCameraProjection`)
- `virtual_controller`: Virtual on-screen controller
- `visual_search`: Visual search model
- `vm_error`: Error codes for configuring and running a guest virtual machine.
- `vm_share`: Host directories shared with a guest virtual machine.
- `vm_state`: Execution state of a guest virtual machine and the lifecycle operations allowed in each state.
- `voice_activity`: Voice activity detection
- `voice_commands`: Spoken commands for driving the interface by voice
- `volume_control`: A device's volume control
- `voxel_array`: Volumetric voxel grids for 3D objects (`NativeVoxelArray`, `NativeVoxelIndex`, `NativeVoxelIndexExtent`).
- `weather_alert`: Weather alerts (`WeatherAlert`)
- `weather_attribution`: Data attribution for weather (`WeatherAttribution`)
- `weather_model`: Weather models that need no network
- `weather_stats`: Historical weather statistics
- `web_auth`: Shared-login authentication session, shaped like the system web-session API
- `web_extension`: Browser-style web extensions as a pure model
- `web_history`: Web history per app
- `web_service_token`: Web service tokens
- `web_usage`: Web-usage reporting for one webpage.
- `webauth_session`: Web sign-in sessions
- `webpage_usage`: Web page usage
- `widget_action`: What a tap on a widget does
- `widget_arrange`: The widgets a user placed on a surface (Today view, desktop or lock screen) and the edit mode that changes them
- `widget_ax`: The spoken description of a widget for assistive technology
- `widget_config`: The settings a user can give a widget (the user-configurable options of a widget configuration)
- `widget_date`: Dated text inside a widget that stays current while the app is not running
- `widget_kind`: The widgets an app offers
- `widget_link`: The link a widget opens when it is tapped (`widgetURL(_:)` and `Link` in a widget).
- `widget_list`: The list a Today widget shows its content in (the Notification Center framework's list controller).
- `widget_search`: The search view of a Today widget (the reference SDK's default search view for a widget, deprecated there in favour
- `window_list`: Window list
- `wire_json`: Small helpers shared by the models that cross the shell service wire (displays, windows, input events)
- `work_stream`: An ordered stream of GPU or CPU work with dependencies derived from the resources each unit reads and writes.
- `workflow`: An ordered workflow of steps (`NativeWorkflow`) and its controller (`NativeWorkflowController`).
- `writing_assist`: Writing assistance for a text view

### `bifrost_render` (`springboard/render/src`)

- `bayer`: Bayer colour-filter-array demosaic with white balance, as a pure model (std only).
- `bezier`: Bezier paths
- `block_motion`: Block motion estimation between two 8-bit planes, as plain std code.
- `blur_kernels`: Directional and disc blurs on packed pixels
- `buffer_attach`: Colour metadata that travels with an image buffer
- `buffer_pool`: A recyclable pool of buffers.
- `cff`: CFF outlines for OpenType fonts with a `CFF ` table (OTTO)
- `colorcube`: A three-dimensional colour lookup table.
- `colorlab`: CIE L*a*b* colour for sRGB values, the perceptual space that colour conversion and palette matching
- `colormap`: Colour maps and palettes for packed pixels.
- `colorspace`: Named RGB working spaces and the matrix conversions between them.
- `display_list`: `DisplayList`
- `emitter`: A particle emitter model.
- `fontfallback`: The process-wide fallback faces
- `fontinfo`: Font file facts read straight from the sfnt tables
- `fontobj`: App-facing font objects
- `frame_interp`: Frame interpolator model for CPU rasters
- `gamma_fade`: Gamma fade ramp for a screen saver's start and stop (the gradual fade behind `performGammaFade`).
- `guided`: Edge-preserving guided filter on single-channel `f32` planes (He, Sun and Tang's guided filter).
- `gvar`: TrueType variations
- `hdr_frame`: Per-frame HDR metadata for video pictures, as plain std code.
- `hdr_metadata`: HDR transfer functions and per-frame light metadata, as a pure model (std only).
- `hdr_transfer`: High dynamic range transfer curves and extended range headroom, as plain f64 math.
- `icc`: Reads ICC colour profiles and keeps the profile each device uses.
- `image_tiles`: Tile grids for large pictures.
- `imgdec`: Scaling helpers that turn decoded pictures into thumbnails and screen-fitted images.
- `imgtile`: Tiled access to a large picture, as plain std code.
- `integral`: Summed-area tables and region statistics on 8-bit single-channel planes
- `lazy_image`: Lazily evaluated images
- `pixel_format`: Pixel format descriptions for buffers that the system allocates and other code reads
- `pixel_layout`: Pixel buffer layout descriptors and the byte conversions between the renderer's premultiplied 8-bit pixels
- `plane_arith`: Element-wise arithmetic and transposition of 8-bit planes.
- `plane_stats`: Reductions, integral images and region statistics over 8-bit planes.
- `raw_params`: Processing parameters for a RAW image processor, as a pure model (std only).
- `replicator`: Replicated instances of one source
- `shape_ot`: An OpenType text shaper
- `temporal_upscale`: Temporal upscaler model for CPU rasters
- `video_transfer`: Pixel transfer and pixel rotation for decoded video pictures, as plain std code.

## Core Graphics: drawing context (`CGContext`, `CGPath`, `CGAffineTransform`, `CGGradient`, `CGLayer`)

Code: `springboard/sdk/src/graphics_context.rs` (`GraphicsContext`, `Path`, scanline rasterizer),
`graphics_stroke.rs` (`LineCap`, `LineJoin`, `StrokeStyle`, stroker and dashes), `graphics_gradient.rs` (`Affine`,
`Gradient`, `GradientOptions`). A view draws through `Canvas::draw(|ctx, size| ...)`; the result is a premultiplied
`Layer`. Coverage is exact horizontally and sampled at four sub-scanlines vertically.

| Apple | BifrOSt |
|---|---|
| `saveGState`, `restoreGState` | `save_gstate`, `restore_gstate`: CTM, clip, colours, alpha, blend, shadow, line parameters |
| `ctm`, `concatenate(_:)`, `translateBy`, `scaleBy`, `rotate(by:)` | `ctm`, `concatenate`, `translate_by`, `scale_by`, `rotate_by` |
| `CGAffineTransform` | `Affine`: `translation`, `scale`, `rotation`, `concatenating`, `inverted`, `translated_by`, `scaled_by`, `rotated_by`, `apply` |
| `beginPath`, `move`, `addLine`, `addPath`, `closePath` | `begin_path`, `move_to`, `add_line`, `add_path`, `close_path` (points are placed with the CTM at call time) |
| `fillPath(using:)`, `strokePath()`, `clip(using:)` | `fill_path(rule)`, `stroke_path()`, `clip_path(rule)`; each clears the current path |
| `clip(to: rect)` | `clip_to_rect`; `clip_with(path, rule)` for an explicit path |
| `setLineWidth`, `setLineCap`, `setLineJoin`, `setMiterLimit`, `setLineDash(phase:lengths:)` | same names in snake case |
| `setFillColor`, `setStrokeColor`, `setAlpha`, `setBlendMode`, `setShadow(offset:blur:color:)` | `set_fill_color`, `set_stroke_color`, `set_alpha`, `set_blend_mode`, `set_shadow` |
| `CGMutablePath` `addArc`, `addCurve`, `addQuadCurve`, `addEllipse`, `addRoundedRect`, `addRect`, `addPath` | `Path::add_arc` (y-down, increasing angle unless `clockwise`), `add_curve`, `add_quad_curve`, `add_ellipse`, `add_rounded_rect`, `add_rect`, `add_path` |
| `CGPath.contains`, `boundingBoxOfPath`, `currentPoint`, `copy(using:)` | `Path::contains(x, y, rule)`, `bounding_box`, `current_point`, `transformed` |
| `CGGradient`, `drawLinearGradient`, `drawRadialGradient` | `Gradient::new` / `Gradient::evenly`, `draw_linear_gradient`, `draw_radial_gradient` (two circles) with `GradientOptions { draws_before_start, draws_after_end }` |
| `CGLayer` | `GraphicsContext::new_layer_context(w, h)` and `draw_layer_at(&layer, dx, dy)`; `draw_layer` for a group |
| `CGColorSpace`, `CGColor`, `CGColorConversionInfo` | `graphics_color`: `ColorSpace` (device, sRGB, Display P3, Lab, indexed, ICC via `space_from_profile`), `Color` (`srgb`, `from_rgba8`, `converted`, `to_rgba8`), `ColorConversionInfo`, `RenderingIntent` |
| `CGPDFDocument`, `CGPDFPage`, `CGPDFContext` (write side) | `graphics_pdf`: `Document::open`, `page_count`, `page_size`, `render`; `PdfWriter` with `add_page`, `fill_rect`, `stroke_line`, `draw_rgb_image`, `finish`. No text drawing or annotation editing yet |
| `CGImageSourceCreateWithData`, `CGImageDestination`, `CGImageSourceCopyTypeIdentifiers` | `graphics_imageio`: `detect`, `pixel_size`, `decode`, `encode(format, w, h, rgba)`, `encode_jpeg`, `can_write`. WebP is read only |

Decisions and gaps: a stroke measures its width and dash with the CTM at stroke time, using the uniform scale
`sqrt(|det|)`, so a non-uniform CTM gives an average width. Gradients are not shadowed. Gradient colours interpolate in
unpremultiplied RGBA, not in a colour space. The old stroker used round joins and caps; the default is now butt caps
and miter joins with a limit of 10, as in Core Graphics. No PDF context (printing task), no patterns or shadings.
Tests are host golden-pixel tests, not yet run on Midget.

#### Declarative drag and drop and share by representation (Sprint 11)

`bifrost_app::transfer_drag` builds on `Transferable`. `draggable(&reps, &value)` returns a `Dragged` (the session items offering every type
the value exports, a promised file for a file representation, and the data per type). `DropTarget::new(id, reps, action)` implements
`bifrost_ui::drag_session::Destination`: it registers for the importable types, answers the operation set with `wanting`, exposes
`is_targeted()` (the `isTargeted` binding, cleared by the new `Destination::exited`), and on drop rebuilds the values for `action`.
`transfer::share_items` and `share::show_transferable` / `SharingService::sharing_services_for_transferable` drive the share menu from the
exported representations; `transfer::paste_values` reads every pasteboard item a model can import.
Not done yet: the shell pasteboard wire still carries one item, and the view kit has no modifier that attaches these to a view.

Wave 15 (2026-10-19): `transfer::drag_items`, `begin_drag_all` and `share_items_all` move a selection of values as several items
(one per value, at most eight); `transfer::drop_values_with_files(reps, items, resolve)` reads dropped file items through the first
file representation whose type accepts the file (`Representations::best_file_import`), with `resolve` mapping the item's grant and
name to the arrived file. On the shell side the general pasteboard now keeps each item's representations together
(`shell::pasteboard::set_items`, `snapshot_items`), so `pasteboard` service writes and reads of `general` return the same
items the writer sent; the app wire (`ToShell::PasteboardSet`, `ToApp::Pasteboard`) is still flat, and the Wayland selection still
offers one MIME type. Not done: a file promise (a lazily resolved file on a pasteboard item) and a file drag source.

## Touch events (`bifrost_ui::touch`, `bifrost_app::touch_events`)

Apps can take full touch events instead of the single-pointer `Input::Touch` stream.

- Model (`springboard/ui/src/touch.rs`): `Touch` (id, `TouchPhase` began / moved / stationary / ended / cancelled, location and
  previous location, timestamp, tap count, optional force, max force and major radius, `TouchType` direct / pencil / indirect)
  and `TouchEvent`, the touches of one frame. A frame lists the changed touches and every other touch that is down as
  `Stationary`. `TouchTracker` builds frames from raw down / move / up / cancel samples and counts taps (same type, within
  400 ms and the slop of the last tap; a drag, a hold over 500 ms or a cancel is not a tap).
- Wire (`springboard/proto/src/touch.rs`): `ToApp::Touches(TouchEvent)`, tag `0x1f0`, at most 16 touches, locations in surface
  pixels like `ToApp::Touch`. The shell sends a frame ahead of the matching `ToApp::Touch`.
- Shell side (`springboard/src/shell/touch_frames.rs`, `AppHost::feed_touch`): each app surface has a `TouchTracker` built with the
  app's scale. Touchscreen contacts are direct samples under their finger id; the pointer is one indirect sample. The first
  finger's pointer-stream copy is skipped while a contact is down. A synthesized accessibility tap (`ax press`) makes a frame too.
  Focus loss, window close and a system gesture that takes the finger cancel every touch the app holds, in one frame.
- SDK: `App::wants_touch_events()` opts in, `App::touches(ctx, &TouchEvent)` receives frames. `TouchEvent::to_points(scale)` and
  `in_view(scale, ox, oy)` convert to points / view coordinates. Apps that do not opt in keep `Input::Touch`. Once frames
  arrive the plain stream is dropped for opted-in apps; with a shell that sends only plain touches the SDK derives one indirect
  touch (with phases and tap counts) from them, so a mouse still works. Touches still down on the plain stream are cancelled on
  `Suspend`.
- Parity: touch objects and the touches-began / moved / ended / cancelled model of the reference toolkit. Not yet: coalesced and
  predicted touches, estimated (pencil) properties, azimuth and altitude, hover.

## Several windows per app (`bifrost_app::app_windows`)

An app can hold more than one window. `AppWindows` is the app's registry: the main window (`WindowId::MAIN`, always open,
cannot be closed) and the windows the app opens, front to back, the first one focused.

- `AppWindows::open(WindowSpec::new(title, width, height))` opens a window in front and returns its `WindowId`. Ids count up
  from 1 and are never reused. `DEFAULT_LIMIT` (8) counts the main window too.
- `focus(id)`, `close(id)` (returns the window; the main window gives `WindowError::MainWindow`), `set_frame(id, title,
  width, height)`, `get(id)`, `list()`, `focused()`.
- `route(id)` gives the window an event or a frame belongs to, or `None` once the window is closed. A late event for a closed
  window is dropped, never sent to the main window.
- `App` trait defaults (existing apps compile unchanged): `window_event(ctx, window, Input)` sends the main window to `event`
  and other windows to nothing; `draw_window(ctx, window, surface, t)` sends the main window to `draw` and others to an empty
  damage; `window_closed(ctx, window)` is a no-op. Override them to serve the app's extra windows.

Wire protocol (`springboard/proto`, tags 0x240 to 0x24e; 0x1c0 to 0x1cf belongs to Search): the app sends `WindowOpen {id, title, w, h, position, flags}`, `WindowClose {id}`,
`WindowTitle {id, title}`, `WindowResize {id, w, h}`, and for the pixels `WindowBuffers {id, w, h, generation}` and `WindowFrame {id, buf, damage}`.
The shell sends `WindowClosed {id}` (its own close control), `WindowConfigure {id, w, h}`, `WindowTouch {id, touch, kind, x, y,
t_ms}` and `WindowRelease {id, buf, generation}`; the wheel and hover over an extra window arrive as `WindowInput::Pointer` for
that window (`App::window_pointer`, see "Pointer events"). Titles are cut to 512 bytes; non-finite or negative sizes are refused.

Units (2026-10-23, E2E follow-up). `WindowTouch` and an extra window's pointer events are in the window's **points**, while
the main window's `ToApp::Touch` is in surface pixels. An app that hit-tests an extra window in px, or hands its touches to code
written for the main window, scales them first: `app_windows::window_touch_in_px(ev, ctx.ui.pt(1.0))` returns the touch in px
and leaves other input alone. Contacts card windows (`card_windows::card_touch`), Dictionary and Files extra windows, the
Maps window and the Music MiniPlayer read the points as px (or divided them by the scale again), so clicks missed at
Midget's 1.5625 scale; Mail, Notes and Calendar already scaled. Tests: sdk
`a_window_touch_in_points_becomes_px_at_the_display_scale_and_other_input_is_kept`, contacts
`a_card_window_click_in_window_points_lands_under_the_pointer_at_a_non_1_scale`, music
`a_window_touch_in_points_hits_play_only_once_scaled_to_px`.

Placement, collapse and title double-click (tags 0x246, 0x247, 0x24d, 0x24e). `WindowOpen` carries an optional `position`
(points, relative to the visible desktop area) and a `flags` word (`window_flags::TITLE_DOUBLE_CLICK`). The app sends
`WindowMove {window, x, y}` to place a window and `WindowCollapse {window, collapsed}` to fold it to its title bar. The shell
sends `WindowMoved {window, x, y}` when the person drags a window (the position is in points and clamped to the screen), and
`WindowTitleDoubleClick {window}` on a title bar double-click, only for windows opened with the flag. Other windows still
zoom. The table (`app_window_table.rs`) keeps the position, a pending app move (applied once), the collapsed flag and the
opt-in; `clamp_to_screen` keeps a grab strip of the title bar on screen. A collapsed desk window is its title bar only: its
app keeps its size, and edge grabs are ignored. Tests: proto `window_placement_*`, `app_window_table`
`placement_tests`, `desk` `a_window_collapses_*`, `app_windows` `a_window_moves_collapses_*`.

SDK (`Ctx`): `open_window`, `close_window`, `set_window_title`, `resize_window`, `move_window`, `set_window_collapsed` change the app's
registry and send the message; `WindowSpec::at(x, y)` and `WindowSpec::with_title_double_click()` ask for placement and the opt-in.
The hooks `App::window_moved` and `App::window_title_double_click` default to no-ops;
`windows()` reads it. The run loop routes `WindowClosed`, `WindowConfigure` and `WindowTouch` through
`app_windows::deliver_window`: a closed window gets no touch, a closed window's close is applied once, and the app's
`window_event` and `window_closed` run with no registry borrow held.

Shell (`springboard/src/shell/app_window_table.rs`): `WindowTable` keeps each app's extra windows front to back (open, close, retitle,
resize, focus, close_app, `of_app` for the Window menu and the Dock list), with per-app limit 7 and unique ids per app.

Each extra window has its own shared-memory slot pair (`springboard/sdk/src/runtime.rs`, `alloc_pair`, `draw_windows`): the SDK
sends `WindowBuffers {id, w, h, generation}` with the two descriptors, draws the window with `App::draw_window` into a free slot
(`pick_free`, the same rule as the main window), and sends `WindowFrame {id, buf, damage}`. The window size in pixels is its
point size times the scale (`window_px`, capped at 8192). A window is drawn when a frame is due for the app or when it is dirty
(a new buffer set, an event). `WindowRelease {id, buf, generation}` frees a slot only when the generation matches (a stale
release is ignored). A window the app closes loses its pixels at the next draw.

Order on the wire (MA6): the app's queued requests (`WindowOpen`) are flushed before `draw_windows` sends a window's
`WindowBuffers` and first `WindowFrame` (`flush_out`); before the fix the open request waited behind the pixels, the shell
dropped them for a window it did not know, and the window stayed unmapped (listed, never drawn: Mail's compose window). The
shell also keeps such early buffers and frames (`AppHost::early`, at most 8) and handles them when the window opens, so
apps built with the older SDK show their second windows too. Test: `window_pixel_tests` (`a_window_opened_this_turn_*`),
`window_table_tests` (`buffers_and_a_frame_that_come_before_the_window_open_*`), `extra_window_frame_tests`.

System sheets and alerts on the desktop (MA6): the shell's own frame sits under every window, so a sheet or alert drawn into it
is hidden behind them, leaves its veil on the wallpaper (the frame is cleared only where the bar, the Dock and the menus are)
and darkens it with each frame. On the desktop they are one overlay surface of their own above the windows, chrome id 15
(`Shell::modal_layer`, `desktop_chrome_frames`, whole screen, pointer and exclusive keyboard); it goes away with the sheet and
leaves the desktop as it was. Mail asks for the address book when a message is composed, so the first compose raises the
Contacts question. Test: `permission_sheet` `on_the_desktop_a_sheet_is_an_overlay_*`.

Shell (`springboard/src/shell/apphost.rs`): each `AppHost` holds its app's `WindowTable` (`pub windows`) and maps each extra
window's buffers (`window_buffers`, the main window's size and seal checks). `window_frame(id)` gives the newest frame, and
`window_blitted(id)` gives the replaced buffer back to the app. `close_window_from_shell`, `configure_window` and `touch_window`
send `WindowClosed`, `WindowConfigure` and `WindowTouch`. The table drops a window the app's exit closes. Tests:
`window_table_tests` in `apphost.rs`, `window_pixel_tests` in `runtime.rs`.

Desk (`springboard/src/shell/desk.rs`): an extra window is `AppId::ExternalWindow(slot, id)`, a window of its own in the desk's
`Win` list (its parent app is `AppId::parent()`). `desk_sync_extra_windows` runs from the app host tick and opens or closes
desk windows with `WindowTable::desk_changes`. The window blits its mapped frame (`ext_window_blit`, `AppHost::blit_window`,
which gives the frame back through `window_blitted`). Its traffic light closes it through the app host (`WindowClosed`) and the
app keeps running. A resize sends `WindowConfigure` in points (`ext_window_resize`); a touch on its body sends `WindowTouch` in
points (`ext_window_touch`). The Window menu and the Dock's app menu list the windows (`window_menu_name`, `menu_items`) and
focus one with `desk_focus_extra_window`. Tests: `app_window_table` (`desk_changes`, `menu_items`), `apps` (`AppId` helpers).

Window server: an extra window has its own frame id, `WINDOW_ID_FLAG | slot << 16 | window id` (`win_id`, `frame_app` is the
inverse), so it takes the same `SPRINGBOARD_WAYLAND` path as an app's window. A window floats with its app (`is_floating` checks
the window and its parent; a built-in window never floats, and only a running app's level counts), so the window-level path
(`take_wayland_windows`) keeps working; `sync_levels` sorts the bands with the same test `band_top` raises into, so a raised
window always lands at the top of its band (W4-1). Cmd+` (`desk_cycle_window`) walks the front app's
windows, its extra windows included; Cmd+Tab still lists apps. The Window menu lists only the front app's windows. The screen
reader lists each window of the front app as a window element (`walk::window`). Not done: the parent app's drop targets and the
window's own controls are not in the reader tree; the direct Wayland path has no second toplevel (`LAYER_CONTEXT_WIRED` is false).
Tests of the desk lifecycle itself are not written yet (the table diff is tested).

Contacts (`springboard/apps/contacts-desktop`, 1.13.0): Card > Open in Separate Window and a double-click on a list row open the
contact's card in a window (`card_windows.rs`, `main.rs` `open_card_window`). The window draws the card at its own size
(`Contacts::draw_card`, `draw_window`) and takes touches (`window_event`, `Contacts::touch_card`): the card scrolls on its own
scroller, a tap on a value copies it as the main card does (the main card has no call or compose action), and Edit shows the
contact in the main window and opens its editor (`edit_from_card`). The SDK has no focus call, so the main window is not raised.
Its close unbinds the card (`window_closed`). The row is partial in `docs/parity-matrix/apps/contacts.json`.
Dictionary, Mail, Calendar and Maps stay gap and name this API in their `wire`.

Key window (keys, text, menus, commands). One window of an app has key focus: the desk's front window when it is one of
the app's extra windows, otherwise the main window (`WindowId::MAIN`). Wire (`springboard/proto`): the shell sends
`WindowFocus {window, focused}` (0x24f) when key focus moves (the old window `focused: false`, the new one `true`; the main
window is 0), and `WindowInput {window, input}` (0x250) carrying a `WindowInput`: `Key`, `Command`, `MenuAction`,
`SetMarkedText`, `UnmarkText`, `DeleteSurrounding` or `AccentHold` (kind 7, the held letter as a code point). Window 0 is sent as the messages every app already handles (`Key`,
`Command`, `MenuAction`, `SetMarkedText`, ...), so an app without extra windows is unchanged. Shell
(`springboard/src/shell/apphost.rs`, `desk.rs`): `sync_key_windows` makes each host's `key_window` follow the desk's front
window before input and on each tick; `AppHost::input` tags keys, commands, menu items and composition with it;
`set_key_window` sends the focus pair. The keyboard's composition goes through `textinput::to_input`. SDK
(`springboard/sdk`): `Ctx::key_window()` (falls back to the main window once the key window is closed); `App::window_input(ctx,
window, input)`, whose default (`app_windows::default_window_input`) sends keys and commands to `window_event` as `Input::Key`
and `Input::Command`, sends a text edit (marked text, unmark, delete surrounding, accent hold) to
`App::window_text_input(ctx, window, ev)` with the window's id (default: the main window goes to `text_input`, an extra
window's is dropped; the runtime sends the main window's plain `ToApp` composition through the same hook), and sends a menu
item to the main window's `menu_action` only; an app whose extra windows edit text overrides `window_text_input` (Notes'
note windows do, so an input method's marked text lands in the note being typed in, not in the main editor);
`App::menus_for(ctx, window)`, whose default is `main_menu`, is what the runtime sends as the app's menus. Clipboard calls
are not window-scoped and did not change. Cmd+W (`app_windows::is_close_window_key`) closes the window it is typed in: the
desk's own Close action already does that for an extra window (it sends `WindowClosed`), and an app that handles the key
closes its window with `ctx.close_window`.

Adopting it (Maps and Mail, not done here): override `window_input` for the extra windows, forward the keys and commands
of the key window to that window's own state (as Dictionary's `extra_event` does), and call
`app_windows::default_window_input(self, ctx, window, input)` for the main window and anything else; override `menus_for` when
an extra window has its own enabled or checked items. Shared state (Maps' store, Mail's compose window) should be read back
before a change, as Dictionary's `change_prefs` does, so one window's save does not overwrite another's.

Tests: proto (`key_window_messages_round_trip_through_the_codec`, `window_input_has_the_documented_layout`,
`a_bad_window_focus_or_window_input_is_refused`); SDK (`app_windows` tests drive the messages through their wire bytes: key
routing, focus and the late lost focus, closed windows, menu items and text edits, `menus_for`, Cmd+W); shell
(`apphost` `the_key_window_gets_the_input_and_the_app_hears_each_focus_change`,
`marked_text_and_accent_holds_follow_the_key_window`); SDK `marked_text_and_accent_holds_reach_the_key_window_with_its_id`;
proto `an_accent_hold_goes_to_the_key_window_and_a_bad_code_point_is_refused`. Hardware dead keys (the Icelandic `´` then
`a`) are composed by the shell's keymap (`installer::keymaps::Keyboard::press`) and arrive in the key window as the composed
text `á`. Since 2026-10-23 the pending accent shows as marked text, as the reference shows it (`shell::dead_key`): the
key window gets `SetMarkedText("´")` (underlined by the field), and the next key sends `UnmarkText { commit: false }` before
the composed text. A click, a shortcut or edit key, Esc and Backspace (which only cancel), a layout switch, and the typing
moving to another window, app or field take the mark back the same way; it is never committed. The shell's Search field and
a Wayland client's field (the input method's preedit) show it too; Settings search and the sheets compose without showing it
(their fields are not reachable from the key path yet). A dead key with a letter it has no composition for still types the
letter alone (the reference types the accent and the letter). Tests: keymaps
`a_pending_dead_key_shows_its_mark_until_a_letter_composes_or_it_is_taken`, hwkeys
`a_dead_key_is_taken_by_the_next_key_and_given_back_only_to_typing`, `dead_key` tests (Search, Wayland, no target). Not verified: no build or test run was done
for this change; the desk's `sync_key_windows` has no test of its own. Apps: Contacts 1.14.0 (Cmd+W), Dictionary 0.6.0 (own
keys and Copy per window, Cmd+W, prefs read back before each change), Activity Monitor 0.9.0 (Cmd+W in CPU windows).
`min_shell` is not raised: an older shell sends none of these messages.

Apple parity: the reference toolkit gives each window a scene and a per-window delegate. Ours is one registry per app with the scene
lifecycle left to `scene_session`, and no tabs yet (the Dictionary row asks for tabs too).

## Compression streams and filters (`bifrost_app::compression`)

- Encoder: `springboard/sdk/src/compression/dynamic.rs`. Lazy LZ77 at `Default` and `Best` (one byte of lookahead), greedy at `Fast`, over a 32 KiB window. Each block is dynamic Huffman, or stored when that is smaller. `Encoder` is the incremental form: it emits a block every 65535 bytes of input, and its output does not depend on how the input was split into writes. Still missing: no optimal parsing and no per-block tables shared across blocks.
- Decoder: `springboard/sdk/src/compression/inflate_stream.rs`, a resumable state machine. It returns output as each symbol completes and rolls back a unit that needs more input. The Books engine keeps the whole-buffer `bifrost_render::cg::pdf::inflate`, which is unchanged.
- `Stream::process` returns output before `finalize`. `InputFilter` asks its source for capacity-sized pieces only as far as the reader needs. `OutputFilter` sends output as each write completes it.
- Errors: decode failures are reported by the call that sees them. The encode side fails only with `Unsupported` at init; writes after finalize report `Finished` or `InvalidState`.
- LZ4 streams and filters are incremental (2026-10-19): `lz4::Bv4Encoder` emits a `bv41` or `bv4-` block per 64 KiB of input and its output without flushes equals `lz4::encode_buffer` of the whole input; `lz4::Bv4Decoder` takes input in any split, returns each whole block, keeps 64 KiB of history for matches across blocks, refuses block sizes past 16 MiB, output past the limit, an unknown magic and bytes after the end marker. LZMA has no incremental codec here: its streams and filters buffer the input and run the one-shot codec (`lzma`) when they end.
- Flush (2026-10-19, a BifrOSt addition; the reference stream API has no flush): `Stream::flush` and `OutputFilter::flush` make everything written so far decodable and leave the stream open. DEFLATE and zlib write the pending block and an empty stored block (the zlib sync flush, `00 00 ff ff`); LZ4 closes its partial block. A decompress stream and an LZMA stream return nothing. Matches in later blocks still reach back over the flush.
- Parity: the four `compression` rows (multiple-step streams, input and output filters, DEFLATE encoding, error reporting) stay `partial` until the tests run on Midget. Flush is in (see above). Not yet: dynamic-table reuse across blocks. Brotli and LZFSE stay `Unsupported`: Brotli needs its 122 KB static dictionary (not in the tree and not reproducible from the format text), and an LZFSE codec cannot be checked here without reference vectors, so no half-compatible codec was added.
- Tests: `springboard/sdk/tests/parity_sdk-tests-20.rs` (compression section), plus inline tests in `compression.rs`, `compression/dynamic.rs` and `compression/inflate_stream.rs`. Not yet compiled or run in this round.

## Archive format (`bifrost_app::archive_format`)

The Apple Archive counterpart (2026-10-19). A BifrOSt archive (`BAR1`, `.bar`) holds a directory tree with its attributes. It is not the reference byte format; the ZIP helpers in `bifrost_app::archive` stay for interchange with other tools.

- Layout: 52 header bytes (magic, version, compression 0 none / 1 deflate / 2 lz4 / 3 lzma, entry count, payload length, SHA-256 of the payload), then the payload compressed as one stream. Each entry carries kind (file, folder, link), path, mode, uid, gid, modification time (seconds and nanoseconds), size, extended attributes and its data. The full layout is the module doc of `springboard/sdk/src/archive_format.rs`.
- API: `Entry::{file, directory, symlink}`, `encode(&entries, Compression)`, `decode(bytes, &Limits)`, `peek` (count, compression, payload length without decompressing), `encode_to` / `decode_from` on `archive::ByteStream`, `pack_directory(root, &Limits)` (sorted, links recorded and never followed, attributes and extended attributes kept; unsafe or special items are listed in `Packed::skipped`), `extract(&entries, dest, &ExtractOptions)`, `archive_directory` / `extract_archive` for files, `pack_file` for one file, `encode_text` / `decode_text` / `save_text` / `load_text` for a string, and `encode_threaded` (thread count for the DEFLATE payload). Paths are relative to the packed folder, so expanding does not nest one level deeper.
- Hostile input: `Limits` caps entries (default 1,000,000), payload bytes (1 GiB) and attributes per entry (64). The decoder checks the header, caps decompression at the declared payload length (a bomb stops there), checks the SHA-256, and refuses paths that are absolute or hold `..`, `.`, an empty part, a backslash or a control character, duplicate paths, links whose target is absolute or climbs above the root, an entry count that cannot fit the payload, and any entry that runs past the end. `extract` validates every entry before it writes one, never writes through a link (each parent must be a real folder), never replaces a file, drops set-user-id, set-group-id and sticky, and sets owner only when `preserve_ownership` is on.
- Signed and sealed: `sign` / `verify` put an ML-DSA signature (`signature_pq`) in front of the archive bytes; `encode_sealed` / `decode_sealed` seal under ChaCha20-Poly1305 (`encrypted_archive`); `encode_signed_sealed` / `decode_signed_sealed` do both, signing first so the signature is hidden. Keys and where they live are the caller's.
- Apple parity gaps (kept open): this is not the reference container, its stream protocols (`ArchiveStream`, `ArchiveByteStream` protocols), field keys or its Apple Encrypted Archive profiles (no key derivation or store-backed keys); files are held in memory per entry, so a tree is bounded by `Limits::max_payload_bytes` rather than streamed; extended attributes restore only when the file system has them; no access control lists or file flags. The Files app and the Archive Utility still write ZIP.
- Tests: inline in `archive_format.rs` (round trips for every codec, every-byte damage, every-length truncation, hostile sizes, bomb, unsafe paths and links, extraction not writing through a link or over a file, signing and sealing). Not yet compiled or run in this round; not yet exercised on Midget.

## Bundle info and entitlements (`bifrost_app::bundle_info`, `bifrost_app::entitlements`)

Bundle Resources work of 2026-10-19.

- `InfoDictionary` is the Info.plist counterpart. `load(dir)` reads `Info.plist` (XML, binary or text form) or, without one, the top level of `manifest.toml` renamed to CFBundle keys (`id` to `CFBundleIdentifier`, `version` to `CFBundleShortVersionString` and `CFBundleVersion`, and so on; `MANIFEST_KEYS`). Accessors: identifier, name, short and build version, executable, icon, category, minimum system version, `url_types()` (`CFBundleURLTypes`), `document_types()` (`CFBundleDocumentTypes`: extensions, content types, MIME types, role, rank) and `document_type_for_extension`. `validate()` lists missing or malformed identity keys. Files over 1 MiB are refused; the plist text readers in core now stop at 128 levels of nesting.
- Per-language values: `localized`, `display_name` and `info_strings` read `<lang>.lproj/InfoPlist.strings` most specific language first, then the dictionary.
- `Entitlements` is the general entitlements file: a plist of known flags and group lists (`KNOWN`), refused for unknown keys, wrong types, duplicates and size. `check_for(bundle_id, kind)` applies the first-party and `system-app` rules; `held_names()` gives the names the sandbox policy takes. `sign` / `verify_signed` give a signed form (ML-DSA over the bundle id and canonical bytes), so a file does not verify for another package. Not done: the shell does not read an entitlements file yet (the manifest `entitlements` list is still what `Policy::with_entitlements` reads), and no signing service or trust root exists.
- Container folders: `springboard/src/bare/app_policy.rs` (`prepare_container`) now creates `Documents`, `Library`, `Library/Caches` and `tmp` in an app's data directory at launch, before the ownership change; apps still write the flat directory, and a data directory with a file or link of one of those names skips the step.
- Tests: inline in `bundle_info.rs`, `entitlements.rs` and `core/src/plist.rs` (depth limit). Not yet compiled or run; not yet verified on Midget.

## BifrOSt Account session (`springboard/src/cloud_session.rs`, `bifrost_app::cloud_account::session`)

The signed-in BifrOSt Account of this device is one keychain item, shared through the `is.bifrost.account` access group.

- **Item**: group `is.bifrost.account`, service `bc_session`, account `session`. The value is JSON
  `{"v":1,"token","account","server","expires"}`: the device bearer token, the account id (empty for a migrated file),
  the `https` cloud API address, and the expiry as Unix seconds (`null` when the sign-in answer gave no lifetime; read it
  from the cookie's `Max-Age` with `cloud_account::cookie_expiry`). The token is letters and digits only.
- **Lifecycle** (`cloud_session::{store, load, sign_out}` over a `Vault`): sign-in stores the session, a refresh stores the
  new one over it (one item), sign-out deletes the item. `GroupVault` is the real group store
  (`keychain::Store::open_group`); tests use a fake vault. The server side of sign-out is `cloud_account::sign_out_request`.
- **Migration**: `cloud_session::migrate_file` moves the old plain file (`$XDG_DATA_HOME/bifrost/cloud_session`, a
  `token=` line; `file_path` gives the path) into the group once and deletes it. An item that already exists wins over
  the file. A failed keychain write keeps the file for the next start. The shell runs it at start (`src/bin/shell.rs`).
- **Who may read it**: `shell::services::keychain_groups::groups_of` grants `is.bifrost.account` only to first-party
  system apps (`first_party`: id under `is.olibuijr.` and `kind = "system-app"`), and only with the `keychain` permission.
  Other apps that list the group are refused with `NotPermitted`. Locate (`is.olibuijr.locate.desktop`) is the one reader.
- **App side**: `cloud_account::session(ctx)` asks the group (reply through `service_reply`), and `decode_session` gives a
  `Session` (`token`, `account_id`, `server`, `expires`), `SessionError::SignedOut` when there is no item (or it does not
  decode), or `SessionError::Keychain(e)` for a refusal. The token is never printed; `Session`'s `Debug` redacts it.
- **Settings page** (`springboard/src/shell/account_page.rs`, the top row of Settings, "BifrOSt Account"): signed out, Sign
  In opens an email sheet, then a password sheet. The password goes to `POST /api/cloud/v1/signin` on a worker thread and
  is kept nowhere; the answer's `bc_session` cookie is stored with `store` and its `Max-Age` as the expiry. A refusal
  reopens the password sheet with the cloud's reason in its message line (the sheet's footer is its message line; the
  one-field sheets have no footer). Signed in: name and email (`GET /me`), the account's devices (`GET /devices`), and Sign
  Out, which deletes the item and then calls `POST /signout`; a cloud that does not confirm is reported on the page. A
  session the cloud refuses with 401 is removed, and the page says to sign in again. There is no sign-up: accounts are
  made with an invitation (`/signup` takes an invite).
- **Protocol**: `springboard/src/cloud_account.rs` holds the pure parts (sign-in body, answers, cookie, endpoints); the
  SDK's `bifrost_app::cloud_account` re-exports them and builds the app requests. The shell sends through the deildu HTTP
  client (`deildu::http::Client`, TLS, no cookie jar).
- **Bug reports**: `bug_report::session_request(ctx)` and `session_from_reply` read the session through the keychain
  service reply, so the bug-report client takes its token from the group, not from the migrated file.
- **Gaps**: the sign-in and sign-out requests are not run on Midget or on the host; the Settings page is not yet
  screenshot-checked against the reference. The device list is the account's `/devices`, not the Find My routes.

## Locate: device locator, end-to-end location (`bifrost-locate`, `bifrost_app::cloud_findmy`)

Locate (`springboard/apps/locate-desktop`, id `is.olibuijr.locate.desktop`) is the BifrOSt answer to Find My. It shows
the account's devices, can play a sound on one or locate it, and shares location with a contact's account. The cloud
side is `bifrost-cloud` under `/api/cloud/v1/findmy`.

**Built (round 2):** the window (People, Devices, Items, Me), the device list from the model, the first-run sheet, the
control methods `locate.locate`, `locate.play_sound`, `locate.share`; the SDK client `bifrost_app::cloud_findmy` (device
list, device report, command request, device poll with ack, share-free event feed); the cloud routes for devices, battery,
commands, acks, shares and the event feed (branch `feat/findmy-devices`, see the report for its sha).

**Round 3 (models and routes, not wired):** the SDK client has the key, pending-device, sealed-location and share routes
(`bifrost_app::cloud_findmy`: `put_account_key_request`, `get_account_key_request`, `create_pending_request`,
`pending_list_request`, `approve_pending_request`, `sealed_key_request`, `delete_pending_request`,
`put_self_location_request`, `delete_self_location_request`, `put_share_location_request`, `shared_with_me_request`, each
with its parser; `CloudDevice::sealed_location` reads `location_sealed`, `location_enc`, `location_updated`). The locate
app has three pure models, each tested headless:

- `sealing` (`springboard/apps/locate-desktop/src/sealing.rs`): HPKE seal and open of a report to the account key (info =
  device id), of a share copy (info = share id), and of the account private key to an enrolling device (info = pending
  id). The payload is JSON `lat`, `lon`, `timestamp`, `accuracy`, `battery`. Coordinates are range-checked on read.
- `pairing` (`pairing.rs`): `Enrollment` (Idle, Waiting, Enrolled, Denied) for a device that asks to join, and
  `Approvals`, the owner's pending list behind the confirmation sheet. Syncing the list approves nothing; only
  `confirm` returns a request for sealing and `deny` removes one.
- `reporting` (`reporting.rs`): `Reporter::step` gives Report (at most every 15 minutes, and at once on a locate while
  sharing is on), Clear (once, on the first step with sharing off, which also clears a previous run's data) or Nothing;
  `copy_targets` gives the share ids of live grants only, so a revoked or expired share gets no copy.

**Round 4 (wired, not yet end to end in production):** the cloud worker thread (`bifrost_locate::cloud`, the Link the
window owns; events come back through a pipe that wakes `tick`), registration at start, the command long poll with
acks, the device list with opened positions, pending devices, shared positions and the account feed as notifications.
The account key is read from and written to the keychain (`bifrost_locate::keys`, asynchronous replies in
`App::keychain`). Reports seal a `core_location` fix with the account key; the battery comes from
`power_sources::battery`. The Me page draws the Share My Location switch and the join button; the approval sheet
(Approve and Deny) and the People list are drawn in `surfaces.rs`, with the same rectangles used for hit-testing.
Play sound uses the new shell message `ToShell::PlaySystemSound`: the SDK call `system_sound::play_named` sends it, the
shell queues up to four sounds per frame and the audio worker plays them on the sound-effects role.

**Not built, or blocked:**
- The session: Settings > BifrOSt Account signs in and stores the session in the `is.bifrost.account` group (see "BifrOSt
  Account session"). Locate reads it at start, so the worker runs once the user has signed in there. Without a session
  the cloud is off, and the join button says so.
- Contact copies: no contact share is created by the app, so a report goes only to the owner's own devices.
- The join key pair is held in memory until approval. A restart during a wait begins the join again.
- A sound's ack is sent when the worker takes the command, not when the tone has played.

### Location design (owner decision; models and routes built in round 3, wiring not built)

Location is end to end encrypted. The cloud never sees plaintext coordinates, and the columns it keeps are opaque blobs.

- **Account key.** Each BifrOSt Account has a Find My key pair (X25519, HPKE via `bifrost_ui::hpke::{seal_base, open_base}`
  and `bifrost_ui::x25519`). The private key lives only on the owner's devices, in the BifrOSt keychain. The cloud keeps
  the public key only: `PUT /findmy/keys` (the owner sets it), `GET /findmy/keys/{account}`.
- **First device.** The first device that enables Locate generates the account key pair.
- **Later devices.** A new device makes its own device key pair and posts the public key as a pending device,
  `POST /findmy/devices/pending`. Existing devices list the pending ones (`GET /findmy/devices/pending`) and show "Approve
  this Mac on another of your devices" with the device name in a confirmation sheet. Nothing is approved automatically.
  On approval the approving device seals the account private key to the new device's public key and uploads that blob,
  `POST /findmy/devices/{id}/key`. The new device fetches it, opens it with its device private key, and deletes it from
  the cloud.
- **Device reports.** A device seals `{lat, lon, accuracy, timestamp, battery}` with HPKE base mode to the account public
  key, with the device id as `info`, and uploads one `location_sealed` blob (a new column in the next migration). Owner
  devices open it locally. Reports go out at most every 15 minutes, and also when a locate request arrives.
- **Sharing.** While a share grant is active, the owner's device also seals a separate copy of each report to the
  contact's account public key. Revoking the share stops new copies and deletes the stored ones.
- **Consent and clearing.** Location reporting is off until the person turns it on in the Me tab. The onboarding sheet
  says exactly what is uploaded and that it is encrypted end to end. Turning reporting off, or clearing it, deletes this
  device's cloud rows.
- **Tests required before this ships:** seal and open round trip; the approval state machine; a revoked share stops
  sealing; the cloud handlers store and return opaque blobs only.

Until the session exists, no position leaves this device.
Routes assumed where the contract left the answer shape open: `GET findmy/devices/pending` answers
`{"pending": [{id, name, device_public_key}]}`; `GET findmy/devices/pending/{id}/key` answers `200` with the sealed key,
`202` while waiting, and `404` once the request is denied or deleted; `GET findmy/shared-with-me` answers a bare array.

## Universal Clipboard (Continuity, `springboard::universal_clipboard`, `bifrost_app::cloud_clipboard`)

Settings > Sharing > AirPlay & Handoff has "Share Clipboard Between Your Devices" (`Options::universal_clipboard`, off by
default). When it is on, a local copy of text, a link or a PNG or JPEG image of at most 4 MiB goes to the account's other
devices. The copy is sealed on this device with the account key (HPKE, AAD `bifrost.clipboard.v1`, `info` `clipboard`),
and the cloud keeps only the sealed blob, for two minutes at most. A copy carrying a concealed or transient marker
(`org.nspasteboard.*`, or a password manager's type) is never shared, and neither is a file copy.

- **Shared key code.** The account key pair helpers and the HPKE seal and open live in `springboard/src/cloud_keys.rs`
  (re-exported as `bifrost_app::cloud_keys`). Locate's `sealing` module now calls them with its own AAD; its API did not change.
- **Policy.** `springboard/src/universal_clipboard.rs`: `shareable` (the filter), `encode`/`decode` and `seal`/`open` of the
  payload, `live` (the cloud's expiry) and `offer` (a remote copy is offered on paste when it is at most 120 s old and newer
  than the last local copy).
- **Cloud.** `PUT /api/cloud/v1/clipboard` (`{sealed, enc}`, base64, at most 8 MiB) and
  `GET /api/cloud/v1/clipboard?after=N&wait=S` (long-poll, at most 25 s; `{"item": null}` or the copy with `seq`,
  `device_id`, `sealed`, `enc`, `expires_at`). The store is `clipboard_items`, one row per account (`payload_enc` holds
  `enc.sealed`); see bifrost-cloud `CHANGELOG.md` "Unreleased". The table is excluded from the generated data API.
- **Gap (not built).** The shell's watch of local copies, the upload and long-poll worker, and the paste offer are not
  wired yet. The SDK request builders and the policy are tested; the worker runs on a background thread and needs the
  Locate account key in a keychain group the shell can read (today it sits under `is.olibuijr.locate.desktop`, not
  `is.bifrost.account`). Until that lands the toggle has no effect beyond storing the setting.

## Screen Time (`shell::screentime`, shell only, no SDK API)

Owner decision (privacy first): usage data stays on the device and is never uploaded. Apps get no read access to it.
The feature lives in `springboard/src/shell/screentime/`; the parity rows are `com.apple.FamilyControlsAgent` and
`LockoutUI` (both `partial`), and `screen-time.json` is unchanged.

- Usage (`usage.rs`): one `key=value` file per local day under the shell data directory (`screentime/<day>.txt`):
  foreground seconds per app and per hour, pickups (screen comes on), and notifications per app. Files older than
  28 days are deleted on the day roll. A tick credits at most 15 s, so a stalled loop invents nothing, and the gap
  across midnight is credited to neither day. The shell records usage before and after every input event and tick,
  so the seconds before a switch go to the app that was in front during them (the switch's own time, not the next
  tick).
- Settings (`page.rs`, `chart.rs`, the Screen Time page in `pages.rs`): day and week bars, most used apps, pickups,
  notifications, Downtime, App Limits (cycled 15 min to 4 h), Always Allowed, and a Content & Privacy Restrictions
  row marked "Not Available" (not enforced yet). Changing limits or Downtime needs the Screen Time passcode.
- Policy (`config.rs`): `Config::block` is the one decision. Downtime (may cross midnight, `Schedule`) blocks apps not
  in Always Allowed (Settings always allowed, Clock allowed by default). A daily limit blocks at its minute count.
  `Config::warn_due` says when a limit is five minutes from running out, and the shell posts one notification per app
  per day. Settings is never blocked, so limits can always be changed.
- Time Limit cover (`shield.rs`): OK (go Home), "Ignore Limit for 1 More Minute" (60 s), "Ignore Limit for 15 min",
  and "Ignore Limit For Today". Each grant is stored as `ignore.until.<app>` or `ignore.day.<app>` in
  `screentime.ini`. The passcode prompt comes first when a Screen Time passcode is set.
- Launching (`mod.rs`, `st_launch_check`): opening a blocked app puts the cover up before its zoom starts, and the
  cover stays up while that open animation runs. On the desktop, `desk_open` runs the same check and skips the
  native-route launches for a blocked app, so its own process does not start.
- Desktop (`desk.rs`, `desk_cover`): the cover is drawn inside the window body below its title bar, so the window
  can still be moved, and closed from its lights. Touches outside the body pass through; OK closes the window. Only
  built-in and software-drawn windows are covered; native toplevels of other processes are not yet.
- Passcode: kept in the shell keychain (`springboard/src/keychain.rs`) as item `bifrost.screentime` / `passcode`,
  with the same salted, stretched SHA-256 record and failure count (`shell/passcode.rs`, `Store::load_keychain`).
  The old `screentime_passcode.ini` is migrated once and removed; clearing the passcode removes the item.
- Not built: category charts (the bars are per app), SDK access for apps (out of scope), family controls and
  remote approval, and cover over native toplevels. Not verified on Midget.

Tests: `usage.rs` (midnight gap, app switches, rollover, pruning), `config.rs` (limits, warning window, one-minute,
15-minute and today grants, Downtime over midnight, Settings exempt), `shield.rs` (button order, passcode gate),
`page.rs` (rows, placeholder). Run `cargo test -p springboard screentime` (lead runs them; not run by this round).

## Help books and the help index (`shell::helpbook`, the `helpd` counterpart)

Reference: `com.apple.helpd` (daemons, `a-c.json`), status `partial`. Apple's help viewer reads help books an app
ships in its bundle; BifrOSt has the same shape as a package.

- **Format.** An app ships a `help/` directory and names it in `manifest.toml` (`help = "help"`). `aap.py pack`
  packs that tree and refuses a missing directory or a path that is absolute or contains `..`. Each page is Markdown
  with flat front matter (`title`, `keywords` as a comma list or `[a, b]`) between `---` lines. A heading may end in
  `{#id}`; without it the anchor is the slugified heading. The page title is the front matter's `title`, else the
  first heading. A page larger than 256 KiB, or a page with broken front matter, fails its whole book.
- **Index.** `HelpIndex::rebuild(apps_dir)` reads the built-in books (Settings, from `springboard/help/settings/`,
  `include_str!`) and every `apps/<id>/current/manifest.toml` with a `help` key. Broken books are returned as
  problems and the others still index. `set_book` / `remove_book` replace one app's book on an update or removal.
- **Search.** `search(query, limit)` splits on non-alphanumerics (any script), lowercases, and requires every query
  word to match somewhere on a page; a prefix counts. Weights: title word 30 (prefix 15), keyword 20 (prefix 10),
  heading 8, body word 1 to 4. Ties sort by app id, then page slug. Each hit names the first matching heading as its
  anchor.
- **Links.** `resolve("help://app/...")`: `help://app` is the first page; `help://app/<page>` a page; `<page>#<id>`
  a page at an anchor (an unknown anchor drops to the page); `help://app/<id>` the page declaring that anchor.
- **Menu search.** `search_menu(items, query)` takes the frontmost app's menu items (`MenuItem { menu, title,
  enabled }`) and returns indices, best first: whole-word title 20, title prefix 10, menu name prefix 3, enabled +5.
  Disabled items stay listed, dimmed.
- **Books shipped.** Files (`springboard/apps/files/help`: basics, organize, info and archives), TextEdit
  (`springboard/apps/textedit-desktop/help`: writing, formatting, find and print) and Settings (overview, appearance,
  desktop and dock). Each describes menu items that the app's source defines.
- **Index build (done).** `with_index(root, f)` shares one index. It rebuilds when `apps/installed.json` changes (every
  install, update and removal rewrites it, in the shell or in privd), and `shell.rs` warms it on a worker thread at start.
- **Help viewer (built, host tests not yet run).** `springboard/src/shell/help_viewer.rs`: a floating window in the
  character viewer's overlay pattern. Sidebar: a search field (typing goes to it through `type_out`, ranked `search`
  hits) over the book's table of contents (`toc`: pages, then the open page's headings). Toolbar: Back and Forward over
  a history stack (`Nav`), a book popup over `HelpIndex::books`, and Done. Pages render through `md::classify`, with
  link markup stripped and the first link on a line followed (`help://` inside the viewer, `http(s)` through
  `desktop_open_url`). `Shell::open_help(app_id, anchor)` opens it; the Help menu's "<App> Help" row calls it for the
  settings book or the installed app in front. Gaps: inline bold and italics are not styled, a tap follows only the
  first link of a line, and SDK apps reached through `External(slot)` get no Help row yet.
- **Not built yet (gap; `com.apple.helpd` stays `gap`).** The Help menu's search field, which would call `search_menu`
  for the frontmost app's menus and `search` for topics, is not drawn yet. Help stays off the parity rows until it is.
- **Tests.** `springboard/src/shell/helpbook.rs` (front matter, anchors, ranking, AND matching, link resolution,
  menu search, built-in book, reading a book from disk). Run by the lead: `cargo test -p springboard helpbook`.


## Memory pressure and purgeable storage (bifrostd)

The `dynamic_pager` and `deleted_helper` daemon rows (`docs/parity-matrix/os/daemons/a-c.json`) are `partial`.

- **Swap.** `springboard/bifrostd/src/pager.rs`. At boot `bifrost-zram.service` runs `bifrostd zram-setup`: a zram swap of
  half the RAM (at most 8 GiB, priority 100). While bifrostd runs, PSI (`/proc/pressure/memory`, `avg10`) and
  `/proc/meminfo` decide: under warning or critical pressure with less than 25 % of swap free, one 1 GiB file
  `/var/cache/bifrost/swap/swapN` is added (priority -2). The directory is on the `@cache` subvolume, which no snapshot
  covers and which is not the compressed copy-on-write root: the file is created empty, marked `chattr +C`, then
  preallocated (btrfs refuses swap otherwise), and only when 1 GiB plus a 2 GiB reserve is free. A failed creation backs
  off 10 minutes, doubling; a definite refusal (or four failures in a row) switches disk swap off until bifrostd
  restarts and zram carries on. After 10 minutes of normal pressure the newest file that holds
  no pages is removed. Decisions are pure (`decide`), tested with fixed inputs.
- **Level.** `/run/bifrost/memory-pressure` holds `normal`, `warn` or `critical`. Apps read it with
  `bifrost_app::memory_pressure::published()`, or subscribe with `DispatchSource::memory_pressure`, which uses the
  in-process PSI watcher and needs no daemon.
- **Purge.** `springboard/bifrostd/src/purge.rs`. Below 10 GiB or 5 % free on `/` or `/home`, it deletes, in order: app
  caches under `~/.cache/<app>` (real directories only, never symlinks), the shell thumbnail cache (`/var/lib/springboard/cache`),
  Recently Deleted items older than 30 days (only when `empty_trash_after_30_days = true` in `/etc/bifrost/storage.conf`),
  and pacman packages beyond the newest two per name (mtime order). It stops at 20 GiB free or 10 % of the disk.
  Documents are never listed.
- **Storage page (not done).** The shell's "Free Up Space" writes `/run/bifrost/purge-request`; bifrostd runs the full
  purge and writes the bytes to `/run/bifrost/purge-result` and the Purgeable figure to `/run/bifrost/purgeable`.
  The Settings > General > Storage UI and the policy file writer are still to be built. Open decision: `/run/bifrost` is
  root-owned, so a user-side request needs a writable request directory or a privd call.


## Layered app icons (IconFoundation, IconRendering: `bifrost_render::icon_doc`)

One icon model and renderer for every app icon. Code: `springboard/render/src/icon_doc.rs`. Built-in icons:
`springboard/res/icons/{terminal,camera,magnifier}`.

- **Package.** An app ships `icon.json` beside its layer PNGs. The document has a `background` (`{"solid": "#rrggbb"}`
  or `{"gradient": [top, bottom]}`, `#rrggbbaa` allowed) and up to four `layers`, each with `image` (a relative path,
  no `..`), `x`, `y` (centre as fractions of the body, default 0.5), `scale` (width as a fraction of the body, default 1),
  `blend` (`normal`, `multiply`, `screen`) and `opacity`. Unknown keys are ignored.
- **Legacy.** A flat `icon.png` is a document with one full-body layer on a neutral plate (`PLATE_RGB`), so it is masked
  the same way.
- **Render.** `render(doc, layers, px, variant, tint)` composites the background and layers, masks them to the body (a
  superellipse with exponent 5, `body_coverage`, 4x4 supersampled), draws a specular edge one px inside the body (brighter
  at the top) and a drop shadow (the body moved down 2.5 percent of the canvas, box-blurred, 35 percent black). The body
  is 80 percent of the canvas, centred, so the shadow has room. Output is premultiplied RGBA.
- **Variants.** `Default` draws the layers as authored. `Dark` maps the background to a near-black plate (85 percent
  toward `#1c1c1e`) and lightens the foreground 25 percent toward white. `Tinted` maps the background to a neutral grey
  plate and the foreground's luma onto the tint (white takes the tint, black stays black). `set_style(variant, tint)`
  publishes the choice; a change empties the cache. The style set in Settings > Appearance > Icon & widget style is not
  wired yet (the shell's default is `Default` and the tint `DEFAULT_TINT`, systemBlue).
- **One entry point.** `app_icon(app, path, size_pt, scale, variant)` loads the package (its `icon.json`, or the flat
  image at `path`) and returns the icon at `size_pt x scale` device pixels from a process cache keyed by `IconKey`
  (app, size in tenths of a point, scale in thousandths, variant). The Dock (`chrome/src/dock.rs`, `load_icon`) and the
  home screen and launcher (`apphost::icon_image`) call it. Built-in icons use `render_builtin`, from bytes compiled in.
- **Gaps (IconFoundation and IconRendering rows stay `partial`).** Not done: the Settings row for the style; the
  Launchpad, app switcher and Files drawing through `app_icon` (the switcher draws window thumbnails, not app icons);
  thumbnail service lookup; SVG layers (the render crate has no SVG rasteriser, so layers are PNG); the body and shadow
  proportions are not yet compared with the reference icon grid on Midget. The cache is not invalidated when an app is
  reinstalled with a new icon until the shell restarts.
- **Tests.** Superellipse area at 32 to 256 px (within 0.01 of 0.9502), corner and centre coverage, Porter-Duff and
  blend formulas, variant colour mapping, cache keys and eviction, legacy fallback on the plate and under the mask, the
  shadow below and not above the body, JSON and hex parsing, and the built-in Terminal icon.


## Math typesetting (`bifrost_render::math`, OS layer MathTypesetting)

Source: `springboard/render/src/math/`. Status in the parity matrix: `partial` (`docs/parity-matrix/os/frameworks/m-o.json`).

* `parse::parse_tex(src)` reads a LaTeX subset: `\frac \dfrac \tfrac`, `\sqrt[n]{x}`, `^` and `_`, `\sum \prod \int`
  with `\limits` and `\nolimits`, Greek letters, operators and relations, `\left ... \right` (with `.` for an invisible
  side), `matrix pmatrix bmatrix vmatrix Vmatrix Bmatrix` environments, `\text{}` and `\mathrm{}`, spacing commands
  `\, \; \: \! \quad \qquad`, and function names (`\sin`, `\lim`, ...). `parse::parse_linear(src)` reads `a/b`, `x^2`,
  `sqrt(x)`, parentheses and Greek words (`pi`). Both return `parse::ParseError` with an index.
* `metrics::FontMetrics` gives per-glyph advance, ascent and descent in em, optional stretchy variants, and optional
  `MathConstants`. `MathConstants::from_math_table(bytes, upem)` reads the OpenType `MATH` table. Without it,
  `MathConstants::DEFAULT` (TeX defaults) is used. `metrics::stretch` picks the variant that covers a target height, or
  scales the largest one. `EstimatedMetrics` is a rough font-free fallback.
* `layout::typeset_tex(src, Options, metrics)` and `layout::typeset_linear(...)` return `Layout`: width, ascent, depth,
  glyphs (`ch`, `x`, `y` on the baseline with y down, `size` in px) and rules (fraction bars, radical overlines). The
  output is in pixels, so a caller maps it onto its own text drawing.
* Layout follows TeX: styles D, T, S, SS with cramped variants, the 8x8 inter-atom spacing table (thin, medium and
  thick spaces suppressed in script styles, bin operators demoted where TeX demotes them), fraction shifts and
  clearances, script shifts with the sub/superscript gap, radical clearance, big operators with limits in display style,
  and delimiters stretched to 0.901 of the contents' height and centred on the axis.

Decisions and gaps:

* The math font is Noto Sans Math (SIL OFL), which the `noto-fonts-extra` package in
  `packaging/arch/runtime-packages.txt` installs at `/usr/share/fonts/noto/NotoSansMath-Regular.ttf`. `math::system`
  loads it once, registers it as a face and reads its MATH table (`MathConstants::from_math_table`). Without the file,
  layout uses estimated metrics and the TeX defaults, and the text area paints from the UI font.
* Painting: `math::paint::paint(layout, canvas, origin, face, color)` draws glyphs as one-character text runs (baseline
  placed with the run's pad and ascent) and rules as rectangles. Inline objects in the typesetter
  (`ui::typeset::InlineObject`, `TextPainter::draw_inline`, `Typesetter::with_inline_objects`) reserve width and
  height for attachments and math.
* Notes (the markdown text area, `shell/textarea.rs`): `$x^2$` is typeset inline and takes its width in place of its
  source; a `$$...$$` paragraph is a centred display line. The source stays in the note file. Multi-line `$$` blocks
  are not supported, and an inline formula that wraps across lines is drawn on its first line only.
* Ordinary letters are upright. TeX sets them italic; without an italic correction table the output is upright.
* `\dfrac` and `\tfrac` are plain `\frac`; the style is not forced.
* Linear input takes one atom as a denominator (`a/bc` is `(a/b)c`); write `a/(bc)` for more.
* Not wired yet: the Calculator display (its text is pre-rendered images from an app cache, with no face access). The
  row stays `partial` until the Calculator, and a device check of Notes, are done.
* Tests: parser trees, errors, fake-font fraction and script metrics, spacing between classes, delimiter sizing, matrix
  and radical geometry, and the `MATH` table reader on a synthetic table. Not run in the session that wrote them; run
  `cargo test -p <render crate> math::` before relying on them.


## Scripting (`bifrost_core::script`, `osascript`, `osacompile`, the `scripting` service)

Apps declare what a script can ask them to do in `manifest.toml`, and a small scripting language sends those
requests as typed AppEvents with replies (`bifrost_app::app_events`, over the app control socket).

**Dictionaries.** `[[script.command]]` and `[[script.class]]` tables are parsed by `bifrost_core::script::dictionary`:

```toml
[[script.command]]
name = "make new note"        # lowercase words; a script says this phrase
event = "notes.make"          # sent as the AppEvent ae.notes.make
params = ["title", "body"]    # labels a script may pass; a direct argument goes to the first one
result = "note"

[[script.class]]
name = "note"
properties = ["name", "id", "folder", "modified"]
elements = []
```

A table with a missing name or event is dropped, the rest of the manifest is unaffected, and the other manifest tables
(`[privacy]`, `[[service]]`, `[[intent]]`) are read as before. The shell's `scripting` service answers `list {}` (the
installed apps with a dictionary) and `dictionary {app}` (its `commands` and `classes`).

**Language.** `springboard/core/src/script/` has a lexer, a recursive-descent parser (`parser.rs`, `ast.rs`) and a
tree-walking interpreter (`interp.rs`). Supported: `set`, `get`, `make new X with properties {...}`, `delete`,
`tell app "Name" [to statement | ... end tell]`, `if ... then ... else if ... else ... end if` and the one-line form,
`repeat` (forever, `n times`, `with i from a to b`, `with x in list`, `exit repeat`), `on name(args) ... end`,
`on run argv`, `return`, `try ... on error e ... end try`, `display dialog` and `display notification` (with `with title`),
`do shell script`, `delay`, `&`, comparisons (`=`, `≠`, `is`, `is not`, `<`, `<=`, `>`, `>=`), `and`, `or`, `not`,
`+ - * /`, `mod`, lists `{a, b}`, records `{name:value}`, `item n of list`, `length of` and `count of` for lists and
text, and `prop of object`. Names and keywords are case-insensitive; `--` starts a comment; a `¬` at a line end
continues it.

Commands in a `tell` block resolve through the target app's dictionary: an unknown phrase is error -1708 ("doesn't
understand the message"), an undeclared label is an error before anything is sent, and the parameter record is built by
`dictionary::command_params` (the direct argument goes to the first declared parameter). `get`, `set`, `make` and
`delete` of objects send the core events `getd`, `setd`, `crel` and `delt`, whose `object` is the specifier record
`{class, index | name | all | whose: {property, value}, property?}`. A `make new X` that the dictionary declares as a
command uses that command instead. A class or property the dictionary does not declare is error -1728. An error reply
keeps its number and message, and `on error` sees them as `number` and `message`.

The host (`interp::Host`) owns every side effect, so the same script runs against a fake host in the tests. The tool's
host (`cmd/src/osascript.rs`) finds the app in the installed manifests, sends the event to the app's socket with
`app_events::send_message` (120 s timeout) and runs `/bin/sh -c` for `do shell script`.

**Tools.** `osascript [-s s|o] [-e statement]... [file [argument ...]]` runs `-e` lines, a file or standard input; the
arguments are `argv` for `on run argv`. A file holding an `osacompile` output runs from its canonical source.
`osacompile [-o output] (-e statement... | file)` checks the syntax and writes a JSON document,
`{"format": "bifrost-osa", "version": 1, "source": <canonical source>, "ast": <syntax tree>}`. The canonical source has LF
line ends, no trailing blanks and one final newline. Each AST node is an object with a `node` field naming its kind
(`set`, `tell`, `command`, `repeat`, `specifier`, and so on; `script::ast::program_json` is the reference).

**Apps.** Notes declares `make new note` (answered through the Create Note intent; the title becomes the first line)
and the `note` class, whose `name`, `id`, `folder` and `modified` properties are read from the note list, so
`get name of every note` works. Calendar declares `make new event`, answered through the Create Event intent.

**Gaps (recorded decisions).** The tools do not launch an app that is not running (error -600). `display dialog` and
`display notification` print on standard error, because the shell has no request path for a command-line tool yet.
Descriptors travel as JSON, not Apple event descriptors; there is no sdef file and no generated proxy. Setting
properties and deleting notes or events from a script are refused by the apps for now.

**JavaScript.** `osascript -l JavaScript` runs a script in a Boa context (`shell/js_automation.rs`; module doc lists
the globals). `Application("Name")` properties are dictionary commands, or the plural of a declared class: a collection
whose `()`, `[n]` (zero-based), `.byName(x)` and `.whose({prop: v})` read objects with the core `getd` event,
`obj.prop()`, `obj.prop = v` / `.set(v)` and `obj.delete()` send `getd`, `setd` and `delt`, after the same grant and
dictionary checks as the AppleScript-style language. Elements nested in an element are not reachable (the event record
names one class). `osacompile -l JavaScript` (or a `.js` file) checks the syntax with the engine and writes
`{"format": "bifrost-osa", "version": 1, "language": "JavaScript", "source": ...}` with no tree; `osascript` runs such
a file as JavaScript without `-l` and refuses a conflicting `-l`. The language has no
`repeat while`, no `considering`, and no object references beyond specifiers.

## Notification Center column and Control Centre modules (desktop, E2E round e4)

- Notification Center (`shell/notify/center.rs`, `draw_desk_center`): a column 0.245 of the screen wide (never under 342 pt)
  at the right under the menu bar. Top to bottom: the title ("Notification Center" with a clear button, or the quiet
  "No Notifications" line), the notification cards (the app's icon, name, title, body), a Calendar and, with a saved forecast,
  Weather card, the widget rows, and the Edit Widgets capsule with a round close button. A card draws no icon when its app
  has none (a package's icon comes from `apphost::icon_image`). Calendar and Weather open the PC packages
  (`is.olibuijr.calendar.desktop`, `is.olibuijr.weather.desktop`), the bare tablet id as the fallback. The weekday and month on
  the date card follow the region (`bifrost_ui::region::day_name`, `month_name`; English and Icelandic). The date and the
  temperature use `MAC_DISPLAY_PT` (twice the large title). The cards are left out when they would leave no room for the
  Edit Widgets capsule above the dock. The Weather card reads the summary the Weather service saved in the installed package's
  data folder (`appdata/<package id>`, found by `shell::weather::data_dir_in`: the `.desktop` package first, the bare id as the
  fallback) and, from `glance.txt` beside it, the next hours of that place. With hours the row grows (132 pt) and the card
  widens to show a strip of hour, condition glyph and temperature; hours of another place or older than three hours are not
  shown, and with no saved summary there is no Weather card. The Weather app (1.8.3) and the shell's hourly refresh both write
  the glance.
- Control Centre (`shell/control_center.rs`, desktop): Bluetooth is always a module (it reads Unavailable and takes no pressed
  state without a controller); Display is a brightness slider when a backlight exists, otherwise a capsule that flips the
  appearance between Light and Dark. From Auto a tap picks a fixed Light or Dark; Auto is chosen again in Settings >
  Appearance. Now Playing shows only while something plays.
- Not done: the column computes its layout several times per frame (`center_desk_layout`, `center_info_rects`,
  `center_content_end`); it is cheap but should become one layout value per frame. Not verified on Midget (written without a
  build).

## App controls in Control Center (`shell::app_controls`, `ui::control_widget`)

Apps add buttons and toggles to Control Center by declaring `[[control]]` tables in `manifest.toml`:
`kind = "toggle"` or `"button"`, `title`, an optional `symbol`, and `intent`, the id of one of the app's `[[intent]]`
tables. The shell reads them with the manifest (`apphost::Manifest::controls`), drops any table with an unknown
kind, a blank title or an unsafe intent id, and registers the rest in a `ControlRegistry` under `<app id>.<intent>`.
A button press sends `intent.<id>` with no parameters. A toggle sends `intent.<id>` with the string parameter
`on` set to `"true"` or `"false"`, the new state. Both travel over the app's control socket (`shell::intents`).

Status: the model, parsing, registration and request mapping are in place with host tests
(`shell/app_controls.rs`). The toggle parameter is `ui::control_widget::TOGGLE_PARAM` (`"on"`), read by apps with
`toggle_param` (`bifrost_app::controls::toggle_param`). `AppControls` is the panel state: add, remove, press, with
toggle values kept by the shell. Its keys persist in the `control_center_modules` pref as `app:<identifier>` items
(`join_pref`, `pref_app_keys`). Menu bar items per module persist in `control_center_menu_bar`
(`parse_menu_bar_items`, `set_menu_bar_item`), and `battery_show_percent` gates the battery percentage.
Done on the mobile Control Center sheet (`shell/control_center.rs`): app-control tiles sit in a four-across grid under
the built-in modules (`app_tile_rect`, `app_hit`); a tap uses the control through `AppControls::press` and sends its
request on a worker thread (`intents::exchange`). An Edit Controls button under the tiles opens a list of every module
and every app control the running apps declare (`cc_gallery_*`). Each module row has a panel switch and a Menu Bar
switch; each app row adds or removes its tile. Every change saves through `join_pref` and `MENU_BAR_PREF`, and an app
control named in the saved list comes back once its app runs. The battery item draws its percentage beside the glyph
when `battery_show_percent` is `1` (`battery_bar_label`). Tests: tile hit-testing, list add/remove persistence, menu
bar switch, battery label (`shell/control_center.rs` tests).
Not done: the Settings switch for `battery_show_percent` (no page yet); the menu bar does not yet draw one status
item per module from `control_center_menu_bar` (it needs a `Menu` variant and a dispatch that opens the panel on that
module); the desktop Control Center (`draw_desktop_cc`) does not draw app-control tiles or the Edit Controls button;
the Edit Controls list does not scroll, and it lists only apps that are running. Not verified on device or host: the
commits were written without a build, so the lead's `cargo test` run is the first check. Per-app
energy is not built: `activity-desktop/src/sample.rs` has the `/proc` CPU-time sampler, but it lives in the app and
is not shared with the shell. Apple parity gap: ControlWidgetButton, ControlWidgetToggle and ControlValueProvider
on screen.

## App widgets (WidgetKit and Notification Center: `shell::services::widgets_timeline`, `bifrost_app::widgets_timeline`)

Apps ship widgets through the `widgets_timeline` service. The shell draws them as rows in the Notification Centre
column (the menu bar clock) and lists them in Edit Widgets. Status (2026-10-19): `partial` for both frameworks.

**Kinds and families.** An app registers the kinds it offers with the service call `register` (SDK `Kind`, `register`)
or as `[[widget]]` tables in `manifest.toml`: `id`, `name`, `description`, `families = ["small", "medium", "large"]`
(left out: small and medium), `configurable`. The shell reads the tables for every installed app
(`apphost::Manifest::widgets`, `ui::widget_kind::parse_manifest`) and registers them on its tick; a table with an unsafe
id or only unknown sizes is dropped. A registered kind is in the gallery before its first timeline and keeps the
person's choices (placed, size, options) when registered again; a new registration that drops the size a widget had
moves it to its smallest size. A kind that was only reloaded keeps the two sizes the gallery always offered (Small,
Large). `set_family` refuses a size the kind does not support. Edit Widgets shows one chip per supported size, and the
column draws a row 58 pt (Small), 77 pt (Medium) or 96 pt (Large) high, stacked by `ui::widget_column::flow`.

**Timelines.** `reload` (SDK `reload`, or `reload_rich` for the fields below) sends dated entries and a reload policy;
the shell asks the app for a new timeline when the reload date passes. Each entry may carry:
- `clock: {style: "relative" | "timer", target}` (SDK `Rich::with_clock`): live date text. `{}` in the entry's text marks
  where it goes, else it follows the text, and a blank text shows the clock alone. The shell writes it at draw time
  (`Text::display`), so it keeps counting while the app is away, and redraws the open column when the wording next
  changes (`ui::widget_date::next_change`, `widgets_timeline::next_refresh`).
- `actions: [{id, label, intent, params?, toggle?}]` (SDK `Button`, `Rich::with_button`), at most three. A tap in the
  column runs the app's own `[[intent]]` with the string parameters over the app's control socket on a worker thread
  (`shell::intents::run_with`); the app must be running. A toggle also sends its new state as the parameter `on`
  (`"true"` or `"false"`, read with `controls::control_widget::toggle_param`), shows the new state at once, is drawn
  lighter while it waits and goes back when the intent fails or cannot be reached; a second tap while it waits is
  refused, and a reload brings the app's own state (`ui::widget_action::ToggleSet`). Button and intent ids are checked
  (`ButtonSpec`), so no unsafe id reaches a request line.
- A reload may carry `link` (`bifrost-app://app/path`, SDK `reload_rich`'s last argument): a tap on the row opens the
  app at it.

Each row carries a spoken label (`Row::label`: the title, the text as the value and an "Actions:" hint naming the
buttons, built with `ui::widget_ax`), returned as `label` by the `widgets` call.

**Tests.** `ui` widget_kind, widget_action, widget_column and widget_date; the service's `widgets_timeline::tests`
(registration, sizes, clocks, buttons, toggles, validation); the SDK's `widgets_timeline::tests` (the JSON the calls
send).

**Apple gaps, recorded.** Apple runs a widget in its own extension process and starts it on demand; BifrOSt apps answer
reload events themselves, and a button needs the app to be running (a tap on a button of an app that is not running
fails, and a toggle goes back). There is no declarative widget view: an entry is a title, a value, live date text and
buttons, and the shell draws the row. Accented and vibrant rendering modes (`ui::widget_render`), location and network
grants (`ui::widget_access`), the AT-SPI export of the spoken label, push-driven reloads (no APNs) and the home-screen
grid for third-party kinds are not built. `ui::widget_column::layout` (Block, Controller, DisplayMode) is still not
called by the shell. The earlier note that the Control Center panel does not draw app controls is out of date: the
panel draws one tile per control the user added and has an Edit Controls list (`shell/control_center.rs`).

## Colour management (ColorSync: `bifrost_render::icc`, `icc_lut`, `icc_transform`, `icc_builtin`, `edid`)

Clean-room from the public ICC.1:2022 specification. Code: `render/src/icc.rs` (header, tag table, `desc`/`mluc`
text, matrix-shaper curves `curv`/`para`, `wtpt`, `vcgt`), `render/src/icc_lut.rs` (lookup tables: `mft1`, `mft2`,
`mAB `, `mBA `, stages of curves, CLUT and matrix, Lab and XYZ connection-space encodings), `render/src/icc_transform.rs`
(profile-to-profile conversion, intents, black point compensation, baked 3D tables), `render/src/icc_builtin.rs`
(built-in profiles), `render/src/edid.rs` (display chromaticities). Named working spaces stay in `colorspace.rs`.

| Piece | Apple | What BifrOSt does |
|---|---|---|
| Profile reader | `ColorSyncProfileCreateWithURL`, `ColorSyncProfileCopyTag` | `icc::Profile::parse` checks the header magic and the tag table and refuses a tag that runs past the profile. `Profile::lut` reads the `A2B0`..`B2A2` tags in all three lookup-table types. |
| Lookup tables | `ColorSyncTransformCreate` with `mft`/`mAB` profiles | `icc_lut::Lut` runs the stages in the order the tag type defines (A, CLUT, M, matrix, B for device-to-PCS; the reverse for PCS-to-device). Lab 8-bit and 16-bit (v2 scaling) and v4 encodings are decoded; XYZ PCS uses the 1 + 32767/32768 scale. |
| Matrix-shaper profiles | ColorSync's matrix/TRC path | `MatrixShaper` from `rXYZ`/`gXYZ`/`bXYZ` and `rTRC`/`gTRC`/`bTRC`; the inverse uses the Bradford-adapted colorant inverse and a closed-form or bisected tone curve. |
| Built-in profiles | `kColorSyncSRGBProfile`, `kColorSyncDisplayP3Profile`, `kColorSyncAdobeRGB1998Profile` | `Builtin::{Srgb, DisplayP3, AdobeRgb1998, Rec2020, Gray22}` generate real ICC bytes at run time from the chromaticities in `colorspace` and a 1024-sample tone table; the same parser reads them. No profile files are shipped. |
| Transform | `ColorSyncTransformConvert`, `ColorSyncProfileCreateLink`, intents | `Conversion::new(src, dst, intent, black_point_compensation)` goes device to PCS (D50) to device. Perceptual and saturation use their own tables and fall back to relative colorimetric; a matrix profile has one transform, so every intent uses it. Absolute colorimetric scales by the media white on the way out and back in. Black point compensation is the ICC v4 formula on the PCS black (zero for matrix profiles, so a no-op there). |
| 3D table | ColorSync's cached link | `Lut3d::build` bakes a 33-point grid per (source profile, display, intent) and samples it trilinearly; eight tables are kept. |
| Display profile | `ColorSyncProfileCreateWithDisplayID` | `icc_transform::display_target` builds the display profile from the first connected connector's EDID under `/sys/class/drm` (chromaticities as ten-bit values, gamma from byte 23 or the sRGB curve); with no readable EDID the built-in sRGB profile is used. |
| Image decode | `CGImageSource` with `ColorSync` conversion to the display | `cg::imageio::ImageSource::all` converts every decoded frame with an embedded profile (PNG `iCCP`, JPEG `APP2`, TIFF and WebP `ICCP`) to the display profile once per decode. A picture already in the display profile is not tagged with its source profile again. Preview, Photos and any other app that decodes through `ImageSource` get this. sRGB-tagged pictures on an sRGB display are not converted. |

Decisions and limits:
- The conversion is the matrix or table path only. Image decode and `icc_transform` still clip out-of-gamut colours at the
  end; the colour-object path (`cg::color::ColorConversionInfo` with `RenderingIntent::Perceptual`) compresses them toward
  the grey of equal luminance (2026-10-19, see the w15 section below). A profile's own perceptual table, when it has one,
  is used as is.
- CMYK and other non-RGB device spaces are not converted: the engine feeds three device channels, so a profile with
  four or more does not convert. A grey profile's pixels take their red channel.
- Not done: lookup-table (A2B and B2A) profile authoring (matrix-shaper and grey profile creation and embedding in PNG,
  JPEG and TIFF are built, see below), `mluc` text in other scripts than the first
  record, `vcgt` loading into the hardware (`kms.rs` still applies no ramp), the HDR Headroom Adaptive Gain Curve, the
  PDF `ICCBased` path (`cg/pdf/image.rs` still takes the component count only), and the `ColorSync` SDK call (no app
  calls `icc_transform` yet; the `bifrost_app::colorsync` surface still exposes only the named-space converter).
- Verified on the host only by unit tests (`cargo test -p bifrost-render icc`, `colorspace`, `edid`). Not verified on Midget.

## Core Graphics, Quartz and ColorSync additions (2026-10-19, wave 15 `core-graphics`)

All of it is host-tested only; the rows stay `partial` until a Midget check, and no shell or app calls the new pieces yet.
The render crate's `cg` tree reaches apps as `bifrost_app::kit::cg`.

| Apple | BifrOSt |
|---|---|
| `CGPath.union`, `intersection`, `subtracting`, `symmetricDifference`, `lineIntersection`, `lineSubtracting`, `normalized(using:)` | `cg::path::Path` methods `union`, `intersection`, `subtracting`, `symmetric_difference`, `line_intersection`, `line_subtracting`, `normalized`, each with a `FillRule` (`render/src/cg/boolean.rs`, also `combine` and `clip_lines`). Curves are flattened, vertices snapped to a 1/1024 grid, edges cut at every crossing with exact integer arithmetic, boundary pieces kept by a left and right probe and linked into closed loops. The result is polygons only. |
| `CGContext.setFillPattern`, `setStrokePattern` | `DrawContext::set_fill_pattern`, `set_stroke_pattern` (a `cg::paint::Pattern`: coloured cell from a draw callback or an uncoloured one with a tint). The step is fitted to device pixels under the CTM for the two constant-spacing tilings. |
| `CGContext.drawShading`, `drawLinearGradient`, `drawRadialGradient` | `DrawContext::draw_shading`, `draw_linear_gradient`, `draw_radial_gradient`: paint over the whole clip region in user space, under alpha, blend mode and shadow. `set_fill_shading`, `set_stroke_shading` and `draw_paint` take a shading or any `Paint` as the paint of fills and strokes. |
| `CGColorConversionInfo` with `kCGRenderingIntentPerceptual` | `ColorConversionInfo` with `RenderingIntent::Perceptual` uses `ColorSpace::from_xyz_gamut_mapped`: an out-of-gamut colour moves in linear light toward the grey of equal luminance until it fits (hue and lightness kept). Other intents clip; extended-range spaces keep out-of-range values; saturation is still relative colorimetric. |
| `CGPDFContext` path drawing | `graphics_pdf::PdfPage::fill_path`, `stroke_path` (a `cg::raster::StrokeStyle`: width, caps, joins, miter limit, dash), `fill_stroke_path`, each in its own `q`/`Q`. |
| `QuartzFilter`, `QuartzFilterManager` | `graphics_quartz_filter::QuartzFilter` (3x3 colour matrix, lightness shift, image size limit; `black_and_white`, `blue_tone`, `gray_tone`, `sepia_tone`, `lightness_decrease`, `lightness_increase`, `reduce_file_size`) and `QuartzFilterManager` (`filters`, `find`, `add`, `remove`; built-ins are protected). `PdfWriter::finish_filtered(&filter)` applies it to the colour operators and images of a written file. |
| `PDFSelection`, `PDFDocument.findString` | `graphics_pdf::Document::search` returns a `Selection` per occurrence (page, line, character range, text as written, baseline, size); `Document::selection_text`. |
| `PDFView`, `PDFThumbnailView` layout | `graphics_pdf_view::PdfViewState`: `DisplayMode` (single page, continuous, two-up, two-up continuous, book), `layout` and `visible_slots`, zoom (0.1 to 8) and `scale_to_fit` with `set_auto_scales`, `go_to_page`, `go_to_next_page`, `page_at_scroll`, `scroll_offset_for_page`, `point_to_page` and `page_to_point` (origin bottom left); `thumbnail_size` and `thumbnail_grid`. No pixels: the app draws `Document::render` output in each slot. |
| `ColorSyncMutableProfile`, `ColorSyncProfileCopyData` | `render/src/icc_write.rs` `MutableProfile` (`new`, `rgb_display`, `from_bytes`, tag setters for description, copyright, white point, colorants, gamma, sampled and parametric curves, `vcgt` tables, raw tags, `to_bytes`); `bifrost_app::colorsync::{MutableProfile, encode_with_profile, embedded_profile}` embed a profile in PNG, JPEG and TIFF files. |

Corrections found while reading the code (the parity rows said otherwise): `DrawContext` already draws glyph ids
(`show_glyphs`, `show_glyphs_at_positions`, `show_text_at_point`), `ui::geometry::Vector` already exists, and
`ui/src/print.rs` already paginates text (`text_pages`) and writes a `PrintOperation` to PDF.

Gaps that remain: Boolean results are polygons, not curves; no `DrawContext` that writes PDF pages (the SDK writer has its
own page calls); `Function` has no callback form; PDF selection has no per-glyph rectangles because the extractor keeps
only a line's baseline and size; quartz filters do not rewrite a PDF read from elsewhere and the print sheet offers none;
no lookup-table profile authoring; the PDF writer embeds no ICC profile; display and window services still need a live
check on Midget.

## SDK modules added 2026-10-17 (parity waves 1 to 8)

The modules below were added to `springboard/sdk/src` and `springboard/ui/src` in the parity waves of 2026-10-17. Each
entry says what the module does, its main items, and what is not wired. "No caller found" means a search of
`springboard/src`, `springboard/apps` and `springboard/core` by module name found no use outside the module and `lib.rs`:
the model is host-tested, but no shell or app drives it yet. Every row stays partial until a Midget check.

### Cryptography and credentials

- **CryptoKit (`bifrost_app::crypto_kit`, `springboard/sdk/src/crypto_kit.rs`)**: digests `sha256`, `sha384`, `sha512`;
  `hkdf_sha256` (RFC 5869); `AesGcmKey` (`generate`, `from_bytes`, `seal`, `open`; a sealed box is nonce, ciphertext, tag);
  `KeyAgreementKey` (X25519: `generate`, `public_key`, `agree`); `SigningKey` and `verify_signature` (Ed25519). Random keys
  and nonces come from the shell's entropy pool. ChaCha20-Poly1305 is in `chacha_poly`. Caller: `springboard/src/shell/services/trust_eval.rs`.
  Parity row: `docs/parity-matrix/data/apple-cryptokit.json`.
- **OAuth 2 tokens (`bifrost_app::social_oauth2`)**: `OAuth2Token::from_response` reads a token endpoint's JSON answer
  (RFC 6749 5.1) against the caller's clock; `needs_refresh` uses a 60 second margin (`REFRESH_MARGIN_SECS`);
  `refresh_request` builds the renewal; `sign` sets `Authorization: Bearer`. The network is the caller's. No caller found.
  Parity row: `social.json`.
- **OAuth credential (`bifrost_app::oauth_credential`)**: `OAuthCredential` holds access token, refresh token, expiry and scope
  (the OAuth form of `ACAccountCredential`). `encode` and `decode` store it as one opaque keychain secret (header
  `bifrost-oauth-v1`), so the keychain never sees the fields. `is_expired`, `needs_renewal`, `can_renew`, `renewed`. No caller found.
- **Credential binding (`bifrost_ui::credential_acl`)**: `Binding::new(owner, service, account, presence)`; `grant(by, app)`
  lets another app read the item; `presence` can require a passcode or biometry. Wired (2026-10-17): `springboard::keychain::Store` keeps the encoded binding next to the item under a reserved service name that apps cannot name or list; `bind`, `grant`, `revoke` (owner only), `get_as` (granted apps) and `get_with_presence`; a plain `get` of a presence-bound item answers `Error::AuthFailed` (-25293, wire status NotPermitted).
- **Recovery key (`bifrost_ui::recovery_key`)**: `from_entropy` makes a 16-character key from a 32-character alphabet without
  look-alikes, shown in four groups; `normalise` cleans typed input; `Record::enrol` stores only a salted hash and `matches`
  checks a typed key. No caller found: forgotten-password reset is not wired.

### Kerberos, LDAP and directories

- **kpasswd framing (`bifrost_app::kpasswd`)**: RFC 3244 request and reply frames for the Kerberos password service
  (`encode_request`, `decode_request`, `encode_reply`, `decode_reply`; versions `0x0001` change and `0xff80` set).
  Framing only: the AP-REQ and KRB-PRIV come from the caller. No caller found.
- **Kerberos client (`bifrost_app::krb5_client`)**: runs the MIT tools `kinit` and `klist` (the `krb5` package) with
  `std::process::Command`, never a shell. A password goes to `kinit` on standard input. `klist`, `kinit`, `copy_ccache`
  and `parse_klist`; the cache is the `CacheName` of `gss_krb5`. No caller found.
- **LDAP (`bifrost_app::ldap_directory`)**: `Server::new(uri, base)` and `with_bind(dn, password)`; `search` runs `ldapsearch`
  (the `openldap` package) and parses LDIF into `DirRecord`s; `load` fills a `Session` node for the directory query model.
  The bind password goes through a private file, not the command line. No caller found.
- **Local directory (`bifrost_ui::directory_local`)**: `parse_passwd` and `parse_group` turn `/etc/passwd` and `/etc/group`
  into `users` and `groups` records with POSIX attribute names; `fill_node` loads a node. Caller: `springboard/src/shell/services/directory.rs`.

### Network

- **Multicast group (`bifrost_app::network_group`)**: `ConnectionGroup::new(port)` binds a UDP socket; `join` and `leave`
  change membership (rules from `bifrost_ui::multicast`); `send`, `receive`, `set_time_to_live`, `set_loopback`. IPv4 only.
  No caller found. Parity row: `network.json`.
- **TLS options (`bifrost_app::tls_client`)**: `client_config(TlsOptions)` builds the rustls client configuration; `connect`
  wraps a `TcpStream`. No caller found.
- **Link conditioner (`bifrost_app::link_conditioner`)**: four built-in `PROFILES` (`profile(name)`), `Profile::validate`,
  `netem_options`, `tc_args(iface)` and `clear_args(iface)`. Only egress is shaped. The caller runs `tc`. No caller found.
- **Proxy settings (`bifrost_app::proxy_settings`)**: `ProxySettings::from_env` reads `http_proxy`, `https_proxy`, `no_proxy`
  (and upper-case forms); `proxy_for(scheme, host)` and `bypasses(host)`. No caller found. Parity row: `cfnetwork.json`.
- **Bonjour advertising (`springboard/sdk/src/avahi_publish.rs`)**: `Publication::start(service, port)` runs
  `avahi-publish-service`; dropping the value withdraws the record. `publish_args` is pure. No caller found.
- **Wi-Fi (`bifrost_app::wlan_service`)**: request functions for the `wifi` service (`request_interfaces`, `request_networks`,
  `request_power`, `request_scan`, `request_join`, `request_forget`, `request_host`); `decode_*` read the replies. The shell
  serves them in `springboard/src/shell/services/wifi.rs`. Parity row: `core-wlan.json`.

### Cloud records and accounts

- **Cloud record zone (`bifrost_app::cloud_records`)**: `CloudZone` implements `record_sync::Remote` over the cloud's `records`
  table. A zone opens only with `Consent::given(true)`, and field values travel only sealed through a `Sealer`.
  Not wired: no app has an account key to seal with, so no app can sync yet.
- **Account suggestions (`bifrost_ui::account_suggestions`)**: `for_address` picks the account type and mail servers for an
  address from a built-in provider table or the address's domain. Nothing goes over the network. Caller: `springboard/src/shell/forms.rs`.
- **Account store (`bifrost_ui::account_store`)**: the shared `accounts.ini` reader and writer, used by the Internet Accounts page,
  the access sheet and Settings. The SDK side is described in "Accounts" above.

### Speech, language and search

- **Live recognition (`bifrost_app::speech_whisper`)**: `start`, `poll`, `stop`, `model`, `set_consent`, `download` over the
  `speech_recognizer` service; `poll_from_reply` decodes the transcripts; `WhisperBackend` plugs them into `SFSpeechRecognizer`.
  Audio stays on the machine. Gaps (from the module): no per-word timing or confidence, English only, no file recognition, and
  the element's reply format is checked on the lab machine only. Callers: `springboard/src/shell/services/recognizer.rs` and the keyboard app.
- **On-device language model (`bifrost_app::language_model`)**: re-exports the session, tool, schema and budget models;
  `run_turn(session, provider)` runs one turn. The SDK ships no provider. `on_device_availability()` always returns
  `Unavailable(NoOnDeviceModel)`, because BifrOSt has no on-device model yet. Caller: the assistant app's `connect.rs`.
- **Core ML (`bifrost_app::core_ml`)**: `model_device(policy)` returns CPU only (`MODEL_DEVICES` is `[Cpu]`), and `top_index`
  reads the best label of a tensor. Models come from `bifrost_ui::ml_model`; the work runs on the CPU evaluator. Parity row: `core-ml.json`.
  No caller found.
- **Embeddings (`bifrost_app::embedding`)**: `embed(text)` is a hashed bag-of-words vector of `DIMENSIONS` (256) buckets, unit
  length; `cosine`; `EmbeddingIndex` with `insert`, `remove` and `nearest`. This is not a neural model. No caller found.
- **Search kit (`bifrost_app::search_kit`)**: `SearchIndex` over an app's own text: `add_document`, `remove_document`,
  `find_matches` (type-ahead words, every query word needed), `summary`, `to_text` and `from_text`. In memory. No caller found.
  Parity row: `core-spotlight.json`.
- **Search cards (`bifrost_app::search_card_provider`)**: `CardProviders::set_cards(app, cards)` replaces one app's cards;
  `results(query, limit)` ranks title matches above subtitle matches; `remove_app`. No caller found.
- **Document links (`bifrost_app::document_links`)**: `index_wiki_links` reads `[[Target]]` links into a
  `bifrost_core::link_index::LinkIndex`; `backlinks_of` lists the documents that link to a target. No caller found.
- **Link metadata (`bifrost_app::link_metadata`)**: `checked_url` (http and https only), `resolve`, `parse` (title, Open Graph and
  Twitter card image, icon links) and `LinkFetch::start` with a timeout between 1 and 60 seconds. No caller found.
  Parity row: `link-presentation.json`.
- **Link card view (`bifrost_ui::link_card`)**: `draw` paints the card (icon or initial tile, title, site name). Callers: the news
  app's `news.rs` and `wire.rs`.
- **Tab selection (`bifrost_ui::tab_selection`)**: `TabSelection` holds the ordered tab identifiers and the selected one; removing
  the selected tab selects its neighbour. No caller found.

### Screen, capture and images

- **Screen stream (`bifrost_app::screen_stream`)**: `start` (size, minimum frame interval, filter, optional record path), `stop`,
  `frame_from_event` (`Fresh` or `Repeated` frames). The shell side is `springboard/src/shell/services/mod.rs`, with pacing in
  `bifrost_ui::capture_stream` (`ScreenStream::start` refuses without screen capture access). Parity row: `screencapturekit.json`.
- **Capture session (`bifrost_app::av_capture_session`)**: `CaptureSession::new(w, h)`, `set_input(VideoDevice)`, and `start`
  returns a `StreamPlan` naming the V4L2 node. It opens no device itself. No caller found.
- **Document scan (`bifrost_app::document_scan`)**: `DocumentScan` keeps the pages; `flatten(frame, w, h, page)` warps RGBA pixels
  onto a flat page by nearest-pixel inverse mapping. Finding the page corners is the camera's job and is not done here. No caller found.
- **Pixel transfer (`bifrost_app::pixel_transfer`)**: `PixelTransfer::new(w, h)` converts NV12 to RGBA (BT.709 at 720 lines and up,
  BT.601 below, or a colorimetry set with `with_colorimetry`); `rotated` and `rotated_by` turn pictures. No caller found.
- **Image render (`bifrost_app::image_render`)**: `perspective_correct`, `render_to_destination` and `VideoFilter` (`process`,
  `with_feedback`, `reset`) for per-frame filter chains. No caller found.
- **Screen usage (`bifrost_app::screen_usage`)**: `own_seconds(dir, day, app_key)` reads only the app's own foreground seconds from
  the per-day file under `screentime/`. No other app's data is visible. No caller found. Parity row: `screen-time.json`.
- **Web usage and history (`bifrost_app::web_usage`, `bifrost_app::web_history`)**: `WebUsage` records seconds per host (video and
  picture-in-picture counted apart) and a block list that matches subdomains; `WebHistory` holds visits filtered by bundle, profile
  and time range. History is used by `springboard/apps/browser/src/store.rs`; web usage has no caller found. Parity rows:
  `safari-services.json` (history) and `screen-time.json` (usage).

### Photos, media and maps

- **Photo editing (`bifrost_app::photo_editing`)**: `PHContentEditingSession::begin`, `input_adjustment`, `finish` with a
  `PHContentEditingOutput`, and `revert`. Adjustments are stored as records (`bifrost_ui::photo_adjustment`), so the original is
  never rewritten. Not wired: there is no extension host, so no third-party editor loads.
- **Photo picker (`bifrost_app::photo_picker`)**: `PHPickerConfiguration` (filters, selection limit, `to_params`), `PHPickerResult`,
  `PHPickerViewController::present`, `is_pending`, `decode_reply`, `decode_event` and `check_limit`. Wired (2026-10-17): the `picker`
  call opens the shell's photo picker sheet (`springboard/src/shell/photo_pick_sheet.rs`, model `photo_pick.rs`): a grid of the
  library's thumbnails with the app's filters (All, Photos, Videos tabs when more than one kind is allowed) and selection limit
  (0: none, default 1, which replaces the choice). The call answers `{status: "pending"}` at once; the person's choice arrives as
  the `picker_did_finish` event (`{results: [{id}], cancelled}`, ids in pick order) and the app can then read exactly those
  assets with `image {id}`, with no photos permission and no view of the rest of the library. Gaps: no keyboard control in the
  sheet, Live Photos and movies show a still or the first frame and do not play, and a pick does not survive a shell restart.
- **Media format (`bifrost_app::media_format`)**: `FormatDescription::video(codec, w, h)` and `sound(codec, SoundFormat)`. The codec is
  a four-character code (space-padded when short); sides are at most `MAX_SIDE` (16384). Pure data, no decoder. No caller found.
  Parity row: `core-media.json`.
- **Media library and picker (`bifrost_app::media_library`, `bifrost_app::media_picker_view`)**: `media_library::run(library, query)`
  returns positions and sections over the app's own index; `media_picker_view` shapes the picker rows (`rows`, `footer`, `tap`).
  Caller: `springboard/src/shell/pickers.rs`.
- **CoreMedia time (`bifrost_app::core_media_time`)**: `MediaTime` (a rational value over a timescale, with invalid, indefinite and
  infinite states) and `CMTimeRange`, the type the media pipeline also uses. No caller found. Parity row: `core-media.json`.
- **Picture in Picture (`bifrost_app::picture_in_picture`)**: `Pip::new(aspect)` wraps `bifrost_ui::pip_window::Controller` and gives
  the window rectangle and corner. `set_possible`, `start` and `stop` send to the shell's service, answered in
  `springboard/src/shell/services/mod.rs`. Nothing is drawn here. Parity row: `avkit.json`.
- **Route picker (`bifrost_app::route_picker`)**: `sections` splits local outputs and nearby receivers; `tap` activates a route
  (`bifrost_ui::route_auth` decides). The shell makes the connection. Callers: `springboard/src/shell/control_center.rs`,
  `pickers.rs` and `services/audio_devices.rs`. Parity row: `avrouting.json`.
- **Positional audio (`bifrost_app::positional_audio`)**: `Listener` and `Source`, with OpenAL-style inverse-distance gain clamped to
  the reference and maximum distance, and an equal-power pan. The model returns gains; the audio service applies them. No caller
  found. Parity row: `avfaudio.json`.
- **Maps (`bifrost_app::maps`)**: makes the MapKit models in `bifrost_ui` reachable from the SDK (coordinates, regions, camera,
  controls, user location, places). `Snapshotter::render` draws a tile picture, `fix_from_location` and `feed_user_location` take a
  Core Location fix, and `search_places` runs the geocoder over a `Net` trait. Parity row: `mapkit.json`.

### Now Playing, remote commands and feedback

- **Up Next queue (`bifrost_app::now_playing`: `QueueEntry`, `publish_with_queue`, `queue_pick`)**: `publish_with_queue` sends the
  app's `QueueEntry` list (`title`, optional `artist`, `playable`). When the person picks an entry in Control Center, the shell sends
  a `queue_select` event with the index, and `queue_pick` decodes it. A non-playable entry is listed but cannot be picked. The shell
  side is `springboard/src/shell/services/now_playing.rs`. Parity row: `now-playing.json`.
- **Feedback (`bifrost_app::now_playing_feedback`)**: `feedback_body` publishes the like, dislike and bookmark toggles and an optional
  rating scale; `feedback_from_event` and `rate_from_event` decode the person's choice. The app applies the change to its own state
  before it republishes, so a refused command is not shown as done. The shell reads the `feedback` field of the publish body.
- **Artwork**: `Artwork`, `Artwork::choose`, `aspect_fit` and `request_pixels` are modelled. Not wired: artwork does not reach the shell yet.
- **Remote routing (`bifrost_app::remote_routing`)**: `action_for(command)` maps a shell command to a `RemoteAction` (skips map to none,
  since they use `SkipIntervalCommand`), `media_key(name)` maps hardware media keys, and `route(center, command)` passes the command to
  the app's `CommandCenter`. A switched-off or unhandled action is refused. No caller found.

### Notifications: authorization and presentation (update)

The gap sentence at the end of "User notifications" is out of date. The shell now answers both calls:

- `request_authorization` (`ToShell::NotifyAuthorize`) returns the stored answer, or holds the request and shows the first-time
  alert, then answers with `Event::Authorization`. A provisional request is granted without a prompt
  (`springboard/src/shell/usernotif.rs`, `usernotif_authorize`).
- `set_presentation_delegate` and `will_present` (`NotifyPresentDelegate`, `NotifyPresentReply`) are wired: an app that opted in is
  asked about each notification that arrives while it is in front, and the answer is cut to the grant.

Still missing from that list: `UNNotificationSound`, attachments, `userInfo`, interruption levels and location triggers.

### Runtime, scheduling and diagnostics

- **Run loop (`bifrost_app::run_loop`)**: `RunLoop::schedule_after(delay, work)` and `advance(now)` run due work on the thread that
  owns the publishers (the `Rc`-based observables). It has no thread of its own. No caller found.
- **Operation queue (`bifrost_app::operation_queue`)**: `OperationQueue` runs closures on background threads, at most
  `max_concurrent_operation_count` at a time (0 means one per CPU core), with `set_suspended`, `cancel_all_operations` and
  `wait_until_all_operations_are_finished`. Dependencies and quality-of-service classes are not modelled; use `dispatch` for those.
  No caller found. Parity row: `dispatch.json`.
- **Performance counters (`bifrost_app::perf_counters`)**: `measure(events, program, args)` runs `perf stat` and parses its CSV. A
  counter the machine does not offer is `None`, not zero. No caller found.
- **Session assertions (`bifrost_app::session_assertion`)**: `PowerAssertion::new(hold, reason, who)` builds the
  `systemd-inhibit --what=... --mode=block` arguments; the caller runs the command, and ending that process ends the hold.
  The shell's `springboard/src/bare/sleep.rs` uses the same mechanism.
- **Watchdog (`bifrost_app::watchdog`)**: `interval()` reads `WATCHDOG_USEC` and gives half of it; `ping()` sends `WATCHDOG=1` to
  `NOTIFY_SOCKET`. Callers: `springboard/src/shell/diag.rs`, `springboard/src/bare/crashlog.rs`.
- **Container migration (`bifrost_app::container_migration`)**: a `.container-version` file in the app's data directory and ordered
  `Step`s; `migrate(dir, steps, target)` runs them in order and writes the version after each one, so a failed step resumes next time.
  A container newer than the target is refused. No caller found.
- **System configuration store (`bifrost_app::system_config_store`)**: a keyed `State:/...` and `Setup:/...` table (the
  `SCDynamicStore` shape) with `set`, `get`, `remove`, `keys(prefix)` and a `revision` counter. No caller found.
  Parity row: `system-configuration.json`.
- **Settings panes (`bifrost_app::settings_panes`)**: `[[settings_pane]]` entries in `manifest.toml`. The shell sends
  `ae.settings.openPane` with the pane id, and `install_open_handler` answers it. Callers: `springboard/src/shell/apphost.rs`,
  `pages.rs` and `mod.rs`.
- **Tips and tooltips (`bifrost_app::tips`, `bifrost_app::tooltips`)**: `tips::present_first` picks the first tip whose rules allow it
  and records the display; `tip_popover_frame` places the 260 by 96 point popover with a 9 point gap. `TooltipHost` tracks the
  controls' regions and delays, and `draw` paints the ToolTip material. Not wired: the shell does not pass hover positions to apps,
  so a real mouse shows no tooltip in an app yet; touch moves are the only pointer input the app sees.
- **Translation popover (`bifrost_app::translation_popover`)**: `TranslationPresenter::present` runs the translation through the app's
  `Translator` and places a 280 by 120 point popover; `replacement` returns the text, and the app writes it into its document.
  No caller found. Parity row: `translation.json`.
- **Accessibility audit (`bifrost_app::accessibility_audit`)**: `Audit::new(window).run(&elements)` returns findings for unnamed
  controls, empty or clipped elements, small targets, duplicate names and low text contrast. The model is `bifrost_ui::ax_audit`.
  No caller found.
- **Workflow (`bifrost_app::workflow`)**: `WorkflowRun` runs an ordered list of closure steps that pass output along, with `Variables`,
  one step at a time, and pause, stop and reset. It models the Automator workflow engine. Callers: `springboard/apps/automator-desktop`
  and `springboard/apps/shortcuts-desktop`.
- **Mail compose (`bifrost_app::mail_compose`)**: `compose(ctx, draft, files)` opens a new message in the `mailto:` handler with
  recipients, subject, body and attached grants. Callers: `springboard/src/shell/sharing.rs` and the contacts app.

### Input, geometry and game models

- **GameplayKit (`bifrost_app::gameplay_kit`)**: re-exports the game-logic models (entities and components, state machines, grid
  pathfinding, agents, noise, decision trees, rule systems, strategist, quadtree). Deterministic; random values come from
  `crate::random`, seeded by `testing::seed()`. Parity row: `gameplaykit.json`.
- **Linear algebra (`bifrost_app::linear_algebra`)**: re-exports `bifrost_ui::linalg` (GLKit vector, matrix and quaternion types).
  No GPU upload; that is a separate API. Parity row: `glkit.json`.
- **FFT (`bifrost_app::fft`)**: in-place radix-2 complex FFT and inverse over `Complex` (`fft`, `ifft`); lengths must be a power of
  two. Caller: the DAW app's `springboard/apps/daw/src/conv.rs`. Parity row: `accelerate.json`.
- **HID reports (`bifrost_app::hid_reports`)**: finds `/dev/hidrawN` nodes (`devices_in`), reads one report per call (`read_report`),
  and keeps a bounded `ReportQueue` that counts dropped reports (`take_dropped`). It does not decode reports. No caller found.
  Parity row: `core-hid.json`.
- **Icon images (`bifrost_app::icon_images`)**: `IconImages` maps content type identifiers to bitmap sets (the nearest registered
  ancestor wins) and a folder icon (`FOLDER`). No caller found.
- **Raster 3D (`bifrost_ui::raster3d`)**: a CPU triangle rasteriser with a depth buffer. `draw` lights each triangle once at
  its centroid; `draw_shaded` lights every pixel (Blinn-Phong or metallic-roughness GGX through `shading::shade_pbr`) from
  perspective-correct interpolated normals and texture coordinates, with an optional bilinear `Texture` (repeat or clamp).
  Both clip triangles against the near and eye planes. Missing: GPU rasterisation, mip-mapping, shadows and image-based
  lighting. A stand-in for a GPU rasteriser; the scene widget still uses the flat path.
- **Photo analysis (`bifrost_render::photo_analysis`)**: on-device quality measures for a photo library (mean brightness,
  contrast, shadow and highlight clipping, Laplacian-variance focus, colourfulness, exposure and focus verdicts) and a
  64-bit perceptual difference hash for near-duplicate detection. Classical measures with fixed thresholds, no trained
  model: learned aesthetics and subject recognition are not covered. No caller found yet.

### Contacts and phone numbers

- **Phone numbers (`bifrost_ui::phone_number`, `bifrost_ui::phone_rules`)**: `parse(input, region)` normalises to E.164 and formats
  nationally or internationally. `phone_rules` holds lengths and trunk prefixes for 20 common regions. This is not the full numbering
  plan: a number of an unusual length is refused. Callers: the calendar and contacts apps (`phone_number`).
- **Recipient recents (`bifrost_ui::recipient_recents`)**: the most recent addresses, favourites first and capped; an address is
  compared without case. No caller found.

### Storage, managed objects and previews

- **Managed context (`bifrost_ui::managed_context`)**: `ManagedContext<R>` keeps inserted, edited and deleted objects in a working
  copy; `save` makes the working copy the saved state and `rollback` discards it. Caller: the reminders app's `archive.rs`.
- **Persistent store (`bifrost_ui::persistent_store`)**: one file for a `ManagedContext`, with a `Codec` for each record. Only the saved
  state is written. Callers: the reminders app's `archive.rs` and `store.rs`.
- **Preview view (`bifrost_ui::preview_view`)**: `view_frame` lays out an embedded preview's title row, body and counter. The shell's
  `springboard/src/shell/preview_panel.rs` uses the same helpers, so both show the same words in the same places.
- **Local authentication view (`bifrost_app::view::local_auth_view`)**: `LocalAuthenticationView` shows the biometric state as a disc and
  sends its action when tapped; the state changes only through `send`. No caller found. Parity row: `local-authentication-embedded-ui.json`.

### Private framework settle wave (slice p04, 2026-10-17)

- **Word model and CJK splitter (`bifrost_ui::word_model`)**: `WordModel` counts words and word pairs of text the user types
  and predicts completions and the next word; it saves as text and nothing leaves the device. `Lexicon` with `segment` and
  `split_words` splits Chinese, Japanese and Korean text by the lowest-cost path through a word list. No word list is shipped yet, so
  unknown characters stand alone (kana and Hangul runs stay whole). Thai, Lao, Khmer and Myanmar are not split. No caller yet. Parity
  row: CVNLP (OS frameworks).
- **Text chunks (`bifrost_ui::text_chunks`)**: `chunk_ranges` and `chunks` cut a document into passages of at most N characters on
  sentence boundaries with an optional overlap. No caller yet. Parity row: ChunkingLibrary.
- **Cloth (`bifrost_ui::cloth`)**: `Cloth` is a CPU Verlet cloth sheet with pins, gravity, wind, a ground plane and mesh triangles.
  No GPU path. No caller yet. Parity row: ClothPhysics.


### Private framework wave 12 (2026-10-17)

- **File-manager sync extensions (`bifrost_app::file_manager_sync`)**: `SyncExtension` (observe begin and end, badge request, menus,
  toolbar button), `SyncController` (observed folders, registered badges, per-item badges limited to the observed folders, the host's
  selection) and `SyncHost` (what the Files app asks). Parity row: `FinderSyncPriv`. Wired in wave 14: an extension app declares
  `file_manager_folders = ["~/Cloud"]` in its manifest (`~/` is home; `/`, home itself, relative paths and `..` are refused) and answers
  `ae.filemanager.*` calls with `file_manager_sync::serve` (describe, observe, badges, menu, perform). Since the extension-host
  unification (Files 1.32.0) the declaration also makes the app an extension of the system point `is.bifrost.file-manager-sync`
  (`extension_manifest::FILE_MANAGER_POINT`; `parse` adds the `[[extension]]` itself, `file_manager_folders` stays the folder list),
  and the app runs `bifrost_app::extension_service::serve(&mut SyncService::new(controller, extension))` when it is started in
  extension mode (see "App extension host and file providers" below). `RemoteExtension` is the host side: badges are asked for on a worker thread in batches and cached (asked again after 5 s), menu
  items are asked for with a 200 ms timeout and an app that does not answer is skipped for 30 s, and only paths inside the declared
  folders are ever sent. Files (`apps/files/src/extensions.rs`) discovers the apps at start, tells them which folder is shown, draws
  their badges on icon, list and column items, adds their rows to the item and folder shortcut menus and sends the choice back.
  Launch on demand: Files reaches each extension through the shared `extension_host::Host` (`extensions::host_transport`, the same host
  and reaper thread as file providers), so showing one of the extension's folders starts the app in extension mode and the host stops
  it after 30 s idle; `RemoteExtension::connect_lazy` sends nothing until the first request needs the app, and a failed describe is
  retried after 30 s. The 200 ms menu request is made on the UI thread and never waits for a launch: when the extension is not
  running (`Host::is_ready`) the transport starts it on a thread and answers `ERR_STARTING` (no mute), so the rows appear on the next
  menu. Gaps: no enable screen in Settings (declaring the folders is the enabling; Login Items does not list the kind), the toolbar button
  an extension describes is not drawn, the app list is read at start, the extension runs with the host's rights (no sandbox, as for
  every extension), and badge images are symbol names. Host tests only, not run on Midget.
- **Exchange notes (`bifrost_app::exchange_notes`, wave 14)**: Settings > Internet Accounts offers Notes (Exchange) (`Kind::Exchange`,
  account type `bifrost.exchange`, service `Notes`; the server address must be `https://`, the password goes to Passwords). The Notes
  app (1.23.0) mirrors the Notes folder of the first Exchange account with Notes switched on into a Notes folder named after the account
  (`notes/exchange.rs`): a round runs at start, every five minutes and from Note > Sync Exchange Notes; local edits, new notes and
  deletions go to the server first (a delete moves the item to Deleted Items), then the server's changes come in. A note edited on both
  sides keeps the local text as a second note; the open note is not replaced until it is closed; a deleted account folder never deletes
  on the server. Gaps: plain text only (formatting is flattened), no autodiscover or OAuth, one account, no sync progress or error shown
  in the window (errors go to the app log).
- **Local language model (`bifrost_app::local_model`)**: `LocalModelProvider` is a `ModelProvider` for the local inference engine over
  a loopback port (chat-completions JSON over HTTP). `LocalEndpoint::parse` refuses any non-loopback host, so a conversation never leaves
  the machine. Tool calls come back either as the engine's `tool_calls` field or as the prompted `{"tool_calls": [...]}` JSON.
  `on_device_availability()` still says Unavailable because the system ships no engine; an app asks `LocalModelProvider::availability`.
  Gap: no token streaming. Parity row: `GenerativeModels`.
- **On-demand resources (`bifrost_app::on_demand_resources`)**: `Catalog` of tagged payloads, `ResourceStore` (fetch through a caller
  closure, verify, install, pin, purge under a budget by preservation priority then least recently used) and `ResourceRequest`
  (begin, conditionally begin, end accessing, progress). Gap: no PC App Store fetch yet. Parity row: `FoundationODR`.
- **Graph runtime (`bifrost_ui::graph_run`)**: `run(graph, executable, feeds)` executes a compiled `graph::Graph` on the CPU for every
  operator and element type; `Graph::constant_data` keeps constant payloads. Integer results saturate and an integer divide by zero is an
  error. Gap: no GPU path. Parity rows: `GraphCompute`, `GraphComputeRT`.
- **Mesh compression (`bifrost_ui::mesh_compress`)**: lossy compact format for `PrimitiveMesh` (quantized positions, octahedral normals,
  16-bit texture coordinates, delta-coded indices, FNV-1a checksum). Parity row: `GeometryCompression`.
- **Focus modes in Settings (`shell::notify_page::modes_group`)**: the Focus page lists every mode and switches the one on by hand.
  Gap: a mode's people, apps and schedule are not editable from Settings yet. Parity row: `FocusSettingsUI`.


## DER, X.509, revocation filters, Automix and dictation commands (private frameworks wave 12)

Four small engines that private reference frameworks stand for, built clean-room and tested on the host:

- `bifrost_core::der`: strict ASN.1 DER reader and writer (shortest lengths, minimal integers, bounded nesting, UTC and
  generalized time). `bifrost_core::x509`: parses a certificate (names in RFC 4514 form, validity, basic constraints, key
  usage, subject alternative names, the signed body) and assembles one around a caller-made signature.
  `bifrost_app::trust::Certificate::from_x509` feeds parsed chains to the existing chain rules. Signature checking stays
  with rustls; apple parity gap kept: no OCSP, no certificate policy trees.
- `bifrost_core::crlite`: offline revocation from a Bloom-filter cascade (exact for every certificate the publisher
  listed, one signal for others). No online lookup, so no site is revealed to a responder. The wire format is ours
  (`BFCR` v1); the reference system's published filter format is not read.
- `bifrost_ui::automix`: plans a beat-matched transition between two songs from their `RhythmResult` grids (whole bars,
  bar-aligned, equal-power gains, bass swap, tempo ramp). The audio service has no crossfade command yet, so nothing
  plays it; the plan is the contract for that command.
- `bifrost_ui::dictation_commands`: spoken punctuation, line breaks, case and "scratch that" for dictation, English and
  Icelandic tables, as an edit (backspaces plus insert). The on-screen keyboard types dictation through it;
  `Dictation::with_commands` offers it to controls. Not verified on Midget.


### OS rows settle wave 13 (slice o00, 2026-10-17)

- **Folder actions (`bifrost_app::folder_actions`)**: a client of the `folder-actions` shell service
  (`springboard/src/shell/services/folder_actions_service.rs`) for the Folder Actions Setup app: `list` answers the attachments and the
  intents installed apps offer for a folder (an intent that takes `file`, or none, with every other parameter optional), `attach`,
  `detach` and `detach_folder` edit them. Only `is.olibuijr.folderactions.desktop` may call the service, because an attached action
  starts an app's intent for every file that lands in the folder. The shell saves each change (`Watcher::save`,
  `folder-actions.tsv`). Apple's folder-picking panel has no counterpart in the SDK yet, so the app takes the folder as a typed path.
- **VNC authentication (`bifrost_rfb::des`, `handshake::connect_with`)**: the Screen Sharing viewer answers the RFB DES challenge when a
  password is typed. The scheme is weak by design (8 characters, 56-bit key); it is offered only when a password is given, and the
  password is cleared once connected. TLS and the Tight, ZRLE and Hextile encodings are still missing.
- **Command-line tools**: `ifconfig` lists IPv4 addresses, `ipconfig`, `leaks`, `smbutil`, `tiffutil`, `security` and `automator` are new
  (`springboard/cmd`), `pmset` gains `-g batt` and `lowpowermode`, `mdutil -i` hides a volume from Search, and `textutil` reads and writes
  docx and odt.


### App extension host and file providers (2026-10-17)

An installed app can provide an extension and another can use it. Wired path, first user: file providers in Files.

- **Declare.** A package's `manifest.toml` carries `[[extension_point]]` (a point its app hosts, under the app's own id) and
  `[[extension]] point = "..."` (an extension it provides; `bundle_id` defaults to the app id and must sit under it). The system
  declares `is.bifrost.file-provider` (`bifrost_ui::extension_manifest::FILE_PROVIDER_POINT`). `extension_manifest::scan`
  builds the `extension::Registry` from every installed manifest; Settings > Login Items & Extensions lists a provider
  under FILE PROVIDER and its switch (`login.extensions.off`) hides it from the host. The system also declares
  `is.bifrost.file-manager-sync` (`FILE_MANAGER_POINT`) for file-manager sync extensions; an app with a `file_manager_folders` key is an
  extension of it without writing the table.
- **Host.** `springboard::shell::extension_host::Host` (shell crate, used by the shell process and by Files) launches the package's
  own binary on the first call with `--extension <point>`, `BIFROST_EXTENSION_POINT` and `BIFROST_CONTROL=<socket>`, waits for
  `extension.ping`, calls it with one JSON-RPC request per connection, and `reap` stops it after 30 s with no call in
  flight (`extension.shutdown`, then kill). `bifrost_core::extension_process::ProcessTable` tracks the process and the calls in flight.
- **Extension side.** In `main`, `if bifrost_app::extension_service::requested().is_some() { serve(&mut service) } else { run(app) }`.
  `FileProviderService(provider)` answers `provider.enumerate` and `provider.fetch` for a `Provider` (`bifrost_ui::file_provider_wire`
  has the wire format).
- **Files (sync extensions).** `apps/files/src/extensions.rs` talks to file-manager sync extensions through the same host
  (`provider::shared_host()`, one process-wide host with one reaper); see "Private framework wave 12" above.
- **Files.** Each enabled provider is a row under Locations (`apps/files/src/provider.rs`). Its items live at stand-in paths
  `/.provider/<domain>/<names>`; a listing calls `provider.enumerate`, opening a file calls `provider.fetch` into
  `~/.cache/bifrost/providers/...` (placeholders are downloaded on open) and then opens the local copy.
- **Gaps.** No sandbox for the extension process (the sandbox does not bind the socket directory); no scene presentation
  (`remote_view` and `ExtensionKit` host views); provider folders are read-only in Files and do not refresh on
  `signalEnumerator`; no cloud mark on placeholders; the domain list is read once per Files launch; no provider app ships yet;
  FileProviderUI actions are not hosted. Host tests only, not run on Midget.


### Public framework settle wave (slice o06, wave 13, 2026-10-17)

- **USB host transport (`bifrost_core::usb_host`, `bifrost_app::usb_device::UsbHostDevice`)**: opens a `/dev/bus/usb` node, reads the
  cached descriptors, claims interfaces, selects configuration and alternate setting, and moves control, bulk and interrupt
  transfers; `list_nodes` lists devices from sysfs. Isochronous transfers need an URB queue and are not in it. A sandboxed app
  reaches only the nodes its sandbox binds, and none is bound for apps yet (owner decision open). Parity row: IOUSBHost.
- **PTP cameras (`bifrost_core::ptp`, `bifrost_app::image_capture`)**: the Picture Transfer Protocol (container codec, DeviceInfo and
  ObjectInfo datasets, a session with list, download, thumbnail, delete, capture, battery, clock), a USB transport, and
  `MemoryCamera` for tests. `ImageCapture` fills the `camera_device` model from it. Scanners stay the `scan_job` model (SANE
  transport not wired). Parity row: ImageCaptureCore.
- **Network connection (`bifrost_app::network_stream`, `avahi_browse`)**: `NetworkStream` opens the socket behind a `Connection` (TCP
  with no-delay, keepalive and connect timeout, TLS through `tls_client`, UDP, a local path). `avahi_browse::browse` fills a
  `network_browser::Browser` from the local Avahi daemon. QUIC and custom framers are not built. Parity row: Network.
- **Packet tunnel host (`bifrost_core::tun_device`, `bifrost_app::packet_tunnel`)**: a Linux tun interface that moves IP packets, the `ip`
  commands that address it, and the full-tunnel and split-tunnel route plan (the server and excluded networks stay on the
  uplink). IKEv2 and IPsec negotiation are not in tree. Parity row: NetworkExtension.
- **Audio scene (`bifrost_app::audio_scene`)**: the OpenAL-shaped context (buffers, sources, listener, mixer to stereo) over
  `positional_audio`. Doppler and multichannel buffers are not built. Parity row: OpenAL.
- **Link metadata fetch (`bifrost_app::link_fetch`)**: the request behind `LPMetadataProvider`. Parity row: LinkPresentation.
- **Map snapshots (`bifrost_app::map_snapshotter`)**: tiles from a `TileSource` (`TileDirectory` reads the Maps app's tile cache layout),
  routes and pins composed into an image. Parity row: MapKit.
- **PC/SC (`bifrost_app::pcsc`)**: a client over `libpcsclite` (context, readers, reader states mapped to the slot manager, connect,
  transmit); `pcscd.socket` is now enabled by the installer. Parity row: PCSC.
- **Video analyzer (`bifrost_ui::video_analyzer`)**: measures decoded frames (luma histogram, motion) and answers highlight segments
  and the key frame with no model. Face detection still needs an open model. Parity row: MediaIntelligence.
- **GPU and core re-exports**: `bifrost_app::gpu` is `bifrost_gpu`, and the core models the system frameworks hand to apps (RFCOMM,
  SDP, OBEX, Matter, GeoJSON, code requirements, surface registry, serial lines and others) are re-exported from `bifrost_app`.
- **Left open this wave**: Matter (the secure channel and commissioning transports are a whole engine) and MediaIntelligence (no
  open face model). MatterSupport is n/a (an ecosystem app extension point).

## Core Image and Core Video, wave 15 (2026-10-19): generators, QR, stylize, lazy graph, pixel buffer conversion

Host-tested only (written, not run); rows in `core-image.json` and `core-video.json` stay `partial` until the lead checks them on Midget.
Images are `gfx::Image` (premultiplied, red in the low byte).

| Apple | BifrOSt |
|---|---|
| `CIQRCodeGenerator`, `CIRoundedQRCodeGenerator` | `image_generate::qr_code(text, Correction, scale)`, `qr_code_styled(.., quiet, dark, light, rounded)`; model `render/src/qr_generate.rs` (`QrCode::encode`, `encode_bytes`; versions 1 to 10, numeric, alphanumeric or byte segment, lowest-penalty mask) |
| `CIDetector` (QR code), `CIQRCodeFeature` | `image_detect::qr_message(&Image)`; the decoder moved from the shell's camera module to `render/src/qr_decode.rs` (the camera keeps `shell/camera/qr.rs` as a re-export) |
| `CICheckerboardGenerator`, `CIStripesGenerator`, `CIRadialGradient`, `CIRandomGenerator` | `image_generate::checkerboard`, `stripes`, `radial_gradient`, `random` |
| `CIColorCube`, `CIColorMap`, `CIPalettize`, `CIConvertRGBtoLab` | `image_generate::color_cube` (+ `color_cube_from_text` for `.cube` tables), `color_map`, `palettize`, `rgb_to_lab`, `lab_to_rgb` |
| `CIUnsharpMask`, `CISharpenLuminance`, `CIBloom`, `CIGloom`, `CIPixellate`, `CICrystallize`, `CIPointillize`, `CIComicEffect`, `CISpotLight` | `image_stylize::unsharp_mask`, `sharpen_luminance`, `bloom`, `gloom`, `pixellate`, `crystallize`, `pointillize`, `comic`, `spot_light` |
| screens, `CICMYKHalftone`, tile filters | `image_stylize::halftone(kind, &ScreenParams)`, `cmyk_halftone`, `kaleidoscope`, `op_tile`, `affine_tile`, `triangle_tile`, `parallelogram_tile`, `perspective_tile` |
| transition filters | `image_stylize::transition(from, to, time, Transition::...)` (dissolve, swipe, bars swipe, copy machine, flash, mod, page curl, ripple, accordion fold, disintegrate) |
| `CIConvolution3X3` to `9Vertical`, `CIConvolutionRGB*` | `image_stylize::convolve(img, side, weights, bias, alpha)`, `convolve_line(img, Axis, taps, bias, alpha)`; `alpha` false gives the RGB forms; bias is a fraction of the channel range |
| `CIAreaAverage`, `CIAreaMinMax`, `CIAreaHistogram`, `CIRowAverage`, `CIColumnAverage`, `CIKMeans` | `image_stylize::average_colour`, `area_average`, `area_min_max`, `area_histogram`, `row_averages`, `column_averages`, `kmeans`, `dominant_colours`; model `render/src/colour_stats.rs` |
| `CIImage` graph with several inputs, `CIContext` cache | `lazy_image::{GraphImage, GraphContext}`: `from_image`, `generate`, `map`, `combine`; `render` evaluates each shared node once and keeps results in a byte-limited cache; helpers `graph_filter`, `graph_blend`, `graph_from_lazy` |
| `CVPixelBuffer` from and to `CIImage` | `PixelBuffer::from_image(&Image, fourcc)`, `to_image()` (needs a lock), `write_image`, `from_planes`; conversions in `render/src/pixel_convert.rs` (four 32-bit RGB layouts, RGB565, NV12, NV16 in video range, matrix from the attachments) |
| `CVPixelFormatDescription` | `pixel_buffer::pixel_format_description(fourcc)`, `pixel_format_codes()` (the table is fixed; no registration) |
| `CVDisplayLinkSetOutputHandler`, `CVTime` arithmetic | `display_link::HandlerLink` (handler runs per reported refresh), `Time::rescaled`, `checked_add`, `checked_sub`, `compare` |
| decoded frames with colour metadata | `media::DmaPicture::colour_attachments()` (HD is BT.709, SD is BT.601 / SMPTE 170M, as `convert::matrix_for`), `to_tight_nv12()` |
| dma-buf NV12 window surfaces | `system::dma_buf::NV12`, `Descriptor::validate` takes two linear planes of an even-sized picture |

Remaining gaps: the lazy graph and every filter are CPU only and no shell window draws a graph; QR stops at version 10 and mixed segments, and
there are no barcode generators, face or text detectors; the colour conversions ignore ICC profiles; RAW and HEIF input are not decoded;
decoders do not parse the stream's own colour description (the attachments follow the height convention); the compositor has no selectable
blend operator; Metal and IOSurface have no counterpart.

### Accessibility Keyboard and the Input menu (2026-10-19)

- Settings > Accessibility > Keyboard > Accessibility Keyboard (`a11y_accessibility_keyboard`, off by default) opens the on-screen
  keyboard on purpose: `Shell::accessibility_keyboard_tick` calls `keyboard_open_explicit` / `keyboard_close_explicit` when the
  switch and the keyboard disagree (`shell/accessibility/onscreen_keyboard.rs`). The two stay in step: opening the keyboard from the
  Input menu turns the switch on, and closing it (hide key, menu row) turns it off. A tablet that never uses the switch is untouched.
- Settings > Keyboard > Input Sources > Show Input menu in menu bar (`show_input_menu`, `Options::input_menu`, off by default): the
  menu bar's Input menu shows with several sources, while the on-screen keyboard is open, or with this switch
  (`desktop_bar::input_menu_visible`), so Show Keyboard Viewer and Show On-Screen Keyboard are reachable with one source.
- Voice Control: one recognition runs, in the `speech_recognizer` service's voice step; the shell only consumes its final transcripts
  (`Shell::voice_consume`). `Shell::voice_control_tick` is gone.

## Calendar event time zones with daylight saving (`calendar-desktop` `calendar/zones.rs`, wave 15)

| Ours | Apple | Notes |
|---|---|---|
| `zones::TABLE` (`Zone { id, title, standard, dst }`), `zones::find` | `TimeZone(identifier:)`, the event inspector's time zone popup | Eleven zones: UTC, Iceland, London, Paris, Helsinki, New York, Chicago, Denver, Los Angeles, India, Tokyo. The event editor's Time Zone row steps through them (`extras::ZONES`). |
| `zones::offset_at(zone, date, minute)`, `offset_at_utc`, `unix_of`, `local_in`, `convert` | `TimeZone.secondsFromGMT(for:)`, `isDaylightSavingTime(for:)`, `Calendar.date(from:)` in a zone | The European rule (last Sunday of March to last Sunday of October, 01:00 UTC) and the North American rule (second Sunday of March to first Sunday of November, 02:00 local). A wall time the spring shift skips counts as summer time; a repeated autumn hour counts as its first pass. |
| `alerts::utc_offset_secs` per occurrence | `UNCalendarNotificationTrigger` in an event's zone | A repeating event in a zone with summer time fires at the right moment on both sides of the shift. |
| `zones::vtimezone_body`, `ics::write` | `VTIMEZONE` export | A zone in the table writes STANDARD and DAYLIGHT blocks with yearly RRULEs; reading a VTIMEZONE still takes only its standard offset. |

Gaps: no toolbar view time-zone override yet (`zones::convert` is ready for it); southern-hemisphere zones (summer shift across
the new year) and the tz database's history (rules before 2007 for North America, before 1996 for Europe) are not modelled;
an imported VTIMEZONE that is not in the table keeps the fixed offset its file gives. Code only: not built or run, not verified on Midget.

## Dictionary, Digital Color Meter and Disk Utility (wave 15 `smallapps`, 2026-10-19)

| Ours | Apple | Notes |
|---|---|---|
| Dictionary manifest `[[service]] look_up`, `DictionaryApp::rpc` (`service.look_up`), `model::service_word` | Services menu "Look Up in Dictionary" | The Services menu lists it for a text selection; it opens the app and shows the word in its window (history, scope bar, find). The look-up popover's Open in Dictionary row and the `look-up` intent also land in the window now (`show_from_intent`). |
| Color Meter `color::DisplayProfile`, `profile_paths`, `load_display_profile`; `ColorMeter::convert_pixel`, `space_title` | Digital Color Meter "Display in Native Values" and the display profile | Native reads the pixel through the display's ICC profile (`bifrost_app::colorsync::ProfileToProfile`, sRGB to the profile). The file is `BIFROST_DISPLAY_PROFILE`, else `~/.local/share/color/icc/display.icc`, else `/usr/share/color/icc/display.icc`. Without one, Native shows the delivered codes. |
| Disk Utility `partition.rs` (`specs`, `segments`), `FormKind::Partition`, Disk > Partition (cmd+shift+P), `form::Control::Bar` | Partition sheet with the layout chart | Sizes (512m, 4g, rest), optional names and a format; a bar shows the layout as it is typed. The old 2 to 4 equal parts items open the same sheet prefilled. No drag-resize of the bar and no per-partition format. |
| Disk Utility `FormKind::Typed` | Erase and Partition confirmation | Erasing or partitioning a whole disk asks for the disk name to be typed before the request goes to the privileged daemon. Volume requests keep the confirmation sheet. |

Gaps: Dictionary find does not match a phrase wrapped over two lines, and there are no tabs (the SDK has no tab strip). Color
Meter's Generic RGB is still a 1.8 gamma on sRGB primaries. Disk Utility has no Image from Device (a root copy into a user-chosen
file needs the privileged daemon to receive an open file descriptor, not a path), no Change Password for encrypted images, no Scan
Image for Restore, no restore progress bar, and Get Info is still a sheet. Code only: not built or run, not verified on Midget.

## Calendar: pointer drags, the view time zone, typed dates, colours, invitees, Birthdays and the printed grid (`calendar-desktop`, wave 15 part two)

| Ours | Apple | Notes |
|---|---|---|
| `App::wants_touch_events` + `touches` in `main.rs`, `Calendar::set_pointer`, `drag.rs` | `NSEvent` mouse drag against a touch on a view | A finger holds 250 ms before it drags in the hour grid; a mouse or trackpad (`TouchType::Indirect`) drags once it has moved 4 points. The shell makes the wheel as a press and a burst of moves (Wayland axis: press, move +16 ms, lift +32 ms), and an app cannot tell that from a hand, so a move within 30 ms of its press scrolls. Gap: the shell should mark the touch a wheel notch makes (a distinct touch type) so this heuristic can go. |
| `drag::Kind::AllDay` | dragging an all-day event between days | An all-day event in the band above the hours moves to the day under the pointer and is saved (the whole series for a repeating event). |
| `viewzone.rs`, `Calendar::set_view_zone`, toolbar Time Zone button, View > Time Zone | the event's time zone popup and the toolbar's time zone override | Timed events are shown on the chosen zone's clock (`zones::convert_clock`) in the Day, Week and month views, the agenda and the print; all-day events keep their day; a moved or resized block is saved on the event's own clock. The choice is kept in `calendar-prefs.txt` (`view_zone`). |
| Settings > Day Ends at, Show Days per Week, `Prefs::week_view_start`, `Prefs::off_hour` | Calendar Settings > General: day starts and ends, days per week | The Week view shows 1 to 7 days (a five day week starting on Sunday is Monday to Friday); the hours outside the working day are shaded. |
| `goto::read_date` and the Date row of Go to Date | `NSDatePicker` text field style | `2026-12-24`, `24.12.2026`, `24 Dec`, `December 24 2027`, `tomorrow`, `next Friday`; what is typed moves the steppers. Gap: no month-grid picker. |
| File > Calendar Colour, `colours.rs`, `overlay::PALETTE` | calendar colour in Get Info | Nine named system colours (no red: that is the today marker and the destructive colour); a check mark on the pick; kept in `calendar-extras.txt`. |
| The Add Invitees row, `inviting.rs`, `birthdays::book` and `suggest` | `CNContactPicker` autocomplete in the invitee field, iTIP invitations | Matches come from the address book Contacts keeps (name or address, three at most); the event keeps up to 20 invitees; Save writes a `METHOD:REQUEST` (organiser: the account's address) to `itip-requests/` and opens it in Mail. Gap: no CalDAV scheduling PUT. |
| `birthdays.rs`, the Birthdays row, Settings > Default Birthday Alert | the Birthdays calendar | Read-only all-day items from the cards' birthdays (`details.txt`); alerts at 09:00 planned through the alert service; never written to `calendar.ics`. Gap: the files are read, not a Contacts service query. |
| `printgrid.rs`, `Calendar::print_pages` | `NSPrintOperation` on the calendar view | The month table, the week and day columns on an hourly grid, and the year as twelve small months, drawn as PDF content for a landscape page with Helvetica and Helvetica-Bold (WinAnsi). Gap: one page per range, no paper sheet. |

Code only: not built or run, not verified on Midget. The toolbar gains a Time Zone button and the calendar list a Birthdays row, so the
compliance snapshots of the calendar need a re-bless on Midget.

## Files: inline rename and New Folder (`files`, E2E fixes F1 to F3, 2026-10-19)

| Ours | Apple | Notes |
|---|---|---|
| `bifrost_ui::TextField::select(range)` | `NSTextView.selectedRange` setter / `NSTextField.currentEditor()?.selectedRange` | Selects a byte range and puts the caret at its end. An offset past the end is clamped and one inside a character moves back to its start; a reversed range selects the same text. It is a move, not an edit: no undo step is recorded and a typing group ends. Typing, Backspace and Delete replace or remove the selection (they already did). |
| `files::ops::rename_selection(name, is_dir)`, `Files::begin_rename(row)` | Finder's rename editor selecting the name without its extension | The one start of an inline rename (Return, Rename in the context menu, New Folder). A file selects the part before its last extension (`report.pdf` selects `report`, `a.tar.gz` selects `a.tar`); a folder, a name without an extension, a dot file (`.bashrc`) and a name that ends in a dot select the whole name. The row is scrolled into view (`Files::reveal`, icon and list views). Escape cancels, Return commits. |
| `Files::new_folder`, `FileManagerState::pending`, `Files::apply_pending` | New Folder (Shift+Cmd+N) | The folder is made on the spot; when the listing that follows arrives, the new folder is selected, scrolled into view and its name is open with the whole name selected. A rename that commits leaves the renamed item selected the same way. A pending item that is not in the listing or is hidden is dropped. |
| Return, Cmd+O, Cmd+Down, Cmd+Up in Files (`keys.rs`, `main.rs`) | Return renames, Cmd+O and Cmd+Down open, Cmd+Up is Enclosing Folder | Return in the Search field still opens the hit. `HwKeys::chord_mods` (the shell's key path to apps) now names Up and Down as chord keys beside Backspace, because the shell dropped Cmd with an arrow; Ctrl+Cmd and Ctrl+Alt arrows stay shortcuts of the shell. |

Gaps: the rename field does not grow with the text (the field scrolls to keep the caret in view); there is no Shift-arrow selection
in the field yet; the SDK's direct Wayland key path (`wl::input::translate_key`) still delivers an arrow with Cmd held as the plain
arrow, so the arrow chords reach an app only through the shell's key path. Code only: not built or run, not verified on Midget.

## Dock: icon cache, minimised windows and the plate (E2E fixes D3, D1, D2, 2026-10-20)

| Ours | Apple | Notes |
|---|---|---|
| `Desk::with_scaled_icon`, `DockIconKey { owner, generation, size }` (`shell/desk.rs`) | The Dock keeps one image per app | The scaled Dock icons were cached by the address of the source pixel buffer. A rebuilt icon (Dark switch, an update) freed the old buffer and the next one reused its address, so a slot drew another app's picture (two Settings, no Camera) until a click cleared the cache. The key is now the app (`apps()` entry or Linux app id), the icon generation (`Shell::icon_rebuilds`) and the size; `rebuild_app_icons` also clears the cache. Drawing and hit-testing already read the same `desk_dock` list. |
| `thumbnail_frame`, minimised slots in `draw_desk_dock`, `Shell::dock_shot`, `dock_picture` | A minimised window is a small thumbnail of the window, right of the separator | A minimised window's slot is a frame at the window's aspect (clamped 1:2 to 2:1) with the window's own picture in it and a small badge of the app. The picture is one window capture asked for when the window is minimized (`desk_minimize_window`, `desk_show_desktop`; at most 256 px a side), while the window server still has it on screen. Without a picture (no capture global, a failure) the frame holds the app's icon as before. One tile per app stays on the left. |
| `Shell::note_dock_glass`, `dock_glass_vacated` (`shell/desk.rs`, `shell/mod.rs`) | The Dock plate always fits its icons | The presenter keeps a glass shape until redrawn pixels wholly cover it, and the frame damage is exact changed rows, so a Dock that lost an icon (or hid) left its old, wider glass shape as a pale slab at the ends (later tinted by the wallpaper). The frame now also damages the capsule it vacated. |

Gaps (not matched yet): the thumbnail is taken once, when the window is minimized, and is not refreshed while it stays minimized. Minimized windows of other Wayland clients (direct and Linux apps) have no slot of their own yet, only their app's tile: the shell does not learn that the window server minimized one. Every slot keeps the same square size, so a thumbnail is letterboxed inside it instead of the slot taking the
window's width. Code only: not built or run, not verified on Midget.

## Mission Control as an overlay (E2E round 3, MC1)

Mission Control (`shell/overview.rs`, "Overview" in code) is an overlay-layer chrome surface (id 9) above every window on the desktop, on the rectangle between the menu bar and the Dock, with exclusive keyboard focus while it is open; the surface list and the reasons are in `docs/gpu-first.md` ("Mission Control is an overlay surface").

- Layout: the windows of the current desktop are tiled by `grid_layout` without overlap, never scaled up, in the area under the desktops strip and above the Dock. Every size comes from `ui.pt()` tokens and the output size, so any output size, aspect ratio or scale works (test: `tiles_fit_any_output_without_overlap`).
- Input: click a tile to raise and focus that window and close Mission Control; click empty space, press Esc, F3 or Ctrl+Up to close it; Return opens the selected tile (Tab and the arrows select); hover highlights a tile. F4 toggles the Apps grid and the wheel (or a two-finger swipe) pages it; Mission Control swallows the wheel.
- Reference parity: the reference also shows full-screen apps as desktops and groups the windows of one app; ours lists desktops (tags on windows) and one tile per window. Windows of apps that are separate Wayland clients (the Browser, Linux apps) get a tile as well, with their own pixels from a window capture (`bifrost_window_capture_v1`, `shell/window_shots.rs`, `docs/window-server.md` "Window capture"), refreshed at most 5 times a second while Mission Control is open; they stay on every desktop and cannot be dragged to another. Not matched yet: the tile is not grouped with the other windows of its app, and the shell does not know such a window's size, so the tile's shape is the aspect of its first picture (16:10 until it arrives).

## Window pictures (`shell/window_shots.rs`, `bifrost_window_capture_v1`, 2026-10-20)

The shell sees the pixels of the windows it draws itself; the window server composes every other client's. `WindowShots` holds what the shell asked the window server for and got: the newest picture of each window (`Source::Foreign(id)` for a window of another client by its foreign-toplevel id, `Source::Surface(id)` for the shell's own window by its presenter id), the requests in flight, and when each window was last asked about, so a consumer's cadence is a number it passes in. The Wayland backend sends the queued `Request`s (`Display::capture_windows`, one shm buffer per request, as large as the largest picture wanted) and hands back `Outcome`s.

| Consumer | When it asks | Fallback |
|---|---|---|
| Mission Control tiles of other clients' windows (`overview.rs`) | on open, then at most every 200 ms per tile while open; quotes the held picture's serial so an idle window answers `unchanged` | a plate with the window's title |
| Dock slot of a minimized window (`desk.rs`) | once, in `desk_minimize_window` and `desk_show_desktop`, `request_now` | the app's icon in the thumbnail frame |
| Screenshot, window mode (`capture_flow.rs`, `WindowGrab`) | once, at the window's full size; the window alone on a transparent ground with its shadow (`capture::window_picture`) | the cut from the composed screen (after a failure, or 2 s without an answer) |

A window that failed is not asked about again for 5 s (`RETRY_AFTER_FAIL_MS`) except by a consumer that asks now; the pictures of windows that closed are freed every 2 s; a request nobody answers is given up after 4 s. The global is listed on the window server's privileged socket only, which the shell uses (`docs/window-server.md`, "Privileged socket and capture globals"): without it `set_window_capture(false)` and nothing is asked. Gaps: the screenshot's window mode picks windows by the rectangles the desk holds, so a window of another client (which the shell holds no rectangle for) cannot be picked in it yet (the window server reports frames only for a client's own toplevels); the Vulkan scanout answers `failed(unsupported)`. Tests: `window_shots::tests`, `overview::tests`, `desk::tests` (Dock), `capture_flow::tests`, `wm/tests/window_capture.rs`.

## OS daemon slices: container migration, audio unit manifests, model compile, voice profile, family controls, local assistant (parity/os-g1)

- Container migration (`sdk/src/container_migration.rs`): `migrate_safely` snapshots the container to `<name>.premigrate`, restores it when a step fails and removes it on success; `migrate_all` migrates every app directory under a containers root and reports each result. Not built: the updater calling it after an OS install.
- Audio unit declarations (`ui/src/audio_extension.rs`): `[[audio_unit]]` tables in an app manifest (name, version, kind, sub_kind, manufacturer, inputs, outputs) parse into checked manifests (`parse_manifest`, `ExtensionRegistry::register_declared`); the shell's `apphost::Manifest.audio_units` carries them. Not built: the plug-in loader on the out-of-process host and the Settings page over `AudioUnitSettings`.
- Model compile (`ui/src/ml_compile.rs`, `sdk/src/core_ml.rs`): a model's text form compiles to a checksummed binary (`.bmlc`), `install_model` and `load_installed` store and load it. Not built: model download.
- Voice profile (`ui/src/voice_profile.rs`, `shell/services/voice_banking.rs`): pitch, pace and loudness are measured from a recording and merged per bank; `VoiceProfile::apply` sets the rule-based synthesiser's pitch and rate; the service keeps the per-phrase profiles (`a11y_voice_profiles`) and answers `voice()`. Not built: a trained voice (timbre).
- Family controls (`sdk/src/family_controls.rs`): app authorization (request, owner decision, revoke) and the blocked-app lists an approved app sets. Not built: the shell reading `is_blocked` next to `Config::block`, the prompt, and remote family management through an account (no family account service).
- Local assistant (`assistant-client/src/local.rs`): with the backend Off, a request is matched against the allowed intents by title keywords and fills the first parameter from a quoted phrase or the rest of the sentence; the confirmation gate still applies. Not built: language understanding beyond keywords.

## OS gap slice g4 (private frameworks: suggestions, embeddings, versions, generative layers)

Rows advanced to `partial`; none is `parity` until the lab verify run. Apple's behaviour not matched is named per item.

- **GenerativeModels: streamed local replies (`sdk/src/local_model_stream.rs`)**: `LocalModelProvider` implements
  `StreamingModelProvider`: it asks the loopback engine for `"stream": true`, decodes chunked transfer and server-sent
  events as bytes arrive, and passes answer text to `run_turn_streamed`. Tool calls (native `tool_calls` deltas or the
  prompted JSON shape) are never shown as text. An engine that ignores `stream` still works (one piece). Stopping closes
  the connection. Not matched: no on-device engine ships, so `on_device_availability()` stays `Unavailable`.
- **EmbeddingCore: semantic score for document search (`ui/src/semantic_hash.rs`)**: `HashedSemantic` (words plus
  character trigrams, 512 dimensions) is the `SemanticHook` of the Spotlight Documents source (`quicksearch::indexed_groups`).
  Not matched: it is not a learned model (no synonyms), and app and settings results still use `NoSemantic`.
- **GenerationalStorage: Versions browser model (`sdk/src/versions_browser.rs`)**: date labels, line-change summary, preview,
  selection and Restore over the revision store; `DocumentController::open_versions` and `restore_version`. Not matched: the
  shell has no Versions window yet; versions cannot be named or locked.
- **GenerativeFunctions: tool choice and output cap (`ui/src/model_tool.rs`)**: `ToolChoice` (`Auto`, `Disabled`, `Only`)
  limits what a request offers and what may run; `ToolSet::set_max_output` cuts long tool output and says how much was omitted.
- **GenerativeFunctionsFoundation: partial output and JSON Schema (`ui/src/generation_partial.rs`)**: `partial_parse` reads
  the JSON a model has written so far, `validate_partial` checks it without counting what has not arrived, `json_schema`
  exports a schema for engines that constrain decoding. Not matched: the local provider does not send the schema as a
  response format yet.
- **CoreSuggestionsUI: durable dismissals (`ui/src/found_in.rs`)**: `Dismissed::to_text` and `from_text` (bounded to 500, oldest
  first), `forget_app`, and `Pager::for_message` (extract, drop dismissed, page) for any message view. Not matched: the
  Messages app does not call it yet, and Search has no suggestion section.
