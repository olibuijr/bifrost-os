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
    Ui, Weight, CARD_BG, CONTENT_BG, DESTRUCTIVE, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_CALLOUT_PT,
    MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MATERIAL_HUD_SOLID, SELECTION, SEPARATOR_C,
    SIDEBAR_BG, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, WHITE,
};
use bifrost_app::kit::{Event, TextCache};
use bifrost_app::layout::{traffic_lights_trailing, SIDEBAR_MIN_PT, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use bifrost_app::controls::contact_card::{self, Card, Entry, Field, Kind};
use bifrost_app::controls::contact_display::{ContactsDefaults, SortOrder};
use bifrost_app::controls::contact_groups::Groups;
use bifrost_app::controls::contact_picker::{Choice, Mode, Picker};
use bifrost_app::{AppMenu, AppMenuItem};
use springboard::deildu::scroll::Scroller;
use springboard::shell::contactstore as store;
use springboard::shell::keyboard::Out;
use springboard::shell::textarea::TextArea;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use crate::display_file;
use crate::dupes;
use crate::exchange;
use crate::extras::{self, Extra, Extras, ValueKind};
use crate::group_file;
use crate::history::History;
use crate::names;
pub use store::Contact;

/// Toolbar items and control ids.
pub const BAR_SIDEBAR: u32 = 1;
pub const BAR_ADD: u32 = 2;
pub const BAR_EDIT: u32 = 3;
pub const BAR_CANCEL: u32 = 4;
pub const BAR_DONE: u32 = 5;
pub const SEARCH_ID: u32 = 6;
/// Import and export of vCards, a new group, and add or remove from the group the list shows.
const BAR_IMPORT: u32 = 7;
const BAR_EXPORT: u32 = 8;
const BAR_EXPORT_V4: u32 = 9;
const BAR_NEW_GROUP: u32 = 10;
const BAR_GROUP: u32 = 11;
/// Group management: add people to the shown group, rename and delete it, and finish picking or renaming.
const BAR_ADD_PEOPLE: u32 = 12;
const BAR_RENAME: u32 = 13;
const BAR_DELETE_GROUP: u32 = 14;
const BAR_FINISH: u32 = 15;
/// Opens the Settings screen (sort order of the list).
const BAR_SETTINGS: u32 = 16;
/// Ids of the app's main-menu items (File and View; the shell's Edit, Window and Help are not the app's).
const MENU_IMPORT: u32 = 1;
const MENU_EXPORT_V3: u32 = 2;
const MENU_EXPORT_V4: u32 = 3;
const MENU_SIDEBAR: u32 = 4;
const MENU_FIRST_LAST: u32 = 5;
const MENU_LAST_FIRST: u32 = 6;
/// Sidebar rows.
const LIST_ALL: u32 = 100;
const LIST_FAVORITES: u32 = 101;
const LIST_RECENT: u32 = 102;
const LIST_DUPLICATES: u32 = 103;
/// Smart group `n` (its offset in `smart::Smart::ALL`) is the sidebar row `LIST_SMART + n`.
const LIST_SMART: u32 = 110;
const LIST_SMART_LAST: u32 = LIST_SMART + crate::smart::Smart::ALL.len() as u32 - 1;
/// Group `n` is the sidebar row `LIST_GROUP + n`; groups number from 1 and there are at most 1000 of them.
const LIST_GROUP: u32 = 1000;
const GROUPS_FILE: &str = "groups.txt";
const DISPLAY_FILE: &str = "display.txt";
/// Extra phone numbers, emails and addresses, beside the contacts file (see `extras.rs`).
const VALUES_FILE: &str = "values.txt";

/// What the list shows. Every filter but `All` narrows the list to a set of contact identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Favorites,
    /// Contacts added or changed since the app started.
    Recent,
    /// Contacts that have a possible duplicate in the list.
    Duplicates,
    /// A built-in smart group: the contacts whose fields match its rule.
    Smart(crate::smart::Smart),
    Group(u64),
}

impl Filter {
    /// The sidebar row that shows this filter.
    fn sidebar_id(self) -> u32 {
        match self {
            Filter::All => LIST_ALL,
            Filter::Favorites => LIST_FAVORITES,
            Filter::Recent => LIST_RECENT,
            Filter::Duplicates => LIST_DUPLICATES,
            Filter::Smart(s) => LIST_SMART + s.index() as u32,
            Filter::Group(g) => LIST_GROUP + g as u32,
        }
    }

    /// The filter a sidebar row shows.
    fn from_sidebar_id(id: u32) -> Filter {
        match id {
            LIST_FAVORITES => Filter::Favorites,
            LIST_RECENT => Filter::Recent,
            LIST_DUPLICATES => Filter::Duplicates,
            id if (LIST_SMART..=LIST_SMART_LAST).contains(&id) => match crate::smart::Smart::from_index((id - LIST_SMART) as usize) {
                Some(s) => Filter::Smart(s),
                None => Filter::All,
            },
            id if id > LIST_GROUP => Filter::Group((id - LIST_GROUP) as u64),
            _ => Filter::All,
        }
    }
}

/// Whether a control identifier names a sidebar row. The group rows sit in their own range below the list rows.
fn sidebar_node(id: u64) -> bool {
    (LIST_ALL as u64..=LIST_SMART_LAST as u64).contains(&id)
        || (LIST_GROUP as u64 + 1..=LIST_GROUP as u64 + 1000).contains(&id)
}

/// Accessibility ids of the rows and card or form controls (see `controls`). Bar and sidebar ids stay below 200.
const NODE_FAVORITE: u64 = 200;
const NODE_SHARE: u64 = 201;
const NODE_DELETE: u64 = 202;
const NODE_CANCEL: u64 = 203;
const NODE_COPY_CONTACT: u64 = 204;
const NODE_COPY: u64 = 210;
const NODE_FIELD: u64 = 220;
const NODE_LABEL: u64 = 230;
const NODE_ADD: u64 = 240;
/// The sort-order row of the Settings screen (its id sits above the form's add rows).
const NODE_SORT: u64 = 250;
const ROW_NODE: u64 = 10_000;

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
/// Focus index of the group name field, which takes the search field's place while a group is renamed.
const RENAME: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// No contact selected.
    List,
    Detail(u64),
    Editor(Option<u64>),
    /// Display settings (the sort order of the list), in the detail pane.
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sidebar(u32),
    Bar(u32),
    Search,
    SearchClear,
    Row(usize),
    Copy(usize),
    CopyContact,
    Favorite,
    Share,
    Delete,
    ConfirmDelete,
    CancelDelete,
    Field(usize),
    AddRow(usize),
    Label(usize),
    /// The sort-order row of the Settings screen: each press moves to the next choice.
    Sort,
    List,
    Content,
}

/// The sort choices the Settings row cycles through, in order.
const SORT_CHOICES: [SortOrder; 4] = [SortOrder::UserDefault, SortOrder::FamilyName, SortOrder::GivenName, SortOrder::None];

