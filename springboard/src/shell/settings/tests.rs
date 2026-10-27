use super::*;
use crate::gfx::pack;
use crate::shell::Network;

fn info() -> Info {
    Info { battery: 42, charging: true, online: false, wifi: "Off".into(), model: "Midget".into(), kernel: "6.12".into(), memory: "2.4 GB".into(), brightness: 0.5, ..Info::default() }
}

fn net(ssid: &str, secure: bool, saved: bool) -> Network {
    Network { ssid: ssid.into(), bars: 2, secure, saved }
}

fn wifi_info() -> Info {
    Info {
        wifi: "Home".into(),
        wifi_on: true,
        current_ssid: Some("Home".into()),
        networks: vec![net("Home", true, true), net("Cafe", false, false), net("Office", true, true), net("Neighbour", true, false)],
        ..info()
    }
}

/// The single-column layout: the page logic tests (push/pop, rows) predate the split view.
fn settings(ui: &Ui, info: &Info) -> (Settings, Prefs) {
    let mut st = Settings::new(ui, info, &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
    st.allow_split(ui, false);
    (st, Prefs::memory())
}

fn tap(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, x: f32, y: f32) -> Vec<Effect> {
    let mut fx = st.touch(ui, prefs, Event::Down(x, y), 0.0);
    fx.extend(st.touch(ui, prefs, Event::Up(x, y), 10.0));
    fx
}

/// Taps the row labelled `label` on the top page and settles the slide.
fn tap_row(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label: &str) -> Vec<Effect> {
    // root rows live in the sidebar in the bifrost split layout
    let (x, y) = st.row_centre(ui, label).or_else(|| st.sidebar_centre(ui, label)).unwrap_or_else(|| panic!("no row {label} on {:?}", st.page()));
    let fx = tap(st, ui, prefs, x, y);
    settle(st, ui);
    fx
}

/// Chooses the `k`th choice of the pop-up row labelled `label`: taps the row to open its menu, then picks from the menu.
fn pick(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label: &str, k: usize) -> Vec<Effect> {
    tap_row(st, ui, prefs, label);
    let fx = st.activate(ui, prefs, Hit::Choice(k), 0.0);
    settle(st, ui);
    fx
}

fn settle(st: &mut Settings, ui: &Ui) {
    st.tick(ui, 1e9);
    // the pages' reads run on worker threads (see `settings_load`): wait for them, as a frame would
    let began = std::time::Instant::now();
    while st.loads.busy() && began.elapsed() < std::time::Duration::from_secs(30) {
        std::thread::sleep(std::time::Duration::from_millis(1));
        st.tick(ui, 1e9);
    }
}

fn back(st: &mut Settings, ui: &Ui, prefs: &mut Prefs) {
    tap(st, ui, prefs, ui.pt(40.0), ui.pt(42.0));
    settle(st, ui);
}

/// A data dir with the packed default fonts and one library font (a copy of the system font).
fn font_dir(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("settings-fonts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
    for f in ["SFUI.ttf", "SFUIMono.ttf"] {
        std::fs::copy(format!("{assets}/{f}"), dir.join(f)).unwrap();
    }
    let lib = dir.join("fonts/Other.ttf");
    std::fs::copy(format!("{assets}/SFUI.ttf"), &lib).unwrap();
    (dir, lib.to_string_lossy().into_owned())
}

/// Lets the Fonts read on its worker land, as the frames would.
fn settle_fonts(st: &mut Settings, ui: &Ui) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while st.loads.fonts.busy() && std::time::Instant::now() < end {
        st.tick(ui, 0.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    st.tick(ui, 0.0);
}

#[test]
fn a_deep_link_from_another_app_opens_its_pane() {
    let ui = super::super::tests::ui();
    let (mut st, _prefs) = settings(&ui, &wifi_info());
    st.open_page(&ui, "Wi-Fi Settings", 0.0);
    assert_eq!(st.page(), PageId::Wifi);
    st.open_page(&ui, "Battery", 0.0);
    assert_eq!(st.page(), PageId::Battery);
    st.open_page(&ui, "Print & Scan", 0.0);
    assert_eq!(st.page(), PageId::PrintScan);
    assert!(st.open_page(&ui, "No Such Pane", 0.0).is_empty());
}

/// Opens General > Startup Disk with a boot table of two usable systems landed.
fn startup_page(ui: &Ui) -> (Settings, Prefs) {
    use super::super::startup_disk_page::{Disk, View};
    let (mut st, mut prefs) = settings(ui, &wifi_info());
    tap_row(&mut st, ui, &mut prefs, "General");
    tap_row(&mut st, ui, &mut prefs, "Startup Disk");
    assert_eq!(st.page(), PageId::StartupDisk);
    let disk = |id: &str, label: &str, default: bool| Disk { id: id.into(), label: label.into(), active: true, current: default, next: false, default };
    st.loads.startup.set(View { disks: vec![disk("0001", "BifrOSt", true), disk("0003", "Rescue", false)], error: None });
    st.rebuild_visible(ui);
    (st, prefs)
}

#[test]
fn the_screen_sharing_switch_asks_for_an_account_before_anything_starts() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "Sharing");
    assert_eq!(st.page(), PageId::Sharing);
    st.loads.sharing.set(super::super::sharing_page::View { computer: "Midget".into(), services: vec![("screen_sharing", false)], screen_user: None });
    st.rebuild_visible(&ui);
    tap_row(&mut st, &ui, &mut prefs, "Screen Sharing");
    assert!(st.sheet.is_some(), "the account prompt is up");
    assert!(st.alert().is_none(), "nothing was sent to privd");
}

#[test]
fn startup_disk_is_a_general_row_that_lists_the_systems() {
    let ui = super::super::tests::ui();
    let (st, _prefs) = startup_page(&ui);
    let spec = st.spec_for(PageId::StartupDisk);
    let labels: Vec<&str> = spec.groups[0].items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["BifrOSt", "Rescue"]);
    assert_eq!(spec.groups[0].items[1].action, Action::StartupDisk(super::super::startup_disk_page::Edit::Choose("0003".into())));
}

#[test]
fn choosing_a_system_waits_for_the_password_sheet_and_a_cancel_changes_nothing() {
    use super::super::services::local_auth::{self, Failure};
    use super::super::startup_disk_page::APP;
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = startup_page(&ui);
    tap_row(&mut st, &ui, &mut prefs, "Rescue");
    assert_eq!(st.startup_pending.as_deref(), Some("0003"));
    let request = local_auth::take_request().expect("the sheet is queued");
    assert_eq!(request.app, APP);
    assert!(request.reason.contains("Rescue"));
    // while the sheet is up a second tap does not queue another
    assert!(local_auth::take_request().is_none());
    local_auth::finish(APP, Err(Failure::UserCancel));
    st.tick(&ui, 2e9);
    assert_eq!(st.startup_pending, None);
    assert!(st.alert().is_none(), "a cancel says nothing");
}

