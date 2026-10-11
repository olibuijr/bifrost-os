use super::*;
use crate::kit::Font;
use crate::layout::ROW_PT;
use bifrost_ui::{values, Role};

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[derive(Default)]
struct Model {
    n: i32,
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

/// A container in the chain that implements "ping" and forbids "locked".
struct Probe {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
    performed: Vec<String>,
}
impl View for Probe {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }
    fn perform(&mut self, action: &str) -> bool {
        self.performed.push(action.into());
        action == "ping"
    }
    fn validate(&self, action: &str) -> Option<bool> {
        match action {
            "ping" => Some(true),
            "locked" => Some(false),
            _ => None,
        }
    }
}

/// root Probe (0,0,200,100) > Button A (10,10,60,28), Button B (80,10,60,28), Label (10,50,100,20).
fn window() -> (Window<Model>, [u64; 4]) {
    let (mut a, mut b, mut l) = (Button::new("A", "a"), Button::new("B", "b"), Label::new("Hello"));
    a.set_frame(rect(10.0, 10.0, 60.0, 28.0));
    b.set_frame(rect(80.0, 10.0, 60.0, 28.0));
    l.set_frame(rect(10.0, 50.0, 100.0, 20.0));
    let ids = [0, a.id(), b.id(), l.id()];
    let mut root = Probe { base: ViewBase::default(), kids: vec![Box::new(a), Box::new(b), Box::new(l)], performed: Vec::new() };
    root.set_frame(rect(0.0, 0.0, 200.0, 100.0));
    let mut ids = ids;
    ids[0] = root.id();
    (Window::new(root, 10), ids)
}

fn click(w: &mut Window<Model>, m: &mut Model, x: f32, y: f32) {
    w.event(&ViewEvent::Pointer(Event::Down(x, y)), m);
    w.event(&ViewEvent::Pointer(Event::Up(x, y)), m);
}

#[test]
fn hit_test_finds_the_deepest_visible_view() {
    let (mut w, [root, a, _, l]) = window();
    assert_eq!(w.hit_test(20.0, 20.0), Some(a));
    assert_eq!(w.hit_test(20.0, 55.0), Some(l));
    assert_eq!(w.hit_test(150.0, 90.0), Some(root));
    assert_eq!(w.hit_test(300.0, 20.0), None);
    w.view_mut::<Button>(a).unwrap().base_mut().hidden = true;
    assert_eq!(w.hit_test(20.0, 20.0), Some(root));
}

