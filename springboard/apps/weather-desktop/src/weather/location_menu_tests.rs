//! Tests of the Location menu, the sidebar's place moves and the delete path, on a real `Weather` with no network.

use super::*;

/// A network that is always down, so the tests never fetch.
struct Offline;

impl Fetch for Offline {
    fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "offline"))
    }
}

fn test_ui() -> Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

/// An app whose saved places are `names` (in that order), with `selected` as the selection.
fn app_with(tag: &str, names: &[&str], selected: usize) -> (Weather, PathBuf) {
    let dir = std::env::temp_dir().join(format!("weather-menu-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::new(dir.clone());
    let places: Vec<Location> = names
        .iter()
        .enumerate()
        .map(|(i, n)| Location { name: (*n).into(), region: "Iceland".into(), lat: 64.0 + i as f64, lon: -21.0 })
        .collect();
    store.save_locations(&places);
    store.save_settings(Units::Celsius, selected);
    let app = Weather::new(&test_ui(), 1200, 780, dir.clone(), Arc::new(Offline));
    (app, dir)
}

fn item(app: &Weather, id: u32) -> AppMenuItem {
    app.menu().into_iter().find(|i| i.id == id).expect("the Location menu has the item")
}

#[test]
fn the_menu_enables_moves_only_where_the_selection_can_go() {
    let (app, dir) = app_with("enable", &["Akureyri", "Reykjavik", "Hafnarfjordur"], 0);
    assert!(!item(&app, MENU_MOVE_UP).enabled);
    assert!(item(&app, MENU_MOVE_DOWN).enabled);
    assert!(item(&app, MENU_DELETE).enabled);
    assert!(item(&app, MENU_CELSIUS).checked);
    assert!(!item(&app, MENU_FAHRENHEIT).checked);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_single_place_cannot_be_deleted_from_the_menu() {
    let (mut app, dir) = app_with("single", &["Reykjavik"], 0);
    assert!(!item(&app, MENU_DELETE).enabled);
    assert!(app.menu_action(&test_ui(), MENU_DELETE, 0.0), "the item is ours, so it is handled");
    assert_eq!(app.places.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moving_the_selected_place_down_keeps_it_selected() {
    let (mut app, dir) = app_with("down", &["Akureyri", "Reykjavik", "Hafnarfjordur"], 0);
    let ui = test_ui();
    assert!(app.menu_action(&ui, MENU_MOVE_DOWN, 0.0));
    let names: Vec<&str> = app.places.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Reykjavik", "Akureyri", "Hafnarfjordur"]);
    assert_eq!(app.selected, 1);
    assert!(app.describe().contains("weather_place=Akureyri\n"), "{}", app.describe());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moving_up_past_the_first_place_changes_nothing() {
    let (mut app, dir) = app_with("up", &["Akureyri", "Reykjavik"], 0);
    assert!(!item(&app, MENU_MOVE_UP).enabled);
    app.menu_action(&test_ui(), MENU_MOVE_UP, 0.0);
    assert_eq!(app.places[0].name, "Akureyri");
    assert_eq!(app.selected, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_removes_the_selected_place() {
    let (mut app, dir) = app_with("delete", &["Akureyri", "Reykjavik", "Hafnarfjordur"], 1);
    assert!(app.menu_action(&test_ui(), MENU_DELETE, 0.0));
    let names: Vec<&str> = app.places.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Akureyri", "Hafnarfjordur"]);
    assert!(app.describe().contains("weather_place=Hafnarfjordur\n"), "{}", app.describe());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_menu_id_is_not_ours() {
    let (mut app, dir) = app_with("unknown", &["Reykjavik"], 0);
    assert!(!app.menu_action(&test_ui(), 999, 0.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_order_is_saved_with_the_places() {
    let (mut app, dir) = app_with("persist", &["Akureyri", "Reykjavik"], 0);
    app.menu_action(&test_ui(), MENU_MOVE_DOWN, 0.0);
    let saved: Vec<String> = Store::new(dir.clone()).load_locations().into_iter().map(|l| l.name).collect();
    assert_eq!(saved, vec!["Reykjavik".to_string(), "Akureyri".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
