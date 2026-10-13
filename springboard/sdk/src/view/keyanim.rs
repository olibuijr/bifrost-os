//! Key-path animation of views: `add_animation(view, key, animation)`, presentation versus model values, implicit
//! per-key animation under the window's transaction stack, and nested transactions.
//!
//! A view exposes these animatable keys: `frame` (rect; also `frame.origin`, `frame.size` and any component such as
//! `frame.size.width`), `position` (the centre of the frame), `bounds` (the frame's size at origin zero; writing it
//! resizes about the centre), `opacity`, `backgroundColor`. Animations never touch the model: the window keeps the
//! model values aside while an animation runs and shows the presentation values in the view, and puts the model back
//! when the last animation on the view is gone. Setting a property directly while an animation runs changes the model.
//!
//! The engine is `bifrost_ui::keypath_anim`; [`ViewState`] is the [`Animatable`] adapter, so layers plug in the same way.
//! Do not animate one property both through [`Window::animate`] and with a key animation: the older tracks cancel when
//! they see the value changed under them.
use super::*;
use bifrost_ui::anim::Curve;
use bifrost_ui::anim_transaction::{TransactionStack, TxToken, TxValue};
use bifrost_ui::anim_value::{read_key, write_key};
use bifrost_ui::keypath_anim::{AnimationSet, KeyAnimation, TimingFunction};
use bifrost_ui::anim_value::{AnimValue, Animatable};
use std::collections::HashMap;

/// The animatable state of a view, as the [`Animatable`] adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub frame: Rect,
    pub opacity: f32,
    pub background: Option<u32>,
}

impl ViewState {
    pub fn of(b: &ViewBase) -> ViewState {
        ViewState { frame: b.frame, opacity: b.opacity, background: b.background }
    }
    fn apply(&self, b: &mut ViewBase) {
        b.frame = self.frame;
        b.opacity = self.opacity;
        b.background = self.background;
    }
    fn centre(&self) -> (f32, f32) {
        (self.frame.x + self.frame.w / 2.0, self.frame.y + self.frame.h / 2.0)
    }
    fn resize_about_centre(&mut self, w: f32, h: f32) {
        let (cx, cy) = self.centre();
        self.frame = Rect { x: cx - w / 2.0, y: cy - h / 2.0, w, h };
    }
}

impl Animatable for ViewState {
    fn value(&self, key: &str) -> Option<AnimValue> {
        let f = self.frame;
        Some(match key {
            "frame" => AnimValue::Rect(f),
            "frame.origin" => AnimValue::Point(f.x, f.y),
            "frame.size" | "bounds.size" => AnimValue::Size(f.w, f.h),
            "position" => {
                let (cx, cy) = self.centre();
                AnimValue::Point(cx, cy)
            }
            "bounds" => AnimValue::Rect(Rect { x: 0.0, y: 0.0, w: f.w, h: f.h }),
            "opacity" => AnimValue::Scalar(self.opacity),
            "backgroundColor" => AnimValue::from_argb(self.background.unwrap_or(0)),
            _ => return None,
        })
    }
    fn set_value(&mut self, key: &str, value: AnimValue) -> bool {
        match (key, value) {
            ("frame", AnimValue::Rect(r)) => self.frame = r,
            ("frame.origin", AnimValue::Point(x, y)) => {
                self.frame.x = x;
                self.frame.y = y;
            }
            ("frame.size", AnimValue::Size(w, h)) | ("bounds.size", AnimValue::Size(w, h)) => {
                if key == "frame.size" {
                    self.frame.w = w;
                    self.frame.h = h;
                } else {
                    self.resize_about_centre(w, h);
                }
            }
            ("position", AnimValue::Point(x, y)) => {
                self.frame.x = x - self.frame.w / 2.0;
                self.frame.y = y - self.frame.h / 2.0;
            }
            ("bounds", AnimValue::Rect(r)) => self.resize_about_centre(r.w, r.h),
            ("opacity", AnimValue::Scalar(v)) => self.opacity = v,
            ("backgroundColor", c @ AnimValue::Color(_)) => {
                let w = c.to_argb().unwrap_or(0);
                self.background = if w == 0 { None } else { Some(w) };
            }
            _ => return false,
        }
        true
    }
}