/// The card model's entry for a contact: its phone, email and address with the labels the contact picked, then its
/// extra values with their labels.
fn card_entry(c: &Contact, extra: &[Extra]) -> Entry {
    let values = [&c.phone, &c.email, &c.address];
    let keys = [contact_card::Key::Phone, contact_card::Key::Email, contact_card::Key::Address];
    let mut fields: Vec<Field> = keys
        .into_iter()
        .enumerate()
        .map(|(k, key)| {
            let labels = store::LABELS[k];
            let label = labels[c.labels[k] as usize % labels.len()];
            Field { key, label: label.to_string(), value: values[k].clone() }
        })
        .collect();
    fields.extend(extra.iter().map(|e| Field { key: card_key(e.kind), label: e.label.clone(), value: e.value.clone() }));
    Entry { id: c.id, name: c.name.clone(), fields }
}

fn card_key(kind: ValueKind) -> contact_card::Key {
    match kind {
        ValueKind::Phone => contact_card::Key::Phone,
        ValueKind::Email => contact_card::Key::Email,
        ValueKind::Address => contact_card::Key::Address,
    }
}

/// The value rows a contact's card shows, in card order: the store's values, then the extra ones. The model leaves
/// out empty values. A row's index is the number a card layout uses to copy it.
fn card_rows(c: &Contact, extra: &[Extra]) -> Vec<contact_card::Row> {
    Card::new(Kind::Existing).rows(&card_entry(c, extra))
}

/// The word an accessibility control names a card value with.
fn value_word(key: contact_card::Key) -> &'static str {
    match key {
        contact_card::Key::Phone => "phone",
        contact_card::Key::Email => "email",
        contact_card::Key::Address => "address",
        _ => "value",
    }
}

/// The contacts the list shows for a search: those whose store fields match, and those with an extra value that
/// matches, in the order of `contacts`. A blank query shows every contact.
fn list_hits(contacts: &[Contact], extras: &Extras, query: &str) -> Vec<usize> {
    let by_field: BTreeSet<usize> = store::search(contacts, query).into_iter().collect();
    (0..contacts.len()).filter(|&i| by_field.contains(&i) || extras.matches(contacts[i].id, query)).collect()
}

fn sort_label(order: SortOrder) -> &'static str {
    match order {
        SortOrder::UserDefault => "System Default",
        SortOrder::FamilyName => "Family Name",
        SortOrder::GivenName => "Given Name",
        SortOrder::None => "Keep Order",
    }
}

