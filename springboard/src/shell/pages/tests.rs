use super::super::desktop_dock::Preferences as DockPrefs;
use super::*;

#[test]
fn the_icon_style_footer_sits_under_the_icon_style_row() {
    let (info, opts) = (info(), Options::default());
    let s = appearance_page(&model(&info, &opts), &DeskPrefs::default());
    let g = s.groups.iter().find(|g| g.footer.as_deref().is_some_and(|f| f.starts_with("Dark draws app icons"))).expect("the icon style footer");
    assert_eq!(g.items.last().map(|i| i.label.as_str()), Some("Icon & widget style"), "the footer explains the row right above it");
    assert!(!g.items.iter().any(|i| i.label == "Show scroll bars"));
}

#[test]
fn a_desktop_without_a_battery_shows_none_not_a_made_up_level() {
    let mut info = Info { battery: 100, ..Info::default() };
    assert_eq!(battery_level(&info), "100%", "a bifrost keeps its level");
    info.desktop_network = Some(Default::default());
    info.battery_present = false;
    assert_eq!(battery_level(&info), "None");
    info.battery_present = true;
    info.battery = 64;
    assert_eq!(battery_level(&info), "64%", "a desktop with a battery shows it");
}
use crate::shell::{Network, SavedNetwork};

#[test]
fn the_speech_voice_row_names_the_saved_voice_and_falls_back_to_the_default() {
    assert_eq!(speech_voice_index(""), 0, "the engine's default voice");
    assert_eq!(super::super::accessibility::SPEECH_VOICES[speech_voice_index("is")].0, "Icelandic");
    assert_eq!(speech_voice_index("no-such-voice"), 0, "an unknown saved voice shows as the default");
}

fn info() -> Info {
    Info {
        battery: 63,
        charging: true,
        wifi: "WIFI-5G".into(),
        wifi_on: true,
        current_ssid: Some("WIFI-5G".into()),
        networks: vec![Network { ssid: "WIFI-5G".into(), bars: 3, secure: true, saved: true }, Network { ssid: "Cafe".into(), bars: 1, secure: false, saved: false }],
        ip: "192.168.1.40".into(),
        router: "192.168.1.1".into(),
        mac: "AC:E4:B5:35:20:F0".into(),
        model: "Midget".into(),
        kernel: "4.14.186".into(),
        memory: "3.6 GB".into(),
        cpu: "Unisoc T618, 8 cores".into(),
        storage_total: 32_000_000_000,
        storage_free: 17_900_000_000,
        uptime_min: 1_500,
        name: "Midget".into(),
        serial: "R9XR40ABC".into(),
        timezone: "Atlantic/Reykjavik".into(),
        hour: 14,
        min: 5,
        ..Info::default()
    }
}

#[test]
fn fonts_pages_show_each_role_and_offer_the_library_by_kind() {
    let (info, opts) = (info(), Options::default());
    let lib = |family: &str, path: &str, mono: bool| Installed { family: family.into(), path: path.into(), mono, variable: true };
    let fonts = FontsView {
        defaults: ["Inter".into(), "JetBrains Mono".into()],
        overrides: [None, Some("/lib/GeistMono.ttf".into())],
        names: [None, Some("Geist Mono".into())],
        library: vec![lib("Geist", "/lib/Geist.ttf", false), lib("Geist Mono", "/lib/GeistMono.ttf", true)],
    };
    let m = Model { fonts: &fonts, ..model(&info, &opts) };
    let root = spec(PageId::Fonts, &m);
    let rows: Vec<_> = root.groups[0].items.iter().map(|i| (i.label.as_str(), i.action.clone())).collect();
    assert_eq!(rows, [("System Font", Action::Push(PageId::FontPick(FontRole::System))), ("Monospaced Font", Action::Push(PageId::FontPick(FontRole::Mono)))]);
    assert_eq!(root.groups[0].items[0].value, "Inter", "no override: the default's family");
    assert_eq!(root.groups[1].items[0].action, Action::Run(Command::Restart), "overrides apply after a restart");

    let system = spec(PageId::FontPick(FontRole::System), &m);
    assert_eq!(system.groups[0].items[0].label, "Inter (Default)");
    assert_eq!(system.groups[0].items[0].accessory, Accessory::Check(true));
    let offered: Vec<_> = system.groups[1].items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(offered, ["Geist"], "proportional fonts only");
    assert_eq!(system.groups[1].items[0].action, Action::Font(FontRole::System, Some("/lib/Geist.ttf".into())));

    let mono = spec(PageId::FontPick(FontRole::Mono), &m);
    assert_eq!(mono.groups[0].items[0].accessory, Accessory::Check(false));
    assert_eq!((mono.groups[1].items[0].label.as_str(), &mono.groups[1].items[0].accessory), ("Geist Mono", &Accessory::Check(true)));
    assert_eq!(mono.groups[0].items[0].action, Action::Font(FontRole::Mono, None), "Default clears the override");
}

// a static: the settings hold a Vec, so a const would be a temporary here
static A11Y_DEFAULT: AccessibilitySettings = AccessibilitySettings::DEFAULT;

fn model<'a>(info: &'a Info, opts: &'a Options) -> Model<'a> {
    Model { info, opts, a11y: &A11Y_DEFAULT, airplane: false, haptics: false, wifi_on: true, brightness: 0.5, wallpaper: "Lake Day".into(), detail: "WIFI-5G", now: 1_000_000, update_badge: false, dock: DockPrefs::default(), defaults: &[], login_items: &[], login_pending: &[], firewall: &[], ext: &crate::shell::login_items_page::NO_EXTENSIONS, fonts: &NO_FONTS, refresh: RefreshChoice::Max, resolution: None, sound: &crate::shell::sound_page::NO_SOUND, sharing: None, printers: None, users: &NO_USERS, account: "" }
}

/// The row labelled `label` anywhere on the spec.
fn popup_row<'a>(spec: &'a Spec, label: &str) -> &'a Item {
    spec.groups.iter().flat_map(|g| &g.items).find(|i| i.label == label).unwrap_or_else(|| panic!("no row {label:?}"))
}

#[test]
fn value_choices_are_pop_up_rows_that_send_the_same_picks() {
    let (info, opts) = (info(), Options::default());
    let m = model(&info, &opts);
    // Desktop & Dock: position and minimise effect
    let dock = desktop_dock_page(&m, &DeskPrefs::default());
    let position = popup_row(&dock, "Position on Screen");
    assert_eq!(position.accessory, Accessory::Popup);
    assert_eq!(position.popup.as_ref().map(|p| (p.choices.clone(), p.selected)), Some((vec!["Bottom".to_string(), "Left".into(), "Right".into()], 0)));
    assert_eq!(chosen(&position.action, 2), Some(Action::Pick(Pick::DockPosition(2))), "the menu's choice replaces the index");
    let minimize = popup_row(&dock, "Minimize Windows Using");
    assert_eq!(minimize.popup.as_ref().map(|p| p.choices.len()), Some(MINIMIZE_EFFECTS.len()));
    assert_eq!(chosen(&minimize.action, 1), Some(Action::Pick(Pick::MinimizeEffect(1))));
    assert!(dock.groups.iter().all(|g| g.items.iter().all(|i| !matches!(i.accessory, Accessory::Check(_)))), "no checklist left on the page");
    // General: Game Mode
    let general = spec(PageId::General, &m);
    let game = popup_row(&general, "Game Mode");
    assert_eq!(game.accessory, Accessory::Popup);
    assert_eq!(chosen(&game.action, 1), Some(Action::Pick(Pick::GameMode(1))));
    // Trackpad: secondary click
    let trackpad = spec(PageId::Trackpad, &m);
    let secondary = popup_row(&trackpad, "Secondary Click");
    assert_eq!(secondary.popup.as_ref().map(|p| p.choices.len()), Some(TRACKPAD_SECONDARY.len()));
    assert_eq!(chosen(&secondary.action, 1), Some(Action::Pick(Pick::TrackpadSecondary(1))));
    // Screen Saver: kind and Start After; a choice option of the saver keeps its option number
    let saver = screen_saver_page(&opts, &Default::default());
    assert_eq!(popup_row(&saver, "Screen Saver").popup.as_ref().map(|p| p.choices.clone()), Some(super::super::screensaver::SAVERS.iter().map(|s| s.to_string()).collect()));
    let start = popup_row(&saver, "Start After");
    assert_eq!(chosen(&start.action, 3), Some(Action::Pick(Pick::ScreenSaverStart(3))));
    assert_eq!(chosen(&Action::Pick(Pick::ScreenSaverOption(2, 0)), 1), Some(Action::Pick(Pick::ScreenSaverOption(2, 1))));
    // a pick that is not a value choice has no menu
    assert_eq!(chosen(&Action::Pick(Pick::ScreenSaverPreview), 1), None);
    assert_eq!(chosen(&Action::Pick(Pick::Remap(0, 0)), 1), None);
}

