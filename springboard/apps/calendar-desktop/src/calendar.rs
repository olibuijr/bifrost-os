//! Golden Gate Golden Gate Calendar: a full-size window with the unified toolbar (`chrome.rs`), a Mac month grid with
//! each day's events in its cell, Day/Week/Year views and an event form. Colours follow Light and Dark.
pub mod alerts;
pub mod archive;
pub mod birthdays;
mod calmenu;
mod chrome;
pub mod clip;
mod colours;
pub mod connect;
mod drag;
mod eventpop;
pub mod exchange;
pub mod extras;
mod inviting;
mod linesheet;
mod lookup;
mod minimonth;
mod goto;
mod pickers;
mod quickentry;
pub mod holidays;
pub mod ics;
pub mod intents;
pub mod invitees;
pub mod layout;
pub mod notes;
pub mod overlay;
pub mod places;
pub mod prefs;
pub mod printgrid;
pub mod printing;
pub mod quick;
pub mod recur;
pub mod reminders;
pub mod rsvp;
pub mod search;
pub use springboard::shell::calendar::{store, Date};
pub mod subscribe;
pub mod timeview;
mod viewzone;
pub mod window;
pub mod zones;

use bifrost_app::layout::top_inset;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_XL, SPACE_XS, SPACE_XXS};
use bifrost_app::kit::ui::{MAC_LARGE_TITLE_PT};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT};
use springboard::layout::in_rect as within;
use springboard::shell::cache::TextCache;
use springboard::shell::clock::{tz, Wall};
use springboard::shell::controls::draw_switch;
use springboard::shell::keyboard::Out;
use springboard::shell::textfield::TextField;
use springboard::shell::ui::{centred, home_zone, Ui, Weight, WHITE};
use bifrost_app::kit::colors;
use bifrost_app::controls::calendar_participant::Participant;
use bifrost_app::kit::ui::{CARD_BG, LABEL, LABEL_2, LABEL_3, SEPARATOR_C};
use bifrost_app::kit::ui::{MAC_TITLE2_PT, MAC_TITLE3_PT};
use springboard::shell::Event;
use bifrost_app::controls::calendar_store_changes::StoreChangeHub;
use springboard::deildu::scroll::Scroller;
use springboard::gfx::Surface;
use std::borrow::Cow;
use std::path::PathBuf;
use store::CalendarEvent;
use timeview::View;
use bifrost_app::kit::ui::{SPACE_M, SPACE_S};
/// The card corner radius (grouped cards, the Undo bar): `prefs_layout::m::CARD_RADIUS`, 13 pt.
const CARD_R: f32 = bifrost_app::controls::prefs_layout::m::CARD_RADIUS;

/// The today marker and the time view's now line: systemRed, the calendar's one other red (COL-03).
pub fn red() -> u32 {
    colors::SYSTEM_RED.now()
}

/// The calendar's accent: links, the Undo button, event tints (systemBlue in Light and Dark).
fn accent() -> u32 {
    colors::LINK.now()
}

/// The month grid's top (under the weekday names), and one event line in a day cell (points).
const GRID_TOP: f32 = 170.0;
const CELL_LINE: f32 = 16.0;
/// Where a cell's event lines start below its top (under the day number).
const CELL_HEAD: f32 = 28.0;
/// How long the Undo bar stays after a delete.
const UNDO_MS: f64 = 5000.0;
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// ISO 8601 week number (weeks start on Monday; week 1 holds the year's first Thursday).
fn iso_week(day: Date) -> u32 {
    let monday_based = (day.weekday() + 6) % 7;
    let thursday = Date::from_days(day.days() - monday_based as i64 + 3);
    let jan1 = Date { year: thursday.year, month: 1, day: 1 };
    ((thursday.days() - jan1.days()) / 7 + 1) as u32
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Today,
    Prev,
    Next,
    Add,
    Day(u32),
    Row(u64),
    Agenda,
    Cancel,
    Save,
    Delete,
    Duplicate,
    Export,
    ExportCalendar,
    WeekNumbers,
    UndoDelete,
    Title,
    Location,
    Notes,
    /// Offers the address written in the notes as the location.
    FillAddress,
    /// Copies the phone number written in the notes to the pasteboard.
    CopyPhone,
    /// Opens the location in Maps (`places.rs`), under the Location field while it has text.
    ShowInMaps,
    /// A read-only row: the invitees of an imported event and how each one answered.
    Invitees,
    /// Accept, Maybe and Decline on an invitation the account has not answered (an index into `rsvp::Answer::ALL`).
    Rsvp(usize),
    /// View > Show Reminders.
    ToggleReminders,
    /// View > Show Declined Events.
    ToggleDeclined,
    Toggle(Slot),
    DateStep(i32),
    MonthStep(i32),
    View(View),
    Pick(Date),
    MonthOf(u32),
    TimeStep(i32),
    AllDay,
    /// show or hide the calendar list
    Sidebar,
    /// Go > Go to Date: open the sheet, step its day, month or year, jump to today in it, and accept it.
    GoOpen,
    GoDay(i32),
    GoMonth(i32),
    GoYear(i32),
    GoToday,
    GoConfirm,
    /// The toolbar's search field, its clear button and a result row (`lookup.rs`).
    SearchField,
    SearchClear,
    SearchHit(usize),
    /// Event > Quick Event: open the sheet, and add the event it describes.
    QuickOpen,
    QuickAdd,
    /// A row of the calendar list: shows or hides that calendar (`overlay.rs`).
    Layer(u32),
    /// The Repeat and Alert rows, which expand their pickers below (`pickers.rs`).
    Repeat,
    Alert,
    /// The Repeat picker: frequency, the interval stepper, the weekday toggles (a row, and a column of it), the end
    /// rule, and the stepper of its count or its end date.
    RepeatFreq,
    RepeatEvery(i32),
    RepeatDays,
    RepeatDay(u8),
    RepeatEnds,
    RepeatCount(i32),
    RepeatUntil(i32),
    /// The Alert picker: a preset (an index into `alerts::ALERT_MINUTES`; 0 clears the list), the custom alarm's
    /// stepper, the row that adds it, and a row per custom alarm that removes it.
    AlarmPreset(u8),
    AlarmStep(i32),
    AlarmAdd,
    AlarmDrop(usize),
    /// The calendar, time zone and travel time rows, each cycled in place.
    Calendar,
    Zone,
    Travel,
    /// The line sheet's Done button (`linesheet.rs`).
    LineDone,
    /// The toolbar's Time Zone button: the next zone of the view time zone (`viewzone.rs`).
    ViewZone,
    /// The Go to Date sheet's typed date row (`goto.rs`).
    GoField,
    /// The colour sheet (`colours.rs`): a row of the palette, and Done.
    ColourPick(u8),
    ColourDone,
    /// The invitee rows of the event form (`inviting.rs`): the Add Invitees row with its line, a match of the address
    /// book to invite, and the Remove row of each invitee.
    AddInvitee,
    InviteeSuggest(usize),
    InviteeDrop(usize),
    /// The toolbar's "+": a new event with the quick-event popover beside it (`eventpop.rs`).
    NewPopover,
    /// A part of the quick-event popover, and a press outside it (which closes it).
    Pop(eventpop::PopPart),
    PopDismiss,
    /// The calendar list's mini month (`minimonth.rs`): its chevrons and a day.
    MiniPrev,
    MiniNext,
    MiniDay(Date),
}

/// Which of the Starts / Ends rows has its picker expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Start,
    End,
}

#[derive(Clone)]
struct Draft {
    event: CalendarEvent,
    existing: bool,
    open: Option<Slot>,
    /// Invitees read from the imported event this entry came from (read-only).
    invitees: Vec<Participant>,
    /// The Repeat and Alert pickers are expanded below their rows.
    repeat_open: bool,
    alert_open: bool,
    /// The working repeat rule (None: never) and alarms (minutes before the start). `commit` saves them.
    rule: Option<recur::Rule>,
    alarms: Vec<u32>,
    /// The custom alarm the Alert picker's stepper is on, in minutes.
    custom_alarm: u32,
    /// What the saved event keeps beside the store: its calendar, zone, travel time and invitees (`extras.rs`).
    extra: extras::Extra,
    /// The line of the Add Invitees row while it is open (`inviting.rs`).
    invitee_line: Option<bifrost_app::controls::TextField>,
}

