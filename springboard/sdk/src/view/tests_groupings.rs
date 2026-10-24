//! Contract tests for `button_style.rs` (ButtonStyle, roles, the built-in styles) and `groupings.rs` (GroupBox,
//! ControlGroup, TabView).
use super::*;
use crate::kit::{Font, Surface};
use crate::layout::ROW_PT;
use bifrost_ui::Role;
use std::cell::Cell;
use std::rc::Rc;

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
        true
    }
}

fn one<V: View + 'static>(v: V, size: (f32, f32)) -> (Window<Model>, u64) {
    let id = v.id();
    let mut w: Window<Model> = Window::new(v, 1);
    w.layout(&ui(), rect(0.0, 0.0, size.0, size.1));
    (w, id)
}

fn draws(w: &mut Window<Model>, wd: usize, ht: usize) {
    let mut px = vec![0u32; wd * ht];
    let mut s = Surface { w: wd, h: ht, stride: wd, px: &mut px, y0: 0 };
    w.draw(&mut s, &ui());
}

fn click(w: &mut Window<Model>, m: &mut Model, x: f32, y: f32) {
    w.event(&ViewEvent::Pointer(Event::Down(x, y)), m);
    w.event(&ViewEvent::Pointer(Event::Up(x, y)), m);
}

/// A style that records what it was asked to draw.
struct Probe {
    seen: Rc<Cell<Option<(bool, Option<ButtonRole>)>>>,
}
impl ButtonStyle for Probe {
    fn draw(&self, _s: &mut dyn bifrost_render::canvas::Canvas, _ui: &Ui, cfg: &ButtonConfiguration) {
        self.seen.set(Some((cfg.is_pressed, cfg.role)));
    }
    fn size(&self, _ui: &Ui, _cfg: &ButtonConfiguration) -> (f32, f32) {
        (77.0, 33.0)
    }
}

// ---- ButtonStyle ----

#[test]
fn a_custom_style_sizes_and_draws_the_button_from_its_configuration() {
    let seen = Rc::new(Cell::new(None));
    let b = Button::new("Delete", "delete").styled(Probe { seen: seen.clone() }).with_role(ButtonRole::Destructive);
    assert_eq!(b.intrinsic(&ui()), (77.0, 33.0));
    assert_eq!(b.role(), Some(ButtonRole::Destructive));
    let cfg = b.configuration();
    assert_eq!((cfg.label, cfg.is_pressed, cfg.enabled), ("Delete", false, true));
    let (mut w, id) = one(b, (100.0, 40.0));
    draws(&mut w, 100, 40);
    assert_eq!(seen.get(), Some((false, Some(ButtonRole::Destructive))));
    // pressing shows the style the pressed state; the action still goes up the chain
    let mut m = Model::default();
    w.event(&ViewEvent::Pointer(Event::Down(10.0, 10.0)), &mut m);
    draws(&mut w, 100, 40);
    assert_eq!(seen.get(), Some((true, Some(ButtonRole::Destructive))));
    w.event(&ViewEvent::Pointer(Event::Up(10.0, 10.0)), &mut m);
    assert_eq!(m.log, vec!["delete"]);
    assert_eq!(w.ax_nodes()[0].role, Role::Button);
    let _ = id;
}

#[test]
fn the_built_in_styles_ask_for_less_room_as_their_bezel_shrinks() {
    let ui = ui();
    let w = |b: Button| b.intrinsic(&ui).0;
    let bordered = w(Button::new("Save", "s"));
    assert_eq!(bordered, w(Button::new("Save", "s").styled(BorderedProminent)));
    let borderless = w(Button::new("Save", "s").styled(Borderless));
    let plain = w(Button::new("Save", "s").styled(Plain));
    let link = w(Button::new("Save", "s").styled(LinkStyle));
    assert!(bordered > borderless && borderless > plain && plain > link, "{bordered} {borderless} {plain} {link}");
    assert_eq!(Button::new("Save", "s").styled(Plain).intrinsic(&ui).1, ROW_PT);
    // every style paints, pressed or not, enabled or not
    for style in [Box::new(Bordered) as Box<dyn ButtonStyle>, Box::new(BorderedProminent), Box::new(Borderless), Box::new(Plain), Box::new(LinkStyle)] {
        let mut b = Button::new("Go", "go").with_role(ButtonRole::Destructive);
        b.set_style(style);
        let (mut win, id) = one(b, (80.0, 30.0));
        draws(&mut win, 80, 30);
        win.view_mut::<Button>(id).unwrap().base_mut().enabled = false;
        draws(&mut win, 80, 30);
    }
}

// ---- GroupBox ----

#[test]
fn a_group_box_insets_its_content_below_the_title() {
    let ui = ui();
    let label = Label::new("Inside");
    let lid = label.id();
    let (mut w, id) = one(GroupBox::new(Box::new(label)).titled("Agreement"), (300.0, 120.0));
    let gb = w.view::<GroupBox>(id).unwrap();
    assert_eq!(gb.title(), Some("Agreement"));
    let card = gb.card();
    assert!(card.y > 0.0 && (card.bottom() - 120.0).abs() < 1e-3);
    let inner = w.view::<Label>(lid).unwrap().frame();
    assert!(inner.x > card.x && inner.y > card.y && inner.right() < card.right() && inner.bottom() < card.bottom(), "{inner:?} in {card:?}");
    assert!(w.view::<GroupBox>(id).unwrap().content::<Label>().is_some());
    // an unlabeled box starts its card at the top; its intrinsic size wraps the content
    let plain = GroupBox::new(Box::new(Label::new("x")));
    let (cw, ch) = Label::new("x").intrinsic(&ui);
    let (gw, gh) = plain.intrinsic(&ui);
    assert!(gw > cw && gh > ch);
    assert!(w.view::<GroupBox>(id).unwrap().opaque_backdrop());
    assert_eq!(w.ax_nodes()[0].label, "Agreement");
    draws(&mut w, 300, 120);
}