#[test]
fn a_wrong_password_is_reported_and_sets_nothing() {
    use super::super::services::local_auth::{self, Failure};
    use super::super::startup_disk_page::APP;
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = startup_page(&ui);
    tap_row(&mut st, &ui, &mut prefs, "Rescue");
    assert!(local_auth::take_request().is_some());
    local_auth::finish(APP, Err(Failure::AuthenticationFailed));
    st.tick(&ui, 2e9);
    assert_eq!(st.startup_pending, None);
    assert!(st.alert().is_some(), "the person is told the change did not happen");
}

#[test]
fn the_firewall_page_hands_the_shell_each_change() {
    use super::super::firewall_sheet::{Edit, FirewallView};
    use bifrost_core::app_firewall::Policy;
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    st.set_firewall(vec![FirewallView { app_id: "is.example.chat".into(), name: "Chat".into(), policy: Policy::Block, ports: "tcp/8080".into() }]);
    st.open_page_id(&ui, PageId::Firewall, 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), PageId::Firewall);
    tap_row(&mut st, &ui, &mut prefs, "Allow Incoming Connections");
    assert_eq!(st.take_firewall_edits(), vec![("is.example.chat".to_string(), Edit::Allow)]);
    assert_eq!(st.firewall[0].policy, Policy::Allow, "the row shows the choice at once");
    assert!(st.take_firewall_edits().is_empty());
    tap_row(&mut st, &ui, &mut prefs, "Forget This Choice");
    assert_eq!(st.take_firewall_edits(), vec![("is.example.chat".to_string(), Edit::Forget)]);
    assert!(st.firewall.is_empty(), "a forgotten choice leaves the page");
}

#[test]
fn picking_a_library_font_saves_the_override_the_shell_reads() {
    use bifrost_render::fontprefs::{family, FontRole};
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    let (dir, lib) = font_dir("pick");
    st.set_data_dir(&dir);
    let name = family(&lib).expect("the library font has a family name");
    for label in ["General", "Fonts"] {
        tap_row(&mut st, &ui, &mut prefs, label);
    }
    settle_fonts(&mut st, &ui);
    tap_row(&mut st, &ui, &mut prefs, "System Font");
    assert_eq!(st.page(), PageId::FontPick(FontRole::System));
    tap_row(&mut st, &ui, &mut prefs, &name);
    assert_eq!(prefs.get(FontRole::System.pref()), Some(lib.as_str()), "saved under the key the shell reads at start");
    assert_eq!(st.loads.fonts.value.overrides[FontRole::System.index()].as_deref(), Some(lib.as_str()));
    let default = format!("{} (Default)", st.loads.fonts.value.defaults[FontRole::System.index()]);
    tap_row(&mut st, &ui, &mut prefs, &default);
    assert_eq!(prefs.get(FontRole::System.pref()), Some(""), "Default clears the override");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Developer preview: `PREVIEW_DIR=/tmp FONTS_DIR=<packed assets> cargo test preview_fonts_pages -- --ignored`
/// writes fonts.png and fonts-system.png, the desktop Fonts pages drawn with the packed default font.
#[test]
#[ignore]
fn preview_fonts_pages() {
    let (Some(out), Some(fonts)) = (std::env::var_os("PREVIEW_DIR"), std::env::var("FONTS_DIR").ok()) else { return };
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{fonts}/SFUI.ttf")).unwrap().into_boxed_slice());
    let ui = Ui::new(crate::font::Font::parse(data).unwrap(), 1.5);
    let (w, h) = (1500usize, 1050usize);
    let info = Info { display_mode: Some((2048, 1152, 60)), ..wifi_info() };
    let mut prefs = Prefs::memory();
    let mut st = Settings::new(&ui, &info, &prefs, w as i32, h as i32, Rc::new(vec![]));
    st.set_data_dir(&fonts);
    st.open_page(&ui, "Fonts", 0.0);
    settle(&mut st, &ui);
    let shot = |st: &mut Settings, name: &str| {
        let mut img = crate::gfx::Image::new(w, h);
        st.draw(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 }, &ui, 1e9);
        std::fs::write(std::path::Path::new(&out).join(name), crate::shell::screenshot::encode(&img)).unwrap();
    };
    shot(&mut st, "fonts.png");
    tap_row(&mut st, &ui, &mut prefs, "System Font");
    shot(&mut st, "fonts-system.png");
}

#[test]
fn banded_page_draw_equals_the_single_thread_draw() {
    let ui = super::super::tests::ui();
    let (st, _) = settings(&ui, &wifi_info());
    let (w, h) = (1200usize, 1920usize);
    let mut one = vec![0u32; w * h];
    st.page_view().expect("plain list").draw(&mut Surface { w, h, stride: w, px: &mut one, y0: 0 }, &ui, 1e9);
    let mut many = vec![0u32; w * h];
    let view = st.page_view().unwrap();
    Surface { w, h, stride: w, px: &mut many, y0: 0 }.draw_rows(&[(0, h)], 4, &|s| view.draw(s, &ui, 1e9));
    assert!(one == many, "banded draw differs from single-thread draw");
}

#[test]
fn rows_push_pages_and_back_pops_them_with_a_slide() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    assert_eq!(st.page(), PageId::Root);
    let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
    let fx = tap(&mut st, &ui, &mut prefs, x, y);
    assert_eq!(fx, vec![Effect::Wifi(WifiAction::Scan)]);
    assert_eq!(st.page(), PageId::Wifi);
    assert!(st.transitioning());
    assert_eq!(st.next_wake(20.0), Some(20.0), "the slide wants frames");
    assert!(tap(&mut st, &ui, &mut prefs, x, y).is_empty(), "touches wait for the slide");
    let mut buf = vec![0u32; 1200 * 1920];
    st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 100.0);
    assert!(st.tick(&ui, 200.0));
    assert!(st.tick(&ui, 1000.0), "one last frame draws the settled page");
    assert!(!st.transitioning());
    assert!(!st.tick(&ui, 1100.0));
    assert_eq!(st.stack(), vec![PageId::Root, PageId::Wifi]);
    back(&mut st, &ui, &mut prefs);
    assert_eq!(st.page(), PageId::Root);
    assert!(tap_row(&mut st, &ui, &mut prefs, "Battery").is_empty(), "the Battery row is a plain link");
    assert_eq!(st.page(), PageId::Battery);
    back(&mut st, &ui, &mut prefs);
    back(&mut st, &ui, &mut prefs);
    assert_eq!(st.stack(), vec![PageId::Root], "the root never pops");
}