#[test]
fn a_click_sends_the_action_up_the_chain_to_the_model() {
    let (mut w, [root, a, ..]) = window();
    let mut m = Model::default();
    click(&mut w, &mut m, 20.0, 20.0);
    assert_eq!(w.take_sent(), vec![(a, "a".to_string())]);
    // nobody but the app saw "a": button, then the Probe, then the model
    assert_eq!(m.log, vec!["a".to_string()]);
    assert_eq!(w.first_responder(), Some(a));
    // releasing outside the button does not send
    w.event(&ViewEvent::Pointer(Event::Down(20.0, 20.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(150.0, 90.0)), &mut m);
    assert!(w.take_sent().is_empty());
    assert_eq!(w.send("ping", &mut m), Some(Handler::View(root)));
}

#[test]
fn actions_travel_first_responder_then_ancestors_then_window_then_app() {
    let (mut w, [root, a, ..]) = window();
    let mut m = Model::default();
    assert!(w.make_first_responder(Some(a)));
    assert_eq!(w.send("ping", &mut m), Some(Handler::View(root)));
    assert_eq!(w.send("quit", &mut m), Some(Handler::App));
    assert_eq!(w.send("nobody-knows", &mut m), None);
    assert_eq!(m.log, vec!["quit".to_string(), "nobody-knows".to_string()]);
    assert_eq!(w.view::<Probe>(root).unwrap().performed, vec!["ping", "quit", "nobody-knows"]);
}

#[test]
fn validate_asks_the_first_responder_outwards() {
    let (mut w, [_, a, ..]) = window();
    let m = Model::default();
    w.make_first_responder(Some(a));
    assert!(w.is_enabled("ping", &m));
    assert!(!w.is_enabled("locked", &m));
    assert!(w.is_enabled("quit", &m));
    assert!(!w.is_enabled("unknown", &m));
    assert!(!w.is_enabled("undo", &m));
    assert!(!w.commands(&m).supports(Command::Undo));
}

#[test]
fn tab_walks_the_key_view_loop_and_wraps() {
    let (mut w, [_, a, b, _]) = window();
    let mut m = Model::default();
    assert_eq!(w.key_view_loop(), vec![a, b]);
    let tab = ViewEvent::Key(KeyInput::Code(9));
    w.event(&tab, &mut m);
    assert_eq!(w.first_responder(), Some(a));
    w.event(&tab, &mut m);
    assert_eq!(w.first_responder(), Some(b));
    w.event(&tab, &mut m);
    assert_eq!(w.first_responder(), Some(a));
    w.event(&ViewEvent::Key(KeyInput::chord(15, CHORD_SHIFT)), &mut m);
    assert_eq!(w.first_responder(), Some(b));
    assert!(w.view::<Button>(b).unwrap().base().focused && !w.view::<Button>(a).unwrap().base().focused);
    // a disabled button leaves the loop, a label never joins it
    w.view_mut::<Button>(b).unwrap().base_mut().enabled = false;
    assert_eq!(w.key_view_loop(), vec![a]);
}

#[test]
fn return_on_the_focused_button_sends_its_action() {
    let (mut w, [_, _, b, _]) = window();
    let mut m = Model::default();
    w.make_first_responder(Some(b));
    w.event(&ViewEvent::Key(KeyInput::Code(13)), &mut m);
    assert_eq!(w.take_sent(), vec![(b, "b".to_string())]);
}

#[test]
fn ax_nodes_come_in_tree_order_with_parents() {
    let (w, [_, a, b, l]) = window();
    let nodes = w.ax_nodes();
    assert_eq!(nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![a, b, l]);
    assert_eq!(nodes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(), vec!["A", "B", "Hello"]);
    assert_eq!(nodes[0].role, Role::Button);
    assert_eq!(nodes[2].role, Role::StaticText);
    assert!(w.ax_tree().iter().all(|e| e.depth == 0 && e.parent.is_none()));
}

#[test]
fn containers_that_are_not_elements_add_no_depth() {
    let mut inner = Stack::vstack(0.0, vec![Box::new(Button::new("Inner", "i"))]);
    inner.set_frame(rect(0.0, 0.0, 10.0, 10.0));
    let w: Window<Model> = Window::new(Container::new(vec![Box::new(inner)]), 1);
    assert_eq!(w.ax_tree().len(), 1);
    assert_eq!(w.ax_tree()[0].depth, 0);
}

#[test]
fn undo_and_redo_run_through_the_window() {
    let (mut w, _) = window();
    let mut m = Model::default();
    m.n = 1;
    w.undo.register("Increment", |m: &mut Model| m.n -= 1, |m: &mut Model| m.n += 1);
    assert!(w.is_enabled("undo", &m) && !w.is_enabled("redo", &m));
    assert!(w.commands(&m).supports(Command::Undo));
    assert_eq!(w.send("undo", &mut m), Some(Handler::Window));
    assert_eq!(m.n, 0);
    assert_eq!(w.send("redo", &mut m), Some(Handler::Window));
    assert_eq!(m.n, 1);
    assert_eq!(w.send("undo", &mut m), Some(Handler::Window));
    assert_eq!(w.send("undo", &mut m), None);
}

#[test]
fn controls_change_their_models_and_send_actions() {
    let mut sw = Switch::new("Wi-Fi", false, "wifi");
    sw.set_frame(rect(0.0, 0.0, 120.0, 28.0));
    let mut sl = Slider::new("Volume", values::Slider::new(0.0, 10.0, 0.0, None), "volume");
    sl.set_frame(rect(0.0, 30.0, 118.0, 28.0));
    let mut sg = Segmented::new(&["One", "Two"], Some(0), "seg");
    sg.set_frame(rect(0.0, 60.0, 100.0, 28.0));
    let (si, li, gi) = (sw.id(), sl.id(), sg.id());
    let mut root = Container::new(vec![Box::new(sw), Box::new(sl), Box::new(sg)]);
    root.set_frame(rect(0.0, 0.0, 200.0, 100.0));
    let mut w: Window<Model> = Window::new(root, 1);
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    assert!(w.view::<Switch>(si).unwrap().on());
    // the knob travels from x = 9 to x = 109 on the 118 pt slider
    click(&mut w, &mut m, 109.0, 44.0);
    assert_eq!(w.view::<Slider>(li).unwrap().value(), 10.0);
    click(&mut w, &mut m, 75.0, 74.0);
    assert_eq!(w.view::<Segmented>(gi).unwrap().selected(), Some(1));
    let actions: Vec<_> = w.take_sent().into_iter().map(|(_, a)| a).collect();
    assert_eq!(actions, vec!["wifi", "volume", "seg"]);
}

#[test]
fn stack_lays_out_children_along_its_axis() {
    let ui = ui();
    let mut s = Stack::hstack(8.0, vec![Box::new(Button::new("OK", "ok")), Box::new(Container::default())]);
    s.layout(&ui, rect(0.0, 0.0, 300.0, 40.0));
    let (b, spacer) = (s.children()[0].frame(), s.children()[1].frame());
    assert_eq!(b.x, 0.0);
    assert!(b.w > 0.0 && b.h == ROW_PT.min(40.0));
    assert!((spacer.x - (b.right() + 8.0)).abs() < 1e-3 && (spacer.right() - 300.0).abs() < 1e-3);
}

#[test]
fn draw_reports_the_dirty_views_only() {
    let ui = ui();
    let (mut w, [_, a, ..]) = window();
    let mut px = vec![0u32; 200 * 100];
    let mut s = Surface { w: 200, h: 100, stride: 200, px: &mut px, y0: 0 };
    assert!(w.needs_display());
    assert!(!w.draw(&mut s, &ui).is_empty());
    assert!(!w.needs_display());
    assert!(w.draw(&mut s, &ui).is_empty());
    w.view_mut::<Button>(a).unwrap().set_title("Changed");
    let d = w.draw(&mut s, &ui);
    assert!(!d.is_empty() && d.areas().iter().all(|r| r.x1 <= 70 && r.y1 <= 38));
}

struct Loader {
    laid_out: std::rc::Rc<std::cell::Cell<u32>>,
}
impl ViewController for Loader {
    fn load_view(&mut self) -> Box<dyn View> {
        Box::new(Label::new("Loaded"))
    }
    fn view_did_layout(&mut self, _view: &mut dyn View, _ui: &Ui) {
        self.laid_out.set(self.laid_out.get() + 1);
    }
    fn perform(&mut self, action: &str) -> bool {
        action == "reload"
    }
}

#[test]
fn a_controller_owns_its_subtree_and_joins_the_chain() {
    let ui = ui();
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let host = ControllerHost::new(Loader { laid_out: count.clone() });
    let host_id = host.id();
    let mut w: Window<Model> = Window::new(host, 1);
    w.layout(&ui, rect(0.0, 0.0, 100.0, 50.0));
    assert_eq!(count.get(), 1);
    assert_eq!(w.ax_nodes()[0].label, "Loaded");
    let mut m = Model::default();
    assert_eq!(w.send("reload", &mut m), Some(Handler::View(host_id)));
}

// ---- Forms, new controls, accessibility bridge, menus, dirty redraw ----


fn key(w: &mut Window<Model>, m: &mut Model, k: KeyInput) {
    w.event(&ViewEvent::Key(k), m);
}

fn text(s: &str) -> KeyInput {
    KeyInput::Text(s.into())
}

fn surface(px: &mut Vec<u32>, w: usize, h: usize) -> Surface<'_> {
    px.resize(w * h, 0);
    Surface { w, h, stride: w, px, y0: 0 }
}

