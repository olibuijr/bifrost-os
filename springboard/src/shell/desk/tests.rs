#[test]
fn dock_dot_states() {
    use super::{dock_dot, SECONDARY, WHITE};
    assert_eq!(dock_dot(true, false), Some(WHITE));
    assert_eq!(dock_dot(true, true), Some(WHITE));
    assert_eq!(dock_dot(false, true), Some(SECONDARY));
    assert_eq!(dock_dot(false, false), None);
}

use super::*;

#[test]
fn quit_children_are_reaped_and_running_ones_kept() {
    let mut children = std::collections::HashMap::new();
    children.insert("quit".to_string(), std::process::Command::new("true").spawn().unwrap());
    children.insert("running".to_string(), std::process::Command::new("sleep").arg("30").spawn().unwrap());
    // `true` exits at once; give it time to become a zombie
    std::thread::sleep(std::time::Duration::from_millis(500));
    reap_exited(&mut children);
    assert_eq!(children.keys().collect::<Vec<_>>(), vec!["running"], "the zombie is waited for and dropped");
    let mut running = children.remove("running").unwrap();
    let _ = running.kill();
    let _ = running.wait();
}

#[test]
fn a_full_size_app_drags_from_its_empty_toolbar_band_but_not_its_controls_or_lights() {
    let m = Metrics { pt: 1.5, ..metrics() };
    let r = Rect { x: 100, y: 50, w: 900, h: 600 };
    let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    let node = |x: f32, y: f32| bifrost_proto::Node { id: 1, role: bifrost_proto::Role::Button, label: "Back".into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w: 64.0, h: 32.0 }, enabled: true };
    let (band, holes) = zone_of(&w, &m, &[node(220.0, 10.0), node(220.0, 300.0)], true);
    assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified bar is the drag band");
    assert_eq!(holes.len(), 2, "the lights and the one control inside the band");
    assert_eq!(holes[1], [330, 15, 96, 48], "control bounds scale from points to window px");
    assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are not a drag area");
    // a normal window drags from its title bar, except over the three lights: a press there must reach the button
    // (the window-move threshold used to swallow a press on the green one)
    let (title, lights) = zone_of(&w, &m, &[node(220.0, 10.0)], false);
    assert_eq!(title, m.title, "a normal window drags from its title bar only");
    assert_eq!(lights.len(), 1, "the one hole is the traffic lights; the toolbar controls of a normal window are not holes");
    let hole = lights[0];
    for i in 0..3 {
        let l = w.light_rect(&m, i);
        let (x, y) = (l.x - w.r.x, l.y - w.r.y);
        assert!(x >= hole[0] && x + l.w <= hole[0] + hole[2] && y >= hole[1] && y + l.h <= hole[1] + hole[3], "light {i} at {x},{y} is inside the hole {hole:?}");
    }
}

#[test]
fn settings_drags_from_its_empty_toolbar_but_not_from_the_pill_or_search_field() {
    use bifrost_ui::prefs_layout::m as pm;
    let m = Metrics { pt: 1.5, ..metrics() };
    let r = Rect { x: 100, y: 50, w: (740.0 * 1.5) as i32, h: (690.0 * 1.5) as i32 };
    let w = Win { app: AppId::Settings, r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    let (band, holes) = drag_zone(&w, &m, &[]);
    assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "Settings has the full-size toolbar band, not the title-bar one");
    let px = |v: f32| (v * 1.5) as i32;
    let pill_x = pm::SIDEBAR_W + pm::TOOLBAR_X;
    let pill_mid_y = pm::TOOLBAR_Y + pm::TOOLBAR_PILL_H / 2.0;
    // both halves of the pill, including their upper part that the old title band covered
    for (x, y) in [(pill_x + 10.0, pill_mid_y), (pill_x + pm::TOOLBAR_PILL_W - 10.0, pill_mid_y), (pill_x + 10.0, pm::TOOLBAR_Y + 2.0), (pill_x + 50.0, pm::TOOLBAR_Y + 4.0)] {
        assert!(!drags(band, &holes, px(x), px(y)), "({x},{y}) is on the pill");
        let hit = w.hit(&m, (r.x + px(x)) as f32, (r.y + px(y)) as f32);
        assert_eq!(hit, Some(Hit::Content), "({x},{y}) reaches Settings");
    }
    // empty toolbar space right of the pill and above the sidebar search field moves the window
    for (x, y) in [(pill_x + pm::TOOLBAR_PILL_W + 200.0, pill_mid_y), (pill_x + pm::TOOLBAR_PILL_W + 200.0, 4.0), (150.0, 45.0)] {
        assert!(drags(band, &holes, px(x), px(y)), "({x},{y}) drags");
        assert_eq!(w.hit(&m, (r.x + px(x)) as f32, (r.y + px(y)) as f32), Some(Hit::Title), "({x},{y}) is title");
    }
    // the search field and the content below the band are Settings'
    let s = pm::SEARCH_Y + pm::SEARCH_H / 2.0;
    assert_eq!(w.hit(&m, (r.x + px(100.0)) as f32, (r.y + px(s)) as f32), Some(Hit::Content));
    assert_eq!(w.hit(&m, (r.x + px(500.0)) as f32, (r.y + px(200.0)) as f32), Some(Hit::Content));
    // the traffic lights stay lights
    let (cx, cy) = bifrost_ui::prefs_layout::Frame::traffic_light(0);
    assert_eq!(w.hit(&m, (r.x + px(cx)) as f32, (r.y + px(cy)) as f32), Some(Hit::Light(Light::Close)));
    assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are a hole");
}

/// E2E9 4: an extra window opened with `window_flags::FULL_SIZE` (a second TextEdit document) has no shell title row:
/// its body is the whole window under the lights, and its outer size is the size the app asked for. A plain extra
/// window keeps the shell's title bar.
#[test]
fn a_full_size_extra_window_has_no_shell_title_row() {
    let m = Metrics { pt: 1.5, ..metrics() };
    let (doc, plain) = (AppId::ExternalWindow(3, 2), AppId::ExternalWindow(3, 4));
    assert!(!full_size(doc) && !full_size(plain), "an extra window's id alone does not say: the window does");
    let r = Rect { x: 100, y: 50, w: 900, h: 700 };
    let w = Win { app: doc, r, maximized: false, minimized: false, full: false, restore: r, full_size: true };
    let p = Win { app: plain, full_size: false, ..w };
    assert!(w.is_full_size());
    assert!(!p.is_full_size(), "a window that did not ask keeps its title bar");
    assert_eq!(w.title_h(&m), 0);
    assert_eq!(p.title_h(&m), m.title);
    assert_eq!(w.body(&m), r, "the app draws the whole window, its own title row included");
    assert_eq!(extra_window_px(doc, true, (600.0, 400.0), 1.5, &m).1, 600, "no title bar is added to the asked height");
    assert_eq!(extra_window_px(plain, false, (600.0, 400.0), 1.5, &m).1, 600 + m.title);
}

/// E2E10: a full-size extra window takes its flag from its app host's window table when the desk opens it, so its
/// unified title band (no shell title bar) is hit right away, before any later sync; a plain one keeps the title bar.
#[test]
fn a_full_size_extra_window_hits_its_title_band_right_after_it_opens() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, _theirs) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    let mut host = apphost::AppHost::with_channel("fixture", mine, None, cfg);
    host.windows.open("fixture", 2, "Untitled 2", 600.0, 400.0).unwrap();
    assert!(host.windows.set_full_size("fixture", 2, true));
    host.windows.open("fixture", 4, "Plain", 300.0, 200.0).unwrap();
    sh.ext.push(Some(host));
    let slot = (sh.ext.len() - 1) as u8;
    let (doc, plain) = (AppId::ExternalWindow(slot, 2), AppId::ExternalWindow(slot, 4));
    sh.desk.as_mut().unwrap().wins.clear();
    sh.desk_sync_extra_windows();
    let m = sh.desk_metrics();
    let d = sh.desk.as_ref().unwrap();
    let w = d.wins[d.index_of(doc).expect("the desk opened the document window")];
    let p = d.wins[d.index_of(plain).expect("and the plain one")];
    assert!(w.full_size && w.is_full_size() && !p.is_full_size());
    assert_eq!(w.title_h(&m), 0, "no shell title row");
    // below the shell's title bar height but inside the unified band (52 pt), past the traffic lights
    let (x, y) = ((w.r.x + w.r.w / 2) as f32, w.r.y as f32 + m.title as f32 + 2.0);
    assert!(y < w.r.y as f32 + UNIFIED_BAR_PT * m.pt);
    assert_eq!(w.hit(&m, x, y), Some(Hit::Title), "the unified band moves the window");
    assert_eq!(w.hit(&m, x, w.r.y as f32 + UNIFIED_BAR_PT * m.pt + 4.0), Some(Hit::Content));
    let (px_, py) = ((p.r.x + p.r.w / 2) as f32, p.r.y as f32 + m.title as f32 + 2.0);
    assert_eq!(p.hit(&m, px_, py), Some(Hit::Content), "a titled window's body starts under its title bar");
}

#[test]
fn the_app_store_is_a_full_size_window_whose_search_field_and_back_control_take_the_press() {
    // a sidebar app: no title bar, the sidebar runs to the top under the traffic lights (N8)
    assert!(full_size(AppId::AppStore));
    let m = Metrics { pt: 1.5, ..metrics() };
    assert_eq!(title_of(AppId::AppStore, &m), 0, "no title bar above the sidebar");
    let r = Rect { x: 100, y: 50, w: 1500, h: 1000 };
    let w = Win { app: AppId::AppStore, r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    assert_eq!(w.body(&m), r, "the store draws the whole window");
    let px = |v: f32| (v * 1.5) as i32;
    let side = ((200.0 * 1.5) as i32).min(r.w / 3);
    // no page open: no back control, so the whole band over the pane drags
    super::super::store::set_back_control_pt(0.0);
    let (band, holes) = drag_zone(&w, &m, &[]);
    assert_eq!(holes.len(), 2, "the lights and the sidebar search field");
    assert!(drags(band, &holes, side + px(20.0), px(26.0)), "with no back control shown its place moves the window");
    // a page is open: its back control (90 pt drawn) takes the press, and only it
    super::super::store::set_back_control_pt(90.0);
    let (band, holes) = drag_zone(&w, &m, &[]);
    assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified toolbar band drags");
    assert_eq!(holes.len(), 3, "the lights, the sidebar search field and the back control");
    assert!(drags(band, &holes, side + px(100.0), px(26.0)), "past the back control the band drags");
    // the search capsule's top edge is inside the band and takes the press; the empty band over the pane drags
    assert!(!drags(band, &holes, px(60.0), px(48.0)), "the search field is not a drag area");
    assert!(!drags(band, &holes, side + px(20.0), px(26.0)), "the back control is not a drag area");
    assert!(drags(band, &holes, side + px(400.0), px(26.0)), "the empty band over the pane moves the window");
    assert_eq!(w.hit(&m, (r.x + side + px(20.0)) as f32, (r.y + px(26.0)) as f32), Some(Hit::Content));
    assert_eq!(w.hit(&m, (r.x + side + px(400.0)) as f32, (r.y + px(26.0)) as f32), Some(Hit::Title));
    let (cx, cy) = bifrost_ui::prefs_layout::Frame::traffic_light(0);
    assert_eq!(w.hit(&m, (r.x + px(cx)) as f32, (r.y + px(cy)) as f32), Some(Hit::Light(Light::Close)));
    super::super::store::set_back_control_pt(0.0);
}

fn kit_node(role: bifrost_proto::Role, label: &str, x: f32, y: f32, w: f32, h: f32) -> bifrost_proto::Node {
    bifrost_proto::Node { id: 1, role, label: label.into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w, h }, enabled: true }
}

/// The press the shell tests on a full-size window: inside the band, right of the lights, outside every hole.
fn drags(band: i32, holes: &[[i32; 4]], x: i32, y: i32) -> bool {
    y < band && !holes.iter().any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
}

#[test]
fn empty_toolbar_space_drags_under_a_toolbar_container_and_only_interactive_nodes_block() {
    use bifrost_proto::Role;
    let m = Metrics { pt: 1.5, ..metrics() };
    let r = Rect { x: 100, y: 50, w: 900, h: 600 };
    let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    // what the kit publishes for a unified toolbar: the "Toolbar" and "Sidebar" containers, the title labels, two
    // buttons and a search field, in points
    let nodes = [
        kit_node(Role::List, "Toolbar", 220.0, 10.0, 520.0, 32.0),
        kit_node(Role::List, "Sidebar", 0.0, 0.0, 200.0, 600.0),
        kit_node(Role::StaticText, "Notes", 230.0, 12.0, 120.0, 28.0),
        kit_node(Role::Row, "All Notes", 0.0, 40.0, 200.0, 30.0),
        kit_node(Role::Window, "Sheet", 0.0, 0.0, 900.0, 600.0),
        kit_node(Role::Button, "Grid", 600.0, 10.0, 32.0, 32.0),
        kit_node(Role::Button, "More", 640.0, 10.0, 32.0, 32.0),
        kit_node(Role::TextField, "Search", 680.0, 10.0, 100.0, 32.0),
    ];
    let (band, holes) = zone_of(&w, &m, &nodes, true);
    assert_eq!(holes.len(), 4, "the lights plus the two buttons and the search field, nothing else");
    // empty toolbar space between the title block and the buttons, at the height a capsule would be hit
    assert!(drags(band, &holes, (400.0 * 1.5) as i32, (20.0 * 1.5) as i32), "empty space drags");
    assert!(drags(band, &holes, (400.0 * 1.5) as i32, (2.0 * 1.5) as i32), "the top strip drags");
    assert!(drags(band, &holes, (100.0 * 1.5) as i32, (20.0 * 1.5) as i32), "the sidebar's top drags past the lights");
    assert!(drags(band, &holes, (260.0 * 1.5) as i32, (20.0 * 1.5) as i32), "the title label drags");
    for (x, y) in [(616.0, 26.0), (656.0, 26.0), (730.0, 26.0)] {
        assert!(!drags(band, &holes, (x * 1.5) as i32, (y * 1.5) as i32), "({x},{y}) is a control");
    }
    assert!(!drags(band, &holes, (400.0 * 1.5) as i32, band + 1), "below the band is the app");
    assert!(takes_press(Role::SegmentedControl) && takes_press(Role::MenuItem) && !takes_press(Role::ProgressIndicator));
}