#[test]
fn every_page_is_reachable_and_draws() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    let mut buf = vec![0u32; 1200 * 1920];
    let mut draw = |st: &mut Settings| st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
    let routes: Vec<(Vec<&str>, PageId)> = vec![
        (vec!["Sounds"], PageId::Sounds),
        (vec!["Focus"], PageId::Focus),
        (vec!["Notifications"], PageId::Notifications),
        (vec!["General"], PageId::General),
        (vec!["General", "About"], PageId::About),
        (vec!["General", "Software Update"], PageId::SoftwareUpdate),
        (vec!["General", "Date & Time"], PageId::DateTime),
        (vec!["General", "Default Apps"], PageId::DefaultApps),
        (vec!["General", "Login Items & Extensions"], PageId::LoginItems),
        (vec!["General", "Game Controllers"], PageId::GameControllers),
        (vec!["General", "Fonts"], PageId::Fonts),
        (vec!["General", "Fonts", "System Font"], PageId::FontPick(bifrost_render::fontprefs::FontRole::System)),
        (vec!["General", "Keyboard"], PageId::Keyboard),
        (vec!["General", "Keyboard", "Keyboards"], PageId::Keyboards),
        (vec!["General", "Keyboard", "Text Replacements"], PageId::TextReplacements),
        (vec!["General", "Keyboard", "Modifier Keys"], PageId::ModifierKeys),
        (vec!["General", "Language & Region"], PageId::Language),
        (vec!["General", "Language & Region", "Region"], PageId::Region),
        (vec!["General", "Transfer or Reset Tab"], PageId::Reset),
        (vec!["Control Center"], PageId::ControlCentre),
        (vec!["Menu Bar"], PageId::MenuBar),
        (vec!["Display & Brightness"], PageId::Display),
        (vec!["Display & Brightness", "Auto-Lock"], PageId::AutoLock),
        (vec!["Home Screen & Dock"], PageId::HomeScreen),
        (vec!["Home Screen & Dock", "Hot Corners"], PageId::HotCorners),
        (vec!["Overview"], PageId::Overview),
        (vec!["Sharing"], PageId::Sharing),
        (vec!["Print & Scan"], PageId::PrintScan),
        (vec!["Sharing", "Nearby Share"], PageId::Nearby),
        (vec!["Sharing", "AirPlay & Handoff"], PageId::AirPlayHandoff),
        (vec!["Lock Screen"], PageId::LockScreen),
        (vec!["Trackpad"], PageId::Trackpad),
        (vec!["Mouse"], PageId::Mouse),
        (vec!["Users & Groups"], PageId::Users),
        (vec!["Keyboard Shortcuts"], PageId::Shortcuts),
        (vec!["Accessibility"], PageId::Accessibility),
        (vec!["Wallpaper"], PageId::Wallpaper),
        (vec!["Screen Saver"], PageId::ScreenSaver),
        (vec!["Battery"], PageId::Battery),
        (vec!["Privacy"], PageId::Privacy),
        (vec!["Privacy", "App Permissions"], PageId::PrivacyPermissions),
        (vec!["Privacy", "Location Services"], PageId::LocationServices),
        (vec!["Privacy", "Analytics & Diagnostics"], PageId::Diagnostics),
        (vec!["General", "bifrost Storage"], PageId::Storage),
        (vec!["General", "Backup"], PageId::Backup),
        (vec!["Passcode"], PageId::Passcode),
        (vec!["Passwords"], PageId::Passwords),
        (vec!["Internet Accounts"], PageId::InternetAccounts),
        (vec!["Developer"], PageId::Developer),
        (vec!["Wi-Fi", "Home"], PageId::WifiDetail),
    ];
    let desk = true;
    let routes: Vec<(Vec<&str>, PageId)> = routes
        .into_iter()
        .filter(|(r, _)| !(desk && r.last() == Some(&"Transfer or Reset Tab")) && (desk || r.last() != Some(&"Default Apps")))
        .map(|(r, id)| if desk && r.last() == Some(&"bifrost Storage") { (vec!["General", "Storage"], id) } else { (r, id) })
        .collect();
    for (route, id) in routes {
        for label in &route {
            tap_row(&mut st, &ui, &mut prefs, label);
        }
        assert_eq!(st.page(), id, "{route:?}");
        draw(&mut st);
        for _ in 0..route.len() {
            back(&mut st, &ui, &mut prefs);
        }
        assert_eq!(st.page(), PageId::Root);
    }
}

#[test]
fn a_drag_off_the_row_cancels_the_tap_and_scrolls() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
    st.touch(&ui, &mut prefs, Event::Down(x, y), 0.0);
    for k in 1..=5 {
        st.touch(&ui, &mut prefs, Event::Move(x, y - 80.0 * k as f32), k as f64 * 30.0);
    }
    assert!(st.touch(&ui, &mut prefs, Event::Up(x, y - 400.0), 160.0).is_empty());
    assert_eq!(st.page(), PageId::Root);
    assert!(st.list().unwrap().offset() > 0.0, "the list scrolled");
}

#[test]
fn airplane_mode_switches_wifi_off_and_on() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Airplane Mode"), vec![Effect::Wifi(WifiAction::Enable(false))]);
    let root = st.list().unwrap();
    let (g, i) = root.find("Wi-Fi").unwrap();
    assert_eq!(root.item(g, i).value, "Off");
    // group 0 is the BifrOSt Account row, so the Airplane Mode switch is the first row of group 1
    assert_eq!(root.item(1, 0).accessory, super::super::list::Accessory::Switch(true));
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Airplane Mode"), vec![Effect::Wifi(WifiAction::Enable(true))]);
    assert_eq!(st.list().unwrap().item(g, i).value, "Home");
}

#[test]
fn wifi_switch_toggles_and_hides_the_networks() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    // the switch, the networks, and the Country or Region group
    assert_eq!(st.list().unwrap().spec().groups.len(), 3);
    // the switch animates right after the tap; the settle that follows reloads the page and ends it
    let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
    let fx = tap(&mut st, &ui, &mut prefs, x, y);
    assert_eq!(fx, vec![Effect::Wifi(WifiAction::Enable(false))]);
    assert!(st.list().unwrap().animating(50.0), "the switch animates");
    settle(&mut st, &ui);
    assert_eq!(st.list().unwrap().spec().groups.len(), 1, "no networks while off");
    let fx = tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    assert_eq!(fx, vec![Effect::Wifi(WifiAction::Enable(true)), Effect::Wifi(WifiAction::Scan)]);
    assert_eq!(st.list().unwrap().spec().groups.len(), 3, "the switch, the networks and Country or Region are back");
}

