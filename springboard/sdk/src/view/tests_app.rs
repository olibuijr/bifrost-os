//! Contract tests for `app.rs` (NativeApplication / NativeViewController glue) and `scroll.rs` (NativeScrollView).
use super::*;
use crate::kit::Font;
use crate::proto::AxAction;
use crate::view::app::{action_for, ControllerHost, MenuEntry, ViewApp, ViewController};
use crate::view::scroll::ScrollView;
use bifrost_ui::responder::Responder;

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[derive(Default)]
struct Model {
    log: Vec<String>,
}
impl Responder for Model {
    fn perform(&mut self, action: &str) -> bool {
        self.log.push(action.into());
        action == "quit"
    }
    fn validate(&self, action: &str) -> Option<bool> {
        (action == "quit").then_some(true)
    }
}

/// A plain view with a fixed intrinsic height, standing in for a tall document.
struct Tall {
    base: ViewBase,
    h: f32,
}
impl Tall {
    fn new(h: f32) -> Self {
        Tall { base: ViewBase::default(), h }
    }
}
impl View for Tall {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, self.h)
    }
}

fn scroller(doc_h: f32, view_h: f32) -> ScrollView {
    let mut sv = ScrollView::new(Box::new(Tall::new(doc_h)));
    sv.layout(&ui(), rect(0.0, 0.0, 200.0, view_h));
    sv
}

fn drag(sv: &mut ScrollView, from: f32, to: f32) {
    sv.handle(&ViewEvent::Pointer(Event::Down(10.0, from)));
    sv.handle(&ViewEvent::Pointer(Event::Move(10.0, (from + to) / 2.0)));
    sv.handle(&ViewEvent::Pointer(Event::Move(10.0, to)));
}

// ---- action_for -------------------------------------------------------------------------------------------------

#[test]
fn standard_edit_commands_map_to_their_cocoa_selector_names() {
    assert_eq!(action_for(Command::Cut), "cut");
    assert_eq!(action_for(Command::Copy), "copy");
    assert_eq!(action_for(Command::Paste), "paste");
    assert_eq!(action_for(Command::SelectAll), "selectAll");
    assert_eq!(action_for(Command::Undo), "undo");
    assert_eq!(action_for(Command::Redo), "redo");
}

// ---- ViewController / ControllerHost -----------------------------------------------------------------------------

#[derive(Default)]
struct Ctl {
    loads: u32,
    did_load: u32,
    layouts: u32,
    performed: Vec<String>,
}
impl ViewController for Ctl {
    fn load_view(&mut self) -> Box<dyn View> {
        self.loads += 1;
        Box::new(Tall::new(0.0))
    }
    fn view_did_load(&mut self, _view: &mut dyn View) {
        self.did_load += 1;
    }
    fn view_did_layout(&mut self, view: &mut dyn View, _ui: &Ui) {
        self.layouts += 1;
        assert_eq!(view.frame().w, 120.0, "the subtree is laid out before viewDidLayout");
    }
    fn perform(&mut self, action: &str) -> bool {
        self.performed.push(action.into());
        action == "ctl"
    }
    fn validate(&self, action: &str) -> Option<bool> {
        (action == "ctl").then_some(true)
    }
}

#[test]
fn a_controller_host_loads_its_view_once_and_reports_did_load() {
    let host = ControllerHost::new(Ctl::default());
    assert_eq!((host.controller().loads, host.controller().did_load), (1, 1));
    assert_eq!(host.children().len(), 1);
}

#[test]
fn a_controller_host_lays_out_its_view_then_calls_view_did_layout() {
    let mut host = ControllerHost::new(Ctl::default());
    host.layout(&ui(), rect(0.0, 0.0, 120.0, 80.0));
    assert_eq!(host.frame(), rect(0.0, 0.0, 120.0, 80.0));
    assert_eq!(host.children()[0].frame(), rect(0.0, 0.0, 120.0, 80.0));
    host.layout(&ui(), rect(0.0, 0.0, 120.0, 40.0));
    assert_eq!(host.controller().layouts, 2);
    assert_eq!(host.children()[0].frame().h, 40.0);
}

#[test]
fn a_controller_host_forwards_perform_and_validate_to_the_controller() {
    let mut host = ControllerHost::new(Ctl::default());
    assert!(host.perform("ctl"));
    assert!(!host.perform("other"));
    assert_eq!(host.controller().performed, vec!["ctl", "other"]);
    assert_eq!(host.validate("ctl"), Some(true));
    assert_eq!(host.validate("other"), None);
    host.controller_mut().performed.clear();
    assert!(host.controller().performed.is_empty());
}