#[test]
fn a_store_app_with_a_full_size_manifest_gets_the_52_pt_band() {
    let m = Metrics { pt: 2.0, ..metrics() };
    let r = Rect { x: 0, y: 0, w: 1200, h: 800 };
    let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    // `drag_zone` asks `full_size`, which is true for an installed app whose manifest says `window = "full-size"`
    // (the shipped Notes and Mail manifests do); `zone_of` is what it computes for such a window
    for path in ["apps/notes-desktop/manifest.toml", "apps/mail-desktop/manifest.toml"] {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
        assert!(apphost::Manifest::parse(&text).unwrap().full_size_content(), "{path} asks for a full-size window");
    }
    let plain = apphost::Manifest::parse("id = \"is.x.app\"\nname = \"X\"\nexec = \"x\"\n").unwrap();
    assert!(!plain.full_size_content(), "a manifest without the style keeps its title bar");
    let nodes = [kit_node(bifrost_proto::Role::List, "Toolbar", 100.0, 8.0, 400.0, 36.0), kit_node(bifrost_proto::Role::Button, "New", 560.0, 8.0, 36.0, 36.0)];
    let (band, holes) = zone_of(&w, &m, &nodes, true);
    assert_eq!(band, 104, "52 pt at 2 px per point");
    assert!(drags(band, &holes, 600, 40), "empty toolbar space moves the window");
    assert!(!drags(band, &holes, 1130, 40), "the button does not");
    let (title, none) = zone_of(&w, &m, &nodes, false);
    assert_eq!((title, none.len()), (m.title, 1), "a window without the style drags from its title bar, but not from the lights");
    assert!(!drags(title, &none, 12 + 2 * (m.light + m.light_gap) + m.light / 2, m.title / 2), "the green light is not a drag");
    assert!(drags(title, &none, 600, m.title / 2), "past the lights the title bar drags");
}

#[test]
fn light_glyphs_draw_dark_marks_inside_the_light() {
    for i in 0..3 {
        let mut px = vec![WHITE; 20 * 20];
        let mut s = Surface { w: 20, h: 20, stride: 20, px: &mut px, y0: 0 };
        draw_light_glyph(&mut s, &Rect { x: 4, y: 4, w: 12, h: 12 }, i, false, 1.0);
        let dark = px.iter().filter(|p| **p != WHITE).count();
        assert!(dark > 4, "glyph {i} drew {dark} px");
        assert!(px.iter().any(|p| (p & 0xff) < 110), "glyph {i} has a clearly dark core");
        assert_eq!(px[0], WHITE, "nothing outside the light");
    }
}

#[test]
fn the_exit_full_screen_glyph_draws_two_marks_inside_the_light_that_meet_at_the_centre() {
    let mut px = vec![WHITE; 20 * 20];
    let mut s = Surface { w: 20, h: 20, stride: 20, px: &mut px, y0: 0 };
    draw_exit_full_screen_glyph(&mut s, &Rect { x: 4, y: 4, w: 12, h: 12 }, 1.0);
    let dark = |x: usize, y: usize| px[y * 20 + x] != WHITE;
    let (tl, br) = ((4..10).flat_map(|y| (4..10).map(move |x| (x, y))).filter(|&(x, y)| dark(x, y)).count(), (10..16).flat_map(|y| (10..16).map(move |x| (x, y))).filter(|&(x, y)| dark(x, y)).count());
    assert!(tl > 3 && br > 3, "one arrowhead each side of the centre: {tl} and {br} px");
    let (tr, bl) = ((4..10).flat_map(|y| (10..16).map(move |x| (x, y))).filter(|&(x, y)| dark(x, y)).count(), (10..16).flat_map(|y| (4..10).map(move |x| (x, y))).filter(|&(x, y)| dark(x, y)).count());
    assert_eq!((tr, bl), (0, 0), "nothing on the other diagonal");
    assert_eq!(px[0], WHITE, "nothing outside the light");
    let mut zoom = vec![WHITE; 20 * 20];
    draw_light_glyph(&mut Surface { w: 20, h: 20, stride: 20, px: &mut zoom, y0: 0 }, &Rect { x: 4, y: 4, w: 12, h: 12 }, 2, true, 1.0);
    assert_ne!(px, zoom, "not the maximized window's hollow square");
}

fn metrics() -> Metrics {
    Metrics { w: 1920, h: 1080, title: 30, menubar: 28, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_left: 0, dock_right: 0, dock_bottom: 100, pt: 1.0 }
}

/// 0.10.39: Mail was full screen at a restart and came back at 0,35,2560,1309 (the screen held to the work area). A
/// full-screen window is saved with its way back, a zoomed one with its frame from before the zoom.
#[test]
fn a_full_screen_or_zoomed_window_is_saved_with_the_frame_it_goes_back_to() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
    let before = d.wins[i].r;
    assert_eq!(d.saved_frame(i), Some((before, false)), "a normal window: its own frame");
    assert!(d.set_full_screen(&m, i, true));
    assert_eq!(d.wins[i].r, Desk::screen_rect(&m));
    assert_eq!(d.saved_frame(i), Some((before, false)), "full screen: the frame before it, not the screen");
    assert!(d.set_full_screen(&m, i, false));
    d.toggle_zoom(&m, i);
    assert_eq!(d.saved_frame(i), Some((before, true)), "zoomed: the frame before the zoom, zoomed on the way back");
    assert!(d.set_full_screen(&m, i, true));
    assert_eq!(d.saved_frame(i), Some((before, true)), "full screen from a zoom: back zoomed, over the same frame");
    assert_eq!(d.saved_frame(i + 1), None);
    // the restore puts it back as the session restore does (`place`, then the zoom): where it was before, zoomed
    let mut e = Desk::new();
    let k = e.open(&m, AppId::ExternalWindow(0, 4), (300, 200));
    let (r, zoomed) = d.saved_frame(i).unwrap();
    e.place(&m, k, r);
    if zoomed {
        e.toggle_zoom(&m, k);
    }
    assert!(e.wins[k].maximized && e.wins[k].r == Desk::area(&m));
    e.toggle_zoom(&m, k);
    assert_eq!(e.wins[k].r, before, "un-zooming the restored window goes back to the frame from before");
}

/// 0.10.39: a client window whose app frame is smaller than the window (Mail at 2560x1309 with a 1200x860 frame) is
/// drawn at the size its frame covers, never with black past it; a frame that covers the body keeps the window's size.
#[test]
fn a_client_window_is_presented_at_the_size_its_frame_covers() {
    let m = metrics();
    let r = Rect { x: 100, y: 100, w: 1600, h: 900 };
    for full_size in [false, true] {
        let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r, full_size };
        let top = w.title_h(&m);
        let b = w.body(&m);
        assert_eq!(presented_size(&w, &m, Some((1200, 860))), (1200, top + 860), "the frame and the title bar ({full_size})");
        assert_eq!(presented_size(&w, &m, Some((b.w as usize, b.h as usize))), (1600, 900), "a frame at the body size: the whole window");
        assert_eq!(presented_size(&w, &m, Some((2000, 1000))), (1600, 900), "a larger frame is clipped to the window, not grown");
        assert_eq!(presented_size(&w, &m, Some((2000, 400))), (1600, top + 400), "only the side the frame falls short on shrinks");
        assert_eq!(presented_size(&w, &m, None), (1600, 900), "no frame yet: the window's frame");
        let full = Win { full: true, r: Desk::screen_rect(&m), ..w };
        assert_eq!(presented_size(&full, &m, Some((1200, 860))), (1920, 1080), "a full-screen window keeps the screen");
    }
}

/// xdg-shell forbids a maximized surface committing smaller than its configured size, and the window server sized a
/// tile: a zoomed or a tiled window keeps its whole frame while the app's frame is smaller.
#[test]
fn a_zoomed_or_tiled_window_is_presented_at_its_whole_frame() {
    let m = metrics();
    let a = Desk::area(&m);
    let r = Rect { x: 100, y: 100, w: 1200, h: 800 };
    let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    assert_eq!(presented_size(&w, &m, Some((600, 400))), (600, w.title_h(&m) + 400), "a normal window: what its frame covers");
    let zoomed = Win { maximized: true, r: a, ..w };
    assert_eq!(presented_size(&zoomed, &m, Some((600, 400))), (a.w, a.h), "zoomed: the whole frame");
    for tile in [Tile::Left, Tile::Right, Tile::Top, Tile::Bottom, Tile::TopLeft, Tile::BottomRight, Tile::Center, Tile::Fill] {
        let tiled = Win { r: tile_rect(a, tile), ..w };
        assert_eq!(presented_size(&tiled, &m, Some((300, 200))), (tiled.r.w, tiled.r.h), "tiled {tile:?}: the whole frame");
    }
}

/// The window cache holds the pixels at the size they were presented at: while a client's frame is smaller than its
/// window the cache is at that smaller size, and it is reused for the next frame (it was compared with the window's
/// own frame, never matched, and the window was drawn afresh on every desk frame).
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_window_cached_at_its_presented_size_is_reused() {
    let m = metrics();
    let r = Rect { x: 100, y: 100, w: 1600, h: 900 };
    let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
    let presented = presented_size(&w, &m, Some((1200, 860)));
    assert_ne!(presented, (r.w, r.h));
    let cached = CachedWindow {
        width: presented.0 as usize, height: presented.1 as usize, pixels: vec![0u32; presented.0 as usize * presented.1 as usize].into(),
        focused: false, maximized: false, hover: false, drawn: 0.0, look: 0,
    };
    assert!(cached.sized(presented), "the next frame at the same app frame reuses the cache");
    assert!(!cached.sized((r.w, r.h)), "the window's own frame is not the size the pixels were presented at");
    let b = w.body(&m);
    assert!(!cached.sized(presented_size(&w, &m, Some((b.w as usize, b.h as usize)))), "the app drew at the window's size: drawn afresh");
}

#[test]
fn closing_a_full_screen_window_forgets_that_it_came_from_a_zoom() {
    let m = metrics();
    let app = AppId::ExternalWindow(0, 4);
    let mut d = Desk::new();
    let i = d.open(&m, app, (500, 400));
    d.toggle_zoom(&m, i);
    assert!(d.wins[i].maximized);
    assert!(d.set_full_screen(&m, i, true));
    assert_eq!(d.full_from_zoom, vec![app]);
    d.close(i);
    assert!(d.full_from_zoom.is_empty(), "the entry goes with the window");
    let i = d.open(&m, app, (500, 400));
    let before = d.wins[i].r;
    assert!(d.set_full_screen(&m, i, true));
    assert!(d.set_full_screen(&m, i, false));
    assert!(!d.wins[i].maximized, "the new window returns to its own frame, not a zoom");
    assert_eq!(d.wins[i].r, before);
}

#[test]
fn an_extra_window_is_never_opened_under_the_window_minimum() {
    let m = metrics();
    let app = AppId::ExternalWindow(0, 4);
    assert_eq!(extra_window_px(app, false, (0.1, 0.1), 1.0, &m), (m.min_w, m.min_h + m.title));
    assert_eq!(extra_window_px(app, false, (f32::NAN, -5.0), 1.0, &m), (m.min_w, m.min_h + m.title));
    assert_eq!(extra_window_px(app, false, (600.0, 400.0), 1.5, &m), (900, 600 + m.title));
}

#[test]
fn a_window_collapses_to_its_title_bar_and_expands_to_the_height_it_had() {
    let m = metrics();
    let app = AppId::ExternalWindow(0, 4);
    let mut d = Desk::new();
    let i = d.open(&m, app, (300, 400));
    let h = d.wins[i].r.h;
    d.set_collapsed(&m, i, true);
    assert_eq!(d.wins[i].r.h, title_of(app, &m));
    assert!(d.is_collapsed(app));
    d.set_collapsed(&m, i, true);
    assert_eq!(d.wins[i].r.h, title_of(app, &m), "collapsing twice keeps the height it had");
    d.set_collapsed(&m, i, false);
    assert_eq!(d.wins[i].r.h, h);
    assert!(!d.is_collapsed(app));
}

#[test]
fn a_floating_extra_window_is_stacked_last_while_its_app_keeps_its_other_windows_normal() {
    let m = metrics();
    let (a, b) = (AppId::ExternalWindow(0, 1), AppId::ExternalWindow(0, 2));
    let mut d = Desk::new();
    d.open(&m, a, (200, 200));
    d.open(&m, b, (200, 200));
    d.sync_levels(vec![a]);
    assert_eq!(d.wins.last().map(|w| w.app), Some(a), "the floating window is above the normal ones");
    d.sync_levels(vec![]);
    assert!(!d.is_floating(a), "dropping the level puts the window back in the normal band");
}