#[test]
fn the_screen_time_day_and_week_switch_is_a_segmented_control() {
    use super::super::screentime::page::StEdit;
    let row = Item::new("Range").segmented(vec!["Day".into(), "Week".into()], 1, Action::ScreenTime(StEdit::Range(true)));
    assert_eq!(chosen(&row.action, 0), Some(Action::ScreenTime(StEdit::Range(false))));
    assert_eq!(chosen(&row.action, 1), Some(Action::ScreenTime(StEdit::Range(true))));
    assert_eq!(chosen(&row.action, 2), None);
}

#[test]
fn app_permissions_lists_each_media_type_with_one_switch_per_app() {
    use bifrost_ui::capture_access::CaptureAccess;
    use bifrost_ui::location_auth::AuthorizationStatus;
    let rows = vec![Row { app_id: "is.example.a".into(), name: "Alpha".into(), access: CaptureAccess::default(), location: AuthorizationStatus::AuthorizedWhenInUse, contacts: false, calendars: true, usb: 2, accessibility_asked: true, accessibility: false }];
    let s = permissions_page(&rows);
    assert_eq!(s.groups.len(), 8, "camera, microphone, screen recording, location, contacts, devices, calendars, accessibility");
    assert_eq!(s.groups[7].header.as_deref(), Some("ACCESSIBILITY"));
    assert_eq!(s.groups[7].items[0].accessory, Accessory::Switch(false), "a manifest alone does not turn control on");
    assert_eq!(s.groups[7].items[0].action, Action::Toggle(Toggle::Accessibility(0)));
    assert_eq!(permissions_page(&[]).groups[7].items[0].label, "No apps ask to control this computer");
    assert_eq!(s.groups[6].header.as_deref(), Some("CALENDARS"));
    assert_eq!(s.groups[6].items[0].accessory, Accessory::Switch(true), "an app the person let use the calendar is on");
    assert_eq!(s.groups[6].items[0].action, Action::Toggle(Toggle::Calendars(0)));
    assert_eq!(s.groups[5].header.as_deref(), Some("USB AND HID DEVICES"));
    assert_eq!(s.groups[5].items[0].action, Action::Toggle(Toggle::Usb(0)), "an app that holds device grants is listed");
    assert_eq!(permissions_page(&[]).groups[5].items[0].label, "No apps use USB or HID devices");
    assert_eq!(s.groups[3].header.as_deref(), Some("LOCATION"));
    assert_eq!(s.groups[3].items[0].accessory, Accessory::Switch(true), "an app the person allowed while in use is on");
    assert_eq!(s.groups[3].items[0].action, Action::Toggle(Toggle::Location(0)));
    assert_eq!(s.groups[0].header.as_deref(), Some("CAMERA"));
    let row = &s.groups[0].items[0];
    assert_eq!(row.label, "Alpha");
    assert_eq!(row.accessory, Accessory::Switch(false), "an app that has not asked is off");
    assert_eq!(row.action, Action::Toggle(Toggle::Permission(Media::Video, 0)));
    assert_eq!(s.groups[2].items[0].action, Action::Toggle(Toggle::Permission(Media::Screen, 0)));
    let empty = permissions_page(&[]);
    assert_eq!(empty.groups[1].items[0].label, "No apps installed");
    assert_eq!(empty.groups[1].items[0].action, Action::None);
}

#[test]
fn the_settings_map_is_generated_from_the_page_specs() {
    let (info, opts) = (info(), Options::default());
    let mut map = bifrost_ui::ax::Map::new();
    crate::shell::axmap::settings(&mut map, &|id| spec(id, &model(&info, &opts)));
    assert!(map.len() > 100, "{} elements", map.len());
    let su = map.get("os/settings/general/software-update").expect("General > Software Update is addressable");
    assert_eq!((su.role.as_str(), su.handle.as_str()), ("page", "General>Software Update"));
    assert!(map.get("os/settings/general").is_some_and(|e| e.role == "page"));
    let mut addresses: Vec<_> = map.elements().iter().map(|e| e.address.as_str()).collect();
    let n = addresses.len();
    addresses.sort();
    addresses.dedup();
    assert_eq!(addresses.len(), n, "addresses are unique");
    assert!(map.elements().iter().any(|e| e.role == "toggle" && (e.value == "on" || e.value == "off")), "switches carry their state");
    eprintln!("{}", map.listing("os/settings/general").lines().take(14).collect::<Vec<_>>().join("\n"));
}

#[test]
fn about_values_are_formatted_like_mobile() {
    assert_eq!(gb(117_870_000_000), "118 GB");
    assert_eq!(gb(17_900_000_000), "17.9 GB");
    assert_eq!(gb(0), "0.0 GB");
    assert_eq!(uptime(7), "7m");
    assert_eq!(uptime(127), "2h 7m");
    assert_eq!(uptime(1_500), "1d 1h 0m");
    let a = about(&info());
    assert_eq!(a[0][0], ("Name", "Midget".into()));
    assert_eq!(a[0][2], ("Model", "Midget".into()));
    assert_eq!(a[1][0], ("Capacity", "32.0 GB".into()));
    assert_eq!(a[2][0], ("Wi-Fi Address", "AC:E4:B5:35:20:F0".into()));
    assert_eq!(a[2][3], ("Uptime", "1d 1h 0m".into()));
    let blank = about(&Info::default());
    assert_eq!(blank[0][0].1, "—", "missing values show a dash");
    assert_eq!(blank[2][1].1, "—");
}

#[test]
fn the_menu_bar_page_has_one_switch_per_control_and_follows_the_saved_choice() {
    use crate::shell::options::MenuExtra;
    let info = info();
    let mut opts = Options::default();
    let s = spec(PageId::MenuBar, &model(&info, &opts));
    let labels: Vec<&str> = s.groups[0].items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["CPU Usage", "Memory Usage", "Battery", "Wi-Fi and Network", "Control Center"]);
    assert!(s.groups[0].items.iter().all(|i| i.accessory == Accessory::Switch(true)), "every control shows by default");
    assert_eq!(s.groups[0].items[2].action, Action::Toggle(Toggle::MenuExtra(MenuExtra::Battery)));
    opts.menu_extras = MenuExtra::Battery.bit() | MenuExtra::ControlCenter.bit();
    let s = spec(PageId::MenuBar, &model(&info, &opts));
    let on: Vec<bool> = s.groups[0].items.iter().map(|i| i.accessory == Accessory::Switch(true)).collect();
    assert_eq!(on, [false, false, true, false, true]);
}

