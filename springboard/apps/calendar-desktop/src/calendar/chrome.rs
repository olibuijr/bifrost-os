//! The Golden Gate window (Golden Gate Calendar): a full-size window with an optional calendar list under the traffic
//! lights, the unified toolbar (New Event, the Day/Week/Month/Year segmented group, week numbers, export), and
//! the header under it with the large month title and the back / Today / forward capsules. The content draws in
//! its own column (`x0` px from the window's left edge).

use super::*;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CONTENT_BG, LABEL, SEPARATOR_C};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets::{self, CAPSULE_PT, TOOLBAR_GAP_PT};

/// The calendar list's width and the window width below which it folds away (points).
const SIDE_PT: f32 = 200.0;
const SIDE_FOLD_PT: f32 = 640.0;
const SIDE_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
/// The header row under the band: the title's centre line and the navigation capsules' width (points).
pub(super) const HEADER_MID_PT: f32 = 115.0;
const NAV_W_PT: f32 = 170.0;
/// The search field's width at the right end of the band, and the band width below which it is left out (points).
const SEARCH_W_PT: f32 = 170.0;
const SEARCH_ROOM_PT: f32 = 560.0;
/// The one local calendar's colour (its sidebar dot and its events).
/// The calendar's colour: the event dots and blocks (the accent, as the list glyph).
pub(super) fn cal_colour() -> u32 {
    accent()
}

pub(super) const B_SIDEBAR: u32 = 1;
pub(super) const B_ADD: u32 = 2;
pub(super) const B_DAY: u32 = 3;
pub(super) const B_WEEK: u32 = 4;
pub(super) const B_MONTH: u32 = 5;
pub(super) const B_YEAR: u32 = 6;
pub(super) const B_WEEKNO: u32 = 7;
pub(super) const B_EXPORT: u32 = 8;
pub(super) const B_PREV: u32 = 9;
pub(super) const B_TODAY: u32 = 10;
pub(super) const B_NEXT: u32 = 11;
pub(super) const B_CANCEL: u32 = 12;
const B_TITLE: u32 = 13;
/// Accessibility ids of the cells, the events in them, the editor's rows and the Undo bar (clear of the toolbar's 1..14).
const DAY_NODE: u64 = 100;
const UNDO_NODE: u64 = 300;
const EVENT_NODE: u64 = 1 << 32;
pub(super) const B_SAVE: u32 = 14;
pub(super) const B_GO: u32 = 15;
pub(super) const B_QUICK: u32 = 16;
/// The line sheet's Done button (`linesheet.rs`), the colour sheet's Done button (`colours.rs`) and the Time Zone button
/// (`viewzone.rs`).
pub(super) const B_LINE: u32 = 17;
pub(super) const B_COLOUR: u32 = 18;
pub(super) const B_ZONE: u32 = 19;

/// The width the kit's `toolbar` gives each button (glyph buttons are a capsule, titled ones their label).
fn item_w(ui: &Ui) -> impl Fn(&ToolbarItem) -> f32 + '_ {
    move |it| {
        if it.symbol.is_some() {
            CAPSULE_PT
        } else {
            (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(CAPSULE_PT)
        }
    }
}

fn band(x: f32, w: f32) -> Rect {
    Rect { x, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: w.max(0.0), h: CAPSULE_PT }
}

impl Calendar {
    /// Splits the window into the calendar list and the content column (`x0`, `w` in px).
    pub(super) fn layout_columns(&mut self, ui: &Ui) {
        self.px_per_pt = ui.s;
        let full = self.full_w as f32;
        let side = if self.side_hidden || full / ui.pt(1.0) < SIDE_FOLD_PT { 0.0 } else { ui.pt(SIDE_PT).round() };
        self.x0 = side;
        self.w = (full - side) as i32;
    }

    fn k(ui: &Ui) -> f32 {
        ui.pt(1.0)
    }