#[test]
fn closing_a_window_releases_its_buffers() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::Settings, (900, 600));
    #[cfg(feature = "desktop-wayland")]
    d.body_cache.insert(AppId::Settings, vec![0u32; 1024]);
    d.icon_cache.borrow_mut().insert(DockIconKey { owner: DockIconOwner::App(0), generation: 0, size: 1 }, Image { w: 1, h: 1, px: vec![0; 1] });
    d.close(i);
    #[cfg(feature = "desktop-wayland")]
    assert!(d.body_cache.is_empty());
    assert!(d.icon_cache.borrow().is_empty());
}

/// A desktop shell with window capture offered and Settings open; its window index.
#[cfg(feature = "desktop-wayland")]
fn desk_with_one_window() -> (Shell, usize) {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    sh.shots.set_available(true);
    sh.desk_open(AppId::Settings, 100.0);
    let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).expect("the window is open");
    (sh, i)
}

/// The centre pixel of the minimized window's thumbnail frame in the Dock, as the Dock draws it at `t` ms.
#[cfg(feature = "desktop-wayland")]
fn minimized_slot_centre(sh: &Shell, wi: usize, t: f64) -> u32 {
    let m = sh.desk_metrics();
    let (list, bar0, rects0) = sh.desk_dock(&m);
    let (_, slid) = sh.dock_slid(bar0, &rects0, t / 1000.0);
    let rects = sh.dock_magnified(&slid, t / 1000.0);
    let k = list.iter().position(|&e| crate::shell::desktop_dock::dock_min_index(e) == Some(wi)).expect("the minimized window has a slot");
    let win = &sh.desk.as_ref().unwrap().wins[wi];
    let frame = thumbnail_frame(rects[k], win.r.w, win.r.h);
    let (w, h) = (m.w as usize, m.h as usize);
    let mut px = vec![0u32; w * h];
    sh.draw_desk_dock(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &m, t);
    px[(frame.y + frame.h / 2) as usize * w + (frame.x + frame.w / 2) as usize]
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn minimizing_a_window_asks_once_for_its_picture() {
    let (mut sh, i) = desk_with_one_window();
    sh.desk_minimize_window(i, 200.0);
    let reqs = sh.take_capture_requests();
    let id = win_id(AppId::Settings).unwrap();
    assert_eq!(reqs.len(), 1, "one request");
    assert_eq!(reqs[0].source, crate::shell::window_shots::Source::Surface(id), "named by the shell's own window");
    assert_eq!(reqs[0].max, DOCK_SHOT_BOUND);
    assert_eq!(reqs[0].since, 0, "a whole picture, not an unchanged answer");
    assert!(sh.take_capture_requests().is_empty(), "and no more while it stays minimized");
    sh.shots_tick(1000.0);
    sh.shots_tick(3000.0);
    assert!(sh.take_capture_requests().is_empty(), "the Dock never refreshes it");
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn the_minimized_slot_shows_the_windows_picture_inside_the_thumbnail_frame() {
    let (mut sh, i) = desk_with_one_window();
    sh.desk_minimize_window(i, 200.0);
    // the fallback first: the icon in the frame, no picture
    assert!(sh.dock_picture(AppId::Settings).is_none());
    let icon_centre = minimized_slot_centre(&sh, i, 5000.0);
    let token = sh.take_capture_requests()[0].token;
    let red = crate::gfx::pack(250, 10, 10, 255);
    sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Ready { image: Image { w: 64, h: 40, px: vec![red; 64 * 40] }, serial: 1 })], 300.0);
    assert!(sh.dock_picture(AppId::Settings).is_some());
    let picture_centre = minimized_slot_centre(&sh, i, 5000.0);
    assert_ne!(picture_centre, icon_centre, "the slot no longer shows the icon");
    assert_eq!(picture_centre & 255, 250, "it shows the window's own pixels");
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn a_failed_capture_leaves_the_icon_in_the_frame() {
    let (mut sh, i) = desk_with_one_window();
    sh.desk_minimize_window(i, 200.0);
    let before = minimized_slot_centre(&sh, i, 5000.0);
    let token = sh.take_capture_requests()[0].token;
    sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Failed)], 300.0);
    assert!(sh.dock_picture(AppId::Settings).is_none());
    assert_eq!(minimized_slot_centre(&sh, i, 5000.0), before, "the same icon-in-frame slot as without window capture");
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn without_window_capture_minimizing_asks_for_nothing() {
    let (mut sh, i) = desk_with_one_window();
    sh.shots.set_available(false);
    sh.desk_minimize_window(i, 200.0);
    assert!(sh.take_capture_requests().is_empty());
    assert!(sh.dock_picture(AppId::Settings).is_none());
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn a_closed_windows_picture_is_freed() {
    let (mut sh, i) = desk_with_one_window();
    sh.desk_minimize_window(i, 200.0);
    let token = sh.take_capture_requests()[0].token;
    sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Ready { image: Image { w: 8, h: 8, px: vec![0xff00_0000; 64] }, serial: 1 })], 300.0);
    assert_eq!(sh.shots.held(), 1);
    sh.desk.as_mut().unwrap().close(i);
    sh.shots_tick(10_000.0);
    assert_eq!(sh.shots.held(), 0, "nothing is kept for a window that is gone");
}

#[test]
fn the_dock_icon_cache_is_keyed_by_app_and_generation_not_by_pixel_address() {
    let d = Desk::new();
    let solid = |c: u32| Image { w: 2, h: 2, px: vec![c; 4] };
    let (red, blue) = (solid(0xffff_0000), solid(0xff00_00ff));
    let key = |i: usize, generation: u32| DockIconKey { owner: DockIconOwner::App(i), generation, size: 2 };
    let pick = |k: DockIconKey, src: &Image| d.with_scaled_icon(k, src, 2, 2, |im| im.px[0]);
    assert_eq!(pick(key(1, 0), &red), 0xffff_0000);
    assert_eq!(pick(key(2, 0), &blue), 0xff00_00ff);
    // one buffer standing in for two apps (a freed buffer's address reused) never shares a slot
    assert_eq!(pick(key(3, 0), &blue), 0xff00_00ff);
    assert_eq!(pick(key(1, 0), &red), 0xffff_0000, "slot 1 keeps its own picture");
    // the icons were rebuilt: the new generation draws the new picture, whatever the old one was
    assert_eq!(pick(key(1, 1), &blue), 0xff00_00ff);
    // a Linux client is its own owner, never an `apps()` entry
    let linux = DockIconKey { owner: DockIconOwner::Linux("firefox".into()), generation: 0, size: 2 };
    assert_eq!(pick(linux, &red), 0xffff_0000);
}

#[test]
fn a_full_dock_icon_cache_keeps_only_the_current_generation() {
    let d = Desk::new();
    let img = Image { w: 2, h: 2, px: vec![0xff11_2233; 4] };
    for i in 0..48 {
        d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(i), generation: 0, size: 2 }, &img, 2, 2, |_| ());
    }
    // 48 entries do not evict. The first generation-1 insert makes 49; the second finds 49, keeps only the
    // generation-1 entry and adds its own.
    d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(0), generation: 1, size: 2 }, &img, 2, 2, |_| ());
    d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(1), generation: 1, size: 2 }, &img, 2, 2, |_| ());
    let cache = d.icon_cache.borrow();
    assert_eq!(cache.len(), 2);
    assert!(cache.keys().all(|k| k.generation == 1));
}

#[test]
fn a_dock_capsule_that_changed_or_hid_vacates_its_old_glass_shape() {
    let sh = crate::shell::tests::desktop_shell();
    let capsule = |x: i32, w: i32| Rect { x, y: 900, w, h: 80 };
    let vacated = || sh.dock_glass_vacated.take();
    sh.note_dock_glass(Some(capsule(100, 800)));
    assert!(vacated().is_empty(), "the first capsule leaves nothing behind");
    sh.note_dock_glass(Some(capsule(100, 800)));
    assert!(vacated().is_empty(), "an unchanged capsule leaves nothing behind");
    sh.note_dock_glass(Some(capsule(150, 700)));
    assert_eq!(vacated(), vec![(100, 900, 800, 80)], "a Dock that lost icons vacates its wider capsule");
    sh.note_dock_glass(None);
    assert_eq!(vacated(), vec![(150, 900, 700, 80)], "a hidden Dock vacates its capsule");
    sh.note_dock_glass(None);
    assert!(vacated().is_empty());
}

#[test]
fn a_minimised_thumbnail_takes_the_windows_aspect_inside_its_slot() {
    let slot = Rect { x: 10, y: 20, w: 48, h: 48 };
    let inside = |f: Rect| f.x >= slot.x && f.y >= slot.y && f.x + f.w <= slot.x + slot.w && f.y + f.h <= slot.y + slot.h;
    let wide = thumbnail_frame(slot, 1600, 900);
    assert_eq!((wide.w, wide.h), (48, 27));
    assert_eq!(wide.x + wide.w / 2, slot.x + slot.w / 2, "centred across");
    assert!((wide.y + wide.h / 2 - (slot.y + slot.h / 2)).abs() <= 1, "centred down");
    let tall = thumbnail_frame(slot, 500, 1000);
    assert_eq!((tall.w, tall.h), (24, 48));
    let extreme = thumbnail_frame(slot, 4000, 100);
    assert_eq!((extreme.w, extreme.h), (48, 24), "the aspect is clamped");
    let none = thumbnail_frame(slot, 0, 0);
    assert_eq!((none.w, none.h), (48, 48), "a window without a size gets a square");
    assert!([wide, tall, extreme, none].into_iter().all(inside));
}

#[test]
fn opening_the_same_app_raises_instead_of_duplicating() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    d.open(&m, AppId::Photos, (900, 600));
    let i = d.open(&m, AppId::Settings, (900, 600));
    assert_eq!(d.wins.len(), 2);
    assert_eq!(i, 1);
    assert_eq!(d.wins[1].app, AppId::Settings);
}

#[test]
fn windows_open_inside_the_work_area_and_cascade() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    d.open(&m, AppId::Photos, (900, 600));
    let a = Desk::area(&m);
    for w in &d.wins {
        assert!(w.r.x >= a.x && w.r.y >= a.y && w.r.x + w.r.w <= a.x + a.w && w.r.y + w.r.h <= a.y + a.h, "{:?}", w.r);
    }
    assert_ne!(d.wins[0].r, d.wins[1].r);
}

/// F4: a viewer opens beside the main window, or over its trailing side, never over the leading list column; each
/// later viewer cascades down and right of the one before it, and every one stays inside the work area.
#[test]
fn a_cascaded_window_leaves_the_main_windows_leading_side_in_view() {
    let a = Rect { x: 0, y: 28, w: 1920, h: 952 };
    let step = 30;
    let narrow = Rect { x: 100, y: 100, w: 800, h: 600 };
    assert_eq!(cascade_origin(a, narrow, true, (560, 640), step), (900, 100), "beside the main window when it fits");
    let wide = Rect { x: 200, y: 80, w: 1500, h: 800 };
    let (x, y) = cascade_origin(a, wide, true, (560, 640), step);
    assert_eq!((x, y), (200 + 1500 - 560, 80 + step), "over the trailing side, one title bar down");
    assert!(x > wide.x + wide.w / 2, "the leading half (sidebar and list) stays in view");
    let first = Rect { x, y, w: 560, h: 640 };
    assert_eq!(cascade_origin(a, first, false, (560, 640), step), (x + step, y + step), "the next one cascades from the first");
    let corner = Rect { x: 1800, y: 900, w: 560, h: 640 };
    let (cx, cy) = cascade_origin(a, corner, false, (560, 640), step);
    assert!(cx + 560 <= a.x + a.w && cy + 640 <= a.y + a.h, "kept inside the work area: {cx},{cy}");
}

/// E2E11 5: a second document window as large as the main window (Cmd+N in a text editor) cascades one title bar
/// down and right of it, not straight below it, whether or not it would fit beside it.
#[test]
fn a_document_window_as_wide_as_the_main_window_cascades_down_and_right() {
    let a = Rect { x: 0, y: 28, w: 1920, h: 952 };
    let step = 30;
    let main = Rect { x: 400, y: 200, w: 900, h: 600 };
    assert_eq!(cascade_origin(a, main, true, (900, 600), step), (430, 230), "down and right by one title bar");
    assert_eq!(cascade_origin(a, main, true, (890, 600), step), (430, 230), "a few px narrower is the same size");
    let narrow = Rect { x: 100, y: 100, w: 600, h: 500 };
    assert_eq!(cascade_origin(a, narrow, true, (600, 500), step), (130, 130), "it fits beside, but cascades as a document");
}

#[test]
fn a_restored_frame_is_kept_inside_the_work_area() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::Photos, (900, 600));
    let a = Desk::area(&m);
    d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
    assert_eq!(d.wins[i].r, Rect { x: 200, y: 150, w: 800, h: 500 }, "a sane frame comes back as saved");
    d.place(&m, i, Rect { x: 99_999, y: -500, w: 99_999, h: 99_999 });
    let r = d.wins[i].r;
    assert!(r.x >= a.x && r.y >= a.y && r.x + r.w <= a.x + a.w && r.y + r.h <= a.y + a.h, "{r:?}");
    assert!(!d.wins[i].maximized);
}

