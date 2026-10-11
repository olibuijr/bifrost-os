//! The Clock app: World Clock cards with analog faces, Alarm cards with an
//! add/edit popover (repeat days, sound, snooze), a Stopwatch with persisted
//! laps and a Timer tab with several concurrent timers. The four tabs are the
//! unified toolbar's segmented control. The app edits the alarms and timers
//! on disk; the shell's headless Clock service fires them (alert, sound,
//! RTC wake) whether or not this app is running, and the app reloads what
//! the service changed (a one-shot alarm switched off, a finished timer).

pub mod stopwatch;
pub mod world;

pub use springboard::shell::clock::{alarm, offset_fn, picker, store, timer, tz, Wall};

use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::widgets;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{DISPLAY_LARGE_PT, DISPLAY_MEDIUM_PT};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE2_PT, MAC_TITLE3_PT};
use bifrost_app::kit::ui::{DESTRUCTIVE, LINK};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::kit::ui::{home_zone, Ui, Weight, BLACK, CARD_BG, CONTROL_PRESSED, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Event, Surface, TextCache};
use springboard::shell::controls::draw_switch;
use springboard::shell::glyphs;
use springboard::shell::keyboard::Out;
use springboard::shell::textfield::TextField;
use springboard::shell::Effect;
use alarm::{Alarm, Alarms};
use picker::Picker;
use stopwatch::Stopwatch;
use timer::Timers;
use tz::{civil, City, CITIES};

pub const ORANGE: u32 = springboard::shell::ui::SYSTEM_ORANGE;

const BUTTON_R_PT: f32 = 48.0;
const BUTTON_X_PT: f32 = 92.0;
const BUTTON_Y_PT: f32 = 625.0;
const DIGITS_Y_PT: f32 = 295.0;
const TABLE_X_PT: f32 = 184.0;
const TABLE_Y_PT: f32 = 588.0;
const LAP_ROW_PT: f32 = 48.0;
const CARD_MARGIN_PT: f32 = 16.0;
const WORLD_CARD_H_PT: f32 = 300.0;
const ALARM_CARD_PT: f32 = 250.0;
const ALARM_CARD_H_PT: f32 = 270.0;
const ALARM_GAP_PT: f32 = 6.0;
const PICKER_Y_PT: f32 = 308.0;
const PICKER_ROW_PT: f32 = 34.0;
const PICKER_COL_PT: f32 = 100.0;
const VISIBLE_ROWS: f32 = 7.0;
const POP_W_PT: f32 = 318.0;
const POP_H_PT: f32 = 540.0;
const POP_Y_PT: f32 = 69.0;
const POP_ROW_PT: f32 = 32.0;
const POP_PICKER_Y_PT: f32 = 156.0;
const ROW_PT: f32 = 44.0;
const LABEL_MAX: usize = 24;
/// Top of the editor's grouped table under the time wheel, as on bifrost:
/// Repeat, Label, Sound and Snooze in one inset group of 44 pt rows.
const POP_TABLE_Y_PT: f32 = 284.0;
/// Where the list on a sub-page (Repeat, Sound) starts below the bar.
const POP_LIST_Y_PT: f32 = 68.0;
/// Gap between the tone group and the switches on the Sound page.
const POP_GROUP_GAP_PT: f32 = 35.0;
const PRESET_Y_PT: f32 = 440.0;
const PRESET_H_PT: f32 = 30.0;
const TIMER_ROW_Y_PT: f32 = 478.0;
const TIMER_ROW_PT: f32 = 34.0;
const TIMER_ROWS: usize = 2;
const SWIPE_PT: f32 = 10.0;
const SWIPE_DELETE: f32 = 0.35;
/// Toolbar item ids: the four tab segments are 1..=4, then Edit and Add.
const BAR_TAB: u32 = 1;
const BAR_EDIT: u32 = 10;
const BAR_ADD: u32 = 11;

/// Popover bar buttons (Cancel, Save, Back) are this wide.
const POP_BAR_BTN_PT: f32 = 100.0;
/// Alarm card: the switch and Skip hit boxes sit on this row, measured from the card's top.
const ALARM_ACTION_Y_PT: f32 = 180.0;
/// Alarm card: the Skip button's left edge and width, measured from the card's left.
const ALARM_SKIP_X_PT: f32 = 130.0;
const ALARM_SKIP_W_PT: f32 = 100.0;
/// Alarm card text rows and the switch row, measured from the card's top.
const ALARM_LABEL_Y_PT: f32 = 123.0;
const ALARM_REPEAT_Y_PT: f32 = 150.0;
const ALARM_SWITCH_Y_PT: f32 = 195.0;
const ALARM_SKIP_Y_PT: f32 = 210.0;
/// World Clock card: the digital time and the relative-day line, measured from the card's top.
const WORLD_TIME_Y_PT: f32 = 205.0;
const WORLD_REL_Y_PT: f32 = 238.0;
/// Milliseconds in a second, as an integer and as the f64 the clocks use.
const MS_PER_SEC: i64 = 1000;
const MS_PER_SEC_F: f64 = 1000.0;
/// The dimming scrim behind sheets: black at an alpha, the same in Light and Dark.
pub const SCRIM: u32 = BLACK;

/// The Repeat page's rows, Monday first, each with its bit in `Alarm::repeat` (bit 0 is Sunday).
const DAY_ROWS: [(&str, u32); 7] = [("Monday", 1), ("Tuesday", 2), ("Wednesday", 3), ("Thursday", 4), ("Friday", 5), ("Saturday", 6), ("Sunday", 0)];

fn on_off(on: bool) -> String {
    String::from(if on { "on" } else { "off" })
}

/// A premultiplied colour at `alpha` of its opacity.
fn fade(c: u32, alpha: u8) -> u32 {
    let m = |sh: u32| (((c >> sh) & 0xff) * alpha as u32 / 255) << sh;
    m(0) | m(8) | m(16) | m(24)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tab {
    World,
    Alarm,
    Stopwatch,
    Timer,
}

const TABS: [(Tab, &str); 4] = [(Tab::World, "World Clock"), (Tab::Alarm, "Alarms"), (Tab::Stopwatch, "Stopwatch"), (Tab::Timer, "Timers")];

/// Something a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Tab(usize),
    /// the left round button (Lap/Reset, Cancel)
    Left,
    /// the right round button (Start/Stop, Pause/Resume)
    Right,
    Add,
    Edit,
    Switch(usize),
    Delete(usize),
    /// a picker column: timer 0..3, add-alarm 0..2
    Picker(usize),
    Cancel,
    Save,
    Field,
    /// outside the popover
    Outside,
    /// an alarm card: opens the edit popover
    EditAlarm(usize),
    /// skip (or restore) the next occurrence of a repeating alarm
    Skip(usize),
    /// a weekday row of the Repeat page, Monday first
    Day(usize),
    /// a tone row of the Sound page
    Tone(usize),
    /// push the editor's Repeat or Sound page
    Page(SheetPage),
    /// pop back to the editor's main page
    Back,
    /// popover switches: 0 vibrate, 1 fade in, 2 snooze
    Toggle(usize),
    /// a timer preset chip
    Preset(usize),
    /// pause or resume timer row `i`
    TimerRow(usize),
    TimerDelete(usize),
}

/// Distance field of `count` evenly spaced radial tick marks between radii
/// `r_in` and `r_out` around `c`, `width` thick, the first at 3 o'clock
/// (so 12 and 60 ticks include 12 o'clock).
fn ticks(c: (f32, f32), r_in: f32, r_out: f32, count: u32, width: f32) -> impl Fn(f32, f32) -> f32 {
    let step = std::f32::consts::TAU / count as f32;
    let (mid, half) = ((r_in + r_out) / 2.0, (r_out - r_in) / 2.0);
    move |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let rad = (dx * dx + dy * dy).sqrt();
        let angle = dy.atan2(dx).rem_euclid(std::f32::consts::TAU);
        let off = (angle - (angle / step).round() * step) * rad;
        (off.abs() - width / 2.0).max((rad - mid).abs() - half)
    }
}

/// The pages of the alarm editor sheet: the table, and the two lists it pushes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SheetPage {
    Main,
    Repeat,
    Sound,
}

struct AddSheet {
    page: SheetPage,
    hour: Picker,
    min: Picker,
    label: TextField,
    /// the alarm being edited
    edit: Option<u32>,
    repeat: u8,
    sound: usize,
    vibrate: bool,
    ramp: bool,
    snooze: bool,
}

/// A finger dragging an alarm card sideways.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swipe {
    idx: usize,
    x0: f32,
    dx: f32,
    active: bool,
}

pub struct Clock {
    w: i32,
    h: i32,
    tab: Tab,
    wall: Wall,
    stopwatch: Stopwatch,
    timers: Timers,
    timer_pickers: [Picker; 3],
    alarms: Alarms,
    world: world::World,
    path: Option<String>,
    /// the home zone's daylight-saving rule, when known
    zone: Option<&'static City>,
    /// delete badges shown on the alarm cards
    editing: bool,
    add: Option<AddSheet>,
    /// the files' modification times as last read or written
    seen: std::cell::Cell<store::Stamp>,
    next_poll: f64,
    swipe: Option<Swipe>,
    swipe_dirty: bool,
    pressed: Option<Target>,
    /// the wall second last drawn, to repaint faces and countdowns once a second
    drawn_sec: i64,
    cache: TextCache,
}