#[test]
fn the_control_centre_page_lists_the_controls_it_holds() {
    let (info, opts) = (info(), Options::default());
    let s = spec(PageId::ControlCentre, &model(&info, &opts));
    let labels: Vec<&str> = s.groups[0].items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Brightness", "Volume", "Wi-Fi", "Screen Lock", "Focus"]);
    assert_eq!(s.groups[0].items[4].value, "Off");
}

#[test]
fn the_vpn_page_lists_the_held_connections_and_a_tap_flips_one() {
    use crate::shell::desktop_network::{Snapshot, VpnConnection};
    let (mut info, opts) = (info(), Options::default());
    let empty = spec(PageId::Vpn, &model(&info, &opts));
    assert_eq!(empty.groups[0].items[0].label, "No VPN Configurations");
    info.desktop_network = Some(Snapshot {
        vpns: vec![
            VpnConnection { name: "Office".into(), kind: "vpn".into(), active: false },
            VpnConnection { name: "wg0".into(), kind: "wireguard".into(), active: true },
        ],
        ..Snapshot::default()
    });
    let s = spec(PageId::Vpn, &model(&info, &opts));
    let items = &s.groups[0].items;
    assert_eq!((items[0].label.as_str(), items[0].value.as_str()), ("Office", "Not Connected"));
    assert_eq!((items[1].label.as_str(), items[1].value.as_str()), ("wg0", "Connected"));
    assert_eq!(items[0].action, Action::Vpn { name: "Office".into(), up: true }, "a tap on a disconnected VPN connects it");
    assert_eq!(items[1].action, Action::Vpn { name: "wg0".into(), up: false }, "a tap on a connected one disconnects it");
}

#[test]
fn root_lists_the_bifrost_sections_with_live_values() {
    let (info, opts) = (info(), Options::default());
    let s = spec(PageId::Root, &model(&info, &opts));
    assert!(s.large);
    let labels: Vec<&str> = s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.as_str())).collect();
    assert_eq!(labels[..4], ["BifrOSt Account", "Airplane Mode", "Wi-Fi", "Bluetooth"]);
    assert_eq!(labels[4..8], ["Notifications", "Sounds", "Focus", "Screen Time"]);
    assert_eq!(labels[8..], ["General", "Control Center", "Menu Bar", "Display & Brightness", "Home Screen & Dock", "Overview", "Sharing", "Print & Scan", "Keyboard Shortcuts", "Assistant", "Trackpad", "Mouse", "Users & Groups", "Accessibility", "Wallpaper", "Screen Saver", "Battery", "Privacy", "Search", "Lock Screen", "Passcode", "Passwords", "Internet Accounts", "Developer"]);
    assert_eq!(s.groups[1].items[1].value, "WIFI-5G");
    // Sharing, Trackpad, Mouse and Users & Groups sit before Wallpaper
    assert_eq!(s.groups[3].items[14].value, "Lake Day");
    assert_eq!(s.groups[3].items[15].value, "Never", "the screen saver is off by default");
    assert_eq!(s.groups[3].items[16].value, "63%");
    assert_eq!(s.groups[1].items[0].accessory, Accessory::Switch(false));
    assert_eq!(s.groups[2].items[0].action, Action::Push(PageId::Notifications));
    let off = Model { wifi_on: false, airplane: true, ..model(&info, &opts) };
    let s = spec(PageId::Root, &off);
    assert_eq!(s.groups[1].items[1].value, "Off");
    assert_eq!(s.groups[1].items[0].accessory, Accessory::Switch(true));
}

fn desktop_info(battery: bool, wifi: bool) -> Info {
    let mut info = Info { desktop_network: Some(Default::default()), battery_present: battery, ..Info::default() };
    if let Some(n) = info.desktop_network.as_mut() {
        n.wifi_available = wifi;
    }
    info
}

fn root_labels(info: &Info, form: bifrost_ui::form_factor::FormFactor) -> Vec<String> {
    set_test_form(form);
    let opts = Options::default();
    let s = spec(PageId::Root, &model(info, &opts));
    set_test_form(bifrost_ui::form_factor::FormFactor::Tablet);
    s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.clone())).collect()
}

#[test]
fn a_desktop_without_a_battery_has_no_battery_pane_but_offers_energy() {
    use bifrost_ui::form_factor::FormFactor::Desktop;
    let labels = root_labels(&desktop_info(false, false), Desktop);
    assert!(!labels.iter().any(|l| l == "Battery"), "no Battery pane: {labels:?}");
    assert!(labels.iter().any(|l| l == "Energy"), "sleep and wake settings stay, as Energy");
    assert!(root_labels(&desktop_info(true, false), Desktop).iter().any(|l| l == "Battery"), "a desktop with a battery keeps it");
    assert!(!root_labels(&desktop_info(true, false), Desktop).iter().any(|l| l == "Energy"));
    assert!(has_battery(&Info::default()), "a machine that is not a desktop always has the pane");
    // the power modes moved with it: Energy carries Low Power Mode and the Power Mode pop-up
    let (info, opts) = (desktop_info(false, false), Options::default());
    let energy = spec(PageId::Energy, &model(&info, &opts));
    let items: Vec<&str> = energy.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.as_str())).collect();
    assert!(items.contains(&"Low Power Mode"), "Energy has Low Power Mode: {items:?}");
    let mode = popup_row(&energy, "Power Mode");
    assert_eq!(mode.accessory, Accessory::Popup, "the power modes are one pop-up row");
    assert_eq!(mode.popup.as_ref().map(|p| p.choices.clone()), Some(vec!["Automatic".to_string(), "Performance".into(), "Balanced".into(), "Battery Saver".into()]));
    assert_eq!(chosen(&mode.action, 3), Some(Action::Pick(Pick::PowerMode(3))), "the menu's choice replaces the index");
    // a computer with a battery keeps them in Battery, not in Energy
    let with = spec(PageId::Energy, &model(&desktop_info(true, false), &opts));
    assert!(!with.groups.iter().any(|g| g.items.iter().any(|i| i.label == "Low Power Mode")));
    // and its menu bar has no Battery control to switch
    let bar = spec(PageId::MenuBar, &model(&info, &opts));
    assert!(!bar.groups[0].items.iter().any(|i| i.label == "Battery"), "no Battery menu extra without a battery");
}

#[test]
fn wifi_has_its_own_row_above_network_when_the_computer_has_a_wireless_adapter() {
    use bifrost_ui::form_factor::FormFactor::Desktop;
    let labels = root_labels(&desktop_info(false, true), Desktop);
    let at = |name: &str| labels.iter().position(|l| l == name).unwrap_or(usize::MAX);
    assert_eq!(at("Wi-Fi") + 1, at("Network"), "Wi-Fi sits right above Network: {labels:?}");
    assert_eq!(labels.iter().filter(|l| *l == "Wi-Fi").count(), 1);
    assert!(!root_labels(&desktop_info(false, false), Desktop).iter().any(|l| l == "Wi-Fi"), "no adapter, no Wi-Fi row");
}

