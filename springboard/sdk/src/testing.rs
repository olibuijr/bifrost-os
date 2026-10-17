//! Testing kit (XCTest / XCUITest parity): a headless window you can click, type into and query by accessibility
//! role and label, with no shell, socket or GPU. See `docs/bifrost-ui.md` "Testing kit (XCTest parity)".
//!
//! ```ignore
//! let mut w = TestWindow::new(root, Model::default(), 320, 200);
//! w.element(Role::Button, "Save").unwrap().press();
//! w.element(Role::TextField, "Name").unwrap().set_value("Ada");
//! assert_eq!(w.model.saved, 1);
//! ```
use crate::kit::{Font, Surface, Ui};
use crate::proto::KeyInput;
use crate::view::{NoApp, View, ViewEvent, Window};
use bifrost_ui::responder::Responder;
use bifrost_ui::{Event, Node, Rect, Role};

// ---- pinned clock and seed (snapshot determinism) ----

pub use bifrost_core::pin::{pinned, ENV_PIN, PINNED_SEED, PINNED_UNIX_SECS};

/// Pins the clock and seed for this process and the app binaries it spawns. Call it once, before spawning.
pub fn pin() {
    std::env::set_var(ENV_PIN, "1");
}

/// Wall-clock Unix seconds: [`PINNED_UNIX_SECS`] when pinned, else the system clock.
pub fn unix_secs() -> i64 {
    if pinned() {
        return PINNED_UNIX_SECS;
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The current wall-clock time ([`PINNED_UNIX_SECS`] when pinned). Apps use this instead of `SystemTime::now()`.
pub fn system_time() -> std::time::SystemTime {
    if pinned() {
        return std::time::UNIX_EPOCH + std::time::Duration::from_secs(PINNED_UNIX_SECS as u64);
    }
    std::time::SystemTime::now()
}

/// A non-zero seed for an app's random generator: [`PINNED_SEED`] when pinned, else taken from the clock.
pub fn seed() -> u64 {
    if pinned() {
        return PINNED_SEED;
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    nanos | 1
}

/// Pointer moves [`TestWindow::swipe`] sends between its endpoints.
pub const SWIPE_STEPS: u32 = 12;

/// `XCUIApplication` for one window: the SDK [`Window`], your model and a loaded [`Ui`] (SFUI font, scale 1).
pub struct TestWindow<S: Responder = NoApp> {
    pub window: Window<S>,
    pub model: S,
    pub ui: Ui,
    w: u32,
    h: u32,
}

impl<S: Responder> TestWindow<S> {
    /// Builds the window around `root` and lays it out in `w` x `h` points.
    pub fn new(root: impl View, model: S, w: u32, h: u32) -> Self {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf");
        let data: &'static [u8] = Box::leak(std::fs::read(path).expect("SFUI.ttf").into_boxed_slice());
        let ui = Ui::new(Font::parse(data).expect("font"), 1.0);
        let mut t = TestWindow { window: Window::new(root, 10), model, ui, w, h };
        t.relayout();
        t
    }
    fn relayout(&mut self) {
        let b = Rect { x: 0.0, y: 0.0, w: self.w as f32, h: self.h as f32 };
        self.window.layout(&self.ui, b);
    }
    /// Pointer down and up at window point (x, y); true when a view handled it.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let d = self.window.event(&ViewEvent::Pointer(Event::Down(x, y)), &mut self.model);
        let u = self.window.event(&ViewEvent::Pointer(Event::Up(x, y)), &mut self.model);
        d || u
    }
    /// `XCUICoordinate.tap()`: pointer down and up at window point (x, y) through the hit test; true when handled.
    pub fn tap(&mut self, x: f32, y: f32) -> bool {
        self.click(x, y)
    }
    /// `XCUICoordinate.press(forDuration:thenDragTo:)`-style swipe: pointer down at `from`, [`SWIPE_STEPS`] moves
    /// along the straight line, then up at `to`. True when any view handled one of those events.
    pub fn swipe(&mut self, from: (f32, f32), to: (f32, f32)) -> bool {
        let mut handled = self.window.event(&ViewEvent::Pointer(Event::Down(from.0, from.1)), &mut self.model);
        for i in 1..=SWIPE_STEPS {
            let t = i as f32 / SWIPE_STEPS as f32;
            let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            handled |= self.window.event(&ViewEvent::Pointer(Event::Move(x, y)), &mut self.model);
        }
        handled | self.window.event(&ViewEvent::Pointer(Event::Up(to.0, to.1)), &mut self.model)
    }
    /// A key event to the first responder.
    pub fn key(&mut self, k: KeyInput) -> bool {
        self.window.event(&ViewEvent::Key(k), &mut self.model)
    }
    /// Types `s` into the first responder.
    pub fn type_text(&mut self, s: &str) -> bool {
        self.key(KeyInput::Text(s.into()))
    }
    /// `XCUIElement.typeText`: taps the centre of the element with this role and label, then types `s`.
    /// False when no such element exists or the tap was not handled.
    pub fn type_into(&mut self, role: Role, label: &str, s: &str) -> bool {
        let Some(mut el) = self.element(role, label) else { return false };
        el.click() && self.type_text(s)
    }
    /// Resizes the window (points) and lays out again.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        self.relayout();
    }
    /// Renders the whole window and returns its pixels (`w * h`, packed as `kit::pack`).
    pub fn draw(&mut self) -> Vec<u32> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        self.window.invalidate_all();
        self.window.draw(&mut s, &self.ui);
        px
    }
    /// Accessibility elements in tree order.
    pub fn elements(&self) -> Vec<Node> {
        self.window.ax_nodes()
    }
    /// `app.buttons["Save"]`: the first element with this role and label.
    pub fn element(&mut self, role: Role, label: &str) -> Option<Element<'_, S>> {
        let node = self.window.ax_nodes().into_iter().find(|n| n.role == role && n.label == label)?;
        Some(Element { win: self, node })
    }
    /// True when an element with this role and label exists.
    pub fn exists(&mut self, role: Role, label: &str) -> bool {
        self.element(role, label).is_some()
    }
}

