 use super::*;
 #[test] fn context_roundtrip_and_limits(){let packet=ToShell::ContextMenu{x:12.,y:34.,items:vec![ContextItem{id:7,label:"Open".into(),enabled:true}]};assert_eq!(ToShell::decode(&packet.encode()),Ok(packet));for message in [ToApp::Context{x:12.,y:34.},ToApp::ContextAction(7)]{assert_eq!(ToApp::decode(&message.encode()),Ok(message));}let bad=ToShell::ContextMenu{x:f32::NAN,y:0.,items:vec![]};assert_eq!(ToShell::decode(&bad.encode()),Err(DecodeError::BadValue));let large=ToShell::ContextMenu{x:0.,y:0.,items:(0..100).map(|id|ContextItem{id,label:"x".repeat(10000),enabled:true}).collect()};let bytes=large.encode();assert!(bytes.len()<MAX_PACKET);let ToShell::ContextMenu{items,..}=ToShell::decode(&bytes).unwrap()else{panic!()};assert_eq!(items.len(),32);}
#[test]
fn context_items_carry_separators_and_key_equivalents_inside_the_wire_fields() {
    let plain = ContextItem::new(7, "Open");
    assert_eq!(plain, ContextItem { id: 7, label: "Open".into(), enabled: true });
    assert_eq!((plain.title(), plain.key_combo(), plain.is_separator()), ("Open", None, false));
    let keyed = ContextItem::new(9, "Get Info").key("cmd+i").enabled(false);
    assert_eq!((keyed.title(), keyed.key_combo(), keyed.enabled), ("Get Info", Some("cmd+i"), false));
    assert_eq!(ContextItem::new(9, "Get Info").key("cmd+i").key("cmd+j").label, "Get Info\tcmd+j", "a second key replaces the first");
    let line = ContextItem::separator();
    assert!(line.is_separator() && !line.enabled);
    assert!(!ContextItem::new(0, "Remove").is_separator(), "id 0 with a title is a row");
    // an app built before keys and separators sends plain titles: they stay rows without a key
    assert_eq!((ContextItem { id: 3, label: "Remove".into(), enabled: true }).key_combo(), None);
    let packet = ToShell::ContextMenu { x: 1., y: 2., items: vec![keyed.clone(), line.clone(), plain.clone()] };
    let ToShell::ContextMenu { items, .. } = ToShell::decode(&packet.encode()).unwrap() else { panic!() };
    assert_eq!(items, vec![keyed, line, plain], "separators and keys survive the wire");
}
#[test]
fn app_menus_roundtrip_and_limits() {
    let menus = vec![AppMenu { title: "Note".into(), items: vec![
        AppMenuItem::new(1, "New Note").key("cmd+n"),
        AppMenuItem::separator(),
        AppMenuItem::new(2, "Pin").checked(true).enabled(false),
        AppMenuItem::submenu("Sort By"),
        AppMenuItem::new(3, "Name").depth(1),
    ] }];
    let packet = ToShell::AppMenus(menus);
    assert_eq!(ToShell::decode(&packet.encode()), Ok(packet));
    let ds = ToShell::DocumentState { title: "Untitled 2".into(), edited: true, represented: "a.txt".into() };
    assert_eq!(ToShell::decode(&ds.encode()), Ok(ds));
    let pb = ToShell::PasteboardSet { items: vec![("public.html".into(), b"<b>x</b>".to_vec())] };
    assert_eq!(ToShell::decode(&pb.encode()), Ok(pb));
    let pa = ToApp::Pasteboard { change_count: 3, items: vec![("public.png".into(), vec![1, 2, 3])] };
    assert_eq!(ToApp::decode(&pa.encode()), Ok(pa));
    assert_eq!(ToApp::decode(&ToApp::MenuAction(9).encode()), Ok(ToApp::MenuAction(9)));
    assert_eq!(ToShell::decode(&ToShell::AppMenus(vec![]).encode()), Ok(ToShell::AppMenus(vec![])));
    let many = ToShell::AppMenus((0..20).map(|_| AppMenu { title: "x".into(), items: (0..200).map(|i| AppMenuItem::new(i, &"t".repeat(1000))).collect() }).collect());
    let bytes = many.encode();
    assert!(bytes.len() < MAX_PACKET);
    let ToShell::AppMenus(m) = ToShell::decode(&bytes).unwrap() else { panic!() };
    assert!(!m.is_empty() && m.len() <= MAX_APP_MENUS);
    assert!(m[0].items.len() < 200, "the packet size cuts the first menu");
    assert_eq!(m[0].items[0].title.chars().count(), MAX_APP_MENU_TITLE);
}

fn level(n: usize, depth: u8) -> Vec<AppMenuItem> {
    (0..n).map(|i| AppMenuItem::new(i as u32 + 1, "x").depth(depth)).collect()
}