    /// The toolbars: over the calendar list, the unified band over the content, and the header's navigation.
    pub(super) fn bars(&self, ui: &Ui) -> Vec<(Toolbar, Rect)> {
        let k = Self::k(ui);
        let (x0, w) = (self.x0 / k, self.w as f32 / k);
        let lead = traffic_lights_trailing() + TOOLBAR_GAP_PT;
        let mut bars = vec![];
        if x0 > 0.0 {
            bars.push((Toolbar::new(vec![ToolbarItem::flexible_space(), ToolbarItem::icon(B_SIDEBAR, "Hide Calendar List", "sidebar.left")]), band(lead, x0 - lead - 8.0)));
        }
        let start = if x0 > 0.0 { x0 + 12.0 } else { lead };
        let end = x0 + w - 12.0;
        if let Some(draft) = &self.editor {
            let title = if draft.existing { "Edit Event" } else { "New Event" };
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_SAVE, "Save"),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        if self.quick_line.is_some() {
            let title = "Quick Event";
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_QUICK, "Add"),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        if let Some(sheet) = &self.line_sheet {
            let title = sheet.purpose.title();
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_LINE, "Done"),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        if self.colour_sheet.is_some() {
            let title = "Calendar Colour";
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_COLOUR, "Done"),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        if self.goto.is_some() {
            let title = "Go to Date";
            bars.push((
                Toolbar::new(vec![
                    ToolbarItem::button(B_CANCEL, "Cancel"),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::custom(B_TITLE, title, (ui.measure(title, MAC_BODY_PT, Weight::Bold) / k).ceil()),
                    ToolbarItem::flexible_space(),
                    ToolbarItem::button(B_GO, "Go"),
                ]),
                band(start, end - start),
            ));
            return bars;
        }
        let mut items = vec![];
        if x0 == 0.0 {
            items.push(ToolbarItem::icon(B_SIDEBAR, "Show Calendar List", "sidebar.left"));
        }
        items.push(ToolbarItem::icon(B_ADD, "New Event", "plus"));
        items.push(ToolbarItem::flexible_space());
        for (id, v) in [(B_DAY, View::Day), (B_WEEK, View::Week), (B_MONTH, View::Month), (B_YEAR, View::Year)] {
            items.push(ToolbarItem::button(id, v.label()).in_group(1).selected(self.view == v));
        }
        items.push(ToolbarItem::flexible_space());
        // the time zone the grids show times in: a tap steps through the zones, back to each event's own (`viewzone.rs`)
        items.push(ToolbarItem::button(B_ZONE, self.view_zone_label()).selected(self.prefs.view_zone.is_some()));
        if self.file_export {
            items.push(ToolbarItem::button(B_WEEKNO, "Week Numbers").selected(self.prefs.week_numbers));
            items.push(ToolbarItem::icon(B_EXPORT, "Export Calendar", "square.and.arrow.up"));
        }
        // the search field takes the right end of the band
        let items_end = self.search_rect(ui).map_or(end, |r| r.x - TOOLBAR_GAP_PT);
        bars.push((Toolbar::new(items), band(start, items_end - start)));
        bars.push((
            Toolbar::new(vec![
                ToolbarItem::flexible_space(),
                ToolbarItem::icon(B_PREV, "Previous", "chevron.left"),
                ToolbarItem::button(B_TODAY, "Today"),
                ToolbarItem::icon(B_NEXT, "Next", "chevron.right"),
            ]),
            Rect { x: x0 + w - SPACE_XL - NAV_W_PT, y: HEADER_MID_PT - CAPSULE_PT / 2.0, w: NAV_W_PT, h: CAPSULE_PT },
        ));
        bars
    }

    /// The search field at the right end of the band over the content (points, window coordinates). None while a sheet
    /// or the event form is open, and when the window is too narrow to fit it beside the toolbar.
    pub(super) fn search_rect(&self, ui: &Ui) -> Option<Rect> {
        if self.editor.is_some() || self.sheet_open() {
            return None;
        }
        let k = Self::k(ui);
        let (x0, w) = (self.x0 / k, self.w as f32 / k);
        let start = if x0 > 0.0 { x0 + 12.0 } else { traffic_lights_trailing() + TOOLBAR_GAP_PT };
        let end = x0 + w - 12.0;
        (end - start >= SEARCH_ROOM_PT).then(|| Rect { x: end - SEARCH_W_PT, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: SEARCH_W_PT, h: CAPSULE_PT })
    }

    fn layouts(&self, ui: &Ui) -> Vec<(Toolbar, ToolbarLayout)> {
        self.bars(ui).into_iter().map(|(bar, r)| {
            let l = bar.layout_with(r, item_w(ui), CAPSULE_PT, TOOLBAR_GAP_PT);
            (bar, l)
        }).collect()
    }