struct ViewAnims {
    set: AnimationSet,
    /// the values the view has when no animation runs
    model: ViewState,
    /// what the window last wrote into the view
    shown: ViewState,
    tokens: HashMap<String, TxToken>,
}

/// Key animations and the transaction stack of a window.
#[derive(Default)]
pub(super) struct Keyed {
    views: HashMap<u64, ViewAnims>,
    pub(super) tx: TransactionStack,
}

impl Keyed {
    pub(super) fn active(&self) -> bool {
        !self.views.is_empty()
    }
    pub(super) fn animating(&self, id: Option<u64>) -> bool {
        self.views.iter().any(|(k, v)| id.map_or(true, |i| i == *k) && !v.set.is_empty())
    }
    /// Mirrors a `Window::animate` or `disable_actions` scope in the transaction stack.
    pub(super) fn open_scope(&mut self, timing: Option<Timing>) {
        self.tx.begin();
        match timing {
            Some(t) => {
                if t.duration >= 0.0 {
                    self.tx.set_animation_duration(t.duration);
                }
                self.tx.set_animation_curve(Some(t.curve));
            }
            None => self.tx.set_disable_actions(true),
        }
    }
}

fn view_state(v: &dyn View, id: u64) -> Option<ViewState> {
    if v.id() == id {
        return Some(ViewState::of(v.base()));
    }
    v.children().iter().find_map(|c| view_state(c.as_ref(), id))
}

fn walk_mut(v: &mut dyn View, f: &mut dyn FnMut(&mut dyn View)) {
    f(v);
    v.children_mut().iter_mut().for_each(|c| walk_mut(c.as_mut(), f));
}

impl<S: Responder> Window<S> {
    /// Adds an animation by key: runs `anim` on view `id`. Under the same `key` an earlier animation is replaced (and told it
    /// did not finish); without a key one is generated. Returns the key, or `None` when the view does not exist.
    /// The animation's clock starts at the next [`Window::tick`]; its delegate hears start and stop.
    pub fn add_animation(&mut self, id: u64, anim: KeyAnimation, key: Option<&str>) -> Option<String> {
        let state = view_state(self.root(), id)?;
        let va = self.anim.keyed.views.entry(id).or_insert_with(|| ViewAnims { set: AnimationSet::new(), model: state, shown: state, tokens: HashMap::new() });
        let k = va.set.add(anim, key);
        // a replaced animation has stopped: its transaction no longer waits for it
        for (stopped, _) in va.set.take_stopped() {
            if let Some(t) = va.tokens.remove(&stopped) {
                t.finish();
            }
        }
        va.tokens.insert(k.clone(), self.anim.keyed.tx.register_animation());
        self.full = true;
        Some(k)
    }

    /// Removes the animation under `key`.
    pub fn remove_animation(&mut self, id: u64, key: &str) -> bool {
        let Some(va) = self.anim.keyed.views.get_mut(&id) else { return false };
        let ok = va.set.remove(key);
        self.settle_view(id);
        ok
    }

    /// Removes every animation of the view.
    pub fn remove_all_animations(&mut self, id: u64) {
        if let Some(va) = self.anim.keyed.views.get_mut(&id) {
            va.set.remove_all();
        }
        self.settle_view(id);
    }

    /// Keys of the animations on the view.
    pub fn animation_keys(&self, id: u64) -> Vec<String> {
        self.anim.keyed.views.get(&id).map(|v| v.set.keys()).unwrap_or_default()
    }

    /// The animation under `key`, if any .
    pub fn animation_for_key(&self, id: u64, key: &str) -> Option<&KeyAnimation> {
        self.anim.keyed.views.get(&id)?.set.get(key)
    }

    /// The value of `key` as the view shows it now, animations included .
    pub fn presentation_value(&self, id: u64, key: &str) -> Option<AnimValue> {
        read_key(&view_state(self.root(), id)?, key)
    }