#[test]
fn the_numeric_passcode_is_a_tablet_pane_and_a_desktop_has_a_login_password() {
    use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
    assert!(!root_labels(&desktop_info(false, false), Desktop).iter().any(|l| l == "Passcode"));
    assert!(root_labels(&desktop_info(false, false), Tablet).iter().any(|l| l == "Passcode"), "a tablet keeps it");
    let (info, opts) = (desktop_info(false, false), Options::default());
    set_test_form(Desktop);
    let lock = spec(PageId::LockScreen, &model(&info, &opts));
    set_test_form(Tablet);
    assert_eq!(lock.groups[0].items[0].label, "Login Password");
    assert_eq!(lock.groups[0].items[0].action, Action::Push(PageId::Users));
}

#[test]
fn touch_wording_gives_way_to_click_wording_on_a_desktop() {
    use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
    let (info, opts) = (desktop_info(false, false), Options::default());
    let footer = |form, id| {
        set_test_form(form);
        let s = spec(id, &model(&info, &opts));
        set_test_form(Tablet);
        s.groups.iter().filter_map(|g| g.footer.clone()).collect::<Vec<_>>().join(" ")
    };
    assert!(footer(Tablet, PageId::ControlCentre).contains("Swipe down from the top-right corner"));
    assert!(!footer(Desktop, PageId::ControlCentre).contains("Swipe"));
    assert!(footer(Desktop, PageId::ControlCentre).contains("Click"));
    assert!(footer(Tablet, PageId::ScreenSaver).contains("Touch the screen"));
    let desktop_saver = footer(Desktop, PageId::ScreenSaver);
    assert!(!desktop_saver.contains("Touch") && desktop_saver.contains("Click or press a key"), "{desktop_saver}");
}

#[test]
fn a_lone_command_key_row_is_left_out_of_the_desktop_shortcuts() {
    use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
    let opts = Options::default();
    let home = |form| {
        set_test_form(form);
        let s = shortcuts_page(&opts);
        set_test_form(Tablet);
        s.groups[0].items.iter().any(|i| i.label == "Home")
    };
    assert!(home(Tablet));
    assert!(!home(Desktop));
    // the other rows still address their own bits
    set_test_form(Desktop);
    let s = shortcuts_page(&opts);
    set_test_form(Tablet);
    let assistant = s.groups[0].items.iter().find(|i| i.label == "Assistant").expect("the Assistant row");
    assert_eq!(assistant.action, Action::Toggle(Toggle::Shortcut(11)));
}

#[test]
fn the_network_and_sidebar_rows_have_their_own_symbols() {
    let (info, opts) = (desktop_info(false, true), Options::default());
    let net = spec(PageId::Network, &model(&info, &opts));
    let icon = |label: &str| net.groups[0].items.iter().find(|i| i.label == label).and_then(|i| i.icon);
    assert_eq!((icon("Ethernet"), icon("VPN")), (Some(Icon::Ethernet), Some(Icon::Vpn)));
    let root = spec(PageId::Root, &model(&info, &opts));
    let icon = |label: &str| root.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == label).and_then(|i| i.icon);
    for (label, want) in [("BifrOSt Account", Icon::Account), ("Print & Scan", Icon::Printer), ("Keyboard Shortcuts", Icon::Keyboard), ("Assistant", Icon::Assistant), ("Trackpad", Icon::Trackpad), ("Mouse", Icon::Mouse), ("Search", Icon::Search), ("Network", Icon::Network)] {
        assert_eq!(icon(label), Some(want), "{label}");
    }
    // every pane row has a tile of its own kind: none of the listed ones is the gear any more
    for label in ["Print & Scan", "Keyboard Shortcuts", "Assistant", "Trackpad", "Mouse", "Search"] {
        assert_ne!(icon(label), Some(Icon::General), "{label}");
    }
}

#[test]
fn the_desktop_has_an_appearance_tile_and_lists_keyboard_once() {
    let (info, opts) = (Info { display_mode: Some((2560, 1440, 60)), ..desktop_info(false, true) }, Options::default());
    let root = desktop_sidebar(spec(PageId::Root, &model(&info, &opts)), true);
    let row = |label: &str| root.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == label).cloned();
    assert_eq!(row("Appearance").and_then(|i| i.icon), Some(Icon::Appearance), "Appearance has its own tile, not the Displays one");
    assert_eq!(row("Keyboard").map(|i| i.action), Some(Action::Push(PageId::Keyboard)), "Keyboard is a sidebar pane");
    let keyboard_in = |s: &Spec| s.groups.iter().flat_map(|g| g.items.iter()).any(|i| i.action == Action::Push(PageId::Keyboard));
    assert!(!keyboard_in(&spec(PageId::General, &model(&info, &opts))), "General does not repeat the Keyboard pane");
    // the tablet has no Keyboard pane in its root list: General keeps the row
    let tablet = Info::default();
    assert!(tablet.display_mode.is_none());
    assert!(keyboard_in(&spec(PageId::General, &model(&tablet, &opts))));
    assert!(spec(PageId::General, &model(&info, &opts)).groups.iter().flat_map(|g| g.items.iter()).any(|i| i.label == "Fonts"), "the rest of the group stays");
}

#[test]
fn the_desktop_network_page_links_to_the_firewall() {
    let mut info = Info { desktop_network: Some(Default::default()), ..Info::default() };
    info.battery_present = true;
    let opts = Options::default();
    let s = spec(PageId::Network, &model(&info, &opts));
    let firewall = s.groups[0].items.iter().find(|i| i.label == "Firewall").expect("a Firewall row");
    assert_eq!(firewall.action, Action::Push(PageId::Firewall));
    assert!(spec(PageId::Firewall, &model(&info, &opts)).groups[0].items[0].label.starts_with("No Apps"));
}

#[test]
fn wifi_page_lists_the_joined_network_first_and_the_rest_under_networks() {
    let (info, opts) = (info(), Options::default());
    let s = spec(PageId::Wifi, &model(&info, &opts));
    assert_eq!(s.groups[0].items[1].label, "WIFI-5G");
    assert!(s.groups[0].items[1].leading_check);
    assert_eq!(s.groups[0].items[1].action, Action::Push(PageId::WifiDetail));
    assert_eq!(s.groups[1].header.as_deref(), Some("NETWORKS"));
    assert_eq!(s.groups[1].items[0].action, Action::Network("Cafe".into()));
    assert_eq!(s.groups[1].items[0].accessory, Accessory::Wifi { secure: false, bars: 1 });
    assert_eq!(s.groups.last().unwrap().items[0].label, "Country or Region", "the regulatory domain shows under the networks");
    let off = spec(PageId::Wifi, &Model { wifi_on: false, ..model(&info, &opts) });
    assert_eq!(off.groups.len(), 1, "nothing but the switch while off");
    assert_eq!(off.groups[0].items.len(), 1);
    let none = Info { networks: vec![], current_ssid: None, ..info.clone() };
    let searching = spec(PageId::Wifi, &model(&none, &opts));
    assert_eq!(searching.groups[1].items[0].label, "Searching…");
    assert!(!searching.groups[1].items[0].tappable());
    let d = spec(PageId::WifiDetail, &model(&info, &opts));
    assert_eq!(d.title, "WIFI-5G");
    assert_eq!(d.groups[0].items[0].action, Action::Run(Command::Forget("WIFI-5G".into())));
    assert_eq!(d.groups[2].items[1].value, "192.168.1.1");
}