#[test]
fn networks_join_select_or_ask_for_a_password() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Cafe"), vec![Effect::Wifi(WifiAction::Join { ssid: "Cafe".into(), psk: None })]);
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Office"), vec![Effect::Wifi(WifiAction::Select("Office".into()))]);
    assert!(tap_row(&mut st, &ui, &mut prefs, "Neighbour").is_empty());
    assert_eq!(st.sheet(), Some("Neighbour"));
}

#[test]
fn other_row_opens_the_sheet_that_joins_a_hidden_network() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    assert!(tap_row(&mut st, &ui, &mut prefs, "Other…").is_empty());
    assert!(st.sheet().is_some());
    st.key(&ui, Out::Text("Ghost".into()), 0.0);
    st.key(&ui, Out::Named("Tab"), 0.0);
    st.key(&ui, Out::Text("secret12".into()), 0.0);
    let fx = st.key(&ui, Out::Enter, 0.0);
    assert!(matches!(&fx[..], [Effect::Wifi(WifiAction::JoinOther(c))] if c.ssid == "Ghost" && c.hidden && c.password == "secret12"));
    assert_eq!(st.sheet(), None);
}

#[test]
fn a_scanned_enterprise_network_opens_the_full_sheet() {
    let ui = super::super::tests::ui();
    let info = Info { enterprise: vec!["Neighbour".into()], ..wifi_info() };
    let (mut st, mut prefs) = settings(&ui, &info);
    tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    assert!(tap_row(&mut st, &ui, &mut prefs, "Neighbour").is_empty());
    assert_eq!(st.sheet(), Some("Neighbour"));
    assert!(st.sheet.as_ref().unwrap().config().unwrap().security.is_enterprise());
}

fn sheet_open(ui: &Ui) -> (Settings, Prefs) {
    let (mut st, mut prefs) = settings(ui, &wifi_info());
    tap_row(&mut st, ui, &mut prefs, "Wi-Fi");
    tap_row(&mut st, ui, &mut prefs, "Neighbour");
    (st, prefs)
}

#[test]
fn join_needs_eight_characters() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    let (jx, jy) = st.sheet.as_ref().unwrap().join_button(&ui, 1200, 1920);
    st.key(&ui, Out::Text("secret1".into()), 0.0);
    assert!(tap(&mut st, &ui, &mut prefs, jx, jy).is_empty(), "7 characters");
    assert!(st.key(&ui, Out::Enter, 0.0).is_empty());
    st.key(&ui, Out::Text("2".into()), 0.0);
    let join = vec![Effect::Wifi(WifiAction::Join { ssid: "Neighbour".into(), psk: Some("secret12".into()) })];
    assert_eq!(tap(&mut st, &ui, &mut prefs, jx, jy), join);
    assert_eq!(st.sheet(), None);
    let (mut st, _) = sheet_open(&ui);
    st.key(&ui, Out::Text("secret12".into()), 0.0);
    assert_eq!(st.key(&ui, Out::Enter, 0.0), join, "return submits");
}

#[test]
fn cancel_closes_the_sheet_without_effects() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    let (cx, cy) = st.sheet.as_ref().unwrap().cancel_button(&ui, 1200, 1920);
    assert!(tap(&mut st, &ui, &mut prefs, cx, cy).is_empty());
    assert_eq!(st.sheet(), None);
    assert_eq!(st.page(), PageId::Wifi);
}

#[test]
fn hidden_keyboard_returns_when_the_field_is_tapped() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    st.key(&ui, Out::Hide, 0.0);
    let sh = st.sheet.as_ref().unwrap();
    assert!(sh.keyboard_top(&ui, 1920).is_none());
    let (fx, fy) = sh.field(&ui, 1200, 1920);
    tap(&mut st, &ui, &mut prefs, fx, fy);
    assert!(st.sheet.as_ref().unwrap().keyboard_top(&ui, 1920).is_some());
}

#[test]
fn long_press_on_the_password_field_offers_paste() {
    use super::super::clipboard;
    let _g = clipboard::tests::exclusive();
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    clipboard::set_text("hunter22");
    let (fx, fy) = st.sheet.as_ref().unwrap().field(&ui, 1200, 1920);
    st.touch(&ui, &mut prefs, Event::Down(fx, fy), 1000.0);
    assert!(st.next_wake(1000.0).is_some_and(|w| w <= 1400.0), "the long press wants a tick");
    assert!(st.tick(&ui, 1400.0));
    assert!(st.touch(&ui, &mut prefs, Event::Up(fx, fy), 1450.0).is_empty());
    let menu = st.sheet.as_ref().unwrap().field.menu().expect("Paste over the field");
    let (mx, my, _, mh) = menu.rect();
    assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(super::super::edit::Action::Paste));
    assert!(tap(&mut st, &ui, &mut prefs, mx + 5.0, my + mh / 2.0).is_empty());
    let sh = st.sheet.as_ref().unwrap();
    assert_eq!(sh.password(), "hunter22");
    assert!(sh.can_join() && sh.field.menu().is_none());
}

#[test]
fn typing_on_the_docked_keyboard_edits_the_password() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    let top = st.sheet.as_ref().unwrap().keyboard_top(&ui, 1920).unwrap();
    let (x, y) = st.sheet.as_ref().unwrap().keyboard.centre("q").unwrap();
    let y = top + y;
    tap(&mut st, &ui, &mut prefs, x, y);
    assert_eq!(st.sheet.as_ref().unwrap().password(), "q");
}

fn stills() -> Rc<Vec<Wallpaper>> {
    let thumb = |c| Image { w: 320, h: 180, px: vec![pack(c, c, c, 255); 320 * 180] };
    Rc::new(vec![
        Wallpaper { id: "a".into(), name: "Desert Peak Day".into(), thumb: thumb(10) },
        Wallpaper { id: "b".into(), name: "Lake The Lake Night".into(), thumb: thumb(20) },
    ])
}

