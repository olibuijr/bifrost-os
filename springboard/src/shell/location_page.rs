//! Settings > Privacy & Security > Location Services. Three parts:
//! - the global switch (`location.services.enabled`): off, no app gets location, and an app that is watching it is told
//!   the location is unavailable on the next tick (`services::location::Location::updates`);
//! - System location: a place the person types as latitude and longitude (`location.manual.*`). Nothing is looked up on
//!   the network, so there is no geocoding, and the place has no city or country until one is typed in;
//! - one row group per app that has an answer in the privacy store: its access popup (Never, Ask Next Time, While
//!   Using the App), its Precise Location switch (`location.precise.<app>`) and whether it used location in the last
//!   24 hours. The last use is kept in memory only (`services::location::last_use_of`).
//!
//! The values are read once from the shell prefs into a [`Store`]. Each change returns the prefs pairs the shell saves
//! (`Effect::Location`), so the page and `settings.ini` stay the same.
use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::prefs::Prefs;
use super::services::location::{self, PREFS_PRECISE_PREFIX, PREFS_SERVICES};
use super::services::privacy_permissions::{self as privacy, LocationAccess};
use bifrost_ui::location_auth::AuthorizationStatus;
use std::collections::BTreeSet;

/// The prefs key of the System location switch.
pub const MANUAL_ENABLED: &str = "location.manual.enabled";
/// Seconds in the "Used in the last 24 hours" window.
pub const DAY: f64 = 86_400.0;

/// A prefs change: the (key, value) pairs the shell saves to `settings.ini`.
pub type Pairs = Vec<(String, String)>;

/// What the page reads from the prefs, and changes as the person edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    pub services_on: bool,
    /// The System location as (latitude, longitude), when one is set.
    pub manual: Option<(f64, f64)>,
    /// The apps whose Precise Location is on.
    pub precise: BTreeSet<String>,
}

fn flag(on: bool) -> String {
    String::from(if on { "1" } else { "0" })
}

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

impl Store {
    /// The page's values as the prefs hold them now.
    pub fn load(prefs: &Prefs) -> Store {
        let precise = prefs.entries().filter(|(_, v)| *v == "1").filter_map(|(k, _)| k.strip_prefix(PREFS_PRECISE_PREFIX)).map(String::from).collect();
        Store { services_on: location::services_on(prefs), manual: location::manual_fix(prefs, 0.0).map(|f| (f.latitude, f.longitude)), precise }
    }

    /// The global switch: the new value and the pref to save.
    pub fn set_services(&mut self, on: bool) -> Pairs {
        self.services_on = on;
        vec![pair(PREFS_SERVICES, &flag(on))]
    }

    /// One app's Precise Location switch.
    pub fn set_precise(&mut self, app_id: &str, on: bool) -> Pairs {
        if on {
            self.precise.insert(app_id.to_string());
        } else {
            self.precise.remove(app_id);
        }
        vec![pair(&format!("{PREFS_PRECISE_PREFIX}{app_id}"), &flag(on))]
    }

    /// Sets the System location. The place has no city or country (nothing is looked up).
    pub fn set_manual(&mut self, latitude: f64, longitude: f64) -> Pairs {
        self.manual = Some((latitude, longitude));
        vec![
            pair(MANUAL_ENABLED, "1"),
            pair("location.manual.latitude", &format!("{latitude:.6}")),
            pair("location.manual.longitude", &format!("{longitude:.6}")),
            pair("location.manual.city", ""),
            pair("location.manual.country", ""),
        ]
    }

    /// Clears the System location.
    pub fn clear_manual(&mut self) -> Pairs {
        self.manual = None;
        vec![pair(MANUAL_ENABLED, "0")]
    }
}

/// A coordinate typed into a form: a decimal number (a decimal comma is accepted) within `limit` degrees either way.
pub fn coordinate(text: &str, limit: f64) -> Option<f64> {
    let v: f64 = text.trim().replace(',', ".").parse().ok()?;
    (v.is_finite() && v.abs() <= limit).then_some(v)
}