/// `XCUIElement`: a snapshot of one accessibility node plus the window to act on it.
pub struct Element<'a, S: Responder> {
    win: &'a mut TestWindow<S>,
    node: Node,
}

impl<S: Responder> Element<'_, S> {
    /// Always true: `element` returns `None` for a missing element.
    pub fn exists(&self) -> bool {
        true
    }
    pub fn label(&self) -> &str {
        &self.node.label
    }
    /// The value at query time.
    pub fn value(&self) -> &str {
        &self.node.value
    }
    pub fn is_enabled(&self) -> bool {
        self.node.enabled
    }
    /// Window-point frame.
    pub fn frame(&self) -> Rect {
        self.node.bounds
    }
    /// `AXPress`; true when handled.
    pub fn press(&mut self) -> bool {
        self.win.window.ax_press(self.node.id, &mut self.win.model)
    }
    /// `AXValue` write; true when accepted.
    pub fn set_value(&mut self, v: &str) -> bool {
        self.win.window.ax_set_value(self.node.id, v, &mut self.win.model)
    }
    /// Clicks the centre of the frame through the real hit test.
    pub fn click(&mut self) -> bool {
        let f = self.node.bounds;
        self.win.click(f.x + f.w / 2.0, f.y + f.h / 2.0)
    }
}

/// `XCTAssertTrue(element.exists)`: panics naming the role and label, and what exists instead.
#[macro_export]
macro_rules! assert_element {
    ($win:expr, $role:expr, $label:expr) => {
        if !$win.exists($role, $label) {
            panic!("no {:?} labelled {:?}; elements: {:?}", $role, $label, $win.elements().iter().map(|n| (n.role, n.label.clone())).collect::<Vec<_>>());
        }
    };
}

// ---- StoreKit test session -------------------------------------------------------------------------------------------

/// One scripted answer from the store to a purchase attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreAnswer {
    /// The store's result: a final outcome, or `Pending` when a later answer finishes the purchase.
    Result(bifrost_ui::purchase::Outcome),
    /// The store could not complete the attempt.
    Fail(bifrost_ui::purchase::Failure),
}