#[test]
fn dynamic_wallpaper_switch_persists_and_turning_it_off_restores_the_picked_still() {
    let ui = super::super::tests::ui();
    let mut prefs = Prefs::memory();
    let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
    st.allow_split(&ui, false);
    assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
    let (x, y, w, h) = super::super::wallpaper_page::dynamic_row(&ui, 1200);
    assert!(tap(&mut st, &ui, &mut prefs, x + w / 2.0, y + h / 2.0).is_empty(), "on: the shell changes the still on its next tick");
    assert_eq!(prefs.get("wallpaper_dynamic"), Some("1"));
    assert_eq!(tap(&mut st, &ui, &mut prefs, x + w / 2.0, y + h / 2.0), vec![Effect::Wallpaper("a".into())], "off: the picked still comes back");
    assert_eq!(prefs.get("wallpaper_dynamic"), Some("0"));
}

#[test]
fn live_look_row_picks_an_animated_wallpaper_persists_it_and_reset_goes_back_to_the_still() {
    let ui = super::super::tests::ui();
    let mut prefs = Prefs::memory();
    let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
    st.allow_split(&ui, false);
    assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
    let centre = |i: usize| {
        let (x, y, w, h) = super::super::wallpaper_page::look_segment(&ui, 1200, i);
        (x + w / 2.0, y + h / 2.0)
    };
    for (i, look) in Look::ALL.into_iter().enumerate() {
        let (x, y) = centre(i);
        assert_eq!(tap(&mut st, &ui, &mut prefs, x, y), vec![Effect::LiveWallpaper(look.pref().to_string())], "{look:?}");
        assert_eq!(prefs.get("wallpaper_look"), Some(look.pref()));
        let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
        assert_eq!(page.look(), look, "the selected plate follows the tap");
    }
    let (x, y) = centre(1);
    tap(&mut st, &ui, &mut prefs, x, y);
    let mut buf = vec![0u32; 1200 * 1920];
    st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
    // a fresh Settings reads the stored look back
    let again = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
    assert_eq!(again.wallpaper_look, Look::Drift);
    let fx = st.run(&ui, &mut prefs, Command::ResetSettings, 0.0);
    assert!(fx.contains(&Effect::LiveWallpaper("still".into())));
    assert_eq!(st.wallpaper_look, Look::Still);
    assert_eq!(prefs.get("wallpaper_look"), None);
}

#[test]
fn on_the_desktop_a_click_applies_a_wallpaper_where_show_on_says_and_the_add_buttons_ask_for_the_panel() {
    use bifrost_ui::form_factor::FormFactor;
    let ui = super::super::tests::ui();
    super::super::pages::set_test_form(FormFactor::Desktop);
    let mut prefs = Prefs::memory();
    let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
    st.allow_split(&ui, false); // single-column geometry
    assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
    let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
    assert!(page.desktop());
    let (cx, cy, cw, ch) = page.cell(&ui, 1200, 1);
    // a click applies it at once, to the desktop and the lock screen together: no preview
    assert_eq!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0), vec![Effect::LockWallpaper(String::new()), Effect::Wallpaper("b".into())]);
    let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
    assert!(!page.previewing(), "the desktop has no Set Lock Screen / Home / Both preview");
    assert_eq!((prefs.get("wallpaper"), st.info.wallpaper.as_str()), (Some("b"), "b"));
    // Lock Screen Only: the next click sets the lock screen alone, and the choice is kept
    let (sx, sy, sw, sh) = super::super::wallpaper_page::show_on_segment(&ui, 1200, 1);
    assert!(tap(&mut st, &ui, &mut prefs, sx + sw / 2.0, sy + sh / 2.0).is_empty());
    assert_eq!(prefs.get(SHOW_ON_PREF), Some("lock"));
    let (cx, cy, cw, ch) = match &st.stack.last().unwrap().view {
        View::Wallpaper(page) => page.cell(&ui, 1200, 0),
        _ => panic!("wallpaper page"),
    };
    assert_eq!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0), vec![Effect::LockWallpaper("a".into())]);
    assert_eq!((prefs.get("wallpaper_lock"), st.info.wallpaper.as_str()), (Some("a"), "b"), "the desktop keeps its wallpaper");
    assert_eq!(Settings::new(&ui, &info(), &prefs, 1200, 1920, stills()).wallpaper_show_on, ShowOn::LockScreen);
    // Add Photo… and Add Folder… ask the shell for its open panel once
    for (i, kind) in Import::ALL.into_iter().enumerate() {
        let (bx, by, bw, bh) = super::super::wallpaper_page::add_button(&ui, 1200, i);
        assert!(tap(&mut st, &ui, &mut prefs, bx + bw / 2.0, by + bh / 2.0).is_empty());
        assert_eq!(st.take_wallpaper_import(), Some(kind));
        assert_eq!(st.take_wallpaper_import(), None);
    }
    // a picture added through the panel joins the grid and is applied where Show On says
    let mut more = (*stills()).clone();
    more.push(Wallpaper { id: "photo_fjord".into(), name: "Fjord".into(), thumb: Image { w: 320, h: 180, px: vec![pack(1, 2, 3, 255); 320 * 180] } });
    let fx = st.wallpapers_added(&ui, &mut prefs, Rc::new(more), &["photo_fjord".to_string()]);
    assert_eq!(fx, vec![Effect::LockWallpaper("photo_fjord".into())]);
    let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
    assert_eq!(page.len(), 3, "the grid lists the added picture");
    super::super::pages::set_test_form(FormFactor::Tablet);
}

#[test]
fn wallpaper_row_opens_the_grid_and_a_tap_picks_and_persists_a_wallpaper() {
    let ui = super::super::tests::ui();
    let mut prefs = Prefs::memory();
    let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
    st.allow_split(&ui, false); // single-column geometry
    assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
    assert_eq!(st.page(), PageId::Wallpaper);
    let (cx, cy, cw, ch) = super::super::wallpaper_page::cell(&ui, 1200, 9.0 / 16.0, 1);
    assert!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0).is_empty(), "opens the preview");
    let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
    let (bx, by, bw, bh) = page.button(&ui, 1200, 1920, 2);
    assert_eq!(tap(&mut st, &ui, &mut prefs, bx + bw / 2.0, by + bh / 2.0), vec![Effect::LockWallpaper(String::new()), Effect::Wallpaper("b".into())]);
    assert_eq!(prefs.get("wallpaper"), Some("b"));
    assert_eq!(st.info.wallpaper, "b");
    let mut buf = vec![0u32; 1200 * 1920];
    st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
    back(&mut st, &ui, &mut prefs);
    assert_eq!(st.page(), PageId::Root);
    let root = st.list().unwrap();
    let (g, i) = root.find("Wallpaper").unwrap();
    assert_eq!(root.item(g, i).value, "Lake The Lake Night");
    st.refresh(&ui, &Info { wallpaper: "b".into(), ..info() });
    assert_eq!(wallpaper_name(&st.wallpapers, "zzz"), "");
}