    /// The value of `key` without animations .
    pub fn model_value(&self, id: u64, key: &str) -> Option<AnimValue> {
        match self.anim.keyed.views.get(&id) {
            Some(va) => read_key(&va.model, key),
            None => read_key(&view_state(self.root(), id)?, key),
        }
    }

    /// Sets the model value of `key` at once, with no animation. A running animation keeps showing its own result.
    pub fn set_model_value(&mut self, id: u64, key: &str, value: AnimValue) -> bool {
        let mut done = false;
        if let Some(va) = self.anim.keyed.views.get_mut(&id) {
            done = write_key(&mut va.model, key, value);
            if done {
                return true;
            }
        }
        walk_mut(self.root.as_mut(), &mut |v| {
            if v.id() == id {
                let mut st = ViewState::of(v.base());
                if write_key(&mut st, key, value) {
                    st.apply(v.base_mut());
                    v.base_mut().dirty = true;
                    done = true;
                }
            }
        });
        self.full |= done;
        done
    }

    /// Sets `key` to `value` the way a property assignment does: under the current transaction it animates from the
    /// value on screen (a spring curve keeps the velocity of a spring already running on the key), unless actions are
    /// disabled, the duration is zero, Reduce Motion is on for a moving property, or nothing changes.
    pub fn set_value_animated(&mut self, id: u64, key: &str, value: AnimValue) -> bool {
        let Some(shown) = view_state(self.root(), id) else { return false };
        let Some(from) = read_key(&shown, key) else { return false };
        if from.kind() != value.kind() {
            return false;
        }
        let reduced = self.anim.reduce.unwrap_or_else(crate::kit::appearance::reduce_motion);
        let moving = !(key.starts_with("opacity") || key.starts_with("backgroundColor"));
        let timing = self.anim.keyed.tx.implicit_timing().filter(|_| !(reduced && moving));
        let Some((duration, curve)) = timing else {
            return self.set_model_value(id, key, value);
        };
        let model = self.model_value(id, key).unwrap_or(from);
        if model == value && from == value {
            return true;
        }
        self.set_model_value_only(id, key, value);
        let implicit = format!("implicit:{key}");
        let now = self.anim.clock;
        if let Some(va) = self.anim.keyed.views.get_mut(&id) {
            if va.set.get(&implicit).is_some() {
                // the model already holds the new value; the running animation is redirected from the presentation
                let model = va.model;
                if va.set.retarget(&implicit, value, now, &model) {
                    return true;
                }
            }
        }
        let curve = if reduced { Curve::Linear } else { curve };
        let anim = KeyAnimation::implicit(key, from, value, duration, curve);
        self.add_animation(id, anim, Some(&implicit)).is_some()
    }

    /// Writes the model value without touching what is shown (creating the bookkeeping for the view if needed).
    fn set_model_value_only(&mut self, id: u64, key: &str, value: AnimValue) {
        if let Some(va) = self.anim.keyed.views.get_mut(&id) {
            write_key(&mut va.model, key, value);
        } else if let Some(state) = view_state(self.root(), id) {
            let mut model = state;
            if write_key(&mut model, key, value) {
                self.anim.keyed.views.insert(id, ViewAnims { set: AnimationSet::new(), model, shown: state, tokens: HashMap::new() });
            }
        }
    }

