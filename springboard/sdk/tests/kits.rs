//! API parity guard: the-kit models an app can reach through `bifrost_app::controls` (the SDK's one import).
use bifrost_app::controls::{detect, keys::KeyCombo, menu::Menu, uttype};

#[test]
fn uniform_type_identifiers_are_reachable_from_the_sdk() {
    let id = uttype::type_for_extension("Report.PDF").unwrap();
    assert_eq!(uttype::description(id), "PDF document");
    assert_eq!(uttype::type_for_mime("application/pdf"), Some(id));
    assert!(uttype::conforms("public.rust-source", "public.text"));
    assert_eq!(uttype::preferred_extension("public.jpeg"), Some("jpg"));
}

#[test]
fn key_equivalents_and_menus_are_reachable_from_the_sdk() {
    assert_eq!(KeyCombo::parse("cmd+shift+z").unwrap().to_string(), "\u{21e7}\u{2318}Z");
    assert!(Menu::new(Vec::new()).selected().is_none());
}

#[test]
fn data_detectors_are_reachable_from_the_sdk() {
    let text = "Sjá https://bifrost.olibuijr.com, fundur 2026-10-09 og 555 1234.";
    let found: Vec<(detect::Kind, &str)> = detect::find(text, &[detect::Kind::Link, detect::Kind::Date, detect::Kind::Address, detect::Kind::Phone]).iter().map(|m| (m.kind, m.text(text))).collect();
    assert_eq!(found, vec![(detect::Kind::Link, "https://bifrost.olibuijr.com"), (detect::Kind::Date, "2026-10-09"), (detect::Kind::Phone, "555 1234")]);
}

#[test]
fn drawing_a_kit_control_publishes_it_to_the_element_map() {
    use bifrost_app::controls::{ax, Rect, Role};
    // an app's own views declare themselves the same way the kit's controls do
    ax::begin_frame();
    ax::record(Role::Button, "New Note", "", Rect { x: 8., y: 8., w: 80., h: 28. }, true);
    ax::record(Role::TextField, "Search", "milk", Rect { x: 100., y: 8., w: 120., h: 28. }, true);
    let nodes = ax::take_frame();
    let mut map = ax::Map::new();
    map.add_nodes("app/is.olibuijr.notes", &nodes);
    assert!(map.get("app/is.olibuijr.notes/button/new-note").is_some_and(|e| e.enabled));
    assert_eq!(map.get("app/is.olibuijr.notes/field/search").unwrap().value, "milk");
}

#[test]
fn grouped_kit_controls_publish_a_parent_child_tree() {
    use bifrost_app::controls::{ax, Rect, Role};
    let r = Rect { x: 0., y: 0., w: 20., h: 20. };
    ax::begin_frame();
    let list = ax::begin_group(Role::List, "Sidebar", r);
    let row = ax::record(Role::Row, "Inbox", "", r, true);
    ax::end_group();
    let links = ax::take_links();
    let nodes = ax::take_frame();
    assert_eq!(ax::parent_of(&links, row), Some(list));
    assert_eq!(ax::children_of(&nodes, &links, None), vec![list]);
}

#[test]
fn uttype_conformance_follows_the_hierarchy() {
    assert!(uttype::conforms("public.jpeg", "public.image"));
    assert!(uttype::conforms("public.jpeg", "public.jpeg"), "a type conforms to itself");
    assert!(!uttype::conforms("public.image", "public.jpeg"), "conformance is directional");
    assert!(!uttype::conforms("public.rust-source", "public.image"));
}

#[test]
fn undo_manager_groups_steps_and_titles_them() {
    use bifrost_app::controls::undo::UndoManager;
    let (mut m, mut s) = (UndoManager::<Vec<i32>>::new(10), vec![]);
    m.begin_group("Typing");
    for v in 1..=3 {
        s.push(v);
        m.register("", |s| { s.pop(); }, move |s| s.push(v));
    }
    m.end_group();
    assert_eq!(m.undo_title().as_deref(), Some("Undo Typing"));
    assert_eq!(m.undo(&mut s).as_deref(), Some("Typing"));
    assert!(s.is_empty(), "the whole group is one undo step");
    assert!(m.can_redo());
    m.redo(&mut s);
    assert_eq!(s, vec![1, 2, 3]);
}

#[test]
fn formatters_match_foundation_output() {
    use bifrost_app::controls::format;
    assert_eq!(format::byte_count(1_500_000), "1.5 MB");
    assert_eq!(format::byte_count(1), "1 byte");
    assert_eq!(format::duration(3723), "1:02:03");
    assert_eq!(format::relative(-90), "1 minute ago");
    assert_eq!(format::relative(7200), "in 2 hours");
    assert_eq!(format::grouped(-1234567), "-1,234,567");
}

#[test]
fn fuzzy_matching_is_an_ordered_case_insensitive_subsequence() {
    use bifrost_app::controls::fuzzy;
    assert!(fuzzy::fuzzy_match("stg", "Settings").is_some());
    assert!(fuzzy::fuzzy_match("gts", "Settings").is_none(), "order matters");
    assert_eq!(fuzzy::rank("", ["a", "b"]).len(), 2, "an empty query keeps everything");
}

#[test]
fn navigation_stack_tracks_titles_and_back() {
    use bifrost_app::controls::nav::NavigationStack;
    let mut n = NavigationStack::new("Lists", 0);
    assert!(!n.can_go_back());
    n.push("Groceries", 1);
    n.push("Milk", 2);
    assert_eq!((n.title(), n.back_title(), n.depth()), ("Milk", Some("Groceries"), 3));
    assert_eq!(n.path(), vec!["Lists", "Groceries", "Milk"]);
    assert_eq!(n.pop(), Some(("Milk".to_string(), 2)));
    assert_eq!(n.pop_to_root(), 1);
    assert_eq!(n.pop(), None, "the root cannot be popped");
}

#[test]
fn split_view_keeps_both_minimum_widths() {
    use bifrost_app::controls::split::SplitView;
    let mut s = SplitView::new(800.0, 250.0, 180.0, 300.0);
    s.drag_to(50.0);
    assert_eq!(s.leading(), 180.0);
    s.drag_to(790.0);
    assert_eq!(s.leading(), 500.0);
    assert!(s.toggle(), "toggling collapses the sidebar");
    assert_eq!(s.leading(), 0.0);
}

#[test]
fn values_controls_clamp_and_snap() {
    use bifrost_app::controls::values::{Slider, Stepper};
    let mut st = Stepper::new(9.0, 0.0, 10.0, 1.0);
    assert!(st.nudge(1));
    assert!(!st.nudge(1), "stops at max");
    assert_eq!(st.value, 10.0);
    let sl = Slider::new(5.0, 5.0, 9.0, None);
    assert_eq!(sl.fraction(), 0.0, "a degenerate range does not divide by zero");
}

#[test]
fn constraint_solver_satisfies_required_equalities() {
    use bifrost_app::controls::constraint::{Expr, Rel, Solver, Strength};
    let mut s = Solver::new();
    let x = s.var();
    s.add(Expr::var(x).minus(&Expr::constant(10.0)), Rel::Eq, Strength::REQUIRED).unwrap();
    assert_eq!(s.value(x), 10.0);
    assert!(s.add(Expr::var(x).minus(&Expr::constant(20.0)), Rel::Eq, Strength::REQUIRED).is_err(), "conflicting required constraints are rejected");
    assert_eq!(s.value(x), 10.0, "the solver is left as it was");
}