#[test]
fn hide_puts_away_a_whole_app_and_raising_it_brings_it_back() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Photos, (900, 600));
    d.open(&m, AppId::External(3), (900, 600));
    d.open(&m, AppId::ExternalWindow(3, 7), (400, 300));
    d.open(&m, AppId::Terminal, (900, 600));
    assert!(d.hide_app(AppId::External(3)), "the app had windows to hide");
    assert!(d.is_hidden(AppId::External(3)) && d.is_hidden(AppId::ExternalWindow(3, 7)), "its extra window goes with it");
    assert!(!d.is_hidden(AppId::Photos) && !d.is_hidden(AppId::Terminal));
    assert!(!d.hide_app(AppId::External(3)), "nothing left to hide");
    assert!(d.others_shown(Some(AppId::Terminal)), "Photos is still shown");
    // raising one of its windows shows the whole app again
    let i = d.index_of(AppId::External(3)).unwrap();
    d.raise(i);
    assert!(!d.any_hidden(), "the extra window came back too");
    assert!(d.hidden.is_empty());
}

/// E2E N2: after Cmd+H the Dock shows Mail again with its viewer over the main window, as it was when hidden, so the
/// viewer keeps the keyboard and Cmd+W closes it; the Window menu choosing the main window still brings that one up.
#[test]
fn a_hidden_app_comes_back_with_its_front_window_in_front() {
    let m = metrics();
    let mut d = Desk::new();
    let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
    d.open(&m, main, (900, 600));
    d.open(&m, viewer, (500, 400));
    d.open(&m, AppId::Terminal, (900, 600));
    assert!(d.hide_app(main));
    let i = d.open(&m, main, (900, 600));
    assert_eq!(d.wins[i].app, viewer, "the window that was in front comes back in front");
    assert_eq!(d.front().map(|w| w.app), Some(viewer));
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, main, viewer], "over the other apps, in their own order");
    assert!(d.hidden.is_empty() && !d.any_hidden());
    assert!(d.hide_app(main));
    let i = d.index_of(main).unwrap();
    let at = d.raise(i);
    assert_eq!(d.wins[at].app, main, "a raise of one window of a hidden app brings that window up");
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, viewer, main]);
}

/// E2E N7: a window the restored session put somewhere maps there, so the frame the shell keeps is the one on
/// screen and the window server never cascades it later (a Dock raise moved Terminal from 768,352 to 880,465).
#[test]
fn a_window_placed_on_purpose_maps_where_it_was_placed() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::Terminal, (900, 600));
    assert!(!d.placed.contains(&AppId::Terminal), "a new window is cascaded by the window server");
    let a = Desk::area(&m);
    d.place(&m, i, Rect { x: a.x + 100, y: a.y + 50, w: 900, h: 600 });
    assert_eq!(origin_hint(&d.placed, AppId::Terminal, d.wins[i].r.x, d.wins[i].r.y), Some((a.x + 100, a.y + 50)));
    let i = d.raise(i);
    assert_eq!((d.wins[i].r.x, d.wins[i].r.y), (a.x + 100, a.y + 50), "a raise moves nothing");
}

/// E2E N2: the Dock activating an app whose viewer is in front keeps the viewer in front (it does not raise the
/// main window over it); an app showing only its main window goes the usual open path.
#[test]
fn the_dock_activates_an_app_with_its_front_window_in_front() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, main, (900, 600));
        d.open(&m, viewer, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
    }
    assert!(sh.desk_activate_shown_app(main, 0.0));
    let d = sh.desk.as_ref().unwrap();
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, main, viewer]);
    assert!(sh.take_raise_front(), "the window server raises and focuses the viewer");
    assert!(!sh.desk_activate_shown_app(AppId::Terminal, 0.0), "one window: the open path");
    assert!(!sh.desk_activate_shown_app(viewer, 0.0), "only an app (its main id) is activated");
}

/// E2E9 5: opening an app that already shows two windows (Search's Return, Cmd+Tab) brings both forward, in their
/// own order, instead of only its main window.
#[test]
fn opening_a_running_app_brings_all_of_its_windows_forward() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = crate::shell::Screen::Home;
    let m = sh.desk_metrics();
    let (main, second) = (AppId::External(3), AppId::ExternalWindow(3, 1));
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, second, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
        d.open(&m, main, (900, 600));
        d.open(&m, AppId::Settings, (700, 500));
    }
    sh.desk_open(main, 0.0);
    let d = sh.desk.as_ref().unwrap();
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, AppId::Settings, second, main]);
    assert_eq!(sh.screen, crate::shell::Screen::App(main));
}

/// E2E12 2: an app that orders its main window out (its last document closed) loses the main desk window and keeps
/// running, with its other windows left alone; the desk never opens it again by itself. Activating the app reopens
/// the window and tells the app (`Reopen`); an app asking for the window back gets it without a `Reopen`.
#[test]
fn a_main_window_the_app_ordered_out_stays_closed_until_the_app_is_activated() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let reopens = || {
        let mut n = 0;
        while let Ok(bifrost_proto::sys::Recv::Packet(p, _)) = peer.recv(false) {
            n += usize::from(bifrost_proto::ToApp::decode(&p) == Ok(bifrost_proto::ToApp::Reopen));
        }
        n
    };
    let (main, other) = (AppId::External(slot), AppId::Terminal);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, other, (900, 600));
        d.open(&m, main, (600, 400));
    }
    sh.desk_order_main_out(slot);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_none(), "the main window is off the desk");
    assert!(sh.desk.as_ref().unwrap().index_of(other).is_some(), "other windows stay");
    assert!(sh.ext[slot as usize].as_ref().is_some_and(|h| h.running()), "the app keeps running");
    sh.desk_sync_extra_windows();
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_none(), "nothing reopens it on its own");
    assert_eq!(reopens(), 0);
    // the shared-memory route (the production path on Midget, built with desktop-wayland too): the shell opens the
    // main window itself
    sh.desk_open(main, 1.0);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "activating the app reopens its main window");
    assert_eq!(reopens(), 1, "and the app is told");
    sh.desk_open(main, 2.0);
    assert_eq!(reopens(), 0, "an app whose main window is open is only activated");
    sh.desk_order_main_out(slot);
    sh.desk_order_main_in(slot, 3.0);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "the app's request brings it back");
    assert_eq!(reopens(), 0, "an app that ordered its window back in is not told to reopen");
    assert!(!sh.ext[slot as usize].as_ref().unwrap().main_ordered_out);
    // ordered out again: the next activation reopens it and tells the app once; the flag is spent after that
    sh.desk_order_main_out(slot);
    sh.desk_open(main, 4.0);
    assert_eq!(reopens(), 1);
    sh.desk.as_mut().unwrap().wins.retain(|w| w.app != main);
    sh.desk_open(main, 5.0);
    assert_eq!(reopens(), 0, "a window closed by the desk, not ordered out by the app, is no reopen");
}

/// E2E13 1: an app in front that orders its last window out (Cmd+W on TextEdit's last document) stays the active
/// app with no window, as on the reference: the screen (and so its menu bar) stays on it, the window under it does
/// not take the front, Cmd+Tab lists it first, and switching to it reopens its window with `Reopen`. A press on
/// another window hands the front over; an app that was not in front gives the front to nobody new.
#[test]
fn an_app_that_closes_its_last_window_stays_the_active_app() {
    use crate::shell::Screen;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let reopens = || {
        let mut n = 0;
        while let Ok(bifrost_proto::sys::Recv::Packet(p, _)) = peer.recv(false) {
            n += usize::from(bifrost_proto::ToApp::decode(&p) == Ok(bifrost_proto::ToApp::Reopen));
        }
        n
    };
    let (main, other) = (AppId::External(slot), AppId::Terminal);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, other, (900, 600));
        d.open(&m, main, (600, 400));
    }
    sh.screen = Screen::App(main);
    sh.desk_order_main_out(slot);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_none(), "the window is gone");
    assert_eq!(sh.screen, Screen::App(main), "the app keeps the front and its menu bar");
    assert_eq!(sh.desk_windowless_front(), Some(main));
    assert!(sh.ext_windowless_apps().contains(&main), "it counts as running with no window (Dock dot, Cmd+Tab)");
    sh.desk_refocus();
    sh.desk_sync_extra_windows();
    assert_eq!(sh.screen, Screen::App(main), "no refocus or tick hands the front to the window under it");
    // Cmd+Tab: the active app leads, the selection starts on the next one
    sh.switcher_cycle(false);
    let (order, sel) = sh.desk.as_ref().unwrap().cmdtab.clone().unwrap();
    assert_eq!(order, vec![main, other]);
    assert_eq!(sel, 1);
    // releasing on the app reopens its window and tells it
    sh.desk.as_mut().unwrap().cmdtab = Some((order, 0));
    sh.cmdtab_release(1.0);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "switching to it reopens its window");
    assert_eq!(reopens(), 1);
    assert_eq!(sh.desk_windowless_front(), None);
    // closed again, then a press on the other window: that window's app is the active one
    sh.desk_order_main_out(slot);
    assert_eq!(sh.screen, Screen::App(main));
    {
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(other).unwrap();
        d.raise(i);
    }
    sh.desk_refocus();
    assert_eq!(sh.screen, Screen::App(other), "a raised window takes the front");
    assert_eq!(sh.desk_windowless_front(), None);
    assert!(sh.ext_windowless_apps().contains(&main), "the app still runs with no window");
    // an app in the background closing its last window takes nothing from the app in front
    sh.desk_open(main, 2.0);
    let _ = reopens();
    sh.desk_open(other, 3.0);
    sh.desk_order_main_out(slot);
    assert_eq!(sh.screen, Screen::App(other));
    assert_eq!(sh.desk_windowless_front(), None);
}

/// `Desk::windowless_front` never goes stale: an app that orders its main window out while an extra window of its
/// own is still shown is not windowless, and once a window comes back the mark is gone for good, so the app does
/// not become windowless again when that window later goes some other way.
#[test]
fn the_windowless_front_mark_is_cleared_once_it_no_longer_holds() {
    use crate::shell::Screen;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, _peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let (main, extra, other) = (AppId::External(slot), AppId::ExternalWindow(slot, 1), AppId::Terminal);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, other, (900, 600));
        d.open(&m, extra, (400, 300));
        d.open(&m, main, (600, 400));
    }
    sh.screen = Screen::App(main);
    sh.desk_order_main_out(slot);
    assert_eq!(sh.desk.as_ref().unwrap().windowless_front.get(), None, "its extra window is still shown");
    // windowless, then a window of the app comes back without `open` or `raise` (pushed by the window server)
    {
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(extra).unwrap();
        d.wins.remove(i);
        d.windowless_front.set(Some(main));
    }
    assert_eq!(sh.desk_windowless_front(), Some(main));
    sh.desk.as_mut().unwrap().open(&m, extra, (400, 300));
    sh.desk.as_mut().unwrap().windowless_front.set(Some(main));
    assert_eq!(sh.desk_windowless_front(), None, "a window came back");
    assert_eq!(sh.desk.as_ref().unwrap().windowless_front.get(), None, "the read cleared the stale mark");
    let i = sh.desk.as_ref().unwrap().index_of(extra).unwrap();
    sh.desk.as_mut().unwrap().wins.remove(i);
    assert_eq!(sh.desk_windowless_front(), None, "the mark does not come back when the window goes again");
}

/// E2E13: an app started in the background (a login item, a notification or share target) or a direct app that fell
/// back to shared memory is running with no main desk window it ordered out: its first activation shows the window
/// it made and sends no `Reopen`, which a document app would answer with a second, empty document.
#[test]
fn the_first_activation_of_a_background_app_is_no_reopen() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let reopens = || {
        let mut n = 0;
        while let Ok(bifrost_proto::sys::Recv::Packet(p, _)) = peer.recv(false) {
            n += usize::from(bifrost_proto::ToApp::decode(&p) == Ok(bifrost_proto::ToApp::Reopen));
        }
        n
    };
    let main = AppId::External(slot);
    sh.desk.as_mut().unwrap().wins.clear();
    assert!(sh.ext[slot as usize].as_ref().is_some_and(|h| h.running() && !h.main_ordered_out));
    sh.desk_open(main, 1.0);
    assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "the first activation shows the main window");
    assert_eq!(reopens(), 0, "and sends no Reopen");
}

/// E2E10: opening an app whose main window is closed (only a second window is shown) reopens the main window
/// instead of only activating the second one; a minimized main window is restored the same way.
#[test]
fn opening_an_app_with_its_main_window_closed_reopens_it() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let (main, second) = (AppId::External(3), AppId::ExternalWindow(3, 1));
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, second, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
    }
    assert!(!sh.desk_main_window_shown(main), "only the second window is open");
    sh.desk.as_mut().unwrap().open(&m, main, (900, 600));
    assert!(sh.desk_main_window_shown(main));
    let i = sh.desk.as_ref().unwrap().index_of(main).unwrap();
    sh.desk.as_mut().unwrap().wins[i].minimized = true;
    assert!(!sh.desk_main_window_shown(main), "a minimized main window is restored by the open path");
    sh.desk.as_mut().unwrap().wins.retain(|w| w.app != main);
    // without the window server the shell opens the main window itself (a direct app's window comes from the app)
    #[cfg(not(feature = "desktop-wayland"))]
    {
        sh.desk_open(main, 0.0);
        assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "the main window is open again");
    }
}