    /// Opens a nested transaction.
    pub fn begin_transaction(&mut self) {
        self.anim.keyed.tx.begin();
    }
    /// Closes the innermost transaction; false when none is open.
    pub fn commit_transaction(&mut self) -> bool {
        self.anim.keyed.tx.commit()
    }
    /// Commits the implicit transaction.
    pub fn flush_transactions(&mut self) {
        self.anim.keyed.tx.flush();
    }
    /// Sets the animation duration of the current transaction.
    pub fn set_animation_duration(&mut self, secs: f32) {
        self.anim.keyed.tx.set_animation_duration(secs);
    }
    pub fn animation_duration(&self) -> f32 {
        self.anim.keyed.tx.animation_duration()
    }
    /// Sets the timing function of the current transaction.
    pub fn set_animation_timing_function(&mut self, t: TimingFunction) {
        self.anim.keyed.tx.set_animation_timing_function(t);
    }
    /// Turns implicit animation off (or on) for the current transaction.
    pub fn set_transaction_disable_actions(&mut self, on: bool) {
        self.anim.keyed.tx.set_disable_actions(on);
    }
    /// Sets a block that runs after the transaction is committed and its animations stopped.
    pub fn set_transaction_completion(&mut self, f: impl FnOnce() + 'static) {
        self.anim.keyed.tx.set_completion_block(f);
    }
    /// Stores a named value in the current transaction.
    pub fn set_transaction_value(&mut self, key: &str, v: TxValue) {
        self.anim.keyed.tx.set_value_for_key(key, v);
    }
    pub fn transaction_value(&self, key: &str) -> Option<&TxValue> {
        self.anim.keyed.tx.value_for_key(key)
    }

    /// Ends the bookkeeping of a view whose animations are all gone: the model goes back into the view.
    fn settle_view(&mut self, id: u64) {
        let Some(va) = self.anim.keyed.views.get_mut(&id) else { return };
        for (k, _) in va.set.take_stopped() {
            if let Some(t) = va.tokens.remove(&k) {
                t.finish();
            }
        }
        if !va.set.is_empty() {
            return;
        }
        let Some(mut va) = self.anim.keyed.views.remove(&id) else { return };
        for (_, t) in va.tokens.drain() {
            t.finish();
        }
        let model = va.model;
        walk_mut(self.root.as_mut(), &mut |v| {
            if v.id() == id {
                model.apply(v.base_mut());
                v.base_mut().dirty = true;
            }
        });
        self.full = true;
    }

