//! Nestable animation transactions: a stack of settings (duration, timing curve, disable actions, completion block,
//! named values) that decide how a property change animates, and a completion that waits for the animations created
//! inside it. Pure bookkeeping; the caller creates the animations and reports when they stop.
//!
//! * `begin` pushes a transaction that starts with its parent's settings; `commit` pops it. Without an explicit
//!   transaction a change uses an implicit one that `flush` commits.
//! * A completion block runs once the transaction is committed and every animation registered in it has stopped.
//! * `lock`/`unlock` count nested locks; the caller may skip drawing while locked.

use crate::anim::Curve;
use crate::keypath_anim::TimingFunction;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The default animation duration of a transaction, in seconds.
pub const DEFAULT_DURATION: f32 = 0.25;

/// A value stored under a name in a transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum TxValue {
    Number(f64),
    Flag(bool),
    Text(String),
}

#[derive(Default)]
struct GroupState {
    pending: usize,
    committed: bool,
    completion: Option<Box<dyn FnOnce()>>,
}

type Group = Rc<RefCell<GroupState>>;

fn settle(g: &Group) {
    let run = {
        let mut s = g.borrow_mut();
        if s.committed && s.pending == 0 { s.completion.take() } else { None }
    };
    if let Some(c) = run {
        c();
    }
}

/// Proof that an animation was registered with a transaction; `finish` tells the transaction it stopped.
pub struct TxToken {
    group: Group,
}

impl TxToken {
    pub fn finish(self) {
        {
            let mut s = self.group.borrow_mut();
            s.pending = s.pending.saturating_sub(1);
        }
        settle(&self.group);
    }
}

struct Frame {
    duration: f32,
    curve: Option<Curve>,
    disable_actions: bool,
    values: HashMap<String, TxValue>,
    group: Group,
    explicit: bool,
}

impl Frame {
    fn root(explicit: bool) -> Frame {
        Frame { duration: DEFAULT_DURATION, curve: None, disable_actions: false, values: HashMap::new(), group: Group::default(), explicit }
    }
    fn child_of(parent: &Frame) -> Frame {
        Frame { duration: parent.duration, curve: parent.curve, disable_actions: parent.disable_actions, values: parent.values.clone(), group: Group::default(), explicit: true }
    }
}

/// The transaction stack of one window or layer tree.
#[derive(Default)]
pub struct TransactionStack {
    frames: Vec<Frame>,
    locks: u32,
}