#[test]
fn saved_networks_show_notes_priority_and_the_other_row() {
    let (mut info, opts) = (info(), Options::default());
    let saved = |ssid: &str, auto_join, note: &str| SavedNetwork { ssid: ssid.into(), auto_join, note: note.into(), security: None };
    info.saved = vec![saved("WIFI-5G", true, ""), saved("Cafe", false, "Incorrect Password")];
    let s = spec(PageId::Wifi, &model(&info, &opts));
    assert_eq!(s.groups[1].items[0].value, "Incorrect Password", "the failure shows in the row");
    assert_eq!(s.groups[1].items.last().unwrap().action, Action::JoinOther);
    assert_eq!(s.groups[2].items[0].action, Action::Push(PageId::KnownNetworks));
    let k = spec(PageId::KnownNetworks, &model(&info, &opts));
    assert_eq!(k.groups[0].items.len(), 2);
    assert_eq!(k.groups[0].items[0].value, "");
    assert_eq!(k.groups[0].items[1].value, "Incorrect Password");
    let d = spec(PageId::WifiDetail, &model(&info, &opts));
    assert_eq!(d.groups[1].items[0].action, Action::Wifi(WifiAction::SetAutoJoin { ssid: "WIFI-5G".into(), on: false }));
    let priority = d.groups.last().unwrap();
    assert_eq!(priority.header.as_deref(), Some("PRIORITY 1 OF 2"));
    assert!(!priority.items[0].tappable(), "already first");
    assert_eq!(priority.items[1].action, Action::Wifi(WifiAction::Move { ssid: "WIFI-5G".into(), up: false }));
    let off = Model { detail: "Cafe", ..model(&info, &opts) };
    assert_eq!(spec(PageId::WifiDetail, &off).groups[1].items[0].accessory, Accessory::Switch(false));
}

#[test]
fn known_networks_name_the_security_and_the_details_show_the_link() {
    use crate::net::wifi_config::Security;
    let (mut info, opts) = (info(), Options::default());
    let saved = |ssid: &str, auto_join, note: &str, security| SavedNetwork { ssid: ssid.into(), auto_join, note: note.into(), security };
    info.saved = vec![saved("WIFI-5G", true, "", Some(Security::Wpa3)), saved("Cafe", false, "Incorrect Password", Some(Security::Open))];
    let k = spec(PageId::KnownNetworks, &model(&info, &opts));
    assert_eq!(k.groups[0].items[0].value, "WPA3 Personal");
    assert_eq!(k.groups[0].items[1].value, "None, Incorrect Password", "the problem follows the type");
    info.desktop_network = Some(Default::default());
    if let Some(n) = info.desktop_network.as_mut() {
        n.interfaces = vec![crate::shell::desktop_network::Interface { name: "wlan0".into(), kind: "wifi".into(), method: "auto".into(), dns: vec!["10.0.0.1".into()], ..Default::default() }];
    }
    let d = spec(PageId::WifiDetail, &model(&info, &opts));
    assert_eq!(d.title, "WIFI-5G");
    let details = d.groups.iter().find(|g| g.header.as_deref() == Some("DETAILS")).unwrap();
    assert_eq!(details.items[0].value, "WPA3 Personal");
    assert_eq!(details.items[1].value, "wlan0");
    assert_eq!(details.items[2].value, "Automatic (DHCP)");
    assert_eq!(details.items[3].value, "10.0.0.1");
    assert!(d.groups.iter().all(|g| g.header.as_deref() != Some("DIAGNOSTICS")), "WIFI-5G has no problem");
    let cafe = spec(PageId::WifiDetail, &Model { detail: "Cafe", ..model(&info, &opts) });
    let problem = cafe.groups.iter().find(|g| g.header.as_deref() == Some("DIAGNOSTICS")).unwrap();
    assert_eq!(problem.items[0].value, "Incorrect Password");
}

#[test]
fn the_reset_page_offers_a_confirmed_erase_of_all_content() {
    let info = info();
    let opts = Options::default();
    let s = spec(PageId::Reset, &model(&info, &opts));
    let erase = s.groups.iter().flat_map(|g| &g.items).find(|i| i.label == "Erase All Content and Settings").expect("the Reset page has the erase row");
    assert_eq!(erase.action, Action::Run(Command::EraseContent), "the row asks for confirmation through the command");
}

#[test]
fn desktop_display_shows_panel_mode_and_only_working_controls() {
    let mut info = info();
    info.display_mode = Some((2048, 1152, 60));
    let opts = Options::default();
    let s = desktop_display(&model(&info, &opts));
    assert_eq!(s.title, "Displays");
    assert_eq!(s.groups[0].items[0].value, "Native", "the Resolution row shows the choice, Native by default");
    assert_eq!(s.groups[0].items[1].value, "Highest Available", "the row shows the choice, Highest Available by default");
    assert_eq!(s.groups[0].items[1].action, Action::Push(PageId::RefreshRate));
    assert!(s.groups[0].items.iter().any(|i| i.label == "Graphics" && i.action == Action::Push(PageId::Graphics)), "the Graphics page is reachable");
    assert!(!s.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "Appearance"), "Appearance is its own pane, not a Displays row");
    assert!(!s.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "Use as" || i.label == "Brightness"));
    assert_eq!(s.groups[1].items[0].label, "Night Shift");
    assert!(!s.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "True Tone"));
    info.brightness_supported = true;
    let s = desktop_display(&model(&info, &opts));
    assert!(matches!(s.groups[1].items[0].body, super::super::list::Body::Slider { ends: Ends::Brightness, .. }));
    assert_eq!(s.groups[1].items.len(), 1, "no light sensor, no automatic brightness switch");
}

#[test]
fn the_speech_page_offers_the_punctuation_levels_and_checks_the_chosen_one() {
    let opts = Options::default();
    let info = info();
    let groups = |a: &AccessibilitySettings| {
        let m = Model { a11y: a, ..model(&info, &opts) };
        spec(PageId::ScreenReaderSpeech, &m).groups
    };
    let off = groups(&AccessibilitySettings::default());
    let punct = off.iter().find(|g| g.header.as_deref() == Some("PUNCTUATION")).expect("the Punctuation group");
    let labels: Vec<&str> = punct.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["None", "Some", "All"]);
    assert_eq!(punct.items[0].accessory, Accessory::Check(true));
    assert_eq!(punct.items[2].action, Action::Pick(Pick::SpeechPunctuation(2)));
    let all = groups(&AccessibilitySettings { speech_punctuation: 2, ..Default::default() });
    let punct = all.iter().find(|g| g.header.as_deref() == Some("PUNCTUATION")).unwrap();
    assert_eq!((punct.items[0].accessory.clone(), punct.items[2].accessory.clone()), (Accessory::Check(false), Accessory::Check(true)));
}

#[test]
fn the_touch_group_has_the_switch_control_and_voice_control_switches() {
    let opts = Options::default();
    let info = info();
    let touch_rows = |a: &AccessibilitySettings| -> Vec<(String, Accessory, Action)> {
        let m = Model { a11y: a, ..model(&info, &opts) };
        let s = spec(PageId::Accessibility, &m);
        let group = s.groups.iter().find(|g| g.header.as_deref() == Some("TOUCH")).expect("the Touch group");
        group.items.iter().map(|i| (i.label.clone(), i.accessory.clone(), i.action.clone())).collect()
    };
    let off = touch_rows(&AccessibilitySettings::default());
    let find = |rows: &[(String, Accessory, Action)], name: &str| rows.iter().find(|r| r.0 == name).cloned().expect(name);
    assert_eq!(find(&off, "Switch Control").1, Accessory::Switch(false));
    assert_eq!(find(&off, "Voice Control").2, Action::Toggle(Toggle::VoiceControl));
    let on = touch_rows(&AccessibilitySettings { switch_control: true, voice_control: true, ..Default::default() });
    assert_eq!(find(&on, "Switch Control").1, Accessory::Switch(true));
    assert_eq!(find(&on, "Voice Control").1, Accessory::Switch(true));
    assert_eq!(find(&on, "Switch Control").2, Action::Toggle(Toggle::SwitchControl));
}