    /// The pixel rect (window coordinates) of toolbar item `id`.
    #[cfg(test)]
    pub(super) fn bar_px(&self, ui: &Ui, id: u32) -> Option<(f32, f32, f32, f32)> {
        let r = self.layouts(ui).iter().find_map(|(_, l)| l.rect_of(id))?;
        Some((ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)))
    }

    /// The target of the toolbar button under window pixel `(x, y)`.
    pub(super) fn bar_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let k = Self::k(ui);
        let id = self.layouts(ui).iter().find_map(|(bar, l)| {
            let id = l.hit(x / k, y / k)?;
            bar.items.iter().find(|i| i.id == id).filter(|i| i.enabled && id != B_TITLE).map(|_| id)
        })?;
        Some(match id {
            B_SIDEBAR => Target::Sidebar,
            B_ADD => Target::Add,
            B_DAY => Target::View(View::Day),
            B_WEEK => Target::View(View::Week),
            B_MONTH => Target::View(View::Month),
            B_YEAR => Target::View(View::Year),
            B_WEEKNO => Target::WeekNumbers,
            B_EXPORT => Target::ExportCalendar,
            B_PREV => Target::Prev,
            B_TODAY => Target::Today,
            B_NEXT => Target::Next,
            B_CANCEL => Target::Cancel,
            B_GO => Target::GoConfirm,
            B_QUICK => Target::QuickAdd,
            B_LINE => Target::LineDone,
            B_COLOUR => Target::ColourDone,
            B_ZONE => Target::ViewZone,
            _ => Target::Save,
        })
    }

    /// Every toolbar button as an accessible control (`App::controls`), in points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = vec![];
        for (bar, l) in self.layouts(ui) {
            for (id, r) in l.visible {
                let Some(item) = bar.items.iter().find(|i| i.id == id && id != B_TITLE) else { continue };
                nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: if item.selected { "selected".into() } else { String::new() }, bounds: r, enabled: item.enabled });
            }
        }
        nodes.extend(self.search_nodes(ui));
        if self.editor.is_some() {
            nodes.extend(self.editor_nodes(ui));
        } else if self.colour_sheet.is_some() {
            nodes.extend(self.colour_nodes(ui));
        } else if self.goto.is_some() {
            nodes.extend(self.goto_nodes(ui));
        } else if self.line_sheet.is_some() {
            nodes.extend(self.line_nodes(ui));
        } else if self.quick_line.is_some() {
            nodes.extend(self.quick_nodes(ui));
        } else if self.searching() {
            // the results are listed by `search_nodes`
        } else {
            if self.view == View::Month {
                nodes.extend(self.grid_nodes(ui));
            }
            if self.undo.is_some() {
                nodes.push(self.undo_node(ui));
            }
        }
        nodes
    }

    /// The month grid's day cells and the events in them, as accessible buttons. A day's button is its number band
    /// (the top `CELL_HEAD` points); each event is one line of its cell.
    fn grid_nodes(&self, ui: &Ui) -> Vec<Node> {
        let k = Self::k(ui);
        let (left, cell, top, rh) = self.grid_geom(ui);
        let (x0, left, cell, top, rh) = (self.x0 / k, left / k, cell / k, top / k, rh / k);
        let start = self.grid_start();
        let mut out = vec![];
        for row in 0..6 {
            for col in 0..7 {
                let date = start.shift_days(row * 7 + col);
                let x = x0 + left + cell * col as f32;
                let y = top + rh * row as f32;
                let label = format!("{} {} {}", WEEKDAYS[date.weekday() as usize], date.day, MONTHS[(date.month - 1) as usize]);
                let value = if date == self.selected { "selected" } else { "" };
                out.push(Node {
                    id: DAY_NODE + (row * 7 + col) as u64,
                    role: Role::Button,
                    label,
                    value: value.into(),
                    bounds: Rect { x, y, w: cell, h: CELL_HEAD },
                    enabled: true,
                });
                for (i, e) in self.cell_events(ui, date).0.into_iter().enumerate() {
                    out.push(Node {
                        id: EVENT_NODE + e.id,
                        role: Role::Button,
                        label: e.title.clone(),
                        value: String::new(),
                        bounds: Rect { x, y: y + CELL_HEAD + CELL_LINE * i as f32, w: cell, h: CELL_LINE },
                        enabled: true,
                    });
                }
            }
        }
        out
    }

    /// The event editor's fields, toggles and buttons, at the rows `rows()` lays out. The step rows (day, month and
    /// time arrows) are pointer targets only; their arrows are drawn, not published.
    fn editor_nodes(&self, ui: &Ui) -> Vec<Node> {
        let Some(draft) = &self.editor else { return vec![] };
        let k = Self::k(ui);
        let x = self.x0 / k + 36.0;
        let w = self.w as f32 / k - 72.0;
        let mut out = vec![];
        for (target, top, h) in self.rows() {
            let (id, role, label, value, enabled) = match target {
                Target::Title => (200, Role::TextField, "Title", self.fields[0].text().to_string(), true),
                Target::Location => (201, Role::TextField, "Location", self.fields[1].text().to_string(), true),
                Target::Notes => (202, Role::TextField, "Notes", self.fields[2].text().to_string(), true),
                Target::AllDay => (203, Role::Toggle, "All-day", if draft.event.all_day { "on" } else { "off" }.to_string(), true),
                Target::Toggle(Slot::Start) => (204, Role::Button, "Starts", String::new(), true),
                Target::Toggle(Slot::End) => (205, Role::Button, "Ends", String::new(), true),
                Target::Repeat => (206, Role::Button, "Repeat", pickers::repeat_label(draft.rule.as_ref()), true),
                Target::Alert => (207, Role::Button, "Alert", pickers::alarm_label(&draft.alarms), true),
                Target::Calendar => {
                    let cur = if self.extras.exists(draft.extra.calendar) { draft.extra.calendar } else { self.extras.default_id() };
                    (212, Role::Button, "Calendar", self.extras.name_of(cur), true)
                }
                Target::Zone => (213, Role::Button, "Time Zone", extras::zone_title(draft.extra.tzid.as_deref()).to_string(), true),
                Target::Travel => (214, Role::Button, "Travel Time", extras::travel_title(draft.extra.travel), true),
                Target::Duplicate => (208, Role::Button, "Duplicate Event", String::new(), true),
                Target::Export => (209, Role::Button, "Export Event", String::new(), true),
                Target::Delete => (210, Role::Button, "Delete Event", String::new(), draft.existing),
                Target::Invitees => (211, Role::StaticText, "Invitees", invitees::summary(&draft.invitees), true),
                Target::ShowInMaps => (215, Role::Button, "Show in Maps", String::new(), true),
                Target::AddInvitee => {
                    let line = draft.invitee_line.as_ref();
                    (216, if line.is_some() { Role::TextField } else { Role::Button }, "Add Invitees", line.map_or_else(String::new, |l| l.to_string()), true)
                }
                Target::InviteeSuggest(i) => {
                    let who = self.invitee_suggestions().get(i).map_or_else(String::new, |e| inviting::row_label(&extras::Invitee { address: e.email.clone(), name: e.name.clone() }));
                    (280 + i as u64, Role::Button, "Invite from Contacts", who, true)
                }
                Target::InviteeDrop(i) => {
                    let who = draft.extra.invitees.get(i).map_or_else(String::new, inviting::row_label);
                    (260 + i as u64, Role::Button, "Remove Invitee", who, true)
                }
                _ => continue,
            };
            out.push(Node { id, role, label: label.into(), value, bounds: Rect { x, y: top, w, h }, enabled });
        }
        out
    }

    /// The Undo bar: one button over its whole strip (a touch anywhere on it undoes the delete).
    fn undo_node(&self, ui: &Ui) -> Node {
        let k = Self::k(ui);
        let (x, y, w, h) = self.undo_rect(ui);
        Node {
            id: UNDO_NODE,
            role: Role::Button,
            label: "Undo".into(),
            value: "Event deleted".into(),
            bounds: Rect { x: (self.x0 + x) / k, y: y / k, w: w / k, h: h / k },
            enabled: true,
        }
    }


    /// The calendar list (`NativeOutlineView` source list): the saved events, then the holidays and the subscribed
    /// calendars. A row's dot is its colour while the calendar is shown and grey while it is hidden; a click flips it.
    fn sidebar(&self) -> SourceList {
        let dot = |shown: bool, colour: u32| if shown { colour } else { LABEL_3.now() };
        let mut items = vec![
            SidebarItem::header("On My Computer"),
            SidebarItem::row(overlay::LAYER_LOCAL, "Calendar").with_dot(dot(self.prefs.show_local, self.calendar_colour(extras::DEFAULT_CALENDAR))),
            SidebarItem::header("Other"),
            SidebarItem::row(overlay::LAYER_HOLIDAYS, "Holidays").with_dot(dot(self.prefs.show_holidays, overlay::colour(0))),
            SidebarItem::row(overlay::LAYER_BIRTHDAYS, "Birthdays").with_dot(dot(self.prefs.show_birthdays, overlay::colour(birthdays::BIRTHDAY_SLOT))),
        ];
        // the calendars the user made sit under the built-in one, before the holidays and the subscriptions
        let made: Vec<SidebarItem> = self
            .extras
            .calendars
            .iter()
            .map(|c| SidebarItem::row(overlay::LAYER_CAL + c.id, &c.name).with_dot(dot(c.shown, self.calendar_colour(c.id))))
            .collect();
        items.splice(2..2, made).for_each(drop);
        for (i, s) in self.subs.iter().enumerate() {
            items.push(SidebarItem::row(overlay::LAYER_SUBS + i as u32, &s.name).with_dot(dot(s.shown, overlay::colour(i + 1))));
        }
        SourceList::new(items, 32.0, 30.0)
    }

    /// The row of the calendar list under window pixel `(x, y)`.
    pub(super) fn layer_at(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if self.x0 <= 0.0 {
            return None;
        }
        let k = Self::k(ui);
        self.sidebar().hit(Rect { x: 0.0, y: 0.0, w: self.x0 / k, h: self.h as f32 / k }, SIDE_SCROLL, x / k, y / k)
    }

    /// The large title on the header row: the month in bold and the year (Golden Gate Calendar), or the shown range.
    pub(super) fn draw_header(&mut self, s: &mut Surface, ui: &Ui) {
        let cy = ui.pt(HEADER_MID_PT);
        let x = ui.pt(SPACE_XL);
        let (bold, rest) = if self.searching() {
            ("Search".to_string(), format!(" {}", lookup::count_label(self.search_hits.len())))
        } else {
            match self.view {
                View::Month => (MONTHS[(self.selected.month - 1) as usize].to_string(), format!(" {}", self.selected.year)),
                View::Year => (self.selected.year.to_string(), String::new()),
                _ => (self.range_label(), String::new()),
            }
        };
        // The header is a two-run title (bold month, regular year) on the content column's leading edge;
        // widgets::large_title centres one run on the readable column, so it cannot draw this header.
        // compliance-allow: LAY-07 two-run calendar header aligned to the content column (see above)
        let a = self.labels.get(ui, &bold, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now()).clone();
        s.blit(&a, x as i32, (cy - a.h as f32 / 2.0) as i32, 255, 255);
        if !rest.is_empty() {
            self.text(s, ui, &rest, MAC_LARGE_TITLE_PT, LABEL.now(), x + a.w as f32, cy);
        }
        let line = self.error.clone().map(|e| (e, LABEL.now())).or(self.notice.clone().map(|n| (n, LABEL_2.now())));
        if let Some((text, colour)) = line {
            let w = self.w as f32;
            self.centre(s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, colour, w / 2.0, ui.pt(HEADER_MID_PT - 34.0));
        }
    }

    /// The window around the content: the calendar list, the toolbar band and every toolbar.
    pub(super) fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let k = Self::k(ui);
        if self.x0 > 0.0 {
            widgets::source_list(s, ui, Rect { x: 0.0, y: 0.0, w: self.x0 / k, h: self.h as f32 / k }, &self.sidebar(), SIDE_SCROLL, None);
            s.fill_rect(self.x0 as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        }
        widgets::toolbar_band(s, ui, Rect { x: self.x0 / k, y: 0.0, w: self.w as f32 / k, h: UNIFIED_BAR_PT });
        self.draw_search(s, ui);
        for (bar, r) in self.bars(ui) {
            let placed = widgets::toolbar(s, ui, r, &bar, None);
            if let Some(t) = placed.rect_of(B_TITLE) {
                let title = bar.items.iter().find(|i| i.id == B_TITLE).map(|i| i.label.clone()).unwrap_or_default();
                let img = self.labels.get(ui, &title, MAC_BODY_PT, Weight::Bold, LABEL.now());
                s.blit(img, ui.pt(t.x) as i32, (ui.pt(t.y + t.h / 2.0) - img.h as f32 / 2.0) as i32, 255, 255);
            }
        }
    }

    /// The search field over the toolbar band, drawn with the kit's `search_field`.
    pub(super) fn draw_search(&self, s: &mut Surface, ui: &Ui) {
        let Some(r) = self.search_rect(ui) else { return };
        widgets::search_field(s, ui, &self.search_box, r, "Search", self.search_focused);
    }

    /// The content column of `s`: the same pixels, starting `x0` px from the left.
    pub(super) fn column<'a>(&self, s: &'a mut Surface) -> Surface<'a> {
        let x0 = (self.x0 as usize).min(s.w);
        Surface { w: s.w - x0, h: s.h, stride: s.stride, px: &mut s.px[x0..], y0: s.y0 }
    }

    pub(super) fn background(&self, s: &mut Surface) {
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
    }
}