#[test]
fn a_checkbox_toggles_on_click_and_key_and_reports_checked_state() {
    let ui = ui();
    let cb = Checkbox::new("Enable", false, "cb");
    let id = cb.id();
    let mut w: Window<Model> = Window::new(cb, 1);
    w.layout(&ui, rect(0.0, 0.0, 120.0, ROW_PT));
    let mut m = Model::default();
    click(&mut w, &mut m, 5.0, 10.0);
    assert!(w.view::<Checkbox>(id).unwrap().checked());
    assert_eq!(w.take_sent(), vec![(id, "cb".to_string())]);
    key(&mut w, &mut m, KeyInput::Code(32));
    assert!(!w.view::<Checkbox>(id).unwrap().checked());
    w.view_mut::<Checkbox>(id).unwrap().set_checked(true);
    let ext = w.ax_ext();
    assert_eq!(ext.len(), 1);
    assert_eq!((ext[0].id, ext[0].checked, ext[0].focused, ext[0].parent), (id, Some(true), true, None));
    assert_eq!(w.ax_nodes()[0].role, Role::Checkbox);
}

#[test]
fn ax_set_presses_writes_values_and_focuses() {
    let ui = ui();
    let (b, c, sl, tf) = (Button::new("Go", "go"), Checkbox::new("C", false, "c"), Slider::new("S", values::Slider::new(0.0, 10.0, 0.0, None), "s"), TextField::new("Name", "name"));
    let ids = [b.id(), c.id(), sl.id(), tf.id()];
    let mut app = ViewApp::new(Stack::vstack(4.0, vec![Box::new(b), Box::new(c), Box::new(sl), Box::new(tf)]), Model::default());
    app.window.layout(&ui, rect(0.0, 0.0, 200.0, 200.0));
    app.ax_set(ids[0], crate::proto::AxAction::Press);
    assert_eq!(app.window.take_sent(), vec![(ids[0], "go".to_string())]);
    app.ax_set(ids[1], crate::proto::AxAction::SetValue("on".into()));
    assert!(app.window.view::<Checkbox>(ids[1]).unwrap().checked());
    app.ax_set(ids[2], crate::proto::AxAction::SetValue("7".into()));
    assert_eq!(app.window.view::<Slider>(ids[2]).unwrap().value(), 7.0);
    app.ax_set(ids[3], crate::proto::AxAction::SetValue("Ada".into()));
    assert_eq!(app.window.view::<TextField>(ids[3]).unwrap().text(), "Ada");
    app.ax_set(ids[3], crate::proto::AxAction::Focus);
    assert_eq!(app.window.first_responder(), Some(ids[3]));
    assert!(app.controls_tree().iter().any(|e| e.id == ids[3] && e.focused));
    assert!(!app.ax_set(9_999_999, crate::proto::AxAction::Press));
}