// ---- MenuEntry ---------------------------------------------------------------------------------------------------

#[test]
fn a_menu_entry_builder_sets_key_and_checked_and_defaults_to_plain() {
    let plain = MenuEntry::new("Open", "open");
    assert_eq!((plain.title.as_str(), plain.action.as_str(), plain.key.as_str(), plain.checked, plain.separator), ("Open", "open", "", false, false));
    let e = MenuEntry::new("Wrap", "wrap").key("cmd+w").checked(true);
    assert_eq!((e.key.as_str(), e.checked), ("cmd+w", true));
}

#[test]
fn a_menu_separator_is_marked_and_has_no_action() {
    let s = MenuEntry::separator();
    assert!(s.separator);
    assert!(s.action.is_empty() && s.title.is_empty());
}

#[test]
fn the_edit_menu_holds_the_six_standard_items_with_their_key_equivalents() {
    let (title, items) = MenuEntry::edit_menu();
    assert_eq!(title, "Edit");
    let actions: Vec<&str> = items.iter().filter(|e| !e.separator).map(|e| e.action.as_str()).collect();
    assert_eq!(actions, vec!["undo", "redo", "cut", "copy", "paste", "selectAll"]);
    let keys: Vec<&str> = items.iter().filter(|e| !e.separator).map(|e| e.key.as_str()).collect();
    assert_eq!(keys, vec!["cmd+z", "cmd+shift+z", "cmd+x", "cmd+c", "cmd+v", "cmd+a"]);
    assert_eq!(items.iter().filter(|e| e.separator).count(), 1);
}

// ---- ViewApp -----------------------------------------------------------------------------------------------------

fn app() -> ViewApp<Model> {
    let mut root = Tall::new(0.0);
    root.set_frame(rect(0.0, 0.0, 200.0, 100.0));
    ViewApp::new(root, Model::default())
}

#[test]
fn a_new_view_app_has_a_window_a_model_and_no_menus() {
    let a = app();
    assert!(a.main_menu().is_empty());
    assert!(a.state.log.is_empty());
    assert_eq!(a.controls_tree().len(), a.controls().len());
}

#[test]
fn the_window_lays_out_its_root_in_the_size_it_is_given() {
    let mut a = app();
    a.window.layout(&ui(), rect(0.0, 0.0, 320.0, 240.0));
    a.window.layout(&ui(), rect(0.0, 0.0, 640.0, 480.0));
    assert!(a.window.hit_test(600.0, 400.0).is_some());
    assert_eq!(a.window.hit_test(700.0, 400.0), None);
}

#[test]
fn main_menu_numbers_items_from_one_skipping_separators_and_keeps_titles() {
    let mut a = app();
    a.set_menus(vec![MenuEntry::edit_menu(), ("File".into(), vec![MenuEntry::new("Quit", "quit").key("cmd+q")])]);
    let m = a.main_menu();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].title, "Edit");
    let ids: Vec<u32> = m[0].items.iter().filter(|i| !i.separator).map(|i| i.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
    assert!(m[0].items.iter().any(|i| i.separator));
    assert_eq!(m[1].items[0].id, 7);
    assert_eq!(m[1].items[0].key, "cmd+q");
}

#[test]
fn main_menu_enables_items_by_responder_chain_validation() {
    let mut a = app();
    a.set_menus(vec![("File".into(), vec![MenuEntry::new("Quit", "quit"), MenuEntry::new("Nothing", "nothing").checked(true)])]);
    let items = a.main_menu().remove(0).items;
    assert!(items[0].enabled);
    assert!(!items[1].enabled);
    assert!(items[1].checked);
}

#[test]
fn choosing_a_menu_item_sends_its_action_to_the_model() {
    let mut a = app();
    a.set_menus(vec![MenuEntry::edit_menu(), ("File".into(), vec![MenuEntry::new("Quit", "quit")])]);
    a.menu_action(7);
    assert_eq!(a.state.log, vec!["quit".to_string()]);
}

#[test]
fn an_unknown_menu_item_id_does_nothing() {
    let mut a = app();
    a.set_menus(vec![("File".into(), vec![MenuEntry::new("Quit", "quit")])]);
    assert!(!a.menu_action(0));
    assert!(!a.menu_action(99));
    assert!(a.state.log.is_empty());
}

#[test]
fn commands_reflect_what_the_responder_chain_can_do() {
    let a = app();
    assert!(!a.commands().supports(Command::Undo));
    assert!(!a.commands().supports(Command::Copy));
}

#[test]
fn an_idle_app_has_no_animation_wake_and_sends_no_actions_on_flush() {
    let mut a = app();
    assert_eq!(a.next_wake(1000.0), None);
    a.flush_animation_actions();
    assert!(a.state.log.is_empty());
}

