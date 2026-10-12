//! Settings > General > Game Controllers: the connected controllers with their battery, a live test of the buttons and
//! sticks, and the button remapping saved per controller in the settings file.
//!
//! The rows are built from `services::gamepad_service::PadView`s, the snapshot the gamepad service publishes on every
//! scan and input change. A remap is a one-to-one table over the thirteen buttons (`Button::ALL`): the button you press
//! sends the output it maps to, and the service applies it before an input event reaches an app. Remaps are keyed by
//! the controller's name, because the ids of the controllers are only valid for one session.
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::prefs::Prefs;
use super::services::gamepad_service::{self, PadView};
use bifrost_ui::controller_power::{Battery, BatteryState};
use bifrost_ui::gamepad::{Axis, Button};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

/// The settings key prefix of a controller's remap: `gamepad_remap.<name>`.
pub const PREF_PREFIX: &str = "gamepad_remap.";

/// A one-to-one table: the output of each physical button. Index = the button's discriminant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remap {
    out: [Button; 13],
}

impl Default for Remap {
    fn default() -> Self {
        Remap::identity()
    }
}

impl Remap {
    /// Every button sends its own output.
    pub fn identity() -> Self {
        Remap { out: Button::ALL }
    }

    /// What pressing `from` sends.
    pub fn target(&self, from: Button) -> Button {
        self.out[from as usize]
    }

    /// Makes `from` send `to`. The button that sent `to` before gets what `from` sent, so the two swap and the table
    /// stays one to one.
    pub fn assign(&mut self, from: Button, to: Button) {
        let old = self.target(from);
        if let Some(other) = Button::ALL.iter().copied().find(|b| *b != from && self.target(*b) == to) {
            self.out[other as usize] = old;
        }
        self.out[from as usize] = to;
    }

    /// Moves `from` to the next output in `Button::ALL` order, wrapping after the last one.
    pub fn cycle(&mut self, from: Button) {
        let next = Button::ALL[(self.target(from) as usize + 1) % Button::ALL.len()];
        self.assign(from, next);
    }

    pub fn is_identity(&self) -> bool {
        Button::ALL.iter().all(|b| self.target(*b) == *b)
    }

    /// The table as text: `from>to` pairs for the buttons that changed, in `Button::ALL` order, joined with commas.
    pub fn to_text(&self) -> String {
        Button::ALL
            .iter()
            .filter(|b| self.target(**b) != **b)
            .map(|b| format!("{}>{}", gamepad_service::button_name(*b), gamepad_service::button_name(self.target(*b))))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Reads the text `to_text` writes. Unknown names are skipped, and a table that is not one to one is ignored
    /// (identity), so a damaged setting never leaves two buttons sending the same output.
    pub fn parse(text: &str) -> Self {
        let mut remap = Remap::identity();
        for pair in text.split(',').filter(|p| !p.is_empty()) {
            let Some((from, to)) = pair.split_once('>') else { continue };
            if let (Some(from), Some(to)) = (gamepad_service::button_from_name(from.trim()), gamepad_service::button_from_name(to.trim())) {
                remap.out[from as usize] = to;
            }
        }
        let mut seen: Vec<Button> = remap.out.to_vec();
        seen.sort_by_key(|b| *b as u8);
        seen.dedup();
        if seen.len() == remap.out.len() {
            remap
        } else {
            Remap::identity()
        }
    }
}

/// The settings key of a controller's remap. Characters other than letters and digits become `_`.
pub fn pref_key(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("{PREF_PREFIX}{safe}")
}

/// The remap saved for a controller; identity when none is saved.
pub fn load(prefs: &Prefs, name: &str) -> Remap {
    prefs.get(&pref_key(name)).map_or_else(Remap::identity, Remap::parse)
}

/// A change a row makes. `pad` is the controller's name.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// the physical button moves to its next output
    Cycle { pad: String, from: Button },
    /// every button sends its own output again
    Reset { pad: String },
}

/// Applies an edit: saves the remap and updates the snapshot the page reads. Returns the new table.
pub fn apply(prefs: &mut Prefs, edit: &Edit) -> io::Result<Remap> {
    let (pad, mut remap) = match edit {
        Edit::Cycle { pad, from } => {
            let mut remap = load(prefs, pad);
            remap.cycle(*from);
            (pad, remap)
        }
        Edit::Reset { pad } => (pad, Remap::identity()),
    };
    prefs.set(&pref_key(pad), &remap.to_text())?;
    gamepad_service::set_remap(pad, remap.clone());
    Ok(remap)
}

