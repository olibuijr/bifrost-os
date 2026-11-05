//! View controllers and the `App` adapter.
use super::{TextInputInfo, View, ViewBase, ViewEvent, Window};
use crate::kit::{Damage, Ui};
use bifrost_render::canvas::Canvas;
use crate::proto::{AxAction, NodeExt};
use crate::{AppMenu, AppMenuItem, Command, Commands, Ctx, Input};
use bifrost_ui::responder::Responder;
use bifrost_ui::{Node, Rect};

/// The action string of a standard edit command (the Cocoa selector without the colon): what `Input::Command` and the
/// Edit menu send down the responder chain.
pub fn action_for(c: Command) -> &'static str {
    match c {
        Command::Cut => "cut",
        Command::Copy => "copy",
        Command::Paste => "paste",
        Command::SelectAll => "selectAll",
        Command::Undo => "undo",
        Command::Redo => "redo",
    }
}

/// Owns a subtree (`NativeViewController`). Put it in the tree with [`ControllerHost`]; it sits in the responder chain
/// between its view and the view's parent.
pub trait ViewController: 'static {
    /// `loadView`: builds the root of the controller's subtree (called once).
    fn load_view(&mut self) -> Box<dyn View>;
    /// `viewDidLoad`.
    fn view_did_load(&mut self, _view: &mut dyn View) {}
    /// `viewDidLayout`: the subtree has been laid out; adjust frames or content here.
    fn view_did_layout(&mut self, _view: &mut dyn View, _ui: &Ui) {}
    fn perform(&mut self, _action: &str) -> bool {
        false
    }
    fn validate(&self, _action: &str) -> Option<bool> {
        None
    }
}

/// A view that hosts a [`ViewController`] and its view.
pub struct ControllerHost<C: ViewController> {
    base: ViewBase,
    controller: C,
    content: Vec<Box<dyn View>>,
}

impl<C: ViewController> ControllerHost<C> {
    pub fn new(mut controller: C) -> Self {
        let mut view = controller.load_view();
        controller.view_did_load(view.as_mut());
        ControllerHost { base: ViewBase::default(), controller, content: vec![view] }
    }
    pub fn controller(&self) -> &C {
        &self.controller
    }
    pub fn controller_mut(&mut self) -> &mut C {
        &mut self.controller
    }
}

impl<C: ViewController> View for ControllerHost<C> {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.content
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.content
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        self.content[0].layout(ui, bounds);
        self.controller.view_did_layout(self.content[0].as_mut(), ui);
    }
    fn perform(&mut self, action: &str) -> bool {
        self.controller.perform(action)
    }
    fn validate(&self, action: &str) -> Option<bool> {
        self.controller.validate(action)
    }
}

/// Backs an `App` with a view window. The model `S` is the last link of the responder chain and the state Undo works on.
///
/// ```ignore
/// struct Notes { views: ViewApp<Model> }
/// impl App for Notes {
///     fn id(&self) -> &str { "is.example.notes" }
///     fn configure(&mut self, ctx: &Ctx) { self.views.configure(ctx) }
///     fn event(&mut self, ctx: &Ctx, ev: Input) -> bool { self.views.event(ctx, ev) }
///     fn controls(&self, _: &Ctx) -> Vec<controls::Node> { self.views.controls() }
///     fn commands(&self, _: &Ctx) -> Commands { self.views.commands() }
///     fn controls_tree(&self, _: &Ctx) -> Vec<proto::NodeExt> { self.views.controls_tree() }
///     fn ax_set(&mut self, _: &Ctx, id: u64, a: proto::AxAction) -> bool { self.views.ax_set(id, a) }
///     fn main_menu(&self, _: &Ctx) -> Vec<AppMenu> { self.views.main_menu() }
///     fn menu_action(&mut self, _: &Ctx, id: u32) -> bool { self.views.menu_action(id) }
///     fn draws_canvas(&self) -> bool { true }
///     fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, t: f64) -> Damage { self.views.draw_at(ctx, canvas, t) }
/// }
/// ```
pub struct ViewApp<S: Responder + 'static> {
    pub window: Window<S>,
    pub state: S,
    menus: Vec<(String, Vec<MenuEntry>)>,
    /// what the shell was last told about the text field in focus (`None`: nothing, or not told yet)
    published: Option<TextInputInfo>,
    /// the dragged types registered with the shell (`registerForDraggedTypes`); empty until a view takes drops
    drop_types_sent: Vec<String>,
}