#[test]
fn accessibility_actions_on_an_unknown_element_are_refused() {
    let mut a = app();
    assert!(!a.ax_set(u64::MAX, AxAction::Press));
    assert!(!a.ax_set(u64::MAX, AxAction::SetValue("x".into())));
    assert!(!a.ax_set(u64::MAX, AxAction::Focus));
}

// ---- ScrollView --------------------------------------------------------------------------------------------------

#[test]
fn a_scroll_view_starts_at_the_top_with_the_document_as_wide_as_itself() {
    let sv = scroller(300.0, 100.0);
    assert_eq!(sv.content_offset(), 0.0);
    assert_eq!(sv.frame(), rect(0.0, 0.0, 200.0, 100.0));
    assert_eq!(sv.children()[0].frame(), rect(0.0, 0.0, 200.0, 300.0));
}

#[test]
fn the_content_size_is_the_document_intrinsic_height_or_the_frame_when_it_has_none() {
    assert_eq!(scroller(300.0, 100.0).max_offset(), 200.0);
    assert_eq!(scroller(0.0, 100.0).max_offset(), 0.0);
    assert_eq!(scroller(60.0, 100.0).max_offset(), 0.0, "a short document never scrolls");
}

#[test]
fn resizing_the_scroll_view_recomputes_the_maximum_offset() {
    let mut sv = scroller(300.0, 100.0);
    sv.layout(&ui(), rect(0.0, 0.0, 200.0, 250.0));
    assert_eq!(sv.max_offset(), 50.0);
    sv.layout(&ui(), rect(0.0, 0.0, 200.0, 400.0));
    assert_eq!(sv.max_offset(), 0.0);
}

#[test]
fn scroll_to_clamps_the_offset_between_zero_and_the_maximum() {
    let mut sv = scroller(300.0, 100.0);
    sv.scroll_to(120.0);
    assert_eq!(sv.content_offset(), 120.0);
    sv.scroll_to(9999.0);
    assert_eq!(sv.content_offset(), 200.0);
    sv.scroll_to(-50.0);
    assert_eq!(sv.content_offset(), 0.0);
}

#[test]
fn scroll_to_marks_the_view_dirty_and_shifts_the_children_by_the_offset() {
    let mut sv = scroller(300.0, 100.0);
    sv.base_mut().dirty = false;
    sv.scroll_to(40.0);
    assert!(sv.base().dirty);
    assert_eq!(sv.child_offset(), (0.0, 40.0));
}

#[test]
fn a_scroll_view_clips_its_children_and_repaints_as_one() {
    let sv = scroller(300.0, 100.0);
    assert!(sv.clips_children());
    assert!(sv.opaque_backdrop());
}

#[test]
fn document_view_downcasts_to_the_concrete_type() {
    let mut sv = scroller(300.0, 100.0);
    assert!(sv.document_view::<Tall>().is_some());
    assert!(sv.document_view::<Label>().is_none());
    sv.document_view_mut::<Tall>().unwrap().h = 10.0;
    assert_eq!(sv.document_view::<Tall>().unwrap().h, 10.0);
}

#[test]
fn dragging_the_content_up_scrolls_down_and_marks_the_view_dirty() {
    let mut sv = scroller(300.0, 100.0);
    sv.base_mut().dirty = false;
    drag(&mut sv, 80.0, 20.0);
    assert!(sv.content_offset() > 0.0);
    assert!(sv.base().dirty);
}

#[test]
fn pointer_events_and_cancel_are_consumed() {
    let mut sv = scroller(300.0, 100.0);
    assert!(sv.handle(&ViewEvent::Pointer(Event::Down(10.0, 50.0))));
    assert!(sv.handle(&ViewEvent::Cancel));
    assert!(sv.handle(&ViewEvent::Pointer(Event::Up(10.0, 50.0))));
}

#[test]
fn a_document_shorter_than_the_view_has_nothing_to_scroll() {
    let mut sv = scroller(60.0, 100.0);
    assert_eq!(sv.max_offset(), 0.0);
    sv.scroll_to(30.0);
    assert_eq!(sv.content_offset(), 0.0);
}

#[test]
fn a_jump_stops_a_fling_in_progress() {
    let mut sv = scroller(1000.0, 100.0);
    drag(&mut sv, 90.0, 10.0);
    sv.handle(&ViewEvent::Pointer(Event::Up(10.0, 10.0)));
    sv.scroll_to(500.0);
    sv.tick(0.5);
    sv.tick(1.0);
    assert_eq!(sv.content_offset(), 500.0);
}