#[test]
fn main_menu_enabled_state_follows_the_responder_chain_and_actions_route_to_send() {
    let ui = ui();
    let tf = TextField::new("Name", "name");
    let id = tf.id();
    let mut app = ViewApp::new(tf, Model::default());
    app.set_menus(vec![MenuEntry::edit_menu(), ("File".into(), vec![MenuEntry::new("Quit", "quit").key("cmd+q")])]);
    app.window.layout(&ui, rect(0.0, 0.0, 200.0, 28.0));
    let item = |app: &ViewApp<Model>, menu: usize, title: &str| app.main_menu()[menu].items.iter().find(|i| i.title == title).cloned().unwrap();
    // nothing focused: the text actions are unknown to the chain, so disabled; Quit comes from the model
    assert!(!item(&app, 0, "Cut").enabled);
    assert!(!item(&app, 0, "Undo").enabled);
    assert!(item(&app, 1, "Quit").enabled);
    app.ax_set(id, crate::proto::AxAction::SetValue("hello".into()));
    app.ax_set(id, crate::proto::AxAction::Focus);
    assert!(!item(&app, 0, "Cut").enabled, "no selection");
    assert!(item(&app, 0, "Undo").enabled, "the edit can be undone");
    assert!(app.window.send("selectAll", &mut app.state).is_some());
    assert!(item(&app, 0, "Cut").enabled && item(&app, 0, "Copy").enabled && item(&app, 0, "Select All").enabled);
    assert_eq!(item(&app, 0, "Cut").key, "cmd+x");
    // ids are sequential over non-separators; Quit is the 7th item
    let quit = item(&app, 1, "Quit");
    assert_eq!(quit.id, 7);
    app.menu_action(quit.id);
    assert_eq!(app.state.log.last().map(String::as_str), Some("quit"), "the SetValue change action came first");
    assert!(!app.menu_action(99));
}

#[test]
fn a_text_field_edits_selects_and_undoes_through_the_responder_chain() {
    let ui = ui();
    let tf = TextField::new("Name", "name");
    let id = tf.id();
    let mut w: Window<Model> = Window::new(tf, 1);
    w.layout(&ui, rect(0.0, 0.0, 200.0, ROW_PT));
    let mut m = Model::default();
    click(&mut w, &mut m, 10.0, 10.0);
    key(&mut w, &mut m, text("a"));
    key(&mut w, &mut m, text("b"));
    key(&mut w, &mut m, text("c"));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "abc");
    assert_eq!(w.take_sent().len(), 3);
    key(&mut w, &mut m, KeyInput::Code(8));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "ab");
    key(&mut w, &mut m, KeyInput::Code(37));
    key(&mut w, &mut m, text("X"));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "aXb");
    assert_eq!(w.view::<TextField>(id).unwrap().model().caret(), 2);
    assert!(w.is_enabled("selectAll", &m) && !w.is_enabled("copy", &m));
    assert_eq!(w.send("selectAll", &mut m), Some(Handler::View(id)));
    assert!(w.is_enabled("copy", &m) && w.is_enabled("cut", &m));
    assert_eq!(w.view::<TextField>(id).unwrap().model().selected_text(), "aXb");
    // typing replaces the selection; undo restores it, redo repeats
    key(&mut w, &mut m, text("Z"));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "Z");
    assert!(w.is_enabled("undo", &m));
    w.send("undo", &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "aXb");
    assert!(w.is_enabled("redo", &m));
    w.send("redo", &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "Z");
    // the change action queued by a performed command is sent
    w.take_sent();
    w.send("undo", &mut m);
    assert_eq!(w.take_sent(), vec![(id, "name".to_string())]);
    assert_eq!(w.ax_nodes()[0].role, Role::TextField);
}

#[test]
fn a_text_field_with_nothing_to_undo_defers_undo_to_the_window() {
    let ui = ui();
    let tf = TextField::new("", "t");
    let id = tf.id();
    let mut w: Window<Model> = Window::new(tf, 4);
    w.layout(&ui, rect(0.0, 0.0, 100.0, ROW_PT));
    let mut m = Model::default();
    w.make_first_responder(Some(id));
    assert!(!w.is_enabled("undo", &m));
    w.undo.register("n", |m: &mut Model| m.n -= 1, |m: &mut Model| m.n += 1);
    m.n = 1;
    assert!(w.is_enabled("undo", &m));
    assert_eq!(w.send("undo", &mut m), Some(Handler::Window));
    assert_eq!(m.n, 0);
}

#[test]
fn a_stepper_nudges_by_click_and_arrows_and_clamps() {
    let ui = ui();
    let st = Stepper::new("Count", values::Stepper::new(1.0, 0.0, 2.0, 1.0), "step");
    let id = st.id();
    let mut w: Window<Model> = Window::new(st, 1);
    w.layout(&ui, rect(0.0, 0.0, 15.0, 22.0));
    let mut m = Model::default();
    click(&mut w, &mut m, 7.0, 3.0);
    assert_eq!(w.view::<Stepper>(id).unwrap().value(), 2.0);
    click(&mut w, &mut m, 7.0, 3.0);
    assert_eq!(w.take_sent().len(), 1, "no action when clamped");
    click(&mut w, &mut m, 7.0, 19.0);
    key(&mut w, &mut m, KeyInput::Code(40));
    assert_eq!(w.view::<Stepper>(id).unwrap().value(), 0.0);
    w.ax_set_value(id, "1", &mut m);
    assert_eq!(w.view::<Stepper>(id).unwrap().value(), 1.0);
    assert_eq!(w.ax_nodes()[0].value, "1");
}