#[test]
fn resize_keeps_page_sheet_and_text_and_widens_the_keyboard() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = sheet_open(&ui);
    st.key(&ui, Out::Text("abc".into()), 0.0);
    let before = st.sheet.as_ref().unwrap().keyboard.h();
    st.resize(&ui, 1920, 1200);
    assert_eq!(st.page(), PageId::Wifi);
    assert_eq!(st.sheet.as_ref().unwrap().password(), "abc");
    assert_eq!(st.sheet.as_ref().unwrap().keyboard.w, 1920.0);
    assert!(st.sheet.as_ref().unwrap().keyboard.h() > before);
    let mut buf = vec![0u32; 1920 * 1200];
    st.draw(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut buf, y0: 0 }, &ui, 0.5);
    assert!(st.touch(&ui, &mut prefs, Event::Down(1900.0, 600.0), 1.0).is_empty());
    st.resize(&ui, 1200, 1920);
    assert_eq!(st.sheet.as_ref().unwrap().keyboard.h(), before);
}

#[test]
fn refresh_keeps_page_sheet_and_scroll() {
    let ui = super::super::tests::ui();
    let (mut st, prefs) = sheet_open(&ui);
    st.key(&ui, Out::Text("abc".into()), 0.0);
    st.refresh(&ui, &Info { battery: 7, ..wifi_info() });
    assert_eq!(st.page(), PageId::Wifi);
    assert_eq!(st.sheet.as_ref().unwrap().password(), "abc");
    // a short screen makes About scroll
    let mut st = Settings::new(&ui, &wifi_info(), &prefs, 1200, 900, Rc::new(vec![]));
    st.allow_split(&ui, false); // single-column geometry
    let mut prefs2 = Prefs::memory();
    tap_row(&mut st, &ui, &mut prefs2, "General");
    tap_row(&mut st, &ui, &mut prefs2, "About");
    st.touch(&ui, &mut prefs2, Event::Down(600.0, 800.0), 0.0);
    for k in 1..=6 {
        st.touch(&ui, &mut prefs2, Event::Move(600.0, 800.0 - 100.0 * k as f32), k as f64 * 30.0);
    }
    st.touch(&ui, &mut prefs2, Event::Up(600.0, 200.0), 1000.0);
    settle(&mut st, &ui);
    let offset = st.list().unwrap().offset();
    assert!(offset > 0.0);
    st.refresh(&ui, &Info { battery: 9, ..wifi_info() });
    assert_eq!(st.list().unwrap().offset(), offset, "scroll survives a refresh");
    assert_eq!(st.stack(), vec![PageId::Root, PageId::General, PageId::About]);
}

#[test]
fn switches_persist_their_options() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Sounds");
    assert!(tap_row(&mut st, &ui, &mut prefs, "Keyboard Clicks").is_empty());
    assert_eq!(prefs.get("keyboard_clicks"), Some("0"));
    tap_row(&mut st, &ui, &mut prefs, "Lock Sound");
    assert_eq!(prefs.get("lock_sound"), Some("0"));
    // Alert sound is a pop-up (the Golden Gate Sound pane): its second choice is Chime
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Alert sound", 1), vec![Effect::AlertSound(1)], "choosing an alert sound plays it once");
    assert_eq!(prefs.get("alert_sound"), Some("1"));
    assert_eq!(st.options().alert_sound, 1);
    back(&mut st, &ui, &mut prefs);
    tap_row(&mut st, &ui, &mut prefs, "General");
    tap_row(&mut st, &ui, &mut prefs, "Date & Time");
    tap_row(&mut st, &ui, &mut prefs, "24-Hour Time");
    assert_eq!(prefs.get("clock_24h"), Some("0"));
    assert!(!st.options().clock_24h);
    back(&mut st, &ui, &mut prefs);
    tap_row(&mut st, &ui, &mut prefs, "Keyboard");
    tap_row(&mut st, &ui, &mut prefs, "Auto-Capitalisation");
    assert_eq!(prefs.get("auto_caps"), Some("0"));
    back(&mut st, &ui, &mut prefs);
    back(&mut st, &ui, &mut prefs);
    tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
    tap_row(&mut st, &ui, &mut prefs, "Bold Text");
    assert_eq!(prefs.get("bold_text"), Some("1"));
    assert_eq!(Options::load(&prefs), *st.options(), "the file matches the live options");
}

#[test]
fn focus_and_assistive_touch_switches_persist_and_reload() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Focus");
    assert_eq!(st.page(), PageId::Focus);
    assert!(tap_row(&mut st, &ui, &mut prefs, "Do Not Disturb").is_empty());
    assert_eq!(prefs.get("dnd"), Some("1"));
    back(&mut st, &ui, &mut prefs);
    let list = st.list().unwrap();
    let (g, i) = list.find("Focus").unwrap();
    assert_eq!(list.item(g, i).value, "On");
    tap_row(&mut st, &ui, &mut prefs, "Accessibility");
    assert!(tap_row(&mut st, &ui, &mut prefs, "AssistiveTouch").is_empty());
    assert_eq!(prefs.get("assistive_touch"), Some("1"));
    let again = Settings::new(&ui, &info(), &prefs, 1200, 1920, Rc::new(vec![]));
    assert!(again.options().dnd);
    assert!(again.options().assistive_touch);
    assert_eq!(Options::load(&prefs), *st.options());
}

#[test]
fn auto_lock_picks_persist_and_update_the_display_page() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
    tap_row(&mut st, &ui, &mut prefs, "Auto-Lock");
    assert_eq!(st.page(), PageId::AutoLock);
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Never"), vec![Effect::AutoLock(0)]);
    assert_eq!(prefs.get("auto_lock_ms"), Some("0"));
    let p = st.list().unwrap();
    assert_eq!(p.item(0, 4).accessory, super::super::list::Accessory::Check(true));
    assert_eq!(p.item(0, 2).accessory, super::super::list::Accessory::Check(false));
    back(&mut st, &ui, &mut prefs);
    let d = st.list().unwrap();
    let (g, i) = d.find("Auto-Lock").unwrap();
    assert_eq!(d.item(g, i).value, "Never");
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Auto-Lock"), vec![]);
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "30 seconds"), vec![Effect::AutoLock(30_000)]);
}