/// One main-menu item (`NativeMenuItem` with a nil-targeted action). Its enabled state is not stored: `ViewApp::main_menu`
/// asks the responder chain (`Window::is_enabled`, `validateMenuItem:`) every time.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub title: String,
    /// the action sent from the first responder on choosing the item ("cut", "selectAll", or an app action)
    pub action: String,
    /// key equivalent in `KeyCombo::parse` notation ("cmd+x"); empty for none
    pub key: String,
    pub checked: bool,
    pub separator: bool,
}

impl MenuEntry {
    pub fn new(title: &str, action: &str) -> Self {
        MenuEntry { title: title.into(), action: action.into(), key: String::new(), checked: false, separator: false }
    }
    pub fn separator() -> Self {
        MenuEntry { separator: true, ..MenuEntry::new("", "") }
    }
    pub fn key(mut self, combo: &str) -> Self {
        self.key = combo.into();
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    /// The standard Edit menu: Undo, Redo, Cut, Copy, Paste, Select All.
    pub fn edit_menu() -> (String, Vec<MenuEntry>) {
        (
            "Edit".into(),
            vec![
                MenuEntry::new("Undo", "undo").key("cmd+z"),
                MenuEntry::new("Redo", "redo").key("cmd+shift+z"),
                MenuEntry::separator(),
                MenuEntry::new("Cut", "cut").key("cmd+x"),
                MenuEntry::new("Copy", "copy").key("cmd+c"),
                MenuEntry::new("Paste", "paste").key("cmd+v"),
                MenuEntry::new("Select All", "selectAll").key("cmd+a"),
            ],
        )
    }
}

impl<S: Responder + 'static> ViewApp<S> {
    pub fn new(root: impl View, state: S) -> Self {
        ViewApp { window: Window::new(root, 100), state, menus: Vec::new(), published: None, drop_types_sent: Vec::new() }
    }
    /// Sets the app's menus (`NativeApp.mainMenu`); item ids are assigned in order from 1.
    pub fn set_menus(&mut self, menus: Vec<(String, Vec<MenuEntry>)>) {
        self.menus = menus;
    }
    /// `App::configure`: lays the tree out in the window size (`Configure::w`/`h` are pixels).
    pub fn configure(&mut self, ctx: &Ctx) {
        let s = ctx.ui.s;
        self.window.layout(&ctx.ui, Rect { x: 0.0, y: 0.0, w: ctx.cfg.w as f32 / s, h: ctx.cfg.h as f32 / s });
        self.sync_drop_types(ctx);
    }
    /// `App::event`: touches and keys go to the window, `Input::Command` down the responder chain. True when a frame is needed.
    pub fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let Input::Command(c) = &ev {
            self.window.send(action_for(*c), &mut self.state);
            self.sync_text_input(ctx);
            return self.window.needs_display();
        }
        let scale = ctx.ui.s;
        let need = ViewEvent::from_input(&ev, scale).is_some_and(|e| self.window.event(&e, &mut self.state));
        self.sync_text_input(ctx);
        // a press on a drag source that moved past the slop begins a session with the shell
        if let Some((view, spec)) = self.window.take_drag_request() {
            let session = crate::dnd::begin(ctx, spec.allowed, spec.items, spec.preview);
            self.window.drag_began(view, session);
        }
        self.sync_drop_types(ctx);
        need
    }
    /// `App::pointer` for the main window (surface pixels): the wheel scrolls the scroll view under the pointer
    /// (`Window::wheel`). Other pointer events are not used by the view kit yet. True when a frame is needed.
    pub fn pointer(&mut self, ctx: &Ctx, ev: &crate::pointer_events::PointerEvent) -> bool {
        self.pointer_points(&ev.to_points(ctx.ui.s))
    }
    /// The same for an event already in window points: an extra window's, from `App::window_pointer`.
    pub fn pointer_points(&mut self, ev: &crate::pointer_events::PointerEvent) -> bool {
        match *ev {
            crate::pointer_events::PointerEvent::Wheel { x, y, dx, dy, .. } => self.window.wheel(x, y, dx, dy),
            _ => false,
        }
    }
    /// Registers the types the views take as drops with the shell (`registerForDraggedTypes`) when they changed since the
    /// last call. `event` and `drag` call it; call it after building or changing the tree outside them.
    pub fn sync_drop_types(&mut self, ctx: &Ctx) {
        let types = self.window.drop_types();
        if types != self.drop_types_sent {
            let refs: Vec<&str> = types.iter().map(|t| t.as_str()).collect();
            crate::dnd::register_types(ctx, &refs);
            self.drop_types_sent = types;
        }
    }
    /// `App::drag`: a session over this window finds the view under the pointer that takes its items, and answers the
    /// shell with the operation (`Entered`, `Updated`) or the result of the drop (`Perform`); an `Ended` event reaches the
    /// source view that began the session. True when a frame is needed.
    pub fn drag(&mut self, ctx: &Ctx, ev: crate::dnd::Event) -> bool {
        match ev {
            crate::dnd::Event::Dest { session, phase, x, y, allowed, items } => {
                let scale = ctx.ui.s;
                let (op, accepted) = self.window.drag_dest(phase, x / scale, y / scale, allowed, &items);
                match phase {
                    crate::proto::DragPhase::Entered | crate::proto::DragPhase::Updated => crate::dnd::reply(ctx, session, op),
                    crate::proto::DragPhase::Perform => crate::dnd::performed(ctx, session, accepted),
                    crate::proto::DragPhase::Exited => {}
                }
            }
            crate::dnd::Event::Ended { session, op } => {
                self.window.drag_ended(session, op);
            }
        }
        self.window.needs_display()
    }
    /// `App::text_input`: IME and keyboard composition go to the first responder (a [`super::TextField`] maps them onto
    /// its `TextEditor`). `AccessoryAction` is not handled here: it belongs to the app. True when a frame is needed.
    pub fn text_input(&mut self, ctx: &Ctx, ev: crate::proto::ToApp) -> bool {
        let handled = self.window.apply_text_input(&ev, &mut self.state);
        self.sync_text_input(ctx);
        handled && self.window.needs_display()
    }
    /// Tells the shell about the focused text field (`ctx.text_state`, `caret_rect`, `input_traits`) when something
    /// changed since the last report; reports an empty state once when the focus leaves text.
    pub fn sync_text_input(&mut self, ctx: &Ctx) {
        let now = self.window.text_input_info();
        if now == self.published {
            return;
        }
        match &now {
            Some(i) => {
                if self.published.as_ref().map(|p| &p.traits) != Some(&i.traits) {
                    ctx.input_traits(i.traits);
                }
                if self.published.as_ref().map(|p| &p.state) != Some(&i.state) {
                    ctx.text_state(i.state.clone());
                }
                if self.published.as_ref().map(|p| p.caret) != Some(i.caret) {
                    ctx.caret_rect(i.caret.0, i.caret.1, i.caret.2, i.caret.3);
                }
            }
            None => ctx.text_state(crate::proto::TextState::default()),
        }
        self.published = now;
    }
    pub fn controls(&self) -> Vec<Node> {
        self.window.ax_nodes()
    }
    pub fn commands(&self) -> Commands {
        self.window.commands(&self.state)
    }
    /// For `App::undo_titles`.
    pub fn undo_titles(&self) -> (String, String) {
        self.window.undo_titles()
    }
    /// Paints the window into `canvas` (a CPU `Surface` or a display list); for `App::draw_canvas`.
    pub fn draw(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas) -> Damage {
        let d = self.window.draw(canvas, &ctx.ui);
        self.sync_text_input(ctx);
        d
    }
    /// `App::draw_canvas` with animation: ticks spinners and indeterminate bars to `t` (the `draw_canvas` argument), then
    /// paints into `canvas`. Apps with views forward `draw_canvas` here and return `true` from `draws_canvas`.
    pub fn draw_at(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, t: f64) -> Damage {
        self.window.tick(t);
        self.flush_animation_actions();
        let d = self.window.draw(canvas, &ctx.ui);
        self.sync_text_input(ctx);
        d
    }
    /// `App::next_wake` for `t` in milliseconds: the next animation frame, `None` when nothing animates.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        self.window.next_wake(t / 1000.0).map(|w| w * 1000.0)
    }
    /// Sends the actions of finished animation transactions down the responder chain; true when a frame is needed.
    pub fn flush_animation_actions(&mut self) -> bool {
        for a in self.window.take_animation_actions() {
            self.window.send(&a, &mut self.state);
        }
        self.window.needs_display()
    }
    /// `App::controls_tree`: parent, focus and checked state of the elements `controls` returns.
    pub fn controls_tree(&self) -> Vec<NodeExt> {
        self.window.ax_ext()
    }
    /// `App::ax_text`: caret, selection and visible range of the text elements.
    pub fn ax_text(&self) -> Vec<bifrost_ui::ax::AxText> {
        self.window.ax_texts()
    }
    /// `App::ax_meta`: `accessibilityTitle` and `accessibilityHelp` of the elements.
    pub fn ax_meta(&self) -> Vec<bifrost_ui::ax::AxMeta> {
        self.window.ax_metas()
    }
    /// `App::ax_set`: Press activates the element, SetValue writes its value, Focus makes it first responder.
    /// True when a frame is needed.
    pub fn ax_set(&mut self, id: u64, action: AxAction) -> bool {
        let done = match action {
            AxAction::Press => self.window.ax_press(id, &mut self.state),
            AxAction::SetValue(v) => self.window.ax_set_value(id, &v, &mut self.state),
            AxAction::Focus => self.window.make_first_responder(Some(id)),
            AxAction::Increment => self.window.ax_adjust(id, true, &mut self.state),
            AxAction::Decrement => self.window.ax_adjust(id, false, &mut self.state),
            // the runtime routes named actions to `App::ax_custom_action`; a view tree has none of its own yet
            AxAction::Custom(_) => false,
        };
        done && self.window.needs_display()
    }
    /// `App::ax_set` variant that also reports the text field's new state; call it where the app has a `Ctx`.
    pub fn ax_set_synced(&mut self, ctx: &Ctx, id: u64, action: AxAction) -> bool {
        let r = self.ax_set(id, action);
        self.sync_text_input(ctx);
        r
    }
    /// `App::main_menu`: the menus set with [`ViewApp::set_menus`], each item enabled by responder-chain validation.
    pub fn main_menu(&self) -> Vec<AppMenu> {
        let mut id = 0u32;
        self.menus
            .iter()
            .map(|(title, entries)| AppMenu {
                title: title.clone(),
                items: entries
                    .iter()
                    .map(|e| {
                        if e.separator {
                            return AppMenuItem::separator();
                        }
                        id += 1;
                        let mut it = AppMenuItem::new(id, &e.title).checked(e.checked).enabled(self.window.is_enabled(&e.action, &self.state));
                        it.key = e.key.clone();
                        it
                    })
                    .collect(),
            })
            .collect()
    }
    /// `App::menu_action`: sends the chosen item's action from the first responder. True when a frame is needed.
    pub fn menu_action(&mut self, id: u32) -> bool {
        let action = self.menus.iter().flat_map(|(_, es)| es.iter()).filter(|e| !e.separator).nth(id.wrapping_sub(1) as usize).map(|e| e.action.clone());
        match action {
            Some(a) => {
                self.window.send(&a, &mut self.state);
                self.window.needs_display()
            }
            None => false,
        }
    }
}
