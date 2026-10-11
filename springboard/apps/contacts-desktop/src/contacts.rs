//! Contacts in the Golden Gate (Golden Gate) layout: an optional lists sidebar, the contact list with its search field, and
//! the card or the edit form in the detail pane, under a unified toolbar. Contacts are stored locally; phone, email
//! and address values copy to the shared clipboard. Geometry is in points; drawing converts with `ui.pt`.
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{search, Command, Commands, Node, Rect, Role, TextField};
use bifrost_app::glyphs::glyph;
use bifrost_app::kit::clipboard;
use bifrost_app::kit::gfx::Surface;
use bifrost_app::kit::ui::{
    Ui, Weight, CARD_BG, DESTRUCTIVE, DETAIL_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_CALLOUT_PT,
    MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MATERIAL_HUD_SOLID, SELECTION, SEPARATOR_C,
    SIDEBAR_BG, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, WHITE,
};
use bifrost_app::kit::{Event, TextCache};
use bifrost_app::layout::{traffic_lights_trailing, SIDEBAR_MIN_PT, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use springboard::deildu::scroll::Scroller;
use springboard::shell::contactstore as store;
use springboard::shell::keyboard::Out;
use springboard::shell::textarea::TextArea;
use std::path::PathBuf;
pub use store::Contact;

/// Toolbar items and control ids.
pub const BAR_SIDEBAR: u32 = 1;
pub const BAR_ADD: u32 = 2;
pub const BAR_EDIT: u32 = 3;
pub const BAR_CANCEL: u32 = 4;
pub const BAR_DONE: u32 = 5;
pub const SEARCH_ID: u32 = 6;
/// Sidebar rows.
const LIST_ALL: u32 = 100;
const LIST_FAVORITES: u32 = 101;
/// Accessibility ids of the rows and card or form controls (see `controls`). Bar and sidebar ids stay below 200.
const NODE_FAVORITE: u64 = 200;
const NODE_SHARE: u64 = 201;
const NODE_DELETE: u64 = 202;
const NODE_CANCEL: u64 = 203;
const NODE_COPY: u64 = 210;
const NODE_FIELD: u64 = 220;
const NODE_LABEL: u64 = 230;
const NODE_ADD: u64 = 240;
const ROW_NODE: u64 = 10_000;
const LIST_ALL_NODE: u64 = LIST_ALL as u64;
const LIST_FAVORITES_NODE: u64 = LIST_FAVORITES as u64;

/// Windows at least this wide (points) open with the lists sidebar shown, as the stock Contacts app does.
const SIDEBAR_AUTO_PT: f32 = 600.0;
const ROW: f32 = 44.0;
const HEADER: f32 = 24.0;
const SEARCH_H: f32 = 28.0;
const FIELD_ROW: f32 = 36.0;
const VALUE_ROW: f32 = 52.0;
const BUTTON_ROW: f32 = 36.0;
const AVATAR_D: f32 = 96.0;
const COLUMN_MAX: f32 = 520.0;
const ADD_TITLES: [&str; 3] = ["add phone", "add email", "add address"];
/// Focus index of the notes and of the search field (0..6 are the form fields).
const NOTES: usize = 6;
const SEARCH: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// No contact selected.
    List,
    Detail(u64),
    Editor(Option<u64>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sidebar(u32),
    Bar(u32),
    Search,
    SearchClear,
    Row(usize),
    Copy(usize),
    Favorite,
    Share,
    Delete,
    ConfirmDelete,
    CancelDelete,
    Field(usize),
    AddRow(usize),
    Label(usize),
    List,
    Content,
}

struct Row {
    contact: usize,
    top: f32,
    section: Option<char>,
}

/// The detail card, scroll applied, in window points.
struct CardLayout {
    avatar: (f32, f32, f32),
    name_y: f32,
    /// phone, email, address rows that have a value: (kind, rect).
    values: Vec<(usize, Rect)>,
    notes: Rect,
    favorite: Rect,
    share: Rect,
    delete: Rect,
    cancel: Option<Rect>,
    height: f32,
}

/// The edit form, scroll applied, in window points.
struct EditLayout {
    avatar: (f32, f32, f32),
    /// first, last, company, phone, email, address, notes.
    fields: [Option<Rect>; 7],
    labels: [Option<Rect>; 3],
    adds: [Option<Rect>; 3],
    notes_card: Rect,
    delete: Option<Rect>,
    cancel: Option<Rect>,
    height: f32,
}

pub struct Contacts {
    /// Window size in points.
    w: f32,
    h: f32,
    path: PathBuf,
    contacts: Vec<Contact>,
    rows: Vec<Row>,
    list_height: f32,
    screen: Screen,
    sidebar_shown: bool,
    favorites_only: bool,
    query: TextField,
    /// first, last, company, phone, email, address.
    fields: [TextField; 6],
    shown: [bool; 3],
    labels_idx: [u8; 3],
    notes: TextArea,
    notes_color: u32,
    focus: Option<usize>,
    list_scroll: Scroller,
    scroll: Scroller,
    pressed: Option<Target>,
    scroll_list: Option<bool>,
    share_req: Option<springboard::shell::share::Item>,
    confirm_delete: bool,
    /// A failed load must not turn into an overwrite of unknown data.
    load_failed: bool,
    error: Option<String>,
    toast_until: Option<f64>,
    labels: TextCache,
    dirty: bool,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Contacts {
    /// Lays the app out for a window of `w` x `h` px.
    pub fn set_size(&mut self, ui: &Ui, w: i32, h: i32) {
        self.w = w as f32 / ui.s;
        self.h = h as f32 / ui.s;
        self.fit_scroll(ui);
        self.dirty = true;
    }

    pub fn new(ui: &Ui, w: i32, h: i32, path: PathBuf) -> Self {
        let (contacts, error) = match store::load_checked(&path) {
            Ok(contacts) => (contacts, None),
            Err(e) => (vec![], Some(format!("Unable to load contacts: {e}"))),
        };
        let mut app = Self {
            w: w as f32 / ui.s,
            h: h as f32 / ui.s,
            path,
            contacts,
            rows: vec![],
            list_height: 0.0,
            screen: Screen::List,
            sidebar_shown: w as f32 / ui.s >= SIDEBAR_AUTO_PT,
            favorites_only: false,
            query: TextField::new(),
            fields: Default::default(),
            shown: [false; 3],
            labels_idx: [0; 3],
            notes: TextArea::new(ui, ui.pt(COLUMN_MAX), MAC_BODY_PT, MAC_BODY_PT, LABEL.now(), LINK),
            notes_color: LABEL.now(),
            focus: None,
            list_scroll: Scroller::new(),
            scroll: Scroller::new(),
            pressed: None,
            scroll_list: None,
            share_req: None,
            confirm_delete: false,
            load_failed: error.is_some(),
            error,
            toast_until: None,
            labels: TextCache::default(),
            dirty: true,
        };
        app.rebuild(ui);
        app
    }

    #[allow(dead_code)]
    pub fn needs_frame(&self) -> bool {
        self.dirty
    }
    #[allow(dead_code)]
    pub fn listed(&self) -> Vec<&Contact> {
        self.rows.iter().map(|r| &self.contacts[r.contact]).collect()
    }
    /// Saved contacts for system search; independent of the list's filter.
    #[allow(dead_code)]
    pub fn all_contacts(&self) -> &[Contact] {
        &self.contacts
    }

    /// Opens a saved contact by stable ID, ignoring a stale search result.
    #[allow(dead_code)]
    pub fn show_contact(&mut self, ui: &Ui, id: u64, _t: f64) -> bool {
        if !self.contacts.iter().any(|contact| contact.id == id) {
            return false;
        }
        self.open_detail(ui, id);
        true
    }

    /// True while the editor is open: its edits are not saved yet.
    pub fn editing(&self) -> bool {
        matches!(self.screen, Screen::Editor(_))
    }

    pub fn selected(&self) -> Option<&Contact> {
        if let Screen::Detail(id) = self.screen {
            self.contacts.iter().find(|c| c.id == id)
        } else {
            None
        }
    }
    #[allow(dead_code)]
    pub fn describe(&self) -> String {
        let screen = match self.screen {
            Screen::List => "list".into(),
            Screen::Detail(id) => format!("detail:{id}"),
            Screen::Editor(id) => format!("editor:{}", id.map_or_else(|| "new".into(), |id| id.to_string())),
        };
        format!(
            "contacts_screen={screen}\ncontacts_count={}\ncontacts_shown={}\ncontacts_sidebar={}\ncontacts_confirm_delete={}\ncontacts_error={}\n",
            self.contacts.len(),
            self.rows.len(),
            self.sidebar_shown,
            self.confirm_delete,
            self.error.is_some()
        )
    }

    /// Completed edits are persisted on Done; an unfinished form stays a draft.
    pub fn flush(&mut self) {}

    /// The contact the user just chose to share, if any.
    pub fn take_share(&mut self) -> Option<springboard::shell::share::Item> {
        self.share_req.take()
    }

    // ---- geometry (points) ----

    fn side(&self) -> f32 {
        if self.sidebar_shown { SIDEBAR_MIN_PT } else { 0.0 }
    }
    fn list_w(&self) -> f32 {
        ((self.w - self.side()) * 0.36).clamp(220.0, 340.0)
    }
    /// Left edge of the detail pane.
    fn detail_x(&self) -> f32 {
        self.side() + self.list_w()
    }
    /// The detail pane's readable column: (left, width).
    fn column(&self) -> (f32, f32) {
        let pane = self.w - self.detail_x();
        let cw = (pane - 2.0 * SPACE_XL * 2.0).clamp(160.0, COLUMN_MAX);
        (self.detail_x() + (pane - cw) / 2.0, cw)
    }
    fn list_bar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::icon(BAR_SIDEBAR, if self.sidebar_shown { "Hide Sidebar" } else { "Show Sidebar" }, "sidebar.left"),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(BAR_ADD, "New Contact", "plus").enabled(!self.load_failed),
        ])
    }
    fn list_bar_bounds(&self) -> Rect {
        let x = if self.sidebar_shown { self.side() + SPACE_M } else { traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT };
        let h = widgets::CAPSULE_PT;
        rect(x, (UNIFIED_BAR_PT - h) / 2.0, (self.detail_x() - SPACE_M - x).max(0.0), h)
    }
    fn detail_bar(&self) -> Toolbar {
        let mut items = vec![ToolbarItem::flexible_space()];
        match self.screen {
            Screen::List => {}
            Screen::Detail(_) => items.push(ToolbarItem::button(BAR_EDIT, "Edit").enabled(!self.load_failed)),
            Screen::Editor(_) => {
                items.push(ToolbarItem::button(BAR_CANCEL, "Cancel"));
                items.push(ToolbarItem::button(BAR_DONE, "Done"));
            }
        }
        Toolbar::new(items)
    }
    fn detail_bar_bounds(&self) -> Rect {
        let x = self.detail_x() + SPACE_M;
        let h = widgets::CAPSULE_PT;
        rect(x, (UNIFIED_BAR_PT - h) / 2.0, (self.w - SPACE_L - x).max(0.0), h)
    }
    fn bar_layouts(&self, ui: &Ui) -> [ToolbarLayout; 2] {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() {
                widgets::CAPSULE_PT
            } else {
                (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT)
            }
        };
        [
            self.list_bar().layout_with(self.list_bar_bounds(), width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT),
            self.detail_bar().layout_with(self.detail_bar_bounds(), width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT),
        ]
    }
    fn sidebar(&self) -> SourceList {
        let mut list = SourceList::new(
            vec![
                SidebarItem::header("Lists"),
                SidebarItem::row(LIST_ALL, "All Contacts").with_symbol("list.bullet"),
                SidebarItem::row(LIST_FAVORITES, "Favorites").with_symbol("tag"),
            ],
            m::SIDEBAR_ROW_PITCH,
            m::SIDEBAR_ROW_H,
        );
        list.set_selected(Some(if self.favorites_only { LIST_FAVORITES } else { LIST_ALL }));
        list
    }
    fn sidebar_bounds(&self) -> Rect {
        rect(0.0, UNIFIED_BAR_PT, self.side(), self.h - UNIFIED_BAR_PT)
    }
    fn search_rect(&self) -> Rect {
        rect(self.side() + SPACE_M, UNIFIED_BAR_PT, self.list_w() - 2.0 * SPACE_M, SEARCH_H)
    }
    fn list_top(&self) -> f32 {
        UNIFIED_BAR_PT + SEARCH_H + SPACE_M
    }
    fn list_bottom(&self) -> f32 {
        self.h - SPACE_XL - SPACE_S
    }
    fn row_rect(&self, i: usize) -> Rect {
        rect(self.side() + SPACE_S, self.list_top() + self.rows[i].top - self.list_scroll.offset(), self.list_w() - 2.0 * SPACE_S, ROW)
    }
    fn card_layout(&self, ui: &Ui) -> CardLayout {
        let (x, cw) = self.column();
        let off = self.scroll.offset();
        let mut y = UNIFIED_BAR_PT + SPACE_S - off;
        let avatar = (x + cw / 2.0, y + AVATAR_D / 2.0, AVATAR_D / 2.0);
        y += AVATAR_D + SPACE_M;
        let name_y = y;
        y += MAC_TITLE1_PT + SPACE_S + SPACE_XL;
        let mut values = vec![];
        if let Some(c) = self.selected() {
            for (k, v) in [&c.phone, &c.email, &c.address].into_iter().enumerate() {
                if !v.is_empty() {
                    values.push((k, rect(x, y, cw, VALUE_ROW)));
                    y += VALUE_ROW + m::CARD_GAP;
                }
            }
        }
        let nh = (self.notes.height() / ui.pt(1.0)).max(3.0 * MAC_BODY_PT) + SPACE_XL + SPACE_L;
        let notes = rect(x, y, cw, nh);
        y += nh + m::CARD_GAP;
        let favorite = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let share = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let delete = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let cancel = self.confirm_delete.then(|| {
            let r = rect(x, y, cw, BUTTON_ROW);
            y += BUTTON_ROW + m::CARD_GAP;
            r
        });
        CardLayout { avatar, name_y, values, notes, favorite, share, delete, cancel, height: y + off + SPACE_XL }
    }
    fn edit_layout(&self, ui: &Ui) -> EditLayout {
        let (x, cw) = self.column();
        let off = self.scroll.offset();
        let mut y = UNIFIED_BAR_PT + SPACE_S - off;
        let avatar = (x + cw / 2.0, y + AVATAR_D / 2.0, AVATAR_D / 2.0);
        y += AVATAR_D + SPACE_XL;
        let mut fields = [None; 7];
        let step = FIELD_ROW + SPACE_S;
        for f in fields.iter_mut().take(3) {
            *f = Some(rect(x, y, cw, FIELD_ROW));
            y += step;
        }
        y += SPACE_M;
        let (mut labels, mut adds) = ([None; 3], [None; 3]);
        let label_w = 88.0;
        for k in 0..3 {
            if self.shown[k] {
                labels[k] = Some(rect(x, y, label_w, FIELD_ROW));
                fields[3 + k] = Some(rect(x + label_w, y, cw - label_w, FIELD_ROW));
                y += step;
            }
        }
        for k in 0..3 {
            if !self.shown[k] {
                adds[k] = Some(rect(x, y, cw, FIELD_ROW));
                y += step;
            }
        }
        y += SPACE_M;
        let nh = (self.notes.height() / ui.pt(1.0)).max(4.0 * MAC_BODY_PT);
        let notes_card = rect(x, y, cw, nh + 2.0 * SPACE_M);
        fields[NOTES] = Some(rect(x + SPACE_M, y + SPACE_M, cw - 2.0 * SPACE_M, nh));
        y += notes_card.h + SPACE_XL;
        let (mut delete, mut cancel) = (None, None);
        if matches!(self.screen, Screen::Editor(Some(_))) {
            delete = Some(rect(x, y, cw, BUTTON_ROW));
            y += BUTTON_ROW + m::CARD_GAP;
            if self.confirm_delete {
                cancel = Some(rect(x, y, cw, BUTTON_ROW));
                y += BUTTON_ROW + m::CARD_GAP;
            }
        }
        EditLayout { avatar, fields, labels, adds, notes_card, delete, cancel, height: y + off + SPACE_XL }
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        self.list_scroll.set_max(self.list_height + SPACE_L - (self.list_bottom() - self.list_top()));
        let height = match self.screen {
            Screen::List => 0.0,
            Screen::Detail(_) => self.card_layout(ui).height,
            Screen::Editor(_) => self.edit_layout(ui).height,
        };
        self.scroll.set_max(height - self.h);
    }

    // ---- model ----

    fn rebuild(&mut self, ui: &Ui) {
        store::sort(&mut self.contacts);
        self.rows.clear();
        let mut top = 0.0;
        let mut previous = None;
        for i in store::search(&self.contacts, &self.query) {
            if self.favorites_only && !self.contacts[i].favorite {
                continue;
            }
            let first = self.contacts[i].display_name().chars().next().unwrap_or('#');
            let letter = first.to_uppercase().next().filter(|c| c.is_alphabetic()).unwrap_or('#');
            let section = (previous != Some(letter)).then_some(letter);
            if section.is_some() {
                top += HEADER;
            }
            self.rows.push(Row { contact: i, top, section });
            top += ROW;
            previous = Some(letter);
        }
        self.list_height = top;
        self.fit_scroll(ui);
        self.labels.clear();
        self.dirty = true;
    }

    fn open_detail(&mut self, ui: &Ui, id: u64) {
        if let Some(contact) = self.contacts.iter().find(|c| c.id == id) {
            self.notes.set_text(ui, &contact.notes);
        }
        self.screen = Screen::Detail(id);
        self.focus = None;
        self.notes.focused = false;
        self.confirm_delete = false;
        self.scroll.jump_top();
        self.fit_scroll(ui);
        self.dirty = true;
    }

    fn close_detail(&mut self, ui: &Ui) {
        self.screen = Screen::List;
        self.focus = None;
        self.notes.focused = false;
        self.confirm_delete = false;
        self.fit_scroll(ui);
    }

    fn focus_field(&mut self, i: usize) {
        self.focus = Some(i);
        self.notes.focused = i == NOTES;
        self.dirty = true;
    }

    fn edit_contact(&mut self, ui: &Ui, id: Option<u64>) {
        if self.load_failed {
            return;
        }
        let contact = id.and_then(|id| self.contacts.iter().find(|c| c.id == id)).cloned().unwrap_or_default();
        let name = contact.name.trim();
        let (first, last) = name.rsplit_once(char::is_whitespace).unwrap_or((name, ""));
        for (field, text) in self.fields.iter_mut().zip([
            first.trim(),
            last.trim(),
            &contact.company,
            &contact.phone,
            &contact.email,
            &contact.address,
        ]) {
            field.clear();
            field.push_str(text);
        }
        self.shown = [!contact.phone.is_empty(), !contact.email.is_empty(), !contact.address.is_empty()];
        self.labels_idx = contact.labels;
        self.notes.set_text(ui, &contact.notes);
        self.screen = Screen::Editor(id);
        self.error = None;
        self.confirm_delete = false;
        self.scroll.jump_top();
        self.fit_scroll(ui);
        self.focus_field(0);
    }

    fn cancel_editor(&mut self, ui: &Ui) {
        self.error = None;
        match self.screen {
            Screen::Editor(Some(id)) => self.open_detail(ui, id),
            _ => self.close_detail(ui),
        }
    }

    fn save_editor(&mut self, ui: &Ui) {
        let Screen::Editor(existing) = self.screen else {
            return;
        };
        if self.load_failed {
            return;
        }
        let Some(id) = existing.or_else(|| store::new_id(&self.contacts)) else {
            self.error = Some("Contact limit reached".into());
            return;
        };
        let contact = Contact {
            id,
            name: format!("{} {}", self.fields[0].trim(), self.fields[1].trim()).trim().into(),
            company: self.fields[2].trim().into(),
            phone: self.fields[3].trim().into(),
            email: self.fields[4].trim().into(),
            address: self.fields[5].trim().into(),
            labels: self.labels_idx,
            notes: self.notes.text().into(),
            favorite: existing.and_then(|id| self.contacts.iter().find(|c| c.id == id)).is_some_and(|c| c.favorite),
        };
        if !contact.valid() {
            self.error = Some("Enter a name, company, phone number or email".into());
            self.dirty = true;
            return;
        }
        let mut updated = self.contacts.clone();
        if let Some(i) = updated.iter().position(|c| c.id == id) {
            updated[i] = contact;
        } else {
            updated.push(contact);
        }
        match store::save(&self.path, &updated) {
            Ok(()) => {
                self.contacts = updated;
                self.error = None;
                self.rebuild(ui);
                self.open_detail(ui, id);
            }
            Err(e) => {
                self.error = Some(format!("Unable to save: {e}"));
                self.dirty = true;
            }
        }
    }

    fn remove_selected(&mut self, ui: &Ui) {
        let (Screen::Detail(id) | Screen::Editor(Some(id))) = self.screen else {
            return;
        };
        if self.load_failed {
            return;
        }
        let updated: Vec<_> = self.contacts.iter().filter(|c| c.id != id).cloned().collect();
        match store::save(&self.path, &updated) {
            Ok(()) => {
                self.contacts = updated;
                self.error = None;
                self.close_detail(ui);
                self.rebuild(ui);
            }
            Err(e) => {
                self.error = Some(format!("Unable to delete: {e}"));
                self.dirty = true;
            }
        }
    }

    fn toggle_favorite(&mut self, ui: &Ui) {
        let Screen::Detail(id) = self.screen else { return };
        let mut updated = self.contacts.clone();
        if let Some(c) = updated.iter_mut().find(|c| c.id == id) {
            c.favorite = !c.favorite;
        }
        match store::save(&self.path, &updated) {
            Ok(()) => {
                self.contacts = updated;
                self.rebuild(ui);
            }
            Err(e) => self.error = Some(format!("Unable to save: {e}")),
        }
    }

    // ---- input ----

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let [list_bar, detail_bar] = self.bar_layouts(ui);
        if let Some(id) = list_bar.hit(x, y).or_else(|| detail_bar.hit(x, y)) {
            return Some(Target::Bar(id));
        }
        if y < UNIFIED_BAR_PT {
            return None;
        }
        if x < self.side() {
            return self.sidebar().hit(self.sidebar_bounds(), 0.0, x, y).map(Target::Sidebar);
        }
        if x < self.detail_x() {
            let s = self.search_rect();
            if search::hits_clear(s, !self.query.is_empty(), x, y) {
                return Some(Target::SearchClear);
            }
            if s.contains(x, y) {
                return Some(Target::Search);
            }
            if y >= self.list_top() && y < self.list_bottom() {
                if let Some(i) = (0..self.rows.len()).find(|&i| self.row_rect(i).contains(x, y)) {
                    return Some(Target::Row(i));
                }
                return Some(Target::List);
            }
            return None;
        }
        match self.screen {
            Screen::List => {}
            Screen::Detail(_) => {
                let l = self.card_layout(ui);
                if let Some(&(k, _)) = l.values.iter().find(|(_, r)| r.contains(x, y)) {
                    return Some(Target::Copy(k));
                }
                if l.favorite.contains(x, y) {
                    return Some(Target::Favorite);
                }
                if l.share.contains(x, y) {
                    return Some(Target::Share);
                }
                if l.delete.contains(x, y) {
                    return Some(if self.confirm_delete { Target::ConfirmDelete } else { Target::Delete });
                }
                if l.cancel.is_some_and(|c| c.contains(x, y)) {
                    return Some(Target::CancelDelete);
                }
            }
            Screen::Editor(_) => {
                let l = self.edit_layout(ui);
                if l.delete.is_some_and(|d| d.contains(x, y)) {
                    return Some(if self.confirm_delete { Target::ConfirmDelete } else { Target::Delete });
                }
                if l.cancel.is_some_and(|c| c.contains(x, y)) {
                    return Some(Target::CancelDelete);
                }
                for k in 0..3 {
                    if l.adds[k].is_some_and(|r| r.contains(x, y)) {
                        return Some(Target::AddRow(k));
                    }
                    if l.labels[k].is_some_and(|r| r.contains(x, y)) {
                        return Some(Target::Label(k));
                    }
                }
                if l.notes_card.contains(x, y) {
                    return Some(Target::Field(NOTES));
                }
                if let Some(i) = (0..6).find(|&i| l.fields[i].is_some_and(|r| r.contains(x, y))) {
                    return Some(Target::Field(i));
                }
            }
        }
        Some(Target::Content)
    }

    fn bar(&mut self, ui: &Ui, id: u32) {
        match id {
            BAR_SIDEBAR => {
                self.sidebar_shown = !self.sidebar_shown;
                self.fit_scroll(ui);
            }
            BAR_ADD => self.edit_contact(ui, None),
            BAR_EDIT => {
                if let Screen::Detail(id) = self.screen {
                    self.edit_contact(ui, Some(id));
                }
            }
            BAR_CANCEL => self.cancel_editor(ui),
            BAR_DONE => self.save_editor(ui),
            SEARCH_ID => self.focus_field(SEARCH),
            _ => {}
        }
    }

    /// Presses the control `id` (assistive technology, keyboard focus).
    pub fn activate_control(&mut self, ui: &Ui, id: u64) -> bool {
        let Some(node) = self.controls(ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
        let target = match id {
            LIST_ALL_NODE | LIST_FAVORITES_NODE => Target::Sidebar(id as u32),
            0..=199 => {
                self.bar(ui, id as u32);
                self.dirty = true;
                return true;
            }
            NODE_FAVORITE => Target::Favorite,
            NODE_SHARE => Target::Share,
            NODE_DELETE if self.confirm_delete => Target::ConfirmDelete,
            NODE_DELETE => Target::Delete,
            NODE_CANCEL => Target::CancelDelete,
            _ if id >= ROW_NODE => Target::Row((id - ROW_NODE) as usize),
            _ if id >= NODE_ADD => Target::AddRow((id - NODE_ADD) as usize),
            _ if id >= NODE_LABEL => Target::Label((id - NODE_LABEL) as usize),
            _ if id >= NODE_FIELD => Target::Field((id - NODE_FIELD) as usize),
            _ => Target::Copy((id - NODE_COPY) as usize),
        };
        let (x, y) = (node.bounds.x + node.bounds.w / 2.0, node.bounds.y + node.bounds.h / 2.0);
        self.activate(ui, target, x, y, bifrost_app::proto::sys::now_ms());
        self.dirty = true;
        true
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, y: f32, t: f64) {
        self.dirty = true;
        if !matches!(target, Target::Field(_) | Target::Search) && self.focus == Some(SEARCH) {
            self.focus = None;
        }
        match target {
            Target::Bar(id) => self.bar(ui, id),
            Target::Sidebar(id) => {
                self.favorites_only = id == LIST_FAVORITES;
                self.list_scroll.jump_top();
                self.rebuild(ui);
            }
            Target::Search => self.focus_field(SEARCH),
            Target::SearchClear => {
                self.query.clear();
                self.list_scroll.jump_top();
                self.rebuild(ui);
            }
            Target::Row(i) => {
                if let Some(row) = self.rows.get(i) {
                    let id = self.contacts[row.contact].id;
                    self.open_detail(ui, id);
                }
            }
            Target::List => {
                if matches!(self.screen, Screen::Detail(_)) {
                    self.close_detail(ui);
                }
            }
            Target::Copy(k) => {
                if let Some(c) = self.selected() {
                    let value = [&c.phone, &c.email, &c.address][k];
                    if !value.is_empty() {
                        clipboard::set_text(value);
                        self.toast_until = Some(t + 2000.0);
                    }
                }
            }
            Target::Share => {
                if let Some(c) = self.selected() {
                    self.share_req = Some(springboard::shell::share::Item::from_contact(c));
                }
            }
            Target::Favorite => self.toggle_favorite(ui),
            Target::Delete => {
                self.confirm_delete = true;
                self.fit_scroll(ui);
            }
            Target::ConfirmDelete => self.remove_selected(ui),
            Target::CancelDelete => {
                self.confirm_delete = false;
                self.fit_scroll(ui);
            }
            Target::Field(i) => {
                if i == NOTES {
                    if let Some(r) = self.edit_layout(ui).fields[NOTES] {
                        self.notes.tap(ui.pt(x - r.x), ui.pt(y - r.y), t);
                    }
                } else {
                    self.fields[i].end();
                }
                self.focus_field(i);
            }
            Target::AddRow(k) => {
                self.shown[k] = true;
                self.fit_scroll(ui);
                self.focus_field(3 + k);
            }
            Target::Label(k) => {
                self.labels_idx[k] = (self.labels_idx[k] + 1) % store::LABELS[k].len() as u8;
            }
            Target::Content => {}
        }
    }

    fn field_mut(&mut self, i: usize) -> Option<&mut TextField> {
        match i {
            SEARCH => Some(&mut self.query),
            0..=5 => Some(&mut self.fields[i]),
            _ => None,
        }
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        let Some(i) = self.focus else {
            if matches!(out, Out::Hide | Out::Named("Escape")) && matches!(self.screen, Screen::Editor(_)) {
                self.cancel_editor(ui);
            }
            return;
        };
        if matches!(out, Out::Hide | Out::Named("Escape")) {
            if i == SEARCH {
                self.focus = None;
            } else {
                self.cancel_editor(ui);
            }
            return;
        }
        if i == NOTES {
            if out == Out::Named("Tab") {
                return;
            }
            let selected = self.notes.selection().map_or(0, |range| range.len());
            let room = store::MAX_NOTES.saturating_sub(self.notes.text().chars().count().saturating_sub(selected));
            let out = match out {
                Out::Text(text) => Out::Text(text.chars().take(room).collect()),
                Out::Enter if room == 0 => return,
                out => out,
            };
            self.notes.edit(ui, &out, t);
            self.fit_scroll(ui);
            return;
        }
        if i < 6 && matches!(out, Out::Enter | Out::Named("Tab")) {
            if let Some(j) = (i + 1..=NOTES).find(|&j| j < 3 || j == NOTES || self.shown[j - 3]) {
                self.focus_field(j);
            }
            return;
        }
        if i == SEARCH && out == Out::Enter {
            self.focus = None;
            return;
        }
        let max = if i < 3 { store::MAX_NAME } else if i < 6 { store::MAX_VALUE } else { 160 };
        let Some(field) = self.field_mut(i) else { return };
        match out {
            Out::Text(s) => {
                let room = max.saturating_sub(field.chars().count());
                field.push_str(&s.chars().take(room).collect::<String>());
            }
            Out::Backspace => field.pop(),
            Out::Named("Left") => field.move_caret(false),
            Out::Named("Right") => field.move_caret(true),
            _ => {}
        }
        if i == SEARCH {
            self.list_scroll.jump_top();
            self.rebuild(ui);
        }
    }

    /// Edit menu state for the focused field.
    pub fn commands(&self) -> Commands {
        let clip = clipboard::text().is_some_and(|s| !s.is_empty());
        match self.focus {
            Some(SEARCH) => self.query.commands(clip),
            Some(i) if i < 6 => self.fields[i].commands(clip),
            _ => Commands::default(),
        }
    }

    /// Runs an Edit menu command on the focused field.
    pub fn command(&mut self, ui: &Ui, command: Command) -> bool {
        let Some(i) = self.focus else { return false };
        let clip = clipboard::text().unwrap_or_default();
        let Some(field) = self.field_mut(i) else { return false };
        if let Some(text) = field.command(command, &clip) {
            clipboard::set_text(&text);
        }
        if i == SEARCH {
            self.rebuild(ui);
        }
        self.dirty = true;
        true
    }

    /// Every visible control with its accessibility label: the unified bar, the search field, the sidebar lists, the
    /// contact rows, and the card or form controls in the detail pane.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = vec![];
        for (bar, placed) in [self.list_bar(), self.detail_bar()].iter().zip(self.bar_layouts(ui)) {
            for (id, r) in &placed.visible {
                if let Some(item) = bar.items.iter().find(|i| i.id == *id && i.id != 0) {
                    nodes.push(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled });
                }
            }
        }
        nodes.push(Node { id: SEARCH_ID as u64, role: Role::TextField, label: "Search".into(), value: self.query.to_string(), bounds: self.search_rect(), enabled: true });
        if self.sidebar_shown {
            let list = self.sidebar();
            let bounds = self.sidebar_bounds();
            for (id, index, label) in [(LIST_ALL, 1, "All Contacts"), (LIST_FAVORITES, 2, "Favorites")] {
                let value = if list.selected() == Some(id) { "selected" } else { "" };
                nodes.push(Node { id: id as u64, role: Role::Row, label: label.into(), value: value.into(), bounds: list.item_rect(index, bounds, 0.0), enabled: true });
            }
        }
        for i in 0..self.rows.len() {
            let r = self.row_rect(i);
            if r.y + r.h < self.list_top() || r.y > self.list_bottom() {
                continue;
            }
            let label = self.contacts[self.rows[i].contact].display_name().to_string();
            nodes.push(Node { id: ROW_NODE + i as u64, role: Role::Row, label, value: String::new(), bounds: r, enabled: true });
        }
        let button = |id: u64, label: &str, bounds: Rect| Node { id, role: Role::Button, label: label.into(), value: String::new(), bounds, enabled: true };
        match self.screen {
            Screen::List => {}
            Screen::Detail(_) => {
                let l = self.card_layout(ui);
                let Some(contact) = self.selected() else { return nodes };
                for &(k, r) in &l.values {
                    let kind = ["phone", "email", "address"][k];
                    nodes.push(button(NODE_COPY + k as u64, &format!("Copy {kind}"), r));
                }
                let fav = if contact.favorite { "Remove from Favorites" } else { "Add to Favorites" };
                nodes.push(button(NODE_FAVORITE, fav, l.favorite));
                nodes.push(button(NODE_SHARE, "Share Contact", l.share));
                let delete = if self.confirm_delete { "Confirm Delete Contact" } else { "Delete Contact" };
                nodes.push(button(NODE_DELETE, delete, l.delete));
                if let Some(c) = l.cancel {
                    nodes.push(button(NODE_CANCEL, "Cancel", c));
                }
            }
            Screen::Editor(_) => {
                let l = self.edit_layout(ui);
                const FIELDS: [&str; 7] = ["First name", "Last name", "Company", "Phone", "Email", "Address", "Notes"];
                for (i, label) in FIELDS.iter().enumerate() {
                    let Some(r) = l.fields[i] else { continue };
                    let value = if i == NOTES { self.notes.text().to_string() } else { self.fields[i].to_string() };
                    nodes.push(Node { id: NODE_FIELD + i as u64, role: Role::TextField, label: (*label).into(), value, bounds: r, enabled: true });
                }
                for k in 0..3 {
                    if let Some(r) = l.labels[k] {
                        nodes.push(button(NODE_LABEL + k as u64, "Label", r));
                    }
                    if let Some(r) = l.adds[k] {
                        nodes.push(button(NODE_ADD + k as u64, ADD_TITLES[k], r));
                    }
                }
                if let Some(d) = l.delete {
                    let delete = if self.confirm_delete { "Confirm Delete Contact" } else { "Delete Contact" };
                    nodes.push(button(NODE_DELETE, delete, d));
                }
                if let Some(c) = l.cancel {
                    nodes.push(button(NODE_CANCEL, "Cancel", c));
                }
            }
        }
        nodes
    }

    /// Pointer input in px.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x / ui.s, y / ui.s),
            _ => return,
        };
        self.dirty = true;
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                self.scroll_list = None;
                if y >= UNIFIED_BAR_PT && x >= self.side() {
                    let list = x < self.detail_x();
                    if !list || y >= self.list_top() {
                        self.scroll_list = Some(list);
                        if list { self.list_scroll.down(y, t) } else { self.scroll.down(y, t) }
                    }
                }
            }
            Event::Move(..) => {
                if let Some(list) = self.scroll_list {
                    let s = if list { &mut self.list_scroll } else { &mut self.scroll };
                    s.moved(y, t);
                    if s.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) => {
                let tap = match self.scroll_list.take() {
                    Some(true) => self.list_scroll.up(y, t),
                    Some(false) => self.scroll.up(y, t),
                    None => true,
                };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|target| self.hit(ui, x, y) == Some(*target)) {
                        self.activate(ui, target, x, y, t);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.fit_scroll(ui);
        let was_moving = self.list_scroll.animating() || self.scroll.animating();
        let moving = self.list_scroll.tick(t) | self.scroll.tick(t);
        if self.toast_until.is_some_and(|until| t >= until) {
            self.toast_until = None;
            self.dirty = true;
        }
        self.dirty |= was_moving || moving;
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.list_scroll.animating() || self.scroll.animating() {
            return Some(t);
        }
        let blink = if self.focus == Some(NOTES) { self.notes.next_blink(t) } else { None };
        match (self.toast_until, blink) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // ---- drawing ----

    fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, x: f32, mid: f32) -> f32 {
        let image = self.labels.get(ui, text, size, weight, color);
        s.blit(image, ui.pt(x) as i32, (ui.pt(mid) - image.h as f32 / 2.0) as i32, 255, 255);
        image.w as f32 / ui.pt(1.0)
    }
    fn centre_text(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, cx: f32, mid: f32) {
        let image = self.labels.get(ui, text, size, weight, color);
        s.blit(image, (ui.pt(cx) - image.w as f32 / 2.0) as i32, (ui.pt(mid) - image.h as f32 / 2.0) as i32, 255, 255);
    }
    fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
        s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(radius), color);
    }
    fn avatar(&mut self, s: &mut Surface, ui: &Ui, (cx, cy, r): (f32, f32, f32), initials: &str, size: f32) {
        Self::fill(s, ui, rect(cx - r, cy - r, 2.0 * r, 2.0 * r), r, LABEL_2.now());
        self.centre_text(s, ui, initials, size, Weight::Medium, WHITE, cx, cy);
    }
    /// A grouped button row (Share Contact, Delete Contact): a card with a leading label.
    fn button_row(&mut self, s: &mut Surface, ui: &Ui, r: Rect, label: &str, color: u32) {
        Self::fill(s, ui, r, m::CARD_RADIUS, CARD_BG.now());
        self.text(s, ui, label, MAC_BODY_PT, Weight::Regular, color, r.x + SPACE_L, r.y + r.h / 2.0);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (self.list_top(), self.list_bottom());
        let x0 = self.side();
        {
            let mut content = s.rows(ui.pt(top) as usize, ui.pt(bottom) as usize);
            for i in 0..self.rows.len() {
                let r = self.row_rect(i);
                if r.y + r.h < top - HEADER || r.y > bottom {
                    continue;
                }
                let contact = self.contacts[self.rows[i].contact].clone();
                if let Some(letter) = self.rows[i].section {
                    self.text(&mut content, ui, &letter.to_string(), MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), r.x + SPACE_S, r.y - HEADER / 2.0);
                }
                let selected = self.screen == Screen::Detail(contact.id) || self.screen == Screen::Editor(Some(contact.id));
                if selected {
                    Self::fill(&mut content, ui, r, m::SELECTION_RADIUS, SELECTION.now());
                }
                let d = ROW - 2.0 * SPACE_XS - SPACE_XS;
                self.avatar(&mut content, ui, (r.x + SPACE_S + d / 2.0, r.y + r.h / 2.0, d / 2.0), &contact.initials(), MAC_CALLOUT_PT);
                let tx = r.x + 2.0 * SPACE_S + d;
                let room = ui.pt(r.x + r.w - tx - SPACE_S);
                let (ink, ink2) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
                let sub = if !contact.phone.is_empty() { contact.phone.clone() } else { contact.email.clone() };
                let name = ui.fit(contact.display_name(), MAC_BODY_PT, room);
                if sub.is_empty() {
                    self.text(&mut content, ui, &name, MAC_BODY_PT, Weight::Semibold, ink, tx, r.y + r.h / 2.0);
                } else {
                    self.text(&mut content, ui, &name, MAC_BODY_PT, Weight::Semibold, ink, tx, r.y + r.h * 0.34);
                    let sub = ui.fit(&sub, MAC_CALLOUT_PT, room);
                    self.text(&mut content, ui, &sub, MAC_CALLOUT_PT, Weight::Regular, ink2, tx, r.y + r.h * 0.7);
                }
            }
            if self.rows.is_empty() {
                let (title, hint) = if self.contacts.is_empty() {
                    ("No Contacts", "Click + to add a contact")
                } else {
                    ("No Results", "Try another name, phone or email")
                };
                let cx = x0 + self.list_w() / 2.0;
                self.centre_text(&mut content, ui, title, MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), cx, top + 3.0 * SPACE_XL);
                self.centre_text(&mut content, ui, hint, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), cx, top + 3.0 * SPACE_XL + MAC_TITLE1_PT + SPACE_S);
            }
        }
        // the list's own band above the rows, so scrolled rows never show under the search field
        let count = format!("{} Contacts", self.contacts.len());
        self.centre_text(s, ui, &count, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0 + self.list_w() / 2.0, (bottom + self.h) / 2.0);
        widgets::search_field(s, ui, &self.query, self.search_rect(), "Search", self.focus == Some(SEARCH));
    }

    fn sync_notes(&mut self, ui: &Ui) {
        let w = ui.pt(self.column().1 - 2.0 * SPACE_L);
        if self.notes_color != LABEL.now() {
            // TextArea has no setter for its text colour; rebuild it for the new appearance.
            let (text, focused) = (self.notes.text().to_string(), self.notes.focused);
            self.notes = TextArea::new(ui, w, MAC_BODY_PT, MAC_BODY_PT, LABEL.now(), LINK);
            self.notes.set_text(ui, &text);
            self.notes.focused = focused;
            self.notes_color = LABEL.now();
        }
        self.notes.set_width(ui, w);
    }

    fn draw_detail(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let Some(contact) = self.selected().cloned() else { return };
        let l = self.card_layout(ui);
        let (x, cw) = self.column();
        self.avatar(s, ui, l.avatar, &contact.initials(), MAC_LARGE_TITLE_PT);
        let name = ui.fit(contact.display_name(), MAC_TITLE1_PT, ui.pt(cw));
        let name_mid = l.name_y + (MAC_TITLE1_PT + SPACE_S) / 2.0;
        self.centre_text(s, ui, &name, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x + cw / 2.0, name_mid);
        if !contact.company.is_empty() && contact.company != contact.display_name() {
            let company = ui.fit(&contact.company, MAC_CALLOUT_PT, ui.pt(cw));
            self.centre_text(s, ui, &company, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x + cw / 2.0, name_mid + MAC_TITLE1_PT);
        }
        for &(k, r) in &l.values {
            Self::fill(s, ui, r, m::CARD_RADIUS, CARD_BG.now());
            let label = store::LABELS[k][contact.labels[k] as usize % store::LABELS[k].len()];
            self.text(s, ui, label, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), r.x + SPACE_L, r.y + r.h * 0.3);
            let value = [&contact.phone, &contact.email, &contact.address][k];
            let value = ui.fit(value, MAC_BODY_PT, ui.pt(r.w - 2.0 * SPACE_L));
            self.text(s, ui, &value, MAC_BODY_PT, Weight::Regular, LINK, r.x + SPACE_L, r.y + r.h * 0.68);
        }
        let n = l.notes;
        Self::fill(s, ui, n, m::CARD_RADIUS, CARD_BG.now());
        self.text(s, ui, "Notes", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), n.x + SPACE_L, n.y + SPACE_L);
        if contact.notes.is_empty() {
            self.text(s, ui, "No notes", MAC_BODY_PT, Weight::Regular, LABEL_3.now(), n.x + SPACE_L, n.y + SPACE_L + SPACE_XL);
        } else {
            self.notes.draw(s, ui, ui.pt(n.x + SPACE_L), ui.pt(n.y + SPACE_L + SPACE_M), t);
        }
        let fav = if contact.favorite { "Remove from Favorites" } else { "Add to Favorites" };
        self.button_row(s, ui, l.favorite, fav, LABEL.now());
        self.button_row(s, ui, l.share, "Share Contact", LABEL.now());
        let delete = if self.confirm_delete { "Confirm Delete Contact" } else { "Delete Contact" };
        self.button_row(s, ui, l.delete, delete, DESTRUCTIVE);
        if let Some(c) = l.cancel {
            self.button_row(s, ui, c, "Cancel", LINK);
        }
    }

    fn draw_editor(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let l = self.edit_layout(ui);
        let probe = Contact { name: format!("{} {}", &*self.fields[0], &*self.fields[1]), company: self.fields[2].to_string(), ..Default::default() };
        if probe.name.trim().is_empty() && probe.company.trim().is_empty() {
            Self::fill(s, ui, rect(l.avatar.0 - l.avatar.2, l.avatar.1 - l.avatar.2, 2.0 * l.avatar.2, 2.0 * l.avatar.2), l.avatar.2, LABEL_2.now());
            self.centre_text(s, ui, "Add Photo", MAC_FOOTNOTE_PT, Weight::Regular, WHITE, l.avatar.0, l.avatar.1);
        } else {
            self.avatar(s, ui, l.avatar, &probe.initials(), MAC_LARGE_TITLE_PT);
        }
        const PLACEHOLDERS: [&str; 6] = ["First name", "Last name", "Company", "Phone", "Email", "Address"];
        for i in 0..6 {
            if let Some(r) = l.fields[i] {
                widgets::text_field(s, ui, &self.fields[i], r, PLACEHOLDERS[i], self.focus == Some(i));
            }
        }
        for k in 0..3 {
            if let Some(r) = l.labels[k] {
                let name = store::LABELS[k][self.labels_idx[k] as usize % store::LABELS[k].len()];
                self.text(s, ui, name, MAC_BODY_PT, Weight::Regular, LINK, r.x + SPACE_XS, r.y + r.h / 2.0);
            }
            if let Some(r) = l.adds[k] {
                let g = MAC_BODY_PT + SPACE_XS;
                glyph(s, ui, "plus", rect(r.x + SPACE_XS, r.y + (r.h - g) / 2.0, g, g), LINK);
                self.text(s, ui, ADD_TITLES[k], MAC_BODY_PT, Weight::Regular, LINK, r.x + g + SPACE_M, r.y + r.h / 2.0);
            }
        }
        Self::fill(s, ui, l.notes_card, m::CARD_RADIUS, CARD_BG.now());
        if let Some(r) = l.fields[NOTES] {
            if self.notes.text().is_empty() && self.focus != Some(NOTES) {
                self.text(s, ui, "Notes", MAC_BODY_PT, Weight::Regular, LABEL_3.now(), r.x, r.y + MAC_BODY_PT / 2.0);
            }
            self.notes.draw(s, ui, ui.pt(r.x), ui.pt(r.y), t);
        }
        if let Some(d) = l.delete {
            self.button_row(s, ui, d, if self.confirm_delete { "Confirm Delete Contact" } else { "Delete Contact" }, DESTRUCTIVE);
        }
        if let Some(c) = l.cancel {
            self.button_row(s, ui, c, "Cancel", LINK);
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        self.sync_notes(ui);
        s.fill_rect(0, 0, s.w, s.h, DETAIL_BG.now());
        // detail pane first, so the list and the bars draw over anything that scrolls under them
        let dx = ui.pt(self.detail_x());
        match self.screen {
            Screen::List => {
                let mid = UNIFIED_BAR_PT + (self.h - UNIFIED_BAR_PT) / 2.0;
                let cx = self.detail_x() + (self.w - self.detail_x()) / 2.0;
                self.centre_text(s, ui, "No Contact Selected", MAC_TITLE1_PT, Weight::Semibold, LABEL_3.now(), cx, mid);
            }
            Screen::Detail(_) => self.draw_detail(s, ui, t),
            Screen::Editor(_) => self.draw_editor(s, ui, t),
        }
        // the list column and the sidebar are opaque columns left of the detail pane
        s.fill_rect(0, 0, dx as usize, s.h, DETAIL_BG.now());
        if self.sidebar_shown {
            let b = rect(0.0, 0.0, self.side(), self.h);
            s.fill_rect(0, 0, ui.pt(b.w) as usize, s.h, SIDEBAR_BG.now());
            widgets::source_list(s, ui, self.sidebar_bounds(), &self.sidebar(), 0.0, None);
            s.fill_rect(ui.pt(self.side()) as usize - 1, 0, 1, s.h, SEPARATOR_C.now());
        }
        self.draw_list(s, ui);
        s.fill_rect(dx as usize, 0, 1, s.h, SEPARATOR_C.now());
        // unified toolbar: one band over the list and the detail pane
        let band_x = ui.pt(self.side()) as usize;
        s.fill_rect(band_x, 0, s.w - band_x, ui.pt(UNIFIED_BAR_PT) as usize, DETAIL_BG.now());
        widgets::toolbar_band(s, ui, rect(self.side(), 0.0, self.w - self.side(), UNIFIED_BAR_PT));
        let pressed = match self.pressed {
            Some(Target::Bar(id)) => Some(id),
            _ => None,
        };
        widgets::toolbar(s, ui, self.list_bar_bounds(), &self.list_bar(), pressed);
        widgets::toolbar(s, ui, self.detail_bar_bounds(), &self.detail_bar(), pressed);
        if let Some(error) = self.error.clone() {
            let error = ui.fit(&error, MAC_FOOTNOTE_PT, ui.pt(self.w - self.detail_x() - 2.0 * SPACE_XL));
            let cx = self.detail_x() + (self.w - self.detail_x()) / 2.0;
            self.centre_text(s, ui, &error, MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE, cx, UNIFIED_BAR_PT + SPACE_S);
        }
        if self.toast_until.is_some_and(|until| t < until) {
            let cx = self.detail_x() + (self.w - self.detail_x()) / 2.0;
            let r = rect(cx - 60.0, self.h - 3.0 * SPACE_XL, 120.0, 2.0 * SPACE_M + MAC_BODY_PT);
            Self::fill(s, ui, r, r.h / 2.0, MATERIAL_HUD_SOLID.now());
            self.centre_text(s, ui, "Copied", MAC_BODY_PT, Weight::Semibold, WHITE, cx, r.y + r.h / 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ui() -> Ui {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
        ui.symbols = springboard::symbols::Symbols::load(dir);
        ui
    }

    /// A fresh contacts file in its own directory, holding `contacts`.
    fn file_with(name: &str, contacts: &[Contact]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("contacts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.txt");
        store::save(&path, contacts).unwrap();
        path
    }

    fn contact(id: u64, name: &str) -> Contact {
        Contact { id, name: name.into(), ..Default::default() }
    }

    #[test]
    fn list_is_sorted_by_name_with_one_section_per_letter() {
        let path = file_with("sorted", &[contact(1, "Bo Berg"), contact(2, "Anna As"), contact(3, "Anders Ek")]);
        let ui = test_ui();
        let app = Contacts::new(&ui, 1200, 780, path.clone());
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Anders Ek", "Anna As", "Bo Berg"]);
        let sections: String = app.rows.iter().filter_map(|r| r.section).collect();
        assert_eq!(sections, "AB");
        assert!(app.describe().contains("contacts_count=3\n"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn favorites_filter_shows_only_starred_contacts() {
        let starred = Contact { favorite: true, ..contact(3, "Cara Cole") };
        let path = file_with("favorites", &[contact(1, "Anna As"), starred]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.favorites_only = true;
        app.rebuild(&ui);
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Cara Cole"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_new_contact_from_the_editor_is_saved_to_the_file() {
        let path = file_with("create", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.edit_contact(&ui, None);
        app.fields[0].push_str("Ada");
        app.fields[1].push_str("Lovelace");
        app.fields[4].push_str("ada@example.test");
        app.save_editor(&ui);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.all_contacts().len(), 2);
        let saved = store::load(&path);
        assert!(saved.iter().any(|c| c.name == "Ada Lovelace" && c.email == "ada@example.test"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_empty_form_is_refused_and_the_file_is_untouched() {
        let path = file_with("refuse", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.edit_contact(&ui, None);
        app.save_editor(&ui);
        assert_eq!(app.error.as_deref(), Some("Enter a name, company, phone number or email"));
        assert_eq!(store::load(&path).len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn favourite_then_delete_persist_to_the_file() {
        let path = file_with("delete", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.open_detail(&ui, 2);
        app.toggle_favorite(&ui);
        assert!(store::load(&path).iter().any(|c| c.id == 2 && c.favorite));
        app.remove_selected(&ui);
        assert!(app.selected().is_none());
        let left = store::load(&path);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unreadable_file_blocks_every_write() {
        let dir = std::env::temp_dir().join(format!("contacts-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.txt");
        std::fs::write(&path, "not a contacts file\n").unwrap();
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.describe().contains("contacts_error=true"));
        app.edit_contact(&ui, None);
        assert!(matches!(app.screen, Screen::List), "the editor must not open");
        app.save_editor(&ui);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a contacts file\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