#[test]
fn require_password_picks_persist_and_update_the_lock_screen_row() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Lock Screen");
    tap_row(&mut st, &ui, &mut prefs, "Require Password");
    assert_eq!(st.page(), PageId::RequirePassword);
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "1 minute"), vec![]);
    assert_eq!(prefs.get("lock_grace_seconds"), Some("60"));
    assert_eq!(st.options().lock_grace_s, 60);
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Immediately"), vec![]);
    assert_eq!(prefs.get("lock_grace_seconds"), Some("0"));
}

#[test]
fn energy_picks_persist_and_apply_without_restart() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Battery");
    tap_row(&mut st, &ui, &mut prefs, "Energy");
    assert_eq!(st.page(), PageId::Energy);
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Sleep After", 3), vec![Effect::SystemSleepAfter(15)]);
    assert_eq!(prefs.get("system_sleep_minutes"), Some("15"));
    assert_eq!(crate::bare::sleep::system_sleep_after(prefs.get("system_sleep_minutes")), Some(900_000.0));
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Sleep After", 6), vec![Effect::SystemSleepAfter(0)]);
    assert_eq!(crate::bare::sleep::system_sleep_after(prefs.get("system_sleep_minutes")), None);
}

#[test]
fn privacy_offers_open_anyway_for_a_blocked_program_and_the_allow_setting() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    let dir = std::env::temp_dir().join(format!("settings-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("tool");
    std::fs::write(&prog, "#!/bin/sh\n").unwrap();
    st.set_data_dir(&dir);
    tap_row(&mut st, &ui, &mut prefs, "Privacy");
    assert!(st.row_centre(&ui, "Open Anyway").is_none(), "nothing waits");
    assert!(st.row_centre(&ui, "Allow Apps From").is_some());
    back(&mut st, &ui, &mut prefs);
    super::super::launch_gate::write_blocked(&dir, &prog, "\"tool\" cannot be opened because its developer cannot be identified.");
    tap_row(&mut st, &ui, &mut prefs, "Privacy");
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Open Anyway"), vec![]);
    assert!(st.row_centre(&ui, "Open Anyway").is_none(), "the block is lifted");
    let remembered = std::fs::read_to_string(dir.join("launch_overrides.txt")).unwrap();
    assert!(remembered.starts_with(&format!("{}\t", prog.display())), "{remembered:?}");
    // the setting persists and the check follows the tap
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Allow Apps From", 2), vec![]);
    assert_eq!(prefs.get("allow_apps_from"), Some("anywhere"));
    assert_eq!(super::super::launch_gate::load_policy(&prefs, &dir).setting(), bifrost_core::launch_policy::Setting::Anywhere);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Probes that answer from a table instead of the network.
struct FakeProbe {
    resolves: bool,
}

impl bifrost_core::net_probe::Probe for FakeProbe {
    fn link_states(&self) -> Vec<(String, String)> {
        vec![("lo".into(), "unknown".into()), ("wlan0".into(), "up".into())]
    }
    fn global_addresses(&self) -> String {
        "3: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic wlan0\n".into()
    }
    fn resolve(&self, _host: &str, port: u16) -> Vec<std::net::SocketAddr> {
        if self.resolves { vec![std::net::SocketAddr::from(([192, 168, 1, 1], port))] } else { Vec::new() }
    }
    fn connect(&self, _addr: std::net::SocketAddr) -> bool {
        true
    }
}

#[test]
fn network_diagnose_runs_the_probe_off_the_draw_thread_and_shows_the_diagnosis() {
    let ui = super::super::tests::ui();
    // the Network row is the desktop's own network page, so the snapshot has to be there
    let info = Info { desktop_network: Some(super::super::desktop_network::Snapshot::default()), ..info() };
    let (mut st, mut prefs) = settings(&ui, &info);
    st.net_probe = std::sync::Arc::new(FakeProbe { resolves: false });
    tap_row(&mut st, &ui, &mut prefs, "Network");
    assert_eq!(st.page(), PageId::Network);
    assert!(st.row_centre(&ui, "Diagnose").is_some());
    assert!(st.row_centre(&ui, "Name Lookup").is_none(), "no result before a run");
    // tap without settling: the run is on its worker, the page says so
    let (x, y) = st.row_centre(&ui, "Diagnose").unwrap();
    assert_eq!(tap(&mut st, &ui, &mut prefs, x, y), vec![]);
    assert!(st.loads.net_diag.busy() || st.loads.net_diag.get().is_some());
    settle(&mut st, &ui);
    let report = st.loads.net_diag.get().and_then(|r| r.as_ref()).expect("the run landed");
    assert_eq!(report.diagnosis.stage, bifrost_core::net_diagnostic::Stage::Name);
    // the steps and the diagnosis are on the page
    assert!(st.row_centre(&ui, "Name Lookup").is_some());
    assert!(st.row_centre(&ui, "Connection").is_some());
    assert!(st.row_centre(&ui, report.diagnosis.title).is_some());
}

#[test]
fn battery_schedule_adds_and_edits_times_and_arms_the_wake() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Battery");
    tap_row(&mut st, &ui, &mut prefs, "Energy");
    tap_row(&mut st, &ui, &mut prefs, "Schedule");
    assert_eq!(st.page(), PageId::WakeSchedule);
    let fx = tap_row(&mut st, &ui, &mut prefs, "Add Time");
    assert!(matches!(fx.as_slice(), [Effect::WakeSchedule(at)] if *at > 0), "{fx:?}");
    assert_eq!(prefs.get("wake_schedule"), Some("31,420,wake,1"));
    tap_row(&mut st, &ui, &mut prefs, "Repeat");
    assert_eq!(prefs.get("wake_schedule"), Some("96,420,wake,1"));
    tap_row(&mut st, &ui, &mut prefs, "Action");
    assert_eq!(prefs.get("wake_schedule"), Some("96,420,poweron,1"));
    // the time row opens the time-of-day picker; its answer moves the entry and arms the clock again
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Time"), vec![]);
    assert!(st.time_picker.is_some());
    let fx = st.wake_set_minute(&ui, &mut prefs, 0, 8 * 60);
    assert!(matches!(fx.as_slice(), [Effect::WakeSchedule(at)] if *at > 0), "{fx:?}");
    assert_eq!(prefs.get("wake_schedule"), Some("96,480,poweron,1"));
    st.time_picker = None;
    // a fresh Settings reads it back and the Battery page shows it
    let reloaded = Settings::new(&ui, &info(), &prefs, 1200, 1920, Rc::new(vec![]));
    assert_eq!(super::super::wake_page::summary(&reloaded.wake), "1 time");
    // off disarms (0), and Remove deletes the entry
    let fx = tap_row(&mut st, &ui, &mut prefs, "Enabled");
    assert_eq!(fx, vec![Effect::WakeSchedule(0)]);
    let fx = tap_row(&mut st, &ui, &mut prefs, "Remove");
    assert_eq!(fx, vec![Effect::WakeSchedule(0)]);
    assert_eq!(prefs.get("wake_schedule"), Some(""));
}

#[test]
fn screen_saver_picks_persist_and_preview_is_queued_once() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Screen Saver");
    assert_eq!(st.page(), PageId::ScreenSaver);
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Screen Saver", 1), vec![]);
    assert_eq!(prefs.get("screensaver_module"), Some("1"));
    let ten = super::super::screensaver::START_AFTER.iter().position(|(_, min)| *min == 10).unwrap();
    assert_eq!(pick(&mut st, &ui, &mut prefs, "Start After", ten), vec![]);
    assert_eq!(prefs.get("screensaver_minutes"), Some("10"));
    assert!(!st.take_screensaver_preview());
    tap_row(&mut st, &ui, &mut prefs, "Preview");
    assert!(st.take_screensaver_preview(), "Preview asks the shell to start the saver");
    assert!(!st.take_screensaver_preview(), "the request is taken once");
}