/// A change the page asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Services(bool),
    /// opens the latitude prompt
    SetLocation,
    ClearLocation,
    Precise { app: String, on: bool },
    Access { app: String, choice: LocationAccess },
}

/// Whether a one-field prompt belongs to this page (its two Set Location steps).
pub fn owns(purpose: &Purpose) -> bool {
    matches!(purpose, Purpose::SetLatitude | Purpose::SetLongitude { .. })
}

/// What a finished prompt step does: ask the next one, or save the place.
#[derive(Clone, Debug, PartialEq)]
pub enum Flow {
    Next(Prompt),
    Save(Pairs),
}

/// One step of Set Location: latitude asks for the longitude, longitude saves the place. Out-of-range text is refused.
pub fn step(store: &mut Store, purpose: &Purpose, text: &str) -> Result<Flow, String> {
    match purpose {
        Purpose::SetLatitude => {
            let lat = coordinate(text, 90.0).ok_or("Latitude must be a number from -90 to 90.")?;
            Ok(Flow::Next(Prompt::set_longitude(&format!("{lat}"))))
        }
        Purpose::SetLongitude { latitude } => {
            let lat = coordinate(latitude, 90.0).ok_or("Latitude must be a number from -90 to 90.")?;
            let lon = coordinate(text, 180.0).ok_or("Longitude must be a number from -180 to 180.")?;
            Ok(Flow::Save(store.set_manual(lat, lon)))
        }
        _ => Err("not a Location Services prompt".to_string()),
    }
}

/// One app that has an answer for location, as the page lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRow {
    pub app_id: String,
    pub name: String,
    pub status: AuthorizationStatus,
    pub precise: bool,
    pub used_recently: bool,
}

/// The apps with a location answer, sorted by id. A name comes from the installed manifest, else the id.
pub fn app_rows(store: &Store, now: f64) -> Vec<AppRow> {
    privacy::location_asked()
        .into_iter()
        .map(|(app_id, status)| AppRow {
            name: super::apphost::installed().iter().find(|a| a.manifest.id == app_id).map_or_else(|| app_id.clone(), |a| a.manifest.name.clone()),
            precise: store.precise.contains(&app_id),
            used_recently: location::used_within(location::last_use_of(&app_id), now, DAY),
            status,
            app_id,
        })
        .collect()
}

/// An app's access row: its icon and a pop-up of the three choices, the current one selected.
fn access_row(r: &AppRow) -> Item {
    let current = LocationAccess::of(r.status);
    let choices = LocationAccess::ALL.iter().map(|c| c.label().to_string()).collect();
    let selected = LocationAccess::ALL.iter().position(|c| *c == current).unwrap_or(0);
    let item = Item::new(r.name.as_str())
        .app_icon(r.app_id.as_str())
        .popup(choices, selected, Action::Location(Edit::Access { app: r.app_id.clone(), choice: current }));
    // a restricted app's answer cannot change: its popup shows greyed out and does not open
    if r.status == AuthorizationStatus::Restricted {
        item.tint(Tint::Dim)
    } else {
        item
    }
}

/// The Location Services page. Each app's access is a pop-up on its row (Never, Ask Next Time, While Using the App).
pub fn spec(store: &Store, now: f64) -> Spec {
    main_page(store, now)
}