    /// Advances key animations to `t` and writes the presentation values into the views. Called by `advance_animations`.
    pub(super) fn advance_key_animations(&mut self, t: f64) {
        self.anim.keyed.tx.flush(); // the implicit transaction ends with the frame
        if !self.anim.keyed.active() {
            return;
        }
        let mut views = std::mem::take(&mut self.anim.keyed.views);
        let mut seen: Vec<u64> = Vec::new();
        let mut changed = false;
        walk_mut(self.root.as_mut(), &mut |v| {
            let id = v.id();
            let Some(va) = views.get_mut(&id) else { return };
            seen.push(id);
            let base = v.base_mut();
            let cur = ViewState::of(base);
            // a value somebody else wrote into the view is a new model value
            if cur.frame != va.shown.frame {
                va.model.frame = cur.frame;
            }
            if cur.opacity != va.shown.opacity {
                va.model.opacity = cur.opacity;
            }
            if cur.background != va.shown.background {
                va.model.background = cur.background;
            }
            va.set.advance(t);
            let mut st = va.model;
            for (k, val) in va.set.presentation(t, &va.model) {
                st.set_value(&k, val);
            }
            for (k, _) in va.set.take_stopped() {
                if let Some(tok) = va.tokens.remove(&k) {
                    tok.finish();
                }
            }
            if st != cur {
                st.apply(base);
                base.dirty = true;
                changed = true;
            }
            va.shown = st;
        });
        for (id, va) in views.iter_mut() {
            if !seen.contains(id) {
                va.set.remove_all(); // the view left the tree
            }
        }
        let mut gone = Vec::new();
        for (id, va) in views.iter_mut() {
            for (k, _) in va.set.take_stopped() {
                if let Some(tok) = va.tokens.remove(&k) {
                    tok.finish();
                }
            }
            if va.set.is_empty() {
                gone.push(*id);
            }
        }
        for id in gone {
            if let Some(mut va) = views.remove(&id) {
                for (_, tok) in va.tokens.drain() {
                    tok.finish();
                }
            }
        }
        // views animated during the callbacks above are kept by merging
        for (id, va) in std::mem::take(&mut self.anim.keyed.views) {
            views.entry(id).or_insert(va);
        }
        self.anim.keyed.views = views;
        self.full |= changed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::keypath_anim::{AnimationDelegate, FillMode, SpringSpec};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn win() -> (Window, u64) {
        let c = Container::new(vec![Box::new(Container::default())]);
        let mut w: Window = Window::new(c, 10);
        let id = w.root().children()[0].id();
        w.root_mut().children_mut()[0].set_frame(rect(0.0, 0.0, 100.0, 50.0));
        w.root_mut().set_frame(rect(0.0, 0.0, 400.0, 400.0));
        w.set_reduce_motion(Some(false));
        (w, id)
    }

    fn base(w: &Window, id: u64) -> ViewState {
        ViewState::of(w.view::<Container>(id).unwrap().base())
    }

    fn sc(v: f32) -> AnimValue {
        AnimValue::Scalar(v)
    }

    #[test]
    fn view_state_keys() {
        let mut s = ViewState { frame: rect(10.0, 20.0, 100.0, 50.0), opacity: 0.5, background: None };
        assert_eq!(s.value("position"), Some(AnimValue::Point(60.0, 45.0)));
        assert_eq!(s.value("bounds"), Some(AnimValue::Rect(rect(0.0, 0.0, 100.0, 50.0))));
        assert_eq!(read_key(&s, "frame.size.width"), Some(sc(100.0)));
        assert_eq!(read_key(&s, "position.y"), Some(sc(45.0)));
        assert!(write_key(&mut s, "position.x", sc(100.0)));
        assert_eq!(s.frame.x, 50.0);
        assert!(s.set_value("bounds.size", AnimValue::Size(20.0, 10.0)));
        assert_eq!(s.frame, rect(90.0, 40.0, 20.0, 10.0), "resized about the centre");
        assert!(s.set_value("backgroundColor", AnimValue::from_argb(0xFF00FF00)));
        assert_eq!(s.background, Some(0xFF00FF00));
        assert!(s.set_value("backgroundColor", AnimValue::Color([0.0; 4])));
        assert_eq!(s.background, None);
        assert!(!s.set_value("opacity", AnimValue::Point(0.0, 0.0)));
        assert!(!s.set_value("nope", sc(1.0)));
        assert_eq!(s.value("nope"), None);
    }

    #[test]
    fn an_explicit_animation_shows_presentation_and_keeps_the_model() {
        let (mut w, id) = win();
        let a = KeyAnimation::basic("position.x").duration(1.0).from(sc(50.0)).to(sc(250.0));
        let key = w.add_animation(id, a, Some("slide")).unwrap();
        assert_eq!(key, "slide");
        assert_eq!(base(&w, id).frame.x, 0.0, "nothing moves before the clock starts");
        w.tick(10.0);
        assert!((base(&w, id).frame.x - 0.0).abs() < 1e-3, "starts at 50 - 50 (half width)");
        w.tick(10.5);
        assert!((w.presentation_value(id, "position.x").unwrap().as_scalar().unwrap() - 150.0).abs() < 1e-3);
        assert_eq!(w.model_value(id, "position.x"), Some(sc(50.0)), "the model stays where it was");
        assert!(w.is_animating(Some(id)));
        w.tick(11.0);
        w.tick(11.1);
        assert!(!w.is_animating(None));
        assert_eq!(base(&w, id).frame, rect(0.0, 0.0, 100.0, 50.0), "back to the model");
        assert!(w.animation_keys(id).is_empty());
        assert!(w.next_wake(12.0).is_none());
    }

    #[test]
    fn fill_forwards_holds_until_removed() {
        let (mut w, id) = win();
        let a = KeyAnimation::basic("opacity").duration(0.5).from(sc(0.0)).to(sc(0.25)).fill(FillMode::Forwards).removed_on_completion(false);
        w.add_animation(id, a, Some("fade"));
        w.tick(0.0);
        w.tick(1.0);
        assert_eq!(base(&w, id).opacity, 0.25);
        assert_eq!(w.model_value(id, "opacity"), Some(sc(1.0)));
        assert!(w.remove_animation(id, "fade"));
        assert_eq!(base(&w, id).opacity, 1.0);
        assert!(!w.remove_animation(id, "fade"));
    }

    #[test]
    fn setting_the_model_while_animating_survives_the_animation() {
        let (mut w, id) = win();
        w.add_animation(id, KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(0.5)), Some("a"));
        w.tick(0.0);
        w.tick(0.5);
        assert!(w.set_model_value(id, "opacity", sc(0.8)));
        assert_eq!(w.model_value(id, "opacity"), Some(sc(0.8)));
        w.tick(0.6);
        assert!(base(&w, id).opacity < 0.5, "still showing the animation");
        w.tick(2.0);
        w.tick(2.1);
        assert_eq!(base(&w, id).opacity, 0.8);
    }