#[test]
fn the_keyboard_group_has_the_accessibility_keyboard_switch() {
    let opts = Options::default();
    let info = info();
    let row = |a: &AccessibilitySettings| -> (Accessory, Action) {
        let m = Model { a11y: a, ..model(&info, &opts) };
        let s = spec(PageId::Accessibility, &m);
        let group = s.groups.iter().find(|g| g.header.as_deref() == Some("KEYBOARD")).expect("the Keyboard group");
        let item = group.items.iter().find(|i| i.label == "Accessibility Keyboard").expect("the Accessibility Keyboard row");
        (item.accessory.clone(), item.action.clone())
    };
    assert_eq!(row(&AccessibilitySettings::default()), (Accessory::Switch(false), Action::Toggle(Toggle::AccessibilityKeyboard)));
    assert_eq!(row(&AccessibilitySettings { accessibility_keyboard: true, ..Default::default() }).0, Accessory::Switch(true));
}

#[test]
fn input_sources_has_a_show_input_menu_switch_that_is_off_by_default() {
    let info = info();
    let row = |opts: &Options| -> (Accessory, Action) {
        let s = spec(PageId::Keyboards, &model(&info, opts));
        let item = s.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == "Show Input menu in menu bar").expect("the Show Input menu row");
        (item.accessory.clone(), item.action.clone())
    };
    assert_eq!(row(&Options::default()), (Accessory::Switch(false), Action::Toggle(Toggle::InputMenu)));
    assert_eq!(row(&Options { input_menu: true, ..Options::default() }).0, Accessory::Switch(true));
}

#[test]
fn the_motion_group_offers_vehicle_motion_cues_only_with_an_accelerometer() {
    let opts = Options::default();
    let mut info = info();
    let motion_rows = |info: &Info, a: &AccessibilitySettings| -> Vec<(String, Accessory)> {
        let m = Model { a11y: a, ..model(info, &opts) };
        let s = spec(PageId::Accessibility, &m);
        let group = s.groups.iter().find(|g| g.header.as_deref() == Some("MOTION")).expect("the Motion group");
        group.items.iter().map(|i| (i.label.clone(), i.accessory.clone())).collect()
    };
    let off = AccessibilitySettings::default();
    let rows = motion_rows(&info, &off);
    assert_eq!(rows.len(), 1, "no accelerometer, no cues switch: {rows:?}");
    info.motion_sensor_supported = true;
    let rows = motion_rows(&info, &off);
    assert_eq!(rows[1], ("Show Vehicle Motion Cues".to_string(), Accessory::Switch(false)));
    let on = AccessibilitySettings { motion_cues: true, ..Default::default() };
    assert_eq!(motion_rows(&info, &on)[1].1, Accessory::Switch(true), "the switch follows the setting");
}

#[test]
fn a_light_sensor_adds_the_automatic_brightness_switch_under_the_slider() {
    let mut info = info();
    info.display_mode = Some((2048, 1152, 60));
    info.brightness_supported = true;
    info.ambient_light_supported = true;
    let mut opts = Options::default();
    let rows = |opts: &Options| desktop_display(&model(&info, opts)).groups[1].items.clone();
    let off = rows(&opts);
    assert_eq!(off.len(), 2);
    assert_eq!(off[1].label, "Automatically Adjust Brightness");
    assert_eq!(off[1].action, Action::Toggle(Toggle::AutoBrightness));
    assert_eq!(off[1].accessory, super::super::list::Accessory::Switch(false));
    opts.auto_brightness = true;
    let on = rows(&opts);
    assert_eq!(on[1].accessory, super::super::list::Accessory::Switch(true), "the switch follows the option");
    // a panel that cannot be dimmed shows neither the slider nor the switch
    info.brightness_supported = false;
    let none = desktop_display(&model(&info, &opts));
    assert!(!none.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "Automatically Adjust Brightness"));
}

#[test]
fn the_refresh_row_shows_the_chosen_rate_and_opens_the_picker() {
    let mut info = info();
    info.display_mode = Some((2560, 1600, 165));
    info.display_rates = vec![164.85, 144.0, 60.0];
    let opts = Options::default();
    let m = Model { refresh: RefreshChoice::Hz(144.0), ..model(&info, &opts) };
    let s = desktop_display(&m);
    let row = &s.groups[0].items[1];
    assert_eq!((row.label.as_str(), row.value.as_str()), ("Refresh Rate", "144 Hz"));
    assert_eq!(row.action, Action::Push(PageId::RefreshRate));
    assert!(s.groups[0].footer.as_deref().unwrap_or("").contains("highest refresh rate is used unless another is chosen"));
}

#[test]
fn the_resolution_row_shows_native_or_the_chosen_size_and_opens_the_picker() {
    let mut info = info();
    info.display_mode = Some((2560, 1600, 165));
    info.display_sizes = vec![(2560, 1600), (1920, 1200)];
    let opts = Options::default();
    let s = desktop_display(&Model { resolution: Some((1920, 1200)), ..model(&info, &opts) });
    let row = &s.groups[0].items[0];
    assert_eq!((row.label.as_str(), row.value.as_str()), ("Resolution", "1920 × 1200"));
    assert_eq!(row.action, Action::Push(PageId::Resolution));
    let native = desktop_display(&model(&info, &opts));
    assert_eq!(native.groups[0].items[0].value, "Native");
}

#[test]
fn the_resolution_picker_lists_native_then_each_size_with_the_choice_checked() {
    let mut info = info();
    info.display_sizes = vec![(2560, 1600), (1920, 1200), (1280, 800)];
    let opts = Options::default();
    let p = spec(PageId::Resolution, &Model { resolution: Some((1920, 1200)), ..model(&info, &opts) });
    assert_eq!(p.title, "Resolution");
    let rows = &p.groups[0].items;
    let labels: Vec<&str> = rows.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Native", "2560 × 1600", "1920 × 1200", "1280 × 800"]);
    let checked: Vec<bool> = rows.iter().map(|i| i.accessory == Accessory::Check(true)).collect();
    assert_eq!(checked, [false, false, true, false], "only the chosen size is checked");
    assert_eq!(rows[0].action, Action::Pick(Pick::Resolution(0)));
    assert_eq!(rows[3].action, Action::Pick(Pick::Resolution(3)));
    let native = spec(PageId::Resolution, &model(&info, &opts));
    assert!(native.groups[0].items[0].accessory == Accessory::Check(true), "Native is checked when nothing is chosen");
}

#[test]
fn the_refresh_picker_lists_highest_available_and_each_rate_with_the_choice_checked() {
    let mut info = info();
    info.display_mode = Some((2560, 1600, 165));
    info.display_rates = vec![164.85, 144.0, 120.0, 100.0, 60.0];
    let opts = Options::default();
    let p = spec(PageId::RefreshRate, &Model { refresh: RefreshChoice::Hz(120.0), ..model(&info, &opts) });
    assert_eq!(p.title, "Refresh Rate");
    let rows = &p.groups[0].items;
    let labels: Vec<&str> = rows.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Highest Available", "164.85 Hz", "144 Hz", "120 Hz", "100 Hz", "60 Hz"]);
    let checked: Vec<bool> = rows.iter().map(|i| i.accessory == Accessory::Check(true)).collect();
    assert_eq!(checked, [false, false, false, true, false, false], "only the chosen rate is checked");
    assert_eq!(rows[0].action, Action::Pick(Pick::RefreshRate(0)));
    assert_eq!(rows[2].action, Action::Pick(Pick::RefreshRate(2)));
    let max = spec(PageId::RefreshRate, &Model { refresh: RefreshChoice::Max, ..model(&info, &opts) });
    assert_eq!(max.groups[0].items[0].accessory, Accessory::Check(true), "Highest Available is checked by default");
}