pub struct Calendar {
    /// the content column's width (the window less the calendar list) and its left edge, in px
    w: i32,
    x0: f32,
    full_w: i32,
    /// View > Hide Calendar List
    side_hidden: bool,
    h: i32,
    path: PathBuf,
    wall: Wall,
    today: Date,
    selected: Date,
    view: View,
    events: Vec<CalendarEvent>,
    /// Store changes (each save or import posts one), for caches and the alert state.
    changes: StoreChangeHub,
    alerts: alerts::AlertState,
    editor: Option<Draft>,
    /// Go to Date: the date the sheet shows while it is open (`goto.rs`).
    goto: Option<Date>,
    /// Go to Date: the date typed in the sheet's first row (`goto.rs`).
    goto_line: bifrost_app::controls::TextField,
    /// The colour sheet while it is open (`colours.rs`).
    colour_sheet: Option<colours::ColourSheet>,
    /// The people of the address book with an email address, read when the invitee line first opens (`inviting.rs`).
    book: Option<Vec<birthdays::Entry>>,
    /// The last press came from a mouse or trackpad, not a finger: a drag in the hour grid starts at once, and the grid
    /// is scrolled with the wheel (`drag.rs`). The app sets it from the type of the touch (`set_pointer`).
    pointer: bool,
    /// Quick Event: the line typed in the sheet while it is open (`quickentry.rs`).
    quick_line: Option<bifrost_app::controls::TextField>,
    /// The quick-event popover on a new event, while it is open (`eventpop.rs`), and the empty slot of the grid the last
    /// single click was on (a double-click there opens the popover).
    event_pop: Option<eventpop::EventPop>,
    /// The menu of calendars a Calendar pop-up button opened (the popover's or the form's), while it is open (`calmenu.rs`).
    cal_menu: Option<calmenu::CalMenu>,
    last_slot: Option<(Date, u16)>,
    /// The month the sidebar's mini month was paged to; None follows the selected day (`minimonth.rs`).
    mini_shown: Option<Date>,
    /// A press in the Day or Week grid that becomes a drag if it is held, and the drag itself (`drag.rs`).
    press: Option<drag::Press>,
    drag: Option<drag::Drag>,
    /// The toolbar's search field, whether it has the cursor, and the events it matches (`lookup.rs`).
    search_box: bifrost_app::controls::TextField,
    search_focused: bool,
    search_hits: Vec<search::Hit>,
    /// The event last opened or saved: what Copy and Cut act on while it is on the shown day (`clip.rs`).
    picked: Option<u64>,
    /// The calendars the user made and the extras of the events (`extras.rs`), and the extras of the last deleted
    /// event, which Undo puts back with it.
    extras: extras::Extras,
    undo_extra: Option<extras::Extra>,
    /// The line sheet (New Calendar, Rename Calendar, New Calendar Subscription), when it is open (`linesheet.rs`).
    line_sheet: Option<linesheet::LineSheet>,
    /// Pixels per point, as of the last layout: the editor's scroll is in pixels and its rows in points.
    px_per_pt: f32,
    /// The time view's scroll, kept while the event form is open and restored when it closes.
    saved_scroll: f32,
    /// Read-only events of the holidays and the subscribed calendars for the years around the shown one
    /// (`overlay.rs`), the year and `overlay_version` they were worked out for, and the subscriptions themselves.
    overlay: Vec<CalendarEvent>,
    overlay_for: Option<(i64, u64)>,
    overlay_version: u64,
    /// The account's own address (the CalDAV account's username): the invitations it can answer, and the events it declined.
    account_email: Option<String>,
    /// The saved events the account declined, worked out for the saved-event count `declined_for` (see `sync_declined`).
    declined: std::collections::HashSet<u64>,
    declined_for: Option<usize>,
    subs: Vec<subscribe::Subscription>,
    /// Title, Location, Notes.
    fields: [TextField; 3],
    /// Event editors shown in windows of their own (`window.rs`), the event a double-click asked to open, and the event
    /// the last single click selected (a double-click on it opens it).
    panes: Vec<window::Pane>,
    /// Editor windows whose event was deleted from the main window (Event > Delete Event, Backspace): the app closes them
    /// (`take_closed_windows`).
    closed_windows: Vec<u32>,
    open_request: Option<u64>,
    last_tap: Option<u64>,
    /// The click count of the press in progress, kept by the kit's counter with the system's double-click interval and
    /// slop (`ClickCounter`), as `NSEvent.clickCount` is: a double-click is two presses, not two releases, in time.
    clicks: bifrost_app::controls::text::ClickCounter,
    click_count: u8,
    /// Replies and invitations waiting for Mail (`rsvp.rs`); the app takes them with `take_outgoing`.
    outbox: Vec<rsvp::Outgoing>,
    /// A `maps:` link the editor asked to open (`places.rs`); the app opens it (`take_map_link`).
    map_link: Option<String>,
    labels: TextCache,
    scroll: Scroller,
    pressed: Option<Target>,
    start: Option<(f32, f32)>,
    scrolling: bool,
    field_touch: bool,
    error: Option<String>,
    /// Confirmation after an export, shown in place of an error.
    notice: Option<String>,
    /// A phone number the editor asked to copy; the app takes it (`take_copied`) and writes the pasteboard.
    copied: Option<String>,
    /// Desktop only: the editor offers Export Event, saved under ~/Documents.
    file_export: bool,
    /// The desktop's settings (`prefs.rs`): week numbers (the toggle is desktop-only, with `file_export`), the zoom of
    /// the hour grid, the first weekday and the defaults for a new event.
    prefs: prefs::Prefs,
    /// The last deleted event and when its Undo bar lapses.
    undo: Option<(CalendarEvent, f64)>,
    storage_blocked: bool,
    text_style: (f32, bool),
    dirty: bool,
}

fn today(wall: Wall, t: f64) -> Date {
    let c = tz::civil(wall.now(t), wall.offset);
    Date {
        year: c.year,
        month: c.month,
        day: c.day,
    }
}