/// E2E13 2: the lock takes the windows' toplevels down and they map again in the order they draw, so when the unlock
/// ends the window server is told every shown window, bottom first, and the stacking matches the shell's again.
#[test]
fn the_unlock_restacks_the_shown_windows_in_the_shells_order() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, AppId::Terminal, (900, 600));
        d.open(&m, AppId::Photos, (900, 600));
        d.open(&m, AppId::Settings, (900, 600));
        let p = d.index_of(AppId::Photos).unwrap();
        d.wins[p].minimized = true;
    }
    sh.screen = Screen::App(AppId::Settings);
    sh.take_wayland_restack();
    sh.lock_screen();
    assert!(sh.is_locked());
    assert!(sh.take_wayland_restack().is_empty(), "nothing is restacked under the lock");
    // a second lock while locked does not forget what the first saw
    sh.lock_screen();
    sh.anim = Some((crate::shell::Anim::Unlock { from: 0.0, to: 1.0 }, 1000.0));
    sh.tick(6_000.0);
    assert!(!sh.is_locked(), "the unlock ended");
    let want: Vec<u32> = [AppId::Terminal, AppId::Settings].into_iter().filter_map(win_id).collect();
    assert_eq!(want.len(), 2);
    assert_eq!(sh.take_wayland_restack(), want, "the shown windows, bottom first; the minimized one stays in the Dock");
    sh.anim = Some((crate::shell::Anim::Unlock { from: 0.0, to: 1.0 }, 7_000.0));
    sh.tick(12_000.0);
    assert!(sh.take_wayland_restack().is_empty(), "told once per lock");
}

/// While the screen is locked or the unlock runs, the window server's focus of a toplevel as it maps again raises
/// nothing, so the unlock restacks to the order from before the lock.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_window_server_focus_under_the_lock_keeps_the_shells_order() {
    use crate::bare::wayland::WindowEvent;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, AppId::Terminal, (900, 600));
        d.open(&m, AppId::Settings, (900, 600));
    }
    sh.screen = Screen::App(AppId::Settings);
    sh.take_wayland_restack();
    sh.lock_screen();
    let term = win_id(AppId::Terminal).unwrap();
    sh.wayland_window_event(WindowEvent::ServerFocus(term));
    sh.wayland_window_event(WindowEvent::HandBack(term));
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Settings), "nothing raised under the lock");
    sh.anim = Some((crate::shell::Anim::Unlock { from: 0.0, to: 1.0 }, 1000.0));
    assert!(sh.desk_lock_holds_order());
    sh.wayland_window_event(WindowEvent::ServerFocus(term));
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Settings), "nothing raised while unlocking");
    sh.tick(6_000.0);
    assert!(!sh.desk_lock_holds_order());
    let want: Vec<u32> = [AppId::Terminal, AppId::Settings].into_iter().filter_map(win_id).collect();
    assert_eq!(sh.take_wayland_restack(), want, "restacked in the order from before the lock");
}

/// When another client's window was in front as the lock came, the shell's windows are not lifted over it at the
/// unlock (as the Apps grid leaves them when it closes).
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_unlock_leaves_another_clients_front_window_in_front() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, AppId::Terminal, (900, 600));
        d.native_windows.push(crate::bare::wayland::ForeignWindow { id: 40, title: "Film".into(), app_id: "x.film".into(), active: true, maximized: false, minimized: false });
    }
    sh.take_wayland_restack();
    sh.lock_screen();
    sh.anim = Some((crate::shell::Anim::Unlock { from: 0.0, to: 1.0 }, 1000.0));
    sh.tick(6_000.0);
    assert!(sh.take_wayland_restack().is_empty());
}

/// E2E5 3: the window server raises only the window it is asked to activate, so a Dock click on an app showing a
/// viewer tells it the app's whole set: the main window comes over the other apps too, under the viewer.
#[test]
fn activating_an_app_tells_the_window_server_all_of_its_windows_in_order() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, main, (900, 600));
        d.open(&m, viewer, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
    }
    sh.take_wayland_restack();
    assert!(sh.desk_activate_shown_app(main, 0.0));
    // the window server ids of the app's windows (the main window's needs the app in the catalogue)
    let ids: Vec<u32> = [main, viewer].into_iter().filter_map(win_id).collect();
    assert!(!ids.is_empty());
    assert_eq!(sh.take_wayland_restack(), ids, "the app's windows, bottom first; not the other app's");
    assert!(sh.take_wayland_restack().is_empty(), "told once");
    // Window > Bring All to Front says the same
    sh.screen = crate::shell::Screen::App(main);
    sh.desk_bring_all_to_front();
    let mut told = sh.take_wayland_restack();
    told.sort_unstable();
    let mut want = ids.clone();
    want.sort_unstable();
    assert_eq!(told, want);
}

/// E2E5 3: a hidden app shown again brings all of its windows back, and the window server is told the set, so the
/// toplevels that map before the front one do not go under the window it activated meanwhile.
#[test]
fn a_hidden_app_shown_again_tells_the_window_server_all_of_its_windows() {
    let m = metrics();
    let mut d = Desk::new();
    let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
    d.open(&m, main, (900, 600));
    d.open(&m, viewer, (500, 400));
    d.open(&m, AppId::Terminal, (900, 600));
    assert!(d.hide_app(main));
    d.restack.clear();
    d.open(&m, main, (900, 600));
    assert!(d.restack.contains(&main) && d.restack.contains(&viewer), "{:?}", d.restack);
    assert!(!d.restack.contains(&AppId::Terminal), "the other app stays where it is");
}

/// The Dock activating an app with extra windows still counts as opening it: it is in Recent Items, as every open is.
#[test]
fn activating_an_app_with_extra_windows_notes_it_in_recent_items() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, main, (900, 600));
        d.open(&m, viewer, (500, 400));
    }
    assert!(sh.desk_activate_shown_app(main, 0.0));
    let newest = sh.recent_list.entries().first().map(|e| e.key.clone());
    assert_eq!(newest, Some(super::super::recent_items::key(main)), "the newest Recent Items entry is the app activated");
}

#[test]
fn a_direct_app_activated_from_the_dock_raises_all_its_toplevels_front_last() {
    let windows = [(7, "x.mail", false), (8, "x.other", true), (9, "x.mail", true), (10, "x.mail", false)];
    assert_eq!(direct_activation_order(windows, "x.mail"), vec![7, 10, 9], "the active one last, so it ends in front");
    assert_eq!(direct_activation_order(windows, "x.none"), Vec::<u32>::new());
    assert_eq!(direct_activation_order([(1, "", false)], ""), Vec::<u32>::new(), "an empty id is no app's");
}

/// E2E N5: a press in the gap between two Dock icons (where the pointer magnifies both alike) opens the nearer one;
/// the wider gap at a divider keeps a dead strip in its middle.
#[test]
fn a_dock_press_between_two_icons_opens_the_nearer_one() {
    let row: Vec<Rect> = (0..3).map(|i| Rect { x: 100 + i * 58, y: 700, w: 48, h: 48 }).collect();
    let press = dock_press_rects(&row, false);
    let hit = |press: &[Rect], x: f32, y: f32| press.iter().position(|r| r.contains(x, y));
    assert_eq!(hit(&press, 120.0, 720.0), Some(0));
    assert_eq!(hit(&press, 150.0, 720.0), Some(0), "just past the first icon");
    assert_eq!(hit(&press, 154.0, 720.0), Some(1), "just before the second");
    assert_eq!(hit(&press, 154.0, 760.0), None, "below the row is not the Dock's icons");
    let divided = vec![Rect { x: 100, y: 700, w: 48, h: 48 }, Rect { x: 158, y: 700, w: 48, h: 48 }, Rect { x: 240, y: 700, w: 48, h: 48 }];
    assert_eq!(hit(&dock_press_rects(&divided, false), 223.0, 720.0), None, "the divider's middle opens nothing");
    let side = dock_press_rects(&[Rect { x: 10, y: 100, w: 48, h: 48 }, Rect { x: 10, y: 158, w: 48, h: 48 }], true);
    assert_eq!(hit(&side, 30.0, 152.0), Some(0));
    assert_eq!(hit(&side, 30.0, 153.0), Some(1));
    assert!(dock_press_rects(&[], false).is_empty());
}

#[test]
fn hide_others_keeps_the_front_app_and_unhide_all_restores_the_rest() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Photos, (900, 600));
    d.open(&m, AppId::Terminal, (900, 600));
    d.open(&m, AppId::Settings, (900, 600));
    let i = d.index_of(AppId::Photos).unwrap();
    d.wins[i].minimized = true; // minimized by the user: not hidden, and Show All leaves it in the Dock
    assert!(d.hide_others(Some(AppId::Settings)));
    assert!(d.is_hidden(AppId::Terminal) && !d.is_hidden(AppId::Settings));
    assert!(!d.is_hidden(AppId::Photos), "a minimized window is not hidden");
    assert!(!d.others_shown(Some(AppId::Settings)));
    d.unhide_all();
    assert!(!d.wins[d.index_of(AppId::Terminal).unwrap()].minimized, "Terminal is back");
    assert!(d.wins[d.index_of(AppId::Photos).unwrap()].minimized, "the minimized window stays in the Dock");
    assert!(!d.any_hidden());
}

#[test]
fn a_closed_window_remembers_its_frame_for_the_next_open() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::Photos, (900, 600));
    d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
    d.close(0);
    assert_eq!(d.closed_sizes.get(&AppId::Photos), Some(&(800, 500)), "the size it was closed with");
    // a zoomed window is remembered at the frame it goes back to, not at the screen size
    let i = d.open(&m, AppId::Photos, (900, 600));
    d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
    d.toggle_zoom(&m, i);
    d.close(0);
    assert_eq!(d.closed_sizes.get(&AppId::Photos), Some(&(800, 500)));
    // so is a full-screen one: size and origin are the windowed frame, not the screen
    let i = d.open(&m, AppId::Photos, (900, 600));
    d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
    assert!(d.set_full_screen(&m, i, true));
    d.close(0);
    assert_eq!(d.closed_sizes.get(&AppId::Photos), Some(&(800, 500)));
    assert_eq!(d.closed_origins.get(&AppId::Photos), Some(&(200, 150)));
}

/// W2-22: a borderless window's shape is where its pixels cover, row runs merged into rects.
#[test]
fn the_alpha_shape_covers_the_drawn_pixels_only() {
    let (w, h) = (10, 6);
    let mut px = vec![0u32; w * h];
    // a 3x2 block at (1,1) and a 2x3 block at (6,2)
    for (x0, y0, bw, bh) in [(1, 1, 3, 2), (6, 2, 2, 3)] {
        for y in y0..y0 + bh { for x in x0..x0 + bw { px[y * w + x] = 0x80ff_ff00; } }
    }
    let shape = alpha_shape(&px, w, h);
    let covered = |x: usize, y: usize| shape_contains(&shape, x as f32 + 0.5, y as f32 + 0.5);
    for y in 0..h { for x in 0..w {
        assert_eq!(covered(x, y), px[y * w + x] >> 24 != 0, "({x},{y})");
    }}
    assert!(shape.len() <= 4, "rows with the same runs merge: {shape:?}");
    assert!(alpha_shape(&vec![0u32; w * h], w, h).is_empty(), "nothing drawn, nothing hit");
    assert_eq!(alpha_shape(&vec![0xff00_0000u32; w * h], w, h), vec![[0, 0, w as i32, h as i32]], "an opaque window is one rect");
    // a noisy picture falls back to boxes that still cover every drawn pixel
    let (w, h) = (2 * MAX_SHAPE_RECTS, 4);
    let noisy: Vec<u32> = (0..w * h).map(|i| if (i + i / w) % 2 == 0 { 0xff00_0000 } else { 0 }).collect();
    let shape = alpha_shape(&noisy, w, h);
    assert!(shape.len() <= MAX_SHAPE_RECTS);
    assert!((0..w * h).filter(|i| noisy[*i] != 0).all(|i| shape_contains(&shape, (i % w) as f32 + 0.5, (i / w) as f32 + 0.5)));
}

/// W2-22: a click on the transparent part of a shaped window picks the window behind it.
#[test]
fn a_shaped_window_lets_a_click_through_where_it_drew_nothing() {
    let m = metrics();
    let mut d = Desk::new();
    let back = d.open(&m, AppId::Photos, (900, 600));
    let back_r = d.wins[back].r;
    let front = d.open(&m, AppId::External(3), (1600, 900));
    let r = d.wins[front].r;
    let note = [100, 100, 200, 150];
    d.shapes.insert(AppId::External(3), std::sync::Arc::from(vec![note]));
    let on_note = ((r.x + 150) as f32, (r.y + 150) as f32);
    assert_eq!(d.pick(&m, on_note.0, on_note.1).map(|(i, _)| d.wins[i].app), Some(AppId::External(3)), "the note takes the click");
    // a point inside both windows but off the note reaches the window behind
    let (x, y) = ((back_r.x + back_r.w / 2) as f32, (back_r.y + back_r.h / 2) as f32);
    assert!(r.contains(x, y) && !shape_contains(&[note], x - r.x as f32, y - r.y as f32));
    assert_eq!(d.pick(&m, x, y).map(|(i, _)| d.wins[i].app), Some(AppId::Photos));
    d.close(d.index_of(AppId::External(3)).unwrap());
    assert!(d.shapes.is_empty(), "closing forgets the shape");
}