#[test]
fn sharing_and_lock_screen_are_reachable_from_the_root_and_link_their_controls() {
    let info = info();
    let mut opts = Options::default();
    let root = spec(PageId::Root, &model(&info, &opts));
    let items: Vec<&Item> = root.groups.iter().flat_map(|g| &g.items).collect();
    assert!(items.iter().any(|i| i.action == Action::Push(PageId::Sharing)), "Sharing is in the root list");
    assert!(items.iter().any(|i| i.action == Action::Push(PageId::LockScreen)), "Lock Screen is in the root list");

    // the computer name and the sharing services come first; Nearby Share and AirPlay & Handoff are the last group
    let view = super::super::sharing_page::View { computer: "Midget".into(), services: Vec::new(), screen_user: None };
    let sharing = sharing_spec(&opts, Some(&view));
    assert_eq!(sharing.groups[0].items[0].action, Action::Sharing(super::super::sharing_page::Edit::ComputerName));
    assert_eq!(sharing.groups[3].items[0].action, Action::Push(PageId::Nearby));
    assert_eq!(sharing.groups[3].items[0].value, NEARBY_CHOICES[opts.nearby.min(NEARBY_CHOICES.len() - 1)]);
    assert_eq!(sharing.groups[3].items[1].action, Action::Push(PageId::AirPlayHandoff), "AirPlay & Handoff is reachable, not an orphan page");
    opts.handoff = true;
    assert_eq!(sharing_spec(&opts, Some(&view)).groups[3].items[1].value, "Handoff On");
    assert_eq!(sharing_spec(&opts, None).groups[0].items[0].value, "Loading…", "the page shows a placeholder until the read lands");

    let lock = spec(PageId::LockScreen, &model(&info, &opts));
    assert_eq!(lock.groups[0].items[0].action, Action::Push(PageId::Passcode));
    assert_eq!(lock.groups[1].items[0].action, Action::Push(PageId::AutoLock));
    assert_eq!(lock.groups[1].items[0].value, opts.auto_lock_label());
    // Require Password has its own group now, so Screen Saver moved from group 2 to group 3
    assert_eq!(lock.groups[2].items[0].action, Action::Push(PageId::RequirePassword));
    assert_eq!(lock.groups[3].items[0].action, Action::Push(PageId::ScreenSaver));
}

#[test]
fn option_pages_reflect_the_options() {
    let info = info();
    let opts = Options { auto_lock_ms: 0, text_scale: 1.3, bold_text: true, icon_size: 1, volume: 0.25, ..Options::default() };
    let m = model(&info, &opts);
    let d = spec(PageId::Display, &m);
    assert_eq!(d.groups[2].items[0].value, "Never");
    assert_eq!(d.groups[3].items[0].body, super::super::list::Body::Slider { ends: Ends::TextSize, value: 1.0 });
    assert_eq!(d.groups[3].items[1].accessory, Accessory::Switch(true));
    let e_opts = Options { system_sleep_min: 15, ..Options::default() };
    let e = spec(PageId::Energy, &Model { opts: &e_opts, ..model(&info, &opts) });
    assert_eq!(e.groups[0].items.len(), 1, "the sleep time is one pop-up row");
    assert_eq!(e.groups[0].items[0].accessory, Accessory::Popup);
    let sleep = e.groups[0].items[0].popup.as_ref().expect("the sleep time choices");
    assert_eq!(sleep.choices.len(), SYSTEM_SLEEP.len());
    assert_eq!(sleep.selected, 3, "15 minutes");
    assert_eq!(chosen(&e.groups[0].items[0].action, 6), Some(Action::Pick(Pick::SystemSleep(6))));
    let saver_opts = Options { screensaver_module: 1, screensaver_minutes: 5, ..Options::default() };
    let ss = spec(PageId::ScreenSaver, &Model { opts: &saver_opts, ..model(&info, &opts) });
    // Colours has its own Speed choice group between Preview and Start After
    assert_eq!(ss.groups.len(), 4);
    let saver = ss.groups[0].items[0].popup.as_ref().expect("the saver choice is one pop-up row");
    assert_eq!(ss.groups[0].items[0].accessory, Accessory::Popup);
    assert_eq!(saver.choices.len(), super::super::screensaver::SAVERS.len());
    assert_eq!(saver.selected, 1);
    assert_eq!(ss.groups[1].items[0].label, "Preview");
    let speed = ss.groups[2].items[0].popup.as_ref().expect("Colours: one Speed pop-up");
    assert_eq!(speed.choices.len(), 3, "Colours: slow, normal, fast");
    let start = ss.groups[3].items[0].popup.as_ref().expect("Start After is one pop-up row");
    assert_eq!(ss.groups[3].items[0].accessory, Accessory::Popup);
    assert_eq!(start.choices.len(), super::super::screensaver::START_AFTER.len());
    assert_eq!(start.selected, super::super::screensaver::START_AFTER.iter().position(|(_, min)| *min == 5).unwrap());
    let a = spec(PageId::AutoLock, &m);
    assert_eq!(a.groups[0].items.len(), AUTO_LOCK.len());
    assert_eq!(a.groups[0].items[4].accessory, Accessory::Check(true));
    assert_eq!(a.groups[0].items[0].accessory, Accessory::Check(false));
    let h = spec(PageId::HomeScreen, &m);
    assert_eq!(h.groups[0].items[1].accessory, Accessory::Check(true));
    let s = spec(PageId::Sounds, &m);
    assert_eq!(s.groups[0].items[0].accessory, Accessory::Popup, "Alert sound is a pop-up");
    let volume = s.groups.iter().find(|g| g.header.as_deref() == Some("OUTPUT VOLUME")).expect("the output volume");
    assert_eq!(volume.items[0].body, super::super::list::Body::Slider { ends: Ends::Volume, value: m.sound.volume }, "the live level, not the saved pref");
    let b = spec(PageId::Battery, &m);
    assert_eq!(b.groups[0].items[0].value, "63%");
    assert_eq!(b.groups[0].items[1].value, "Charging");
    assert!(matches!(b.groups[1].items[0].body, super::super::list::Body::Graph(_)));
    let u = spec(PageId::SoftwareUpdate, &m);
    assert_eq!(u.groups[0].items[0].value, env!("CARGO_PKG_VERSION"));
    assert!(!BUILD.git.is_empty() && !BUILD.date.is_empty());
    let b = build_from(Some("git=0123abcd4567\ndate=2026-10-22\n"));
    assert_eq!((b.git, b.date), ("0123abcd4567", "2026-10-22"));
    let none = build_from(None);
    assert_eq!((none.git, none.date), ("unknown", "unknown"), "no build-info file: a dev build");
    assert_eq!(build_from(Some("date=2026-10-22")).git, "unknown");
    let t = spec(PageId::DateTime, &m);
    // the clock options (24-hour, date and day in the menu bar) take groups 1 and 2, so Time Zone and Time are group 3
    assert_eq!(t.groups[3].items[1].value, "14:05");
    assert_eq!(t.groups[3].items[0].action, Action::TimeZone);
    let acc = spec(PageId::Accessibility, &m);
    assert_eq!(acc.groups[0].items[1].value, "130%");
    assert_eq!(acc.groups[0].items[6].accessory, super::super::list::Accessory::Switch(false));
    for id in [PageId::General, PageId::About, PageId::Keyboard, PageId::Language, PageId::Region, PageId::Reset, PageId::ControlCentre, PageId::Privacy] {
        assert!(!spec(id, &m).groups.is_empty(), "{id:?}");
    }
}