/// Orders two contacts for the list. `Keep Order` keeps the identifier order. The family and given names are split
/// at the last space; the collation is the app's own (see `names`), so accents and Icelandic letters sort as before.
fn compare_contacts(order: SortOrder, a: &Contact, b: &Contact) -> Ordering {
    if order == SortOrder::None {
        return a.id.cmp(&b.id);
    }
    let (ag, af) = names::split(a.display_name());
    let (bg, bf) = names::split(b.display_name());
    let (af, bf) = (if af.is_empty() { ag } else { af }, if bf.is_empty() { bg } else { bf });
    let by_given = names::key(ag).cmp(&names::key(bg)).then(names::key(af).cmp(&names::key(bf)));
    let by_family = names::key(af).cmp(&names::key(bf)).then(names::key(ag).cmp(&names::key(bg)));
    let by_name = match order {
        SortOrder::GivenName => by_given,
        _ => by_family,
    };
    by_name.then(names::key(a.display_name().trim()).cmp(&names::key(b.display_name().trim()))).then(a.id.cmp(&b.id))
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
    copy_contact: Rect,
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
    filter: Filter,
    /// Edits made this session, for the Recently Changed list.
    history: History,
    /// Contact groups, saved beside the contacts file.
    groups: Groups,
    /// The identifiers of the groups in sidebar order (the model does not list them).
    group_ids: Vec<u64>,
    /// The groups file could not be read; the group commands are off so it is not overwritten.
    groups_failed: bool,
    /// The group being renamed. Its name is typed in `rename`, which takes the search field's place.
    renaming: Option<u64>,
    rename: TextField,
    /// The group that people are being added to from the picker. The list shows every contact while it is set.
    picking: Option<u64>,
    /// Display settings: the sort order of the list, saved beside the contacts file.
    display: ContactsDefaults,
    /// A line of feedback about the last import or export, shown in place of nothing when there is no error.
    status: Option<String>,
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
    /// The contact, with its extra values, that a Copy Contact press asked to put on the pasteboard; `main.rs` takes it
    /// with `take_copy`.
    copy_out: Option<(Contact, Vec<Extra>)>,
    confirm_delete: bool,
    /// A failed load must not turn into an overwrite of unknown data.
    load_failed: bool,
    /// The extra phone numbers, emails and addresses of each contact (`values.txt`).
    extras: Extras,
    error: Option<String>,
    toast_until: Option<f64>,
    labels: TextCache,
    dirty: bool,
    /// Whether names show given name first or family name first (View menu). Saved with the display settings.
    name_order: names::NameOrder,
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
        let (mut contacts, error) = match store::load_checked(&path) {
            Ok(contacts) => (contacts, None),
            Err(e) => (vec![], Some(format!("Unable to load contacts: {e}"))),
        };
        let (mut extras, values_error) = match extras::load_checked(&path.with_file_name(VALUES_FILE)) {
            Ok(values) => (values, None),
            Err(e) => (Extras::default(), Some(format!("Unable to load contact values: {e}"))),
        };
        let load_failed = error.is_some() || values_error.is_some();
        let error = error.or(values_error);
        // Values of contacts that are no longer in the store are dropped on load.
        let live: BTreeSet<u64> = contacts.iter().map(|c| c.id).collect();
        extras.retain(|id| live.contains(&id));
        let (groups, group_ids, groups_error) = match group_file::load(&path.with_file_name(GROUPS_FILE)) {
            Ok((groups, ids)) => (groups, ids, None),
            Err(e) => (Groups::new(), Vec::new(), Some(format!("Unable to load groups: {e}"))),
        };
        let groups_failed = groups_error.is_some();
        let (display, name_order) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap_or_default();
        contacts.sort_by(|a, b| names::compare(a.display_name(), b.display_name()).then(a.id.cmp(&b.id)));
        let history = History::new(&contacts);
        let mut app = Self {
            w: w as f32 / ui.s,
            h: h as f32 / ui.s,
            path,
            contacts,
            rows: vec![],
            list_height: 0.0,
            screen: Screen::List,
            sidebar_shown: w as f32 / ui.s >= SIDEBAR_AUTO_PT,
            filter: Filter::All,
            history,
            groups,
            group_ids,
            groups_failed,
            renaming: None,
            rename: TextField::new(),
            picking: None,
            display,
            status: None,
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
            copy_out: None,
            confirm_delete: false,
            load_failed,
            extras,
            error: error.or(groups_error),
            toast_until: None,
            labels: TextCache::default(),
            dirty: true,
            name_order,
        };
        app.rebuild(ui);
        app
    }

    #[cfg_attr(not(test), allow(dead_code))] // the tests read the shown rows through this
    pub fn listed(&self) -> Vec<&Contact> {
        self.rows.iter().map(|r| &self.contacts[r.contact]).collect()
    }
    /// Saved contacts for the search index; independent of the list's filter. `None` when the file did not load, so an
    /// unreadable file never clears what the index holds.
    pub fn all_contacts(&self) -> Option<&[Contact]> {
        (!self.load_failed).then_some(self.contacts.as_slice())
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
    #[cfg_attr(not(test), allow(dead_code))] // the tests read the state dump through this
    pub fn describe(&self) -> String {
        let screen = match self.screen {
            Screen::List => "list".into(),
            Screen::Detail(id) => format!("detail:{id}"),
            Screen::Editor(id) => format!("editor:{}", id.map_or_else(|| "new".into(), |id| id.to_string())),
            Screen::Settings => "settings".into(),
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

    /// The contact the user asked to copy to the pasteboard, if any (taken once).
    pub fn take_copy(&mut self) -> Option<(Contact, Vec<Extra>)> {
        self.copy_out.take()
    }

    /// Shows a line of feedback in the header, such as the outcome of a copy.
    pub fn show_status(&mut self, text: String) {
        self.status = Some(text);
        self.dirty = true;
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
        let groups_ok = !self.load_failed && !self.groups_failed;
        let mut items = vec![ToolbarItem::icon(BAR_SIDEBAR, if self.sidebar_shown { "Hide Sidebar" } else { "Show Sidebar" }, "sidebar.left")];
        if self.renaming.is_some() || self.picking.is_some() {
            items.push(ToolbarItem::flexible_space());
            items.push(ToolbarItem::button(BAR_FINISH, "Done"));
            return Toolbar::new(items);
        }
        items.push(ToolbarItem::flexible_space());
        if let Filter::Group(_) = self.filter {
            items.push(ToolbarItem::button(BAR_ADD_PEOPLE, "Add People").enabled(groups_ok));
            items.push(ToolbarItem::button(BAR_RENAME, "Rename Group").enabled(groups_ok));
            items.push(ToolbarItem::button(BAR_DELETE_GROUP, "Delete Group").enabled(groups_ok));
        }
        items.push(ToolbarItem::button(BAR_IMPORT, "Import").enabled(!self.load_failed));
        items.push(ToolbarItem::button(BAR_EXPORT, "Export 3.0").enabled(!self.load_failed));
        items.push(ToolbarItem::button(BAR_EXPORT_V4, "Export 4.0").enabled(!self.load_failed));
        items.push(ToolbarItem::button(BAR_NEW_GROUP, "New Group").enabled(groups_ok));
        items.push(ToolbarItem::icon(BAR_SETTINGS, "Settings", "gearshape"));
        items.push(ToolbarItem::icon(BAR_ADD, "New Contact", "plus").enabled(!self.load_failed));
        Toolbar::new(items)
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
            Screen::Detail(id) => {
                if let Filter::Group(group) = self.filter {
                    let member = self.groups.groups_of(id).contains(&group);
                    let label = if member { "Remove from Group" } else { "Add to Group" };
                    items.push(ToolbarItem::button(BAR_GROUP, label).enabled(!self.load_failed && !self.groups_failed));
                }
                items.push(ToolbarItem::button(BAR_EDIT, "Edit").enabled(!self.load_failed));
            }
            Screen::Editor(_) => {
                items.push(ToolbarItem::button(BAR_CANCEL, "Cancel"));
                items.push(ToolbarItem::button(BAR_DONE, "Done"));
            }
            Screen::Settings => items.push(ToolbarItem::button(BAR_DONE, "Done")),
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
    /// The sidebar from top to bottom as (row identifier, label, symbol). A header has identifier 0.
    fn sidebar_rows(&self) -> Vec<(u32, String, &'static str)> {
        let mut rows = vec![
            (0, "Lists".to_string(), ""),
            (LIST_ALL, "All Contacts".to_string(), "list.bullet"),
            (LIST_FAVORITES, "Favorites".to_string(), "tag"),
            (LIST_RECENT, "Recently Changed".to_string(), "arrow.clockwise"),
            (LIST_DUPLICATES, "Possible Duplicates".to_string(), "doc"),
            (0, "Smart Groups".to_string(), ""),
        ];
        for s in crate::smart::Smart::ALL {
            rows.push((LIST_SMART + s.index() as u32, s.title().to_string(), s.symbol()));
        }
        rows.push((0, "Groups".to_string(), ""));
        for &id in &self.group_ids {
            if let Some(group) = self.groups.get(id) {
                rows.push((LIST_GROUP + id as u32, group.name.clone(), "square.grid.2x2"));
            }
        }
        rows
    }
    fn sidebar(&self) -> SourceList {
        let items: Vec<SidebarItem> = self
            .sidebar_rows()
            .into_iter()
            .map(|(id, label, symbol)| {
                if id == 0 {
                    SidebarItem::header(&label)
                } else {
                    SidebarItem::row(id, &label).with_symbol(symbol)
                }
            })
            .collect();
        let mut list = SourceList::new(items, m::SIDEBAR_ROW_PITCH, m::SIDEBAR_ROW_H);
        list.set_selected(Some(self.filter.sidebar_id()));
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
            for k in 0..card_rows(c, self.extras.get(c.id)).len() {
                values.push((k, rect(x, y, cw, VALUE_ROW)));
                y += VALUE_ROW + m::CARD_GAP;
            }
        }
        let nh = (self.notes.height() / ui.pt(1.0)).max(3.0 * MAC_BODY_PT) + SPACE_XL + SPACE_L;
        let notes = rect(x, y, cw, nh);
        y += nh + m::CARD_GAP;
        let favorite = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let share = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let copy_contact = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let delete = rect(x, y, cw, BUTTON_ROW);
        y += BUTTON_ROW + m::CARD_GAP;
        let cancel = self.confirm_delete.then(|| {
            let r = rect(x, y, cw, BUTTON_ROW);
            y += BUTTON_ROW + m::CARD_GAP;
            r
        });
        CardLayout { avatar, name_y, values, notes, favorite, share, copy_contact, delete, cancel, height: y + off + SPACE_XL }
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
            Screen::Settings => 0.0,
        };
        self.scroll.set_max(height - self.h);
    }

    /// The sort-order row of the Settings screen, in window points.
    fn sort_row(&self) -> Rect {
        let (x, cw) = self.column();
        rect(x, UNIFIED_BAR_PT + 3.0 * SPACE_XL, cw, ROW)
    }

    // ---- model ----

    /// The folder beside the contacts file that holds `name` (Import or Export).
    fn values_path(&self) -> PathBuf {
        self.path.with_file_name(VALUES_FILE)
    }

    fn folder(&self, name: &str) -> PathBuf {
        self.path.parent().unwrap_or(Path::new("")).join(name)
    }

    /// The contact identifiers the list shows, or `None` for every contact.
    fn filter_ids(&self) -> Option<BTreeSet<u64>> {
        match self.filter {
            Filter::All => None,
            Filter::Favorites => Some(self.contacts.iter().filter(|c| c.favorite).map(|c| c.id).collect()),
            Filter::Recent => Some(self.history.changed_this_session()),
            Filter::Duplicates => Some(dupes::duplicate_ids(&self.contacts)),
            Filter::Smart(smart) => Some(smart.ids(&self.contacts)),
            Filter::Group(group) => Some(
                self.groups
                    .members(group, false)
                    .map(|members| members.into_iter().collect::<BTreeSet<u64>>())
                    .unwrap_or_default(),
            ),
        }
    }

    fn rebuild(&mut self, ui: &Ui) {
        let order = self.display.effective_sort_order(SortOrder::FamilyName);
        self.contacts.sort_by(|a, b| compare_contacts(order, a, b));
        self.rows.clear();
        let mut top = 0.0;
        let mut previous = None;
        let shown = self.filter_ids();
        for i in list_hits(&self.contacts, &self.extras, &self.query) {
            if shown.as_ref().is_some_and(|ids| !ids.contains(&self.contacts[i].id)) {
                continue;
            }
            let name = self.contacts[i].display_name();
            let letter = match order {
                // The given name leads the sort, so its first word gives the section.
                SortOrder::GivenName => names::section(names::split(name).0.split_whitespace().next().unwrap_or(name)),
                _ => names::section(name),
            };
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
        self.status = None;
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
        let (first, last) = names::split(name);
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
                let before = std::mem::replace(&mut self.contacts, updated);
                self.record(&before);
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
                let before = std::mem::replace(&mut self.contacts, updated);
                self.record(&before);
                self.error = None;
                if self.groups.forget_contact(id) > 0 {
                    self.save_groups();
                }
                if self.extras.remove(id) {
                    if let Err(e) = extras::save(&self.values_path(), &self.extras) {
                        self.error = Some(format!("Unable to save contact values: {e}"));
                    }
                }
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
                let before = std::mem::replace(&mut self.contacts, updated);
                self.record(&before);
                self.rebuild(ui);
            }
            Err(e) => self.error = Some(format!("Unable to save: {e}")),
        }
    }

    /// Records a saved change in the session history. The file is already written, so a history error is not shown.
    fn record(&mut self, before: &[Contact]) {
        let _ = self.history.record(before, &self.contacts);
    }

    fn save_groups(&mut self) {
        if let Err(e) = group_file::save(&self.path.with_file_name(GROUPS_FILE), &self.groups, &self.group_ids) {
            self.error = Some(format!("Unable to save groups: {e}"));
        }
    }

    /// Makes a group named "Group n" and shows it.
    fn new_group(&mut self, ui: &Ui) {
        if self.load_failed || self.groups_failed {
            return;
        }
        let name = format!("Group {}", self.groups.len() + 1);
        match self.groups.add(&name, None) {
            Ok(id) => {
                self.error = None;
                self.group_ids.push(id);
                self.filter = Filter::Group(id);
                self.list_scroll.jump_top();
                self.save_groups();
                self.rebuild(ui);
            }
            Err(e) => self.error = Some(format!("Unable to add a group: {e:?}")),
        }
    }

    /// The group the list shows, when a group is selected in the sidebar and the groups file is usable.
    fn shown_group(&self) -> Option<u64> {
        match self.filter {
            Filter::Group(group) if !self.load_failed && !self.groups_failed => Some(group),
            _ => None,
        }
    }

    /// Starts renaming the group the list shows: its name moves into the field that takes the search field's place.
    fn start_rename(&mut self, ui: &Ui) {
        let Some(group) = self.shown_group() else { return };
        let Some(name) = self.groups.get(group).map(|g| g.name.clone()) else { return };
        self.rename = TextField::new();
        self.rename.push_str(&name);
        self.renaming = Some(group);
        self.query.clear();
        self.error = None;
        self.focus_field(RENAME);
        self.rebuild(ui);
    }

    /// Saves the typed name, or keeps the old one when the name is empty or too long.
    fn finish_rename(&mut self, ui: &Ui) {
        let Some(group) = self.renaming.take() else { return };
        let name = self.rename.to_string();
        self.focus = None;
        match self.groups.rename(group, &name) {
            Ok(()) => {
                self.error = None;
                self.save_groups();
            }
            Err(e) => self.error = Some(format!("A group needs a name of 1 to 160 characters ({e:?})")),
        }
        self.rebuild(ui);
    }

    /// Ends a rename without saving it.
    fn cancel_rename(&mut self, ui: &Ui) {
        self.renaming = None;
        self.focus = None;
        self.rename = TextField::new();
        self.rebuild(ui);
    }

    /// Deletes the group the list shows. Its contacts stay in the list; only the group and its memberships go.
    fn delete_group(&mut self, ui: &Ui) {
        let Some(group) = self.shown_group() else { return };
        match self.groups.remove(group) {
            Ok(removed) => {
                self.group_ids.retain(|id| !removed.contains(id));
                self.filter = Filter::All;
                self.error = None;
                self.list_scroll.jump_top();
                self.save_groups();
                self.rebuild(ui);
            }
            Err(e) => self.error = Some(format!("Unable to delete the group: {e:?}")),
        }
    }

    /// Starts the people picker for the group the list shows. Every contact is listed; a press adds it to the group.
    fn start_picking(&mut self, ui: &Ui) {
        let Some(group) = self.shown_group() else { return };
        self.picking = Some(group);
        self.filter = Filter::All;
        self.error = None;
        self.status = Some("Click the people to add to the group, then Done".into());
        self.list_scroll.jump_top();
        self.rebuild(ui);
    }

    /// Picks one contact in the people picker. Members of the group are listed but cannot be chosen again.
    fn pick_person(&mut self, ui: &Ui, row: usize) {
        let Some(group) = self.picking else { return };
        let Some(contact) = self.rows.get(row).map(|r| self.contacts[r.contact].clone()) else { return };
        let entries: Vec<Entry> = self
            .contacts
            .iter()
            .map(|c| Entry { id: c.id, name: c.display_name().to_string(), fields: Vec::new() })
            .collect();
        let mut picker = Picker::new(entries, Mode::Contact { need: None });
        for member in self.groups.members(group, false).unwrap_or_default() {
            picker.disable(member);
        }
        let name = contact.display_name().to_string();
        match picker.select(contact.id) {
            Ok(Some(Choice::Contact(id))) => match self.groups.add_member(group, id) {
                Ok(_) => {
                    self.error = None;
                    self.status = Some(format!("Added {name} to the group"));
                    self.save_groups();
                }
                Err(_) => self.error = Some("That group no longer exists".into()),
            },
            Ok(_) | Err(_) => self.status = Some(format!("{name} is already in the group")),
        }
        self.dirty = true;
        self.rebuild(ui);
    }

    /// Ends the people picker and shows the group again.
    fn finish_picking(&mut self, ui: &Ui) {
        if let Some(group) = self.picking.take() {
            self.filter = Filter::Group(group);
            self.status = None;
            self.list_scroll.jump_top();
            self.rebuild(ui);
        }
    }

    /// Moves the sort order to the next choice and saves it. The list is sorted again at once.
    fn cycle_sort(&mut self, ui: &Ui) {
        let current = self.display.sort_order();
        let at = SORT_CHOICES.iter().position(|&c| c == current).unwrap_or(0);
        self.display.set_sort_order(SORT_CHOICES[(at + 1) % SORT_CHOICES.len()]);
        self.save_display();
        self.rebuild(ui);
    }

    /// Saves the sort order and the name order beside the contacts file.
    fn save_display(&mut self) {
        if let Err(e) = display_file::save(&self.path.with_file_name(DISPLAY_FILE), &self.display, self.name_order) {
            self.error = Some(format!("Unable to save the settings: {e}"));
        }
    }

    /// Shows names first name first or last name first, saves the choice and redraws the list and the card.
    fn set_name_order(&mut self, order: names::NameOrder) {
        self.name_order = order;
        self.save_display();
        self.dirty = true;
    }

    /// The app's main menu: File holds the vCard import and export, View the sidebar and the name order. Both titles
    /// merge into the shell's File and View menus.
    pub fn main_menu(&self) -> Vec<AppMenu> {
        let ready = !self.load_failed;
        let sidebar = if self.sidebar_shown { "Hide Sidebar" } else { "Show Sidebar" };
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_IMPORT, "Import vCards...").enabled(ready),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_EXPORT_V3, "Export vCard 3.0...").enabled(ready),
                    AppMenuItem::new(MENU_EXPORT_V4, "Export vCard 4.0...").enabled(ready),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_SIDEBAR, sidebar),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIRST_LAST, "First Name First").checked(self.name_order == names::NameOrder::FirstLast),
                    AppMenuItem::new(MENU_LAST_FIRST, "Last Name First").checked(self.name_order == names::NameOrder::LastFirst),
                ],
            },
        ]
    }

    /// Runs a main-menu item. True when the screen changed and needs a frame.
    pub fn menu_action(&mut self, ui: &Ui, id: u32) -> bool {
        match id {
            MENU_IMPORT => self.import_vcards(ui),
            MENU_EXPORT_V3 => self.export_vcards(exchange::Version::V3),
            MENU_EXPORT_V4 => self.export_vcards(exchange::Version::V4),
            MENU_SIDEBAR => self.bar(ui, BAR_SIDEBAR),
            MENU_FIRST_LAST => self.set_name_order(names::NameOrder::FirstLast),
            MENU_LAST_FIRST => self.set_name_order(names::NameOrder::LastFirst),
            _ => return false,
        }
        true
    }

    /// Adds the open contact to the group the list shows, or takes it out when it is already a member.
    fn toggle_group(&mut self, ui: &Ui) {
        let (Screen::Detail(id), Filter::Group(group)) = (self.screen, self.filter) else {
            return;
        };
        if self.load_failed || self.groups_failed {
            return;
        }
        let member = self.groups.groups_of(id).contains(&group);
        let changed = if member { self.groups.remove_member(group, id) } else { self.groups.add_member(group, id) };
        if changed.is_err() {
            self.error = Some("That group no longer exists".into());
            return;
        }
        self.error = None;
        self.save_groups();
        self.rebuild(ui);
    }

    /// Reads every `.vcf` file in the Import folder. A card that matches a contact already in the list is skipped.
    fn import_vcards(&mut self, ui: &Ui) {
        if self.load_failed {
            return;
        }
        let folder = self.folder("Import");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("vcf")))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if files.is_empty() {
            self.error = None;
            self.status = Some("Put vCard (.vcf) files in the Contacts Import folder, then choose Import".into());
            self.dirty = true;
            return;
        }
        let texts: Vec<Option<String>> = files.iter().map(|file| std::fs::read(file).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())).collect();
        let _ = self.merge_card_texts(ui, texts);
    }

    /// Merges vCard texts into the contacts (a card that duplicates one is skipped), saves them and returns the summary
    /// the status line shows. `None` is a text that could not be read. The file import and the CardDAV sync both use it.
    pub fn merge_card_texts(&mut self, ui: &Ui, texts: Vec<Option<String>>) -> String {
        if self.load_failed {
            return "The contacts could not be read, so nothing was imported.".into();
        }
        let mut updated = self.contacts.clone();
        let mut updated_extras = self.extras.clone();
        let (mut added, mut skipped, mut invalid, mut extra, mut unreadable) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for text in texts {
            let read = text.and_then(|t| exchange::import(&t).ok());
            let Some(read) = read else {
                unreadable += 1;
                continue;
            };
            invalid += read.invalid;
            extra += read.extra_values;
            for (contact, values) in read.contacts.into_iter().zip(read.values) {
                if dupes::has_duplicate(&updated, &contact) {
                    skipped += 1;
                    continue;
                }
                let Some(id) = store::new_id(&updated) else {
                    self.error = Some("Contact limit reached".into());
                    break;
                };
                updated.push(Contact { id, ..contact });
                for value in values {
                    updated_extras.add(id, value);
                }
                added += 1;
            }
        }
        let mut parts = vec![format!("Imported {added} contacts")];
        if skipped > 0 {
            parts.push(format!("skipped {skipped} duplicates"));
        }
        if invalid > 0 {
            parts.push(format!("{invalid} cards could not be kept"));
        }
        if extra > 0 {
            parts.push(format!("{extra} extra phone numbers or emails were not kept (a contact keeps {} extra values)", extras::MAX_EXTRAS));
        }
        if unreadable > 0 {
            parts.push(format!("{unreadable} files could not be read"));
        }
        let summary = parts.join(", ") + ".";
        if added == 0 {
            self.status = Some(summary.clone());
            self.dirty = true;
            return summary;
        }
        match store::save(&self.path, &updated) {
            Ok(()) => {
                let before = std::mem::replace(&mut self.contacts, updated);
                self.record(&before);
                self.error = None;
                self.extras = updated_extras;
                if let Err(e) = extras::save(&self.values_path(), &self.extras) {
                    self.error = Some(format!("Unable to save contact values: {e}"));
                }
                self.status = Some(summary.clone());
                self.rebuild(ui);
            }
            Err(e) => self.error = Some(format!("Unable to import: {e}")),
        }
        summary
    }

    /// Writes every contact to `Export/contacts-3.0.vcf` or `Export/contacts-4.0.vcf` beside the contacts file.
    fn export_vcards(&mut self, version: exchange::Version) {
        if self.load_failed {
            return;
        }
        let (file, label) = match version {
            exchange::Version::V3 => ("contacts-3.0.vcf", "3.0"),
            exchange::Version::V4 => ("contacts-4.0.vcf", "4.0"),
        };
        let folder = self.folder("Export");
        let text = exchange::export(&self.contacts, &self.extras, version);
        match std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(file), text)) {
            Ok(()) => {
                self.error = None;
                self.status = Some(format!("Exported {} contacts as vCard {label} to the Contacts Export folder", self.contacts.len()));
            }
            Err(e) => self.error = Some(format!("Unable to export: {e}")),
        }
        self.dirty = true;
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
                if l.copy_contact.contains(x, y) {
                    return Some(Target::CopyContact);
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
            Screen::Settings => {
                if self.sort_row().contains(x, y) {
                    return Some(Target::Sort);
                }
            }
        }
        Some(Target::Content)
    }

    fn bar(&mut self, ui: &Ui, id: u32) {
        // A rename is cancelled by any other control; the picker ends when a control outside it is pressed.
        if self.renaming.is_some() && !matches!(id, BAR_FINISH | SEARCH_ID) {
            self.cancel_rename(ui);
        }
        if self.picking.is_some() && !matches!(id, BAR_FINISH | SEARCH_ID) {
            self.finish_picking(ui);
        }
        match id {
            BAR_FINISH => {
                if self.renaming.is_some() {
                    self.finish_rename(ui);
                } else {
                    self.finish_picking(ui);
                }
            }
            BAR_ADD_PEOPLE => self.start_picking(ui),
            BAR_RENAME => self.start_rename(ui),
            BAR_DELETE_GROUP => self.delete_group(ui),
            // An open editor keeps its unsaved edits, so Settings waits until it is done or cancelled.
            BAR_SETTINGS if !self.editing() => {
                self.screen = Screen::Settings;
                self.focus = None;
                self.status = None;
                self.fit_scroll(ui);
            }
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
            BAR_DONE => {
                if self.screen == Screen::Settings {
                    self.screen = Screen::List;
                    self.fit_scroll(ui);
                } else {
                    self.save_editor(ui);
                }
            }
            BAR_IMPORT => self.import_vcards(ui),
            BAR_EXPORT => self.export_vcards(exchange::Version::V3),
            BAR_EXPORT_V4 => self.export_vcards(exchange::Version::V4),
            BAR_NEW_GROUP => self.new_group(ui),
            BAR_GROUP => self.toggle_group(ui),
            SEARCH_ID => self.focus_field(if self.renaming.is_some() { RENAME } else { SEARCH }),
            _ => {}
        }
    }

    /// Presses the control `id` (assistive technology, keyboard focus).
    pub fn activate_control(&mut self, ui: &Ui, id: u64) -> bool {
        let Some(node) = self.controls(ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
        let target = match id {
            _ if sidebar_node(id) => Target::Sidebar(id as u32),
            0..=199 => {
                self.bar(ui, id as u32);
                self.dirty = true;
                return true;
            }
            NODE_FAVORITE => Target::Favorite,
            NODE_SHARE => Target::Share,
            NODE_COPY_CONTACT => Target::CopyContact,
            NODE_SORT => Target::Sort,
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
        if self.renaming.is_some() && !matches!(target, Target::Search | Target::Bar(_)) {
            self.cancel_rename(ui);
        }
        match target {
            Target::Bar(id) => self.bar(ui, id),
            Target::Sidebar(id) => {
                if self.picking.is_some() {
                    self.finish_picking(ui);
                }
                self.filter = Filter::from_sidebar_id(id);
                self.status = None;
                self.list_scroll.jump_top();
                self.rebuild(ui);
            }
            Target::Search => self.focus_field(if self.renaming.is_some() { RENAME } else { SEARCH }),
            Target::SearchClear => {
                self.query.clear();
                self.list_scroll.jump_top();
                self.rebuild(ui);
            }
            Target::Row(i) => {
                if self.picking.is_some() {
                    self.pick_person(ui, i);
                } else if let Some(row) = self.rows.get(i) {
                    let id = self.contacts[row.contact].id;
                    self.open_detail(ui, id);
                }
            }
            Target::Sort => self.cycle_sort(ui),
            Target::List => {
                if matches!(self.screen, Screen::Detail(_)) {
                    self.close_detail(ui);
                }
            }
            Target::Copy(k) => {
                if let Some(c) = self.selected() {
                    let value = card_rows(c, self.extras.get(c.id)).get(k).map(|row| row.value.clone()).unwrap_or_default();
                    if !value.is_empty() {
                        clipboard::set_text(&value);
                        self.toast_until = Some(t + 2000.0);
                    }
                }
            }
            Target::Share => {
                if let Some(c) = self.selected() {
                    self.share_req = Some(springboard::shell::share::Item::from_contact(c));
                }
            }
            Target::CopyContact => {
                let copy = self.selected().cloned().map(|c| {
                    let extra = self.extras.get(c.id).to_vec();
                    (c, extra)
                });
                self.copy_out = copy;
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
            RENAME => Some(&mut self.rename),
            0..=5 => Some(&mut self.fields[i]),
            _ => None,
        }
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        let Some(i) = self.focus else {
            if matches!(out, Out::Hide | Out::Named("Escape")) && matches!(self.screen, Screen::Editor(_)) {
                self.cancel_editor(ui);
            } else if matches!(out, Out::Hide | Out::Named("Escape")) && self.screen == Screen::Settings {
                self.screen = Screen::List;
                self.fit_scroll(ui);
            }
            return;
        };
        if i == RENAME {
            match out {
                Out::Enter => self.finish_rename(ui),
                Out::Hide | Out::Named("Escape") => self.cancel_rename(ui),
                Out::Text(s) => {
                    let room = bifrost_app::controls::contact_groups::MAX_NAME.saturating_sub(self.rename.chars().count());
                    let text: String = s.chars().take(room).collect();
                    self.rename.push_str(&text);
                }
                Out::Backspace => self.rename.pop(),
                Out::Named("Left") => self.rename.move_caret(false),
                Out::Named("Right") => self.rename.move_caret(true),
                _ => {}
            }
            return;
        }
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
            Some(RENAME) => self.rename.commands(clip),
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
        if self.renaming.is_some() {
            nodes.push(Node { id: SEARCH_ID as u64, role: Role::TextField, label: "Group Name".into(), value: self.rename.to_string(), bounds: self.search_rect(), enabled: true });
        } else {
            nodes.push(Node { id: SEARCH_ID as u64, role: Role::TextField, label: "Search".into(), value: self.query.to_string(), bounds: self.search_rect(), enabled: true });
        }
        if self.sidebar_shown {
            let list = self.sidebar();
            let bounds = self.sidebar_bounds();
            for (index, (id, label, _)) in self.sidebar_rows().into_iter().enumerate() {
                if id == 0 {
                    continue;
                }
                let value = if list.selected() == Some(id) { "selected" } else { "" };
                nodes.push(Node { id: id as u64, role: Role::Row, label, value: value.into(), bounds: list.item_rect(index, bounds, 0.0), enabled: true });
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
                let rows = card_rows(contact, self.extras.get(contact.id));
                for &(k, r) in &l.values {
                    let kind = rows.get(k).map_or("value", |row| value_word(row.key));
                    nodes.push(button(NODE_COPY + k as u64, &format!("Copy {kind}"), r));
                }
                let fav = if contact.favorite { "Remove from Favorites" } else { "Add to Favorites" };
                nodes.push(button(NODE_FAVORITE, fav, l.favorite));
                nodes.push(button(NODE_SHARE, "Share Contact", l.share));
                nodes.push(button(NODE_COPY_CONTACT, "Copy Contact", l.copy_contact));
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
            Screen::Settings => {
                let value = sort_label(self.display.effective_sort_order(SortOrder::FamilyName));
                let shown = if self.display.sort_order() == SortOrder::UserDefault { format!("{value} (System Default)") } else { value.to_string() };
                nodes.push(Node { id: NODE_SORT, role: Role::Button, label: "Sort Order".into(), value: shown, bounds: self.sort_row(), enabled: true });
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
                let shown = names::shown(contact.display_name(), self.name_order);
                let name = ui.fit(&shown, MAC_BODY_PT, room);
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
        if self.renaming.is_some() {
            widgets::search_field(s, ui, &self.rename, self.search_rect(), "Group Name", self.focus == Some(RENAME));
        } else {
            widgets::search_field(s, ui, &self.query, self.search_rect(), "Search", self.focus == Some(SEARCH));
        }
    }

    /// The Settings screen: one grouped card with the sort order. A press on the row moves to the next choice.
    fn draw_settings(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, cw) = self.column();
        let mid = UNIFIED_BAR_PT + SPACE_XL;
        self.centre_text(s, ui, "Settings", MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x + cw / 2.0, mid);
        let r = self.sort_row();
        Self::fill(s, ui, r, m::CARD_RADIUS, CARD_BG.now());
        self.text(s, ui, "Sort Order", MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + SPACE_L, r.y + r.h / 2.0);
        let value = sort_label(self.display.effective_sort_order(SortOrder::FamilyName));
        let room = ui.pt(cw - 2.0 * SPACE_L - MAC_BODY_PT * 8.0);
        let value = ui.fit(value, MAC_BODY_PT, room);
        let end = r.x + r.w - SPACE_L;
        let width = ui.measure(&value, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0);
        self.text(s, ui, &value, MAC_BODY_PT, Weight::Regular, LINK, end - width, r.y + r.h / 2.0);
        let note = "Applies to the contact list. Keep Order lists contacts as they were added.";
        let note = ui.fit(note, MAC_FOOTNOTE_PT, ui.pt(cw));
        self.centre_text(s, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + cw / 2.0, r.y + r.h + SPACE_M);
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
        let shown = names::shown(contact.display_name(), self.name_order);
        let name = ui.fit(&shown, MAC_TITLE1_PT, ui.pt(cw));
        let name_mid = l.name_y + (MAC_TITLE1_PT + SPACE_S) / 2.0;
        self.centre_text(s, ui, &name, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x + cw / 2.0, name_mid);
        if !contact.company.is_empty() && contact.company != contact.display_name() {
            let company = ui.fit(&contact.company, MAC_CALLOUT_PT, ui.pt(cw));
            self.centre_text(s, ui, &company, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x + cw / 2.0, name_mid + MAC_TITLE1_PT);
        }
        // the value rows come from the card model, in the same order as the layout's rects
        for (row, &(_, r)) in card_rows(&contact, self.extras.get(contact.id)).iter().zip(&l.values) {
            Self::fill(s, ui, r, m::CARD_RADIUS, CARD_BG.now());
            self.text(s, ui, &row.label, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), r.x + SPACE_L, r.y + r.h * 0.3);
            let value = ui.fit(&row.value, MAC_BODY_PT, ui.pt(r.w - 2.0 * SPACE_L));
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
        self.button_row(s, ui, l.copy_contact, "Copy Contact", LABEL.now());
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
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
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
            Screen::Settings => self.draw_settings(s, ui),
        }
        // the list column and the sidebar are opaque columns left of the detail pane
        s.fill_rect(0, 0, dx as usize, s.h, CONTENT_BG.now());
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
        s.fill_rect(band_x, 0, s.w - band_x, ui.pt(UNIFIED_BAR_PT) as usize, CONTENT_BG.now());
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
        } else if let Some(status) = self.status.clone() {
            let status = ui.fit(&status, MAC_FOOTNOTE_PT, ui.pt(self.w - self.detail_x() - 2.0 * SPACE_XL));
            let cx = self.detail_x() + (self.w - self.detail_x()) / 2.0;
            self.centre_text(s, ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), cx, UNIFIED_BAR_PT + SPACE_S);
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
    fn copy_contact_queues_the_open_contact_once_from_the_card_and_the_accessibility_control() {
        let path = file_with("copy", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path);
        app.open_detail(&ui, 1);
        assert!(app.activate_control(&ui, NODE_COPY_CONTACT));
        assert_eq!(app.take_copy().map(|(c, _)| c.id), Some(1));
        assert!(app.take_copy().is_none(), "taken once");
        app.open_detail(&ui, 2);
        app.activate(&ui, Target::CopyContact, 0.0, 0.0, 0.0);
        assert_eq!(app.take_copy().map(|(c, _)| c.name), Some("Bo Berg".to_string()));
    }

    #[test]
    fn list_is_sorted_by_name_with_one_section_per_letter() {
        let path = file_with("sorted", &[contact(1, "Bo Berg"), contact(2, "Anna As"), contact(3, "Anders Ek")]);
        let ui = test_ui();
        let app = Contacts::new(&ui, 1200, 780, path.clone());
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Anna As", "Bo Berg", "Anders Ek"], "sorted by family name");
        let sections: String = app.rows.iter().filter_map(|r| r.section).collect();
        assert_eq!(sections, "ABE");
        assert!(app.describe().contains("contacts_count=3\n"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn favorites_filter_shows_only_starred_contacts() {
        let starred = Contact { favorite: true, ..contact(3, "Cara Cole") };
        let path = file_with("favorites", &[contact(1, "Anna As"), starred]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.filter = Filter::Favorites;
        app.rebuild(&ui);
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Cara Cole"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_group_is_made_filled_from_the_open_contact_and_saved_to_its_file() {
        let path = file_with("groups", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.open_detail(&ui, 1);
        app.new_group(&ui);
        assert_eq!(app.filter, Filter::Group(1));
        assert!(app.listed().is_empty(), "the new group has no members yet");
        app.open_detail(&ui, 1);
        app.toggle_group(&ui);
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Anna As"]);
        let (saved, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
        assert_eq!(saved.members(ids[0], false).unwrap(), vec![1]);
        app.open_detail(&ui, 1);
        app.toggle_group(&ui);
        assert!(app.listed().is_empty(), "a second press takes the contact out again");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_group_can_be_renamed_and_deleted_and_the_file_keeps_the_rest() {
        let path = file_with("rename", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.new_group(&ui);
        app.new_group(&ui);
        assert_eq!(app.group_ids, vec![1, 2]);
        app.filter = Filter::Group(1);
        app.start_rename(&ui);
        assert_eq!(app.renaming, Some(1));
        assert_eq!(app.rename.to_string(), "Group 1", "the name is typed into the field that replaces search");
        app.rename.clear();
        app.rename.push_str("Friends");
        app.finish_rename(&ui);
        assert_eq!(app.renaming, None);
        assert_eq!(app.groups.get(1).unwrap().name, "Friends");

        app.filter = Filter::Group(1);
        app.delete_group(&ui);
        assert_eq!(app.filter, Filter::All);
        assert_eq!(app.group_ids, vec![2], "the deleted group leaves the sidebar");
        let (back, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
        assert_eq!(ids.len(), 1, "the file keeps only the group that is left");
        assert_eq!(back.get(ids[0]).unwrap().name, "Group 2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_people_picker_adds_each_contact_once() {
        let path = file_with("picker", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.new_group(&ui);
        app.start_picking(&ui);
        assert_eq!(app.picking, Some(1));
        assert_eq!(app.filter, Filter::All, "the picker lists every contact");
        app.pick_person(&ui, 0);
        assert_eq!(app.groups.members(1, false).unwrap(), vec![1]);
        assert!(app.status.as_deref().unwrap().contains("Added Anna As"), "{:?}", app.status);
        app.pick_person(&ui, 0);
        assert_eq!(app.groups.members(1, false).unwrap(), vec![1], "a member cannot be chosen twice");
        assert!(app.status.as_deref().unwrap().contains("already"), "{:?}", app.status);
        app.finish_picking(&ui);
        assert_eq!(app.picking, None);
        assert_eq!(app.filter, Filter::Group(1));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_card_shows_the_values_a_contact_has_under_their_labels() {
        let mut ann = contact(1, "Anna As");
        ann.phone = "555 0100".into();
        ann.labels = [2, 1, 0];
        ann.address = "Hverfisgata 1".into();
        let rows: Vec<(String, String)> = card_rows(&ann, &[]).into_iter().map(|r| (r.label, r.value)).collect();
        assert_eq!(
            rows,
            vec![("work".to_string(), "555 0100".to_string()), ("home".to_string(), "Hverfisgata 1".to_string())],
            "the empty email has no row; the phone is the third phone label"
        );
        let path = file_with("card-rows", &[ann]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.open_detail(&ui, 1);
        let slots: Vec<usize> = app.card_layout(&ui).values.iter().map(|v| v.0).collect();
        assert_eq!(slots, vec![0, 1], "one row for the phone and one for the address, so copy and layout use the same row numbers");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn extra_values_follow_the_store_values_on_the_card_and_copy_by_row() {
        let mut ann = contact(1, "Anna As");
        ann.phone = "555 0100".into();
        ann.labels = [2, 1, 0];
        let extra = [
            Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0199".into() },
            Extra { kind: ValueKind::Email, label: "work".into(), value: "anna@work.is".into() },
        ];
        let rows: Vec<(String, String)> = card_rows(&ann, &extra).into_iter().map(|r| (r.label, r.value)).collect();
        assert_eq!(
            rows,
            vec![
                ("work".to_string(), "555 0100".to_string()),
                ("home".to_string(), "555 0199".to_string()),
                ("work".to_string(), "anna@work.is".to_string()),
            ],
            "store phone, extra phone, then extra email: the card groups by kind"
        );
        assert_eq!(value_word(card_rows(&ann, &extra)[2].key), "email");
    }

    #[test]
    fn a_search_matches_extra_values_as_well_as_store_fields() {
        let contacts = vec![contact(1, "Anna As"), contact(2, "Bo Berg")];
        let mut extras = Extras::default();
        assert!(extras.add(2, Extra { kind: ValueKind::Email, label: "work".into(), value: "bo@berg.is".into() }));
        assert_eq!(list_hits(&contacts, &extras, "BO@BERG"), vec![1], "only Bo has that extra email");
        assert_eq!(list_hits(&contacts, &extras, "anna"), vec![0], "the store's name still matches");
        assert_eq!(list_hits(&contacts, &extras, "zzz"), Vec::<usize>::new());
        assert_eq!(list_hits(&contacts, &extras, ""), vec![0, 1], "a blank search shows everyone");
    }

    #[test]
    fn deleting_a_contact_drops_its_extra_values_from_the_file() {
        let path = file_with("extras-delete", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.extras.add(1, Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0199".into() }));
        assert!(app.extras.add(2, Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0299".into() }));
        extras::save(&path.with_file_name(VALUES_FILE), &app.extras).unwrap();
        app.open_detail(&ui, 1);
        app.remove_selected(&ui);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.extras.get(1).is_empty());
        let left = extras::load_checked(&path.with_file_name(VALUES_FILE)).unwrap();
        assert_eq!(left.get(2).len(), 1, "Bo keeps his value");
        assert!(left.get(1).is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_sort_order_decides_which_name_leads() {
        let zoe = contact(1, "Zoe Ann");
        let adam = contact(2, "Adam Zed");
        assert_eq!(compare_contacts(SortOrder::FamilyName, &zoe, &adam), Ordering::Less, "Ann sorts before Zed");
        assert_eq!(compare_contacts(SortOrder::GivenName, &zoe, &adam), Ordering::Greater, "Adam sorts before Zoe");
        assert_eq!(compare_contacts(SortOrder::None, &zoe, &adam), Ordering::Less, "identifier order");
    }

    #[test]
    fn the_settings_row_cycles_the_sort_and_saves_it() {
        let path = file_with("settings", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.screen = Screen::Settings;
        assert_eq!(app.display.effective_sort_order(SortOrder::FamilyName), SortOrder::FamilyName);
        app.cycle_sort(&ui);
        assert_eq!(app.display.sort_order(), SortOrder::FamilyName);
        app.cycle_sort(&ui);
        assert_eq!(app.display.sort_order(), SortOrder::GivenName);
        let (saved, _) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved.sort_order(), SortOrder::GivenName);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_file_menu_and_view_menu_are_the_app_menus_and_name_order_is_saved() {
        let path = file_with("menus", &[contact(1, "Ada Lovelace")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        let menus = app.main_menu();
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "View"]);
        assert_eq!(menus[0].items.iter().filter(|i| !i.separator).count(), 3, "import, export 3.0, export 4.0");
        assert_eq!(menus[1].items[0].title, "Hide Sidebar", "the sidebar starts shown on a wide window");
        assert!(menus[1].items[2].checked, "first name first is the default");
        assert!(app.menu_action(&ui, MENU_LAST_FIRST));
        assert_eq!(app.name_order, names::NameOrder::LastFirst);
        let menus = app.main_menu();
        assert!(!menus[1].items[2].checked);
        assert!(menus[1].items[3].checked);
        let (_, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved, names::NameOrder::LastFirst);
        assert!(!app.menu_action(&ui, 99), "an unknown id is not handled");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_vcard_import_folder_adds_new_people_and_skips_duplicates() {
        let path = file_with("import", &[contact(1, "Anna As")]);
        let folder = path.parent().unwrap().join("Import");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("people.vcf"),
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Anna As\r\nEMAIL;TYPE=\"home\":anna@example.test\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Bo Berg\r\nN:Berg;Bo;;;\r\nEND:VCARD\r\n",
        )
        .unwrap();
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.import_vcards(&ui);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.all_contacts().map(|all| all.len()), Some(2), "Anna is a duplicate by name; Bo is new");
        assert!(app.status.as_deref().unwrap().contains("Imported 1 contacts"), "{:?}", app.status);
        assert_eq!(store::load(&path).len(), 2);
        app.export_vcards(exchange::Version::V4);
        let exported = std::fs::read_to_string(path.parent().unwrap().join("Export/contacts-4.0.vcf")).unwrap();
        assert!(exported.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"), "{exported}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_duplicates_filter_lists_only_contacts_with_a_twin() {
        let twin = Contact { email: "anna@x.is".into(), ..contact(2, "A. As") };
        let path = file_with("dupes", &[Contact { email: "anna@x.is".into(), ..contact(1, "Anna As") }, twin, contact(3, "Cara Cole")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.filter = Filter::Duplicates;
        app.rebuild(&ui);
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A. As", "Anna As"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_smart_group_row_lists_the_contacts_that_match_its_rule() {
        let path = file_with(
            "smart",
            &[Contact { email: "a@x.is".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg"), contact(3, "Cara Cole")],
        );
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        let with_email = LIST_SMART + crate::smart::Smart::WithEmail.index() as u32;
        assert!(sidebar_node(with_email as u64));
        app.filter = Filter::from_sidebar_id(with_email);
        assert_eq!(app.filter, Filter::Smart(crate::smart::Smart::WithEmail));
        assert_eq!(app.filter.sidebar_id(), with_email);
        app.rebuild(&ui);
        let names: Vec<&str> = app.listed().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Anna As"]);
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
        assert_eq!(app.all_contacts().map(|all| all.len()), Some(2));
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
    fn an_unreadable_file_gives_the_search_index_nothing_to_clear() {
        let ui = test_ui();
        let loaded = Contacts::new(&ui, 1200, 780, file_with("index-ok", &[contact(1, "Anna As")]));
        assert_eq!(loaded.all_contacts().map(|all| all.len()), Some(1));
        let dir = std::env::temp_dir().join(format!("contacts-index-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.txt");
        std::fs::write(&path, "not a contacts file\n").unwrap();
        let broken = Contacts::new(&ui, 1200, 780, path);
        assert!(broken.all_contacts().is_none(), "no contacts are offered to the index");
        let _ = std::fs::remove_dir_all(&dir);
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