/// W2-26: a window closed and opened again comes back at the frame it was closed with, and its toplevel asks the
/// window server to map it there; a window never closed is cascaded as before.
#[test]
fn a_reopened_window_comes_back_where_it_was_closed() {
    let m = metrics();
    let mut d = Desk::new();
    let i = d.open(&m, AppId::Settings, (700, 600));
    let a = Desk::area(&m);
    let at = Rect { x: a.x + 40, y: a.y + 30, w: 700, h: 600 };
    d.place(&m, i, at);
    d.close(i);
    assert!(!d.placed.contains(&AppId::Settings), "the closed toplevel is gone");
    let i = d.open(&m, AppId::Settings, (700, 600));
    assert_eq!((d.wins[i].r.x, d.wins[i].r.y), (at.x, at.y), "back where it was");
    assert!(d.placed.contains(&AppId::Settings), "its new toplevel maps there (origin_hint)");
    assert_eq!(origin_hint(&d.placed, AppId::Settings, d.wins[i].r.x, d.wins[i].r.y), Some((at.x, at.y)));
    // a remembered spot off the work area (a smaller screen now) is held inside it
    d.close(i);
    d.closed_origins.insert(AppId::Settings, (a.x + a.w + 500, a.y - 200));
    let i = d.open(&m, AppId::Settings, (700, 600));
    let r = d.wins[i].r;
    assert!(r.x >= a.x && r.x + r.w <= a.x + a.w && r.y >= a.y, "inside the work area");
    // a window never closed is cascaded and asks nothing
    let j = d.open(&m, AppId::Photos, (500, 400));
    let p = d.wins[j].app;
    assert!(!d.placed.contains(&p));
}

#[test]
fn oversized_windows_are_clamped_to_the_work_area() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (5000, 5000));
    let a = Desk::area(&m);
    assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (a.w, a.h));
}

#[test]
fn full_screen_takes_the_screen_and_goes_back_to_its_size() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::External(0), (500, 400));
    let before = d.wins[0].r;
    assert!(d.set_full_screen(&m, 0, true));
    assert!(d.wins[0].full && d.front_full());
    assert_eq!(d.wins[0].r, Desk::screen_rect(&m), "the whole screen, under the menu bar");
    assert!(!d.set_full_screen(&m, 0, true), "already full: no change");
    d.toggle_zoom(&m, 0);
    assert_eq!(d.wins[0].r, Desk::screen_rect(&m), "a full window does not zoom");
    assert!(d.set_full_screen(&m, 0, false));
    assert_eq!(d.wins[0].r, before);
    assert!(!d.front_full());
}

/// Metrics of an output of `w` x `h` px at `pt` px per point: the sizes scale with it, as the shell's do.
fn metrics_for(w: i32, h: i32, pt: f32) -> Metrics {
    let px = |v: f32| (v * pt).round() as i32;
    Metrics { w, h, title: px(30.0), menubar: px(28.0), min_w: px(320.0), min_h: px(220.0), edge: px(6.0), light: px(12.0), light_gap: px(8.0), light_x: px(12.0), dock_left: 0, dock_right: 0, dock_bottom: px(100.0), pt }
}

#[test]
fn full_screen_covers_the_whole_output_at_its_origin_for_any_size_and_scale() {
    for (w, h, pt) in [(1024, 768, 1.0), (1280, 720, 1.0), (1920, 1080, 1.0), (2560, 1440, 1.5625), (3840, 2160, 2.0), (1440, 3200, 3.0)] {
        let m = metrics_for(w, h, pt);
        let mut d = Desk::new();
        d.open(&m, AppId::ExternalWindow(0, 4), (m.w / 3, m.h / 3));
        let before = d.wins[0].r;
        assert!(d.set_full_screen(&m, 0, true));
        let r = d.wins[0].r;
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, w, h), "{w}x{h} at {pt}x: the output, not the old origin");
        let body = d.wins[0].body(&m);
        assert_eq!((body.x, body.y, body.w, body.h), (0, 0, w, h), "no title bar in full screen");
        assert!(d.set_full_screen(&m, 0, false));
        assert_eq!(d.wins[0].r, before, "{w}x{h}: the previous frame comes back");
    }
}

#[test]
fn a_zoomed_window_returns_zoomed_from_full_screen_and_keeps_the_frame_before_the_zoom() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
    let before = d.wins[0].r;
    d.toggle_zoom(&m, 0);
    assert_eq!(d.wins[0].r, Desk::area(&m));
    assert!(d.set_full_screen(&m, 0, true));
    assert_eq!(d.wins[0].r, Desk::screen_rect(&m));
    assert!(!d.wins[0].maximized, "full screen is not the zoom");
    assert!(d.set_full_screen(&m, 0, false));
    assert!(d.wins[0].maximized && d.wins[0].r == Desk::area(&m), "back where it was: zoomed");
    d.toggle_zoom(&m, 0);
    assert_eq!(d.wins[0].r, before, "un-zooming still goes to the frame before the zoom");
}

#[test]
fn a_full_screen_window_is_all_content_with_no_lights_or_edges() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
    let lights = (d.wins[0].r.x + m.light_x + 2 * (m.light + m.light_gap) + m.light / 2, d.wins[0].r.y + m.title / 2);
    assert!(matches!(d.wins[0].hit(&m, lights.0 as f32, lights.1 as f32), Some(Hit::Light(_))), "the lights are there before");
    d.set_full_screen(&m, 0, true);
    for (x, y) in [(lights.0, lights.1), (0, 0), (m.w / 2, m.h - 1), (m.w - 1, m.h / 2)] {
        assert_eq!(d.wins[0].hit(&m, x as f32, y as f32), Some(Hit::Content), "({x},{y})");
    }
    assert_eq!(d.wins[0].hit(&m, -50.0, -50.0), None);
}

#[test]
fn a_full_screen_front_window_makes_the_dock_slide_away() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
    assert!(!d.dock_autohides(), "an ordinary window leaves the Dock as the Dock's own setting has it");
    d.set_full_screen(&m, 0, true);
    assert!(d.dock_autohides());
    d.set_full_screen(&m, 0, false);
    assert!(!d.dock_autohides());
}

#[test]
fn the_green_button_menu_has_fill_centre_the_halves_and_quarters_and_full_screen() {
    let lines = arrange_lines(false, false);
    let items: Vec<(&str, Arrange)> = lines.iter().filter_map(|l| if let ArrangeLine::Item(t, a) = l { Some((*t, *a)) } else { None }).collect();
    for tile in [Tile::Fill, Tile::Center, Tile::Left, Tile::Right, Tile::Top, Tile::Bottom, Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight] {
        assert!(items.iter().any(|(_, a)| *a == Arrange::Tile(tile)), "{tile:?} is in the menu");
    }
    assert!(items.iter().any(|(_, a)| *a == Arrange::FullScreen(true)));
    assert!(items.iter().any(|(_, a)| *a == Arrange::Previous));
    assert!(!items.iter().any(|(_, a)| *a == Arrange::SideBySide), "no other window to share the screen with");
    assert!(lines.contains(&ArrangeLine::Header("Move & Resize")));
    for (i, (t, a)) in items.iter().enumerate() {
        assert!(items.iter().skip(i + 1).all(|(u, b)| u != t && b != a), "{t} appears once");
    }
    assert_eq!(lines.first(), Some(&ArrangeLine::Item("Fill", Arrange::Tile(Tile::Fill))));
    assert_ne!(lines.last(), Some(&ArrangeLine::Separator));
}

#[test]
fn the_green_button_menu_offers_a_side_by_side_tile_only_with_a_second_window() {
    let has = |others| arrange_lines(false, others).contains(&ArrangeLine::Item("Tile with Next Window", Arrange::SideBySide));
    assert!(!has(false));
    assert!(has(true));
}

#[test]
fn a_full_screen_window_can_only_leave_full_screen() {
    assert_eq!(arrange_lines(true, true), vec![ArrangeLine::Item("Exit Full Screen", Arrange::FullScreen(false))]);
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn every_tile_is_a_distinct_placement_for_the_window_server() {
    let tiles = [Tile::Fill, Tile::Center, Tile::Left, Tile::Right, Tile::Top, Tile::Bottom, Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight];
    let wires: Vec<u32> = tiles.iter().map(|t| crate::bare::window_level_client::placement_wire(placement_of(*t))).collect();
    for (i, a) in wires.iter().enumerate() {
        assert!(wires.iter().skip(i + 1).all(|b| a != b), "{:?} shares its wire value", tiles[i]);
    }
    assert_eq!(placement_of(Tile::Center), crate::bare::window_level_client::Placement::Centre, "Centre is the window server's centre key");
}

#[test]
fn the_menu_bar_comes_back_only_at_the_top_edge() {
    assert!(!bar_hidden(false, None, 24.0, 0.0), "an ordinary window never hides it");
    assert!(bar_hidden(true, None, 24.0, 0.0), "no pointer seen yet: hidden");
    assert!(bar_hidden(true, Some(400.0), 24.0, 0.0));
    assert!(!bar_hidden(true, Some(3.0), 24.0, 0.0), "pointer at the top: shown");
    assert!(bar_hidden(true, Some(30.0), 24.0, 0.0), "under the bar with nothing revealed: hidden");
    assert!(!bar_hidden(true, Some(30.0), 24.0, 52.0), "on the revealed toolbar: kept");
    assert!(bar_hidden(true, Some(76.0), 24.0, 52.0), "below the revealed toolbar: hidden");
}

/// The reference's full screen: with the menu bar back, the window's toolbar sits directly under it (never under the
/// bar), its traffic lights are in that band, and a press on the band reaches the app at the toolbar row it shows.
#[test]
fn the_band_revealed_under_the_menu_bar_maps_to_the_toolbar_and_shows_the_lights() {
    for (sw, sh, pt) in [(2560, 1440, 1.5625f32), (1920, 1080, 1.0), (3840, 2160, 2.0)] {
        let m = metrics_for(sw, sh, pt);
        let mut d = Desk::new();
        d.open(&m, AppId::ExternalWindow(0, 4), (m.w / 3, m.h / 3));
        d.wins[0].full_size = true;
        assert!(d.set_full_screen(&m, 0, true));
        let w = d.wins[0];
        let rv = Reveal { top: m.menubar, band: reveal_band(&w, &m) };
        assert_eq!(rv.band, (UNIFIED_BAR_PT * m.pt).round() as i32, "the unified toolbar's height");
        // the menu bar's own band is not the window's, nor is the content under the band
        assert_eq!(reveal_hit(&w, &m, rv, (m.w / 2) as f32, (m.menubar - 1) as f32), None);
        assert_eq!(reveal_hit(&w, &m, rv, (m.w / 2) as f32, (m.menubar + rv.band) as f32), None);
        // a point in the band is the app's at the toolbar row the band shows
        let (x, y) = ((m.w / 2) as f32, (m.menubar + rv.band / 2) as f32);
        assert_eq!(reveal_hit(&w, &m, rv, x, y), Some(RevealHit::App(y - m.menubar as f32)));
        // the lights are in the band, below the bar
        let b = revealed_win(&w, rv);
        for (k, l) in [Light::Close, Light::Minimize, Light::Zoom].into_iter().enumerate() {
            let lr = b.light_rect(&m, k);
            assert!(lr.y >= m.menubar && lr.y + lr.h <= m.menubar + rv.band, "{sw}x{sh}: light {k} is inside the band");
            let (cx, cy) = ((lr.x + lr.w / 2) as f32, (lr.y + lr.h / 2) as f32);
            assert_eq!(reveal_hit(&w, &m, rv, cx, cy), Some(RevealHit::Light(l)));
        }
        // a full-screen window never starts a window server move
        assert_eq!(drag_zone(&w, &m, &[]), (0, Vec::new()));
    }
}

/// A titled window in full screen reveals its title bar under the menu bar: the lights are there, and the rest of the
/// strip covers content, so a press on it reaches nothing.
#[test]
fn a_titled_full_screen_window_reveals_its_title_bar_with_the_lights() {
    let m = metrics_for(1920, 1080, 1.0);
    let mut d = Desk::new();
    d.open(&m, AppId::ExternalWindow(0, 4), (600, 400));
    assert!(d.set_full_screen(&m, 0, true));
    let w = d.wins[0];
    assert!(!w.is_full_size());
    let rv = Reveal { top: m.menubar, band: reveal_band(&w, &m) };
    assert_eq!(rv.band, m.title);
    let lr = revealed_win(&w, rv).light_rect(&m, 0);
    assert_eq!(reveal_hit(&w, &m, rv, (lr.x + lr.w / 2) as f32, (lr.y + lr.h / 2) as f32), Some(RevealHit::Light(Light::Close)));
    assert_eq!(reveal_hit(&w, &m, rv, (m.w / 2) as f32, (m.menubar + m.title / 2) as f32), Some(RevealHit::Bar));
}

#[test]
fn a_frame_id_names_the_app_whose_level_it_takes() {
    // the wayland frame of an app is `win_id(app)`, and take_wayland_windows finds the app again from that id
    for (i, a) in apps().iter().enumerate() {
        assert_eq!(win_id(a.id), Some(i as u32 + 1));
        assert_eq!(win_id(a.id).and_then(|id| apps().get(id as usize - 1)).map(|b| b.id), Some(a.id));
    }
}

/// A window the window server placed asks its toplevel to map where it is (`origin`); a new one asks nothing.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_frames_of_placed_windows_carry_their_origin_and_new_windows_do_not() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let (placed, fresh) = (AppId::Settings, AppId::AppStore);
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, placed, (600, 400));
        d.open(&m, fresh, (600, 400));
    }
    // the window server reported where the person dragged the first window
    let id = win_id(placed).unwrap();
    sh.wayland_window_event(crate::bare::wayland::WindowEvent::Frame(id, 1065, 524, 600, 400));
    let rect = |sh: &Shell, app| sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).map(|w| w.r).unwrap();
    assert_eq!((rect(&sh, placed).x, rect(&sh, placed).y), (1065, 524), "the shell follows the reported frame");
    let frames: Vec<_> = [placed, fresh].iter().map(|&app| {
        let r = rect(&sh, app);
        crate::bare::wayland::WindowFrame { id: win_id(app).unwrap(), title: "", x: r.x, y: r.y, width: r.w as usize, height: r.h as usize, pixels: None, damage: crate::gfx::Damage::new(1, 1), maximized: false, fullscreen: false, title_height: 0, drag_height: 0, no_drag: Vec::new(), floating: false, fx: bifrost_render::glass::Fx::IDENTITY, opaque: true, origin: None, position: None, min_size: (0, 0), input: None }
    }).collect();
    sh.desk.as_mut().unwrap().frames = frames;
    let out = sh.take_wayland_windows();
    let of = |app| out.iter().find(|f| Some(f.id) == win_id(app)).unwrap();
    assert_eq!(of(placed).origin, Some((1065, 524)));
    assert_eq!(of(fresh).origin, None, "a window the window server never placed is cascaded by it");
    // each toplevel advertises its app's own minimum: Settings' split width, the desk minimum for the others
    let (sw, sh_min, _, _) = size_range(placed, &m);
    assert_eq!(of(placed).min_size, (sw, sh_min));
    assert!(sw >= (crate::shell::settings::SPLIT_MIN_PT * m.pt) as i32 || sw == m.w, "Settings' minimum is its split layout width");
    assert_eq!(of(fresh).min_size, (m.min_w, m.min_h + title_of(fresh, &m)));
    // closing forgets the place: the window opened again is a new one
    let d = sh.desk.as_mut().unwrap();
    let i = d.index_of(placed).unwrap();
    d.close(i);
    assert!(!d.placed.contains(&placed));
}