/// A scripted store for purchase tests (StoreKit's test session role). Each purchase takes answers from the front of
/// the script, so an app's purchase code runs with no store and no account. When the script runs out before a final
/// result, the attempt fails with `Failure::StoreUnavailable`.
#[derive(Clone, Debug, Default)]
pub struct StoreSession {
    script: std::collections::VecDeque<StoreAnswer>,
}

impl StoreSession {
    /// A session that answers with `answers`, in order, across every purchase it runs.
    pub fn new(answers: impl IntoIterator<Item = StoreAnswer>) -> Self {
        StoreSession { script: answers.into_iter().collect() }
    }

    /// Answers still waiting in the script.
    pub fn remaining(&self) -> usize {
        self.script.len()
    }

    /// Runs one purchase of `product` at `price` through the scripted answers and returns the attempt.
    pub fn purchase(&mut self, product: &str, price: bifrost_ui::purchase::Price) -> bifrost_ui::purchase::Purchase {
        use bifrost_ui::purchase::{Failure, Outcome, Purchase};
        let mut attempt = Purchase::new(product, price);
        attempt.begin();
        loop {
            match self.script.pop_front() {
                None => {
                    attempt.fail(Failure::StoreUnavailable);
                    break;
                }
                Some(StoreAnswer::Fail(failure)) => {
                    attempt.fail(failure);
                    break;
                }
                Some(StoreAnswer::Result(outcome)) => {
                    let finished = outcome != Outcome::Pending;
                    attempt.complete(outcome);
                    if finished {
                        break;
                    }
                }
            }
        }
        attempt
    }
}

#[cfg(test)]
mod store_session_tests {
    use super::*;
    use bifrost_ui::purchase::{Failure, Outcome, Price, Step};

    fn price() -> Price {
        Price::new(990, 2, "USD")
    }

    #[test]
    fn a_success_answer_finishes_the_purchase_and_allows_install() {
        let mut store = StoreSession::new([StoreAnswer::Result(Outcome::Success(7))]);
        let attempt = store.purchase("app.pro", price());
        assert_eq!(attempt.outcome(), Some(&Outcome::Success(7)));
        assert!(attempt.may_install());
        assert_eq!(store.remaining(), 0);
    }

    #[test]
    fn a_pending_answer_waits_for_the_next_answer() {
        let mut store = StoreSession::new([StoreAnswer::Result(Outcome::Pending), StoreAnswer::Result(Outcome::UserCancelled)]);
        let attempt = store.purchase("app.pro", price());
        assert_eq!(attempt.outcome(), Some(&Outcome::UserCancelled));
        assert!(!attempt.may_install());
    }

    #[test]
    fn a_failure_answer_ends_the_attempt_and_can_be_retried() {
        let mut store = StoreSession::new([StoreAnswer::Fail(Failure::NetworkUnavailable)]);
        let mut attempt = store.purchase("app.pro", price());
        assert_eq!(attempt.failure(), Some(Failure::NetworkUnavailable));
        assert!(attempt.retry());
        assert_eq!(attempt.step(), &Step::Idle);
    }

    #[test]
    fn an_empty_script_fails_the_attempt_with_store_unavailable() {
        let mut store = StoreSession::default();
        let attempt = store.purchase("app.pro", price());
        assert_eq!(attempt.failure(), Some(Failure::StoreUnavailable));
    }

    #[test]
    fn a_script_pending_with_no_answer_left_fails_the_attempt() {
        let mut store = StoreSession::new([StoreAnswer::Result(Outcome::Pending)]);
        let attempt = store.purchase("app.pro", price());
        assert_eq!(attempt.step(), &Step::Failed(Failure::StoreUnavailable));
    }

    #[test]
    fn answers_are_used_in_order_across_purchases() {
        let mut store = StoreSession::new([
            StoreAnswer::Result(Outcome::Success(1)),
            StoreAnswer::Fail(Failure::NotAllowed),
        ]);
        assert_eq!(store.purchase("a", price()).outcome(), Some(&Outcome::Success(1)));
        let mut second = store.purchase("b", price());
        assert_eq!(second.failure(), Some(Failure::NotAllowed));
        assert!(!second.retry(), "a restricted account may not retry");
        assert_eq!(store.remaining(), 0);
    }
}
