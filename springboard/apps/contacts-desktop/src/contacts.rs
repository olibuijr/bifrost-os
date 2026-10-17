//! Contacts in the Golden Gate (Golden Gate) layout: an optional lists sidebar, the contact list with its search field, and
//! the card or the edit form in the detail pane, under a unified toolbar. Contacts are stored locally; phone, email
//! and address values copy to the shared clipboard. Geometry is in points; drawing converts with `ui.pt`.
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{search, Command, Commands, Node, Rect, Role, TextField};
use bifrost_app::glyphs::glyph;
use bifrost_app::kit::clipboard;
use bifrost_app::print::{self as print_service, Destination, PrintOperation};
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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use crate::archive;
use crate::details::{self, CardDetails, Detail, Details};
use crate::display_file::{self, Prefs};
use crate::dupes;
use crate::exchange;
use crate::extras::{self, Extra, Extras, ValueKind};
use crate::printing;
use crate::smart::{self, Condition, Rule, RuleField, UserList};
use crate::textimport;
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
/// Merge the duplicates of the open card, and merge every group of duplicates (Possible Duplicates list).
const BAR_MERGE: u32 = 17;
const BAR_MERGE_ALL: u32 = 18;
/// Smart list editor: edit the shown list. A new list is started from the File menu.
const BAR_EDIT_LIST: u32 = 23;
/// Selection mode (started from the Card menu): make a group of the chosen contacts, merge them, and leave it.
const BAR_GROUP_SELECTION: u32 = 20;
const BAR_MERGE_SELECTION: u32 = 21;
const BAR_SELECT_DONE: u32 = 22;
/// Ids of the app's main-menu items (File and View; the shell's Edit, Window and Help are not the app's).
const MENU_IMPORT: u32 = 1;
const MENU_EXPORT_V3: u32 = 2;
const MENU_EXPORT_V4: u32 = 3;
const MENU_SIDEBAR: u32 = 4;
const MENU_FIRST_LAST: u32 = 5;
const MENU_LAST_FIRST: u32 = 6;
const MENU_EXPORT_ARCHIVE: u32 = 7;
const MENU_RESTORE_ARCHIVE: u32 = 8;
const MENU_PRINT: u32 = 9;
const MENU_PRINT_LABELS: u32 = 10;
const MENU_EXPORT_PDF: u32 = 11;
/// File > Refresh Accounts: the CardDAV sync lives in `main.rs`, which answers this item.
pub const MENU_REFRESH: u32 = 12;
/// Select Contacts, New Group From Selection and Merge Selected Cards.
const MENU_SELECT: u32 = 13;
const MENU_GROUP_SELECTION: u32 = 14;
const MENU_MERGE_SELECTION: u32 = 15;
/// File > New Smart List and Edit Smart List.
const MENU_NEW_LIST: u32 = 16;
const MENU_EDIT_LIST: u32 = 17;
/// The Card menu.
const MENU_MY_CARD: u32 = 20;
const MENU_GO_MY_CARD: u32 = 21;
const MENU_COMPANY: u32 = 22;
const MENU_NEXT: u32 = 23;
const MENU_PREVIOUS: u32 = 24;
const MENU_MERGE: u32 = 25;
const MENU_DUPLICATES: u32 = 26;
const MENU_LAST_IMPORT: u32 = 27;
/// Card > Add Field: item `MENU_FIELD + n` adds the field `Detail::ALL[n]`.
const MENU_FIELD: u32 = 30;
/// Card > Open in Separate Window: the open card gets a window of its own (`take_card_request`).
const MENU_OPEN_WINDOW: u32 = 60;
/// Two presses on one row this close together (ms) open its card in a window.
const DOUBLE_CLICK_MS: f64 = 400.0;
/// Sidebar rows.
const LIST_ALL: u32 = 100;
const LIST_FAVORITES: u32 = 101;
const LIST_RECENT: u32 = 102;
const LIST_DUPLICATES: u32 = 103;
const LIST_LAST_IMPORT: u32 = 104;
/// Smart group `n` (its offset in `smart::Smart::ALL`) is the sidebar row `LIST_SMART + n`.
const LIST_SMART: u32 = 110;
const LIST_SMART_LAST: u32 = LIST_SMART + crate::smart::Smart::ALL.len() as u32 - 1;
/// Group `n` is the sidebar row `LIST_GROUP + n`; groups number from 1 and there are at most 1000 of them.
const LIST_GROUP: u32 = 1000;
/// The person's smart list `n` (its place in the saved lists) is the sidebar row `LIST_USER + n`.
const LIST_USER: u32 = 2100;
const LIST_USER_LAST: u32 = LIST_USER + smart::MAX_LISTS as u32 - 1;
const GROUPS_FILE: &str = "groups.txt";
const DISPLAY_FILE: &str = "display.txt";
/// Extra phone numbers, emails and addresses, beside the contacts file (see `extras.rs`).
const VALUES_FILE: &str = "values.txt";
/// Card fields and company flags, beside the contacts file (see `details.rs`).
const DETAILS_FILE: &str = "details.txt";
/// The Contacts archive written to the Export folder and read from the Import folder (see `archive.rs`).
const ARCHIVE_FILE: &str = "contacts.bcarchive";
/// The first line of a contacts file.
const CONTACTS_HEADER: &str = "springboard-contacts-v1";
/// The person's smart lists, beside the contacts file (see `group_file.rs`).
const SMART_FILE: &str = "smartlists.txt";
/// The data files of the Contacts folder besides the contacts file; the contacts archive holds them all.
const ARCHIVED_FILES: [&str; 5] = [VALUES_FILE, GROUPS_FILE, DISPLAY_FILE, DETAILS_FILE, SMART_FILE];

/// What the list shows. Every filter but `All` narrows the list to a set of contact identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Favorites,
    /// Contacts added or changed since the app started.
    Recent,
    /// Contacts that have a possible duplicate in the list.
    Duplicates,
    /// The contacts the last import added.
    LastImport,
    /// A built-in smart group: the contacts whose fields match its rule.
    Smart(crate::smart::Smart),
    /// One of the person's smart lists, by its place in the saved lists.
    UserList(usize),
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
            Filter::LastImport => LIST_LAST_IMPORT,
            Filter::Smart(s) => LIST_SMART + s.index() as u32,
            Filter::UserList(n) => LIST_USER + n as u32,
            Filter::Group(g) => LIST_GROUP + g as u32,
        }
    }

    /// The filter a sidebar row shows.
    fn from_sidebar_id(id: u32) -> Filter {
        match id {
            LIST_FAVORITES => Filter::Favorites,
            LIST_RECENT => Filter::Recent,
            LIST_DUPLICATES => Filter::Duplicates,
            LIST_LAST_IMPORT => Filter::LastImport,
            id if (LIST_SMART..=LIST_SMART_LAST).contains(&id) => match crate::smart::Smart::from_index((id - LIST_SMART) as usize) {
                Some(s) => Filter::Smart(s),
                None => Filter::All,
            },
            id if (LIST_USER..=LIST_USER_LAST).contains(&id) => Filter::UserList((id - LIST_USER) as usize),
            id if id > LIST_GROUP => Filter::Group((id - LIST_GROUP) as u64),
            _ => Filter::All,
        }
    }
}

/// Whether a control identifier names a sidebar row. The group rows sit in their own range below the list rows.
fn sidebar_node(id: u64) -> bool {
    (LIST_ALL as u64..=LIST_SMART_LAST as u64).contains(&id)
        || (LIST_GROUP as u64 + 1..=LIST_GROUP as u64 + 1000).contains(&id)
        || (LIST_USER as u64..=LIST_USER_LAST as u64).contains(&id)
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
/// The Mark as Company row of the edit form.
const NODE_COMPANY: u64 = 260;
/// The toggle rows of the Settings screen: `NODE_SETTING + n` for the n-th `Setting`.
const NODE_SETTING: u64 = 270;
/// The card field rows of the edit form: `NODE_DETAIL + n` for the field `Detail::ALL[n]`.
const NODE_DETAIL: u64 = 300;
/// Value rows of a card past the tenth (a card can hold more than ten with its extra values and card fields): row `k`
/// is the node `NODE_COPY_MORE + k`.
const NODE_COPY_MORE: u64 = 400;

/// The accessibility id of the copy control for value row `k` of a card.
fn copy_node(k: usize) -> u64 {
    if k < 10 {
        NODE_COPY + k as u64
    } else {
        NODE_COPY_MORE + k as u64
    }
}
/// The smart list editor: the name field, the match mode row, Add Rule and Delete List, then four controls per rule.
const NODE_SMART_NAME: u64 = 500;
const NODE_SMART_ANY: u64 = 501;
const NODE_SMART_ADD: u64 = 502;
const NODE_SMART_DELETE: u64 = 503;
const NODE_SMART_FIELD: u64 = 510;
const NODE_SMART_CONDITION: u64 = 520;
const NODE_SMART_VALUE: u64 = 530;
const NODE_SMART_REMOVE: u64 = 540;
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
/// Focus index of the first card field (Add Field): field `Detail::ALL[n]` has the index `DETAIL_BASE + n`.
const DETAIL_BASE: usize = 10;
const N_DETAILS: usize = Detail::ALL.len();
/// Focus indexes of the smart list editor: the name, then the value field of rule `n` at `SMART_VALUE + n`.
const SMART_NAME: usize = 30;
const SMART_VALUE: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// No contact selected.
    List,
    Detail(u64),
    Editor(Option<u64>),
    /// Display settings (the sort order of the list), in the detail pane.
    Settings,
    /// The smart list editor, in the detail pane: the list being edited, or `None` for a new one.
    SmartEditor(Option<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sidebar(u32),
    Bar(u32),
    Search,
    SearchClear,
    Row(usize),
    Copy(usize),
    /// A tap on a value row of the card: a phone calls, an email opens Mail, any other value copies.
    Use(usize),
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
    /// A switch row of the Settings screen.
    Setting(Setting),
    /// The Mark as Company row of the edit form.
    CompanyToggle,
    /// The smart list editor: the match mode row, the field and condition buttons and the remove button of a rule, Add
    /// Rule, and Delete List.
    SmartAny,
    SmartField(usize),
    SmartCondition(usize),
    SmartRemove(usize),
    SmartAdd,
    SmartDelete,
    List,
    Content,
}

/// The switch rows of the Settings screen, after the sort order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    /// Show a contact by its nickname when it has one.
    Nicknames,
    /// Write the notes when exporting vCards.
    ExportNotes,
    /// Show the card field `Detail::ALL[n]` in the form of a new card.
    Template(usize),
}

impl Setting {
    /// Every switch row, in screen order.
    fn all() -> Vec<Setting> {
        let mut rows = vec![Setting::Nicknames, Setting::ExportNotes];
        rows.extend((0..N_DETAILS).map(Setting::Template));
        rows
    }

    /// The row's title.
    fn title(self) -> String {
        match self {
            Setting::Nicknames => "Prefer Nicknames".into(),
            Setting::ExportNotes => "Include Notes in vCards".into(),
            Setting::Template(n) => format!("New Cards Show {}", Detail::ALL[n].title()),
        }
    }
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

/// The contacts the list shows for a search: those whose store fields match, those with an extra value that matches
/// and those with a card field (nickname, job title, ...) that matches, in the order of `contacts`. A blank query shows
/// every contact.
fn list_hits(contacts: &[Contact], extras: &Extras, card_details: &Details, query: &str) -> Vec<usize> {
    let by_field: BTreeSet<usize> = store::search(contacts, query).into_iter().collect();
    (0..contacts.len())
        .filter(|&i| by_field.contains(&i) || extras.matches(contacts[i].id, query) || card_details.matches(contacts[i].id, query))
        .collect()
}

/// A file name stem for a title: letters, digits, spaces, `-` and `_` are kept, anything else becomes `_`; at most 40
/// characters; `contact` when nothing is left.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' })
        .take(40)
        .collect();
    let stem = stem.trim().to_string();
    if stem.is_empty() || stem.chars().all(|c| c == '_') {
        "contact".into()
    } else {
        stem
    }
}

/// The list scroll offset that brings a row (its `top` in list coordinates, `height` tall) into a window `visible` tall
/// that is now scrolled to `offset`: unchanged when the row is in view, else the smallest move that shows it.
fn reveal_offset(top: f32, height: f32, offset: f32, visible: f32) -> f32 {
    if top < offset {
        // leave room for the section letter above the row
        (top - HEADER).max(0.0)
    } else if top + height > offset + visible {
        top + height - visible
    } else {
        offset
    }
}

/// Whether a file with this extension can be read by Import: a vCard, a table or an LDIF file.
fn importable(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("vcf") || textimport::reader_for(extension).is_some()
}

/// The value rows a contact's card shows: the phones, emails and addresses (`card_rows`), then the card fields.
fn rows_with_details(c: &Contact, extra: &[Extra], card_details: &CardDetails) -> Vec<contact_card::Row> {
    let mut rows = card_rows(c, extra);
    rows.extend(card_details.filled().into_iter().map(|(detail, value)| contact_card::Row {
        key: contact_card::Key::Company,
        label: detail.title().to_string(),
        value: value.to_string(),
        action: contact_card::Action::None,
    }));
    rows
}

/// The word an accessibility control names a card row with: `phone`, `email`, `address`, or the card field's title.
fn row_word(row: &contact_card::Row) -> String {
    match row.key {
        contact_card::Key::Company => row.label.to_lowercase(),
        key => value_word(key).to_string(),
    }
}

fn sort_label(order: SortOrder) -> &'static str {
    match order {
        SortOrder::UserDefault => "System Default",
        SortOrder::FamilyName => "Family Name",
        SortOrder::GivenName => "Given Name",
        SortOrder::None => "Keep Order",
    }
}

/// The given and family parts a contact sorts by. A company marked on its card sorts by its company name, which is one
/// part (so the family part repeats it). Other contacts split their display name at the last space.
fn sort_parts(c: &Contact, company: Option<&str>) -> (String, String) {
    let (given, family) = match company {
        Some(name) => (name, ""),
        None => names::split(c.display_name()),
    };
    if family.is_empty() {
        (given.to_string(), given.to_string())
    } else {
        (given.to_string(), family.to_string())
    }
}

/// Orders two contacts for the list. `Keep Order` keeps the identifier order. The family and given names are split
/// at the last space; `company_a` and `company_b` are the company names of contacts marked as companies, which sort by
/// that name. The collation is the app's own (see `names`), so accents and Icelandic letters sort as before.
fn compare_contacts(order: SortOrder, a: &Contact, b: &Contact, company_a: Option<&str>, company_b: Option<&str>) -> Ordering {
    if order == SortOrder::None {
        return a.id.cmp(&b.id);
    }
    let (ag, af) = sort_parts(a, company_a);
    let (bg, bf) = sort_parts(b, company_b);
    let by_given = names::key(&ag).cmp(&names::key(&bg)).then(names::key(&af).cmp(&names::key(&bf)));
    let by_family = names::key(&af).cmp(&names::key(&bf)).then(names::key(&ag).cmp(&names::key(&bg)));
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

/// What a tap on a phone or email value of a card hands to the shell: a `tel:` link, or a new message in Mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handoff {
    Call(String),
    Mail(String),
}