#[test]
fn screen_saver_options_are_saved_per_saver() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Screen Saver");
    // Clock is the first saver: its switch and its choices are on the page
    tap_row(&mut st, &ui, &mut prefs, "Show Date");
    assert_eq!(prefs.get("ScreenSaver.Clock.show_date"), Some("false"));
    pick(&mut st, &ui, &mut prefs, "Move Every", 0);
    assert_eq!(prefs.get("ScreenSaver.Clock.move_every"), Some("30s"));
    assert_eq!(prefs.get("ScreenSaver.Colours.speed"), None, "another saver's keys are untouched");
    // the next saver shows its own options
    pick(&mut st, &ui, &mut prefs, "Screen Saver", 1);
    pick(&mut st, &ui, &mut prefs, "Colour Change", 2);
    assert_eq!(prefs.get("ScreenSaver.Colours.speed"), Some("fast"));
    assert_eq!(prefs.get("ScreenSaver.Clock.move_every"), Some("30s"));
}

fn drag_slider(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label_row_above: Option<&str>, g: usize, i: usize, from: f32, to: f32) -> Vec<Effect> {
    let _ = label_row_above;
    let (tx, ty, tw) = st.list().unwrap().track(ui, g, i);
    let mut fx = st.touch(ui, prefs, Event::Down(tx + tw * from, ty), 0.0);
    fx.extend(st.touch(ui, prefs, Event::Move(tx + tw * to, ty + 200.0), 50.0));
    fx.extend(st.touch(ui, prefs, Event::Up(tx + tw * to, ty + 200.0), 100.0));
    fx
}

#[test]
fn sliders_emit_live_effects_and_persist_on_release() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &info());
    tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
    let fx = drag_slider(&mut st, &ui, &mut prefs, None, 0, 0, 0.25, 0.75);
    assert_eq!(fx, vec![Effect::Brightness(0.25), Effect::Brightness(0.75), Effect::Brightness(0.75)]);
    assert_eq!(st.brightness(), 0.75);
    assert_eq!(st.page(), PageId::Display, "dragging never taps");
    // text size snaps to the seven notches and persists on release
    let (g, i) = st.list().unwrap().find("Bold Text").map(|(g, i)| (g, i - 1)).unwrap();
    assert!(drag_slider(&mut st, &ui, &mut prefs, None, g, i, 0.5, 1.0).is_empty());
    assert_eq!(st.options().text_scale, TEXT_SIZES[6]);
    assert_eq!(prefs.get("text_scale"), Some("1.30"));
    drag_slider(&mut st, &ui, &mut prefs, None, g, i, 1.0, 0.0);
    assert_eq!(st.options().text_scale, TEXT_SIZES[0]);
    back(&mut st, &ui, &mut prefs);
    tap_row(&mut st, &ui, &mut prefs, "Sounds");
    // the Output volume slider sits under the output devices
    let g = st.list().unwrap().spec().groups.iter().position(|g| g.header.as_deref() == Some("OUTPUT VOLUME")).expect("Output volume");
    let fx = drag_slider(&mut st, &ui, &mut prefs, None, g, 0, 0.5, 0.0);
    assert_eq!(fx.last(), Some(&Effect::Volume(0.0)));
    assert_eq!(prefs.get("volume"), Some("0.00"));
}

#[test]
fn the_restart_alert_offers_reopen_windows_and_remembers_the_choice() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "General");
    tap_row(&mut st, &ui, &mut prefs, "Restart");
    assert_eq!(st.alert().unwrap().reopen(), Some(true), "on by default");
    let (cx, cy) = st.alert().unwrap().reopen_centre(&ui, 1200, 1920).expect("a checkbox row");
    assert!(tap(&mut st, &ui, &mut prefs, cx, cy).is_empty(), "the checkbox does not confirm");
    assert_eq!(st.alert().unwrap().reopen(), Some(false));
    assert_eq!(prefs.get(super::super::session::REOPEN_PREF), Some("0"));
    let (cx, cy) = st.alert().unwrap().button(&ui, 1200, 1920, AlertHit::Confirm);
    assert_eq!(tap(&mut st, &ui, &mut prefs, cx, cy), vec![Effect::Power(PowerAction::Restart)]);
}

#[test]
fn close_windows_when_quitting_is_a_general_setting() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "General");
    assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), None);
    tap_row(&mut st, &ui, &mut prefs, "Close windows when quitting an application");
    assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), Some("1"));
    tap_row(&mut st, &ui, &mut prefs, "Close windows when quitting an application");
    assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), Some("0"));
}

#[test]
fn forget_network_from_the_detail_page() {
    let ui = super::super::tests::ui();
    let (mut st, mut prefs) = settings(&ui, &wifi_info());
    tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
    tap_row(&mut st, &ui, &mut prefs, "Home");
    assert_eq!(st.page(), PageId::WifiDetail);
    assert_eq!(st.list().unwrap().spec().title, "Home");
    assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Auto-Join"), vec![Effect::Wifi(WifiAction::SetAutoJoin { ssid: "Home".into(), on: false })]);
    tap_row(&mut st, &ui, &mut prefs, "Forget This Network");
    let (cx, cy) = st.alert().unwrap().button(&ui, 1200, 1920, AlertHit::Confirm);
    assert_eq!(tap(&mut st, &ui, &mut prefs, cx, cy), vec![Effect::Wifi(WifiAction::Forget("Home".into()))]);
    settle(&mut st, &ui);
    assert_eq!(st.page(), PageId::Wifi, "back to the list");
}