    #[test]
    fn a_direct_write_by_other_code_becomes_the_model() {
        let (mut w, id) = win();
        w.add_animation(id, KeyAnimation::basic("position.x").duration(1.0).from(sc(50.0)).to(sc(250.0)), Some("a"));
        w.tick(0.0);
        w.tick(0.2);
        w.view_mut::<Container>(id).unwrap().base_mut().opacity = 0.3;
        w.tick(0.4);
        assert_eq!(w.model_value(id, "opacity"), Some(sc(0.3)));
        assert_eq!(base(&w, id).opacity, 0.3);
    }

    #[derive(Default)]
    struct Log(RefCell<Vec<String>>);
    impl AnimationDelegate for Log {
        fn did_start(&self, k: &str) {
            self.0.borrow_mut().push(format!("start {k}"));
        }
        fn did_stop(&self, k: &str, f: bool) {
            self.0.borrow_mut().push(format!("stop {k} {f}"));
        }
    }

    #[test]
    fn delegate_and_transaction_completion() {
        let (mut w, id) = win();
        let log = Rc::new(Log::default());
        let done = Rc::new(Cell::new(0));
        let d = done.clone();
        w.begin_transaction();
        w.set_transaction_completion(move || d.set(d.get() + 1));
        w.add_animation(id, KeyAnimation::basic("opacity").duration(1.0).from(sc(0.0)).to(sc(1.0)).delegate(log.clone()), Some("a"));
        w.commit_transaction();
        assert_eq!(done.get(), 0);
        w.tick(0.0);
        assert_eq!(*log.0.borrow(), vec!["start a".to_string()]);
        w.tick(2.0);
        assert_eq!(*log.0.borrow(), vec!["start a".to_string(), "stop a true".to_string()]);
        assert_eq!(done.get(), 1);
    }

    #[test]
    fn replacing_and_removing_report_not_finished_and_still_complete_the_transaction() {
        let (mut w, id) = win();
        let log = Rc::new(Log::default());
        let done = Rc::new(Cell::new(0));
        let d = done.clone();
        w.begin_transaction();
        w.set_transaction_completion(move || d.set(d.get() + 1));
        let mk = || KeyAnimation::basic("opacity").duration(5.0).from(sc(0.0)).to(sc(1.0)).delegate(log.clone());
        w.add_animation(id, mk(), Some("a"));
        w.commit_transaction();
        w.tick(0.0);
        w.add_animation(id, mk(), Some("a"));
        assert_eq!(log.0.borrow().last().unwrap(), "stop a false");
        w.remove_all_animations(id);
        assert_eq!(done.get(), 1);
        assert_eq!(base(&w, id).opacity, 1.0);
    }

    #[test]
    fn unknown_views_and_keys() {
        let (mut w, id) = win();
        assert!(w.add_animation(9999, KeyAnimation::basic("opacity"), None).is_none());
        assert!(!w.set_value_animated(id, "nope", sc(1.0)));
        assert!(!w.set_value_animated(id, "opacity", AnimValue::Point(0.0, 0.0)));
        assert_eq!(w.presentation_value(9999, "opacity"), None);
        assert!(w.animation_for_key(id, "x").is_none());
    }