fn time_label(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

impl Calendar {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    /// Desktop builds only: Export Event writes an .ics file to ~/Documents.
    pub fn enable_file_export(&mut self) {
        self.file_export = true;
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.full_w = w;
        self.w = w - self.x0 as i32;
        self.h = h;
    }

    pub fn new(ui: &Ui, w: i32, h: i32, path: PathBuf, wall: Wall) -> Self {
        let (events, error, storage_blocked) = match store::load(&path) {
            Ok(events) => (events, None, false),
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                (vec![], Some("Could not read saved events".into()), true)
            }
        };
        let today = today(wall, wall.at);
        let fields = window::new_fields(ui);
        let prefs = prefs::Prefs::load(&path);
        let subs = subscribe::load(&path);
        let mut extras = extras::Extras::load(&path);
        let live: Vec<u64> = events.iter().map(|e| e.id).collect();
        extras.retain_events(&live);
        let mut c = Self {
            w,
            x0: 0.0,
            full_w: w,
            side_hidden: false,
            h,
            path,
            wall,
            today,
            selected: today,
            view: View::Month,
            events,
            changes: StoreChangeHub::new(),
            alerts: alerts::AlertState::default(),
            error,
            notice: None,
            copied: None,
            file_export: false,
            prefs,
            undo: None,
            storage_blocked,
            text_style: (ui.text_scale, ui.bold),
            editor: None,
            goto: None,
            goto_line: bifrost_app::controls::TextField::new(),
            colour_sheet: None,
            book: None,
            pointer: false,
            quick_line: None,
            event_pop: None,
            cal_menu: None,
            last_slot: None,
            mini_shown: None,
            press: None,
            drag: None,
            search_box: bifrost_app::controls::TextField::new(),
            search_focused: false,
            search_hits: vec![],
            picked: None,
            extras,
            undo_extra: None,
            line_sheet: None,
            px_per_pt: 1.0,
            saved_scroll: 0.0,
            overlay: vec![],
            overlay_for: None,
            account_email: None,
            declined: Default::default(),
            declined_for: None,
            overlay_version: 0,
            subs,
            fields,
            panes: Vec::new(),
            closed_windows: Vec::new(),
            open_request: None,
            last_tap: None,
            clicks: bifrost_app::controls::text::ClickCounter::default(),
            click_count: 1,
            outbox: Vec::new(),
            map_link: None,
            labels: TextCache::default(),
            scroll: Scroller::new(),
            pressed: None,
            start: None,
            scrolling: false,
            field_touch: false,
            dirty: true,
        };
        c.layout_columns(ui);
        c.fit_scroll(ui);
        c
    }

    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
        let date = today(wall, wall.at);
        if self.today != date {
            self.today = date;
            self.dirty = true;
        }
    }

    pub fn needs_frame(&self) -> bool {
        self.dirty
    }
    /// The clock the calendar reads (its local zone offset applies to floating events).
    pub fn wall(&self) -> Wall {
        self.wall
    }
    /// The store-change generation: each save, delete or import moves it.
    pub fn store_generation(&self) -> u64 {
        self.changes.generation()
    }
    /// The .ics extras kept beside the store (repeat rules, exceptions), for the shell services.
    pub fn rich_events(&self) -> ics::IcsCalendar {
        exchange::load_rich(&exchange::rich_path(&self.path))
    }
    /// `rich_events` with the events that fire the birthday alerts (Settings > Default Birthday Alert, `birthdays.rs`)
    /// added, for the shell services that plan alerts and the Up Next list. Never written back to the .ics file.
    pub fn rich_events_with_birthdays(&self) -> ics::IcsCalendar {
        let mut rich = self.rich_events();
        rich.events.extend(birthdays::alert_events_at(&self.path, self.prefs.alert_birthday, self.today.year));
        rich
    }
    pub fn events(&self) -> &[CalendarEvent] {
        &self.events
    }
    pub fn selected(&self) -> Date {
        self.selected
    }
    /// Whether a text field in the event editor has focus (typing goes to it).
    pub fn keyboard_shown(&self) -> bool {
        self.editor.is_some() && (self.focused().is_some() || self.invitee_line_open())
    }

    pub fn describe(&self) -> String {
        format!("calendar_screen={}\ncalendar_date={}\ncalendar_events={}\ncalendar_day_events={}\ncalendar_keyboard={}\ncalendar_error={}\n",
            if self.editor.is_some() { "editor" } else { self.view.name() }, self.selected.iso(), self.events.len(), self.agenda().len(), self.keyboard_shown(), self.error.as_deref().unwrap_or(""))
    }

    fn agenda(&self) -> Vec<Cow<'_, CalendarEvent>> {
        self.shown_events().filter(|e| self.occurs(e, self.selected)).collect()
    }
    fn bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }
    /// The Title field's frame, for the layout tests.
    #[cfg(test)]
    fn title_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.field_frame(ui, 0)
    }
    fn field_frame(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let want = [Target::Title, Target::Location, Target::Notes][i];
        let (top, h) = self
            .rows()
            .into_iter()
            .find(|r| r.0 == want)
            .map_or((0.0, 44.0), |r| (r.1, r.2));
        (ui.pt(36.0), ui.pt(top), self.w as f32 - ui.pt(72.0), ui.pt(h))
    }
    /// Editor rows as (target, top pt, height pt), grouped like an bifrost form, moved up by the form's scroll.
    fn rows(&self) -> Vec<(Target, f32, f32)> {
        let scroll = self.scroll.offset() / self.px_per_pt.max(f32::EPSILON);
        self.editor_rows(scroll)
    }

    /// The editor's rows with the form scrolled up by `scroll` points.
    fn editor_rows(&self, scroll: f32) -> Vec<(Target, f32, f32)> {
        let Some(draft) = &self.editor else {
            return vec![];
        };
        let e = &draft.event;
        let mut items: Vec<Option<Target>> = vec![Some(Target::Title), Some(Target::Location), None];
        if places::search_link(self.fields[1].text()).is_some() {
            items.insert(2, Some(Target::ShowInMaps));
        }
        items.push(Some(Target::AllDay));
        items.push(Some(Target::Toggle(Slot::Start)));
        if draft.open == Some(Slot::Start) {
            items.extend([Some(Target::DateStep(0)), Some(Target::MonthStep(0))]);
            if !e.all_day {
                items.push(Some(Target::TimeStep(0)));
            }
        }
        if !e.all_day {
            items.push(Some(Target::Toggle(Slot::End)));
            if draft.open == Some(Slot::End) {
                items.extend([Some(Target::DateStep(0)), Some(Target::TimeStep(0))]);
            }
        }
        items.extend([None, Some(Target::Repeat)]);
        items.extend(pickers::repeat_targets(draft).into_iter().map(Some));
        items.push(Some(Target::Alert));
        items.extend(pickers::alarm_targets(draft).into_iter().map(Some));
        items.extend([Some(Target::Calendar), Some(Target::Zone), Some(Target::Travel), None, Some(Target::AddInvitee)]);
        items.extend(self.invitee_targets(draft).into_iter().map(Some));
        items.extend([None, Some(Target::Notes)]);
        if self.notes_address().is_some() {
            items.push(Some(Target::FillAddress));
        }
        if self.notes_phone().is_some() {
            items.push(Some(Target::CopyPhone));
        }
        if !draft.invitees.is_empty() {
            items.extend([None, Some(Target::Invitees)]);
        }
        if self.awaiting_reply(&draft.invitees) {
            items.extend([None, Some(Target::Rsvp(0)), Some(Target::Rsvp(1)), Some(Target::Rsvp(2))]);
        }
        if draft.existing {
            items.extend([None, Some(Target::Duplicate)]);
            if self.file_export {
                items.push(Some(Target::Export));
            }
            items.push(Some(Target::Delete));
        }
        let mut y = 108.0 - scroll;
        let mut out = vec![];
        for item in items {
            match item {
                Some(t) => {
                    out.push((t, y, 44.0));
                    y += 44.0;
                }
                None => y += 20.0,
            }
        }
        out
    }
    /// The first street address in the notes field (live text, before Save), if any.
    fn notes_address(&self) -> Option<String> {
        notes::address(self.fields[2].text())
    }

    /// End of an event as a date and minute of day.
    fn end_of(e: &CalendarEvent) -> (Date, u16) {
        let total = e.minute as i64 + e.duration as i64;
        (Date::from_days(e.date.days() + total / 1440), (total % 1440) as u16)
    }
    fn focus(&mut self, which: Option<usize>) {
        for (i, f) in self.fields.iter_mut().enumerate() {
            f.focused = Some(i) == which;
        }
        // a field that takes the cursor takes it from the invitee line
        if which.is_some() {
            if let Some(draft) = self.editor.as_mut() {
                draft.invitee_line = None;
            }
        }
    }
    fn focused(&self) -> Option<usize> {
        self.fields.iter().position(|f| f.focused)
    }
    fn fit_scroll(&mut self, ui: &Ui) {
        // the event form scrolls past the window's bottom when its pickers are open
        if self.editor.is_some() {
            let content = self.editor_rows(0.0).last().map_or(0.0, |r| r.1 + r.2) + 40.0;
            let visible = self.bottom(ui) / ui.s;
            self.scroll.set_max(ui.pt((content - visible).max(0.0)));
            return;
        }
        if self.searching() {
            self.scroll.set_max(self.results_scroll_max(ui));
            return;
        }
        if self.time_view() {
            self.scroll.set_max(self.time_scroll_max(ui));
            return;
        }
        // the month grid fits the window; only the event form scrolls past it
        let _ = ui;
        self.scroll.set_max(0.0);
    }

    /// The month grid in px: left edge (after the week-number column), cell width, top, row height.
    fn grid_geom(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let left = if self.prefs.week_numbers { ui.pt(28.0) } else { 0.0 };
        let top = ui.pt(GRID_TOP);
        ((left), (self.w as f32 - left) / 7.0, top, ((self.bottom(ui) - top) / 6.0).max(ui.pt(CELL_HEAD)))
    }

    /// The first day shown in the month grid (the first weekday on or before the 1st).
    fn grid_start(&self) -> Date {
        self.week_of(Date { day: 1, ..self.selected })
    }

    /// The events a day cell lists: up to as many lines as fit, the last one "n more" when some are left out.
    fn cell_events(&self, ui: &Ui, date: Date) -> (Vec<Cow<'_, CalendarEvent>>, usize) {
        let (.., rh) = self.grid_geom(ui);
        let room = ((rh - ui.pt(CELL_HEAD)) / ui.pt(CELL_LINE)).floor().max(0.0) as usize;
        let all: Vec<Cow<'_, CalendarEvent>> = self.shown_events().filter(|e| self.occurs(e, date)).collect();
        if all.len() <= room {
            return (all, 0);
        }
        let shown = room.saturating_sub(1);
        let more = all.len() - shown;
        (all.into_iter().take(shown).collect(), more)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        // the quick-event popover is transient: a press outside it closes it before it reaches anything else
        if self.event_pop.is_some() {
            return Some(self.pop_target(ui, x + self.x0, y));
        }
        if let Some(r) = self.search_rect(ui) {
            let k = ui.pt(1.0);
            if within(x + self.x0, y, (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h))) {
                let clear = bifrost_app::controls::search::hits_clear(r, !self.search_box.is_empty(), (x + self.x0) / k, y / k);
                return Some(if clear { Target::SearchClear } else { Target::SearchField });
            }
        }
        if let Some(t) = self.bar_target(ui, x + self.x0, y) {
            return Some(t);
        }
        // the calendar list is left of the content column
        if x < 0.0 {
            if let Some(t) = self.mini_target(ui, x + self.x0, y) {
                return Some(t);
            }
            return self.layer_at(ui, x + self.x0, y).map(Target::Layer);
        }
        if self.editor.is_none() && self.undo.is_some() && within(x, y, self.undo_rect(ui)) {
            return Some(Target::UndoDelete);
        }
        if x < 0.0 || y < ui.pt(top_inset() + bifrost_app::layout::UNIFIED_BAR_PT) || y >= self.bottom(ui) {
            return None;
        }
        let xp = x / ui.s;
        let yp = y / ui.s;
        let wp = self.w as f32 / ui.s;
        if self.goto.is_some() {
            return self.hit_goto(ui, x, y);
        }
        if self.colour_sheet.is_some() {
            return self.hit_colour(ui, x, y);
        }
        if self.quick_line.is_some() || self.line_sheet.is_some() {
            return None;
        }
        if self.searching() {
            return self.hit_results(ui, x, y);
        }
        if self.editor.is_some() {
            for (t, top, h) in self.rows() {
                if (top..top + h).contains(&yp) {
                    let side = |n: i32| {
                        if xp < 100.0 {
                            Some(-n)
                        } else if xp > wp - 100.0 {
                            Some(n)
                        } else {
                            None
                        }
                    };
                    return match t {
                        Target::DateStep(_) => side(1).map(Target::DateStep),
                        Target::MonthStep(_) => Some(Target::MonthStep(if xp < wp / 2.0 { -1 } else { 1 })),
                        Target::TimeStep(_) => side(15).map(Target::TimeStep),
                        Target::RepeatEvery(_) => side(1).map(Target::RepeatEvery),
                        Target::RepeatCount(_) => side(1).map(Target::RepeatCount),
                        Target::RepeatUntil(_) => side(1).map(Target::RepeatUntil),
                        Target::AlarmStep(_) => side(1).map(Target::AlarmStep),
                        Target::RepeatDays => Some(Target::RepeatDay(pickers::weekday_column(xp, wp))),
                        t => Some(t),
                    };
                }
            }
            return None;
        }
        if let Some(t) = self.hit_view(ui, x, y) {
            return Some(t);
        }
        if self.view != View::Month {
            return None;
        }
        let (left, cell, top, rh) = self.grid_geom(ui);
        if y >= top && x >= left {
            let col = (((x - left) / cell) as i32).clamp(0, 6);
            let row = (((y - top) / rh) as i32).clamp(0, 5);
            let date = self.grid_start().shift_days(row * 7 + col);
            let line = ((y - top - rh * row as f32 - ui.pt(CELL_HEAD)) / ui.pt(CELL_LINE)).floor();
            if line >= 0.0 {
                if let Some(e) = self.cell_events(ui, date).0.get(line as usize) {
                    return Some(Target::Row(e.id));
                }
            }
            if date.year == self.selected.year && date.month == self.selected.month {
                return Some(Target::Day(date.day));
            }
            return Some(Target::Pick(date));
        }
        None
    }

    fn open(&mut self, ui: &Ui, id: Option<u64>) {
        let event = if let Some(id) = id {
            let Some(e) = self.events.iter().find(|e| e.id == id) else {
                return;
            };
            e.clone()
        } else {
            CalendarEvent {
                id: store::next_id(&self.events),
                date: self.selected,
                minute: 9 * 60,
                duration: self.prefs.duration,
                all_day: false,
                title: String::new(),
                location: String::new(),
                notes: String::new(),
                repeat: 0,
                alert: self.prefs.alert, uid: String::new(),
            }
        };
        self.open_draft(ui, event, id.is_some());
    }

    /// Shows the editor on `event`: an existing one, or a new one whose Title field takes the focus.
    fn open_draft(&mut self, ui: &Ui, event: CalendarEvent, existing: bool) {
        self.fields[0].set_text(ui, &event.title);
        self.fields[1].set_text(ui, &event.location);
        self.fields[2].set_text(ui, &event.notes);
        self.focus((!existing).then_some(0));
        if existing {
            self.picked = Some(event.id);
        }
        let people = if existing {
            invitees::for_event(&self.rich_events(), &event.title, event.date, event.minute, event.all_day)
        } else {
            vec![]
        };
        // the time view's scroll comes back when the form closes; the form starts at its top
        self.saved_scroll = self.scroll.offset();
        self.scroll.jump_top();
        self.editor = Some(self.new_draft(event, existing, people));
        self.error = None;
        self.fit_scroll(ui);
    }

    /// File > New Event from Text: opens the editor on the event `text` describes (`quick::parse`). Empty text opens a
    /// plain New Event. The event is not saved until Save, like any other.
    pub fn new_event_from_text(&mut self, ui: &Ui, text: &str, t: f64) -> bool {
        if text.trim().is_empty() {
            self.open(ui, None);
            return true;
        }
        let q = quick::parse(text, today(self.wall, t));
        let event = CalendarEvent {
            id: store::next_id(&self.events),
            date: q.date.unwrap_or(self.selected),
            minute: q.minute.unwrap_or(9 * 60),
            duration: self.prefs.duration,
            all_day: false,
            title: q.title,
            location: q.location.unwrap_or_default(),
            notes: String::new(),
            repeat: 0,
            alert: self.prefs.alert, uid: String::new(),
        };
        self.open_draft(ui, event, false);
        self.dirty = true;
        true
    }

    /// The first phone number in the notes field (live text, before Save), if any.
    fn notes_phone(&self) -> Option<String> {
        notes::phone(self.fields[2].text())
    }

    /// Asks the app to copy the first phone number in the notes; the notice confirms it.
    fn copy_phone(&mut self) {
        if let Some(phone) = self.notes_phone() {
            self.notice = Some(format!("Copied {phone}"));
            self.copied = Some(phone);
        }
    }

    /// The phone number the editor last asked to copy, once (the app writes it to the pasteboard).
    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }

    /// The `maps:` link the editor asked to open, once (the app opens it with the shell).
    pub fn take_map_link(&mut self) -> Option<String> {
        self.map_link.take()
    }

    fn close(&mut self) {
        if self.editor.take().is_some() {
            let saved = self.saved_scroll;
            // the scroller still has the form's range, which is shorter than the hour grid's: widen it first, or the
            // grid comes back clamped (8 AM at the top became 5 AM, E2E W2-18). The next `fit_scroll` sets the real range.
            self.scroll.set_max(saved);
            self.scroll.jump_to(saved);
        }
        self.focus(None);
    }

    /// Persist first so errors never silently discard the user's draft or
    /// present an unsaved deletion as successful.
    /// Opens an unsaved copy of the event being edited, keeping its unsaved edits. Save adds it.
    fn duplicate(&mut self) {
        let Some(draft) = &self.editor else { return };
        let mut copy = draft.clone();
        copy.event.id = store::next_id(&self.events);
        copy.event.title = self.fields[0].text().trim().to_string();
        copy.event.location = self.fields[1].text().trim().to_string();
        copy.event.notes = self.fields[2].text().trim().to_string();
        copy.existing = false;
        copy.open = None;
        copy.invitees = vec![];
        copy.invitee_line = None;
        self.error = None;
        self.editor = Some(copy);
    }

    /// Writes the saved event as an .ics file under ~/Documents/Calendar Export.
    fn export_event(&mut self) {
        self.notice = None;
        let Some(draft) = &self.editor else { return };
        if !draft.existing {
            self.error = Some("Save the event before exporting it".into());
            return;
        }
        let id = draft.event.id;
        let Some(text) = self.export_event_ics(id) else {
            self.error = Some("This event is no longer saved".into());
            return;
        };
        let title = self.events.iter().find(|e| e.id == id).map(|e| e.title.clone()).unwrap_or_default();
        let safe: String = title
            .chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
            .collect();
        let name = match safe.trim() {
            "" => format!("Event {id}"),
            n => n.to_string(),
        };
        self.save_export(&name, &text);
    }

    /// Writes the whole calendar as Calendar.ics, the same folder as event exports.
    /// The Undo bar: a flat strip above the home zone, like Reminders and Notes.
    fn undo_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(SPACE_XL), self.bottom(ui) - ui.pt(56.0), self.w as f32 - ui.pt(SPACE_HUGE), ui.pt(44.0))
    }

    fn draw_undo(&mut self, s: &mut Surface, ui: &Ui) {
        if self.undo.is_none() {
            return;
        }
        let (x, y, w, h) = self.undo_rect(ui);
        s.fill_rrect(x, y, w, h, ui.pt(CARD_R), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(CARD_R), 1.0, SEPARATOR_C.now());
        self.text(s, ui, "Event deleted", MAC_BODY_PT, LABEL.now(), x + ui.pt(SPACE_L), y + h / 2.0);
        self.centre(s, ui, "Undo", MAC_BODY_PT, Weight::Semibold, accent(), x + w - ui.pt(SPACE_HUGE), y + h / 2.0);
    }

    /// Puts the deleted event back (saved again) while its Undo bar is up.
    fn undo_delete(&mut self, ui: &Ui, t: f64) {
        let Some((event, until)) = self.undo.take() else { return };
        if self.events.len() >= store::MAX_EVENTS {
            self.error = Some("Calendar is full".into());
            self.undo = Some((event, until));
            return;
        }
        let mut events = self.events.clone();
        events.push(event.clone());
        store::sort(&mut events);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                if let Some(extra) = self.undo_extra.take() {
                    self.extras.put(event.id, extra);
                }
                self.save_extras();
                self.selected = event.date;
                self.error = None;
                self.fit_scroll(ui);
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save. Your edit is still here.".into());
                self.undo = Some((event, t + UNDO_MS));
            }
        }
    }

    fn save_prefs(&mut self) {
        if let Err(e) = self.prefs.save(&self.path) {
            eprintln!("springboard: calendar: {e}");
        }
    }

    /// Writes the extras beside the events. Entries of events that are gone are dropped first, so an id the store
    /// hands out again starts with nothing attached.
    pub(super) fn save_extras(&mut self) {
        let live: Vec<u64> = self.events.iter().map(|e| e.id).collect();
        self.extras.retain_events(&live);
        if let Err(e) = self.extras.save(&self.path) {
            eprintln!("springboard: calendar: {e}");
        }
    }

    /// The calendars the user made, and the extras of the events (`extras.rs`). The app reads them for the alerts.
    pub fn extras(&self) -> &extras::Extras {
        &self.extras
    }

    fn export_calendar(&mut self) {
        self.notice = None;
        let text = self.export_ics();
        self.save_export("Calendar", &text);
    }

    /// Writes `text` as `<name>.ics` under ~/Documents/Calendar Export and says where it went.
    fn save_export(&mut self, name: &str, text: &str) {
        let dir = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Documents").join("Calendar Export");
        let path = dir.join(format!("{name}.ics"));
        let written = std::fs::create_dir_all(&dir).and_then(|_| springboard::shell::fsutil::write_atomic(&path, text));
        self.error = None;
        self.notice = Some(match written {
            Ok(()) => format!("Exported to Documents/Calendar Export/{name}.ics"),
            Err(e) => format!("Could not export: {e}"),
        });
    }

    fn commit(&mut self, ui: &Ui, delete: bool, t: f64) {
        if self.storage_blocked {
            self.error = Some("Saved events could not be read. Save is unavailable.".into());
            return;
        }
        let Some(draft) = &self.editor else {
            return;
        };
        let (original, existing) = (draft.event.clone(), draft.existing);
        let mut event = draft.event.clone();
        event.title = self.fields[0].text().trim().to_string();
        event.location = self.fields[1].text().trim().to_string();
        event.notes = self.fields[2].text().trim().to_string();
        // the pickers' working copies become the preset fields the store keeps, and the rest goes to the extras
        let (repeat, rrule) = pickers::repeat_fields(draft.rule.as_ref());
        let (alert, alarms) = pickers::alarm_fields(&draft.alarms);
        event.repeat = repeat;
        event.alert = alert;
        let mut extra = draft.extra.clone();
        extra.rrule = rrule;
        extra.alarms = alarms;
        if !delete && !event.valid() {
            self.error = Some("Enter a title for this event".into());
            return;
        }
        let mut events = self.events.clone();
        if !delete && !draft.existing && events.len() >= store::MAX_EVENTS {
            self.error = Some("Calendar is full".into());
            return;
        }
        let removed = if delete { self.events.iter().find(|e| e.id == event.id).cloned() } else { None };
        events.retain(|e| e.id != event.id);
        if !delete {
            events.push(event.clone());
        }
        store::sort(&mut events);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                if delete {
                    self.undo_extra = self.extras.events.remove(&event.id);
                } else {
                    self.extras.put(event.id, extra);
                }
                self.save_extras();
                self.selected = event.date;
                self.error = None;
                self.picked = (!delete).then_some(event.id);
                self.undo = removed.map(|e| (e, t + UNDO_MS));
                // an event the account organises, with invitees, goes to them again (`rsvp::invite_attendees`)
                if !delete && existing {
                    self.invite_attendees(&original, &event);
                }
                // the people invited from the editor get the event as an invitation (`inviting.rs`)
                if !delete {
                    self.invite_new_attendees(&event);
                }
                // the grid comes back where it was (`close`), not at midnight
                self.close();
                self.fit_scroll(ui);
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save. Your edit is still here.".into());
            }
        }
    }

    /// Desktop chords (Cmd or Ctrl; evdev keys): N new event, T today, 1 to 4 for Day, Week,
    /// Month and Year. Not while an event is open. False when unused here.
    pub fn chord(&mut self, ui: &Ui, key: u32, mods: u32, t: f64) -> bool {
        use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META};
        if mods & (CHORD_CTRL | CHORD_META) == 0 || mods & CHORD_ALT != 0 || self.editor.is_some() || self.sheet_open() || self.event_pop.is_some() {
            return false;
        }
        let target = match key {
            49 => {
                self.open(ui, None);
                self.dirty = true;
                return true;
            }
            20 => Target::Today,
            33 => Target::SearchField,
            2 => Target::View(View::Day),
            3 => Target::View(View::Week),
            4 => Target::View(View::Month),
            5 => Target::View(View::Year),
            _ => return false,
        };
        self.activate(ui, target, t);
        true
    }

    /// Escape closes the open event editor without saving.
    pub fn escape(&mut self, ui: &Ui, t: f64) -> bool {
        // Escape closes an open pop-up menu of calendars first, and changes nothing
        if self.cal_menu_cancel() {
            return true;
        }
        // Escape drops the new event of the quick-event popover
        if self.pop_cancel() {
            return true;
        }
        // Escape closes the invitee line before it closes the form
        if self.invitee_line_open() {
            self.invitee_key(Out::Hide);
            return true;
        }
        if self.editor.is_none() && !self.sheet_open() && (self.search_focused || !self.search_box.is_empty()) {
            self.end_search(ui);
            self.dirty = true;
            return true;
        }
        if self.editor.is_none() && !self.sheet_open() {
            return false;
        }
        self.activate(ui, Target::Cancel, t);
        true
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) {
        self.error = None;
        self.notice = None;
        self.dirty = true;
        // choosing another view or day, or a new event, ends a search
        if matches!(target, Target::Today | Target::Prev | Target::Next | Target::View(_) | Target::Add | Target::NewPopover | Target::MiniDay(_)) {
            self.end_search(ui);
        }
        // the mini month follows the selected day again once the main view moves
        if matches!(target, Target::Today | Target::Prev | Target::Next | Target::Pick(_) | Target::Day(_) | Target::MonthOf(_) | Target::GoConfirm) {
            self.mini_shown = None;
        }
        match target {
            Target::SearchField => self.focus_search(),
            Target::SearchClear => self.clear_search(ui),
            Target::SearchHit(i) => self.open_hit(ui, i),
            Target::Today => {
                self.selected = today(self.wall, t);
                self.scroll.jump_top();
            }
            Target::Prev | Target::Next => {
                self.step_view(if target == Target::Prev { -1 } else { 1 });
                self.scroll.jump_top();
            }
            Target::View(v) => {
                self.view = v;
                self.fit_scroll(ui);
                if self.time_view() {
                    self.scroll_to_morning(ui);
                } else {
                    self.scroll.jump_top();
                }
            }
            Target::Pick(d) => self.selected = d,
            Target::MonthOf(m) => {
                self.selected.month = m;
                self.selected.day = self.selected.day.min(self.selected.month_len());
                self.view = View::Month;
                self.scroll.jump_top();
            }
            Target::Day(day) => {
                self.selected.day = day;
                self.scroll.jump_top();
            }
            Target::Add => self.open(ui, None),
            Target::NewPopover => self.open_event_pop(self.selected, 9 * 60),
            Target::Pop(part) => self.pop_tap(ui, part, t),
            Target::PopDismiss => self.pop_dismiss(ui, t),
            Target::MiniPrev | Target::MiniNext | Target::MiniDay(_) => self.mini_tap(target),
            Target::Row(id) if overlay::is_overlay(id) => self.explain_overlay(id),
            Target::Row(id) => self.open(ui, Some(id)),
            Target::Layer(id) => self.toggle_layer(id),
            Target::Cancel => {
                self.event_pop = None;
                self.goto = None;
                self.goto_line.clear();
                self.colour_sheet = None;
                self.quick_line = None;
                self.line_sheet = None;
                self.close();
            }
            Target::LineDone => self.line_done(ui, t),
            Target::ViewZone => self.cycle_view_zone(ui),
            Target::GoField => {}
            Target::ColourPick(i) => self.colour_pick(i),
            Target::ColourDone => self.colour_done(),
            Target::AddInvitee | Target::InviteeSuggest(_) | Target::InviteeDrop(_) => self.invitee_tap(target),
            Target::QuickOpen => self.open_quick(),
            Target::QuickAdd => self.quick_add(ui, t),
            Target::GoOpen => self.open_goto(),
            Target::GoDay(_) | Target::GoMonth(_) | Target::GoYear(_) => self.goto_step(target),
            Target::GoToday => {
                self.goto = Some(today(self.wall, t));
                self.goto_line.clear();
            }
            Target::GoConfirm => self.goto_confirm(ui),
            Target::Save => self.commit(ui, false, t),
            Target::Delete => self.commit(ui, true, t),
            Target::Duplicate => self.duplicate(),
            Target::Export => self.export_event(),
            Target::ExportCalendar => self.export_calendar(),
            Target::WeekNumbers => {
                self.prefs.week_numbers = !self.prefs.week_numbers;
                self.save_prefs();
            }
            Target::UndoDelete => self.undo_delete(ui, t),
            Target::Title => self.focus(Some(0)),
            Target::Location => self.focus(Some(1)),
            Target::Notes => self.focus(Some(2)),
            Target::FillAddress => {
                if let Some(address) = self.notes_address() {
                    self.fields[1].set_text(ui, &address);
                }
            }
            Target::CopyPhone => self.copy_phone(),
            Target::ShowInMaps => self.map_link = places::search_link(self.fields[1].text()),
            Target::Invitees => {}
            Target::Rsvp(i) => self.respond_to_invitation(rsvp::Answer::ALL[i]),
            Target::ToggleReminders => self.toggle_layer(overlay::LAYER_REMINDERS),
            Target::ToggleDeclined => {
                self.prefs.show_declined = !self.prefs.show_declined;
                self.save_prefs();
            }
            Target::Repeat
            | Target::Alert
            | Target::RepeatFreq
            | Target::RepeatEvery(_)
            | Target::RepeatDays
            | Target::RepeatDay(_)
            | Target::RepeatEnds
            | Target::RepeatCount(_)
            | Target::RepeatUntil(_)
            | Target::AlarmPreset(_)
            | Target::AlarmStep(_)
            | Target::AlarmAdd
            | Target::AlarmDrop(_)
            | Target::Zone
            | Target::Travel => self.picker_tap(target),
            // a pop-up menu of the calendars with their colours, not a step to the next one
            Target::Calendar => self.open_form_cal_menu(ui),
            Target::DateStep(_)
            | Target::MonthStep(_)
            | Target::TimeStep(_)
            | Target::AllDay
            | Target::Toggle(_) => {
                self.focus(None);
                let prefs = self.prefs;
                if let Some(draft) = &mut self.editor {
                    let end = draft.open == Some(Slot::End);
                    let e = &mut draft.event;
                    match target {
                        Target::DateStep(n) if end => {
                            e.duration = (e.duration as i32 + n * 1440).clamp(15, 1440) as u16
                        }
                        Target::DateStep(n) => e.date = e.date.shift_days(n),
                        Target::MonthStep(n) => e.date = e.date.shift_months(n),
                        Target::TimeStep(n) if end => {
                            e.duration = (e.duration as i32 + n).clamp(15, 1440) as u16
                        }
                        Target::TimeStep(n) => {
                            e.minute = (e.minute as i32 + n).rem_euclid(1440) as u16
                        }
                        Target::AllDay => {
                            e.all_day = !e.all_day;
                            if e.all_day && end {
                                draft.open = None;
                            }
                            // a new event still on the default alert follows the kind of event it became
                            if !draft.existing {
                                let (from, to) = if e.all_day { (prefs.alert, prefs.alert_all_day) } else { (prefs.alert_all_day, prefs.alert) };
                                if e.alert == from {
                                    e.alert = to;
                                }
                                if draft.alarms == extras::preset_alarms(from) {
                                    draft.alarms = extras::preset_alarms(to);
                                }
                            }
                        }
                        Target::Toggle(slot) => {
                            draft.open = if draft.open == Some(slot) { None } else { Some(slot) }
                        }
                        _ => {}
                    }
                }
            }
            Target::Agenda => {}
            Target::Sidebar => {
                self.side_hidden = !self.side_hidden;
                self.layout_columns(ui);
            }
        }
        self.sync_overlay();
        self.fit_scroll(ui);
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.sync_overlay();
        if self.cal_menu.is_some() {
            self.cal_menu_key(out, t);
            return;
        }
        if self.event_pop.is_some() {
            self.pop_key(ui, out, t);
            return;
        }
        if self.line_sheet.is_some() {
            self.line_key(ui, out, t);
            return;
        }
        if self.quick_line.is_some() {
            self.quick_key(ui, out, t);
            return;
        }
        if self.colour_sheet.is_some() {
            // Return gives the calendar the picked colour; Up and Down move the pick
            match out {
                Out::Enter => self.activate(ui, Target::ColourDone, t),
                Out::Named("Up") => self.colour_step(-1),
                Out::Named("Down") => self.colour_step(1),
                _ => return,
            }
            self.dirty = true;
            return;
        }
        if self.goto.is_some() {
            // Typing fills the date row; Return accepts the date; the arrow keys step it by a day (Left, Right, while
            // the date row is empty) or a week (Up, Down)
            let typing = !self.goto_line.is_empty();
            let step = match out {
                Out::Enter => {
                    self.activate(ui, Target::GoConfirm, t);
                    return;
                }
                Out::Text(text) => {
                    self.goto_line.push_str(&text);
                    self.goto_read();
                    self.dirty = true;
                    return;
                }
                Out::Backspace => {
                    self.goto_line.pop();
                    self.goto_read();
                    self.dirty = true;
                    return;
                }
                Out::Named("Left") if typing => {
                    self.goto_line.move_caret(false);
                    self.dirty = true;
                    return;
                }
                Out::Named("Right") if typing => {
                    self.goto_line.move_caret(true);
                    self.dirty = true;
                    return;
                }
                Out::Named("Left") => -1,
                Out::Named("Right") => 1,
                Out::Named("Up") => -7,
                Out::Named("Down") => 7,
                _ => return,
            };
            self.goto_step(Target::GoDay(step));
            self.dirty = true;
            return;
        }
        if self.search_focused && self.editor.is_none() {
            self.search_key(ui, out);
            return;
        }
        if self.editor.is_none() {
            // Backspace and Delete delete the event selected with a click, with the Undo bar (E2E W2-16)
            if matches!(out, Out::Backspace | Out::Named("Delete")) {
                if self.delete_picked(t) {
                    self.fit_scroll(ui);
                }
                self.dirty = true;
                return;
            }
            if out == Out::Enter {
                self.open(ui, None);
                self.dirty = true;
                return;
            }
            // Arrow keys move the selected day: Left/Right one day, Up/Down one week.
            self.mini_shown = None;
            let step = match out {
                Out::Named("Left") => -1,
                Out::Named("Right") => 1,
                Out::Named("Up") => -7,
                Out::Named("Down") => 7,
                _ => return,
            };
            self.selected = self.selected.shift_days(step);
            self.dirty = true;
            return;
        }
        // the Add Invitees line, when it is open, takes the keys
        if self.invitee_key(out.clone()) {
            return;
        }
        // Return in the title or the location is Save, as in the stock app's event popover (E2E W2-18)
        if out == Out::Enter && matches!(self.focused(), Some(0 | 1)) {
            self.focus(None);
            self.activate(ui, Target::Save, t);
            return;
        }
        if matches!(out, Out::Hide | Out::Enter) {
            self.focus(None);
        } else if let Some(i) = self.focused() {
            self.fields[i].edit(ui, &out, t);
        }
        self.error = None;
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        // the content column's coordinates: the calendar list is left of it
        self.layout_columns(ui);
        let ev = match ev {
            Event::Down(x, y) => Event::Down(x - self.x0, y),
            Event::Move(x, y) => Event::Move(x - self.x0, y),
            Event::Up(x, y) => Event::Up(x - self.x0, y),
            other => other,
        };
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.dirty = true;
        // an open pop-up menu of calendars takes every press: a row picks, outside closes it (`calmenu.rs`)
        if self.cal_menu.is_some() {
            match ev {
                Event::Down(..) => self.cal_menu_press(ui, x + self.x0, y),
                Event::Move(..) => self.cal_menu_hover(ui, x + self.x0, y),
                _ => {}
            }
            self.pressed = None;
            self.start = None;
            self.scrolling = false;
            return;
        }
        // Menu bubbles and selection handles extend beyond the title's
        // rectangle. Let the field inspect the whole gesture before app
        // controls, retaining selection when a key is tapped below it.
        if self.editor.is_some() {
            for i in 0..3 {
                let frame = self.field_frame(ui, i);
                if !(self.fields[i].focused || within(x, y, frame)) {
                    continue;
                }
                if self.fields[i].touch(ui, ev, frame, self.w as f32, t) {
                    self.field_touch = !matches!(ev, Event::Up(..));
                    self.pressed = None;
                    self.start = None;
                    self.scrolling = false;
                    self.focus(Some(i));
                    return;
                }
            }
        }
        match ev {
            Event::Down(..) => {
                // the counter works in points, so its slop is the same on every screen
                let k = ui.pt(1.0).max(f32::EPSILON);
                self.click_count = self.clicks.press(t, x / k, y / k);
                self.pressed = self.hit(ui, x, y);
                self.start = Some((x, y));
                // the event form scrolls too: a press that moves on drags it, a press that stays is a tap
                self.scrolling = matches!(self.pressed, Some(Target::Row(_) | Target::Agenda | Target::SearchHit(_)))
                    || (self.editor.is_some() && self.pressed.is_some());
                if self.scrolling {
                    self.scroll.down(y, t);
                }
                // a press anywhere but the search field takes the cursor out of it
                if !matches!(self.pressed, Some(Target::SearchField | Target::SearchClear)) {
                    self.search_focused = false;
                }
                self.begin_press(ui, x, y, t);
            }
            Event::Move(..) => {
                if self.track_drag(ui, x, y, t) {
                    // a press held and dragged in the hour grid: the block follows the pointer
                } else if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                } else if self
                    .start
                    .is_some_and(|(sx, sy)| (x - sx).hypot(y - sy) > ui.pt(SPACE_M))
                {
                    self.pressed = None;
                }
            }
            Event::Up(..) => {
                if self.drag.is_some() {
                    self.finish_drag(ui);
                    self.pressed = None;
                    self.start = None;
                    self.scrolling = false;
                    return;
                }
                self.press = None;
                let pressed = self.pressed.take();
                self.start = None;
                let tap = if std::mem::take(&mut self.scrolling) {
                    self.scroll.up(y, t)
                } else {
                    true
                };
                if tap {
                    if let Some(target) = pressed.filter(|p| Some(*p) == self.hit(ui, x, y)) {
                        // an event is selected by a tap and opened in its own window by a double-click (`window.rs`)
                        if let Target::Row(id) = target {
                            if !overlay::is_overlay(id) && self.editor.is_none() {
                                self.tap_event(id, self.click_count);
                                return;
                            }
                        }
                        // a double-click on an empty slot of the grid makes an event there, with its popover
                        let slot = if self.event_pop.is_none() && self.editor.is_none() { self.slot_of(ui, target, x, y) } else { None };
                        self.activate(ui, target, t);
                        if let Some((date, minute)) = slot {
                            if self.slot_double(date, minute, self.click_count) {
                                self.open_event_pop(date, minute);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `scrollWheel:` at `(x, y)` in window pixels: the wheel moved the content `dy` pixels (positive toward the start).
    /// The hour grid, the search results and the event form scroll by the wheel's travel (`Scroller::wheel`); the
    /// calendar list and a view with nothing to scroll (the month grid) take none. Not taken while a press or drag is in
    /// progress. True when the content moved.
    pub fn wheel(&mut self, ui: &Ui, x: f32, _y: f32, dy: f32) -> bool {
        self.layout_columns(ui);
        if x < self.x0 || self.drag.is_some() || self.scrolling || self.field_touch || self.cal_menu.is_some() {
            return false;
        }
        if !self.scroll.wheel(dy) {
            return false;
        }
        self.dirty = true;
        true
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if self.undo.as_ref().is_some_and(|(_, until)| t >= *until) {
            self.undo = None;
            self.dirty = true;
        }
        // Include the final animation frame, after tick clears its motion.
        let was_animating = self.scroll.animating();
        let date = today(self.wall, t);
        if date != self.today {
            self.today = date;
            self.dirty = true;
        }
        self.dirty |= self.scroll.tick(t)
            || was_animating
            || (self.editor.is_some() && self.focused().is_some());
        if let Some(i) = self.focused() {
            self.dirty |= self.fields[i].tick(ui, t);
        }
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let local = self.wall.now(t) + self.wall.offset as i64;
        let midnight = self.wall.at
            + ((local.div_euclid(86400) + 1) * 86400 - self.wall.offset as i64 - self.wall.unix)
                as f64
                * 1000.0;
        let mut due = midnight;
        if let Some((_, until)) = &self.undo {
            due = due.min(*until);
        }
        if let Some(alert) = self.alert_wake() {
            due = due.min(alert);
        }
        if self.scroll.animating() {
            due = t;
        }
        if let (Some(i), true) = (self.focused(), self.editor.is_some()) {
            if let Some(blink) = self.fields[i].next_wake(t) {
                due = due.min(blink);
            }
        }
        Some(due)
    }

    fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, x: f32, cy: f32) {
        let img = self.labels.get(ui, text, pt, Weight::Regular, color);
        s.blit(img, x as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
    }

    fn centre(
        &mut self,
        s: &mut Surface,
        ui: &Ui,
        text: &str,
        pt: f32,
        weight: Weight,
        color: u32,
        cx: f32,
        cy: f32,
    ) {
        let img = self.labels.get(ui, text, pt, weight, color);
        centred(s, img, cx as i32, (cy - img.h as f32 / 2.0) as i32);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.text_style != (ui.text_scale, ui.bold) {
            self.labels.clear();
            for f in &mut self.fields {
                let value = f.text().to_string();
                f.set_text(ui, &value);
            }
            self.text_style = (ui.text_scale, ui.bold);
        }
        self.layout_columns(ui);
        self.sync_overlay();
        self.background(s);
        {
            let mut c = self.column(s);
            if self.editor.is_some() {
                self.draw_editor(&mut c, ui, t);
            } else if self.goto.is_some() {
                self.draw_goto(&mut c, ui);
            } else if self.colour_sheet.is_some() {
                self.draw_colour(&mut c, ui);
            } else if self.line_sheet.is_some() {
                self.draw_line(&mut c, ui);
            } else if self.quick_line.is_some() {
                self.draw_quick(&mut c, ui);
            } else if self.searching() {
                self.draw_results(&mut c, ui);
                self.draw_undo(&mut c, ui);
            } else {
                match self.view {
                    View::Month => self.draw_month(&mut c, ui),
                    View::Year => self.draw_year(&mut c, ui),
                    _ => self.draw_time(&mut c, ui, t),
                }
                self.draw_undo(&mut c, ui);
            }
        }
        self.draw_chrome(s, ui);
        self.draw_event_pop(s, ui);
        self.draw_cal_menu(s, ui);
        self.dirty = false;
    }

    /// The Mac month grid: weekday names, hairline cells edge to edge, the day number at each cell's top right
    /// (today in a red circle), the selected day shaded, and each day's events listed in its cell.
    fn draw_month(&mut self, s: &mut Surface, ui: &Ui) {
        self.draw_header(s, ui);
        let w = self.w as f32;
        let (left, cell, top, rh) = self.grid_geom(ui);
        for i in 0..7 {
            let name = WEEKDAYS[self.prefs.weekday_at(i)];
            self.right(s, ui, &name[..3], LABEL_2.now(), left + cell * (i as f32 + 1.0) - ui.pt(SPACE_S), top - ui.pt(SPACE_M));
        }
        let start = self.grid_start();
        if self.prefs.week_numbers {
            for row in 0..6 {
                // the row's Monday falls in the ISO week the row shows
                let monday = start.shift_days(7 * row + if self.prefs.monday_first { 0 } else { 1 });
                self.centre(s, ui, &iso_week(monday).to_string(), MAC_SUBHEAD_PT, Weight::Medium, LABEL_2.now(), left / 2.0, top + rh * row as f32 + ui.pt(SPACE_M));
            }
        }
        for row in 0..6 {
            for col in 0..7 {
                let date = start.shift_days(row * 7 + col);
                let (x, y) = (left + cell * col as f32, top + rh * row as f32);
                let in_month = date.month == self.selected.month && date.year == self.selected.year;
                if date == self.selected {
                    s.fill_rrect(x, y, cell, rh, 0.0, CARD_BG.now());
                }
                let number = date.day.to_string();
                let (nx, ny) = (x + cell - ui.pt(SPACE_M), y + ui.pt(SPACE_M + SPACE_XXS));
                if date == self.today {
                    let r = ui.pt(SPACE_M - 1.0);
                    let img = self.labels.get(ui, &number, MAC_BODY_PT, Weight::Semibold, WHITE).clone();
                    let cx = nx - img.w as f32 / 2.0;
                    s.fill_rrect(cx - r, ny - r, 2.0 * r, 2.0 * r, r, red());
                    centred(s, &img, cx as i32, (ny - img.h as f32 / 2.0) as i32);
                } else {
                    self.right(s, ui, &number, if in_month { LABEL.now() } else { LABEL_3.now() }, nx, ny);
                }
                let (events, more) = self.cell_events(ui, date);
                let lines: Vec<(String, u32)> = events.iter().map(|e| (e.title.clone(), self.event_colour(e.id))).collect();
                for (k, (title, colour)) in lines.iter().enumerate() {
                    let ly = y + ui.pt(CELL_HEAD + CELL_LINE * (k as f32 + 0.5));
                    let d = ui.pt(SPACE_S - 2.0);
                    s.fill_rrect(x + ui.pt(SPACE_S), ly - d / 2.0, d, d, d / 2.0, *colour);
                    let fit = ui.fit(title, MAC_SUBHEAD_PT, cell - ui.pt(SPACE_XL + SPACE_XS));
                    self.text(s, ui, &fit, MAC_SUBHEAD_PT, if in_month { LABEL.now() } else { LABEL_2.now() }, x + ui.pt(SPACE_L), ly);
                }
                if more > 0 {
                    let ly = y + ui.pt(CELL_HEAD + CELL_LINE * (lines.len() as f32 + 0.5));
                    self.text(s, ui, &format!("{more} more"), MAC_SUBHEAD_PT, LABEL_2.now(), x + ui.pt(SPACE_L), ly);
                }
            }
        }
        let hair = ui.pt(0.5).max(1.0);
        for row in 0..=6 {
            s.fill_rrect(0.0, top + rh * row as f32, w, hair, 0.0, SEPARATOR_C.now());
        }
        for col in 1..7 {
            s.fill_rrect(left + cell * col as f32, top, hair, rh * 6.0, 0.0, SEPARATOR_C.now());
        }
        if self.prefs.week_numbers {
            s.fill_rrect(left, top, hair, rh * 6.0, 0.0, SEPARATOR_C.now());
        }
    }

    fn draw_editor(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let Some(draft) = &self.editor else {
            return;
        };
        let draft = draft.clone();
        let event = draft.event.clone();
        let open = draft.open;
        let people = draft.invitees.clone();
        let w = self.w as f32;
        let rows = self.rows();
        // Group backgrounds: consecutive rows without a gap share one cell.
        let mut i = 0;
        while i < rows.len() {
            let mut j = i;
            while j + 1 < rows.len() && rows[j + 1].1 == rows[j].1 + rows[j].2 {
                j += 1;
            }
            let (top, bottom) = (rows[i].1, rows[j].1 + rows[j].2);
            s.fill_rrect(
                ui.pt(SPACE_XL),
                ui.pt(top),
                w - ui.pt(SPACE_HUGE),
                ui.pt(bottom - top),
                ui.pt(CARD_R),
                CARD_BG.now(),
            );
            for r in &rows[i + 1..=j] {
                s.fill_rrect(ui.pt(36.0), ui.pt(r.1), w - ui.pt(56.0), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
            }
            i = j + 1;
        }
        let (end_date, end_min) = Self::end_of(&event);
        let short = |d: Date| format!("{} {} {}", d.day, &MONTHS[(d.month - 1) as usize][..3], d.year);
        let when = |d: Date, m: u16| {
            if event.all_day {
                short(d)
            } else {
                format!("{}  {}", short(d), time_label(m))
            }
        };
        for (target, top, h) in rows {
            let mid = ui.pt(top + h / 2.0);
            match target {
                Target::Title => self.draw_field(s, ui, 0, "Title"),
                Target::Location => self.draw_field(s, ui, 1, "Location"),
                Target::Notes => self.draw_field(s, ui, 2, "Notes"),
                Target::AllDay => {
                    self.text(s, ui, "All-day", MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                    draw_switch(
                        s,
                        ui,
                        w - ui.pt(88.0),
                        ui.pt(top + (h - 31.0) / 2.0),
                        if event.all_day { 1.0 } else { 0.0 },
                    );
                }
                Target::Toggle(slot) => {
                    let (name, value) = if slot == Slot::Start {
                        ("Starts", when(event.date, event.minute))
                    } else {
                        ("Ends", when(end_date, end_min))
                    };
                    self.text(s, ui, name, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                    let tint = if open == Some(slot) { colors::LINK.now() } else { LABEL_2.now() };
                    self.right(s, ui, &value, tint, w - ui.pt(36.0), mid);
                }
                Target::Repeat
                | Target::Alert
                | Target::RepeatFreq
                | Target::RepeatEvery(_)
                | Target::RepeatDays
                | Target::RepeatEnds
                | Target::RepeatCount(_)
                | Target::RepeatUntil(_)
                | Target::AlarmPreset(_)
                | Target::AlarmStep(_)
                | Target::AlarmAdd
                | Target::AlarmDrop(_)
                | Target::Calendar
                | Target::Zone
                | Target::Travel => self.draw_picker(s, ui, target, &draft, top, h),
                Target::DateStep(_) => {
                    let end = open == Some(Slot::End);
                    let d = if end { end_date } else { event.date };
                    self.step_row(s, ui, &short(d), mid);
                }
                Target::MonthStep(_) => {
                    self.centre(s, ui, "‹ Previous month", MAC_SUBHEAD_PT, Weight::Regular, colors::LINK.now(), w * 0.25, mid);
                    self.centre(s, ui, "Next month ›", MAC_SUBHEAD_PT, Weight::Regular, colors::LINK.now(), w * 0.75, mid);
                }
                Target::TimeStep(_) => {
                    let m = if open == Some(Slot::End) { end_min } else { event.minute };
                    self.step_row(s, ui, &time_label(m), mid);
                }
                Target::FillAddress => {
                    let text = self.notes_address().map_or_else(String::new, |a| format!("Use {a} as Location"));
                    self.centre(s, ui, &text, MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid)
                }
                Target::ShowInMaps => self.centre(s, ui, "Show in Maps", MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid),
                Target::CopyPhone => {
                    let text = self.notes_phone().map_or_else(String::new, |p| format!("Copy {p}"));
                    self.centre(s, ui, &text, MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid)
                }
                Target::Invitees => {
                    let text = invitees::summary(&people);
                    self.centre(s, ui, &text, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), w / 2.0, mid)
                }
                Target::Rsvp(i) => self.centre(s, ui, rsvp::Answer::ALL[i].label(), MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid),
                Target::AddInvitee => match &draft.invitee_line {
                    Some(line) => {
                        let k = ui.pt(1.0);
                        let field = bifrost_app::controls::Rect { x: 36.0, y: top + SPACE_S, w: w / k - 72.0, h: h - 2.0 * SPACE_S };
                        bifrost_app::widgets::text_field(s, ui, line, field, "Name or email address", true);
                    }
                    None => self.text(s, ui, "Add Invitees...", MAC_BODY_PT, colors::LINK.now(), ui.pt(36.0), mid),
                },
                Target::InviteeSuggest(i) => {
                    let who = self.invitee_suggestions().get(i).map_or_else(String::new, |e| inviting::row_label(&extras::Invitee { address: e.email.clone(), name: e.name.clone() }));
                    let fit = ui.fit(&who, MAC_BODY_PT, w - ui.pt(130.0));
                    self.text(s, ui, &fit, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                    self.right(s, ui, "Invite", colors::LINK.now(), w - ui.pt(36.0), mid);
                }
                Target::InviteeDrop(i) => {
                    let who = draft.extra.invitees.get(i).map_or_else(String::new, inviting::row_label);
                    let fit = ui.fit(&who, MAC_BODY_PT, w - ui.pt(130.0));
                    self.text(s, ui, &fit, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                    self.right(s, ui, "Remove", colors::SYSTEM_RED.now(), w - ui.pt(36.0), mid);
                }
                Target::Duplicate => {
                    self.centre(s, ui, "Duplicate Event", MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid)
                }
                Target::Export => {
                    self.centre(s, ui, "Export Event", MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid)
                }
                Target::Delete => {
                    self.centre(s, ui, "Delete Event", MAC_BODY_PT, Weight::Regular, colors::SYSTEM_RED.now(), w / 2.0, mid)
                }
                _ => {}
            }
        }
        if let Some(error) = self.error.clone() {
            let y = self.rows().last().map_or(600.0, |r| r.1 + r.2) + 28.0;
            self.centre(s, ui, &error, MAC_BODY_PT, Weight::Semibold, LABEL.now(), w / 2.0, ui.pt(y));
        } else if let Some(notice) = self.notice.clone() {
            let y = self.rows().last().map_or(600.0, |r| r.1 + r.2) + 28.0;
            self.centre(s, ui, &notice, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), w / 2.0, ui.pt(y));
        }
        let _ = t;
    }

    /// An event form field, drawn with the kit's `NativeTextField` look so it follows Light and Dark; the shell field
    /// keeps the text, caret and editing gestures.
    fn draw_field(&mut self, s: &mut Surface, ui: &Ui, i: usize, placeholder: &str) {
        let (x, y, w, h) = self.field_frame(ui, i);
        let f = &self.fields[i];
        let mut field = bifrost_app::controls::TextField::new();
        field.push_str(f.text());
        field.home();
        for _ in 0..f.text().get(..f.caret()).map_or(f.caret(), |p| p.chars().count()) {
            field.move_caret(true);
        }
        let k = ui.pt(1.0);
        let r = bifrost_app::controls::Rect { x: x / k, y: y / k + SPACE_S, w: w / k, h: h / k - 2.0 * SPACE_S };
        bifrost_app::widgets::text_field(s, ui, &field, r, placeholder, f.focused);
    }

    fn right(&mut self, s: &mut Surface, ui: &Ui, text: &str, color: u32, rx: f32, cy: f32) {
        let img = self.labels.get(ui, text, MAC_BODY_PT, Weight::Regular, color);
        s.blit(img, rx as i32 - img.w as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
    }

    fn step_row(&mut self, s: &mut Surface, ui: &Ui, title: &str, mid: f32) {
        let w = self.w as f32;
        self.centre(s, ui, "−", MAC_TITLE2_PT, Weight::Regular, colors::LINK.now(), ui.pt(55.0), mid);
        self.centre(s, ui, title, MAC_BODY_PT, Weight::Regular, LABEL.now(), w / 2.0, mid);
        self.centre(s, ui, "+", MAC_TITLE2_PT, Weight::Regular, colors::LINK.now(), w - ui.pt(55.0), mid);
    }
}

/// The app's menu commands (`App::main_menu`). The id is what the shell sends back in `menu_action`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewEvent,
    Save,
    Duplicate,
    ExportEvent,
    Delete,
    ExportCalendar,
    Today,
    Previous,
    Next,
    ShowDay,
    ShowWeek,
    ShowMonth,
    ShowYear,
    WeekNumbers,
    CalendarList,
    /// File > New Event from Text (the app reads the pasteboard, see `Calendar::new_event_from_text`).
    NewEventFromText,
    /// View > Zoom In / Zoom Out: the hour height of the Day and Week grids.
    ZoomIn,
    ZoomOut,
    /// Go > Go to Date...
    GoToDate,
    /// File > Import... (an .ics or .vcs file, or an archive), File > Export Archive... and File > Import Archive...:
    /// the app runs the open and save panels (`main.rs`), the calendar reads and writes the text (`archive.rs`).
    Import,
    ExportArchive,
    ImportArchive,
    /// Event > Search Events (Cmd+F): the cursor goes to the search field.
    Search,
    /// File > Print...: the app sends `Calendar::print_job` to the printing service.
    Print,
    /// File > Subscribe to Calendar from Clipboard: the address on the pasteboard becomes a read-only calendar.
    Subscribe,
    /// Event > Quick Event...: a sheet to type a sentence that becomes an event.
    QuickEvent,
    /// File > New Calendar...: a line sheet names the calendar (the app shows nothing else; see `perform`).
    NewCalendar,
    /// View > Refresh Calendars: the CalDAV calendars are read now (the app asks `dav_sync` to start).
    RefreshCalendars,
    /// View > Show Reminders: the open dated reminders of the Reminders app are drawn as all-day items.
    ShowReminders,
    /// View > Show Declined Events: the events the account declined are drawn (hidden by default).
    ShowDeclined,
}

impl Command {
    pub const ALL: [Command; 30] = [
        Command::NewEvent,
        Command::Save,
        Command::Duplicate,
        Command::ExportEvent,
        Command::Delete,
        Command::ExportCalendar,
        Command::Today,
        Command::Previous,
        Command::Next,
        Command::ShowDay,
        Command::ShowWeek,
        Command::ShowMonth,
        Command::ShowYear,
        Command::WeekNumbers,
        Command::CalendarList,
        Command::NewEventFromText,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::GoToDate,
        Command::Import,
        Command::ExportArchive,
        Command::ImportArchive,
        Command::Search,
        Command::Print,
        Command::Subscribe,
        Command::QuickEvent,
        Command::NewCalendar,
        Command::RefreshCalendars,
        Command::ShowReminders,
        Command::ShowDeclined,
    ];

    pub fn id(self) -> u32 {
        self as u32 + 1
    }

    pub fn from_id(id: u32) -> Option<Command> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Command::NewEvent => "New Event",
            Command::Save => "Save Event",
            Command::Duplicate => "Duplicate Event",
            Command::ExportEvent => "Export Event",
            Command::Delete => "Delete Event",
            Command::ExportCalendar => "Export Calendar",
            Command::Today => "Today",
            Command::Previous => "Previous",
            Command::Next => "Next",
            Command::ShowDay => "Day",
            Command::ShowWeek => "Week",
            Command::ShowMonth => "Month",
            Command::ShowYear => "Year",
            Command::WeekNumbers => "Week Numbers",
            Command::CalendarList => "Calendar List",
            Command::NewEventFromText => "New Event from Text",
            Command::ZoomIn => "Zoom In",
            Command::ZoomOut => "Zoom Out",
            Command::GoToDate => "Go to Date...",
            Command::Import => "Import...",
            Command::ExportArchive => "Export Archive...",
            Command::ImportArchive => "Import Archive...",
            Command::Search => "Search Events",
            Command::Print => "Print...",
            Command::Subscribe => "New Calendar Subscription...",
            Command::QuickEvent => "Quick Event...",
            Command::NewCalendar => "New Calendar...",
            Command::RefreshCalendars => "Refresh Calendars",
            Command::ShowReminders => "Show Reminders",
            Command::ShowDeclined => "Show Declined Events",
        }
    }

    /// The key equivalent shown in the menu; empty when the command has none.
    pub fn key(self) -> &'static str {
        match self {
            Command::NewEvent => "cmd+n",
            Command::Save => "cmd+s",
            Command::Today => "cmd+t",
            Command::ShowDay => "cmd+1",
            Command::ShowWeek => "cmd+2",
            Command::ShowMonth => "cmd+3",
            Command::ShowYear => "cmd+4",
            Command::ZoomIn => "cmd+=",
            Command::ZoomOut => "cmd+-",
            Command::GoToDate => "cmd+shift+t",
            Command::Search => "cmd+f",
            Command::Print => "cmd+p",
            Command::QuickEvent => "cmd+shift+n",
            Command::RefreshCalendars => "cmd+r",
            _ => "",
        }
    }
}

impl Calendar {
    /// Whether menu command `cmd` is enabled now. While an event is open only the editor's commands are.
    pub fn enabled(&self, cmd: Command) -> bool {
        let editing = self.editor.is_some();
        // the Go to Date sheet is modal too
        let busy = editing || self.sheet_open() || self.event_pop.is_some();
        let saved = self.editor.as_ref().is_some_and(|d| d.existing);
        match cmd {
            Command::Save | Command::Duplicate => editing,
            Command::ExportEvent => editing && saved && self.file_export,
            // the open event, or with no form open the event selected with a click (E2E W2-16)
            Command::Delete => (editing && saved) || (!busy && self.picked_event().is_some()),
            Command::WeekNumbers | Command::ExportCalendar => !busy && self.file_export,
            Command::ZoomIn => !busy && self.time_view() && self.prefs.hour_h < prefs::HOUR_H_MAX,
            Command::ZoomOut => !busy && self.time_view() && self.prefs.hour_h > prefs::HOUR_H_MIN,
            _ => !busy,
        }
    }

    /// The check mark of a menu command that is a state (the view, Week Numbers, the calendar list).
    pub fn checked(&self, cmd: Command) -> bool {
        match cmd {
            Command::ShowDay => self.view == View::Day,
            Command::ShowWeek => self.view == View::Week,
            Command::ShowMonth => self.view == View::Month,
            Command::ShowYear => self.view == View::Year,
            Command::WeekNumbers => self.prefs.week_numbers,
            Command::ShowReminders => self.prefs.show_reminders,
            Command::ShowDeclined => self.prefs.show_declined,
            Command::CalendarList => !self.side_hidden,
            _ => false,
        }
    }

    /// The menu title of `cmd`; the calendar list's title says what the item does now.
    pub fn command_title(&self, cmd: Command) -> &'static str {
        match cmd {
            Command::CalendarList if self.side_hidden => "Show Calendar List",
            Command::CalendarList => "Hide Calendar List",
            other => other.title(),
        }
    }

    /// Runs menu command `cmd` through the same actions as the toolbar, keys and touches. False when it is disabled.
    pub fn perform(&mut self, ui: &Ui, cmd: Command, t: f64) -> bool {
        if !self.enabled(cmd) {
            return false;
        }
        if matches!(cmd, Command::ZoomIn | Command::ZoomOut) {
            self.zoom_hours(ui, if cmd == Command::ZoomIn { 1 } else { -1 });
            self.dirty = true;
            return true;
        }
        if cmd == Command::NewCalendar {
            self.open_line(linesheet::Purpose::NewCalendar, "");
            return true;
        }
        if cmd == Command::Delete && self.editor.is_none() {
            let deleted = self.delete_picked(t);
            self.fit_scroll(ui);
            self.dirty = true;
            return deleted;
        }
        let target = match cmd {
            Command::NewEvent => Target::Add,
            Command::Save => Target::Save,
            Command::Duplicate => Target::Duplicate,
            Command::ExportEvent => Target::Export,
            Command::Delete => Target::Delete,
            Command::ExportCalendar => Target::ExportCalendar,
            Command::Today => Target::Today,
            Command::Previous => Target::Prev,
            Command::Next => Target::Next,
            Command::ShowDay => Target::View(View::Day),
            Command::ShowWeek => Target::View(View::Week),
            Command::ShowMonth => Target::View(View::Month),
            Command::ShowYear => Target::View(View::Year),
            Command::WeekNumbers => Target::WeekNumbers,
            Command::CalendarList => Target::Sidebar,
            Command::ShowReminders => Target::ToggleReminders,
            Command::ShowDeclined => Target::ToggleDeclined,
            // the app reads the pasteboard and calls `new_event_from_text`; this path is only a plain New Event
            Command::NewEventFromText => Target::Add,
            Command::GoToDate => Target::GoOpen,
            Command::Search => Target::SearchField,
            Command::QuickEvent => Target::QuickOpen,
            // the app shows the panels and passes the file's text to `import_file` / reads `archive_text`
            Command::ZoomIn | Command::ZoomOut | Command::Import | Command::ExportArchive | Command::ImportArchive | Command::Print | Command::Subscribe | Command::NewCalendar | Command::RefreshCalendars => return false,
        };
        self.activate(ui, target, t);
        true
    }

    /// Whether the Settings menu can be used now (not while an event is open).
    pub fn settings_enabled(&self) -> bool {
        self.editor.is_none() && !self.sheet_open() && self.event_pop.is_none()
    }

    /// Whether Settings menu choice `setting` is the current one (its check mark).
    pub fn setting_checked(&self, setting: prefs::Setting) -> bool {
        match setting {
            prefs::Setting::DefaultCalendar(id) => self.extras.default_id() == id,
            other => self.prefs.is(other),
        }
    }

    /// Settings menu: picks `setting`, saves the settings beside the events and redraws. The settings change the
    /// grids and the defaults of new events only; saved events are never touched. False while an event is open.
    pub fn change_setting(&mut self, ui: &Ui, setting: prefs::Setting) -> bool {
        if !self.settings_enabled() {
            return false;
        }
        // the default calendar is kept with the calendars, not with the other settings
        if let prefs::Setting::DefaultCalendar(id) = setting {
            if !self.extras.exists(id) {
                return false;
            }
            self.extras.default_calendar = id;
            self.save_extras();
            self.dirty = true;
            return true;
        }
        self.prefs.apply(setting);
        self.save_prefs();
        if self.time_view() && matches!(setting, prefs::Setting::DayStart(_)) {
            self.scroll_to_morning(ui);
        }
        // the birthday alerts are planned with the events: a new choice plans them again
        if matches!(setting, prefs::Setting::BirthdayAlert(_)) {
            self.changes.post();
        }
        self.fit_scroll(ui);
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests;