fn main_page(store: &Store, now: f64) -> Spec {
    let on = store.services_on;
    let services = Group::new(vec![Item::new("Location Services").switch(on, Action::Location(Edit::Services(!on)))]).footer(
        "Location Services lets the apps you allow use this computer's location. Turned off, no app gets it, and an app that was watching it is told the location is unavailable.",
    );

    let place = store.manual.map_or_else(|| "Not Set".to_string(), |(la, lo)| format!("{la:.4}, {lo:.4}"));
    let mut system = vec![Item::new("Set Location\u{2026}").value(place).action(Action::Location(Edit::SetLocation))];
    if store.manual.is_some() {
        system.push(Item::new("Clear").tint(Tint::Red).action(Action::Location(Edit::ClearLocation)));
    }
    let system = Group::new(system).header("SYSTEM LOCATION").footer(
        "The place this computer reports to apps that ask. You type it in: nothing looks it up on the network, and it is accurate to about 100 m.",
    );

    let rows = app_rows(store, now);
    let apps: Vec<Item> = if rows.is_empty() {
        vec![Item::new("No apps have asked to use location")]
    } else {
        rows.iter()
            .flat_map(|r| {
                vec![
                    access_row(r),
                    Item::new("Precise Location").switch(r.precise, Action::Location(Edit::Precise { app: r.app_id.clone(), on: !r.precise })),
                    Item::new("Used in the last 24 hours").check(r.used_recently, Action::None),
                ]
            })
            .collect()
    };
    let apps = Group::new(apps).header("APPS").footer(
        "Approximate location is the default: an app with Precise Location off gets your place rounded to a grid about 3 km wide. Precise Location gives its exact place.",
    );

    Spec { id: PageId::LocationServices, title: "Location Services".into(), large: false, groups: vec![services, system, apps] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::list::{Accessory, Popup};
    use super::super::pages;

    const NOW: f64 = 1_000_000.0;

    fn prefs_with(pairs: &[(&str, &str)]) -> Prefs {
        let mut p = Prefs::memory();
        p.set_many(pairs).unwrap();
        p
    }

    fn apply(pairs: &Pairs) -> Prefs {
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        prefs_with(&refs)
    }

    #[test]
    fn a_coordinate_must_be_a_number_within_its_limit() {
        assert_eq!(coordinate("65.68", 90.0), Some(65.68));
        assert_eq!(coordinate(" -18,10 ", 180.0), Some(-18.10));
        assert_eq!(coordinate("90", 90.0), Some(90.0));
        assert_eq!(coordinate("90.0001", 90.0), None);
        assert_eq!(coordinate("-180.5", 180.0), None);
        assert_eq!(coordinate("north", 90.0), None);
        assert_eq!(coordinate("", 90.0), None);
        assert_eq!(coordinate("NaN", 90.0), None);
    }

    #[test]
    fn the_form_refuses_an_out_of_range_latitude_and_asks_for_the_longitude_next() {
        let mut s = Store::default();
        assert!(!super::super::forms::acceptable(&Purpose::SetLatitude, "95"));
        assert!(step(&mut s, &Purpose::SetLatitude, "95").is_err());
        match step(&mut s, &Purpose::SetLatitude, "65.68") {
            Ok(Flow::Next(p)) => assert_eq!(p.purpose, Purpose::SetLongitude { latitude: "65.68".into() }),
            other => panic!("expected the longitude prompt, got {other:?}"),
        }
        assert!(s.manual.is_none(), "nothing is saved before the longitude");
    }

    #[test]
    fn the_longitude_step_saves_the_place_as_prefs_that_load_back() {
        let mut s = Store::default();
        let purpose = Purpose::SetLongitude { latitude: "65.68".into() };
        assert!(step(&mut s, &purpose, "181").is_err());
        let Ok(Flow::Save(pairs)) = step(&mut s, &purpose, "-18.1") else { panic!("the place is saved") };
        assert_eq!(s.manual, Some((65.68, -18.1)));
        let loaded = Store::load(&apply(&pairs));
        assert_eq!(loaded.manual, Some((65.68, -18.1)));
        assert!(location::manual_fix(&apply(&pairs), 0.0).is_some_and(|f| f.city.is_empty()), "no city: nothing is looked up");
    }

    #[test]
    fn clearing_the_system_location_writes_it_off() {
        let mut s = Store { manual: Some((1.0, 2.0)), ..Store::default() };
        let pairs = s.clear_manual();
        assert_eq!(pairs, vec![pair(MANUAL_ENABLED, "0")]);
        assert_eq!(Store::load(&apply(&pairs)).manual, None);
    }

    #[test]
    fn the_store_loads_its_switches_from_the_prefs() {
        let p = prefs_with(&[("location.precise.is.example.a", "1"), ("location.precise.is.example.b", "0"), ("location.services.enabled", "0")]);
        let s = Store::load(&p);
        assert!(!s.services_on);
        assert!(s.precise.contains("is.example.a") && !s.precise.contains("is.example.b"));
        assert_eq!(s.manual, None);
        assert!(Store::load(&Prefs::memory()).services_on, "Location Services is on by default");
    }

    #[test]
    fn the_switch_writes_its_pref_and_the_precise_switch_its_own_key() {
        let mut s = Store::default();
        assert_eq!(s.set_services(false), vec![pair(PREFS_SERVICES, "0")]);
        assert!(!s.services_on);
        assert_eq!(s.set_precise("is.example.a", true), vec![pair("location.precise.is.example.a", "1")]);
        assert!(s.precise.contains("is.example.a"));
        assert_eq!(s.set_precise("is.example.a", false), vec![pair("location.precise.is.example.a", "0")]);
        assert!(!s.precise.contains("is.example.a"));
    }

    #[test]
    fn last_use_counts_only_inside_24_hours() {
        assert!(location::used_within(Some(NOW - 3600.0), NOW, DAY));
        assert!(!location::used_within(Some(NOW - 2.0 * DAY), NOW, DAY));
        assert!(!location::used_within(None, NOW, DAY));
        assert!(!location::used_within(Some(NOW + 10.0), NOW, DAY), "a fix in the future does not count");
    }

    #[test]
    fn the_popup_choice_maps_to_the_grant_and_back() {
        use AuthorizationStatus as A;
        assert_eq!(LocationAccess::of(A::NotDetermined), LocationAccess::AskNextTime);
        assert_eq!(LocationAccess::of(A::Denied), LocationAccess::Never);
        assert_eq!(LocationAccess::of(A::AuthorizedWhenInUse), LocationAccess::WhileUsing);
        assert_eq!(LocationAccess::of(A::AuthorizedAlways), LocationAccess::WhileUsing);
        assert_eq!(privacy::access_label_of(A::Restricted), "Restricted");

        let id = "is.example.loc-page-popup";
        assert_eq!(privacy::set_location_access(id, LocationAccess::WhileUsing), A::AuthorizedWhenInUse);
        assert_eq!(privacy::set_location_access(id, LocationAccess::Never), A::Denied);
        assert_eq!(privacy::set_location_access(id, LocationAccess::AskNextTime), A::NotDetermined);
        assert!(!privacy::location_asked().iter().any(|(a, _)| a == id), "Ask Next Time forgets the answer");
    }

    #[test]
    fn the_app_rows_come_from_the_privacy_store_with_precise_and_recent_use() {
        let (a, b) = ("is.example.loc-page-a", "is.example.loc-page-b");
        privacy::set_location_access(a, LocationAccess::WhileUsing);
        privacy::set_location_access(b, LocationAccess::Never);
        location::record_use(a, NOW - 60.0);
        location::record_use(b, NOW - 3.0 * DAY);
        let store = Store { precise: BTreeSet::from([a.to_string()]), ..Store::default() };
        let rows = app_rows(&store, NOW);
        let ra = rows.iter().find(|r| r.app_id == a).expect("a has asked");
        let rb = rows.iter().find(|r| r.app_id == b).expect("b has asked");
        assert_eq!((ra.status, ra.precise, ra.used_recently), (AuthorizationStatus::AuthorizedWhenInUse, true, true));
        assert_eq!((rb.status, rb.precise, rb.used_recently), (AuthorizationStatus::Denied, false, false));
        assert_eq!(ra.name, a, "an app that is not installed is named by its id");
    }

    #[test]
    fn the_main_page_lists_the_switches_the_system_location_and_each_app() {
        let (a, b) = ("is.example.loc-spec-a", "is.example.loc-spec-b");
        privacy::set_location_access(a, LocationAccess::WhileUsing);
        privacy::set_location_access(b, LocationAccess::Never);
        location::record_use(a, NOW - 10.0);
        let store = Store { services_on: true, manual: Some((65.68, -18.1)), precise: BTreeSet::from([a.to_string()]) };
        let s = spec(&store, NOW);
        assert_eq!(s.groups.len(), 3);
        assert_eq!(s.groups[0].items[0].accessory, Accessory::Switch(true));
        assert_eq!(s.groups[1].items[0].value, "65.6800, -18.1000");
        assert_eq!(s.groups[1].items.len(), 2, "Set Location and Clear once a place is set");
        assert_eq!(s.groups[1].items[1].action, Action::Location(Edit::ClearLocation));

        let apps = &s.groups[2].items;
        assert_eq!(apps.len(), 6, "three rows per app");
        let pa = apps.iter().position(|i| i.label == a).expect("row for a");
        assert_eq!(apps[pa].accessory, Accessory::Popup, "the access is a pop-up on the row");
        assert_eq!(apps[pa].app_icon.as_deref(), Some(a));
        assert_eq!(apps[pa].popup, Some(Popup { choices: vec!["Never".into(), "Ask Next Time".into(), "While Using the App".into()], selected: 2 }));
        assert_eq!(apps[pa].action, Action::Location(Edit::Access { app: a.into(), choice: LocationAccess::WhileUsing }));
        assert_eq!(apps[pa + 1].accessory, Accessory::Switch(true));
        assert_eq!(apps[pa + 1].action, Action::Location(Edit::Precise { app: a.into(), on: false }));
        assert_eq!(apps[pa + 2].accessory, Accessory::Check(true), "used in the last 24 hours");
        let pb = apps.iter().position(|i| i.label == b).expect("row for b");
        assert_eq!(apps[pb].popup.as_ref().map(|p| p.selected), Some(0), "Never is the first choice");
        assert_eq!(apps[pb].tint, Tint::Plain);
        assert_eq!(apps[pb + 2].accessory, Accessory::Check(false));
    }

    #[test]
    fn the_main_page_with_the_switch_off_offers_to_turn_it_on_and_no_clear_without_a_place() {
        let s = spec(&Store::default(), NOW);
        assert_eq!(s.groups[0].items[0].accessory, Accessory::Switch(true));
        assert_eq!(s.groups[0].items[0].action, Action::Location(Edit::Services(false)));
        assert_eq!(s.groups[1].items[0].value, "Not Set");
        assert_eq!(s.groups[1].items.len(), 1);
        assert_eq!(s.groups[2].items[0].label, "No apps have asked to use location");
    }

    #[test]
    fn the_access_popup_sends_the_row_action_with_the_chosen_index() {
        let a = "is.example.loc-page-popup-map";
        privacy::set_location_access(a, LocationAccess::AskNextTime);
        let s = spec(&Store::default(), NOW);
        let row = s.groups[2].items.iter().find(|i| i.label == a).cloned();
        let row = row.expect("a has asked, so it has a row");
        assert_eq!(row.popup.as_ref().map(|p| p.selected), Some(1));
        let edit = |choice| Action::Location(Edit::Access { app: a.into(), choice });
        assert_eq!(pages::chosen(&row.action, 0), Some(edit(LocationAccess::Never)));
        assert_eq!(pages::chosen(&row.action, 2), Some(edit(LocationAccess::WhileUsing)));
        assert_eq!(pages::chosen(&row.action, 3), None, "no fourth choice");
        assert_eq!(pages::chosen(&Action::None, 0), None, "only location rows map a choice");
    }

    #[test]
    fn a_restricted_app_shows_its_popup_greyed_out_and_does_not_open() {
        let row = |status| AppRow { app_id: "is.example.restricted".into(), name: "Restricted".into(), status, precise: false, used_recently: false };
        let restricted = access_row(&row(AuthorizationStatus::Restricted));
        assert_eq!(restricted.tint, Tint::Dim);
        assert!(!restricted.tappable(), "a dimmed popup does not open");
        let open = access_row(&row(AuthorizationStatus::AuthorizedWhenInUse));
        assert_eq!(open.tint, Tint::Plain);
        assert!(open.tappable());
    }

    #[test]
    fn only_the_set_location_prompts_belong_to_this_page() {
        assert!(owns(&Purpose::SetLatitude));
        assert!(owns(&Purpose::SetLongitude { latitude: "1".into() }));
        assert!(!owns(&Purpose::NewGroup));
    }
}