/// The last generation of the snapshot the Settings page showed; see `take_change`.
static SHOWN: AtomicU64 = AtomicU64::new(0);

/// True once after each change of the controller snapshot, so the open test page refreshes only when something moved.
pub fn take_change() -> bool {
    let now = gamepad_service::generation();
    SHOWN.swap(now, Ordering::Relaxed) != now
}

/// The name shown for a button.
pub fn button_label(b: Button) -> &'static str {
    match b {
        Button::South => "South",
        Button::East => "East",
        Button::North => "North",
        Button::West => "West",
        Button::LeftShoulder => "Left Shoulder",
        Button::RightShoulder => "Right Shoulder",
        Button::LeftTrigger => "Left Trigger",
        Button::RightTrigger => "Right Trigger",
        Button::LeftStick => "Left Stick Click",
        Button::RightStick => "Right Stick Click",
        Button::Select => "Select",
        Button::Start => "Start",
        Button::Home => "Home",
    }
}

/// The battery as text: `72%`, `72% (Charging)`; empty without a report.
pub fn battery_text(b: &Battery) -> String {
    match b.percent() {
        Some(p) if b.state == BatteryState::Charging => format!("{p}% (Charging)"),
        Some(p) => format!("{p}%"),
        None => String::new(),
    }
}

/// The direction pad as a compass point. `y` grows downwards, as the hat reports it.
pub fn dpad_name(x: i8, y: i8) -> &'static str {
    match (x, y) {
        (0, -1) => "Up",
        (1, -1) => "Up Right",
        (1, 0) => "Right",
        (1, 1) => "Down Right",
        (0, 1) => "Down",
        (-1, 1) => "Down Left",
        (-1, 0) => "Left",
        (-1, -1) => "Up Left",
        _ => "Centre",
    }
}

/// A stick or trigger value to two decimals; a value that rounds to zero prints as `0.00`, never `-0.00`.
fn fmt_axis(v: f32) -> String {
    let v = if v.abs() < 0.005 { 0.0 } else { v };
    format!("{v:.2}")
}

/// The live rows of the test view: the controller's battery, then what it is doing now. Labels are fixed, so the
/// Settings page can update a row's value in place.
pub fn live_rows(view: &PadView) -> Vec<(&'static str, String)> {
    let pad = &view.pad;
    let pressed: Vec<&str> = Button::ALL.iter().filter(|b| pad.is_pressed(**b)).map(|b| button_label(*b)).collect();
    let (dx, dy) = pad.dpad();
    vec![
        ("Battery", battery_or_unknown(&view.battery)),
        ("Pressed", if pressed.is_empty() { "None".to_string() } else { pressed.join(", ") }),
        ("Left Stick", format!("{}, {}", fmt_axis(pad.axis(Axis::LeftX)), fmt_axis(pad.axis(Axis::LeftY)))),
        ("Right Stick", format!("{}, {}", fmt_axis(pad.axis(Axis::RightX)), fmt_axis(pad.axis(Axis::RightY)))),
        ("Triggers", format!("Left {}, Right {}", fmt_axis(pad.axis(Axis::LeftTrigger)), fmt_axis(pad.axis(Axis::RightTrigger)))),
        ("Direction Pad", dpad_name(dx, dy).to_string()),
    ]
}

fn battery_or_unknown(b: &Battery) -> String {
    let text = battery_text(b);
    if text.is_empty() {
        "Not Available".to_string()
    } else {
        text
    }
}

/// The live rows of the controller `id`, for the Settings page to refresh; empty once it is gone.
pub fn live_rows_for(views: &[PadView], id: u32) -> Vec<(&'static str, String)> {
    views.iter().find(|v| v.id == id).map(live_rows).unwrap_or_default()
}

/// The page list: `Game Controllers` (every connected controller) and one controller's page.
pub fn spec(id: PageId, views: &[PadView]) -> Spec {
    match id {
        PageId::GameController(n) => controller_spec(n, views),
        _ => list_spec(views),
    }
}