/// Each submenu has a budget of its own: a Font submenu with every family does not push out the items after it.
#[test]
fn menu_items_are_counted_per_level() {
    let mut items = vec![AppMenuItem::submenu("Font")];
    items.extend(level(200, 1));
    items.push(AppMenuItem::submenu("Text"));
    items.extend(level(10, 1));
    items.extend(level(40, 0));
    let menu = AppMenu { title: "Format".into(), items };
    assert_eq!(menu.dropped_items(), 0, "{} items in all, but no level is over the cap", menu.items.len());
    let (kept, _) = menu.kept_items();
    assert_eq!(kept.len(), menu.items.len());
    // one level over the cap: only that level loses items, the next submenu and the top level keep theirs
    let mut items = vec![AppMenuItem::submenu("Font")];
    items.extend(level(MAX_APP_MENU_ITEMS + 5, 1));
    items.push(AppMenuItem::submenu("Text"));
    items.extend(level(3, 1));
    let menu = AppMenu { title: "Format".into(), items };
    let (kept, dropped) = menu.kept_items();
    assert_eq!(dropped, 5);
    assert_eq!(kept.iter().filter(|i| i.title == "Text").count(), 1, "the Text submenu is kept");
    assert_eq!(kept.iter().rev().take_while(|i| i.depth == 1).count(), 3, "with all its items");
}

/// A submenu header past its level's cap goes with its children; items at the parent level after it are counted again.
#[test]
fn a_dropped_submenu_takes_its_children_along() {
    let mut items = level(MAX_APP_MENU_ITEMS, 0);
    items.push(AppMenuItem::submenu("Too Many"));
    items.extend(level(4, 1));
    let menu = AppMenu { title: "Long".into(), items };
    let (kept, dropped) = menu.kept_items();
    assert_eq!((kept.len(), dropped), (MAX_APP_MENU_ITEMS, 5));
    assert!(kept.iter().all(|i| i.depth == 0));
}

/// Menus that do not fit one packet go in `AppMenusMore` packets after the first, whole; what is cut is reported.
#[test]
fn long_menus_are_split_over_packets_and_nothing_is_cut_silently() {
    let family = |i: usize| AppMenuItem::new(i as u32 + 1, &format!("Family Name Number {i:03}")).depth(1);
    let big = |title: &str| {
        let mut items = vec![AppMenuItem::submenu("Font")];
        items.extend((0..250).map(family));
        AppMenu { title: title.into(), items }
    };
    let menus = vec![AppMenu { title: "File".into(), items: level(10, 0) }, big("Format"), big("Preferences"), AppMenu { title: "Window".into(), items: level(3, 0) }];
    let out = app_menu_packets(&menus);
    assert!(out.packets.len() >= 2, "too much for one packet");
    assert!(out.dropped.is_empty(), "everything fits in some packet: {:?}", out.dropped);
    let mut got = Vec::new();
    for (k, p) in out.packets.iter().enumerate() {
        assert!(p.len() <= MAX_PACKET);
        match ToShell::decode(p).unwrap() {
            ToShell::AppMenus(m) if k == 0 => got.extend(m),
            ToShell::AppMenusMore(m) if k > 0 => got.extend(m),
            other => panic!("packet {k} is {other:?}"),
        }
    }
    assert_eq!(got, menus, "the menus arrive whole and in order");
    // one menu bigger than a packet is cut, and the cut is reported
    let huge = AppMenu { title: "Huge".into(), items: (0..MAX_APP_MENU_ITEMS).map(|i| AppMenuItem::new(i as u32, &"t".repeat(MAX_APP_MENU_TITLE))).collect() };
    let out = app_menu_packets(&[huge]);
    assert_eq!(out.packets.len(), 1);
    assert!(matches!(out.dropped.as_slice(), [(t, n)] if t == "Huge" && *n > 0), "{:?}", out.dropped);
    // a ninth menu is reported with all its items
    let nine: Vec<AppMenu> = (0..MAX_APP_MENUS + 1).map(|i| AppMenu { title: format!("M{i}"), items: level(2, 0) }).collect();
    assert_eq!(app_menu_packets(&nine).dropped, vec![(format!("M{MAX_APP_MENUS}"), 2)]);
    // no menus: one packet that clears them
    let out = app_menu_packets(&[]);
    assert_eq!(out.packets.len(), 1);
    assert_eq!(ToShell::decode(&out.packets[0]), Ok(ToShell::AppMenus(vec![])));
}
#[test]
fn the_apps_own_print_item_keeps_its_flag_over_the_wire() {
    let print = AppMenuItem::print(7);
    assert_eq!((print.title.as_str(), print.key.as_str(), print.prints), ("Print\u{2026}", "cmd+p", true));
    assert!(!AppMenuItem::new(8, "Print Address Labels\u{2026}").prints, "only the declared item is the app's Print");
    let packet = ToShell::AppMenus(vec![AppMenu { title: "File".into(), items: vec![print.clone().enabled(false), AppMenuItem::new(8, "Export")] }]);
    let bytes = packet.encode();
    assert_eq!(ToShell::decode(&bytes), Ok(packet));
    // a marker item that hides the shell's Print… keeps its flag over the wire
    let marker = ToShell::AppMenus(vec![AppMenu { title: "File".into(), items: vec![AppMenuItem::no_print()] }]);
    assert_eq!(ToShell::decode(&marker.encode()), Ok(marker));
    assert!(AppMenuItem::no_print().separator && !AppMenuItem::print(1).no_print, "a marker, not the app's own Print");
    // a flag byte past the six known bits is refused
    let flags_at = 4 + 4 + (4 + 4) + 4 + 4 + (4 + "Print\u{2026}".len()) + (4 + 5);
    assert_eq!(bytes[flags_at], 16, "disabled, with the print bit");
    let mut bad = bytes.clone();
    bad[flags_at] = 64;
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
}