// ---- ControlGroup ----

#[test]
fn a_control_group_joins_borderless_buttons_in_equal_shares() {
    let (a, b) = (Button::new("Back", "back"), Button::new("Forward", "forward"));
    let (ia, ib) = (a.id(), b.id());
    let (mut w, id) = one(ControlGroup::new("Navigation", vec![Box::new(a), Box::new(b)]), (200.0, ROW_PT));
    let (fa, fb) = (w.view::<Button>(ia).unwrap().frame(), w.view::<Button>(ib).unwrap().frame());
    assert!((fa.w - 100.0).abs() < 1e-3 && (fb.x - 100.0).abs() < 1e-3, "{fa:?} {fb:?}");
    // the buttons took the borderless style: narrower than the bordered default
    let ui = ui();
    assert!(w.view::<Button>(ia).unwrap().intrinsic(&ui).0 < Button::new("Back", "back").intrinsic(&ui).0);
    assert_eq!(w.view::<ControlGroup>(id).unwrap().label(), "Navigation");
    let mut m = Model::default();
    click(&mut w, &mut m, 150.0, 10.0);
    assert_eq!(m.log, vec!["forward"]);
    draws(&mut w, 200, ROW_PT as usize);
}

#[test]
fn a_palette_group_packs_its_controls_and_a_narrow_group_shrinks_them_together() {
    let ui = ui();
    let mk = |style| ControlGroup::new("Tools", vec![Box::new(Button::new("A", "a")), Box::new(Button::new("B", "b"))]).styled(style);
    let mut palette = mk(ControlGroupStyle::Palette);
    palette.layout(&ui, rect(0.0, 0.0, 400.0, ROW_PT));
    let (a, b) = (palette.children()[0].frame(), palette.children()[1].frame());
    assert!(a.w < 200.0 && b.x > a.right(), "packed with a gap: {a:?} {b:?}");
    assert!(!palette.opaque_backdrop());
    let mut narrow = mk(ControlGroupStyle::Automatic);
    let need = narrow.intrinsic(&ui).0;
    narrow.layout(&ui, rect(0.0, 0.0, need / 2.0, ROW_PT));
    let total: f32 = narrow.children().iter().map(|c| c.frame().w).sum();
    assert!((total - need / 2.0).abs() < 1e-3);
}

// ---- TabView ----

fn tabs() -> TabView {
    TabView::new(
        "tabChanged",
        vec![("general", "General", Box::new(Label::new("one")) as Box<dyn View>), ("advanced", "Advanced", Box::new(Label::new("two"))), ("general", "Again", Box::new(Label::new("dup")))],
    )
}

#[test]
fn a_tab_view_shows_only_the_selected_tab_and_drops_repeated_ids() {
    let t = tabs();
    assert_eq!(t.selected_id(), Some("general"));
    assert_eq!(t.children().len(), 2, "the repeated id is dropped");
    assert!(!t.children()[0].base().hidden && t.children()[1].base().hidden);
    let mut t = t;
    assert!(t.select("advanced"));
    assert!(t.children()[0].base().hidden && !t.children()[1].base().hidden);
    assert!(!t.select("missing"));
    assert_eq!(t.selected_id(), Some("advanced"));
    assert_eq!(t.page::<Label>("advanced").map(|l| l.text()), Some("two"));
}

#[test]
fn clicking_a_tab_or_arrow_keys_select_it_and_send_the_action() {
    let ui = ui();
    let (mut w, id) = one(tabs(), (400.0, 200.0));
    let bar = w.view::<TabView>(id).unwrap().bar(&ui);
    let mut m = Model::default();
    // the second tab's middle
    click(&mut w, &mut m, bar.x + bar.w * 0.75, bar.y + bar.h / 2.0);
    assert_eq!(w.view::<TabView>(id).unwrap().selected_id(), Some("advanced"));
    assert_eq!(m.log, vec!["tabChanged"]);
    // clicking the selected tab again sends nothing
    click(&mut w, &mut m, bar.x + bar.w * 0.75, bar.y + bar.h / 2.0);
    assert_eq!(m.log.len(), 1);
    // Left moves back while the tab view has focus
    w.make_first_responder(Some(id));
    w.event(&ViewEvent::Key(KeyInput::Code(37)), &mut m);
    assert_eq!(w.view::<TabView>(id).unwrap().selected_id(), Some("general"));
    assert_eq!(m.log.len(), 2);
    // content sits inside the card, below the tab row
    let page = w.view::<TabView>(id).unwrap().children()[0].frame();
    assert!(page.y > bar.bottom());
    let n = &w.ax_nodes()[0];
    assert_eq!((n.role, n.value.as_str()), (Role::SegmentedControl, "General"));
    draws(&mut w, 400, 200);
}