fn list_spec(views: &[PadView]) -> Spec {
    let groups = if views.is_empty() {
        vec![Group::new(vec![Item::new("No Controllers")])
            .header("CONNECTED")
            .footer("Plug in a game controller or pair a Bluetooth one. It appears here, and apps that use controllers receive its buttons while they are in front.")]
    } else {
        vec![Group::new(views.iter().map(|v| Item::new(v.name.as_str()).value(if v.battery.percent().is_some() { battery_text(&v.battery) } else { String::new() }).link(PageId::GameController(v.id))).collect())
            .header("CONNECTED")
            .footer("Choose a controller to test its buttons and sticks and to change what its buttons send.")]
    };
    Spec { id: PageId::GameControllers, title: "Game Controllers".into(), large: false, groups }
}

fn controller_spec(id: u32, views: &[PadView]) -> Spec {
    let Some(view) = views.iter().find(|v| v.id == id) else {
        let groups = vec![Group::new(vec![Item::new("This controller is no longer connected")])];
        return Spec { id: PageId::GameController(id), title: "Game Controller".into(), large: false, groups };
    };
    let test: Vec<Item> = live_rows(view).into_iter().skip(1).map(|(label, value)| Item::new(label).value(value)).collect();
    let mut groups = vec![
        Group::new(vec![Item::new("Battery").value(battery_or_unknown(&view.battery))]),
        Group::new(test).header("TEST").footer("Shows the buttons and sticks as the controller sends them, before any remapping. Updates while this page is open."),
    ];
    let rows: Vec<Item> = Button::ALL
        .iter()
        .map(|b| Item::new(button_label(*b)).value(button_label(view.remap.target(*b))).action(Action::GameController(Edit::Cycle { pad: view.name.clone(), from: *b })))
        .collect();
    groups.push(
        Group::new(rows)
            .header("BUTTONS")
            .footer("Choose a button to change what it sends. If another button already sends that output, the two swap outputs."),
    );
    groups.push(Group::new(vec![Item::new("Restore Default Buttons").tint(Tint::Blue).action(Action::GameController(Edit::Reset { pad: view.name.clone() }))]));
    Spec { id: PageId::GameController(id), title: view.name.clone(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::gamepad::Gamepad;

    fn view(id: u32, name: &str, remap: Remap) -> PadView {
        PadView { id, name: name.into(), battery: Battery::from_percent(72, BatteryState::Discharging), pad: Gamepad::new(), remap }
    }

    #[test]
    fn a_cycle_moves_the_button_and_swaps_the_output_it_displaced() {
        let mut r = Remap::identity();
        // South sends East: East (which sent East) takes South's output, South
        r.cycle(Button::South);
        assert_eq!(r.target(Button::South), Button::East);
        assert_eq!(r.target(Button::East), Button::South);
        assert_eq!(r.target(Button::North), Button::North);
        let mut q = Remap::identity();
        q.assign(Button::South, Button::North);
        assert_eq!(q.target(Button::South), Button::North);
        assert_eq!(q.target(Button::North), Button::South);
        assert_eq!(q.target(Button::East), Button::East);
    }

    #[test]
    fn a_remap_stays_one_to_one_after_many_cycles() {
        let mut r = Remap::identity();
        for _ in 0..13 {
            r.cycle(Button::South);
        }
        // thirteen steps around the ring bring South back to its own output (the others are moved, and still one to one)
        assert_eq!(r.target(Button::South), Button::South);
        assert!(!r.is_identity());
        r.assign(Button::Home, Button::South);
        let mut outs: Vec<u8> = Button::ALL.iter().map(|b| r.target(*b) as u8).collect();
        outs.sort();
        outs.dedup();
        assert_eq!(outs.len(), 13);
    }

    #[test]
    fn the_text_form_round_trips_and_lists_only_changes() {
        let mut r = Remap::identity();
        r.assign(Button::South, Button::East);
        assert_eq!(r.to_text(), "south>east,east>south");
        assert_eq!(Remap::parse(&r.to_text()), r);
        assert_eq!(Remap::identity().to_text(), "");
        assert!(Remap::parse("").is_identity());
    }

    #[test]
    fn a_damaged_remap_text_is_ignored_or_partly_read() {
        // two buttons sending the same output is not a table: identity
        assert!(Remap::parse("south>east,west>east").is_identity());
        // unknown names are skipped; the valid pair is kept
        let r = Remap::parse("triangle>cross,south>east,east>south");
        assert_eq!(r.target(Button::South), Button::East);
        assert_eq!(r.target(Button::East), Button::South);
    }

    #[test]
    fn the_pref_key_is_the_name_with_safe_characters() {
        assert_eq!(pref_key("Test Pad"), "gamepad_remap.test_pad");
        assert_eq!(pref_key("Xbox/Wireless (2)"), "gamepad_remap.xbox_wireless__2_");
    }

    #[test]
    fn an_applied_edit_is_saved_and_read_back() {
        let mut prefs = Prefs::memory();
        let edit = Edit::Cycle { pad: "Test Pad".into(), from: Button::South };
        let r = apply(&mut prefs, &edit).unwrap();
        assert_eq!(r.target(Button::South), Button::East);
        assert_eq!(load(&prefs, "Test Pad").target(Button::South), Button::East);
        assert!(load(&prefs, "Other Pad").is_identity());
        apply(&mut prefs, &Edit::Reset { pad: "Test Pad".into() }).unwrap();
        assert!(load(&prefs, "Test Pad").is_identity());
    }

    #[test]
    fn battery_text_and_dpad_names() {
        assert_eq!(battery_text(&Battery::from_percent(72, BatteryState::Discharging)), "72%");
        assert_eq!(battery_text(&Battery::from_percent(40, BatteryState::Charging)), "40% (Charging)");
        assert_eq!(battery_text(&Battery::unknown()), "");
        assert_eq!(dpad_name(0, 0), "Centre");
        assert_eq!(dpad_name(1, -1), "Up Right");
        assert_eq!(dpad_name(-1, 1), "Down Left");
    }

    #[test]
    fn the_list_names_each_controller_with_its_battery() {
        let views = vec![view(1, "Test Pad", Remap::identity())];
        let s = spec(PageId::GameControllers, &views);
        assert_eq!(s.title, "Game Controllers");
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].items.len(), 1);
        assert_eq!(s.groups[0].items[0].label, "Test Pad");
        assert_eq!(s.groups[0].items[0].value, "72%");
        assert_eq!(s.groups[0].items[0].action, Action::Push(PageId::GameController(1)));
        let empty = spec(PageId::GameControllers, &[]);
        assert_eq!(empty.groups[0].items[0].label, "No Controllers");
    }

    #[test]
    fn the_detail_page_has_test_rows_and_one_row_per_button() {
        let views = vec![view(1, "Test Pad", Remap::identity())];
        let s = spec(PageId::GameController(1), &views);
        assert_eq!(s.title, "Test Pad");
        // battery, test, buttons, reset
        assert_eq!(s.groups.len(), 4);
        assert_eq!(s.groups[0].items[0].value, "72%");
        let test: Vec<&str> = s.groups[1].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(test, vec!["Pressed", "Left Stick", "Right Stick", "Triggers", "Direction Pad"]);
        assert_eq!(s.groups[1].items[0].value, "None");
        assert_eq!(s.groups[2].items.len(), 13);
        assert_eq!(s.groups[2].items[0].action, Action::GameController(Edit::Cycle { pad: "Test Pad".into(), from: Button::South }));
        assert_eq!(s.groups[3].items[0].label, "Restore Default Buttons");
    }

    #[test]
    fn a_remapped_button_shows_its_output() {
        let mut r = Remap::identity();
        r.assign(Button::South, Button::East);
        let views = vec![view(1, "Test Pad", r)];
        let s = spec(PageId::GameController(1), &views);
        assert_eq!(s.groups[2].items[0].value, "East");
        assert_eq!(s.groups[2].items[1].value, "South");
    }

    #[test]
    fn the_live_rows_show_pressed_buttons_and_centred_sticks() {
        let mut pad = Gamepad::new();
        pad.feed(0x01, 0x130, 1); // BTN_SOUTH down
        let v = PadView { id: 1, name: "Test Pad".into(), battery: Battery::unknown(), pad, remap: Remap::identity() };
        let rows = live_rows(&v);
        assert_eq!(rows[0], ("Battery", "Not Available".to_string()));
        assert_eq!(rows[1], ("Pressed", "South".to_string()));
        assert_eq!(rows[2], ("Left Stick", "0.00, 0.00".to_string()));
        assert_eq!(rows[5], ("Direction Pad", "Centre".to_string()));
        assert!(live_rows_for(&[], 1).is_empty());
    }
}