impl Clock {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        let (th, tm, ts) = timer::split(15.0 * 60_000.0);
        let row = ui.pt(PICKER_ROW_PT);
        Clock {
            w,
            h,
            tab: Tab::World,
            wall: Wall::default(),
            stopwatch: Stopwatch::default(),
            timers: Timers::default(),
            timer_pickers: [Picker::new(24, row, th as usize), Picker::new(60, row, tm as usize), Picker::new(60, row, ts as usize)],
            alarms: Alarms::default(),
            world: world::World::default(),
            path: None,
            zone: None,
            editing: false,
            add: None,
            seen: Default::default(),
            next_poll: 0.0,
            swipe: None,
            swipe_dirty: false,
            pressed: None,
            drawn_sec: -1,
            cache: TextCache::default(),
        }
    }

    /// Loads alarms from `path` and selected cities from sibling `world-cities.ini`,
    /// preserving the legacy city defaults when no selection file exists. Saves there
    /// from now on. Returns the RTC wake effect for the next alarm.
    pub fn load(&mut self, path: &str, t: f64) -> Vec<Effect> {
        self.path = Some(path.to_string());
        self.reload(path, t);
        let now_ms = self.wall.now_ms(t);
        let read = |name: &str| std::fs::read_to_string(store::sibling(path, name)).unwrap_or_default();
        self.stopwatch = Stopwatch::from_text(&read("clock-stopwatch.ini"), t, now_ms);
        self.world.load(path);
        vec![self.wake_effect(t)]
    }

    /// Re-reads the alarms and timers, which the shell's service also writes.
    fn reload(&mut self, path: &str, t: f64) {
        self.alarms = Alarms::parse(&std::fs::read_to_string(path).unwrap_or_default());
        self.alarms.start_at(self.wall.now(t));
        let timers = std::fs::read_to_string(store::sibling(path, "clock-timers.ini")).unwrap_or_default();
        self.timers = Timers::parse(&timers, t, self.wall.now_ms(t));
        self.seen.set(store::stamp(path));
    }

    /// Tells the Clock which zone's daylight-saving rule the device follows,
    /// so alarm times hold across a change of offset. Without it the
    /// device's current offset is assumed to stay.
    pub fn set_zone(&mut self, zone: Option<&'static City>) {
        self.zone = zone;
    }

    /// Anchors the wall clock: `unix` seconds and the local `offset` as of
    /// monotonic `t`.
    pub fn set_wall(&mut self, unix: i64, offset: i32, t: f64) {
        self.wall = Wall { unix, offset, at: t };
    }

    /// The current wall-clock anchor, shared with apps that show dates.
    pub fn wall(&self) -> Wall {
        self.wall
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Shows `tab` next time the app is on screen.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    pub fn stopwatch(&self) -> &Stopwatch {
        &self.stopwatch
    }

    pub fn timers(&self) -> &Timers {
        &self.timers
    }

    pub fn timers_mut(&mut self) -> &mut Timers {
        &mut self.timers
    }

    /// "Alarm in 7 hr 5 min" for the lock screen and Control Center, when an
    /// alarm is armed.
    pub fn next_alarm_text(&self, t: f64) -> Option<String> {
        self.alarms.summary(self.wall.now(t), &offset_fn(self.zone, self.wall.offset))
    }

    pub fn alarms(&self) -> &Alarms {
        &self.alarms
    }

    /// The add-alarm popover's label field, while it is open.
    pub fn label_field(&self) -> Option<&TextField> {
        self.add.as_ref().map(|a| &a.label)
    }

    /// Arms the RTC for the earlier of the next alarm and the next timer.
    fn wake_effect(&self, t: f64) -> Effect {
        let alarm = self.alarms.next_due(self.wall.now(t), &offset_fn(self.zone, self.wall.offset));
        let at = match (alarm, self.timers.next_due_unix()) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b).unwrap_or(0),
        };
        Effect::WakeAt(at)
    }

    fn write(&self, name: Option<&str>, text: String) {
        let Some(p) = &self.path else { return };
        let path = name.map_or_else(|| p.clone(), |n| store::sibling(p, n));
        if let Err(e) = store::write_atomic(&path, &text) {
            eprintln!("springboard: clock: {e}");
        }
        self.seen.set(store::stamp(p));
    }

    fn persist(&self, t: f64) -> Vec<Effect> {
        self.write(None, self.alarms.serialize());
        vec![self.wake_effect(t)]
    }

    fn persist_timers(&self, t: f64) -> Vec<Effect> {
        self.write(Some("clock-timers.ini"), self.timers.serialize());
        vec![self.wake_effect(t)]
    }

    fn persist_stopwatch(&self, t: f64) {
        self.write(Some("clock-stopwatch.ini"), self.stopwatch.to_text(t, self.wall.now_ms(t)));
    }

    // ----- geometry -----

    /// The bottom of the content: the window has no tab bar (the tabs are the toolbar's segmented control).
    fn tab_top(&self, _ui: &Ui) -> f32 {
        self.h as f32
    }

    fn content_top(ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT)
    }

    /// The unified toolbar (points): Edit, the World Clock / Alarms / Stopwatch / Timers segments, Add.
    fn bar(&self) -> Toolbar {
        let editable = matches!(self.tab, Tab::World | Tab::Alarm);
        let has_items = match self.tab { Tab::World => !self.world.selected.is_empty(), _ => !self.alarms.list.is_empty() };
        let mut items = vec![];
        if editable {
            items.push(ToolbarItem::button(BAR_EDIT, if self.editing { "Done" } else { "Edit" }).enabled(has_items));
        }
        items.push(ToolbarItem::flexible_space());
        for (i, (tab, name)) in TABS.iter().enumerate() {
            items.push(ToolbarItem::button(BAR_TAB + i as u32, name).in_group(1).selected(*tab == self.tab));
        }
        items.push(ToolbarItem::flexible_space());
        if editable {
            items.push(ToolbarItem::icon(BAR_ADD, if self.tab == Tab::World { "Add City" } else { "Add Alarm" }, "plus"));
        }
        Toolbar::new(items)
    }

    /// The toolbar's rectangle in points, right of the traffic lights.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (self.w as f32 / ui.pt(1.0) - x - SPACE_M).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The toolbar laid out as `widgets::toolbar` draws it (titled buttons measured at body size).
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        let h = widgets::CAPSULE_PT;
        let b = self.bar_bounds(ui);
        self.bar().layout_with(Rect { y: b.y + (b.h - h) / 2.0, h, ..b }, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Every visible control as a semantic node, in points: the toolbar buttons (so the shell does not drag the
    /// window over them), then the body controls of the current tab or open editor sheet.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar();
        let mut nodes: Vec<Node> = self.bar_layout(ui).visible.iter().filter_map(|(id, r)| {
            let item = bar.items.iter().find(|i| i.id == *id)?;
            let role = if (BAR_TAB..BAR_TAB + 4).contains(id) { Role::SegmentedControl } else { Role::Button };
            Some(Node { id: *id as u64, role, label: item.label.clone(), value: if item.selected { "selected".into() } else { String::new() }, bounds: *r, enabled: item.enabled })
        }).collect();
        nodes.extend(self.body_controls(ui));
        nodes
    }

    /// The body controls as nodes (points). Ids: 1..=11 are the toolbar; the rest are grouped by 100s.
    fn body_controls(&self, ui: &Ui) -> Vec<Node> {
        let p = ui.pt(1.0);
        let mut out: Vec<Node> = Vec::new();
        // `r` is (x, y, w, h) in px, as the drawing and hit code use it
        let mut push = |id: u64, role: Role, label: String, value: String, r: (f32, f32, f32, f32), enabled: bool| {
            out.push(Node { id, role, label, value, bounds: Rect { x: r.0 / p, y: r.1 / p, w: r.2 / p, h: r.3 / p }, enabled });
        };
        let circle = |c: (f32, f32), r: f32| (c.0 - r, c.1 - r, 2.0 * r, 2.0 * r);
        if let Some(a) = &self.add {
            let (px, py, pw, _) = self.popover(ui);
            let bar_h = ui.pt(ROW_PT);
            if a.page == SheetPage::Main {
                push(120, Role::Button, "Cancel".into(), String::new(), (px, py, ui.pt(POP_BAR_BTN_PT), bar_h), true);
                push(121, Role::Button, "Save".into(), String::new(), (px + pw - ui.pt(POP_BAR_BTN_PT), py, ui.pt(POP_BAR_BTN_PT), bar_h), true);
            } else {
                push(122, Role::Button, "Back".into(), String::new(), (px, py, ui.pt(POP_BAR_BTN_PT), bar_h), true);
            }
            match a.page {
                SheetPage::Main => {
                    push(130, Role::Button, "Repeat".into(), alarm::repeat_text(a.repeat), self.pop_row(ui, 0), true);
                    push(131, Role::TextField, "Label".into(), a.label.text().to_string(), self.pop_field(ui), true);
                    push(132, Role::Button, "Sound".into(), alarm::SOUNDS[a.sound].to_string(), self.pop_row(ui, 2), true);
                    push(133, Role::Toggle, "Snooze".into(), on_off(a.snooze), self.pop_row(ui, 3), true);
                    let half = ui.pt(POP_ROW_PT) * VISIBLE_ROWS / 2.0;
                    for (i, (name, picker)) in [("Hours", &a.hour), ("Minutes", &a.min)].into_iter().enumerate() {
                        let (cx, cw, cy) = self.pop_column(ui, i);
                        push(150 + i as u64, Role::Slider, name.into(), format!("{}", picker.position().round() as usize), (cx, cy - half, cw, 2.0 * half), true);
                    }
                }
                SheetPage::Repeat => {
                    for (i, (name, bit)) in DAY_ROWS.iter().enumerate() {
                        let on = (a.repeat & (1u8 << *bit)) != 0;
                        push(140 + i as u64, Role::Toggle, name.to_string(), on_off(on), self.pop_list_row(ui, i), true);
                    }
                }
                SheetPage::Sound => {
                    let n = alarm::SOUNDS.len();
                    for (i, tone) in alarm::SOUNDS.iter().enumerate() {
                        let value = if a.sound == i { "selected".to_string() } else { String::new() };
                        push(170 + i as u64, Role::Button, tone.to_string(), value, self.pop_list_row(ui, i), true);
                    }
                    push(180, Role::Toggle, "Vibrate".into(), on_off(a.vibrate), self.pop_list_row(ui, n), true);
                    push(181, Role::Toggle, "Fade In".into(), on_off(a.ramp), self.pop_list_row(ui, n + 1), true);
                }
            }
            return out;
        }
        match self.tab {
            Tab::World => {
                for (i, &index) in self.world.selected.iter().enumerate() {
                    let r = self.world_card(ui, i);
                    if r.1 + r.3 <= Self::content_top(ui) || r.1 >= self.h as f32 { continue }
                    push(100 + i as u64, Role::Row, CITIES[index].name.to_string(), String::new(), r, true);
                }
            }
            Tab::Alarm => {
                for (i, a) in self.alarms.list.iter().enumerate() {
                    let card = Self::alarm_card(ui, i);
                    if card.1 + card.3 <= Self::content_top(ui) || card.1 >= self.h as f32 { continue }
                    let time = a.time_text();
                    push(200 + i as u64, Role::Row, format!("Alarm {time}"), a.label.clone(), card, true);
                    push(300 + i as u64, Role::Toggle, format!("{time} alarm"), on_off(a.on), (card.0 + ui.pt(30.0), card.1 + ui.pt(ALARM_ACTION_Y_PT), ui.pt(81.0), ui.pt(61.0)), true);
                    if self.editing {
                        push(400 + i as u64, Role::Button, format!("Delete alarm {time}"), String::new(), circle((card.0 + ui.pt(SPACE_XXL), card.1 + ui.pt(75.0)), ui.pt(SPACE_XXL)), true);
                    }
                    if a.on && a.repeat != 0 {
                        let label = if a.skip_day.is_some() { "Restore next alarm" } else { "Skip next alarm" };
                        push(500 + i as u64, Role::Button, label.into(), String::new(), (card.0 + ui.pt(ALARM_SKIP_X_PT), card.1 + ui.pt(ALARM_ACTION_Y_PT), ui.pt(ALARM_SKIP_W_PT), ui.pt(61.0)), true);
                    }
                }
            }
            Tab::Stopwatch => {
                let running = self.stopwatch.running();
                let left = self.button_centre(ui, false);
                let right = self.button_centre(ui, true);
                let r = ui.pt(BUTTON_R_PT + 8.0);
                push(600, Role::Button, (if running { "Lap" } else { "Reset" }).into(), String::new(), circle(left, r), running || self.stopwatch.started(0.0));
                push(601, Role::Button, (if running { "Stop" } else { "Start" }).into(), String::new(), circle(right, r), true);
            }
            Tab::Timer => {
                let left = self.button_centre(ui, false);
                let right = self.button_centre(ui, true);
                let r = ui.pt(BUTTON_R_PT + 8.0);
                push(610, Role::Button, "Cancel".into(), String::new(), circle(left, r), true);
                push(611, Role::Button, "Start".into(), String::new(), circle(right, r), self.picked_ms() > 0);
                for (i, (name, _)) in timer::PRESETS.iter().enumerate() {
                    push(620 + i as u64, Role::Button, format!("{name} timer"), String::new(), self.preset_rect(ui, i), true);
                }
                for (i, item) in self.timers.list.iter().take(TIMER_ROWS).enumerate() {
                    let row = self.timer_row(ui, i);
                    let state = if item.running() { "running" } else { "paused" };
                    push(630 + i as u64, Role::Row, format!("Timer {}", item.label), state.into(), row, true);
                    push(640 + i as u64, Role::Button, format!("Delete timer {}", item.label), String::new(), (row.0 + row.2 - ui.pt(56.0), row.1, ui.pt(56.0), row.3), true);
                }
                for (i, name) in ["Hours", "Minutes", "Seconds"].iter().enumerate() {
                    let (cx, cw, cy) = self.timer_column(ui, i);
                    let half = ui.pt(PICKER_ROW_PT) * VISIBLE_ROWS / 2.0;
                    push(650 + i as u64, Role::Slider, name.to_string(), format!("{}", self.timer_pickers[i].position().round() as usize), (cx, cy - half, cw, 2.0 * half), true);
                }
            }
        }
        out
    }

    fn button_centre(&self, ui: &Ui, right: bool) -> (f32, f32) {
        (if right { self.w as f32 - ui.pt(BUTTON_X_PT) } else { ui.pt(BUTTON_X_PT) }, ui.pt(BUTTON_Y_PT))
    }

    fn near((cx, cy): (f32, f32), r: f32, x: f32, y: f32) -> bool {
        (x - cx).powi(2) + (y - cy).powi(2) < r * r
    }

    /// World Clock card `i`: (x, y, w, h).
    fn world_card(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (m, g) = (ui.pt(CARD_MARGIN_PT), ui.pt(CARD_MARGIN_PT));
        let cw = (self.w as f32 - 2.0 * m - 2.0 * g) / 3.0;
        let ch = ui.pt(WORLD_CARD_H_PT);
        (m + (i % 3) as f32 * (cw + g), Self::content_top(ui) + ui.pt(SPACE_M) + (i / 3) as f32 * (ch + g) - self.world.scroll.offset(), cw, ch)
    }

    fn alarm_card(ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (cw, ch, g) = (ui.pt(ALARM_CARD_PT), ui.pt(ALARM_CARD_H_PT), ui.pt(ALARM_GAP_PT));
        ((i % 3) as f32 * (cw + g), Self::content_top(ui) + (i / 3) as f32 * (ch + g), cw, ch)
    }

    /// Timer picker column `i`: (x, w) and the shared centre y.
    fn timer_column(&self, ui: &Ui, i: usize) -> (f32, f32, f32) {
        let x0 = (self.w as f32 - 3.0 * ui.pt(PICKER_COL_PT)) / 2.0;
        (x0 + i as f32 * ui.pt(PICKER_COL_PT), ui.pt(PICKER_COL_PT), ui.pt(PICKER_Y_PT))
    }

    fn popover(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (self.w as f32 - ui.pt(20.0 + POP_W_PT), ui.pt(POP_Y_PT), ui.pt(POP_W_PT), ui.pt(POP_H_PT))
    }

    /// Add-alarm picker column `i` (0 hours, 1 minutes): (x, w, centre y).
    fn pop_column(&self, ui: &Ui, i: usize) -> (f32, f32, f32) {
        let (px, py, pw, _) = self.popover(ui);
        let col = ui.pt(74.0);
        (px + pw / 2.0 - col + i as f32 * col, col, py + ui.pt(POP_PICKER_Y_PT))
    }

    /// Main-page table row `k` (0 Repeat, 1 Label, 2 Sound, 3 Snooze).
    fn pop_row(&self, ui: &Ui, k: usize) -> (f32, f32, f32, f32) {
        let (px, py, pw, _) = self.popover(ui);
        (px + ui.pt(SPACE_L), py + ui.pt(POP_TABLE_Y_PT + 44.0 * k as f32), pw - ui.pt(SPACE_XXXL), ui.pt(44.0))
    }

    fn pop_field(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.pop_row(ui, 1)
    }

    /// Row `k` of a pushed page: Repeat has 7 day rows, Sound 6 tones then
    /// the Vibrate and Fade In switches in a second group.
    fn pop_list_row(&self, ui: &Ui, k: usize) -> (f32, f32, f32, f32) {
        let (px, py, pw, _) = self.popover(ui);
        let gap = if k >= alarm::SOUNDS.len() { ui.pt(POP_GROUP_GAP_PT) } else { 0.0 };
        (px + ui.pt(SPACE_L), py + ui.pt(POP_LIST_Y_PT + 44.0 * k as f32) + gap, pw - ui.pt(SPACE_XXXL), ui.pt(44.0))
    }

    /// Timer preset chip `i`: (x, y, w, h).
    fn preset_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let n = timer::PRESETS.len() as f32;
        let (gap, margin) = (ui.pt(SPACE_S), ui.pt(TABLE_X_PT) - ui.pt(SPACE_HUGE));
        let cw = (self.w as f32 - 2.0 * margin - gap * (n - 1.0)) / n;
        (margin + i as f32 * (cw + gap), ui.pt(PRESET_Y_PT), cw, ui.pt(PRESET_H_PT))
    }

    /// Row `i` of the running-timer list: (x, y, w, h); its right 56 pt delete it.
    fn timer_row(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let x = ui.pt(TABLE_X_PT) - ui.pt(SPACE_HUGE);
        (x, ui.pt(TIMER_ROW_Y_PT) + i as f32 * ui.pt(TIMER_ROW_PT), self.w as f32 - 2.0 * x, ui.pt(TIMER_ROW_PT))
    }

    /// The label field's text box, after the "Label" caption.
    fn pop_text_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (fx, fy, fw, fh) = self.pop_field(ui);
        let text_x = fx + ui.pt(90.0);
        (text_x, fy, fx + fw - ui.pt(SPACE_L) - text_x, fh)
    }

    fn within(x: f32, y: f32, (rx, ry, rw, rh): (f32, f32, f32, f32)) -> bool {
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.add.is_some() {
            let (px, py, pw, _) = self.popover(ui);
            if !Self::within(x, y, self.popover(ui)) {
                return Some(Target::Outside);
            }
            let page = self.add.as_ref().map_or(SheetPage::Main, |a| a.page);
            if y < py + ui.pt(ROW_PT) {
                let left = x < px + ui.pt(POP_BAR_BTN_PT);
                return Some(match (page, left) {
                    (SheetPage::Main, true) => Target::Cancel,
                    (SheetPage::Main, false) if x > px + pw - ui.pt(POP_BAR_BTN_PT) => Target::Save,
                    (_, true) => Target::Back,
                    _ => Target::Outside,
                });
            }
            match page {
                SheetPage::Repeat => return (0..7).find(|&i| Self::within(x, y, self.pop_list_row(ui, i))).map(Target::Day),
                SheetPage::Sound => {
                    let n = alarm::SOUNDS.len();
                    return (0..n).find(|&i| Self::within(x, y, self.pop_list_row(ui, i))).map(Target::Tone).or_else(|| (n..n + 2).find(|&k| Self::within(x, y, self.pop_list_row(ui, k))).map(|k| Target::Toggle(k - n)));
                }
                SheetPage::Main => {}
            }
            if Self::within(x, y, self.pop_row(ui, 0)) {
                return Some(Target::Page(SheetPage::Repeat));
            }
            if Self::within(x, y, self.pop_field(ui)) {
                return Some(Target::Field);
            }
            if Self::within(x, y, self.pop_row(ui, 2)) {
                return Some(Target::Page(SheetPage::Sound));
            }
            if Self::within(x, y, self.pop_row(ui, 3)) {
                return Some(Target::Toggle(2));
            }
            for i in 0..2 {
                let (cx, cw, cy) = self.pop_column(ui, i);
                let half = ui.pt(POP_ROW_PT) * VISIBLE_ROWS / 2.0;
                if x >= cx && x < cx + cw && (y - cy).abs() < half {
                    return Some(Target::Picker(i));
                }
            }
            return None;
        }
        if y < Self::content_top(ui) {
            let p = ui.pt(1.0);
            return match self.bar_layout(ui).hit(x / p, y / p)? {
                BAR_EDIT => Some(Target::Edit),
                BAR_ADD => Some(Target::Add),
                id if (BAR_TAB..BAR_TAB + 4).contains(&id) => Some(Target::Tab((id - BAR_TAB) as usize)),
                _ => None,
            };
        }
        match self.tab {
            Tab::World => None,
            Tab::Alarm => {
                for i in 0..self.alarms.list.len() {
                    let (cx, cy, _, _) = Self::alarm_card(ui, i);
                    if self.editing && Self::near((cx + ui.pt(SPACE_XXL), cy + ui.pt(75.0)), ui.pt(SPACE_XXL), x, y) {
                        return Some(Target::Delete(i));
                    }
                    if Self::within(x, y, (cx + ui.pt(30.0), cy + ui.pt(ALARM_ACTION_Y_PT), ui.pt(81.0), ui.pt(61.0))) {
                        return Some(Target::Switch(i));
                    }
                    let a = &self.alarms.list[i];
                    if a.on && a.repeat != 0 && Self::within(x, y, (cx + ui.pt(ALARM_SKIP_X_PT), cy + ui.pt(ALARM_ACTION_Y_PT), ui.pt(ALARM_SKIP_W_PT), ui.pt(61.0))) {
                        return Some(Target::Skip(i));
                    }
                    if Self::within(x, y, Self::alarm_card(ui, i)) {
                        return Some(Target::EditAlarm(i));
                    }
                }
                None
            }
            Tab::Stopwatch | Tab::Timer => {
                let r = ui.pt(BUTTON_R_PT + 8.0);
                if Self::near(self.button_centre(ui, false), r, x, y) {
                    return Some(Target::Left);
                }
                if Self::near(self.button_centre(ui, true), r, x, y) {
                    return Some(Target::Right);
                }
                if self.tab == Tab::Timer {
                    for i in 0..timer::PRESETS.len() {
                        if Self::within(x, y, self.preset_rect(ui, i)) {
                            return Some(Target::Preset(i));
                        }
                    }
                    for i in 0..self.timers.list.len().min(TIMER_ROWS) {
                        let (rx, ry, rw, rh) = self.timer_row(ui, i);
                        if Self::within(x, y, (rx, ry, rw, rh)) {
                            return Some(if x > rx + rw - ui.pt(56.0) { Target::TimerDelete(i) } else { Target::TimerRow(i) });
                        }
                    }
                    for i in 0..3 {
                        let (cx, cw, cy) = self.timer_column(ui, i);
                        let half = ui.pt(PICKER_ROW_PT) * VISIBLE_ROWS / 2.0;
                        if x >= cx && x < cx + cw && (y - cy).abs() < half {
                            return Some(Target::Picker(i));
                        }
                    }
                }
                None
            }
        }
    }

    fn picker_mut(&mut self, i: usize) -> Option<&mut Picker> {
        match &mut self.add {
            Some(a) => [&mut a.hour, &mut a.min].into_iter().nth(i),
            None => self.timer_pickers.get_mut(i),
        }
    }

    fn picker_centre(&self, ui: &Ui, i: usize) -> f32 {
        if self.add.is_some() {
            self.pop_column(ui, i).2
        } else {
            self.timer_column(ui, i).2
        }
    }

    // ----- input -----

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Vec<Effect> {
        if let Some(effects) = self.world_event(ui, ev, t) { return effects }
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return vec![],
        };
        // the label field takes presses inside it, on its handles or its menu
        let frame = self.pop_text_frame(ui);
        if let Some(a) = &mut self.add {
            if a.label.touch(ui, ev, frame, self.w as f32, t) {
                return vec![];
            }
        }
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                self.swipe = match self.pressed {
                    Some(Target::EditAlarm(idx)) => Some(Swipe { idx, x0: x, dx: 0.0, active: false }),
                    _ => None,
                };
                match self.pressed {
                    Some(Target::Picker(i)) => {
                        let cy = self.picker_centre(ui, i);
                        if let Some(p) = self.picker_mut(i) {
                            p.down(y - cy, t);
                        }
                    }
                    _ => {}
                }
                vec![]
            }
            Event::Move(..) => {
                if let Some(sw) = &mut self.swipe {
                    sw.dx = (x - sw.x0).min(0.0);
                    sw.active |= -sw.dx > ui.pt(SWIPE_PT);
                    self.swipe_dirty = true;
                }
                match self.pressed {
                    Some(Target::Picker(i)) => {
                        let cy = self.picker_centre(ui, i);
                        if let Some(p) = self.picker_mut(i) {
                            p.moved(y - cy, t);
                        }
                    }
                    _ => {}
                }
                vec![]
            }
            Event::Up(..) => {
                if let Some(sw) = self.swipe.take() {
                    if sw.active {
                        self.pressed = None;
                        self.swipe_dirty = true;
                        let (_, _, cw, _) = Self::alarm_card(ui, sw.idx);
                        if -sw.dx > cw * SWIPE_DELETE && sw.idx < self.alarms.list.len() {
                            self.alarms.list.remove(sw.idx);
                            self.editing &= !self.alarms.list.is_empty();
                            return self.persist(t);
                        }
                        return vec![];
                    }
                }
                let pressed = self.pressed.take();
                match pressed {
                    Some(Target::Picker(i)) => {
                        let cy = self.picker_centre(ui, i);
                        if let Some(p) = self.picker_mut(i) {
                            p.up(y - cy, t);
                        }
                        vec![]
                    }
                    Some(p) if Some(p) == self.hit(ui, x, y) => self.activate(ui, p, x, t),
                    _ => vec![],
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => vec![],
        }
    }

    /// A typed key for the add-alarm popover's label field.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        let Some(a) = &mut self.add else { return };
        match out {
            Out::Hide => {
                a.label.focused = false;
            }
            Out::Enter => {
                a.label.edit(ui, &out, t);
                a.label.focused = false;
            }
            out => {
                a.label.edit(ui, &out, t);
            }
        }
    }

    fn open_sheet(&mut self, ui: &Ui, hour: usize, min: usize, alarm: Option<&Alarm>) {
        let row = ui.pt(POP_ROW_PT);
        let mut label = TextField::new(ui, "Alarm", MAC_TITLE2_PT, false, LABEL_MAX);
        label.focused = false;
        let template = alarm.cloned().unwrap_or_else(|| Alarm::new(0, 0));
        if let Some(a) = alarm {
            label.set_text(ui, &a.label);
        }
        let sound = alarm::SOUNDS.iter().position(|s| *s == template.sound).unwrap_or(0);
        self.add = Some(AddSheet {
            page: SheetPage::Main,
            hour: Picker::new(24, row, hour),
            min: Picker::new(60, row, min),
            label,
            edit: alarm.map(|a| a.id),
            repeat: template.repeat,
            sound,
            vibrate: template.vibrate,
            ramp: template.ramp,
            snooze: template.snooze,
        });
        self.editing = false;
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, t: f64) -> Vec<Effect> {
        match target {
            Target::Tab(i) => {
                self.tab = TABS[i].0;
                self.editing = false;
            }
            Target::Left => match self.tab {
                Tab::Stopwatch => {
                    self.stopwatch.lap_or_reset(t);
                    self.persist_stopwatch(t);
                }
                Tab::Timer => {
                    // cancel the newest timer
                    if self.timers.list.pop().is_some() {
                        return self.persist_timers(t);
                    }
                }
                _ => {}
            },
            Target::Right => match self.tab {
                Tab::Stopwatch => {
                    self.stopwatch.toggle(t);
                    self.persist_stopwatch(t);
                }
                Tab::Timer => {
                    let ms = self.picked_ms();
                    if self.timers.start("", ms, t, self.wall.now_ms(t)).is_some() {
                        return self.persist_timers(t);
                    }
                }
                _ => {}
            },
            Target::Preset(i) => {
                let (name, ms) = timer::PRESETS[i];
                if self.timers.start(name, ms, t, self.wall.now_ms(t)).is_some() {
                    return self.persist_timers(t);
                }
            }
            Target::TimerRow(i) => {
                if let Some(id) = self.timers.list.get(i).map(|item| item.id) {
                    self.timers.toggle(id, t, self.wall.now_ms(t));
                    return self.persist_timers(t);
                }
            }
            Target::TimerDelete(i) => {
                if let Some(id) = self.timers.list.get(i).map(|item| item.id) {
                    self.timers.remove(id);
                    return self.persist_timers(t);
                }
            }
            Target::Add => {
                if self.tab == Tab::World {
                    self.add_city_sheet(ui);
                    return vec![];
                }
                let now = civil(self.wall.now(t), self.wall.offset);
                self.open_sheet(ui, now.hour as usize, now.min as usize, None);
            }
            Target::EditAlarm(i) => {
                if let Some(a) = self.alarms.list.get(i).cloned() {
                    self.open_sheet(ui, a.hour as usize, a.min as usize, Some(&a));
                }
            }
            Target::Edit => self.editing = !self.editing && if self.tab == Tab::World { !self.world.selected.is_empty() } else { !self.alarms.list.is_empty() },
            Target::Switch(i) => {
                if let Some(a) = self.alarms.list.get_mut(i) {
                    a.on = !a.on;
                    a.skip_day = None;
                    return self.persist(t);
                }
            }
            Target::Skip(i) => {
                if let Some(id) = self.alarms.list.get(i).map(|a| a.id) {
                    let skipped = self.alarms.list[i].skip_day.is_some();
                    if skipped {
                        self.alarms.unskip(id);
                    } else {
                        let f = offset_fn(self.zone, self.wall.offset);
                        self.alarms.skip_next(id, self.wall.now(t), &f);
                    }
                    return self.persist(t);
                }
            }
            Target::Delete(i) => {
                if i < self.alarms.list.len() {
                    self.alarms.list.remove(i);
                    self.editing &= !self.alarms.list.is_empty();
                    return self.persist(t);
                }
            }
            Target::Cancel | Target::Outside => self.add = None,
            Target::Save => {
                if let Some(a) = self.add.take() {
                    let label = if a.label.is_empty() { "Alarm".to_string() } else { a.label.text().to_string() };
                    let base = a.edit.and_then(|id| self.alarms.index_of(id)).map(|i| self.alarms.list[i].clone());
                    let alarm = Alarm {
                        hour: a.hour.selected() as u32,
                        min: a.min.selected() as u32,
                        label,
                        on: true,
                        repeat: a.repeat,
                        sound: alarm::SOUNDS[a.sound].to_string(),
                        snooze: a.snooze,
                        vibrate: a.vibrate,
                        ramp: a.ramp,
                        ..base.clone().unwrap_or_else(|| Alarm::new(0, 0))
                    };
                    if base.is_some() {
                        self.alarms.replace(alarm);
                    } else {
                        self.alarms.add(alarm);
                    }
                    return self.persist(t);
                }
            }
            Target::Day(i) => {
                if let Some(a) = &mut self.add {
                    a.repeat ^= 1 << ((i + 1) % 7);
                }
            }
            Target::Tone(i) => {
                if let Some(a) = &mut self.add {
                    a.sound = i.min(alarm::SOUNDS.len() - 1);
                }
            }
            Target::Page(page) => {
                if let Some(a) = &mut self.add {
                    a.page = page;
                    a.label.focused = false;
                }
            }
            Target::Back => {
                if let Some(a) = &mut self.add {
                    a.page = SheetPage::Main;
                }
            }
            Target::Toggle(k) => {
                if let Some(a) = &mut self.add {
                    match k {
                        0 => a.vibrate = !a.vibrate,
                        1 => a.ramp = !a.ramp,
                        _ => a.snooze = !a.snooze,
                    }
                }
            }
            Target::Field => {
                let text_x = self.pop_field(ui).0 + ui.pt(SPACE_L);
                if let Some(a) = &mut self.add {
                    a.label.tap(ui, x - text_x, t);
                }
            }
            Target::Picker(_) => {}
        }
        vec![]
    }

    /// The duration the timer pickers show, in ms.
    fn picked_ms(&self) -> i64 {
        let [h, m, s] = &self.timer_pickers;
        (h.selected() as i64 * 3600 + m.selected() as i64 * 60 + s.selected() as i64) * MS_PER_SEC
    }

    // ----- time -----

    /// Advances pickers and the countdown, and picks up what the shell's
    /// service changed on disk. Returns whether the screen changed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut dirty = self.world_tick(ui, t);
        for p in &mut self.timer_pickers {
            dirty |= p.tick(t);
        }
        dirty |= std::mem::take(&mut self.swipe_dirty);
        if let Some(a) = &mut self.add {
            dirty |= a.hour.tick(t) | a.min.tick(t);
            dirty |= a.label.tick(ui, t);
            if let Some(due) = a.label.next_wake(t) {
                dirty |= due <= t + 1.0;
            }
        }
        if t >= self.next_poll {
            self.next_poll = t + 1000.0;
            if let Some(p) = self.path.clone() {
                if store::stamp(&p) != self.seen.get() {
                    self.reload(&p, t);
                    dirty = true;
                }
            }
        }
        let sec = self.wall.now(t);
        let per_second = self.tab == Tab::World || (self.tab == Tab::Timer && self.timers.any_running());
        if per_second && sec != self.drawn_sec {
            dirty = true;
        }
        dirty |= self.tab == Tab::Stopwatch && self.stopwatch.running();
        dirty
    }

    /// When the app next needs a frame or a wake-up on its own: pickers and
    /// keyboard sliding, a running stopwatch (every frame), countdown and
    /// world-clock seconds, the caret blink, a timer or alarm coming due.
    pub fn next_wake(&self, t: f64, visible: bool) -> Option<f64> {
        let mut due: Option<f64> = None;
        let mut want = |at: f64| due = Some(due.map_or(at, |d: f64| d.min(at)));
        if visible {
            want(self.next_poll.max(t));
        } else {
            return None;
        }
        let elapsed = (t - self.wall.at).rem_euclid(MS_PER_SEC_F);
        let next_second = t + 1000.0 - elapsed;
        match self.tab {
            Tab::World => want(next_second),
            Tab::Stopwatch if self.stopwatch.running() => want(t),
            Tab::Timer if self.timers.any_running() => {
                let next = self.timers.list.iter().filter(|i| i.running()).map(|i| {
                    let rem = i.remaining(t).rem_euclid(1000.0);
                    if rem == 0.0 { 1000.0 } else { rem }
                });
                want(t + next.fold(MS_PER_SEC_F, f64::min));
            }
            _ => {}
        }
        if self.timer_pickers.iter().any(Picker::animating) || self.swipe_dirty || self.swipe.is_some_and(|s| s.active) {
            want(t);
        }
        if let Some(a) = &self.add {
            if a.hour.animating() || a.min.animating() {
                want(t);
            }
            if let Some(b) = a.label.next_wake(t) {
                want(b);
            }
        }
        if let Some(at) = self.world_next_wake(t) { want(at); }
        due
    }

    // ----- drawing -----

    #[allow(clippy::too_many_arguments)]
    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, align: f32) -> f32 {
        let img = self.cache.get(ui, text, pt, weight, color);
        let (w, h) = (img.w as f32, img.h as f32);
        s.blit(img, (x - w * align) as i32, (mid - h / 2.0) as i32, 255, 255);
        w
    }

    /// Text that changes often, drawn glyph by glyph; `align` 0 = left,
    /// 0.5 = centred, 1 = right of `x`.
    #[allow(clippy::too_many_arguments)]
    fn dynamic(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, align: f32) {
        let w = ui.font.measure(text, ui.pt(pt));
        let h = self.cache.line_h(ui, pt);
        self.cache.draw_chars(s, ui, text, pt, weight, color, x - w * align, mid - h / 2.0);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rect(0, 0, self.w as usize, self.h as usize, DETAIL_BG.now());
        self.drawn_sec = self.wall.now(t);
        match self.tab {
            Tab::World => self.draw_world(s, ui, t),
            Tab::Alarm => self.draw_alarms(s, ui, t),
            Tab::Stopwatch => self.draw_stopwatch(s, ui, t),
            Tab::Timer => self.draw_timer(s, ui, t),
        }
        self.draw_bar(s, ui);
        if self.tab == Tab::World { self.draw_world_notice(s, ui); }
        if self.add.is_some() {
            self.draw_add(s, ui, t);
        }
        self.draw_world_sheet(s, ui, t);
    }

    /// The unified toolbar band with the segmented tab control (Golden Gate Clock), Edit and Add.
    fn draw_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let w = self.w as f32 / ui.pt(1.0);
        widgets::toolbar_band(s, ui, Rect { x: 0.0, y: 0.0, w, h: UNIFIED_BAR_PT });
        let pressed = match self.pressed {
            Some(Target::Edit) => Some(BAR_EDIT),
            Some(Target::Add) => Some(BAR_ADD),
            Some(Target::Tab(i)) => Some(BAR_TAB + i as u32),
            _ => None,
        };
        widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(), pressed);
    }

    /// Round Stopwatch/Timer button: `fill` disc inside a thin ring of the
    /// same colour, text in `ink`.
    fn draw_button(&mut self, s: &mut Surface, ui: &Ui, right: bool, text: &str, fill: u32, ink: u32) {
        let c = self.button_centre(ui, right);
        let r = ui.pt(BUTTON_R_PT);
        glyphs::paint(s, (c.0 - r - 2.0, c.1 - r - 2.0, 2.0 * r + 4.0, 2.0 * r + 4.0), fill, glyphs::either(glyphs::disc(c, r - ui.pt(SPACE_XS)), glyphs::ring(c, r - ui.pt(0.75), ui.pt(1.5))));
        self.label(s, ui, text, MAC_TITLE3_PT, Weight::Regular, ink, c.0, c.1, 0.5);
    }

    fn draw_world(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let unix = self.wall.now(t);
        let home = self.wall.offset;
        self.fit_world(ui);
        if self.world.selected.is_empty() {
            self.label(s, ui, "No World Clocks", MAC_LARGE_TITLE_PT, Weight::Regular, LABEL_3.now(), self.w as f32 / 2.0, (Self::content_top(ui) + self.tab_top(ui)) / 2.0, 0.5);
            self.label(s, ui, "Tap + to add a city", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, (Self::content_top(ui) + self.tab_top(ui)) / 2.0 + ui.pt(SPACE_HUGE), 0.5);
            return;
        }
        let mut content = s.rows(Self::content_top(ui) as usize, self.tab_top(ui) as usize);
        let s = &mut content;
        for (i, index) in self.world.selected.clone().into_iter().enumerate() {
            let city = &CITIES[index];
            let (x, y, w, h) = self.world_card(ui, i);
            if y + h < Self::content_top(ui) || y >= self.tab_top(ui) { continue }
            s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
            let local = city.local(unix);
            let day = tz::is_daytime(local.hour);
            let (face, ink) = if day { (WHITE, BLACK) } else { (BLACK, WHITE) };
            let c = (x + w / 2.0, y + ui.pt(95.0));
            let r = ui.pt(70.0);
            glyphs::paint(s, (c.0 - r - 1.0, c.1 - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), face, glyphs::disc(c, r));
            // minute and hour ticks, one paint each
            let face_box = (c.0 - r - 1.0, c.1 - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0);
            let minor = if day { LABEL_2.light } else { LABEL_2.dark };
            glyphs::paint(s, face_box, minor, ticks(c, r * 0.95, r * 0.985, 60, ui.pt(0.8)));
            glyphs::paint(s, face_box, ink, ticks(c, r * 0.915, r * 0.985, 12, ui.pt(SPACE_XXS)));
            for n in 1..=12 {
                let a = n as f32 / 12.0 * std::f32::consts::TAU;
                let (nx, ny) = (c.0 + r * 0.8 * a.sin(), c.1 - r * 0.8 * a.cos());
                self.label(s, ui, &n.to_string(), MAC_BODY_PT, Weight::Regular, ink, nx, ny, 0.5);
            }
            let hand = |frac: f32, len: f32| (c.0 + r * len * (frac * std::f32::consts::TAU).sin(), c.1 - r * len * (frac * std::f32::consts::TAU).cos());
            let hour_frac = (local.hour % 12) as f32 / 12.0 + local.min as f32 / (12.0 * 60.0);
            let min_frac = local.min as f32 / 60.0 + local.sec as f32 / 3600.0;
            let sec_frac = local.sec as f32 / 60.0;
            let bbox = (c.0 - r, c.1 - r, 2.0 * r, 2.0 * r);
            glyphs::paint(s, bbox, ink, glyphs::either(glyphs::segment(c, hand(hour_frac, 0.5), ui.pt(SPACE_XS)), glyphs::segment(c, hand(min_frac, 0.72), ui.pt(SPACE_XS))));
            glyphs::paint(s, bbox, ORANGE, glyphs::either(glyphs::segment(hand(sec_frac + 0.5, 0.15), hand(sec_frac, 0.8), ui.pt(1.3)), glyphs::disc(c, ui.pt(SPACE_XS))));
            let line = format!("{}, {:02}:{:02}", city.name, local.hour, local.min);
            self.dynamic(s, ui, &line, MAC_TITLE3_PT, Weight::Regular, LABEL.now(), c.0, y + ui.pt(WORLD_TIME_Y_PT), 0.5);
            let (word, diff) = tz::relative(city, unix, home);
            let rel = format!("{word}, {}", tz::hours_text(diff));
            self.dynamic(s, ui, &rel, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), c.0, y + ui.pt(WORLD_REL_Y_PT), 0.5);
            // day or night at that city, or the move-earlier arrow while editing
            let corner = (x + w - ui.pt(SPACE_XXL), y + ui.pt(SPACE_XXL));
            let sym = ui.pt(SPACE_S);
            let corner_box = (corner.0 - sym - 2.0, corner.1 - sym - 2.0, 2.0 * sym + 4.0, 2.0 * sym + 4.0);
            if self.editing {
                if i > 0 {
                    let k = ui.pt(SPACE_S);
                    let chevron = glyphs::either(glyphs::segment((corner.0 - k, corner.1 + k * 0.5), (corner.0, corner.1 - k * 0.6), ui.pt(SPACE_XXS)), glyphs::segment((corner.0, corner.1 - k * 0.6), (corner.0 + k, corner.1 + k * 0.5), ui.pt(SPACE_XXS)));
                    glyphs::paint(s, corner_box, LINK, chevron);
                }
            } else if day {
                glyphs::paint(s, corner_box, LINK, glyphs::either(glyphs::disc(corner, sym * 0.55), glyphs::ring(corner, sym, ui.pt(1.5))));
            } else {
                glyphs::paint(s, corner_box, LABEL_2.now(), glyphs::disc(corner, sym * 0.8));
                glyphs::paint(s, corner_box, CARD_BG.now(), glyphs::disc((corner.0 + sym * 0.45, corner.1 - sym * 0.3), sym * 0.7));
            }
            if self.editing {
                let badge = (x + ui.pt(SPACE_XL), y + ui.pt(SPACE_XL));
                let r = ui.pt(SPACE_M);
                glyphs::paint(s, (badge.0 - r - 1.0, badge.1 - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), DESTRUCTIVE, glyphs::disc(badge, r));
                glyphs::paint(s, (badge.0 - r, badge.1 - r, 2.0 * r, 2.0 * r), WHITE, glyphs::segment((badge.0 - r * 0.5, badge.1), (badge.0 + r * 0.5, badge.1), ui.pt(SPACE_XXS)));
            }
        }
    }

    /// The nav bar's thin orange plus: 17 pt wide, its right edge 23 pt in.
    fn draw_alarms(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.alarms.list.is_empty() {
            let mid = (Self::content_top(ui) + self.h as f32) / 2.0;
            self.label(s, ui, "No Alarms", MAC_LARGE_TITLE_PT, Weight::Regular, LABEL_3.now(), self.w as f32 / 2.0, mid, 0.5);
            return;
        }
        let alarms = self.alarms.list.clone();
        for (i, a) in alarms.iter().enumerate() {
            let (cx, y, w, h) = Self::alarm_card(ui, i);
            // a card dragged sideways uncovers its Delete button
            let dx = self.swipe.filter(|sw| sw.active && sw.idx == i).map_or(0.0, |sw| sw.dx);
            if dx < 0.0 {
                s.fill_rrect(cx, y, w, h, ui.pt(m::CARD_RADIUS), DESTRUCTIVE);
                self.label(s, ui, "Delete", MAC_BODY_PT, Weight::Semibold, WHITE, cx + w - ui.pt(SPACE_XL), y + h / 2.0, 1.0);
            }
            let x = cx + dx;
            s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
            let left = x + ui.pt(45.0);
            let ink = if a.on { LABEL.now() } else { LABEL_3.now() };
            self.dynamic(s, ui, &a.time_text(), DISPLAY_MEDIUM_PT, Weight::Regular, ink, left, y + ui.pt(75.0), 0.0);
            self.dynamic(s, ui, &ui.fit(&a.label, MAC_TITLE3_PT, w - ui.pt(60.0)), MAC_TITLE3_PT, Weight::Regular, ink, left, y + ui.pt(ALARM_LABEL_Y_PT), 0.0);
            let repeat = format!("Repeat: {}", a.repeat_text());
            self.dynamic(s, ui, &ui.fit(&repeat, MAC_BODY_PT, w - ui.pt(60.0)), MAC_BODY_PT, Weight::Regular, LABEL_2.now(), left, y + ui.pt(ALARM_REPEAT_Y_PT), 0.0);
            draw_switch(s, ui, left, y + ui.pt(ALARM_SWITCH_Y_PT), if a.on { 1.0 } else { 0.0 });
            if a.on && a.repeat != 0 {
                let (word, color) = if a.skip_day.is_some() { ("Skipped", LABEL_2.now()) } else { ("Skip", LINK) };
                self.label(s, ui, word, MAC_BODY_PT, Weight::Regular, color, x + ui.pt(ALARM_SKIP_X_PT), y + ui.pt(ALARM_SKIP_Y_PT), 0.0);
            }
            if let Some(until) = a.snoozed_until {
                let left = (until - self.wall.now(t)).max(0);
                let text = format!("Snoozed, {}", alarm::eta_text(left));
                self.dynamic(s, ui, &text, MAC_SUBHEAD_PT, Weight::Regular, LINK, x + w - ui.pt(SPACE_XXL), y + ui.pt(75.0), 1.0);
            }
            if self.editing {
                let c = (x + ui.pt(SPACE_XXL), y + ui.pt(75.0));
                let r = ui.pt(SPACE_M);
                glyphs::paint(s, (c.0 - r - 1.0, c.1 - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), DESTRUCTIVE, glyphs::disc(c, r));
                glyphs::paint(s, (c.0 - r, c.1 - r, 2.0 * r, 2.0 * r), WHITE, glyphs::segment((c.0 - r * 0.5, c.1), (c.0 + r * 0.5, c.1), ui.pt(SPACE_XXS)));
            }
        }
        if let Some(text) = self.next_alarm_text(t) {
            self.label(s, ui, &text, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, self.tab_top(ui) - ui.pt(SPACE_XL), 0.5);
        }
    }

    fn draw_stopwatch(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let elapsed = self.stopwatch.elapsed(t);
        self.dynamic(s, ui, &stopwatch::format(elapsed), DISPLAY_LARGE_PT, Weight::Regular, LABEL.now(), self.w as f32 / 2.0, ui.pt(DIGITS_Y_PT), 0.5);
        let running = self.stopwatch.running();
        let started = self.stopwatch.started(t);
        let (left, fill, ink) = if running { ("Lap", CONTROL_PRESSED.now(), LABEL.now()) } else if started { ("Reset", CONTROL_PRESSED.now(), LABEL.now()) } else { ("Lap", CARD_BG.now(), LABEL_3.now()) };
        self.draw_button(s, ui, false, left, fill, ink);
        if running {
            self.draw_button(s, ui, true, "Stop", CARD_BG.now(), LABEL.now());
        } else {
            self.draw_button(s, ui, true, "Start", CARD_BG.now(), LINK);
        }
        let (x0, x1) = (ui.pt(TABLE_X_PT), self.w as f32 - ui.pt(TABLE_X_PT));
        let mid = (x0 + x1) / 2.0;
        let hy = ui.pt(TABLE_Y_PT);
        self.label(s, ui, "LAP NO.", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), x0, hy, 0.0);
        self.label(s, ui, "SPLIT", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), mid, hy, 0.5);
        self.label(s, ui, "TOTAL", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), x1, hy, 1.0);
        s.fill_rrect(x0, hy + ui.pt(SPACE_XL), x1 - x0, 1.0, 0.0, SEPARATOR_C.now());
        let rows = ((self.tab_top(ui) - hy - ui.pt(SPACE_XXL)) / ui.pt(LAP_ROW_PT)).max(0.0) as usize;
        let rh = ui.pt(LAP_ROW_PT);
        let laps = self.stopwatch.laps(t);
        let extremes = stopwatch::extremes(&laps, running);
        for (i, lap) in laps.into_iter().take(rows).enumerate() {
            let y = hy + ui.pt(SPACE_XL) + rh * i as f32;
            let m = y + rh / 2.0;
            // the best finished lap in green, the worst in red
            let tint = match extremes {
                Some((best, _)) if best == lap.number => LINK,
                Some((_, worst)) if worst == lap.number => LABEL_2.now(),
                _ => LABEL.now(),
            };
            self.dynamic(s, ui, &format!("Lap {}", lap.number), MAC_TITLE3_PT, Weight::Regular, tint, x0, m, 0.0);
            self.dynamic(s, ui, &stopwatch::format(lap.split), MAC_TITLE3_PT, Weight::Regular, tint, mid, m, 0.5);
            self.dynamic(s, ui, &stopwatch::format(lap.total), MAC_TITLE3_PT, Weight::Regular, LABEL.now(), x1, m, 1.0);
            s.fill_rrect(x0, y + rh - 1.0, x1 - x0, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// One picker column of `count` rows scrolled to `pos`: rows fade and
    /// shrink away from the centre.
    #[allow(clippy::too_many_arguments)]
    fn draw_picker(&mut self, s: &mut Surface, ui: &Ui, count: usize, pos: f32, cx: f32, cy: f32, row_h: f32, pad: bool, align: f32) {
        let first = (pos - VISIBLE_ROWS / 2.0).floor().max(0.0) as usize;
        let last = ((pos + VISIBLE_ROWS / 2.0).ceil().max(0.0) as usize).min(count.saturating_sub(1));
        for i in first..=last {
            let d = (i as f32 - pos).abs();
            if d > VISIBLE_ROWS / 2.0 || count == 0 {
                continue;
            }
            let y = cy + (i as f32 - pos) * row_h;
            let f = (1.0 - d / (VISIBLE_ROWS / 2.0)).clamp(0.0, 1.0);
            let alpha = (255.0 * f.powf(1.4)) as u8;
            let color = if d < 0.5 { LABEL.now() } else { fade(LABEL_2.now(), alpha) };
            let text = if pad { format!("{i:02}") } else { i.to_string() };
            let size = MAC_TITLE1_PT + 1.0 - 3.0 * (d / (VISIBLE_ROWS / 2.0));
            self.dynamic(s, ui, &text, size, Weight::Regular, color, cx, y, align);
        }
    }

    fn draw_timer(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let cy = ui.pt(PICKER_Y_PT);
        let (x0, _, _) = self.timer_column(ui, 0);
        let band_h = ui.pt(PICKER_ROW_PT);
        s.fill_rrect(x0, cy - band_h / 2.0, 3.0 * ui.pt(PICKER_COL_PT), band_h, ui.pt(m::CARD_RADIUS), FIELD_BG.now());
        let columns: Vec<(usize, f32)> = self.timer_pickers.iter().map(|p| (p.count, p.position())).collect();
        for (i, unit) in ["hours", "min", "sec"].iter().enumerate() {
            let (x, w, _) = self.timer_column(ui, i);
            let (count, pos) = columns[i];
            self.draw_picker(s, ui, count, pos, x + w * 0.42, cy, band_h, false, 1.0);
            self.label(s, ui, unit, MAC_BODY_PT, Weight::Semibold, LABEL.now(), x + w * 0.42 + ui.pt(SPACE_S), cy, 0.0);
        }
        for (i, (name, _)) in timer::PRESETS.iter().enumerate() {
            let (px, py, pw, ph) = self.preset_rect(ui, i);
            s.fill_rrect(px, py, pw, ph, 1.0e4, CARD_BG.now());
            self.label(s, ui, name, MAC_SUBHEAD_PT, Weight::Regular, LINK, px + pw / 2.0, py + ph / 2.0, 0.5);
        }
        let items: Vec<(String, f64, f32, bool)> = self.timers.list.iter().map(|i| (i.label.clone(), i.remaining(t), i.fraction(t), i.running())).collect();
        for (i, (label, rem, frac, running)) in items.iter().take(TIMER_ROWS).enumerate() {
            let (rx, ry, rw, rh) = self.timer_row(ui, i);
            let body = rh - ui.pt(SPACE_XS);
            s.fill_rrect(rx, ry, rw, body, ui.pt(m::CARD_RADIUS), CARD_BG.now());
            s.fill_rrect(rx, ry + body - ui.pt(SPACE_XS), rw * frac, ui.pt(SPACE_XS), 0.0, if *running { LINK } else { LABEL_3.now() });
            let mid = ry + (body - ui.pt(SPACE_XS)) / 2.0;
            self.dynamic(s, ui, &ui.fit(label, MAC_BODY_PT, rw * 0.4), MAC_BODY_PT, Weight::Regular, LABEL.now(), rx + ui.pt(SPACE_L), mid, 0.0);
            if !*running {
                self.label(s, ui, "Paused", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), rx + rw * 0.5, mid, 0.5);
            }
            self.dynamic(s, ui, &timer::format(*rem), MAC_TITLE1_PT, Weight::Regular, if *running { LABEL.now() } else { LABEL_3.now() }, rx + rw - ui.pt(70.0), mid, 1.0);
            let c = (rx + rw - ui.pt(28.0), mid);
            let k = ui.pt(SPACE_S);
            let cross = glyphs::either(glyphs::segment((c.0 - k, c.1 - k), (c.0 + k, c.1 + k), ui.pt(SPACE_XXS)), glyphs::segment((c.0 - k, c.1 + k), (c.0 + k, c.1 - k), ui.pt(SPACE_XXS)));
            glyphs::paint(s, (c.0 - k - 2.0, c.1 - k - 2.0, 2.0 * k + 4.0, 2.0 * k + 4.0), LABEL_2.now(), cross);
        }
        if items.len() > TIMER_ROWS {
            let (_, ry, _, _) = self.timer_row(ui, TIMER_ROWS);
            self.label(s, ui, &format!("+{} more", items.len() - TIMER_ROWS), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, ry + ui.pt(SPACE_S), 0.5);
        }
        let (fill, ink) = if items.is_empty() { (CARD_BG.now(), LABEL_3.now()) } else { (CONTROL_PRESSED.now(), LABEL.now()) };
        self.draw_button(s, ui, false, "Cancel", fill, ink);
        if self.picked_ms() > 0 {
            self.draw_button(s, ui, true, "Start", CARD_BG.now(), LINK);
        } else {
            self.draw_button(s, ui, true, "Start", CARD_BG.now(), LABEL_3.now());
        }
    }

    fn draw_add(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, fade(SCRIM, 90));
        let (px, py, pw, ph) = self.popover(ui);
        s.fill_rrect(px, py, pw, ph, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let bar = py + ui.pt(ROW_PT / 2.0);
        let page = self.add.as_ref().map_or(SheetPage::Main, |a| a.page);
        let title = match (page, self.add.as_ref().is_some_and(|a| a.edit.is_some())) {
            (SheetPage::Repeat, _) => "Repeat",
            (SheetPage::Sound, _) => "Sound",
            (_, true) => "Edit Alarm",
            _ => "Add Alarm",
        };
        if page == SheetPage::Main {
            self.label(s, ui, "Cancel", MAC_BODY_PT, Weight::Regular, LINK, px + ui.pt(SPACE_L), bar, 0.0);
            self.label(s, ui, "Save", MAC_BODY_PT, Weight::Semibold, LINK, px + pw - ui.pt(SPACE_L), bar, 1.0);
        } else {
            self.draw_chevron(s, ui, px + ui.pt(SPACE_XXL), bar, true);
            self.label(s, ui, "Back", MAC_BODY_PT, Weight::Regular, LINK, px + ui.pt(SPACE_XXXL), bar, 0.0);
        }
        self.label(s, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), px + pw / 2.0, bar, 0.5);
        if page != SheetPage::Main {
            self.draw_add_page(s, ui, page);
            return;
        }
        let row = ui.pt(POP_ROW_PT);
        let cy = py + ui.pt(POP_PICKER_Y_PT);
        s.fill_rrect(px + ui.pt(SPACE_L), cy - row / 2.0, pw - ui.pt(30.0), row, ui.pt(m::SELECTION_RADIUS), FIELD_BG.now());
        let Some(add) = &self.add else { return };
        let columns = [(add.hour.count, add.hour.position()), (add.min.count, add.min.position())];
        for (i, (count, pos)) in columns.into_iter().enumerate() {
            let (x, w, _) = self.pop_column(ui, i);
            self.draw_picker(s, ui, count, pos, x + w / 2.0, cy, row, true, 0.5);
        }
        self.draw_add_rows(s, ui);
        let frame = self.pop_text_frame(ui);
        if let Some(add) = &mut self.add {
            add.label.draw(s, ui, frame, t);
        }
    }

    /// A disclosure chevron centred at (`x`, `y`), pointing back when `left`.
    fn draw_chevron(&self, s: &mut Surface, ui: &Ui, x: f32, y: f32, left: bool) {
        let (d, h) = (if left { -1.0 } else { 1.0 }, ui.pt(SPACE_S));
        let (tip, tail) = (x + d * ui.pt(SPACE_XS), x - d * ui.pt(SPACE_XS));
        let color = if left { LINK } else { LABEL_2.now() };
        springboard::shell::icons::line(s, (tail, y - h), (tip, y), ui.pt(1.8), color);
        springboard::shell::icons::line(s, (tip, y), (tail, y + h), ui.pt(1.8), color);
    }

    /// An inset group of `n` rows: the FIELD_BG.now() fill and hairlines inset to the text.
    fn draw_group(&self, s: &mut Surface, ui: &Ui, rows: &[(f32, f32, f32, f32)]) {
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else { return };
        s.fill_rrect(first.0, first.1, first.2, last.1 + last.3 - first.1, ui.pt(m::CARD_RADIUS), FIELD_BG.now());
        for r in &rows[1..] {
            s.fill_rrect(r.0 + ui.pt(SPACE_L), r.1, r.2 - ui.pt(SPACE_L), 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// The editor's table: Repeat, Label (drawn with its field), Sound, Snooze.
    fn draw_add_rows(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(add) = &self.add else { return };
        let (repeat, sound, snooze) = (add.repeat, add.sound, add.snooze);
        let rows: Vec<_> = (0..4).map(|k| self.pop_row(ui, k)).collect();
        self.draw_group(s, ui, &rows);
        let values = [Some(alarm::repeat_text(repeat)), None, Some(alarm::SOUNDS[sound].to_string()), None];
        for (k, name) in ["Repeat", "Label", "Sound", "Snooze"].iter().enumerate() {
            let (x, y, w, h) = rows[k];
            self.label(s, ui, name, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + h / 2.0, 0.0);
            if let Some(v) = &values[k] {
                self.label(s, ui, v, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + w - ui.pt(36.0), y + h / 2.0, 1.0);
                self.draw_chevron(s, ui, x + w - ui.pt(SPACE_XL), y + h / 2.0, false);
            }
        }
        let (x, y, w, h) = rows[3];
        draw_switch(s, ui, x + w - ui.pt(SPACE_L) - ui.pt(51.0), y + (h - ui.pt(31.0)) / 2.0, if snooze { 1.0 } else { 0.0 });
    }

    /// The pushed Repeat page (a day checklist) or Sound page (tones with a
    /// checkmark, then Vibrate and Fade In).
    fn draw_add_page(&mut self, s: &mut Surface, ui: &Ui, page: SheetPage) {
        let Some(add) = &self.add else { return };
        let (repeat, sound, switches) = (add.repeat, add.sound, [add.vibrate, add.ramp]);
        let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
        let (names, checked): (Vec<&str>, Vec<bool>) = if page == SheetPage::Repeat {
            (days.iter().map(|d| *d).collect(), (0..7).map(|i| repeat & (1 << ((i + 1) % 7)) != 0).collect())
        } else {
            (alarm::SOUNDS.to_vec(), (0..alarm::SOUNDS.len()).map(|i| i == sound).collect())
        };
        let rows: Vec<_> = (0..names.len()).map(|k| self.pop_list_row(ui, k)).collect();
        self.draw_group(s, ui, &rows);
        for (k, &(x, y, w, h)) in rows.iter().enumerate() {
            self.label(s, ui, names[k], MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + h / 2.0, 0.0);
            if checked[k] {
                let (cx, cy) = (x + w - ui.pt(SPACE_XXL), y + h / 2.0);
                springboard::shell::icons::line(s, (cx - ui.pt(SPACE_S), cy), (cx - ui.pt(SPACE_XXS), cy + ui.pt(SPACE_XS)), ui.pt(SPACE_XXS), LINK);
                springboard::shell::icons::line(s, (cx - ui.pt(SPACE_XXS), cy + ui.pt(SPACE_XS)), (cx + ui.pt(SPACE_S), cy - ui.pt(SPACE_S)), ui.pt(SPACE_XXS), LINK);
            }
        }
        if page == SheetPage::Sound {
            let n = alarm::SOUNDS.len();
            let rows: Vec<_> = (n..n + 2).map(|k| self.pop_list_row(ui, k)).collect();
            self.draw_group(s, ui, &rows);
            for (k, name) in ["Vibrate", "Fade In"].iter().enumerate() {
                let (x, y, w, h) = rows[k];
                self.label(s, ui, name, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + h / 2.0, 0.0);
                draw_switch(s, ui, x + w - ui.pt(SPACE_L) - ui.pt(51.0), y + (h - ui.pt(31.0)) / 2.0, if switches[k] { 1.0 } else { 0.0 });
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn ui() -> Ui {
        crate::test_ui()
    }

    pub fn clock(ui: &Ui) -> Clock {
        let mut c = Clock::new(ui, 1200, 1920);
        // Sat 3 Oct 2026 23:32:00 UTC, Reykjavik
        c.set_wall(tz::days_from_civil(2026, 10, 3) * 86_400 + 23 * 3600 + 32 * 60, 0, 1000.0);
        c
    }

    pub fn tap(c: &mut Clock, ui: &Ui, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let mut fx = c.touch(ui, Event::Down(x, y), t);
        fx.extend(c.touch(ui, Event::Up(x, y), t + 20.0));
        fx
    }

    fn select_tab(c: &mut Clock, ui: &Ui, i: usize) {
        let (x, y) = bar_point(c, ui, BAR_TAB + i as u32);
        tap(c, ui, x, y, 0.0);
    }

    /// The centre of toolbar item `id`, in pixels.
    pub fn bar_point(c: &Clock, ui: &Ui, id: u32) -> (f32, f32) {
        let r = c.bar_layout(ui).rect_of(id).expect("item on the toolbar");
        (ui.pt(r.x + r.w / 2.0), ui.pt(r.y + r.h / 2.0))
    }

    fn draw(c: &mut Clock, ui: &Ui, t: f64) -> Vec<u32> {
        let mut buf = vec![0u32; 1200 * 1920];
        c.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, t);
        buf
    }

    #[test]
    fn tabs_switch_and_every_tab_draws() {
        let ui = ui();
        let mut c = clock(&ui);
        assert_eq!(c.tab(), Tab::World);
        for (i, (tab, _)) in TABS.iter().enumerate() {
            select_tab(&mut c, &ui, i);
            assert_eq!(c.tab(), *tab);
            // the tabs are the toolbar's segmented control (Golden Gate Clock): the selected segment is a neutral plate
            let buf = draw(&mut c, &ui, 2000.0);
            assert!(buf.iter().any(|&p| p != 0), "the {tab:?} tab draws");
        }
    }

    #[test]
    fn stopwatch_buttons_start_lap_stop_reset() {
        let ui = ui();
        let mut c = clock(&ui);
        select_tab(&mut c, &ui, 2);
        let right = c.button_centre(&ui, true);
        let left = c.button_centre(&ui, false);
        assert_eq!(c.hit(&ui, right.0, right.1), Some(Target::Right));
        tap(&mut c, &ui, right.0, right.1, 1000.0);
        assert!(c.stopwatch().running());
        tap(&mut c, &ui, left.0, left.1, 4610.0);
        assert_eq!(c.stopwatch().laps(5000.0).len(), 2);
        assert_eq!(c.next_wake(5000.0, true), Some(5000.0), "running stopwatch wants every frame");
        tap(&mut c, &ui, right.0, right.1, 6000.0);
        assert!(!c.stopwatch().running());
        c.tick(&ui, 7000.0);
        assert!(c.next_wake(7000.0, true).is_none_or(|w| w > 7000.0), "stopped: only the file poll remains");
        draw(&mut c, &ui, 7000.0);
        tap(&mut c, &ui, left.0, left.1, 8000.0);
        assert!(!c.stopwatch().started(8000.0), "reset");
        select_tab(&mut c, &ui, 0);
        select_tab(&mut c, &ui, 2);
        assert!(!c.stopwatch().started(9000.0));
    }

    #[test]
    fn timer_picks_runs_and_fires_an_alert() {
        let ui = ui();
        let mut c = clock(&ui);
        select_tab(&mut c, &ui, 3);
        let (sx, sw, cy) = c.timer_column(&ui, 2);
        let row = ui.pt(PICKER_ROW_PT);
        // tap one row below the centre of the seconds column: 1 s
        tap(&mut c, &ui, sx + sw / 2.0, cy + row, 100.0);
        c.tick(&ui, 1000.0);
        let (mx, mw, _) = c.timer_column(&ui, 1);
        // drag the minutes down by fifteen rows and lift: 0 min
        c.touch(&ui, Event::Down(mx + mw / 2.0, cy), 1100.0);
        c.touch(&ui, Event::Move(mx + mw / 2.0, cy + 15.0 * row), 1116.0);
        c.touch(&ui, Event::Move(mx + mw / 2.0, cy + 15.0 * row), 1300.0);
        c.touch(&ui, Event::Up(mx + mw / 2.0, cy + 15.0 * row), 1400.0);
        c.tick(&ui, 2000.0);
        assert_eq!(timer::split(c.picked_ms() as f64), (0, 0, 1));
        let right = c.button_centre(&ui, true);
        tap(&mut c, &ui, right.0, right.1, 3000.0);
        assert_eq!(c.timers().list.len(), 1);
        assert!(c.timers().list[0].running());
        assert_eq!(c.next_wake(3500.0, false), None, "a hidden app needs no wake: the shell service rings the timer");
        draw(&mut c, &ui, 3500.0);
        // the shell service removes the timer once it has rung; the app picks that up from the file
        c.timers.poll(4020.0);
        // several at once: each gets its own label, the left button cancels the newest
        let left = c.button_centre(&ui, false);
        tap(&mut c, &ui, right.0, right.1, 6000.0);
        tap(&mut c, &ui, right.0, right.1, 6100.0);
        assert_eq!(c.timers().list.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Timer", "Timer 2"]);
        tap(&mut c, &ui, left.0, left.1, 6200.0);
        assert_eq!(c.timers().list.len(), 1, "cancel");
        // the row pauses and resumes, its cross deletes
        let (rx, ry, rw, rh) = c.timer_row(&ui, 0);
        tap(&mut c, &ui, rx + ui.pt(30.0), ry + rh / 2.0, 6300.0);
        assert!(!c.timers().list[0].running());
        draw(&mut c, &ui, 6400.0);
        tap(&mut c, &ui, rx + ui.pt(30.0), ry + rh / 2.0, 6500.0);
        assert!(c.timers().list[0].running());
        tap(&mut c, &ui, rx + rw - ui.pt(20.0), ry + rh / 2.0, 6600.0);
        assert!(c.timers().list.is_empty());
        // a preset starts at once
        let (px, py, pw, ph) = c.preset_rect(&ui, 2);
        tap(&mut c, &ui, px + pw / 2.0, py + ph / 2.0, 7000.0);
        assert_eq!((c.timers().list[0].label.as_str(), c.timers().list[0].duration_ms), ("5 min", 300_000));
    }

    fn scratch(name: &str) -> String {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock").join(std::process::id().to_string()).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("clock.ini").to_string_lossy().into_owned()
    }

    #[test]
    fn timers_and_the_stopwatch_survive_a_restart_by_absolute_time() {
        let ui = ui();
        let path = scratch("restart");
        let mut c = clock(&ui);
        c.load(&path, 1000.0);
        let right = c.button_centre(&ui, true);
        let left = c.button_centre(&ui, false);
        // a stopwatch with a lap, still running
        select_tab(&mut c, &ui, 2);
        tap(&mut c, &ui, right.0, right.1, 2000.0);
        tap(&mut c, &ui, left.0, left.1, 5000.0);
        // a ten-minute timer
        select_tab(&mut c, &ui, 3);
        let (px, py, pw, ph) = c.preset_rect(&ui, 3);
        let fx = tap(&mut c, &ui, px + pw / 2.0, py + ph / 2.0, 6000.0);
        let end = (c.wall.now_ms(6020.0) + 600_000 + 999) / 1000;
        assert_eq!(fx, vec![Effect::WakeAt(end)], "the RTC is armed for the timer");
        // the shell restarts a minute later; its frame clock starts over
        let mut again = Clock::new(&ui, 1200, 1920);
        again.set_wall(c.wall.unix + 66, 0, 500.0);
        let fx = again.load(&path, 500.0);
        assert_eq!(again.timers().list.len(), 1);
        assert!((again.timers().list[0].remaining(500.0) - (600_000.0 - 60_000.0)).abs() < 1500.0);
        assert_eq!(fx, vec![Effect::WakeAt(end)]);
        assert!(again.stopwatch().running());
        assert_eq!(again.stopwatch().laps(500.0).len(), 2, "the lap is still there");
    }

    #[test]
    fn alarm_cards_edit_repeat_sound_skip_and_swipe_to_delete() {
        let ui = ui();
        let path = scratch("cards");
        let mut c = clock(&ui);
        c.load(&path, 1000.0);
        let id = c.alarms.add(Alarm::new(7, 0));
        select_tab(&mut c, &ui, 1);
        let (cx, cy, cw, _) = Clock::alarm_card(&ui, 0);
        // a tap on the card edits it
        tap(&mut c, &ui, cx + ui.pt(150.0), cy + ui.pt(40.0), 2000.0);
        assert_eq!(c.add.as_ref().and_then(|a| a.edit), Some(id));
        draw(&mut c, &ui, 2100.0);
        let centre = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (x, y) = centre(c.pop_row(&ui, 0));
        tap(&mut c, &ui, x, y, 2150.0);
        assert_eq!(c.add.as_ref().map(|a| a.page), Some(SheetPage::Repeat));
        draw(&mut c, &ui, 2160.0);
        for (r, t) in [(c.pop_list_row(&ui, 0), 2200.0), (c.pop_list_row(&ui, 6), 2300.0)] {
            let (x, y) = centre(r);
            tap(&mut c, &ui, x, y, t);
        }
        let (px0, py0, _, _) = c.popover(&ui);
        tap(&mut c, &ui, px0 + ui.pt(40.0), py0 + ui.pt(20.0), 2350.0);
        assert_eq!(c.add.as_ref().map(|a| a.page), Some(SheetPage::Main));
        let (x, y) = centre(c.pop_row(&ui, 2));
        tap(&mut c, &ui, x, y, 2380.0);
        draw(&mut c, &ui, 2390.0);
        let (x, y) = centre(c.pop_list_row(&ui, 1));
        tap(&mut c, &ui, x, y, 2400.0);
        tap(&mut c, &ui, px0 + ui.pt(40.0), py0 + ui.pt(20.0), 2450.0);
        let (x, y) = centre(c.pop_row(&ui, 3));
        tap(&mut c, &ui, x, y, 2500.0);
        let (px, py, pw, _) = c.popover(&ui);
        tap(&mut c, &ui, px + pw - ui.pt(30.0), py + ui.pt(20.0), 3000.0);
        let a = &c.alarms().list[0];
        assert_eq!(c.alarms().list.len(), 1, "editing replaces, never duplicates");
        assert_eq!(a.id, id);
        assert_eq!(a.repeat, 0b11, "Monday and Sunday");
        assert_eq!(a.repeat_text(), "Mon, Sun");
        assert_eq!(a.sound, "Beacon");
        assert!(!a.snooze && a.vibrate);
        assert!(std::fs::read_to_string(&path).unwrap().contains("alarm.0.repeat=3\n"));
        draw(&mut c, &ui, 3100.0);
        // skip the next occurrence, then restore it
        tap(&mut c, &ui, cx + ui.pt(150.0), cy + ui.pt(210.0), 4000.0);
        assert!(c.alarms().list[0].skip_day.is_some());
        tap(&mut c, &ui, cx + ui.pt(150.0), cy + ui.pt(210.0), 4100.0);
        assert!(c.alarms().list[0].skip_day.is_none());
        assert!(c.next_alarm_text(5000.0).unwrap().starts_with("Alarm in "));
        // dragging the card left past a third of its width deletes it
        let (x, y) = (cx + ui.pt(200.0), cy + ui.pt(40.0));
        c.touch(&ui, Event::Down(x, y), 6000.0);
        c.touch(&ui, Event::Move(x - cw * 0.2, y), 6050.0);
        assert!(c.tick(&ui, 6060.0), "the card follows the finger");
        draw(&mut c, &ui, 6070.0);
        c.touch(&ui, Event::Move(x - cw * 0.5, y), 6100.0);
        c.touch(&ui, Event::Up(x - cw * 0.5, y), 6150.0);
        assert!(c.alarms().list.is_empty());
        assert!(c.add.is_none(), "a swipe is not a tap");
        // a short drag springs back
        let id = c.alarms.add(Alarm::new(8, 0));
        c.touch(&ui, Event::Down(x, y), 7000.0);
        c.touch(&ui, Event::Move(x - cw * 0.1, y), 7050.0);
        c.touch(&ui, Event::Up(x - cw * 0.1, y), 7100.0);
        assert_eq!(c.alarms().list[0].id, id);
        assert!(c.add.is_none());
    }

    #[test]
    fn alarms_add_toggle_delete_and_persist() {
        let ui = ui();
        let dir = std::env::temp_dir().join(format!("springboard-clock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clock.ini");
        let path = path.to_str().unwrap();
        let mut c = clock(&ui);
        assert_eq!(c.load(path, 1000.0), vec![Effect::WakeAt(0)]);
        select_tab(&mut c, &ui, 1);
        let nav = ui.pt(UNIFIED_BAR_PT / 2.0);
        tap(&mut c, &ui, 1200.0 - ui.pt(20.0), nav, 2000.0);
        assert!(c.add.is_some());
        let add = c.add.as_ref().unwrap();
        assert_eq!((add.hour.selected(), add.min.selected()), (23, 32), "popover opens at the current time");
        // tap the row below the minutes: 33
        let (mx, mw, cy) = c.pop_column(&ui, 1);
        tap(&mut c, &ui, mx + mw / 2.0, cy + ui.pt(POP_ROW_PT), 2100.0);
        c.tick(&ui, 3000.0);
        assert_eq!(c.add.as_ref().unwrap().min.selected(), 33);
        // label field: a tap focuses it, typing edits it
        let (fx, fy, _, fh) = c.pop_field(&ui);
        tap(&mut c, &ui, fx + ui.pt(120.0), fy + fh / 2.0, 3100.0);
        assert!(c.label_field().unwrap().focused);
        c.key(&ui, Out::Text("q".into()), 3500.0);
        assert_eq!(c.label_field().unwrap().text(), "q");
        draw(&mut c, &ui, 3600.0);
        let (px, py, pw, _) = c.popover(&ui);
        let fx = tap(&mut c, &ui, px + pw - ui.pt(30.0), py + ui.pt(20.0), 4000.0);
        let due = tz::days_from_civil(2026, 10, 3) * 86_400 + 23 * 3600 + 33 * 60;
        assert_eq!(fx, vec![Effect::WakeAt(due)]);
        assert!(c.add.is_none());
        assert_eq!(c.alarms().list.len(), 1);
        assert_eq!(c.alarms().list[0].label, "q");
        assert!(std::fs::read_to_string(path).unwrap().contains("alarm.0.time=23:33"));
        let (cx, cy, _, _) = Clock::alarm_card(&ui, 0);
        let fx = tap(&mut c, &ui, cx + ui.pt(60.0), cy + ui.pt(210.0), 5000.0);
        assert!(!c.alarms().list[0].on);
        assert_eq!(fx, vec![Effect::WakeAt(0)]);
        draw(&mut c, &ui, 5100.0);
        // edit mode: the minus badge deletes
        let (ex, ey) = bar_point(&c, &ui, BAR_EDIT);
        tap(&mut c, &ui, ex, ey, 7000.0);
        assert!(c.editing);
        tap(&mut c, &ui, cx + ui.pt(22.0), cy + ui.pt(75.0), 7100.0);
        assert!(c.alarms().list.is_empty() && !c.editing);
        assert_eq!(Alarms::parse(&std::fs::read_to_string(path).unwrap()).list.len(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn popover_cancel_and_outside_tap_close_it() {
        let ui = ui();
        let mut c = clock(&ui);
        select_tab(&mut c, &ui, 1);
        let nav = ui.pt(UNIFIED_BAR_PT / 2.0);
        tap(&mut c, &ui, 1200.0 - ui.pt(20.0), nav, 0.0);
        let (px, py, _, _) = c.popover(&ui);
        tap(&mut c, &ui, px + ui.pt(30.0), py + ui.pt(20.0), 100.0);
        assert!(c.add.is_none(), "cancel");
        tap(&mut c, &ui, 1200.0 - ui.pt(20.0), nav, 200.0);
        tap(&mut c, &ui, 100.0, 1000.0, 300.0);
        assert!(c.add.is_none(), "outside");
    }

    #[test]
    fn world_clock_wakes_each_second_and_wall_time_advances() {
        let ui = ui();
        let mut c = clock(&ui);
        assert!(c.next_wake(1300.0, true).is_some_and(|w| w <= 2000.0), "wakes by the next second");
        assert_eq!(c.next_wake(1300.0, false), None);
        assert_eq!(c.wall.now(2999.0), c.wall.unix + 1);
        draw(&mut c, &ui, 1500.0);
        assert!(!c.tick(&ui, 1600.0), "same second, nothing to repaint");
        assert!(c.tick(&ui, 2100.0), "next second repaints the faces");
    }
}