impl TransactionStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes an explicit transaction that inherits its parent's settings.
    pub fn begin(&mut self) {
        let f = match self.frames.last() {
            Some(p) => Frame::child_of(p),
            None => Frame::root(true),
        };
        self.frames.push(f);
    }

    /// Pops the innermost explicit transaction and returns true; false when none is open (an implicit one is left for
    /// `flush`). The completion block runs now if no animation is pending.
    pub fn commit(&mut self) -> bool {
        if !self.frames.last().is_some_and(|f| f.explicit) {
            return false;
        }
        if let Some(f) = self.frames.pop() {
            f.group.borrow_mut().committed = true;
            settle(&f.group);
        }
        true
    }

    /// Commits the implicit transaction if no explicit one is open.
    pub fn flush(&mut self) {
        if self.frames.len() == 1 && !self.frames[0].explicit {
            if let Some(f) = self.frames.pop() {
                f.group.borrow_mut().committed = true;
                settle(&f.group);
            }
        }
    }

    /// Number of open explicit transactions.
    pub fn depth(&self) -> usize {
        self.frames.iter().filter(|f| f.explicit).count()
    }

    fn top(&mut self) -> &mut Frame {
        if self.frames.is_empty() {
            self.frames.push(Frame::root(false));
        }
        let n = self.frames.len();
        &mut self.frames[n - 1]
    }

    pub fn animation_duration(&self) -> f32 {
        self.frames.last().map_or(DEFAULT_DURATION, |f| f.duration)
    }
    pub fn set_animation_duration(&mut self, secs: f32) {
        self.top().duration = secs.max(0.0);
    }

    /// The curve set for the transaction; `None` means the default timing function.
    pub fn animation_curve(&self) -> Option<Curve> {
        self.frames.last().and_then(|f| f.curve)
    }
    pub fn set_animation_curve(&mut self, c: Option<Curve>) {
        self.top().curve = c;
    }
    pub fn set_animation_timing_function(&mut self, t: TimingFunction) {
        self.top().curve = Some(t.curve());
    }

    pub fn disable_actions(&self) -> bool {
        self.frames.last().is_some_and(|f| f.disable_actions)
    }
    pub fn set_disable_actions(&mut self, on: bool) {
        self.top().disable_actions = on;
    }

    /// Runs `f` once the current transaction is committed and its animations have stopped. Replaces an earlier block.
    pub fn set_completion_block(&mut self, f: impl FnOnce() + 'static) {
        self.top().group.borrow_mut().completion = Some(Box::new(f));
    }

    pub fn value_for_key(&self, key: &str) -> Option<&TxValue> {
        self.frames.last().and_then(|f| f.values.get(key))
    }
    pub fn set_value_for_key(&mut self, key: &str, v: TxValue) {
        self.top().values.insert(key.into(), v);
    }

    pub fn lock(&mut self) {
        self.locks += 1;
    }
    pub fn unlock(&mut self) {
        self.locks = self.locks.saturating_sub(1);
    }
    pub fn is_locked(&self) -> bool {
        self.locks > 0
    }

    /// What an implicit change animates with now: `None` when actions are disabled or the duration is zero, else the
    /// duration and curve (the default ease when none was set).
    pub fn implicit_timing(&self) -> Option<(f32, Curve)> {
        let Some(f) = self.frames.last() else {
            return Some((DEFAULT_DURATION, TimingFunction::Default.curve()));
        };
        if f.disable_actions || f.duration <= 0.0 {
            return None;
        }
        Some((f.duration, f.curve.unwrap_or_else(|| TimingFunction::Default.curve())))
    }

    /// Registers an animation created in the current transaction; its completion waits for `TxToken::finish`.
    pub fn register_animation(&mut self) -> TxToken {
        let group = self.top().group.clone();
        group.borrow_mut().pending += 1;
        TxToken { group }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn flag() -> (Rc<Cell<u32>>, impl FnOnce() + 'static) {
        let c = Rc::new(Cell::new(0));
        let d = c.clone();
        (c, move || d.set(d.get() + 1))
    }

    #[test]
    fn defaults_and_setters() {
        let mut t = TransactionStack::new();
        assert_eq!(t.animation_duration(), DEFAULT_DURATION);
        assert!(!t.disable_actions());
        assert_eq!(t.animation_curve(), None);
        t.begin();
        t.set_animation_duration(1.5);
        t.set_animation_timing_function(TimingFunction::EaseIn);
        assert_eq!(t.animation_duration(), 1.5);
        assert_eq!(t.animation_curve(), Some(Curve::EaseIn));
        assert_eq!(t.implicit_timing(), Some((1.5, Curve::EaseIn)));
        assert!(t.commit());
        assert_eq!(t.animation_duration(), DEFAULT_DURATION);
    }

    #[test]
    fn nested_transactions_inherit_and_restore() {
        let mut t = TransactionStack::new();
        t.begin();
        t.set_animation_duration(2.0);
        t.set_value_for_key("tag", TxValue::Text("outer".into()));
        t.begin();
        assert_eq!(t.animation_duration(), 2.0, "inherited");
        assert_eq!(t.value_for_key("tag"), Some(&TxValue::Text("outer".into())));
        t.set_animation_duration(0.5);
        t.set_value_for_key("tag", TxValue::Number(3.0));
        assert_eq!(t.depth(), 2);
        t.commit();
        assert_eq!(t.animation_duration(), 2.0);
        assert_eq!(t.value_for_key("tag"), Some(&TxValue::Text("outer".into())));
        assert_eq!(t.depth(), 1);
        t.commit();
        assert_eq!(t.depth(), 0);
        assert!(!t.commit(), "nothing left to commit");
    }

    #[test]
    fn disable_actions_and_zero_duration_stop_implicit_animation() {
        let mut t = TransactionStack::new();
        t.begin();
        assert!(t.implicit_timing().is_some());
        assert_eq!(t.implicit_timing().unwrap().1, TimingFunction::Default.curve());
        t.begin();
        t.set_disable_actions(true);
        assert!(t.implicit_timing().is_none());
        t.commit();
        t.set_animation_duration(0.0);
        assert!(t.implicit_timing().is_none());
        t.commit();
        assert_eq!(t.implicit_timing(), Some((DEFAULT_DURATION, TimingFunction::Default.curve())), "no transaction open: the defaults");
    }

    #[test]
    fn completion_runs_on_commit_without_animations() {
        let mut t = TransactionStack::new();
        let (n, f) = flag();
        t.begin();
        t.set_completion_block(f);
        assert_eq!(n.get(), 0);
        t.commit();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn completion_waits_for_every_registered_animation() {
        let mut t = TransactionStack::new();
        let (n, f) = flag();
        t.begin();
        t.set_completion_block(f);
        let a = t.register_animation();
        let b = t.register_animation();
        t.commit();
        assert_eq!(n.get(), 0, "committed but animating");
        a.finish();
        assert_eq!(n.get(), 0);
        b.finish();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn animations_finishing_before_commit_do_not_run_the_block_early() {
        let mut t = TransactionStack::new();
        let (n, f) = flag();
        t.begin();
        t.set_completion_block(f);
        let a = t.register_animation();
        a.finish();
        assert_eq!(n.get(), 0);
        t.commit();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn an_implicit_transaction_is_committed_by_flush() {
        let mut t = TransactionStack::new();
        let (n, f) = flag();
        t.set_completion_block(f); // no begin: implicit
        assert_eq!(t.depth(), 0);
        assert!(!t.commit(), "an implicit transaction is not committed by commit");
        t.flush();
        assert_eq!(n.get(), 1);
        t.flush();
        assert_eq!(n.get(), 1);
        t.begin();
        let (m, g) = flag();
        t.set_completion_block(g);
        t.flush();
        assert_eq!(m.get(), 0, "flush does nothing inside an explicit transaction");
        t.commit();
        assert_eq!(m.get(), 1);
    }

    #[test]
    fn locks_nest() {
        let mut t = TransactionStack::new();
        assert!(!t.is_locked());
        t.lock();
        t.lock();
        t.unlock();
        assert!(t.is_locked());
        t.unlock();
        t.unlock();
        assert!(!t.is_locked());
    }
}