/// O2: a press that lands on the body of a window behind another one raises it and is the app's press as well: the
/// app hears Down, then the drag's Moves, then Up, so a drag that starts on an item of a background window begins
/// (the first click of a window is delivered, as `acceptsFirstMouse` does for a drag source).
#[test]
fn a_press_that_raises_a_background_window_still_reaches_its_app_as_a_drag() {
    use crate::shell::Screen;
    use bifrost_proto::{ToApp, TouchKind};
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let (back, front) = (AppId::External(slot), AppId::Terminal);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, back, (500, 400));
        d.open(&m, front, (500, 400));
        // side by side, so the press on the first lands on nothing of the second
        let a = Desk::area(&m);
        for (app, x) in [(back, a.x + 10), (front, a.x + a.w - 510)] {
            let i = d.index_of(app).unwrap();
            d.wins[i].r = Rect { x, y: a.y + 10, w: 500, h: 400 };
            d.wins[i].restore = d.wins[i].r;
        }
    }
    sh.screen = Screen::App(front);
    let w = *sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == back).unwrap();
    let b = w.body(&m);
    let (x, y) = ((b.x + b.w / 2) as f32, (b.y + b.h / 2) as f32);
    sh.desk_event(Event::Down(x, y), 100.);
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(back), "the press raises the window");
    assert_eq!(sh.screen, Screen::App(back));
    assert!(matches!(sh.desk.as_ref().unwrap().grab, Some(Grab::Content(a, _)) if a == back), "and the press is the app's");
    sh.desk_event(Event::Move(x + 12., y + 9.), 116.);
    sh.desk_event(Event::Up(x + 12., y + 9.), 132.);
    let mut kinds = Vec::new();
    while let Ok(bifrost_proto::sys::Recv::Packet(p, _)) = peer.recv(false) {
        if let Ok(ToApp::Touch { kind, .. }) = ToApp::decode(&p) { kinds.push(kind); }
    }
    assert_eq!(kinds, vec![TouchKind::Down, TouchKind::Move, TouchKind::Up], "the app hears the whole drag");
}

/// The drag session of an app's window takes the pointer from the press that began it: the window hears Down and the
/// Moves up to the grants, then the session runs (Entered on its own window), and its release ends the press, so
/// the pointer moves that follow (no button held) are not touches of a drag the window believes is still held.
#[test]
fn a_drag_session_takes_the_release_and_ends_the_press_that_began_it() {
    use crate::shell::{sharing::INBOUND, Screen};
    use bifrost_proto::{dragop, DragItem, DragPhase, DragPreview, Payload, ToApp, ToShell, TouchKind};
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let app = AppId::External(slot);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, app, (500, 400));
    }
    sh.screen = Screen::App(app);
    let w = *sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).unwrap();
    let b = w.body(&m);
    let (x, y) = ((b.x + b.w / 2) as f32, (b.y + b.h / 2) as f32);
    sh.desk_event(Event::Down(x, y), 100.);
    sh.desk_event(Event::Move(x + 14., y), 116.);
    sh.dnd_msg(slot, ToShell::DragRegister { types: vec!["public.data".into()] });
    let item = DragItem::new(Payload::File { grant: [1; 16], name: "a.txt".into() });
    sh.dnd_msg(slot, ToShell::DragBegin { session: INBOUND | 1, allowed: dragop::COPY | dragop::MOVE, items: vec![item], preview: DragPreview::default() });
    assert!(sh.dnd.session.is_some(), "the window in front began the session");
    // the session owns the pointer: the move enters the window itself, the release is the window's drop
    assert!(sh.dnd_event(Event::Move(x + 30., y + 4.), 132.).is_some());
    assert!(sh.dnd_event(Event::Up(x + 30., y + 4.), 148.).is_some());
    assert!(sh.desk.as_ref().unwrap().grab_idle(), "the release ended the press");
    // the pointer moves after the release reach nobody as a touch
    sh.desk_event(Event::Move(x + 50., y), 164.);
    let (mut touches, mut phases) = (Vec::new(), Vec::new());
    while let Ok(bifrost_proto::sys::Recv::Packet(p, _)) = peer.recv(false) {
        match ToApp::decode(&p) {
            Ok(ToApp::Touch { kind, .. }) => touches.push(kind),
            Ok(ToApp::DragDest { phase, .. }) => phases.push(phase),
            _ => {}
        }
    }
    assert_eq!(touches, vec![TouchKind::Down, TouchKind::Move], "the app heard the press and the moves up to the session only");
    assert_eq!(phases.first(), Some(&DragPhase::Entered), "the session entered the source window");
    assert!(phases.contains(&DragPhase::Updated), "the release is a pointer step in the window, then its drop");
}

/// O8: the active app that closed its last window keeps the menu bar, so the window under it must not draw as the
/// active one (key traffic lights, title and shadow): no window is active until a window is raised or opened.
#[test]
fn no_window_draws_active_while_a_windowless_app_is_in_front() {
    use crate::shell::Screen;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let (mine, _peer) = bifrost_proto::sys::socketpair_pair().unwrap();
    let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
    sh.ext.push(Some(apphost::AppHost::with_channel("fixture", mine, None, cfg)));
    let slot = (sh.ext.len() - 1) as u8;
    let (main, other) = (AppId::External(slot), AppId::Terminal);
    let m = sh.desk_metrics();
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, other, (900, 600));
        d.open(&m, main, (600, 400));
    }
    sh.screen = Screen::App(main);
    let at = |sh: &Shell, app| sh.desk.as_ref().unwrap().index_of(app);
    assert_eq!(sh.desk_active_window_index(), at(&sh, main), "with a window in front it is the active one");
    sh.desk_order_main_out(slot);
    assert_eq!(sh.desk_windowless_front(), Some(main));
    assert_eq!(sh.desk_active_window_index(), None, "the window under the windowless app is not active");
    // a window raised takes the front again, and with it the active look
    {
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(other).unwrap();
        d.raise(i);
    }
    assert_eq!(sh.desk_windowless_front(), None);
    assert_eq!(sh.desk_active_window_index(), at(&sh, other));
}

/// O1: `take_wayland_windows` empties the desk's frames on every present, so a window that starts closing found no
/// frame to keep and the close effect had nothing to draw. It keeps the frame the last present took (without
/// pixels: the presenter holds them), and a present after the window is gone keeps nothing of it.
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_closing_window_keeps_the_frame_the_last_present_took_for_its_close_effect() {
    // window effects run only under the window server
    let mut sh = crate::shell::tests::desktop_shell();
    sh.wayland = true;
    let m = sh.desk_metrics();
    let app = AppId::Settings;
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, app, (600, 400));
    }
    let id = win_id(app).unwrap();
    let r = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).map(|w| w.r).unwrap();
    let frame = crate::bare::wayland::WindowFrame { id, title: "", x: r.x, y: r.y, width: r.w as usize, height: r.h as usize, pixels: Some(std::sync::Arc::from(vec![0u32; 4])), damage: crate::gfx::Damage::new(1, 1), maximized: false, fullscreen: false, title_height: 0, drag_height: 0, no_drag: Vec::new(), floating: false, fx: bifrost_render::glass::Fx::IDENTITY, opaque: true, origin: Some((1, 2)), position: Some((3, 4)), min_size: (0, 0), input: None };
    sh.desk.as_mut().unwrap().frames = vec![frame];
    assert_eq!(sh.take_wayland_windows().len(), 1);
    assert!(sh.desk.as_ref().unwrap().frames.is_empty(), "the present took the frames");
    sh.desk_close_app(app);
    let d = sh.desk.as_ref().unwrap();
    assert!(d.fx.get(id).is_some_and(|f| f.effect == window_fx::Effect::Close), "the close effect started");
    assert_eq!(d.ghosts.len(), 1, "the closing window keeps its last frame for the effect");
    let (z, ghost) = &d.ghosts[0];
    assert_eq!((*z, ghost.id, ghost.x, ghost.y, ghost.width, ghost.height), (0, id, r.x, r.y, r.w as usize, r.h as usize));
    assert!(ghost.pixels.is_none(), "the presenter still holds the pixels");
    assert_eq!((ghost.origin, ghost.position), (None, None), "a kept frame places nothing again");
    // the next present has no frame of the closed window to remember
    assert!(sh.take_wayland_windows().is_empty());
    assert!(sh.desk.as_ref().unwrap().presented.is_empty(), "nothing of the closed window outlives its present");
}

/// E2E11 3: an app moving its mapped window (`WindowMove`) makes the next frame ask the window server to move the
/// toplevel there (`set_position`), once; a frame after that asks nothing, so a later drag is never pulled back.
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_window_its_app_moved_asks_the_window_server_to_move_its_toplevel_once() {
    let mut sh = crate::shell::tests::desktop_shell();
    let m = sh.desk_metrics();
    let app = AppId::Settings;
    {
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        d.open(&m, app, (600, 400));
    }
    let id = win_id(app).unwrap();
    // the window server mapped it and said where
    sh.wayland_window_event(crate::bare::wayland::WindowEvent::Frame(id, 300, 200, 600, 400));
    assert!(sh.ext_window_place(app, Some((140.0, 120.0)), false));
    let r = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).map(|w| w.r).unwrap();
    let frame = |r: Rect| crate::bare::wayland::WindowFrame { id, title: "", x: r.x, y: r.y, width: r.w as usize, height: r.h as usize, pixels: None, damage: crate::gfx::Damage::new(1, 1), maximized: false, fullscreen: false, title_height: 0, drag_height: 0, no_drag: Vec::new(), floating: false, fx: bifrost_render::glass::Fx::IDENTITY, opaque: true, origin: None, position: None, min_size: (0, 0), input: None };
    sh.desk.as_mut().unwrap().frames = vec![frame(r)];
    let out = sh.take_wayland_windows();
    assert_eq!(out[0].position, Some((r.x, r.y)), "the mapped toplevel moves where the shell put the window");
    assert_ne!((r.x, r.y), (300, 200));
    sh.desk.as_mut().unwrap().frames = vec![frame(r)];
    let out = sh.take_wayland_windows();
    assert_eq!(out[0].position, None, "asked once");
}

/// Wave b2: Mail's main window was closed while its compose window stayed open. The Dock showed no Mail tile or dot
/// for it (only the main window counted), so the lone compose window looked like a leftover frame of a dead app.
#[test]
fn an_app_with_only_an_extra_window_still_has_a_window_on_the_desk() {
    let m = metrics();
    let mut d = Desk::default();
    d.open(&m, AppId::ExternalWindow(3, 2), (300, 200));
    assert!(d.index_of(AppId::External(3)).is_none(), "its main window is gone");
    assert!(d.has_window_of(AppId::External(3)), "the app still has the extra window");
    assert!(d.has_window_of(AppId::ExternalWindow(3, 7)), "an extra window asks for its app");
    assert!(!d.has_window_of(AppId::External(4)), "another app's windows do not count");
    let i = d.index_of(AppId::ExternalWindow(3, 2)).unwrap();
    d.close(i);
    assert!(!d.has_window_of(AppId::External(3)), "closing the last window leaves the app with none");
}

#[test]
fn a_floating_extra_window_stays_above_the_normal_windows_of_the_desk() {
    let m = metrics();
    let mut d = Desk::default();
    d.open(&m, AppId::ExternalWindow(3, 2), (300, 200));
    d.open(&m, AppId::Files, (400, 300));
    d.sync_levels(vec![AppId::ExternalWindow(3, 2)]);
    assert_eq!(d.wins.last().map(|w| w.app), Some(AppId::ExternalWindow(3, 2)), "the floating extra window is last");
    d.sync_levels(Vec::new());
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::ExternalWindow(3, 2)], "a window that stops floating keeps its place in the normal band");
}

#[test]
fn a_floating_window_stays_above_the_normal_ones_when_raised_or_opened() {
    let m = metrics();
    let mut d = Desk::default();
    d.open(&m, AppId::External(3), (400, 300));
    d.open(&m, AppId::Files, (400, 300));
    d.sync_levels(vec![AppId::External(3)]);
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::External(3)], "the floating window is last");
    assert_eq!(d.raise(0), 0, "a normal window raised goes in front of the normal ones, under the floating one");
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::External(3)]);
    d.open(&m, AppId::Settings, (400, 300));
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::Settings, AppId::External(3)], "a new normal window opens under the floating one");
    let i = d.index_of(AppId::External(3)).unwrap();
    d.raise(i);
    assert_eq!(d.wins.last().map(|w| w.app), Some(AppId::External(3)), "raising the floating window keeps it on top");
    d.sync_levels(Vec::new());
    assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::Settings, AppId::External(3)], "dropping the level keeps the order");
}