#[test]
fn language_and_region_mark_the_fresh_install_defaults() {
    // nothing is published in this test, so the page shows the defaults: English, Iceland, automatic weeks
    use super::super::list::Accessory;
    let opts = Options::default();
    let info = info();
    let lang = spec(PageId::Language, &model(&info, &opts));
    assert_eq!(lang.groups[0].items[0].accessory, Accessory::Check(true));
    assert_eq!(lang.groups[0].items[1].accessory, Accessory::Check(false));
    assert_eq!(lang.groups[1].items[0].value, "Iceland");
    assert_eq!(lang.groups[1].items[0].action, Action::Push(PageId::Region));
    assert_eq!(lang.groups[2].items[0].accessory, Accessory::Check(true));
    assert_eq!(lang.groups[2].items[2].accessory, Accessory::Check(false));
    let region = spec(PageId::Region, &model(&info, &opts));
    assert_eq!(region.groups[0].items[0].accessory, Accessory::Check(true));
    assert_eq!(region.groups[0].items[1].action, Action::Pick(Pick::Region(1)));
}

fn bt_device(addr: &str, name: &str, paired: bool, connected: bool) -> crate::bare::bluetooth::Device {
    crate::bare::bluetooth::Device { addr: addr.into(), kind: 0, name: name.into(), paired, connected, audio: true, rssi: None, class: None }
}

#[test]
fn bluetooth_page_lists_my_devices_then_the_ones_nearby() {
    use crate::bare::bluetooth::{Cmd, State, Status};
    let opts = Options::default();
    let mut info = info();
    info.bt = Status { state: State::On, scanning: true, devices: vec![bt_device("A", "Buds", true, true), bt_device("B", "Speaker", false, false)], ..Status::default() };
    let s = spec(PageId::Bluetooth, &model(&info, &opts));
    assert_eq!(s.groups[0].items[0].action, Action::Toggle(Toggle::Bluetooth));
    assert_eq!(s.groups[1].header.as_deref(), Some("MY DEVICES"));
    assert_eq!((s.groups[1].items[0].label.as_str(), s.groups[1].items[0].value.as_str()), ("Buds", "Connected"));
    assert_eq!(s.groups[1].items[0].action, Action::Push(PageId::BluetoothDetail));
    assert_eq!(s.groups[2].items[0].action, Action::Bluetooth(Cmd::Pair("B".into())));
    let root = spec(PageId::Root, &model(&info, &opts));
    assert_eq!(root.groups[1].items[2].value, "Connected");
    info.bt.state = State::Off;
    assert_eq!(spec(PageId::Bluetooth, &model(&info, &opts)).groups.len(), 1, "off: just the switch");
    info.bt.state = State::Unavailable;
    let none = spec(PageId::Bluetooth, &model(&info, &opts));
    assert!(none.groups[0].footer.as_deref().unwrap().contains("not available"));
}

#[test]
fn bluetooth_page_offers_a_scan_row_only_when_not_scanning() {
    use crate::bare::bluetooth::{Cmd, State, Status};
    let opts = Options::default();
    let mut info = info();
    info.bt = Status { state: State::On, scanning: false, devices: vec![bt_device("B", "Speaker", false, false)], ..Status::default() };
    let s = spec(PageId::Bluetooth, &model(&info, &opts));
    let nearby = &s.groups[1].items;
    assert_eq!(nearby.len(), 2);
    assert_eq!(nearby[0].action, Action::Bluetooth(Cmd::Pair("B".into())));
    assert_eq!(nearby[1].label.as_str(), "Scan for Devices");
    assert!(nearby[1].action == Action::Bluetooth(Cmd::Scan));
    info.bt.scanning = true;
    let s = spec(PageId::Bluetooth, &model(&info, &opts));
    assert_eq!(s.groups[1].items.len(), 1, "no scan row while a scan runs");
}

#[test]
fn bluetooth_detail_offers_connect_or_disconnect_and_forget() {
    use crate::bare::bluetooth::{Cmd, State, Status};
    let opts = Options::default();
    let mut info = info();
    info.bt = Status { state: State::On, devices: vec![bt_device("A", "Buds", true, false)], ..Status::default() };
    let m = Model { detail: "Buds", ..model(&info, &opts) };
    let s = spec(PageId::BluetoothDetail, &m);
    assert_eq!(s.title, "Buds");
    assert_eq!(s.groups[0].items[0].action, Action::Bluetooth(Cmd::Connect("A".into())));
    assert_eq!(s.groups[1].items[0].action, Action::Run(Command::ForgetBluetooth("A".into())));
    info.bt.devices[0].connected = true;
    let m = Model { detail: "Buds", ..model(&info, &opts) };
    let detail = spec(PageId::BluetoothDetail, &m);
    assert_eq!(detail.groups[0].items[0].action, Action::Bluetooth(Cmd::Disconnect("A".into())));
    // a connected audio device can take the sound; the type row says what the class says, and "Unknown" with no class
    assert_eq!(detail.groups[0].items[1].label.as_str(), "Use for Sound Output");
    assert_eq!(detail.groups[0].items[1].action, Action::AudioOutput);
    let kind = &detail.groups[2].items[1];
    assert_eq!((kind.label.as_str(), kind.value.as_str()), ("Type", "Unknown"));
    // the signal row reads "Not reported" until a scan heard the device, then its strength
    let signal = &detail.groups[2].items[3];
    assert_eq!((signal.label.as_str(), signal.value.as_str()), ("Signal", "Not reported"));
    info.bt.devices[0].rssi = Some(-61);
    let heard = spec(PageId::BluetoothDetail, &Model { detail: "Buds", ..model(&info, &opts) });
    assert_eq!(heard.groups[2].items[3].value.as_str(), "-61 dBm");
    let services = &detail.groups[3];
    assert_eq!((services.items[0].label.as_str(), services.items[0].value.as_str()), ("Services", "None listed"));
    assert_eq!(services.header.as_deref(), Some("SERVICES"));
}

#[test]
fn keyboard_haptics_switch_needs_a_vibrator() {
    let (info, opts) = (info(), Options::default());
    let labels = |haptics| {
        let s = spec(PageId::Sounds, &Model { haptics, ..model(&info, &opts) });
        format!("{s:?}")
    };
    assert!(!labels(false).contains("Keyboard Haptics"));
    assert!(labels(true).contains("Keyboard Haptics"));
    assert!(labels(false).contains("Keyboard Clicks"));
}

#[test]
fn keyboard_links_to_text_replacements_and_the_empty_list_says_so() {
    let (info, opts) = (info(), Options::default());
    let keyboard = spec(PageId::Keyboard, &model(&info, &opts));
    let row = keyboard.groups[0].items.iter().find(|i| i.label == "Text Replacements").expect("the Keyboard page has the row");
    assert_eq!(row.action, Action::Push(PageId::TextReplacements));
    let page = spec(PageId::TextReplacements, &model(&info, &opts));
    assert_eq!(page.groups[0].items[0].label, "No Text Replacements", "the list is empty in a fresh shell");
    assert_eq!(page.groups[0].items[0].action, Action::None);
}