/// The `tel:` link for a phone value: spaces, dashes and brackets are dropped and a leading plus is kept.
pub fn tel_url(value: &str) -> String {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    format!("tel:{digits}")
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

/// The Settings screen, scroll applied, in window points.
struct SettingsLayout {
    title_y: f32,
    sort: Rect,
    switches: Vec<(Setting, Rect)>,
    /// The vertical middle of the New Cards heading above the template switches.
    heading_y: f32,
    height: f32,
}

/// One rule in the smart list editor: its field and condition, and the text of its value.
struct RuleDraft {
    field: RuleField,
    condition: Condition,
    value: TextField,
}

/// One rule row of the smart list editor, scroll applied, in window points.
struct SmartRow {
    field: Rect,
    condition: Rect,
    value: Rect,
    remove: Rect,
}

/// The smart list editor, scroll applied, in window points.
struct SmartLayout {
    title_y: f32,
    name: Rect,
    any: Rect,
    rules: Vec<SmartRow>,
    add: Option<Rect>,
    delete: Option<Rect>,
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
    /// The Mark as Company switch row.
    company: Rect,
    /// The card fields the form shows: (label, field) for each of `Detail::ALL`.
    details: [Option<(Rect, Rect)>; N_DETAILS],
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
    /// The edits of this session, which are added to `prefs.recent` as they are saved.
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
    /// The phone or email a card tap asked to hand to the shell; `main.rs` takes it with `take_handoff`.
    handoff: Option<Handoff>,
    /// The contact whose card asked for its own window (Card > Open in Separate Window, or a double-click on a row);
    /// `main.rs` takes it with `take_card_request`.
    card_req: Option<u64>,
    /// The last row press: a second press on the same contact within `DOUBLE_CLICK_MS` opens its card in a window.
    last_row_click: Option<(u64, f64)>,
    /// True while a card window draws or takes a touch (`draw_card`, `touch_card`): the detail pane fills the surface.
    card_window: bool,
    /// The size of the card window in points, from its last draw.
    card_size: (f32, f32),
    /// A card window's own scroller and press state: its scrolling leaves the main window's detail pane alone.
    card_scroll: Scroller,
    card_pressed: Option<Target>,
    card_scroll_list: Option<bool>,
    /// The Edit button of a card window was pressed down (its release edits the contact).
    card_edit_down: bool,
    /// The contact whose card window asked to edit it (taken once, `take_card_edit`).
    card_edit: Option<u64>,
    confirm_delete: bool,
    /// A failed load must not turn into an overwrite of unknown data.
    load_failed: bool,
    /// The extra phone numbers, emails and addresses of each contact (`values.txt`).
    extras: Extras,
    /// The card fields and company flag of each contact (`details.txt`).
    details: Details,
    /// My Card, the export options, the card template and the last import (saved in `display.txt`).
    prefs: Prefs,
    /// The card fields of the edit form, one per `Detail::ALL`, and which of them the form shows.
    detail_fields: [TextField; N_DETAILS],
    detail_shown: [bool; N_DETAILS],
    /// The edit form's Mark as Company switch.
    edit_company: bool,
    /// Restore from Archive was chosen once and replaces the address book on the second choice.
    confirm_restore: bool,
    /// Selection mode: a press on a row marks it instead of opening it. The list has no Shift or Command click, so
    /// several contacts are chosen this way.
    selecting: bool,
    selection: BTreeSet<u64>,
    /// The person's smart lists (`smartlists.txt`).
    lists: Vec<UserList>,
    /// The smart list editor: the list's name, whether any rule may match, and the rules.
    smart_name: TextField,
    smart_any: bool,
    smart_rules: Vec<RuleDraft>,
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
        let (display, name_order, prefs) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap_or_default();
        let (mut card_details, details_error) = match details::load_checked(&path.with_file_name(DETAILS_FILE)) {
            Ok(loaded) => (loaded, None),
            Err(e) => (Details::default(), Some(format!("Unable to load contact details: {e}"))),
        };
        card_details.retain(|id| live.contains(&id));
        let load_failed = load_failed || details_error.is_some();
        let error = error.or(details_error);
        let (lists, lists_error) = match group_file::load_lists(&path.with_file_name(SMART_FILE)) {
            Ok(lists) => (lists, None),
            Err(e) => (Vec::new(), Some(format!("Unable to load smart lists: {e}"))),
        };
        let load_failed = load_failed || lists_error.is_some();
        let error = error.or(lists_error);
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
            handoff: None,
            card_req: None,
            last_row_click: None,
            card_window: false,
            card_size: (480.0, 560.0),
            card_scroll: Scroller::new(),
            card_pressed: None,
            card_scroll_list: None,
            card_edit_down: false,
            card_edit: None,
            confirm_delete: false,
            load_failed,
            extras,
            details: card_details,
            prefs,
            detail_fields: Default::default(),
            detail_shown: [false; N_DETAILS],
            edit_company: false,
            confirm_restore: false,
            selecting: false,
            selection: BTreeSet::new(),
            lists,
            smart_name: TextField::new(),
            smart_any: false,
            smart_rules: Vec::new(),
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

    /// True while a form is open, the contact editor or the smart list editor: its edits are not saved yet.
    pub fn editing(&self) -> bool {
        matches!(self.screen, Screen::Editor(_) | Screen::SmartEditor(_))
    }

    /// True while the contact editor is open.
    fn editing_contact(&self) -> bool {
        matches!(self.screen, Screen::Editor(_))
    }

    pub fn selected(&self) -> Option<&Contact> {
        if let Screen::Detail(id) = self.screen {
            self.contacts.iter().find(|c| c.id == id)
        } else {
            None
        }
    }
    /// The value rows of a contact's card: phones, emails and addresses, then its card fields.
    fn rows_of(&self, c: &Contact) -> Vec<contact_card::Row> {
        rows_with_details(c, self.extras.get(c.id), &self.details.of(c.id))
    }

    /// The title the list and the card show for a contact (company, nickname or name; see `names::title`).
    fn title_of(&self, c: &Contact) -> String {
        names::title(
            c.display_name(),
            &c.company,
            self.details.nickname(c.id),
            self.details.is_company(c.id),
            self.prefs.prefer_nicknames,
            self.name_order,
        )
    }

    #[cfg_attr(not(test), allow(dead_code))] // the tests read the state dump through this
    pub fn describe(&self) -> String {
        let screen = match self.screen {
            Screen::List => "list".into(),
            Screen::Detail(id) => format!("detail:{id}"),
            Screen::Editor(id) => format!("editor:{}", id.map_or_else(|| "new".into(), |id| id.to_string())),
            Screen::Settings => "settings".into(),
            Screen::SmartEditor(n) => format!("smart-editor:{}", n.map_or_else(|| "new".into(), |n| n.to_string())),
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

    /// The contact whose card asked for its own window, with its name for the window title (taken once).
    pub fn take_card_request(&mut self) -> Option<(u64, String)> {
        let id = self.card_req.take()?;
        let name = self.contacts.iter().find(|c| c.id == id).map(|c| self.title_of(c)).unwrap_or_default();
        Some((id, name))
    }

    /// Card > Open in Separate Window: asks for the open card's window. False when no card is open.
    fn ask_card_window(&mut self) -> bool {
        match self.screen {
            Screen::Detail(id) => {
                self.card_req = Some(id);
                true
            }
            _ => false,
        }
    }

    /// One contact's card drawn at the size of its own window (`s`, px): the detail pane fills the surface, with no list
    /// or sidebar beside it. The main window's screen and layout are restored afterwards.
    pub fn draw_card(&mut self, s: &mut Surface, ui: &Ui, id: u64) {
        let saved = (self.w, self.h, self.screen, self.card_window);
        self.w = s.w as f32 / ui.s;
        self.h = s.h as f32 / ui.s;
        self.card_size = (self.w, self.h);
        self.screen = Screen::Detail(id);
        self.card_window = true;
        std::mem::swap(&mut self.scroll, &mut self.card_scroll);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        self.draw_detail(s, ui, 0.0);
        let edit = self.card_edit_rect();
        self.text(s, ui, "Edit", MAC_BODY_PT, Weight::Regular, LINK, edit.x + SPACE_S, edit.y + edit.h * 0.3);
        std::mem::swap(&mut self.scroll, &mut self.card_scroll);
        (self.w, self.h, self.screen, self.card_window) = saved;
    }

    /// The Edit button at the top right of a card window, in points.
    fn card_edit_rect(&self) -> Rect {
        rect(self.card_size.0 - 72.0, 12.0, 60.0, 28.0)
    }

    /// A touch in a card window showing contact `id`. The card scrolls on its own scroller; a tap on a value copies it, as
    /// in the main card; Delete is not offered here; Edit asks the main window to edit the contact (`take_card_edit`).
    /// The main window's screen, scroll and press state are left as they were.
    pub fn touch_card(&mut self, ui: &Ui, id: u64, ev: Event, t: f64) {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x / ui.s, y / ui.s),
            _ => return,
        };
        let edit = self.card_edit_rect();
        match ev {
            Event::Down(..) => {
                self.card_edit_down = edit.contains(x, y);
                if self.card_edit_down {
                    return;
                }
            }
            Event::Up(..) => {
                if std::mem::take(&mut self.card_edit_down) {
                    if edit.contains(x, y) {
                        self.card_edit = Some(id);
                    }
                    return;
                }
            }
            _ => {}
        }
        // a card has no toolbar: the top band is the card's own area, not the main window's bar buttons
        if y < UNIFIED_BAR_PT {
            return;
        }
        let saved = (self.w, self.h, self.screen, self.card_window, self.sidebar_shown);
        self.w = self.card_size.0;
        self.h = self.card_size.1;
        self.screen = Screen::Detail(id);
        self.card_window = true;
        self.sidebar_shown = false;
        std::mem::swap(&mut self.scroll, &mut self.card_scroll);
        std::mem::swap(&mut self.pressed, &mut self.card_pressed);
        std::mem::swap(&mut self.scroll_list, &mut self.card_scroll_list);
        let blocked = matches!(self.hit(ui, x, y), Some(Target::Delete | Target::ConfirmDelete | Target::CancelDelete));
        if !blocked {
            self.touch(ui, ev, t);
        }
        // the share sheet is a shell feature, as in the main window
        let _ = self.take_share();
        std::mem::swap(&mut self.scroll, &mut self.card_scroll);
        std::mem::swap(&mut self.pressed, &mut self.card_pressed);
        std::mem::swap(&mut self.scroll_list, &mut self.card_scroll_list);
        (self.w, self.h, self.screen, self.card_window, self.sidebar_shown) = saved;
    }

    /// The contact whose card window asked to be edited, once.
    pub fn take_card_edit(&mut self) -> Option<u64> {
        self.card_edit.take()
    }

    /// Card window > Edit: the main window shows the contact and opens its editor.
    pub fn edit_from_card(&mut self, ui: &Ui, id: u64) {
        if self.contacts.iter().any(|c| c.id == id) {
            self.show_card(ui, id);
            self.edit_contact(ui, Some(id));
        }
    }

    /// The contact the user asked to copy to the pasteboard, if any (taken once).
    pub fn take_copy(&mut self) -> Option<(Contact, Vec<Extra>)> {
        self.copy_out.take()
    }

    /// The phone or email a card tap asked to hand to the shell, if any (taken once).
    pub fn take_handoff(&mut self) -> Option<Handoff> {
        self.handoff.take()
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
        if self.card_window {
            return 0.0;
        }
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
        if self.selecting {
            items.push(ToolbarItem::flexible_space());
            items.push(ToolbarItem::button(BAR_GROUP_SELECTION, "Group Selection").enabled(groups_ok && !self.selection.is_empty()));
            items.push(ToolbarItem::button(BAR_MERGE_SELECTION, "Merge").enabled(groups_ok && self.selection.len() >= 2));
            items.push(ToolbarItem::button(BAR_SELECT_DONE, "Done"));
            return Toolbar::new(items);
        }
        items.push(ToolbarItem::flexible_space());
        if let Filter::Group(_) = self.filter {
            items.push(ToolbarItem::button(BAR_ADD_PEOPLE, "Add People").enabled(groups_ok));
            items.push(ToolbarItem::button(BAR_RENAME, "Rename Group").enabled(groups_ok));
            items.push(ToolbarItem::button(BAR_DELETE_GROUP, "Delete Group").enabled(groups_ok));
        }
        if self.filter == Filter::Duplicates {
            items.push(ToolbarItem::button(BAR_MERGE_ALL, "Merge All").enabled(groups_ok));
        }
        if matches!(self.filter, Filter::UserList(_)) {
            items.push(ToolbarItem::button(BAR_EDIT_LIST, "Edit List").enabled(!self.load_failed));
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
                if self.filter == Filter::Duplicates {
                    items.push(ToolbarItem::button(BAR_MERGE, "Merge Duplicates").enabled(!self.load_failed && !self.groups_failed));
                }
                items.push(ToolbarItem::button(BAR_EDIT, "Edit").enabled(!self.load_failed));
            }
            Screen::Editor(_) | Screen::SmartEditor(_) => {
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
        ];
        if !self.last_import_ids().is_empty() {
            rows.push((LIST_LAST_IMPORT, "Last Import".to_string(), "arrow.down.circle"));
        }
        rows.push((0, "Smart Groups".to_string(), ""));
        for s in crate::smart::Smart::ALL {
            rows.push((LIST_SMART + s.index() as u32, s.title().to_string(), s.symbol()));
        }
        for (n, list) in self.lists.iter().enumerate() {
            rows.push((LIST_USER + n as u32, list.name.clone(), "list.bullet"));
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
            for k in 0..self.rows_of(c).len() {
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
        let company = rect(x, y, cw, FIELD_ROW);
        y += step;
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
        let mut details = [None; N_DETAILS];
        let label_w = 120.0;
        for (k, slot) in details.iter_mut().enumerate() {
            if self.detail_shown[k] {
                *slot = Some((rect(x, y, label_w, FIELD_ROW), rect(x + label_w, y, cw - label_w, FIELD_ROW)));
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
        EditLayout { avatar, fields, labels, adds, company, details, notes_card, delete, cancel, height: y + off + SPACE_XL }
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        self.list_scroll.set_max(self.list_height + SPACE_L - (self.list_bottom() - self.list_top()));
        let height = match self.screen {
            Screen::List => 0.0,
            Screen::Detail(_) => self.card_layout(ui).height,
            Screen::Editor(_) => self.edit_layout(ui).height,
            Screen::Settings => self.settings_layout().height,
            Screen::SmartEditor(_) => self.smart_layout().height,
        };
        self.scroll.set_max(height - self.h);
    }

    /// The Settings screen, scroll applied, in window points.
    fn settings_layout(&self) -> SettingsLayout {
        let (x, cw) = self.column();
        let off = self.scroll.offset();
        let title_y = UNIFIED_BAR_PT + SPACE_XL - off;
        let mut y = UNIFIED_BAR_PT + 3.0 * SPACE_XL - off;
        let sort = rect(x, y, cw, ROW);
        y += ROW + 2.0 * SPACE_M;
        let mut switches = Vec::new();
        let mut heading_y = y;
        for setting in Setting::all() {
            if setting == Setting::Template(0) {
                y += SPACE_M;
                heading_y = y;
                y += SPACE_XL;
            }
            switches.push((setting, rect(x, y, cw, ROW)));
            y += ROW + SPACE_XS;
        }
        SettingsLayout { title_y, sort, switches, heading_y, height: y + off + SPACE_XL }
    }

    /// Whether a switch row of the Settings screen is on.
    fn setting_on(&self, setting: Setting) -> bool {
        match setting {
            Setting::Nicknames => self.prefs.prefer_nicknames,
            Setting::ExportNotes => self.prefs.export_notes,
            Setting::Template(n) => self.prefs.template.contains(&Detail::ALL[n]),
        }
    }

    /// The smart list editor, scroll applied, in window points.
    fn smart_layout(&self) -> SmartLayout {
        let (x, cw) = self.column();
        let off = self.scroll.offset();
        let title_y = UNIFIED_BAR_PT + SPACE_XL - off;
        let mut y = UNIFIED_BAR_PT + 3.0 * SPACE_XL - off;
        let name = rect(x, y, cw, FIELD_ROW);
        y += FIELD_ROW + SPACE_S;
        let any = rect(x, y, cw, FIELD_ROW);
        y += FIELD_ROW + SPACE_M;
        let gap = SPACE_XS;
        let field_w = (cw * 0.2).clamp(70.0, 110.0);
        let condition_w = (cw * 0.28).clamp(100.0, 150.0);
        let remove_w = FIELD_ROW;
        let mut rules = Vec::new();
        for _ in &self.smart_rules {
            let value_x = x + field_w + condition_w + 2.0 * gap;
            let value_w = (x + cw - remove_w - gap - value_x).max(40.0);
            rules.push(SmartRow {
                field: rect(x, y, field_w, FIELD_ROW),
                condition: rect(x + field_w + gap, y, condition_w, FIELD_ROW),
                value: rect(value_x, y, value_w, FIELD_ROW),
                remove: rect(x + cw - remove_w, y, remove_w, FIELD_ROW),
            });
            y += FIELD_ROW + SPACE_S;
        }
        let add = (self.smart_rules.len() < smart::MAX_RULES).then(|| rect(x, y, cw, FIELD_ROW));
        if add.is_some() {
            y += FIELD_ROW + SPACE_S;
        }
        y += SPACE_M;
        let (mut delete, mut cancel) = (None, None);
        if matches!(self.screen, Screen::SmartEditor(Some(_))) {
            delete = Some(rect(x, y, cw, BUTTON_ROW));
            y += BUTTON_ROW + m::CARD_GAP;
            if self.confirm_delete {
                cancel = Some(rect(x, y, cw, BUTTON_ROW));
                y += BUTTON_ROW + m::CARD_GAP;
            }
        }
        SmartLayout { title_y, name, any, rules, add, delete, cancel, height: y + off + SPACE_XL }
    }

    /// The smart list editor's text fields in the order Tab visits them: the name, then the value of each rule that
    /// compares with one.
    fn smart_focus_order(&self) -> Vec<usize> {
        let mut order = vec![SMART_NAME];
        order.extend(self.smart_rules.iter().enumerate().filter(|(_, r)| r.condition.needs_value()).map(|(n, _)| SMART_VALUE + n));
        order
    }

    /// File > New Smart List: opens the editor on an empty list with one rule.
    fn new_list(&mut self, ui: &Ui) {
        if self.load_failed || self.editing() || self.renaming.is_some() || self.picking.is_some() {
            return;
        }
        if self.lists.len() >= smart::MAX_LISTS {
            self.status = Some(format!("A person can make at most {} smart lists", smart::MAX_LISTS));
            self.dirty = true;
            return;
        }
        self.smart_name.clear();
        self.smart_any = false;
        self.smart_rules = vec![RuleDraft { field: RuleField::Name, condition: Condition::Contains, value: TextField::new() }];
        self.open_smart_editor(ui, None);
    }

    /// File > Edit Smart List: opens the editor on the smart list the sidebar shows.
    fn edit_list(&mut self, ui: &Ui) {
        let Filter::UserList(n) = self.filter else { return };
        if self.load_failed || self.editing() || self.renaming.is_some() || self.picking.is_some() {
            return;
        }
        let Some(list) = self.lists.get(n) else { return };
        self.smart_name.clear();
        self.smart_name.push_str(&list.name);
        self.smart_any = list.any;
        self.smart_rules = list
            .rules
            .iter()
            .map(|rule| {
                let mut value = TextField::new();
                value.push_str(&rule.value);
                RuleDraft { field: rule.field, condition: rule.condition, value }
            })
            .collect();
        self.open_smart_editor(ui, Some(n));
    }

    fn open_smart_editor(&mut self, ui: &Ui, which: Option<usize>) {
        self.screen = Screen::SmartEditor(which);
        self.error = None;
        self.status = None;
        self.confirm_delete = false;
        self.scroll.jump_top();
        self.fit_scroll(ui);
        self.focus_field(SMART_NAME);
    }

    /// Done in the smart list editor: checks the list, saves the lists file and shows the list.
    fn save_smart(&mut self, ui: &Ui) {
        let Screen::SmartEditor(which) = self.screen else { return };
        if self.load_failed {
            return;
        }
        let list = UserList {
            name: self.smart_name.trim().to_string(),
            any: self.smart_any,
            rules: self
                .smart_rules
                .iter()
                .map(|r| Rule { field: r.field, condition: r.condition, value: if r.condition.needs_value() { r.value.trim().to_string() } else { String::new() } })
                .collect(),
        };
        if !list.valid() {
            self.error = Some("A smart list needs a name, and each rule that compares needs a value".into());
            self.dirty = true;
            return;
        }
        let mut lists = self.lists.clone();
        let at = match which {
            Some(n) if n < lists.len() => {
                lists[n] = list;
                n
            }
            _ => {
                lists.push(list);
                lists.len() - 1
            }
        };
        match group_file::save_lists(&self.path.with_file_name(SMART_FILE), &lists) {
            Ok(()) => {
                self.lists = lists;
                self.error = None;
                self.filter = Filter::UserList(at);
                self.sidebar_shown = true;
                self.list_scroll.jump_top();
                self.close_detail(ui);
                self.rebuild(ui);
            }
            Err(e) => {
                self.error = Some(format!("Unable to save the smart list: {e}"));
                self.dirty = true;
            }
        }
    }

    /// Deletes the smart list the editor is on. The first press asks, the second deletes.
    fn delete_list(&mut self, ui: &Ui) {
        let Screen::SmartEditor(Some(n)) = self.screen else { return };
        if self.load_failed || n >= self.lists.len() {
            return;
        }
        if !self.confirm_delete {
            self.confirm_delete = true;
            self.fit_scroll(ui);
            return;
        }
        let mut lists = self.lists.clone();
        lists.remove(n);
        match group_file::save_lists(&self.path.with_file_name(SMART_FILE), &lists) {
            Ok(()) => {
                self.lists = lists;
                self.error = None;
                self.filter = Filter::All;
                self.list_scroll.jump_top();
                self.close_detail(ui);
                self.rebuild(ui);
            }
            Err(e) => {
                self.error = Some(format!("Unable to delete the smart list: {e}"));
                self.dirty = true;
            }
        }
    }

    /// Adds an empty rule to the smart list editor.
    fn add_rule(&mut self, ui: &Ui) {
        if self.smart_rules.len() >= smart::MAX_RULES {
            return;
        }
        self.smart_rules.push(RuleDraft { field: RuleField::Name, condition: Condition::Contains, value: TextField::new() });
        self.fit_scroll(ui);
        self.focus_field(SMART_VALUE + self.smart_rules.len() - 1);
    }

    /// Takes rule `n` out of the smart list editor. A list keeps at least one rule.
    fn remove_rule(&mut self, ui: &Ui, n: usize) {
        if self.smart_rules.len() <= 1 || n >= self.smart_rules.len() {
            return;
        }
        self.smart_rules.remove(n);
        self.focus = None;
        self.fit_scroll(ui);
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
            Filter::Recent => Some(self.recent_ids()),
            Filter::Duplicates => Some(dupes::duplicate_ids(&self.contacts)),
            Filter::LastImport => Some(self.last_import_ids()),
            Filter::UserList(n) => Some(self.lists.get(n).map(|list| list.ids(&self.contacts, &self.extras)).unwrap_or_default()),
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
        if self.filter == Filter::LastImport && self.last_import_ids().is_empty() {
            self.filter = Filter::All;
        }
        let order = self.display.effective_sort_order(SortOrder::FamilyName);
        // Contacts marked as companies sort and section by their company name.
        let companies: BTreeMap<u64, String> = self
            .contacts
            .iter()
            .filter(|c| self.details.is_company(c.id) && !c.company.trim().is_empty())
            .map(|c| (c.id, c.company.trim().to_string()))
            .collect();
        self.contacts.sort_by(|a, b| compare_contacts(order, a, b, companies.get(&a.id).map(String::as_str), companies.get(&b.id).map(String::as_str)));
        self.rows.clear();
        let mut top = 0.0;
        let mut previous = None;
        let shown = self.filter_ids();
        for i in list_hits(&self.contacts, &self.extras, &self.details, &self.query) {
            if shown.as_ref().is_some_and(|ids| !ids.contains(&self.contacts[i].id)) {
                continue;
            }
            let name = companies.get(&self.contacts[i].id).map_or(self.contacts[i].display_name(), String::as_str);
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
        let own = id.map(|id| self.details.of(id)).unwrap_or_default();
        self.edit_company = own.company;
        for (k, detail) in Detail::ALL.into_iter().enumerate() {
            self.detail_fields[k].clear();
            self.detail_fields[k].push_str(own.get(detail));
            // a new card shows the fields of the card template; an old one shows the fields it has
            self.detail_shown[k] = !own.get(detail).is_empty() || (id.is_none() && self.prefs.template.contains(&detail));
        }
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
                self.save_card_details(id);
                self.rebuild(ui);
                self.open_detail(ui, id);
            }
            Err(e) => {
                self.error = Some(format!("Unable to save: {e}"));
                self.dirty = true;
            }
        }
    }

    /// Stores the edit form's card fields and company flag for contact `id`, in memory and in `details.txt`.
    fn save_card_details(&mut self, id: u64) {
        let mut card = CardDetails::default();
        card.company = self.edit_company;
        for (k, detail) in Detail::ALL.into_iter().enumerate() {
            if !self.detail_shown[k] {
                continue;
            }
            let typed = if detail == Detail::Birthday {
                details::normalize_birthday(&self.detail_fields[k])
            } else {
                self.detail_fields[k].to_string()
            };
            card.set(detail, &typed);
        }
        let mut updated = self.details.clone();
        updated.set(id, card);
        if updated != self.details {
            self.details = updated;
            self.write_details();
        }
    }

    /// Writes `details.txt`; a failure is shown as an error.
    fn write_details(&mut self) {
        if let Err(e) = details::save(&self.path.with_file_name(DETAILS_FILE), &self.details) {
            self.error = Some(format!("Unable to save contact details: {e}"));
        }
    }

    /// Writes `values.txt`; a failure is shown as an error.
    fn write_extras(&mut self) {
        if let Err(e) = extras::save(&self.values_path(), &self.extras) {
            self.error = Some(format!("Unable to save contact values: {e}"));
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
                    self.write_extras();
                }
                if self.details.remove(id) {
                    self.write_details();
                }
                self.forget_in_prefs(&[id]);
                self.selection.remove(&id);
                self.close_detail(ui);
                self.rebuild(ui);
            }
            Err(e) => {
                self.error = Some(format!("Unable to delete: {e}"));
                self.dirty = true;
            }
        }
    }

    /// The contacts of the last import that still exist.
    fn last_import_ids(&self) -> BTreeSet<u64> {
        let live: BTreeSet<u64> = self.contacts.iter().map(|c| c.id).collect();
        self.prefs.last_import.iter().copied().filter(|id| live.contains(id)).collect()
    }

    /// The contacts that an edit changed, in this run or an earlier one, that still exist.
    fn recent_ids(&self) -> BTreeSet<u64> {
        let live: BTreeSet<u64> = self.contacts.iter().map(|c| c.id).collect();
        self.prefs.recent.iter().copied().filter(|id| live.contains(id)).collect()
    }

    /// Forgets deleted contacts in My Card, the last import and the recent changes, and saves the preferences when
    /// that changed them.
    fn forget_in_prefs(&mut self, ids: &[u64]) {
        let mut changed = false;
        if self.prefs.me.is_some_and(|me| ids.contains(&me)) {
            self.prefs.me = None;
            changed = true;
        }
        for id in ids {
            changed |= self.prefs.last_import.remove(id);
            changed |= self.prefs.recent.remove(id);
        }
        if changed {
            self.save_display();
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

    /// Records a saved change in the history and in the Recently Changed list, which is saved with the display settings.
    /// The file is already written, so a history error is not shown.
    fn record(&mut self, before: &[Contact]) {
        let _ = self.history.record(before, &self.contacts);
        let known = self.prefs.recent.len();
        self.prefs.recent.extend(self.history.changed_this_session());
        if self.prefs.recent.len() != known {
            self.save_display();
        }
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

    /// Flips a switch of the Settings screen and saves it.
    fn toggle_setting(&mut self, setting: Setting) {
        match setting {
            Setting::Nicknames => self.prefs.prefer_nicknames = !self.prefs.prefer_nicknames,
            Setting::ExportNotes => self.prefs.export_notes = !self.prefs.export_notes,
            Setting::Template(n) => {
                let detail = Detail::ALL[n];
                if !self.prefs.template.remove(&detail) {
                    self.prefs.template.insert(detail);
                }
            }
        }
        self.save_display();
        self.dirty = true;
    }

    /// Saves the sort order, the name order and the preferences beside the contacts file.
    fn save_display(&mut self) {
        if let Err(e) = display_file::save(&self.path.with_file_name(DISPLAY_FILE), &self.display, self.name_order, &self.prefs) {
            self.error = Some(format!("Unable to save the settings: {e}"));
        }
    }

    /// Shows names first name first or last name first, saves the choice and redraws the list and the card.
    fn set_name_order(&mut self, order: names::NameOrder) {
        self.name_order = order;
        self.save_display();
        self.dirty = true;
    }

    /// The app's main menu: File holds the imports, exports and printing, View the sidebar and the name order, and Card the
    /// commands on the open card. File and View merge into the shell's menus of those titles.
    pub fn main_menu(&self) -> Vec<AppMenu> {
        let ready = !self.load_failed;
        let groups_ok = ready && !self.groups_failed;
        let sidebar = if self.sidebar_shown { "Hide Sidebar" } else { "Show Sidebar" };
        let open = matches!(self.screen, Screen::Detail(_));
        let on_card = open || self.editing_contact();
        let company = match self.screen {
            Screen::Detail(id) => self.details.is_company(id),
            Screen::Editor(_) => self.edit_company,
            _ => false,
        };
        let mut card = vec![
            AppMenuItem::new(MENU_MY_CARD, "Make This My Card").enabled(ready && open),
            AppMenuItem::new(MENU_GO_MY_CARD, "Go to My Card").enabled(self.prefs.me.is_some()),
            AppMenuItem::new(MENU_OPEN_WINDOW, "Open in Separate Window").enabled(ready && open),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_COMPANY, "Mark as a Company").checked(company).enabled(ready && on_card),
            AppMenuItem::submenu("Add Field"),
        ];
        for (n, detail) in Detail::ALL.into_iter().enumerate() {
            card.push(AppMenuItem::new(MENU_FIELD + n as u32, detail.title()).depth(1).enabled(ready && on_card));
        }
        card.extend([
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_NEXT, "Go to Next Card").key("cmd+]").enabled(!self.rows.is_empty() && !self.editing()),
            AppMenuItem::new(MENU_PREVIOUS, "Go to Previous Card").key("cmd+[").enabled(!self.rows.is_empty() && !self.editing()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_DUPLICATES, "Look for Duplicates"),
            AppMenuItem::new(MENU_MERGE, "Merge Duplicates").enabled(groups_ok && open && self.filter == Filter::Duplicates),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_SELECT, "Select Contacts").enabled(ready && !self.editing() && !self.contacts.is_empty()),
            AppMenuItem::new(MENU_GROUP_SELECTION, "New Group From Selection").enabled(groups_ok && self.selecting && !self.selection.is_empty()),
            AppMenuItem::new(MENU_MERGE_SELECTION, "Merge Selected Cards").enabled(groups_ok && self.selecting && self.selection.len() >= 2),
        ]);
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_IMPORT, "Import Contacts...").enabled(ready),
                    AppMenuItem::new(MENU_RESTORE_ARCHIVE, "Restore from Archive...").enabled(ready),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_NEW_LIST, "New Smart List...").enabled(ready),
                    AppMenuItem::new(MENU_EDIT_LIST, "Edit Smart List...").enabled(ready && matches!(self.filter, Filter::UserList(_))),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_EXPORT_V3, "Export vCard 3.0...").enabled(ready),
                    AppMenuItem::new(MENU_EXPORT_V4, "Export vCard 4.0...").enabled(ready),
                    AppMenuItem::new(MENU_EXPORT_ARCHIVE, "Export Contacts Archive...").enabled(ready),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(ready),
                    AppMenuItem::new(MENU_PRINT_LABELS, "Print Address Labels...").enabled(ready),
                    AppMenuItem::new(MENU_EXPORT_PDF, "Export as PDF...").enabled(ready),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_REFRESH, "Refresh Accounts"),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_SIDEBAR, sidebar),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIRST_LAST, "First Name First").checked(self.name_order == names::NameOrder::FirstLast),
                    AppMenuItem::new(MENU_LAST_FIRST, "Last Name First").checked(self.name_order == names::NameOrder::LastFirst),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_LAST_IMPORT, "Show Last Import").enabled(!self.last_import_ids().is_empty()),
                ],
            },
            AppMenu { title: "Card".into(), items: card },
        ]
    }

    /// Runs a main-menu item. True when the screen changed and needs a frame.
    pub fn menu_action(&mut self, ui: &Ui, id: u32) -> bool {
        // a restore only goes ahead when its menu item is chosen twice in a row
        if id != MENU_RESTORE_ARCHIVE {
            self.confirm_restore = false;
        }
        match id {
            MENU_IMPORT => self.import_vcards(ui),
            MENU_RESTORE_ARCHIVE => self.restore_archive(ui),
            MENU_EXPORT_V3 => self.export_vcards(exchange::Version::V3),
            MENU_EXPORT_V4 => self.export_vcards(exchange::Version::V4),
            MENU_EXPORT_ARCHIVE => self.export_archive(),
            MENU_PRINT => self.print_contacts(),
            MENU_PRINT_LABELS => self.print_labels(),
            MENU_EXPORT_PDF => self.export_pdf(),
            MENU_SIDEBAR => self.bar(ui, BAR_SIDEBAR),
            MENU_FIRST_LAST => self.set_name_order(names::NameOrder::FirstLast),
            MENU_LAST_FIRST => self.set_name_order(names::NameOrder::LastFirst),
            MENU_LAST_IMPORT => self.show_list(ui, Filter::LastImport),
            MENU_DUPLICATES => self.show_list(ui, Filter::Duplicates),
            MENU_OPEN_WINDOW => self.ask_card_window(),
            MENU_MY_CARD => self.make_my_card(),
            MENU_GO_MY_CARD => self.go_to_my_card(ui),
            MENU_COMPANY => self.toggle_company(ui),
            MENU_NEXT => self.step_card(ui, true),
            MENU_PREVIOUS => self.step_card(ui, false),
            MENU_MERGE => self.merge_duplicates(ui, false),
            MENU_NEW_LIST => self.new_list(ui),
            MENU_EDIT_LIST => self.edit_list(ui),
            MENU_SELECT => self.start_selecting(ui),
            MENU_GROUP_SELECTION => self.group_selection(ui),
            MENU_MERGE_SELECTION => self.merge_selection(ui),
            id if (MENU_FIELD..MENU_FIELD + N_DETAILS as u32).contains(&id) => self.add_field(ui, Detail::ALL[(id - MENU_FIELD) as usize]),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// Shows a list of the sidebar (and closes the card or the editor).
    fn show_list(&mut self, ui: &Ui, filter: Filter) {
        if self.editing() || self.renaming.is_some() || self.picking.is_some() {
            return;
        }
        self.sidebar_shown = true;
        self.filter = filter;
        self.status = None;
        self.query.clear();
        self.list_scroll.jump_top();
        self.close_detail(ui);
        self.rebuild(ui);
    }

    /// Card > Select Contacts: a press on a row marks it instead of opening it. The list has no Shift or Command click,
    /// so several contacts are chosen this way.
    fn start_selecting(&mut self, ui: &Ui) {
        if self.load_failed || self.editing() || self.renaming.is_some() || self.picking.is_some() {
            return;
        }
        self.selecting = true;
        self.selection.clear();
        self.close_detail(ui);
        self.status = Some("Click the contacts to choose, then Group Selection or Merge".into());
        self.dirty = true;
    }

    /// Leaves selection mode and forgets the choice.
    fn stop_selecting(&mut self) {
        self.selecting = false;
        self.selection.clear();
        self.status = None;
        self.dirty = true;
    }

    /// Card > New Group From Selection: makes a group of the chosen contacts and shows it.
    fn group_selection(&mut self, ui: &Ui) {
        if self.load_failed || self.groups_failed || self.selection.is_empty() {
            return;
        }
        let name = format!("Group {}", self.groups.len() + 1);
        match self.groups.add(&name, None) {
            Ok(id) => {
                for contact in &self.selection {
                    let _ = self.groups.add_member(id, *contact);
                }
                let count = self.selection.len();
                self.group_ids.push(id);
                self.stop_selecting();
                self.error = None;
                self.filter = Filter::Group(id);
                self.save_groups();
                self.list_scroll.jump_top();
                self.rebuild(ui);
                self.status = Some(format!("Made {name} from {count} contacts"));
            }
            Err(e) => self.error = Some(format!("Unable to add a group: {e:?}")),
        }
    }

    /// Card > Merge Selected Cards: folds the chosen contacts into the first of them (the lowest identifier).
    fn merge_selection(&mut self, ui: &Ui) {
        if self.load_failed || self.groups_failed {
            return;
        }
        let ids: Vec<u64> = self.selection.iter().copied().filter(|id| self.contacts.iter().any(|c| c.id == *id)).collect();
        if ids.len() < 2 {
            self.status = Some("Choose at least two contacts to merge".into());
            self.dirty = true;
            return;
        }
        self.stop_selecting();
        self.merge_groups(ui, &[ids], None);
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

    /// Reads every vCard (`.vcf`), table (`.csv`, `.tsv`, `.txt`) and LDIF (`.ldif`, `.ldf`) file in the Import folder. A
    /// card that matches a contact already in the list is skipped. The contacts a file adds make up the Last Import list.
    fn import_vcards(&mut self, ui: &Ui) {
        if self.load_failed {
            return;
        }
        let folder = self.folder("Import");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.extension().and_then(|ext| ext.to_str()).is_some_and(importable))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if files.is_empty() {
            self.error = None;
            self.status = Some("Put vCard (.vcf), CSV or LDIF files in the Contacts Import folder, then choose Import".into());
            self.dirty = true;
            return;
        }
        let mut reads = Vec::new();
        let mut notes = Vec::new();
        for file in &files {
            let extension = file.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let text = std::fs::read(file).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            match (text, textimport::reader_for(extension)) {
                (Some(text), Some(reader)) => match reader(&text) {
                    Ok(done) => {
                        notes.push(format!("{name}: {}", done.mapping_summary()));
                        reads.push(Some(done.import));
                    }
                    Err(e) => {
                        notes.push(format!("{name}: {e}"));
                        reads.push(None);
                    }
                },
                (Some(text), None) => reads.push(exchange::import(&text).ok()),
                (None, _) => reads.push(None),
            }
        }
        let _ = self.merge_imports(ui, reads, &notes, true);
    }

    /// Merges vCard texts into the contacts (a card that duplicates one is skipped), saves them and returns the summary
    /// the status line shows. `None` is a text that could not be read. The CardDAV sync uses it.
    pub fn merge_card_texts(&mut self, ui: &Ui, texts: Vec<Option<String>>) -> String {
        let reads = texts.into_iter().map(|text| text.and_then(|t| exchange::import(&t).ok())).collect();
        self.merge_imports(ui, reads, &[], false)
    }

    /// Merges what imports read into the contacts, their extra values and their card fields. `None` is a file that could
    /// not be read; `notes` are lines added to the summary (how a table's columns were mapped). With `track` the added
    /// contacts become the Last Import list. Returns the summary.
    fn merge_imports(&mut self, ui: &Ui, reads: Vec<Option<exchange::Import>>, notes: &[String], track: bool) -> String {
        if self.load_failed {
            return "The contacts could not be read, so nothing was imported.".into();
        }
        let mut updated = self.contacts.clone();
        let mut updated_extras = self.extras.clone();
        let mut updated_details = self.details.clone();
        let mut new_ids = BTreeSet::new();
        let (mut added, mut skipped, mut invalid, mut extra, mut unreadable) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for read in reads {
            let Some(read) = read else {
                unreadable += 1;
                continue;
            };
            invalid += read.invalid;
            extra += read.extra_values;
            for ((contact, values), card) in read.contacts.into_iter().zip(read.values).zip(read.details) {
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
                updated_details.set(id, card);
                new_ids.insert(id);
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
        let mut summary = parts.join(", ") + ".";
        for note in notes {
            summary.push(' ');
            summary.push_str(note);
            summary.push('.');
        }
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
                self.write_extras();
                if updated_details != self.details {
                    self.details = updated_details;
                    self.write_details();
                }
                if track {
                    self.prefs.last_import = new_ids;
                    self.save_display();
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
        let options = exchange::ExportOptions { notes: self.prefs.export_notes };
        let text = exchange::export_with(&self.contacts, &self.extras, &self.details, options, version);
        match std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(file), text)) {
            Ok(()) => {
                self.error = None;
                self.status = Some(format!("Exported {} contacts as vCard {label} to the Contacts Export folder", self.contacts.len()));
            }
            Err(e) => self.error = Some(format!("Unable to export: {e}")),
        }
        self.dirty = true;
    }

    // ---- archive, print and PDF ----

    /// The names of the data files in the Contacts folder: the contacts file, then the files beside it.
    fn archive_names(&self) -> Vec<String> {
        let own = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::iter::once(own).chain(ARCHIVED_FILES.iter().map(|n| n.to_string())).collect()
    }

    fn data_dir(&self) -> PathBuf {
        self.path.parent().unwrap_or(Path::new("")).to_path_buf()
    }

    /// Writes `Export/contacts.bcarchive`: one file with the contacts, their values and card fields, the groups and the
    /// settings.
    fn export_archive(&mut self) {
        if self.load_failed {
            return;
        }
        let names = self.archive_names();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let files = match archive::read_files(&self.data_dir(), &refs) {
            Ok(files) => files,
            Err(e) => {
                self.error = Some(format!("Unable to read the Contacts folder: {e}"));
                self.dirty = true;
                return;
            }
        };
        let folder = self.folder("Export");
        match std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(ARCHIVE_FILE), archive::pack(&files))) {
            Ok(()) => {
                self.error = None;
                self.status = Some(format!("Exported a Contacts archive of {} files to the Contacts Export folder", files.len()));
            }
            Err(e) => self.error = Some(format!("Unable to export: {e}")),
        }
        self.dirty = true;
    }

    /// Replaces the address book with the newest `.bcarchive` file in the Import folder. The first choice only asks; the
    /// second one restores. A file that does not read puts the old data back.
    fn restore_archive(&mut self, ui: &Ui) {
        if self.load_failed {
            return;
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.folder("Import"))
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bcarchive")))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        let Some(file) = found.pop() else {
            self.confirm_restore = false;
            self.status = Some("Put a Contacts archive (.bcarchive) in the Contacts Import folder, then choose Restore from Archive".into());
            self.dirty = true;
            return;
        };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !self.confirm_restore {
            self.confirm_restore = true;
            self.error = None;
            self.status = Some(format!("Restoring replaces every contact, group and setting with {name}. Choose Restore from Archive again to go on"));
            self.dirty = true;
            return;
        }
        self.confirm_restore = false;
        match self.restore_from(ui, &file) {
            Ok(count) => {
                self.status = Some(format!("Restored {count} contacts from {name}"));
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.dirty = true;
    }

    fn restore_from(&mut self, ui: &Ui, file: &Path) -> Result<usize, String> {
        let bytes = std::fs::read(file).map_err(|e| format!("Unable to read the archive: {e}"))?;
        let files = archive::unpack(&bytes)?;
        let names = self.archive_names();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        archive::check_names(&files, &refs)?;
        let text_of = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, text)| text.as_str());
        if text_of(&names[0]).and_then(|text| text.lines().next()) != Some(CONTACTS_HEADER) {
            return Err("The archive holds no contacts file".into());
        }
        if text_of(VALUES_FILE).is_some_and(|text| extras::parse(text).is_none())
            || text_of(DETAILS_FILE).is_some_and(|text| details::parse(text).is_none())
        {
            return Err("The archive holds a damaged values file".into());
        }
        let dir = self.data_dir();
        let before = archive::read_files(&dir, &refs).map_err(|e| format!("Unable to read the Contacts folder: {e}"))?;
        archive::restore(&dir, &files, &refs).map_err(|e| format!("Unable to restore: {e}"))?;
        let fresh = Contacts::new(ui, (self.w * ui.s).round() as i32, (self.h * ui.s).round() as i32, self.path.clone());
        if fresh.error.is_some() || fresh.load_failed {
            // put the old files back; the old data loaded before, so it loads again
            archive::restore(&dir, &before, &refs).map_err(|e| format!("The archive could not be read, and the old data could not be put back: {e}"))?;
            return Err("The archive could not be read, so nothing was restored".into());
        }
        let count = fresh.contacts.len();
        *self = fresh;
        Ok(count)
    }

    /// The text to print or save as PDF, and its title: the open card, or the list as it is shown.
    fn printable(&self) -> (String, String) {
        if let Some(c) = self.selected() {
            return (printing::card_text(c, &self.extras, &self.details), self.title_of(c));
        }
        let shown: Vec<&Contact> = self.rows.iter().map(|r| &self.contacts[r.contact]).collect();
        (printing::list_text(&shown, &self.extras, &self.details), "Contacts".into())
    }

    /// Sends the open card, or the list as it is shown, to the printer.
    fn print_contacts(&mut self) {
        if self.load_failed {
            return;
        }
        let (text, title) = self.printable();
        self.send_to_printer(title, text);
    }

    /// Prints address labels for the contacts the list shows (or the open card).
    fn print_labels(&mut self) {
        if self.load_failed {
            return;
        }
        let shown: Vec<&Contact> = match self.selected() {
            Some(c) => vec![c],
            None => self.rows.iter().map(|r| &self.contacts[r.contact]).collect(),
        };
        let (text, count) = printing::labels_text(&shown, &self.extras);
        if count == 0 {
            self.status = Some("None of the contacts shown has an address".into());
            self.dirty = true;
            return;
        }
        self.send_to_printer("Address Labels".into(), text);
    }

    fn send_to_printer(&mut self, title: String, text: String) {
        self.error = None;
        if !print_service::available() {
            self.status = Some("No print service is running. Use Export as PDF to save the pages instead".into());
        } else {
            // the job goes to the print scheduler on its own thread, so the window keeps drawing
            let job = PrintOperation::text(title, text);
            bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                if let Err(e) = job.run() {
                    eprintln!("Contacts print: {e:?}");
                }
            });
            self.status = Some("Sent to the printer".into());
        }
        self.dirty = true;
    }

    /// Writes the open card, or the list as it is shown, to a PDF in the Export folder.
    fn export_pdf(&mut self) {
        if self.load_failed {
            return;
        }
        let (text, title) = self.printable();
        let folder = self.folder("Export");
        let file = format!("{}.pdf", file_stem(&title));
        let result = std::fs::create_dir_all(&folder)
            .map_err(|e| e.to_string())
            .and_then(|()| PrintOperation::text(title, text).to(Destination::Pdf(folder.join(&file))).run().map_err(|e| format!("{e:?}")));
        match result {
            Ok(_) => {
                self.error = None;
                self.status = Some(format!("Saved {file} in the Contacts Export folder"));
            }
            Err(e) => self.error = Some(format!("Unable to export: {e}")),
        }
        self.dirty = true;
    }

    // ---- cards: My Card, stepping, company, fields, merging ----

    /// Card > Make This My Card.
    fn make_my_card(&mut self) {
        let Screen::Detail(id) = self.screen else {
            self.status = Some("Open a card first, then choose Make This My Card".into());
            self.dirty = true;
            return;
        };
        if self.load_failed {
            return;
        }
        self.prefs.me = Some(id);
        self.save_display();
        self.status = Some("This card is now My Card".into());
        self.dirty = true;
    }

    /// Card > Go to My Card: clears the search and the list filter and opens the card.
    fn go_to_my_card(&mut self, ui: &Ui) {
        let Some(me) = self.prefs.me.filter(|me| self.contacts.iter().any(|c| c.id == *me)) else {
            self.status = Some("No card is My Card yet. Open a card and choose Make This My Card".into());
            self.dirty = true;
            return;
        };
        self.show_card(ui, me);
    }

    /// Shows the list unfiltered and unsearched, opens the card of contact `id` and scrolls the list to it.
    fn show_card(&mut self, ui: &Ui, id: u64) {
        self.filter = Filter::All;
        self.query.clear();
        self.rebuild(ui);
        self.open_detail(ui, id);
        self.reveal(id);
    }

    /// Scrolls the list so the row of contact `id` is in view.
    fn reveal(&mut self, id: u64) {
        let Some(row) = self.rows.iter().find(|r| self.contacts[r.contact].id == id) else { return };
        let visible = self.list_bottom() - self.list_top();
        let offset = reveal_offset(row.top, ROW, self.list_scroll.offset(), visible);
        self.list_scroll.jump_to(offset);
        self.dirty = true;
    }

    /// Card > Go to Next Card and Go to Previous Card: moves the open card along the list. Stops at the ends. With no
    /// card open, the next card is the first and the previous card is the last.
    fn step_card(&mut self, ui: &Ui, forward: bool) {
        // the form keeps its unsaved edits
        if self.rows.is_empty() || self.editing() {
            return;
        }
        let current = match self.screen {
            Screen::Detail(id) => self.rows.iter().position(|r| self.contacts[r.contact].id == id),
            _ => None,
        };
        let last = self.rows.len() - 1;
        let target = match (current, forward) {
            (Some(i), true) => (i + 1).min(last),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => 0,
            (None, false) => last,
        };
        let id = self.contacts[self.rows[target].contact].id;
        self.open_detail(ui, id);
        self.reveal(id);
    }

    /// Card > Mark as a Company: the card shows its company in place of the name. In the edit form it flips the switch;
    /// on a card it is saved at once.
    fn toggle_company(&mut self, ui: &Ui) {
        match self.screen {
            Screen::Editor(_) => {
                self.edit_company = !self.edit_company;
                self.dirty = true;
            }
            Screen::Detail(id) if !self.load_failed => {
                let mut card = self.details.of(id);
                card.company = !card.company;
                let no_company_name = self.contacts.iter().find(|c| c.id == id).map(|c| c.company.trim().is_empty()).unwrap_or(true);
                let on = card.company;
                self.details.set(id, card);
                self.write_details();
                self.status = (on && no_company_name).then(|| "Add a company name for the card to show it as the title".to_string());
                self.rebuild(ui);
            }
            _ => {}
        }
    }

    /// Card > Add Field: opens the edit form on the open card and shows the field.
    fn add_field(&mut self, ui: &Ui, detail: Detail) {
        if self.load_failed {
            return;
        }
        if let Screen::Detail(id) = self.screen {
            self.edit_contact(ui, Some(id));
        }
        if !self.editing_contact() {
            self.status = Some("Open a card first, then choose Add Field".into());
            self.dirty = true;
            return;
        }
        let k = detail.index();
        self.detail_shown[k] = true;
        self.fit_scroll(ui);
        self.focus_field(DETAIL_BASE + k);
    }

    /// Merges the duplicates of the open card, or with `all` every group of duplicates: the first card of a group keeps
    /// its place and takes the others' fields, extra values, card fields, groups and notes; the others are deleted.
    fn merge_duplicates(&mut self, ui: &Ui, all: bool) {
        if self.load_failed || self.groups_failed {
            return;
        }
        let found = dupes::groups(&self.contacts);
        let open = match self.screen {
            Screen::Detail(id) => Some(id),
            _ => None,
        };
        let chosen: Vec<Vec<u64>> = if all {
            found
        } else {
            found.into_iter().filter(|group| open.is_some_and(|id| group.contains(&id))).collect()
        };
        self.merge_groups(ui, &chosen, open);
    }

    /// Folds each group of cards (identifiers, the first one stays) into its first card and saves the result. `open` is
    /// the card on show, which is shown again when it was merged.
    fn merge_groups(&mut self, ui: &Ui, chosen: &[Vec<u64>], open: Option<u64>) {
        let mut contacts = self.contacts.clone();
        let mut values = self.extras.clone();
        let mut cards = self.details.clone();
        let mut merges = Vec::new();
        let mut dropped = 0;
        for group in chosen {
            let Some(merged) = dupes::merge(&contacts, &values, &cards, group) else { continue };
            if let Some(slot) = contacts.iter_mut().find(|c| c.id == merged.keep.id) {
                *slot = merged.keep.clone();
            }
            contacts.retain(|c| !merged.removed.contains(&c.id));
            values.remove(merged.keep.id);
            for extra in merged.extras {
                values.add(merged.keep.id, extra);
            }
            cards.set(merged.keep.id, merged.details);
            for id in &merged.removed {
                values.remove(*id);
                cards.remove(*id);
            }
            dropped += merged.dropped;
            merges.push((merged.keep.id, merged.removed));
        }
        if merges.is_empty() {
            self.status = Some("There are no duplicates to merge".into());
            self.dirty = true;
            return;
        }
        if let Err(e) = store::save(&self.path, &contacts) {
            self.error = Some(format!("Unable to merge: {e}"));
            self.dirty = true;
            return;
        }
        let before = std::mem::replace(&mut self.contacts, contacts);
        self.record(&before);
        self.error = None;
        self.extras = values;
        self.write_extras();
        self.details = cards;
        self.write_details();
        let mut groups_changed = false;
        let mut prefs_changed = false;
        let mut removed_total = 0;
        let mut reopen = None;
        for (keep, removed) in &merges {
            for id in removed {
                for group in self.groups.groups_of(*id) {
                    let _ = self.groups.add_member(group, *keep);
                }
                groups_changed |= self.groups.forget_contact(*id) > 0;
                if self.prefs.me == Some(*id) {
                    self.prefs.me = Some(*keep);
                    prefs_changed = true;
                }
                prefs_changed |= self.prefs.last_import.remove(id);
            }
            // an open card that was merged, or that took the others in, is shown again with its new fields
            if open.is_some_and(|id| id == *keep || removed.contains(&id)) {
                reopen = Some(*keep);
            }
            removed_total += removed.len();
        }
        if groups_changed {
            self.save_groups();
        }
        if prefs_changed {
            self.save_display();
        }
        let mut status = format!("Merged {removed_total} duplicate cards into {} cards", merges.len());
        if dropped > 0 {
            status.push_str(&format!("; {dropped} values did not fit and were left out"));
        }
        self.rebuild(ui);
        if let Some(keep) = reopen {
            self.open_detail(ui, keep);
        }
        // opening a card clears the status line, so the summary is set last
        self.status = Some(status);
    }

    // ---- intents and the control socket ----

    /// The contact a name or identifier points to: an identifier that exists, a name that matches exactly (ignoring case
    /// and accents), or else the first contact a search for the text finds.
    fn find_contact(&self, query: &str) -> Option<u64> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        if let Some(id) = query.parse::<u64>().ok().filter(|id| self.contacts.iter().any(|c| c.id == *id)) {
            return Some(id);
        }
        let wanted = names::fold(query);
        if let Some(c) = self.contacts.iter().find(|c| names::fold(c.display_name()) == wanted) {
            return Some(c.id);
        }
        list_hits(&self.contacts, &self.extras, &self.details, query).first().map(|&i| self.contacts[i].id)
    }

    /// The `view-card` intent: opens the card of the contact the text names. Returns the contact's identifier and title.
    pub fn open_card(&mut self, ui: &Ui, query: &str) -> Result<(u64, String), String> {
        let id = self.find_contact(query).ok_or_else(|| format!("No contact matches \"{}\"", query.trim()))?;
        self.show_card(ui, id);
        let title = self.contacts.iter().find(|c| c.id == id).map(|c| self.title_of(c)).unwrap_or_default();
        Ok((id, title))
    }

    /// The `create-contact` intent: adds a contact and opens its card. A name or a company, a phone number or an email is
    /// needed, and a contact that duplicates one in the list is refused.
    pub fn create_contact(&mut self, ui: &Ui, name: &str, phone: &str, email: &str, company: &str) -> Result<u64, String> {
        if self.load_failed {
            return Err("The contacts could not be read".into());
        }
        let mut contact = Contact {
            id: 0,
            name: name.trim().into(),
            phone: phone.trim().into(),
            email: email.trim().into(),
            company: company.trim().into(),
            labels: [0, 2, 2],
            ..Default::default()
        };
        if !contact.valid() {
            return Err("A contact needs a name, a company, a phone number or an email, and the values must fit".into());
        }
        if let Some(twin) = self.contacts.iter().find(|c| dupes::is_duplicate(c, &contact)) {
            return Err(format!("{} is already in the contacts (id {})", twin.display_name(), twin.id));
        }
        contact.id = store::new_id(&self.contacts).ok_or_else(|| "Contact limit reached".to_string())?;
        let id = contact.id;
        let mut updated = self.contacts.clone();
        updated.push(contact);
        store::save(&self.path, &updated).map_err(|e| format!("Unable to save: {e}"))?;
        let before = std::mem::replace(&mut self.contacts, updated);
        self.record(&before);
        self.error = None;
        self.show_card(ui, id);
        Ok(id)
    }

    /// The `search-contacts` intent: puts the text in the search field and returns what the list then shows, as
    /// (identifier, title, phone, email).
    pub fn search_contacts(&mut self, ui: &Ui, query: &str) -> Vec<(u64, String, String, String)> {
        self.filter = Filter::All;
        self.query.clear();
        let room = store::MAX_NAME.saturating_sub(self.query.chars().count());
        self.query.push_str(&query.trim().chars().take(room).collect::<String>());
        self.list_scroll.jump_top();
        self.rebuild(ui);
        self.rows
            .iter()
            .map(|r| {
                let c = &self.contacts[r.contact];
                (c.id, self.title_of(c), c.phone.clone(), c.email.clone())
            })
            .collect()
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
                    return Some(Target::Use(k));
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
                if l.company.contains(x, y) {
                    return Some(Target::CompanyToggle);
                }
                for (k, row) in l.details.iter().enumerate() {
                    if let Some((label, field)) = row {
                        if label.contains(x, y) || field.contains(x, y) {
                            return Some(Target::Field(DETAIL_BASE + k));
                        }
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
                let l = self.settings_layout();
                if l.sort.contains(x, y) {
                    return Some(Target::Sort);
                }
                if let Some((setting, _)) = l.switches.iter().find(|(_, r)| r.contains(x, y)) {
                    return Some(Target::Setting(*setting));
                }
            }
            Screen::SmartEditor(_) => {
                let l = self.smart_layout();
                if l.delete.is_some_and(|d| d.contains(x, y)) {
                    return Some(Target::SmartDelete);
                }
                if l.cancel.is_some_and(|c| c.contains(x, y)) {
                    return Some(Target::CancelDelete);
                }
                if l.name.contains(x, y) {
                    return Some(Target::Field(SMART_NAME));
                }
                if l.any.contains(x, y) {
                    return Some(Target::SmartAny);
                }
                for (n, row) in l.rules.iter().enumerate() {
                    if row.remove.contains(x, y) {
                        return Some(Target::SmartRemove(n));
                    }
                    if row.field.contains(x, y) {
                        return Some(Target::SmartField(n));
                    }
                    if row.condition.contains(x, y) {
                        return Some(Target::SmartCondition(n));
                    }
                    if row.value.contains(x, y) && self.smart_rules[n].condition.needs_value() {
                        return Some(Target::Field(SMART_VALUE + n));
                    }
                }
                if l.add.is_some_and(|a| a.contains(x, y)) {
                    return Some(Target::SmartAdd);
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
                self.scroll.jump_top();
                self.fit_scroll(ui);
            }
            BAR_MERGE => self.merge_duplicates(ui, false),
            BAR_MERGE_ALL => self.merge_duplicates(ui, true),
            BAR_GROUP_SELECTION => self.group_selection(ui),
            BAR_MERGE_SELECTION => self.merge_selection(ui),
            BAR_SELECT_DONE => self.stop_selecting(),
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
                } else if matches!(self.screen, Screen::SmartEditor(_)) {
                    self.save_smart(ui);
                } else {
                    self.save_editor(ui);
                }
            }
            BAR_EDIT_LIST => self.edit_list(ui),
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
            NODE_COMPANY => Target::CompanyToggle,
            NODE_SMART_NAME => Target::Field(SMART_NAME),
            NODE_SMART_ANY => Target::SmartAny,
            NODE_SMART_ADD => Target::SmartAdd,
            NODE_SMART_DELETE => Target::SmartDelete,
            _ if (NODE_SMART_FIELD..NODE_SMART_FIELD + smart::MAX_RULES as u64).contains(&id) => Target::SmartField((id - NODE_SMART_FIELD) as usize),
            _ if (NODE_SMART_CONDITION..NODE_SMART_CONDITION + smart::MAX_RULES as u64).contains(&id) => {
                Target::SmartCondition((id - NODE_SMART_CONDITION) as usize)
            }
            _ if (NODE_SMART_VALUE..NODE_SMART_VALUE + smart::MAX_RULES as u64).contains(&id) => Target::Field(SMART_VALUE + (id - NODE_SMART_VALUE) as usize),
            _ if (NODE_SMART_REMOVE..NODE_SMART_REMOVE + smart::MAX_RULES as u64).contains(&id) => Target::SmartRemove((id - NODE_SMART_REMOVE) as usize),
            _ if (NODE_SETTING..NODE_SETTING + Setting::all().len() as u64).contains(&id) => {
                Target::Setting(Setting::all()[(id - NODE_SETTING) as usize])
            }
            _ if (NODE_DETAIL..NODE_DETAIL + N_DETAILS as u64).contains(&id) => Target::Field(DETAIL_BASE + (id - NODE_DETAIL) as usize),
            _ if (NODE_COPY_MORE + 10..NODE_COPY_MORE + 40).contains(&id) => Target::Copy((id - NODE_COPY_MORE) as usize),
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
                } else if self.selecting {
                    if let Some(id) = self.rows.get(i).map(|row| self.contacts[row.contact].id) {
                        if !self.selection.remove(&id) {
                            self.selection.insert(id);
                        }
                    }
                } else if let Some(row) = self.rows.get(i) {
                    let id = self.contacts[row.contact].id;
                    // a second press on the same row within the double-click time opens the card in its own window
                    let again = self.last_row_click.is_some_and(|(last, at)| last == id && t - at <= DOUBLE_CLICK_MS);
                    self.last_row_click = Some((id, t));
                    if again {
                        self.last_row_click = None;
                        self.card_req = Some(id);
                    }
                    self.open_detail(ui, id);
                }
            }
            Target::Sort => self.cycle_sort(ui),
            Target::List => {
                if matches!(self.screen, Screen::Detail(_)) {
                    self.close_detail(ui);
                }
            }
            Target::Use(k) => {
                if let Some(c) = self.selected() {
                    let rows = self.rows_of(c);
                    let row = rows.get(k);
                    let value = row.map(|r| r.value.clone()).unwrap_or_default();
                    let handoff = match row.map(|r| &r.action) {
                        Some(contact_card::Action::Call) => Some(Handoff::Call(value.clone())),
                        Some(contact_card::Action::Email) => Some(Handoff::Mail(value.clone())),
                        _ => None,
                    };
                    match handoff {
                        Some(h) if !value.is_empty() => self.handoff = Some(h),
                        _ if !value.is_empty() => {
                            clipboard::set_text(&value);
                            self.toast_until = Some(t + 2000.0);
                        }
                        _ => {}
                    }
                }
            }
            Target::Copy(k) => {
                if let Some(c) = self.selected() {
                    let value = self.rows_of(c).get(k).map(|row| row.value.clone()).unwrap_or_default();
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
                } else if let Some(field) = self.field_mut(i) {
                    field.end();
                }
                self.focus_field(i);
            }
            Target::CompanyToggle => self.toggle_company(ui),
            Target::Setting(setting) => self.toggle_setting(setting),
            Target::SmartAny => self.smart_any = !self.smart_any,
            Target::SmartField(n) => {
                if let Some(rule) = self.smart_rules.get_mut(n) {
                    rule.field = rule.field.next();
                }
            }
            Target::SmartCondition(n) => {
                if let Some(rule) = self.smart_rules.get_mut(n) {
                    rule.condition = rule.condition.next();
                    if !rule.condition.needs_value() && self.focus == Some(SMART_VALUE + n) {
                        self.focus = None;
                    }
                }
            }
            Target::SmartRemove(n) => self.remove_rule(ui, n),
            Target::SmartAdd => self.add_rule(ui),
            Target::SmartDelete => self.delete_list(ui),
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
            _ if (DETAIL_BASE..DETAIL_BASE + N_DETAILS).contains(&i) => Some(&mut self.detail_fields[i - DETAIL_BASE]),
            SMART_NAME => Some(&mut self.smart_name),
            _ if (SMART_VALUE..SMART_VALUE + smart::MAX_RULES).contains(&i) => self.smart_rules.get_mut(i - SMART_VALUE).map(|rule| &mut rule.value),
            _ => None,
        }
    }

    /// The form's fields in the order Tab visits them: the names, the phone, email and address that are shown, the card
    /// fields that are shown, and the notes.
    fn focus_order(&self) -> Vec<usize> {
        let mut order = vec![0, 1, 2];
        order.extend((0..3).filter(|&k| self.shown[k]).map(|k| 3 + k));
        order.extend((0..N_DETAILS).filter(|&k| self.detail_shown[k]).map(|k| DETAIL_BASE + k));
        order.push(NOTES);
        order
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        let Some(i) = self.focus else {
            if matches!(out, Out::Hide | Out::Named("Escape")) && self.editing() {
                self.cancel_editor(ui);
            } else if matches!(out, Out::Hide | Out::Named("Escape")) && self.screen == Screen::Settings {
                self.screen = Screen::List;
                self.fit_scroll(ui);
            } else if matches!(out, Out::Hide | Out::Named("Escape")) && self.selecting {
                self.stop_selecting();
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
        if (i == SMART_NAME || i >= SMART_VALUE) && matches!(out, Out::Enter | Out::Named("Tab")) {
            let order = self.smart_focus_order();
            if let Some(j) = order.iter().position(|&f| f == i).and_then(|at| order.get(at + 1)) {
                self.focus_field(*j);
            }
            return;
        }
        if (i < 6 || (DETAIL_BASE..DETAIL_BASE + N_DETAILS).contains(&i)) && matches!(out, Out::Enter | Out::Named("Tab")) {
            let order = self.focus_order();
            if let Some(j) = order.iter().position(|&f| f == i).and_then(|at| order.get(at + 1)) {
                self.focus_field(*j);
            }
            return;
        }
        if i == SEARCH && out == Out::Enter {
            self.focus = None;
            return;
        }
        let max = if i < 3 {
            store::MAX_NAME
        } else if i < 6 || (DETAIL_BASE..DETAIL_BASE + N_DETAILS).contains(&i) {
            store::MAX_VALUE
        } else if i == SMART_NAME {
            smart::MAX_NAME
        } else if i >= SMART_VALUE {
            smart::MAX_VALUE
        } else {
            160
        };
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
            Some(i) if (DETAIL_BASE..DETAIL_BASE + N_DETAILS).contains(&i) => self.detail_fields[i - DETAIL_BASE].commands(clip),
            Some(SMART_NAME) => self.smart_name.commands(clip),
            Some(i) if (SMART_VALUE..SMART_VALUE + smart::MAX_RULES).contains(&i) => {
                self.smart_rules.get(i - SMART_VALUE).map(|rule| rule.value.commands(clip)).unwrap_or_default()
            }
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
            let contact = &self.contacts[self.rows[i].contact];
            let label = self.title_of(contact);
            let value = if self.selecting && self.selection.contains(&contact.id) { "selected" } else { "" };
            nodes.push(Node { id: ROW_NODE + i as u64, role: Role::Row, label, value: value.into(), bounds: r, enabled: true });
        }
        let button = |id: u64, label: &str, bounds: Rect| Node { id, role: Role::Button, label: label.into(), value: String::new(), bounds, enabled: true };
        match self.screen {
            Screen::List => {}
            Screen::Detail(_) => {
                let l = self.card_layout(ui);
                let Some(contact) = self.selected() else { return nodes };
                let rows = self.rows_of(contact);
                for &(k, r) in &l.values {
                    let kind = rows.get(k).map_or_else(|| "value".to_string(), row_word);
                    nodes.push(button(copy_node(k), &format!("Copy {kind}"), r));
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
                let on = if self.edit_company { "on" } else { "off" };
                nodes.push(Node { id: NODE_COMPANY, role: Role::Button, label: "Mark as Company".into(), value: on.into(), bounds: l.company, enabled: true });
                for (k, detail) in Detail::ALL.into_iter().enumerate() {
                    if let Some((_, r)) = l.details[k] {
                        nodes.push(Node {
                            id: NODE_DETAIL + k as u64,
                            role: Role::TextField,
                            label: detail.title().into(),
                            value: self.detail_fields[k].to_string(),
                            bounds: r,
                            enabled: true,
                        });
                    }
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
                let l = self.settings_layout();
                nodes.push(Node { id: NODE_SORT, role: Role::Button, label: "Sort Order".into(), value: shown, bounds: l.sort, enabled: true });
                for (n, (setting, bounds)) in l.switches.iter().enumerate() {
                    let value = if self.setting_on(*setting) { "on" } else { "off" };
                    nodes.push(Node { id: NODE_SETTING + n as u64, role: Role::Button, label: setting.title(), value: value.into(), bounds: *bounds, enabled: true });
                }
            }
            Screen::SmartEditor(_) => {
                let l = self.smart_layout();
                nodes.push(Node { id: NODE_SMART_NAME, role: Role::TextField, label: "List name".into(), value: self.smart_name.to_string(), bounds: l.name, enabled: true });
                let mode = if self.smart_any { "any rule" } else { "all rules" };
                nodes.push(Node { id: NODE_SMART_ANY, role: Role::Button, label: "Contacts match".into(), value: mode.into(), bounds: l.any, enabled: true });
                for (n, (row, rule)) in l.rules.iter().zip(&self.smart_rules).enumerate() {
                    let n = n as u64;
                    nodes.push(Node { id: NODE_SMART_FIELD + n, role: Role::Button, label: "Rule field".into(), value: rule.field.title().into(), bounds: row.field, enabled: true });
                    nodes.push(Node { id: NODE_SMART_CONDITION + n, role: Role::Button, label: "Rule condition".into(), value: rule.condition.title().into(), bounds: row.condition, enabled: true });
                    if rule.condition.needs_value() {
                        nodes.push(Node { id: NODE_SMART_VALUE + n, role: Role::TextField, label: "Rule value".into(), value: rule.value.to_string(), bounds: row.value, enabled: true });
                    }
                    nodes.push(Node { id: NODE_SMART_REMOVE + n, role: Role::Button, label: "Remove rule".into(), value: String::new(), bounds: row.remove, enabled: self.smart_rules.len() > 1 });
                }
                if let Some(a) = l.add {
                    nodes.push(button(NODE_SMART_ADD, "add rule", a));
                }
                if let Some(d) = l.delete {
                    let delete = if self.confirm_delete { "Confirm Delete Smart List" } else { "Delete Smart List" };
                    nodes.push(button(NODE_SMART_DELETE, delete, d));
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
    /// The round avatar: the initials, or a small building for a card marked as a company.
    fn avatar(&mut self, s: &mut Surface, ui: &Ui, (cx, cy, r): (f32, f32, f32), initials: &str, size: f32, company: bool) {
        Self::fill(s, ui, rect(cx - r, cy - r, 2.0 * r, 2.0 * r), r, LABEL_2.now());
        if !company {
            self.centre_text(s, ui, initials, size, Weight::Medium, WHITE, cx, cy);
            return;
        }
        // a white building with two columns of windows in the avatar's own colour
        let (bw, bh) = (0.8 * r, 1.0 * r);
        Self::fill(s, ui, rect(cx - bw / 2.0, cy - bh / 2.0, bw, bh), 1.0, CARD_BG.now());
        let pane = 0.16 * r;
        for col in 0..2 {
            for row in 0..3 {
                let x = cx - bw / 2.0 + 0.16 * r + col as f32 * 0.32 * r;
                let y = cy - bh / 2.0 + 0.14 * r + row as f32 * 0.28 * r;
                Self::fill(s, ui, rect(x, y, pane, pane), 0.0, LABEL_2.now());
            }
        }
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
                let selected = self.screen == Screen::Detail(contact.id)
                    || self.screen == Screen::Editor(Some(contact.id))
                    || (self.selecting && self.selection.contains(&contact.id));
                if selected {
                    Self::fill(&mut content, ui, r, m::SELECTION_RADIUS, SELECTION.now());
                }
                let d = ROW - 2.0 * SPACE_XS - SPACE_XS;
                let company = self.details.is_company(contact.id) && !contact.company.trim().is_empty();
                self.avatar(&mut content, ui, (r.x + SPACE_S + d / 2.0, r.y + r.h / 2.0, d / 2.0), &contact.initials(), MAC_CALLOUT_PT, company);
                let tx = r.x + 2.0 * SPACE_S + d;
                let (ink, ink2) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
                // My Card carries a small "Me" mark at the row's end
                let mark = (self.prefs.me == Some(contact.id)).then(|| ui.measure("Me", MAC_FOOTNOTE_PT, Weight::Regular) / ui.pt(1.0));
                if let Some(width) = mark {
                    self.text(&mut content, ui, "Me", MAC_FOOTNOTE_PT, Weight::Regular, ink2, r.x + r.w - SPACE_S - width, r.y + r.h / 2.0);
                }
                let room = ui.pt(r.x + r.w - tx - SPACE_S - mark.map_or(0.0, |width| width + SPACE_S));
                let sub = if !contact.phone.is_empty() { contact.phone.clone() } else { contact.email.clone() };
                let shown = self.title_of(&contact);
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

    /// The Settings screen: the sort order (a press moves to the next choice), then switches for nicknames, vCard notes and
    /// the fields a new card shows.
    fn draw_settings(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, cw) = self.column();
        let l = self.settings_layout();
        self.centre_text(s, ui, "Settings", MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x + cw / 2.0, l.title_y);
        let r = l.sort;
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
        for (setting, row) in &l.switches {
            if *setting == Setting::Template(0) {
                self.text(s, ui, "Fields a New Card Shows", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + SPACE_S, l.heading_y);
            }
            Self::fill(s, ui, *row, m::CARD_RADIUS, CARD_BG.now());
            let title = ui.fit(&setting.title(), MAC_BODY_PT, ui.pt(cw - 2.0 * SPACE_L - MAC_BODY_PT * 4.0));
            self.text(s, ui, &title, MAC_BODY_PT, Weight::Regular, LABEL.now(), row.x + SPACE_L, row.y + row.h / 2.0);
            let state = if self.setting_on(*setting) { "On" } else { "Off" };
            let width = ui.measure(state, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0);
            self.text(s, ui, state, MAC_BODY_PT, Weight::Regular, LINK, row.x + row.w - SPACE_L - width, row.y + row.h / 2.0);
        }
    }

    /// The smart list editor: the name, the match mode, one row per rule, and Add Rule.
    fn draw_smart_editor(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, cw) = self.column();
        let l = self.smart_layout();
        let title = if matches!(self.screen, Screen::SmartEditor(Some(_))) { "Edit Smart List" } else { "New Smart List" };
        self.centre_text(s, ui, title, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x + cw / 2.0, l.title_y);
        widgets::text_field(s, ui, &self.smart_name, l.name, "List name", self.focus == Some(SMART_NAME));
        let any = l.any;
        Self::fill(s, ui, any, m::CARD_RADIUS, CARD_BG.now());
        self.text(s, ui, "Contacts match", MAC_BODY_PT, Weight::Regular, LABEL.now(), any.x + SPACE_L, any.y + any.h / 2.0);
        let mode = if self.smart_any { "Any Rule" } else { "All Rules" };
        let mode_w = ui.measure(mode, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0);
        self.text(s, ui, mode, MAC_BODY_PT, Weight::Regular, LINK, any.x + any.w - SPACE_L - mode_w, any.y + any.h / 2.0);
        for (n, row) in l.rules.iter().enumerate() {
            let (field, condition) = (self.smart_rules[n].field, self.smart_rules[n].condition);
            for (cell, label) in [(row.field, field.title()), (row.condition, condition.title())] {
                Self::fill(s, ui, cell, m::CARD_RADIUS, CARD_BG.now());
                let text = ui.fit(label, MAC_CALLOUT_PT, ui.pt(cell.w - 2.0 * SPACE_S));
                self.text(s, ui, &text, MAC_CALLOUT_PT, Weight::Regular, LINK, cell.x + SPACE_S, cell.y + cell.h / 2.0);
            }
            if condition.needs_value() {
                widgets::text_field(s, ui, &self.smart_rules[n].value, row.value, "Value", self.focus == Some(SMART_VALUE + n));
            }
            if self.smart_rules.len() > 1 {
                let r = row.remove;
                glyph(s, ui, "xmark", rect(r.x + SPACE_S, r.y + SPACE_S, r.w - 2.0 * SPACE_S, r.h - 2.0 * SPACE_S), LINK);
            }
        }
        if let Some(r) = l.add {
            let g = MAC_BODY_PT + SPACE_XS;
            glyph(s, ui, "plus", rect(r.x + SPACE_XS, r.y + (r.h - g) / 2.0, g, g), LINK);
            self.text(s, ui, "add rule", MAC_BODY_PT, Weight::Regular, LINK, r.x + g + SPACE_M, r.y + r.h / 2.0);
        }
        if let Some(d) = l.delete {
            self.button_row(s, ui, d, if self.confirm_delete { "Confirm Delete Smart List" } else { "Delete Smart List" }, DESTRUCTIVE);
        }
        if let Some(c) = l.cancel {
            self.button_row(s, ui, c, "Cancel", LINK);
        }
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
        let is_company = self.details.is_company(contact.id) && !contact.company.trim().is_empty();
        self.avatar(s, ui, l.avatar, &contact.initials(), MAC_LARGE_TITLE_PT, is_company);
        let shown = self.title_of(&contact);
        let name = ui.fit(&shown, MAC_TITLE1_PT, ui.pt(cw));
        let name_mid = l.name_y + (MAC_TITLE1_PT + SPACE_S) / 2.0;
        self.centre_text(s, ui, &name, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x + cw / 2.0, name_mid);
        let mut caption_y = name_mid + MAC_TITLE1_PT;
        if !contact.company.is_empty() && contact.company != shown && contact.company != contact.display_name() {
            let company = ui.fit(&contact.company, MAC_CALLOUT_PT, ui.pt(cw));
            self.centre_text(s, ui, &company, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x + cw / 2.0, caption_y);
            caption_y += MAC_CALLOUT_PT + SPACE_XS;
        }
        if self.prefs.me == Some(contact.id) {
            self.centre_text(s, ui, "My Card", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + cw / 2.0, caption_y);
        }
        // the value rows come from the card model, in the same order as the layout's rects
        for (row, &(_, r)) in self.rows_of(&contact).iter().zip(&l.values) {
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
            self.avatar(s, ui, l.avatar, &probe.initials(), MAC_LARGE_TITLE_PT, self.edit_company && !probe.company.trim().is_empty());
        }
        const PLACEHOLDERS: [&str; 6] = ["First name", "Last name", "Company", "Phone", "Email", "Address"];
        for i in 0..6 {
            if let Some(r) = l.fields[i] {
                widgets::text_field(s, ui, &self.fields[i], r, PLACEHOLDERS[i], self.focus == Some(i));
            }
        }
        let switch = l.company;
        Self::fill(s, ui, switch, m::CARD_RADIUS, CARD_BG.now());
        self.text(s, ui, "Mark as Company", MAC_BODY_PT, Weight::Regular, LABEL.now(), switch.x + SPACE_L, switch.y + switch.h / 2.0);
        let state = if self.edit_company { "On" } else { "Off" };
        let state_w = ui.measure(state, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0);
        self.text(s, ui, state, MAC_BODY_PT, Weight::Regular, LINK, switch.x + switch.w - SPACE_L - state_w, switch.y + switch.h / 2.0);
        for (k, detail) in Detail::ALL.into_iter().enumerate() {
            if let Some((label, field)) = l.details[k] {
                self.text(s, ui, detail.title(), MAC_BODY_PT, Weight::Regular, LINK, label.x + SPACE_XS, label.y + label.h / 2.0);
                widgets::text_field(s, ui, &self.detail_fields[k], field, detail.title(), self.focus == Some(DETAIL_BASE + k));
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
            Screen::SmartEditor(_) => self.draw_smart_editor(s, ui),
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
    fn a_tap_on_a_phone_or_email_hands_it_off_and_other_values_copy() {
        let mut ann = contact(1, "Anna As");
        ann.phone = "555 0100".into();
        ann.email = "anna@work.is".into();
        let path = file_with("handoff", &[ann]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path);
        app.open_detail(&ui, 1);
        let rows = app.rows_of(app.selected().unwrap());
        let phone = rows.iter().position(|r| r.action == contact_card::Action::Call).unwrap();
        let email = rows.iter().position(|r| r.action == contact_card::Action::Email).unwrap();
        app.activate(&ui, Target::Use(phone), 0.0, 0.0, 0.0);
        assert_eq!(app.take_handoff(), Some(Handoff::Call("555 0100".into())));
        app.activate(&ui, Target::Use(email), 0.0, 0.0, 0.0);
        assert_eq!(app.take_handoff(), Some(Handoff::Mail("anna@work.is".into())));
        assert!(app.take_handoff().is_none(), "taken once");
    }

    #[test]
    fn a_phone_value_becomes_a_tel_link_without_spaces_or_dashes() {
        assert_eq!(tel_url("+354 555-0100"), "tel:+3545550100");
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
        assert_eq!(list_hits(&contacts, &extras, &Details::default(), "BO@BERG"), vec![1], "only Bo has that extra email");
        assert_eq!(list_hits(&contacts, &extras, &Details::default(), "anna"), vec![0], "the store's name still matches");
        assert_eq!(list_hits(&contacts, &extras, &Details::default(), "zzz"), Vec::<usize>::new());
        assert_eq!(list_hits(&contacts, &extras, &Details::default(), ""), vec![0, 1], "a blank search shows everyone");
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
        assert_eq!(compare_contacts(SortOrder::FamilyName, &zoe, &adam, None, None), Ordering::Less, "Ann sorts before Zed");
        assert_eq!(compare_contacts(SortOrder::GivenName, &zoe, &adam, None, None), Ordering::Greater, "Adam sorts before Zoe");
        assert_eq!(compare_contacts(SortOrder::None, &zoe, &adam, None, None), Ordering::Less, "identifier order");
    }

    #[test]
    fn a_company_sorts_by_its_company_name_in_the_list() {
        let acme = contact(3, "Jon Smith");
        let bravo = contact(4, "Ann Berg");
        assert_eq!(
            compare_contacts(SortOrder::FamilyName, &acme, &bravo, Some("Acme"), None),
            Ordering::Less,
            "the company Acme sorts before the person Ann Berg, whose family name is Berg"
        );
        assert_eq!(
            compare_contacts(SortOrder::FamilyName, &acme, &bravo, None, None),
            Ordering::Greater,
            "without the company flag Jon Smith sorts after Ann Berg"
        );
        assert_eq!(compare_contacts(SortOrder::FamilyName, &acme, &bravo, Some("Zeta"), None), Ordering::Greater, "Zeta sorts after Berg");
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
        let (saved, _, _) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved.sort_order(), SortOrder::GivenName);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_file_menu_and_view_menu_are_the_app_menus_and_name_order_is_saved() {
        let path = file_with("menus", &[contact(1, "Ada Lovelace")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        let menus = app.main_menu();
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "View", "Card"]);
        assert_eq!(
            menus[0].items.iter().filter(|i| !i.separator).count(),
            11,
            "import, restore, new and edit smart list, export 3.0, 4.0 and archive, print, labels, PDF, refresh"
        );
        assert_eq!(menus[1].items[0].title, "Hide Sidebar", "the sidebar starts shown on a wide window");
        assert!(menus[1].items[2].checked, "first name first is the default");
        assert!(app.menu_action(&ui, MENU_LAST_FIRST));
        assert_eq!(app.name_order, names::NameOrder::LastFirst);
        let menus = app.main_menu();
        assert!(!menus[1].items[2].checked);
        assert!(menus[1].items[3].checked);
        let (_, saved, _) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
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

    fn data_dir_of(path: &Path) -> PathBuf {
        path.parent().unwrap().to_path_buf()
    }

    fn put_import(path: &Path, name: &str, text: &str) {
        let folder = data_dir_of(path).join("Import");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(name), text).unwrap();
    }

    #[test]
    fn the_edit_form_saves_card_fields_and_the_company_flag_and_the_card_shows_them() {
        let path = file_with("card-fields", &[Contact { company: "Acme".into(), ..contact(1, "Anna As") }]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.open_detail(&ui, 1);
        app.add_field(&ui, Detail::Nickname);
        assert!(app.editing(), "Add Field opens the form on the open card");
        assert_eq!(app.focus, Some(DETAIL_BASE + Detail::Nickname.index()));
        app.detail_fields[Detail::Nickname.index()].push_str("Annie");
        app.add_field(&ui, Detail::Birthday);
        app.detail_fields[Detail::Birthday.index()].push_str("19900517");
        assert!(app.activate_control(&ui, NODE_COMPANY), "the switch is a control for assistive technology");
        assert!(app.edit_company);
        app.save_editor(&ui);
        assert!(app.error.is_none(), "{:?}", app.error);
        let saved = details::load_checked(&path.with_file_name(DETAILS_FILE)).unwrap();
        assert_eq!(saved.of(1).get(Detail::Nickname), "Annie");
        assert_eq!(saved.of(1).get(Detail::Birthday), "1990-05-17");
        assert!(saved.of(1).company);
        let rows: Vec<(String, String)> = app.rows_of(app.selected().unwrap()).into_iter().map(|r| (r.label, r.value)).collect();
        assert_eq!(
            rows,
            vec![("Nickname".to_string(), "Annie".to_string()), ("Birthday".to_string(), "1990-05-17".to_string())],
            "the card fields follow the phones, emails and addresses"
        );
        assert_eq!(app.title_of(app.selected().unwrap()), "Acme", "a company card shows its company as the title");
        assert_eq!(app.card_layout(&ui).values.len(), 2);
        app.prefs.prefer_nicknames = true;
        app.toggle_company(&ui);
        assert_eq!(app.title_of(app.selected().unwrap()), "Annie", "with the company flag off, the nickname is preferred");
        let reloaded = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(!reloaded.details.is_company(1), "the toggle on the card is saved");
        assert_eq!(reloaded.details.nickname(1), "Annie");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_search_finds_a_contact_by_a_card_field_and_a_deleted_contact_loses_its_fields() {
        let path = file_with("card-search", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        let mut card = CardDetails::default();
        card.set(Detail::JobTitle, "Lighthouse keeper");
        app.details.set(2, card);
        app.write_details();
        app.query.push_str("lighthouse");
        app.rebuild(&ui);
        assert_eq!(app.listed().iter().map(|c| c.id).collect::<Vec<_>>(), vec![2]);
        app.open_detail(&ui, 2);
        app.remove_selected(&ui);
        assert!(details::load_checked(&path.with_file_name(DETAILS_FILE)).unwrap().of(2).is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn tab_visits_the_shown_card_fields_before_the_notes() {
        let path = file_with("card-tab", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.edit_contact(&ui, Some(1));
        app.detail_shown[Detail::Url.index()] = true;
        app.detail_shown[Detail::Birthday.index()] = true;
        assert_eq!(app.focus_order(), vec![0, 1, 2, DETAIL_BASE + Detail::Url.index(), DETAIL_BASE + Detail::Birthday.index(), NOTES]);
        app.focus_field(DETAIL_BASE + Detail::Url.index());
        app.key(&ui, Out::Named("Tab"), 0.0);
        assert_eq!(app.focus, Some(DETAIL_BASE + Detail::Birthday.index()));
        app.key(&ui, Out::Text("1990".into()), 0.0);
        assert_eq!(&*app.detail_fields[Detail::Birthday.index()], "1990");
        app.key(&ui, Out::Named("Tab"), 0.0);
        assert_eq!(app.focus, Some(NOTES));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn merging_duplicates_folds_the_cards_and_moves_groups_and_my_card() {
        let first = Contact { email: "anna@x.is".into(), ..contact(1, "Anna As") };
        let second = Contact { email: "ANNA@x.is".into(), phone: "555 0100".into(), notes: "Likes tea".into(), ..contact(2, "A. As") };
        let path = file_with("merge", &[first, second, contact(3, "Cara Cole")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.new_group(&ui);
        app.groups.add_member(1, 2).unwrap();
        app.prefs.me = Some(2);
        app.filter = Filter::Duplicates;
        app.rebuild(&ui);
        assert_eq!(app.listed().len(), 2);
        app.open_detail(&ui, 2);
        app.merge_duplicates(&ui, false);
        assert!(app.error.is_none(), "{:?}", app.error);
        let saved = store::load(&path);
        assert_eq!(saved.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3]);
        let kept = saved.iter().find(|c| c.id == 1).unwrap();
        assert_eq!((kept.phone.as_str(), kept.notes.as_str()), ("555 0100", "Likes tea"));
        assert_eq!(app.groups.members(1, false).unwrap(), vec![1], "the group follows the merged card");
        assert_eq!(app.prefs.me, Some(1));
        assert_eq!(app.screen, Screen::Detail(1), "the open card that was merged away is replaced by the kept one");
        assert!(app.status.as_deref().unwrap().starts_with("Merged 1 duplicate"), "{:?}", app.status);
        assert!(app.listed().is_empty(), "nothing is a duplicate any more");
        app.merge_duplicates(&ui, true);
        assert_eq!(app.status.as_deref(), Some("There are no duplicates to merge"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn merge_all_merges_every_group_in_one_save() {
        let list = [
            Contact { email: "a@x.is".into(), ..contact(1, "Anna As") },
            Contact { email: "a@x.is".into(), ..contact(2, "A. As") },
            Contact { phone: "555 0300".into(), ..contact(3, "Cara Cole") },
            Contact { phone: "555 0300".into(), ..contact(4, "C. Cole") },
            contact(5, "Dan Dahl"),
        ];
        let path = file_with("merge-all", &list);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.merge_duplicates(&ui, true);
        assert_eq!(store::load(&path).iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3, 5]);
        assert!(app.status.as_deref().unwrap().starts_with("Merged 2 duplicate cards into 2 cards"), "{:?}", app.status);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_smart_list_is_made_in_the_editor_shown_in_the_sidebar_edited_and_deleted() {
        let path = file_with(
            "smart-ui",
            &[Contact { company: "Acme".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg"), Contact { company: "Acme Ltd".into(), ..contact(3, "Cara Cole") }],
        );
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.menu_action(&ui, MENU_NEW_LIST));
        assert_eq!(app.screen, Screen::SmartEditor(None));
        assert_eq!(app.focus, Some(SMART_NAME));
        assert!(app.editing(), "an open smart list editor counts as unsaved work");
        app.key(&ui, Out::Text("Acme people".into()), 0.0);
        app.activate(&ui, Target::SmartField(0), 0.0, 0.0, 0.0);
        assert_eq!(app.smart_rules[0].field, RuleField::Company);
        app.activate(&ui, Target::Field(SMART_VALUE), 0.0, 0.0, 0.0);
        assert_eq!(app.focus, Some(SMART_VALUE));
        app.key(&ui, Out::Text("acme".into()), 0.0);
        app.bar(&ui, BAR_DONE);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.screen, Screen::List);
        assert_eq!(app.filter, Filter::UserList(0));
        assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Anna As", "Cara Cole"]);
        assert!(app.sidebar_rows().iter().any(|row| row.0 == LIST_USER && row.1 == "Acme people"));
        assert_eq!(Filter::from_sidebar_id(LIST_USER), Filter::UserList(0));
        assert!(sidebar_node(LIST_USER as u64));
        assert_eq!(group_file::load_lists(&path.with_file_name(SMART_FILE)).unwrap().len(), 1);
        assert_eq!(Contacts::new(&ui, 1200, 780, path.clone()).lists, app.lists, "the list is there after a restart");

        assert!(app.menu_action(&ui, MENU_EDIT_LIST));
        assert_eq!(app.screen, Screen::SmartEditor(Some(0)));
        assert_eq!(&*app.smart_name, "Acme people");
        assert_eq!(&*app.smart_rules[0].value, "acme");
        assert!(app.activate_control(&ui, NODE_SMART_ADD));
        assert_eq!(app.smart_rules.len(), 2);
        assert_eq!(app.focus, Some(SMART_VALUE + 1), "the new rule's value is ready to type");
        app.key(&ui, Out::Text("bo".into()), 0.0);
        assert!(app.activate_control(&ui, NODE_SMART_ANY));
        assert!(app.smart_any);
        app.bar(&ui, BAR_DONE);
        assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Anna As", "Bo Berg", "Cara Cole"], "any rule");

        assert!(app.menu_action(&ui, MENU_EDIT_LIST));
        app.activate(&ui, Target::SmartRemove(1), 0.0, 0.0, 0.0);
        assert_eq!(app.smart_rules.len(), 1);
        app.activate(&ui, Target::SmartRemove(0), 0.0, 0.0, 0.0);
        assert_eq!(app.smart_rules.len(), 1, "a list keeps its last rule");
        app.activate(&ui, Target::SmartDelete, 0.0, 0.0, 0.0);
        assert!(app.confirm_delete, "the first press asks");
        assert_eq!(app.lists.len(), 1);
        app.activate(&ui, Target::SmartDelete, 0.0, 0.0, 0.0);
        assert!(app.lists.is_empty());
        assert_eq!(app.filter, Filter::All);
        assert_eq!(app.screen, Screen::List);
        assert!(group_file::load_lists(&path.with_file_name(SMART_FILE)).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_smart_list_without_a_name_or_a_value_is_refused_and_tab_skips_rules_without_a_value() {
        let path = file_with("smart-ui-bad", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.new_list(&ui);
        app.bar(&ui, BAR_DONE);
        assert!(app.error.as_deref().unwrap().contains("needs a name"), "{:?}", app.error);
        assert_eq!(app.screen, Screen::SmartEditor(None), "the editor stays open");
        app.key(&ui, Out::Text("Friends".into()), 0.0);
        app.bar(&ui, BAR_DONE);
        assert!(app.error.is_some(), "the rule has no value yet");
        app.add_rule(&ui);
        app.smart_rules[1].condition = Condition::IsSet;
        assert_eq!(app.smart_focus_order(), vec![SMART_NAME, SMART_VALUE], "a rule that needs no value is skipped");
        app.focus_field(SMART_NAME);
        app.key(&ui, Out::Named("Tab"), 0.0);
        assert_eq!(app.focus, Some(SMART_VALUE));
        app.key(&ui, Out::Text("an".into()), 0.0);
        app.key(&ui, Out::Named("Escape"), 0.0);
        assert_eq!(app.screen, Screen::List, "Escape leaves the editor without saving");
        assert!(app.lists.is_empty());
        assert!(!path.with_file_name(SMART_FILE).exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn selection_mode_marks_rows_instead_of_opening_them_and_makes_a_group() {
        let path = file_with("select", &[contact(1, "Anna As"), contact(2, "Bo Berg"), contact(3, "Cara Cole")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.menu_action(&ui, MENU_SELECT));
        assert!(app.selecting);
        app.activate(&ui, Target::Row(0), 0.0, 0.0, 0.0);
        app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
        assert_eq!(app.screen, Screen::List, "a press only marks the row");
        assert_eq!(app.selection, BTreeSet::from([1, 2]));
        assert_eq!(app.controls(&ui).iter().filter(|n| n.id >= ROW_NODE && n.value == "selected").count(), 2, "assistive technology sees the marked contacts");
        app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
        assert_eq!(app.selection, BTreeSet::from([1]), "a second press takes the mark off");
        app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
        let bar: Vec<String> = app.list_bar().items.iter().filter(|i| i.id != 0 && i.id != BAR_SIDEBAR).map(|i| i.label.clone()).collect();
        assert_eq!(bar, ["Group Selection", "Merge", "Done"]);
        assert!(app.menu_action(&ui, MENU_GROUP_SELECTION));
        assert!(!app.selecting && app.selection.is_empty());
        assert_eq!(app.filter, Filter::Group(1));
        assert_eq!(app.groups.members(1, false).unwrap(), vec![1, 2]);
        assert_eq!(app.status.as_deref(), Some("Made Group 1 from 2 contacts"));
        let (saved, ids) = group_file::load(&path.with_file_name(GROUPS_FILE)).unwrap();
        assert_eq!(saved.members(ids[0], false).unwrap(), vec![1, 2]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn selected_cards_merge_into_the_first_and_escape_leaves_selection_mode() {
        let path = file_with(
            "select-merge",
            &[Contact { email: "a@x.is".into(), ..contact(1, "Anna As") }, Contact { phone: "555 0100".into(), ..contact(2, "Bo Berg") }, contact(3, "Cara Cole")],
        );
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.menu_action(&ui, MENU_SELECT);
        app.activate(&ui, Target::Row(0), 0.0, 0.0, 0.0);
        app.menu_action(&ui, MENU_MERGE_SELECTION);
        assert_eq!(app.status.as_deref(), Some("Choose at least two contacts to merge"));
        assert!(app.selecting, "the choice is kept");
        app.activate(&ui, Target::Row(1), 0.0, 0.0, 0.0);
        assert!(app.menu_action(&ui, MENU_MERGE_SELECTION));
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.selecting);
        let saved = store::load(&path);
        assert_eq!(saved.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3]);
        let kept = saved.iter().find(|c| c.id == 1).unwrap();
        assert_eq!((kept.name.as_str(), kept.phone.as_str(), kept.email.as_str()), ("Anna As", "555 0100", "a@x.is"));
        app.menu_action(&ui, MENU_SELECT);
        app.key(&ui, Out::Named("Escape"), 0.0);
        assert!(!app.selecting);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn my_card_is_marked_found_again_and_forgotten_when_deleted() {
        let path = file_with("my-card", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.go_to_my_card(&ui);
        assert!(app.status.as_deref().unwrap().contains("No card is My Card yet"), "{:?}", app.status);
        app.open_detail(&ui, 2);
        assert!(app.menu_action(&ui, MENU_MY_CARD));
        assert_eq!(app.prefs.me, Some(2));
        let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved.me, Some(2), "My Card is saved with the display settings");
        app.close_detail(&ui);
        app.query.push_str("zzz");
        app.rebuild(&ui);
        assert!(app.listed().is_empty());
        assert!(app.menu_action(&ui, MENU_GO_MY_CARD));
        assert_eq!(app.screen, Screen::Detail(2));
        assert_eq!(app.listed().len(), 2, "going to My Card clears the search");
        app.remove_selected(&ui);
        assert_eq!(app.prefs.me, None);
        let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved.me, None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn recent_changes_are_kept_between_runs_and_forgotten_when_deleted() {
        let path = file_with("recent", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.recent_ids().is_empty(), "the contacts loaded at launch are not changes");
        app.open_detail(&ui, 2);
        app.toggle_favorite(&ui);
        assert_eq!(app.recent_ids(), BTreeSet::from([2]));
        let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert_eq!(saved.recent, BTreeSet::from([2]), "the change is saved with the display settings");

        let next_run = Contacts::new(&ui, 1200, 780, path.clone());
        assert_eq!(next_run.recent_ids(), BTreeSet::from([2]), "the change is still listed in the next run");

        app.remove_selected(&ui);
        let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert!(saved.recent.is_empty(), "a deleted contact leaves the recent changes");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn next_and_previous_card_step_along_the_list_and_stop_at_the_ends() {
        let path = file_with("step", &[contact(1, "Anna As"), contact(2, "Bo Berg"), contact(3, "Cara Cole")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(app.menu_action(&ui, MENU_NEXT));
        assert_eq!(app.screen, Screen::Detail(1), "with no card open, next is the first card");
        app.step_card(&ui, true);
        app.step_card(&ui, true);
        assert_eq!(app.screen, Screen::Detail(3));
        app.step_card(&ui, true);
        assert_eq!(app.screen, Screen::Detail(3), "the last card has no next card");
        assert!(app.menu_action(&ui, MENU_PREVIOUS));
        assert_eq!(app.screen, Screen::Detail(2));
        app.close_detail(&ui);
        app.step_card(&ui, false);
        assert_eq!(app.screen, Screen::Detail(3), "with no card open, previous is the last card");
        app.edit_contact(&ui, Some(3));
        app.step_card(&ui, false);
        assert!(app.editing(), "stepping does not leave an open form");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_list_scrolls_just_enough_to_show_a_row() {
        assert_eq!(reveal_offset(100.0, 44.0, 0.0, 400.0), 0.0, "already in view");
        assert_eq!(reveal_offset(500.0, 44.0, 0.0, 400.0), 144.0, "below the window: its bottom lines up with the window's");
        assert_eq!(reveal_offset(100.0, 44.0, 300.0, 400.0), 76.0, "above the window: its section letter shows too");
        assert_eq!(reveal_offset(10.0, 44.0, 300.0, 400.0), 0.0, "never before the top");
    }

    #[test]
    fn a_table_in_the_import_folder_is_mapped_added_and_becomes_the_last_import_list() {
        let path = file_with("import-csv", &[contact(1, "Anna As")]);
        put_import(&path, "people.csv", "Name,Phone,Email,Fax\nBo Berg,555 0200,bo@x.is,555 9999\nCara Cole,,cara@x.is,\nAnna As,,,\n");
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert!(!app.sidebar_rows().iter().any(|row| row.0 == LIST_LAST_IMPORT), "no import yet");
        app.import_vcards(&ui);
        assert!(app.error.is_none(), "{:?}", app.error);
        let status = app.status.clone().unwrap();
        assert!(status.contains("Imported 2 contacts") && status.contains("skipped 1 duplicates"), "{status}");
        assert!(status.contains("3 columns mapped, ignored: Fax"), "{status}");
        assert_eq!(app.last_import_ids(), BTreeSet::from([2, 3]));
        assert!(app.sidebar_rows().iter().any(|row| row.0 == LIST_LAST_IMPORT && row.1 == "Last Import"));
        assert!(app.menu_action(&ui, MENU_LAST_IMPORT));
        assert_eq!(app.filter, Filter::LastImport);
        assert_eq!(app.listed().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Bo Berg", "Cara Cole"]);
        let again = Contacts::new(&ui, 1200, 780, path.clone());
        assert_eq!(again.last_import_ids(), BTreeSet::from([2, 3]), "the last import is remembered");
        app.close_detail(&ui);
        put_import(&path, "later.ldif", "dn: cn=Dan Dahl\nobjectClass: person\ncn: Dan Dahl\nmail: dan@x.is\n");
        std::fs::remove_file(data_dir_of(&path).join("Import/people.csv")).unwrap();
        app.import_vcards(&ui);
        assert_eq!(app.last_import_ids(), BTreeSet::from([4]), "a new import replaces the list");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_import_that_adds_nothing_keeps_the_last_import_list() {
        let path = file_with("import-same", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.prefs.last_import = BTreeSet::from([1]);
        put_import(&path, "same.csv", "Name\nAnna As\n");
        app.import_vcards(&ui);
        assert_eq!(app.last_import_ids(), BTreeSet::from([1]));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_archive_backs_up_every_data_file_and_restores_it_after_a_second_confirmation() {
        let path = file_with("archive", &[contact(1, "Anna As"), contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.new_group(&ui);
        app.groups.add_member(1, 2).unwrap();
        app.save_groups();
        app.export_archive();
        assert!(app.error.is_none(), "{:?}", app.error);
        let archive_file = data_dir_of(&path).join("Export").join(ARCHIVE_FILE);
        let text = std::fs::read_to_string(&archive_file).unwrap();
        assert!(text.starts_with("bifrost-contacts-archive 1\nfile\tcontacts.txt\t"), "{text}");
        assert!(text.contains("file\tgroups.txt\t"), "{text}");

        app.open_detail(&ui, 1);
        app.remove_selected(&ui);
        app.filter = Filter::Group(1);
        app.delete_group(&ui);
        assert_eq!(app.contacts.len(), 1);

        app.restore_archive(&ui);
        assert!(app.status.as_deref().unwrap().contains("Put a Contacts archive"), "{:?}", app.status);
        std::fs::create_dir_all(data_dir_of(&path).join("Import")).unwrap();
        std::fs::copy(&archive_file, data_dir_of(&path).join("Import").join(ARCHIVE_FILE)).unwrap();
        assert!(app.menu_action(&ui, MENU_RESTORE_ARCHIVE));
        assert!(app.confirm_restore, "the first choice only asks");
        assert_eq!(app.contacts.len(), 1);
        assert!(app.status.as_deref().unwrap().contains("Choose Restore from Archive again"), "{:?}", app.status);
        assert!(app.menu_action(&ui, MENU_RESTORE_ARCHIVE));
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.contacts.len(), 2, "both contacts are back");
        assert_eq!(app.groups.members(1, false).unwrap(), vec![2], "and the group with its member");
        assert_eq!(app.status.as_deref(), Some("Restored 2 contacts from contacts.bcarchive"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn choosing_something_else_between_the_two_restore_choices_cancels_the_restore() {
        let path = file_with("archive-cancel", &[contact(1, "Anna As")]);
        put_import(&path, ARCHIVE_FILE, &archive::pack(&[("contacts.txt".to_string(), "springboard-contacts-v1\n".to_string())]));
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.menu_action(&ui, MENU_RESTORE_ARCHIVE);
        assert!(app.confirm_restore);
        app.menu_action(&ui, MENU_SIDEBAR);
        assert!(!app.confirm_restore);
        assert_eq!(app.contacts.len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_damaged_archive_is_refused_and_a_half_good_one_puts_the_old_data_back() {
        let path = file_with("archive-bad", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        put_import(&path, ARCHIVE_FILE, "not an archive\n");
        app.confirm_restore = true;
        app.restore_archive(&ui);
        assert_eq!(app.error.as_deref(), Some("This file is not a Contacts archive"));
        assert_eq!(app.contacts.len(), 1);

        // a contacts file that reads and a groups file that does not
        let files = vec![
            ("contacts.txt".to_string(), "springboard-contacts-v1\n7\tBo\t\t\t\t0\n".to_string()),
            ("groups.txt".to_string(), "garbage\n".to_string()),
        ];
        put_import(&path, ARCHIVE_FILE, &archive::pack(&files));
        app.error = None;
        app.confirm_restore = true;
        app.restore_archive(&ui);
        assert_eq!(app.error.as_deref(), Some("The archive could not be read, so nothing was restored"));
        assert_eq!(app.contacts.len(), 1);
        assert_eq!(store::load(&path)[0].name, "Anna As", "the old contacts file is back");
        assert!(!path.with_file_name(GROUPS_FILE).exists(), "the groups file the archive brought is gone again");

        put_import(&path, ARCHIVE_FILE, &archive::pack(&[("evil.sh".to_string(), "x".to_string())]));
        app.error = None;
        app.confirm_restore = true;
        app.restore_archive(&ui);
        assert!(app.error.as_deref().unwrap().contains("does not use: evil.sh"), "{:?}", app.error);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn export_as_pdf_writes_the_list_or_the_open_card_and_print_without_a_service_says_so() {
        let path = file_with("pdf", &[Contact { phone: "555 0100".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.export_pdf();
        assert!(app.error.is_none(), "{:?}", app.error);
        let list = std::fs::read(data_dir_of(&path).join("Export/Contacts.pdf")).unwrap();
        assert!(list.starts_with(b"%PDF"), "a PDF file");
        app.open_detail(&ui, 1);
        assert!(app.menu_action(&ui, MENU_EXPORT_PDF));
        assert!(data_dir_of(&path).join("Export/Anna As.pdf").exists(), "the open card is saved under its title");
        let (text, title) = app.printable();
        assert_eq!(title, "Anna As");
        assert!(text.contains("555 0100"), "{text}");
        app.close_detail(&ui);
        app.print_labels();
        assert_eq!(app.status.as_deref(), Some("None of the contacts shown has an address"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn file_names_for_exports_keep_letters_and_drop_the_rest() {
        assert_eq!(file_stem("Anna As"), "Anna As");
        assert_eq!(file_stem("A/B: C?"), "A_B_ C_");
        assert_eq!(file_stem("///"), "contact");
        assert_eq!(file_stem("   "), "contact");
        assert_eq!(file_stem(&"x".repeat(60)).len(), 40);
    }

    #[test]
    fn the_settings_switches_are_saved_and_change_what_the_app_does() {
        let path = file_with("switches", &[Contact { notes: "private".into(), ..contact(1, "Anna As") }]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        app.screen = Screen::Settings;
        let rows = app.settings_layout().switches;
        assert_eq!(rows.len(), 2 + N_DETAILS);
        assert!(app.controls(&ui).iter().any(|n| n.id == NODE_SETTING + 1 && n.label == "Include Notes in vCards" && n.value == "on"));
        assert!(app.activate_control(&ui, NODE_SETTING + 1));
        assert!(!app.prefs.export_notes);
        app.export_vcards(exchange::Version::V3);
        let exported = std::fs::read_to_string(data_dir_of(&path).join("Export/contacts-3.0.vcf")).unwrap();
        assert!(!exported.contains("NOTE"), "{exported}");
        let birthday = 2 + Detail::Birthday.index();
        app.toggle_setting(Setting::Template(Detail::Birthday.index()));
        assert!(app.setting_on(Setting::Template(Detail::Birthday.index())));
        assert_eq!(Setting::all()[birthday], Setting::Template(Detail::Birthday.index()));
        app.toggle_setting(Setting::Nicknames);
        let (_, _, saved) = display_file::load(&path.with_file_name(DISPLAY_FILE)).unwrap();
        assert!(!saved.export_notes && saved.prefer_nicknames);
        assert_eq!(saved.template, BTreeSet::from([Detail::Birthday]));
        app.screen = Screen::List;
        app.edit_contact(&ui, None);
        assert!(app.detail_shown[Detail::Birthday.index()], "a new card shows the template's fields");
        assert!(!app.detail_shown[Detail::Url.index()]);
        app.cancel_editor(&ui);
        app.edit_contact(&ui, Some(1));
        assert!(!app.detail_shown[Detail::Birthday.index()], "an old card shows only the fields it has");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_settings_screen_scrolls_to_reach_every_switch() {
        let path = file_with("settings-scroll", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 480, path.clone());
        app.menu_action(&ui, MENU_SIDEBAR);
        app.bar(&ui, BAR_SETTINGS);
        assert_eq!(app.screen, Screen::Settings);
        let layout = app.settings_layout();
        assert!(layout.height > app.h, "eleven switches do not fit a short window");
        let last = layout.switches.last().unwrap().1;
        assert!(last.y + last.h > app.h);
        app.scroll.set_max(layout.height - app.h);
        app.scroll.jump_to(layout.height - app.h);
        let scrolled = app.settings_layout().switches.last().unwrap().1;
        assert!(scrolled.y + scrolled.h <= app.h, "after scrolling the last switch is in view");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_intents_open_a_card_create_a_contact_and_search() {
        let path = file_with("intents", &[Contact { phone: "555 0100".into(), ..contact(1, "Anna As") }, contact(2, "Bo Berg")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        assert_eq!(app.open_card(&ui, "bo berg").unwrap(), (2, "Bo Berg".to_string()));
        assert_eq!(app.screen, Screen::Detail(2));
        assert_eq!(app.open_card(&ui, "1").unwrap().0, 1, "an identifier works too");
        assert_eq!(app.open_card(&ui, "555 0100").unwrap().0, 1, "so does a search hit");
        assert!(app.open_card(&ui, "Nobody").unwrap_err().contains("No contact matches"));
        assert!(app.open_card(&ui, "  ").is_err());

        let id = app.create_contact(&ui, "Cara Cole", "555 0300", "", "Acme").unwrap();
        assert_eq!(id, 3);
        assert_eq!(app.screen, Screen::Detail(3), "the new card is opened");
        let saved = store::load(&path);
        assert!(saved.iter().any(|c| c.id == 3 && c.phone == "555 0300" && c.company == "Acme"));
        let twice = app.create_contact(&ui, "Cara Cole", "", "", "").unwrap_err();
        assert!(twice.contains("already in the contacts"), "{twice}");
        assert!(app.create_contact(&ui, "", "", "", "").is_err());

        let found = app.search_contacts(&ui, "berg");
        assert_eq!(found, vec![(2, "Bo Berg".to_string(), String::new(), String::new())]);
        assert_eq!(&*app.query, "berg", "the search field shows the query");
        assert_eq!(app.search_contacts(&ui, "").len(), 3);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn card_value_rows_past_the_tenth_get_their_own_control_ids() {
        assert_eq!(copy_node(0), NODE_COPY);
        assert_eq!(copy_node(9), NODE_COPY + 9);
        assert_eq!(copy_node(10), NODE_COPY_MORE + 10);
        let path = file_with("many-rows", &[contact(1, "Anna As")]);
        let ui = test_ui();
        let mut app = Contacts::new(&ui, 1200, 780, path.clone());
        let mut card = CardDetails::default();
        for detail in Detail::ALL {
            card.set(detail, "value");
        }
        app.details.set(1, card);
        let mut ann = contact(1, "Anna As");
        ann.phone = "1".into();
        ann.email = "a@x.is".into();
        ann.address = "Gata 1".into();
        app.contacts = vec![ann];
        for n in 0..extras::MAX_EXTRAS {
            assert!(app.extras.add(1, Extra { kind: ValueKind::Phone, label: "home".into(), value: format!("55{n}") }));
        }
        app.open_detail(&ui, 1);
        let rows = app.rows_of(app.selected().unwrap());
        assert!(rows.len() > 10, "{} rows", rows.len());
        let controls = app.controls(&ui);
        let copies: Vec<u64> = controls.iter().filter(|n| n.label.starts_with("Copy ") && n.id != NODE_COPY_CONTACT).map(|n| n.id).collect();
        assert_eq!(copies.len(), rows.len());
        let unique: BTreeSet<u64> = copies.iter().copied().collect();
        assert_eq!(unique.len(), copies.len(), "no two controls share an id");
        let last = *copies.last().unwrap();
        assert!(app.activate_control(&ui, last), "the last row's control is found and pressed");
        assert!(app.toast_until.is_some(), "its value was copied");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