    #[test]
    fn set_value_animated_follows_the_transaction() {
        let (mut w, id) = win();
        // no transaction: the default timing applies, a quarter second
        w.set_value_animated(id, "opacity", sc(0.0));
        assert_eq!(base(&w, id).opacity, 1.0, "starts from what is shown");
        w.tick(0.0);
        w.tick(0.125);
        let mid = base(&w, id).opacity;
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(w.model_value(id, "opacity"), Some(sc(0.0)), "the model has the new value at once");
        w.tick(1.0);
        w.tick(1.1);
        assert_eq!(base(&w, id).opacity, 0.0);
        // actions disabled: immediate
        w.begin_transaction();
        w.set_transaction_disable_actions(true);
        w.set_value_animated(id, "opacity", sc(1.0));
        w.commit_transaction();
        assert_eq!(base(&w, id).opacity, 1.0);
        assert!(!w.is_animating(None));
        // custom duration
        w.begin_transaction();
        w.set_animation_duration(2.0);
        w.set_animation_timing_function(TimingFunction::Linear);
        w.set_value_animated(id, "opacity", sc(0.0));
        w.commit_transaction();
        w.tick(5.0);
        w.tick(6.0);
        assert!((base(&w, id).opacity - 0.5).abs() < 1e-3);
        w.tick(8.0);
        w.tick(8.1);
        assert_eq!(base(&w, id).opacity, 0.0);
    }

    #[test]
    fn a_second_set_value_animated_retargets_without_a_jump() {
        let (mut w, id) = win();
        w.begin_transaction();
        w.set_animation_duration(1.0);
        w.set_animation_timing_function(TimingFunction::Linear);
        w.set_value_animated(id, "opacity", sc(0.0));
        w.tick(0.0);
        w.tick(0.5);
        let mid = base(&w, id).opacity;
        w.set_value_animated(id, "opacity", sc(1.0));
        w.tick(0.5);
        assert!((base(&w, id).opacity - mid).abs() < 1e-3, "no jump");
        w.tick(5.0);
        w.tick(5.1);
        assert_eq!(base(&w, id).opacity, 1.0);
        w.commit_transaction();
    }

    #[test]
    fn spring_curves_in_the_transaction_make_springs() {
        let (mut w, id) = win();
        w.animate(Timing::spring(0.5, 0.4), |w| {
            w.set_value_animated(id, "opacity", sc(0.0));
        });
        let k = w.animation_keys(id);
        assert_eq!(k, vec!["implicit:opacity".to_string()]);
        let _ = SpringSpec::default();
    }

    #[test]
    fn reduce_motion_makes_moving_keys_instant_but_fades_stay() {
        let (mut w, id) = win();
        w.set_reduce_motion(Some(true));
        w.set_value_animated(id, "frame", AnimValue::Rect(rect(10.0, 10.0, 100.0, 50.0)));
        assert_eq!(base(&w, id).frame.x, 10.0);
        w.set_value_animated(id, "opacity", sc(0.0));
        assert!(w.is_animating(Some(id)));
    }

    #[test]
    fn a_view_that_leaves_the_tree_ends_its_animations() {
        let (mut w, id) = win();
        let log = Rc::new(Log::default());
        w.add_animation(id, KeyAnimation::basic("opacity").duration(5.0).from(sc(0.0)).to(sc(1.0)).delegate(log.clone()), Some("a"));
        w.tick(0.0);
        let root = w.root().id();
        w.view_mut::<Container>(root).unwrap().children.clear();
        w.tick(0.1);
        assert_eq!(log.0.borrow().last().unwrap(), "stop a false");
        assert!(!w.is_animating(None));
    }

    #[test]
    fn spring_velocity_carries_through_a_transaction_retarget() {
        let (mut w, id) = win();
        let spring = SpringSpec { stiffness: 100.0, damping: 12.0, ..SpringSpec::default() };
        let a = KeyAnimation::spring("opacity", spring).from(sc(0.0)).to(sc(100.0));
        w.add_animation(id, a, Some("implicit:opacity"));
        w.tick(0.0);
        w.tick(0.15);
        let before = base(&w, id).opacity;
        w.begin_transaction();
        w.set_animation_timing_function(TimingFunction::Linear);
        w.set_value_animated(id, "opacity", sc(200.0));
        w.commit_transaction();
        w.tick(0.15);
        assert!((base(&w, id).opacity - before).abs() < 0.5, "no jump");
    }
}