#[test]
fn progress_indicators_animate_with_tick_time() {
    let ui = ui();
    let sp = ProgressIndicator::spinner();
    let id = sp.id();
    let mut w: Window<Model> = Window::new(Stack::vstack(4.0, vec![Box::new(sp), Box::new(ProgressIndicator::bar(0.5))]), 1);
    w.layout(&ui, rect(0.0, 0.0, 100.0, 60.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 100, 60);
    w.draw(&mut s, &ui);
    assert!(!w.needs_display());
    assert!(w.tick(0.2));
    let head = w.view::<ProgressIndicator>(id).unwrap().head_spoke();
    w.draw(&mut s, &ui);
    assert!(w.tick(0.6));
    assert_ne!(head, w.view::<ProgressIndicator>(id).unwrap().head_spoke());
    w.draw(&mut s, &ui);
    assert!(!w.tick(0.6), "same time: nothing to redraw");
    w.view_mut::<ProgressIndicator>(id).unwrap().stop_animation();
    w.draw(&mut s, &ui);
    assert!(!w.tick(0.9), "stopped spinners do not redraw");
    let mut bar = ProgressIndicator::indeterminate_bar();
    bar.tick(0.5);
    assert!(bar.needs_display());
    assert_eq!(ProgressIndicator::bar(0.42).model.label(), "42%");
}

#[test]
fn a_popup_button_opens_picks_and_repaints_the_area_under_its_menu() {
    let ui = ui();
    let pb = PopUpButton::new(&["Automatic", "Intel", "NVIDIA"], 0, "gpu");
    let id = pb.id();
    let mut w: Window<Model> = Window::new(Stack::vstack(4.0, vec![Box::new(pb), Box::new(Label::new("below"))]), 1);
    w.layout(&ui, rect(0.0, 0.0, 160.0, 120.0));
    let mut m = Model::default();
    let mut px = Vec::new();
    let mut s = surface(&mut px, 160, 120);
    w.draw(&mut s, &ui);
    let f = w.view::<PopUpButton>(id).unwrap().frame();
    click(&mut w, &mut m, f.x + 5.0, f.y + 5.0);
    assert!(w.view::<PopUpButton>(id).unwrap().is_open());
    assert!(w.needs_display());
    assert!(!w.draw(&mut s, &ui).is_empty());
    let r = w.view::<PopUpButton>(id).unwrap().menu_rect();
    assert_eq!(w.hit_test(r.x + 5.0, r.y + 5.0), Some(id), "the open menu takes pointer events");
    // third row: padding + 2 rows + into the row
    click(&mut w, &mut m, r.x + 10.0, r.y + 5.0 + 2.0 * 22.0 + 4.0);
    assert_eq!(w.view::<PopUpButton>(id).unwrap().selected_title(), "NVIDIA");
    assert!(!w.view::<PopUpButton>(id).unwrap().is_open());
    assert_eq!(w.take_sent(), vec![(id, "gpu".to_string())]);
    // closing leaves a stale menu area: the next draw repaints everything under it
    assert!(w.needs_display());
    assert!(w.draw(&mut s, &ui).is_all());
    assert!(!w.needs_display());
    // keyboard: Down opens, Escape closes
    w.make_first_responder(Some(id));
    key(&mut w, &mut m, KeyInput::Code(40));
    assert!(w.view::<PopUpButton>(id).unwrap().is_open());
    key(&mut w, &mut m, KeyInput::Code(27));
    assert!(!w.view::<PopUpButton>(id).unwrap().is_open());
    assert!(w.ax_set_value(id, "Intel", &mut m));
    assert_eq!(w.view::<PopUpButton>(id).unwrap().selected(), 1);
}

#[test]
fn a_section_lays_rows_in_a_rounded_card_with_header_and_footer() {
    let ui = ui();
    let (a, b) = (Switch::new("Wi-Fi", true, "wifi"), Checkbox::new("Ask to join", false, "ask"));
    let (ia, ib) = (a.id(), b.id());
    let section = Section::new(vec![Box::new(a), Box::new(b)]).header("NETWORK").footer("Known networks are joined automatically.");
    let mut w: Window<Model> = Window::new(Form::new(vec![section]), 1);
    w.layout(&ui, rect(0.0, 0.0, 400.0, 300.0));
    let fa = w.view::<Switch>(ia).unwrap().frame();
    let fb = w.view::<Checkbox>(ib).unwrap().frame();
    let margin = bifrost_ui::prefs_layout::m::DETAIL_MARGIN;
    assert!(fa.x > margin && fa.right() < 400.0 - margin);
    assert!(fa.y >= 10.0, "header sits above the first row");
    assert_eq!(fb.y, fa.y + fa.h);
    assert!(fa.h >= ROW_PT);
    let mut m = Model::default();
    click(&mut w, &mut m, fb.x + 2.0, fb.y + fb.h / 2.0);
    assert!(w.view::<Checkbox>(ib).unwrap().checked());
    // the form is a pure container: only the rows are accessibility elements
    assert_eq!(w.ax_nodes().len(), 2);
}

#[test]
fn a_dirty_row_repaints_its_card_instead_of_punching_a_hole_in_it() {
    use crate::kit::ui::{CARD_BG, DETAIL_BG};
    let ui = ui();
    let cb = Checkbox::new("Opt in", false, "opt");
    let id = cb.id();
    let mut w: Window<Model> = Window::new(Form::new(vec![Section::new(vec![Box::new(cb), Box::new(Label::new("second"))])]), 1);
    w.layout(&ui, rect(0.0, 0.0, 300.0, 120.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 300, 120);
    w.draw(&mut s, &ui);
    let f = w.view::<Checkbox>(id).unwrap().frame();
    let probe = ((f.x + f.w - 8.0) as usize, (f.y + 2.0) as usize);
    assert_eq!(s.px[probe.1 * 300 + probe.0], CARD_BG.now(), "inside the card, right of the box");
    w.view_mut::<Checkbox>(id).unwrap().set_checked(true);
    assert!(w.needs_display());
    assert!(!w.draw(&mut s, &ui).is_empty());
    assert_eq!(s.px[probe.1 * 300 + probe.0], CARD_BG.now(), "still card, not the window background");
    assert_ne!(s.px[probe.1 * 300 + probe.0], DETAIL_BG.now());
    assert!(!w.needs_display());
}

#[test]
fn every_new_control_draws_without_panicking() {
    let ui = ui();
    let mut w: Window<Model> = Window::new(
        Stack::vstack(4.0, vec![
            Box::new(Checkbox::new("A", true, "a")),
            Box::new(Stepper::new("N", values::Stepper::new(1.0, 0.0, 5.0, 1.0), "n")),
            Box::new(TextField::new("Placeholder", "t")),
            Box::new(PopUpButton::new(&["One", "Two"], 1, "p")),
            Box::new(ProgressIndicator::indeterminate_bar()),
            Box::new(ProgressIndicator::spinner()),
        ]),
        1,
    );
    w.layout(&ui, rect(0.0, 0.0, 200.0, 220.0));
    let mut px = Vec::new();
    let mut s = surface(&mut px, 200, 220);
    w.draw(&mut s, &ui);
    for id in w.key_view_loop() {
        w.make_first_responder(Some(id));
        w.tick(0.3);
        w.draw(&mut s, &ui);
    }
    assert_eq!(w.key_view_loop().len(), 4);
}

// ---- Text input client: marked text, traits, pointer caret ----

use crate::proto::{InputTraits, ReturnKeyType, TextContentKind, ToApp};

fn focused_field(text_in: &str) -> (Window<Model>, u64) {
    let ui = ui();
    let mut tf = TextField::new("Name", "name");
    tf.set_text(text_in);
    let id = tf.id();
    let mut w: Window<Model> = Window::new(tf, 4);
    w.layout(&ui, rect(0.0, 0.0, 200.0, ROW_PT));
    w.make_first_responder(Some(id));
    (w, id)
}

#[test]
fn a_text_field_reports_state_traits_and_caret_only_while_first_responder() {
    let ui = ui();
    let tf = TextField::new("Name", "name").with_traits(InputTraits { return_key: ReturnKeyType::Search, content: TextContentKind::Email, flags: 0 });
    let id = tf.id();
    let mut w: Window<Model> = Window::new(tf, 4);
    w.layout(&ui, rect(0.0, 0.0, 200.0, ROW_PT));
    assert!(w.text_input_info().is_none());
    w.make_first_responder(Some(id));
    let info = w.text_input_info().unwrap();
    assert_eq!(info.traits.return_key, ReturnKeyType::Search);
    assert_eq!(info.traits.content, TextContentKind::Email);
    assert_eq!(info.state.text, "");
    assert!(info.caret.0 >= 12.0 && info.caret.3 > 0.0);
}

#[test]
fn marked_text_commits_cancels_and_deletes_surrounding_through_the_window() {
    let (mut w, id) = focused_field("ab");
    let mut m = Model::default();
    assert!(w.apply_text_input(&ToApp::SetMarkedText { text: "にほ".into(), sel_start: 6, sel_len: 0 }, &mut m));
    let t = w.view::<TextField>(id).unwrap();
    assert_eq!(t.text(), "abにほ");
    assert!(t.model().has_marked_text());
    let info = w.text_input_info().unwrap();
    assert_eq!(info.state.marked, Some((2, 8)));
    assert_eq!(info.state.cursor, 8);
    assert_eq!(w.take_sent().len(), 1, "the change action fires for composition too");
    assert!(w.apply_text_input(&ToApp::UnmarkText { commit: true }, &mut m));
    let t = w.view::<TextField>(id).unwrap();
    assert!(!t.model().has_marked_text());
    assert_eq!(t.text(), "abにほ");
    // a cancelled composition restores the text
    w.apply_text_input(&ToApp::SetMarkedText { text: "ん".into(), sel_start: 3, sel_len: 0 }, &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "abにほん");
    w.apply_text_input(&ToApp::UnmarkText { commit: false }, &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "abにほ");
    // text-input-v3 delete_surrounding_text, bytes around the caret
    w.apply_text_input(&ToApp::DeleteSurrounding { before: 3, after: 0 }, &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "abに");
    // accessory buttons are the app's business
    assert!(!w.apply_text_input(&ToApp::AccessoryAction(1), &mut m));
}

#[test]
fn emacs_chords_and_pointer_placement_edit_the_field() {
    use crate::proto::CHORD_CTRL;
    let ui = ui();
    let mut tf = TextField::new("Name", "name");
    tf.set_text("hello world");
    let id = tf.id();
    let mut w: Window<Model> = Window::new(tf, 4);
    w.layout(&ui, rect(0.0, 0.0, 300.0, ROW_PT));
    let mut m = Model::default();
    let mut px = Vec::new();
    let mut s = surface(&mut px, 300, 40);
    w.make_first_responder(Some(id));
    w.draw(&mut s, &ui);
    // ctrl-A (evdev 30) goes to the line start, ctrl-K (37) kills to the end, ctrl-Y (21) yanks it back
    key(&mut w, &mut m, KeyInput::chord(30, CHORD_CTRL));
    assert_eq!(w.view::<TextField>(id).unwrap().model().caret(), 0);
    key(&mut w, &mut m, KeyInput::chord(37, CHORD_CTRL));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "");
    key(&mut w, &mut m, KeyInput::chord(21, CHORD_CTRL));
    assert_eq!(w.view::<TextField>(id).unwrap().text(), "hello world");
    // a click at the far left puts the caret at the start; a drag selects to the pointer
    w.draw(&mut s, &ui);
    w.event(&ViewEvent::Pointer(Event::Down(12.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(12.0, 10.0)), &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().model().caret(), 0);
    w.event(&ViewEvent::Pointer(Event::Down(12.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Move(290.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Up(290.0, 10.0)), &mut m);
    assert_eq!(w.view::<TextField>(id).unwrap().model().selected_text(), "hello world");
}

#[test]
fn text_input_info_changes_with_the_text_and_goes_away_on_blur() {
    // `ViewApp::sync_text_input` sends only when this value differs from the last one it sent
    let (mut w, _id) = focused_field("x");
    let a = w.text_input_info();
    assert_eq!(a, w.text_input_info());
    let mut m = Model::default();
    key(&mut w, &mut m, text("y"));
    assert_ne!(a, w.text_input_info());
    w.make_first_responder(None);
    assert!(w.text_input_info().is_none());
}

#[test]
fn clicking_outside_a_popup_resigns_it_and_closes_its_menu() {
    let ui = ui();
    let pb = PopUpButton::new(&["Automatic", "Intel", "NVIDIA"], 0, "gpu");
    let (id, label) = (pb.id(), Label::new("below"));
    let lid = label.id();
    let mut w: Window<Model> = Window::new(Stack::vstack(4.0, vec![Box::new(pb), Box::new(label)]), 1);
    w.layout(&ui, rect(0.0, 0.0, 160.0, 300.0));
    let mut m = Model::default();
    let f = w.view::<PopUpButton>(id).unwrap().frame();
    click(&mut w, &mut m, f.x + 5.0, f.y + 5.0);
    assert!(w.view::<PopUpButton>(id).unwrap().is_open());
    assert_eq!(w.first_responder(), Some(id));
    // far below the menu: empty window background
    click(&mut w, &mut m, 150.0, 290.0);
    assert!(!w.view::<PopUpButton>(id).unwrap().is_open());
    assert_eq!(w.first_responder(), None);
    assert!(w.needs_display());
    // a view that takes no focus (the label) resigns too
    w.make_first_responder(Some(id));
    let lf = w.view::<Label>(lid).unwrap().frame();
    click(&mut w, &mut m, lf.x + 1.0, lf.y + 1.0);
    assert_eq!(w.first_responder(), None);
}

/// Accepts focus and refuses to give it up.
struct Stubborn {
    base: ViewBase,
    resigned: u32,
}
impl View for Stubborn {
    crate::view_base!();
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn resign_first_responder(&mut self) -> bool {
        false
    }
    fn did_resign_first_responder(&mut self) {
        self.resigned += 1;
    }
}

#[test]
fn a_view_that_refuses_to_resign_keeps_the_focus() {
    let stubborn = Stubborn { base: ViewBase::default(), resigned: 0 };
    let sid = stubborn.id();
    let (mut a, mut b) = (Button::new("A", "a"), Button::new("B", "b"));
    a.set_frame(rect(10.0, 10.0, 60.0, 28.0));
    b.set_frame(rect(80.0, 10.0, 60.0, 28.0));
    let bid = b.id();
    let mut w: Window<Model> = Window::new(Probe { base: ViewBase::default(), kids: vec![Box::new(stubborn), Box::new(a), Box::new(b)], performed: vec![] }, 1);
    w.root_mut().set_frame(rect(0.0, 0.0, 200.0, 100.0));
    assert!(w.make_first_responder(Some(sid)));
    assert!(!w.make_first_responder(Some(bid)));
    assert_eq!(w.first_responder(), Some(sid));
    assert!(!w.make_first_responder(None));
    let mut m = Model::default();
    click(&mut w, &mut m, 90.0, 20.0);
    assert_eq!(w.first_responder(), Some(sid), "clicking another view or the background does not steal it");
    assert_eq!(w.view::<Stubborn>(sid).unwrap().resigned, 0);
    // re-selecting itself is a no-op, not a resign
    assert!(w.make_first_responder(Some(sid)));
}

/// A document 400 pt tall: a button at content y 300 and another at 120, placed relative to its origin.
struct Doc {
    base: ViewBase,
    kids: Vec<Box<dyn View>>,
}
impl View for Doc {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.kids
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.kids
    }
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, 400.0)
    }
    fn layout(&mut self, ui: &Ui, b: Rect) {
        self.set_frame(b);
        for (c, y) in self.kids.iter_mut().zip([300.0, 120.0]) {
            c.layout(ui, rect(b.x + 10.0, b.y + y, 60.0, 28.0));
        }
    }
}

fn scroll_window() -> (Window<Model>, u64, u64, u64) {
    let (far, near) = (Button::new("Far", "far"), Button::new("Near", "near"));
    let (fid, nid) = (far.id(), near.id());
    let sv = ScrollView::new(Box::new(Doc { base: ViewBase::default(), kids: vec![Box::new(far), Box::new(near)] }));
    let sid = sv.id();
    let mut w: Window<Model> = Window::new(sv, 1);
    w.layout(&ui(), rect(0.0, 0.0, 200.0, 100.0));
    (w, sid, fid, nid)
}

#[test]
fn a_scroll_view_clamps_scroll_to_and_reports_the_offset() {
    let (mut w, sid, ..) = scroll_window();
    let sv = w.view_mut::<ScrollView>(sid).unwrap();
    assert_eq!(sv.max_offset(), 300.0);
    sv.scroll_to(250.0);
    assert_eq!(sv.content_offset(), 250.0);
    sv.scroll_to(9999.0);
    assert_eq!(sv.content_offset(), 300.0);
    sv.scroll_to(-5.0);
    assert_eq!(sv.content_offset(), 0.0);
    assert!(sv.document_view::<Doc>().is_some());
    assert!(sv.document_view::<Label>().is_none());
}

#[test]
fn hit_tests_and_clicks_inside_scrolled_content_reach_the_right_child() {
    let (mut w, sid, fid, nid) = scroll_window();
    let mut m = Model::default();
    assert_ne!(w.hit_test(20.0, 5.0), Some(fid));
    w.view_mut::<ScrollView>(sid).unwrap().scroll_to(250.0);
    // far button: content y 300..328 shows at window y 50..78; the near one (120..148) has scrolled out above
    assert_eq!(w.hit_test(20.0, 60.0), Some(fid));
    assert_ne!(w.hit_test(20.0, 10.0), Some(nid));
    // below the scroll view's frame nothing is hit, even though content continues
    assert_eq!(w.hit_test(20.0, 150.0), None);
    click(&mut w, &mut m, 20.0, 60.0);
    assert_eq!(w.take_sent(), vec![(fid, "far".to_string())]);
    // the stored frames are untouched (content coordinates)
    assert_eq!(w.view::<Button>(fid).unwrap().frame().y, 300.0);
}

#[test]
fn dragging_a_scroll_view_pans_it() {
    let (mut w, sid, ..) = scroll_window();
    let mut m = Model::default();
    w.event(&ViewEvent::Pointer(Event::Down(150.0, 10.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Move(150.0, -40.0)), &mut m);
    w.event(&ViewEvent::Pointer(Event::Move(150.0, -90.0)), &mut m);
    let off = w.view::<ScrollView>(sid).unwrap().content_offset();
    assert!(off > 50.0 && off <= 100.0, "{off}");
    assert!(w.needs_display());
    w.event(&ViewEvent::Pointer(Event::Up(150.0, -90.0)), &mut m);
}

#[test]
fn a_scroll_view_clips_rows_and_draws_content_at_the_offset() {
    let ui = ui();
    let (mut w, sid, fid, _) = scroll_window();
    let mut px = Vec::new();
    let mut s = surface(&mut px, 200, 200);
    w.draw(&mut s, &ui);
    let bg = s.px[150 * 200 + 150];
    // offset 0: the "near" button (content y 120) is below the 100 pt frame and must be clipped away
    assert!(s.px[100 * 200..].iter().all(|&p| p == bg), "nothing is painted below the scroll view");
    w.view_mut::<ScrollView>(sid).unwrap().scroll_to(80.0);
    assert!(w.needs_display());
    w.draw(&mut s, &ui);
    // near button now at window y 40..68
    let painted = (40..68).any(|y| (10..70).any(|x| s.px[y * 200 + x] != bg));
    assert!(painted, "the button is drawn at frame - offset");
    assert!(s.px[100 * 200..].iter().all(|&p| p == bg));
    assert_eq!(w.view::<Button>(fid).unwrap().frame().y, 300.0, "frames restored after drawing");
}