#[test]
fn hits_find_lights_title_content_and_edges() {
    let m = metrics();
    let w = Win { app: AppId::Files, r: Rect { x: 100, y: 100, w: 600, h: 400 }, maximized: false, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 }, full_size: false };
    assert_eq!(w.hit(&m, 118.0, 115.0), Some(Hit::Light(Light::Close)));
    assert_eq!(w.hit(&m, 138.0, 115.0), Some(Hit::Light(Light::Minimize)));
    assert_eq!(w.hit(&m, 158.0, 115.0), Some(Hit::Light(Light::Zoom)));
    assert_eq!(w.hit(&m, 400.0, 115.0), Some(Hit::Title));
    assert_eq!(w.hit(&m, 400.0, 300.0), Some(Hit::Content));
    assert_eq!(w.hit(&m, 699.0, 499.0), Some(Hit::Edge(R | B)));
    assert_eq!(w.hit(&m, 101.0, 300.0), Some(Hit::Edge(L)));
    assert_eq!(w.hit(&m, 50.0, 50.0), None);
}

#[test]
fn a_maximized_window_has_no_resize_edges() {
    let m = metrics();
    let w = Win { app: AppId::Settings, r: Desk::area(&m), maximized: true, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 }, full_size: false };
    assert_eq!(w.hit(&m, 1.0, 600.0), Some(Hit::Content));
}

#[test]
fn the_front_window_wins_and_minimized_ones_are_skipped() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    d.open(&m, AppId::Photos, (900, 600));
    let (x, y) = (d.wins[1].r.x as f32 + 300.0, d.wins[1].r.y as f32 + 300.0);
    assert_eq!(d.pick(&m, x, y).map(|p| p.0), Some(1));
    d.wins[1].minimized = true;
    assert!(d.pick(&m, x, y).map_or(true, |p| p.0 == 0));
    assert_eq!(d.front().map(|w| w.app), Some(AppId::Settings));
}

#[test]
fn an_app_resizes_its_window_from_the_top_left_inside_the_work_area() {
    let m = metrics();
    let app = AppId::External(250);
    let mut d = Desk::new();
    d.open(&m, app, (900, 600));
    let before = d.wins[0].r;
    // a size inside the window's range (the desktop minimum is m.min_w wide; smaller is held to it, below)
    assert!(d.set_content_size(&m, 0, (430.0, 510.0)));
    let (w, h) = outer_px(app, &m, (430.0, 510.0));
    assert_eq!(d.wins[0].r, Rect { x: before.x, y: before.y, w, h }, "the top-left corner stays");
    assert_eq!(d.wins[0].restore, d.wins[0].r);
    assert!(!d.set_content_size(&m, 0, (430.0, 510.0)), "the same size again changes nothing");
    // larger than the screen: held to the work area, and moved back onto it
    assert!(d.set_content_size(&m, 0, (100_000.0, 100_000.0)));
    assert_eq!(d.wins[0].r, Desk::area(&m));
    // under the window minimum: held to it
    assert!(d.set_content_size(&m, 0, (1.0, 1.0)));
    let (min_w, min_h, _, _) = size_range(app, &m);
    assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (min_w, min_h));
}

#[test]
fn a_zoomed_window_keeps_its_frame_and_returns_at_the_size_the_app_asked_for() {
    let m = metrics();
    let app = AppId::External(251);
    let mut d = Desk::new();
    d.open(&m, app, (900, 600));
    d.toggle_zoom(&m, 0);
    assert!(!d.set_content_size(&m, 0, (560.0, 522.0)));
    assert_eq!(d.wins[0].r, Desk::area(&m));
    d.toggle_zoom(&m, 0);
    let (w, h) = outer_px(app, &m, (560.0, 522.0));
    assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (w, h));
}

#[test]
fn content_limits_hold_the_apps_own_resize_and_go_with_the_app() {
    let m = metrics();
    let app = AppId::External(252);
    set_content_limits(252, Some((600.0, 500.0)), None);
    let mut d = Desk::new();
    d.open(&m, app, (2000, 1200));
    assert!(d.set_content_size(&m, 0, (230.0, 300.0)));
    let (w, h) = outer_px(app, &m, (600.0, 500.0));
    assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (w.min(Desk::area(&m).w), h.min(Desk::area(&m).h)));
    forget_content_limits(252);
    assert_eq!(size_limits(app).min, None, "the manifest's limits (none here) apply again");
}

#[test]
fn zoom_toggles_between_the_work_area_and_the_old_rect() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    let before = d.wins[0].r;
    d.toggle_zoom(&m, 0);
    assert_eq!(d.wins[0].r, Desk::area(&m));
    d.toggle_zoom(&m, 0);
    assert_eq!(d.wins[0].r, before);
}

#[test]
fn tiling_moves_a_window_to_a_half_and_return_restores_its_size() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    let before = d.wins[0].r;
    d.tile(&m, 0, Tile::Left);
    assert_eq!(d.wins[0].r, tile_rect(Desk::area(&m), Tile::Left));
    assert!(!d.wins[0].maximized);
    d.return_to_previous(0);
    assert_eq!(d.wins[0].r, before);
    d.tile(&m, 0, Tile::Fill);
    assert!(d.wins[0].maximized);
    assert_eq!(d.wins[0].r, Desk::area(&m));
}

#[test]
fn tiles_cover_the_work_area_and_snap_zones_pick_edges() {
    let a = Rect { x: 0, y: 30, w: 1001, h: 601 };
    let (l, r) = (tile_rect(a, Tile::Left), tile_rect(a, Tile::Right));
    assert_eq!(l.w + r.w, a.w);
    assert_eq!(r.x, l.w);
    assert_eq!(tile_rect(a, Tile::Fill), a);
    assert_eq!(snap_zone(a, 2, 300, 8), Some(Tile::Left));
    assert_eq!(snap_zone(a, 500, 31, 8), Some(Tile::Fill));
    assert_eq!(snap_zone(a, 1000, 31, 8), Some(Tile::TopRight));
    assert_eq!(snap_zone(a, 500, 300, 8), None);
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    let before = d.wins[0].r;
    d.tile(&m, 0, Tile::Left);
    d.return_to_previous(0);
    assert_eq!(d.wins[0].r, before);
}

#[test]
fn split_view_shares_the_width_around_the_divider_and_keeps_both_sides_usable() {
    let a = Rect { x: 0, y: 30, w: 1001, h: 601 };
    let (l, r) = split_view_rects(a, 500, 8, 320);
    assert_eq!(l.w + 8 + r.w, a.w, "the two sides and the divider cover the area");
    assert_eq!((l.w, r.w), (496, 497), "an even split takes the usable width in halves");
    assert_eq!((l.x, r.x), (0, 504), "the trailing side starts after the divider");
    assert!(l.y == 30 && r.y == 30 && l.h == 601 && r.h == 601);
    // the ratio is clamped so neither side drops below the minimum width
    let (l, r) = split_view_rects(a, 50, 8, 320);
    assert_eq!((l.w, r.w), (320, 1001 - 8 - 320));
    let (l, r) = split_view_rects(a, 1000, 8, 320);
    assert_eq!((l.w, r.w), (1001 - 8 - 320, 320));
    // a narrow area splits in halves instead of honouring the minimum
    let (l, r) = split_view_rects(Rect { x: 0, y: 0, w: 300, h: 200 }, 900, 8, 320);
    assert_eq!((l.w, r.w), (146, 146));
    // degenerate width never goes negative
    let (l, r) = split_view_rects(Rect { x: 0, y: 0, w: 4, h: 10 }, 500, 8, 320);
    assert!(l.w >= 0 && r.w >= 0);
}

#[test]
fn moving_keeps_the_title_bar_reachable() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (900, 600));
    d.move_to(&m, 0, -5000, -5000);
    assert!(d.wins[0].r.y >= m.menubar);
    assert!(d.wins[0].r.x + d.wins[0].r.w >= m.title * 2);
    d.move_to(&m, 0, 9000, 9000);
    assert!(d.wins[0].r.x <= m.w - m.title * 2 && d.wins[0].r.y < m.h);
}

#[test]
fn resizing_respects_the_minimum_and_the_menu_bar() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Photos, (900, 600));
    let start = d.wins[0].r;
    d.resize_to(&m, 0, R | B, start, -5000, -5000);
    assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (m.min_w, m.min_h + title_of(AppId::Photos, &m)), "the desktop minimum plus the title bar");
    d.resize_to(&m, 0, L | T, start, -50, -5000);
    assert_eq!(d.wins[0].r.x, start.x - 50);
    assert!(d.wins[0].r.y >= m.menubar);
    assert_eq!(d.wins[0].r.y + d.wins[0].r.h, start.y + start.h, "the bottom edge stays put");
}

/// W2-23: a press latches the icon it hit, so the release opens it even when the row has re-centred under the
/// pointer since (or the layout fell back to rest); a different entry now in that slot does not count.
#[test]
fn a_dock_release_opens_the_icon_latched_at_the_press() {
    let row: Vec<Rect> = (0..5).map(|i| Rect { x: 100 + i * 58, y: 700, w: 48, h: 48 }).collect();
    // the icon under the pointer in the row magnified around it, as the press hit-tests it
    let live = super::super::desktop_dock::magnified_rects(&row, false, true, 175.0, 96, 150.0, 1.0);
    let k = live.iter().position(|r| r.contains(175.0, 740.0)).expect("an icon under the pointer");
    assert!(dock_release_hits(live[k], Some(row[k]), Some(7), 7, 175.0, 740.0), "the latched rect decides");
    assert!(!dock_release_hits(live[k], Some(live[k]), Some(8), 7, 400.0, 740.0), "a release elsewhere opens nothing");
    assert!(dock_release_hits(Rect { x: 0, y: 0, w: 1, h: 1 }, Some(row[2]), Some(7), 7, 230.0, 720.0), "the slot's current rect counts for the same entry");
    assert!(!dock_release_hits(Rect { x: 0, y: 0, w: 1, h: 1 }, Some(row[2]), Some(8), 7, 230.0, 720.0), "but not once another entry is there");
}

/// W2-23: pressing a Dock icon keeps the row magnified (it used to drop to the resting layout under the pointer);
/// a drag that moved, and any other grab, lay the row out at rest.
#[test]
fn a_pressed_dock_icon_keeps_the_dock_magnified_until_it_is_dragged() {
    let mut d = Desk::new();
    assert!(d.dock_grab_magnifies());
    let r = Rect { x: 0, y: 0, w: 48, h: 48 };
    d.grab = Some(Grab::DockDrag { from: 1, entry: 3, rect: r, x: 0.0, y: 0.0, at_x: 0.0, at_y: 0.0, moved: false });
    assert!(d.dock_grab_magnifies());
    d.grab = Some(Grab::DockDrag { from: 1, entry: 3, rect: r, x: 0.0, y: 0.0, at_x: 40.0, at_y: 0.0, moved: true });
    assert!(!d.dock_grab_magnifies());
    d.grab = Some(Grab::Content(AppId::Settings, 0.0));
    assert!(!d.dock_grab_magnifies());
}

/// W2-25: Cmd+` visits every window of the app in turn although each press only raises one, and Shift+Cmd+`
/// walks the other way; a window brought forward some other way starts a new walk.
#[test]
fn cmd_grave_walks_every_window_of_the_app_both_ways() {
    let (a, b, c) = (AppId::External(3), AppId::ExternalWindow(3, 1), AppId::ExternalWindow(3, 2));
    // raising only: the window raised goes in front, the rest keep their order
    let raise = |stack: &[AppId], w: AppId| { let mut s: Vec<AppId> = stack.iter().copied().filter(|x| *x != w).collect(); s.insert(0, w); s };
    let mut stack = vec![a, b, c];
    let mut cycle = None;
    let mut seen = vec![];
    for _ in 0..3 {
        let (order, sel) = window_cycle_next(cycle.take(), &stack, false).expect("three windows cycle");
        stack = raise(&stack, order[sel]);
        seen.push(order[sel]);
        cycle = Some((order, sel));
    }
    assert_eq!(seen, vec![b, c, a], "forwards: every window once, then back to the first");
    let (order, sel) = window_cycle_next(None, &[a, b, c], true).unwrap();
    assert_eq!(order[sel], c, "Shift+Cmd+` brings the last window forward");
    // the person clicked b: a new walk from the stack as it is now
    let (order, sel) = window_cycle_next(Some((vec![a, b, c], 0)), &[b, a, c], false).unwrap();
    assert_eq!(order[sel], a);
    assert!(window_cycle_next(None, &[a], false).is_none(), "one window: nothing to cycle");
    assert_eq!(cycle_step(0, 3, true), 2);
    assert_eq!(cycle_step(2, 3, false), 0);
}

#[test]
fn the_dock_is_centred_and_its_icons_do_not_overlap() {
    let m = metrics();
    let (bar, icons) = dock_rects(&m, 48, 10, 10, 5, Some(1), 14);
    assert_eq!(bar.x * 2 + bar.w, m.w);
    for p in icons.windows(2) {
        assert!(p[0].x + p[0].w < p[1].x);
    }
    assert!(icons.iter().all(|i| bar.contains(i.x as f32, i.y as f32)));
}
