//! Derived view state: values computed from the observable properties of a model, recomputed only when something
//! they read changed (the declarative state of a view, `@State` values and the computed properties built on them).
//!
//! A [`DerivedState`] mirrors chosen [`Observable`]s into a `bifrost_ui::attribute_graph::Graph` as inputs: a write to
//! the observable sets the graph input, which marks only the rules that depend on it. [`Derived::get`] runs the rules
//! that are out of date and nothing else, and a rule whose result did not change stops the update there. The
//! dependencies of a rule are recorded from what it reads, so they follow the rule (a rule that reads `b` only
//! when a flag is set stops depending on `b` when the flag clears).
//!
//! `observable_model!` models announce every field write to their `objectWillChange` subscribers; a view that shows
//! a value computed from several fields reads it through a `Derived`, so the announcement costs one recompute of
//! what actually depends on the field instead of a recompute of every computed value. Single-threaded, like
//! [`Observable`].

use crate::observable::{Cancellables, Observable};
use bifrost_ui::attribute_graph::{Attr, Graph, Reader};
use std::cell::RefCell;
use std::rc::Rc;

/// The graph of one view's (or one model's) derived values, and the links that keep its inputs current.
pub struct DerivedState {
    graph: Rc<RefCell<Graph>>,
    links: Cancellables,
}

impl Default for DerivedState {
    fn default() -> Self {
        DerivedState::new()
    }
}

impl DerivedState {
    pub fn new() -> DerivedState {
        DerivedState { graph: Rc::new(RefCell::new(Graph::new())), links: Cancellables::new() }
    }

    /// Mirrors `source` as a graph input: it holds the current value now and follows every later change. Rules read
    /// the returned handle through their [`Reader`]. The link lives as long as this state.
    pub fn input<T: Clone + PartialEq + 'static>(&mut self, source: &Observable<T>) -> Attr<T> {
        let attr = self.graph.borrow_mut().input(source.get());
        let graph = self.graph.clone();
        self.links.store(source.subscribe(move |value: &T| {
            let _ = graph.borrow_mut().set(attr, value.clone());
        }));
        attr
    }

    /// A value computed by `rule` from the inputs and other derived values it reads. It first runs when it is first asked
    /// for.
    pub fn derive<T: Clone + PartialEq + 'static>(&self, rule: impl Fn(&mut Reader<'_>) -> T + 'static) -> Derived<T> {
        let attr = self.graph.borrow_mut().rule(rule);
        Derived { graph: self.graph.clone(), attr }
    }

    /// How many times any rule of this state has run.
    pub fn eval_count(&self) -> u64 {
        self.graph.borrow().eval_count()
    }
}

/// A computed value of a [`DerivedState`]. Clones read the same value.
pub struct Derived<T> {
    graph: Rc<RefCell<Graph>>,
    attr: Attr<T>,
}

impl<T> Clone for Derived<T> {
    fn clone(&self) -> Self {
        Derived { graph: self.graph.clone(), attr: self.attr }
    }
}

impl<T: Clone + 'static> Derived<T> {
    /// The current value, running the rules that are out of date first.
    pub fn get(&self) -> T {
        self.graph.borrow_mut().get(self.attr)
    }

    /// The attribute, for a rule of the same state that reads this value.
    pub fn attr(&self) -> Attr<T> {
        self.attr
    }

    /// Whether the next `get` has to run a rule or check the ones this value depends on.
    pub fn is_stale(&self) -> bool {
        self.graph.borrow().is_stale(self.attr)
    }

    /// How many times the value has changed.
    pub fn version(&self) -> u64 {
        self.graph.borrow().version(self.attr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::observable_model! {
        struct Cart {
            price: u32 = 10,
            count: u32 = 2,
            note: String = String::from("gift"),
        }
    }

    #[test]
    fn a_write_recomputes_only_what_depends_on_the_field() {
        let cart = Cart::new();
        let mut state = DerivedState::new();
        let price = state.input(&cart.price);
        let count = state.input(&cart.count);
        let note = state.input(&cart.note);
        let total = state.derive(move |r| r.read(price) * r.read(count));
        let label = state.derive(move |r| format!("{} ({})", r.read(note), r.read(count)));
        assert_eq!(state.eval_count(), 0, "nothing runs until a value is asked for");
        assert_eq!((total.get(), label.get()), (20, "gift (2)".to_string()));
        assert_eq!(state.eval_count(), 2);
        cart.price.set(15);
        assert!(total.is_stale() && !label.is_stale(), "the label does not read the price");
        assert_eq!(total.get(), 30);
        assert_eq!(label.get(), "gift (2)");
        assert_eq!(state.eval_count(), 3, "only the total ran again");
        cart.note.set("thanks".to_string());
        assert_eq!(label.get(), "thanks (2)");
        assert_eq!(state.eval_count(), 4);
        assert_eq!(total.get(), 30);
        assert_eq!(state.eval_count(), 4, "a current value is not recomputed");
    }

    #[test]
    fn several_writes_between_reads_cost_one_recompute() {
        let cart = Cart::new();
        let mut state = DerivedState::new();
        let price = state.input(&cart.price);
        let total = state.derive(move |r| r.read(price) + 1);
        assert_eq!(total.get(), 11);
        cart.price.set(1);
        cart.price.set(2);
        cart.price.set(3);
        assert_eq!(state.eval_count(), 1);
        assert_eq!(total.get(), 4);
        assert_eq!(state.eval_count(), 2);
    }

    #[test]
    fn a_result_that_did_not_change_stops_the_update() {
        let cart = Cart::new();
        let mut state = DerivedState::new();
        let count = state.input(&cart.count);
        let plural = state.derive(move |r| r.read(count) != 1);
        let plural_attr = plural.attr();
        let text = state.derive(move |r| (if r.read(plural_attr) { "items" } else { "item" }).to_string());
        assert_eq!(text.get(), "items");
        assert_eq!(state.eval_count(), 2);
        let first = text.version();
        cart.count.set(5);
        assert_eq!(text.get(), "items");
        assert_eq!(state.eval_count(), 3, "plural ran again; the text did not");
        assert_eq!(text.version(), first);
        cart.count.set(1);
        assert_eq!(text.get(), "item");
        assert_eq!(state.eval_count(), 5);
    }

    #[test]
    fn setting_an_equal_value_marks_nothing() {
        let cart = Cart::new();
        let mut state = DerivedState::new();
        let price = state.input(&cart.price);
        let doubled = state.derive(move |r| r.read(price) * 2);
        assert_eq!(doubled.get(), 20);
        cart.price.set(10);
        assert!(!doubled.is_stale());
        assert_eq!(state.eval_count(), 1);
    }

    #[test]
    fn the_link_ends_with_the_state() {
        let cart = Cart::new();
        let before = cart.price.subscriber_count();
        {
            let mut state = DerivedState::new();
            state.input(&cart.price);
            assert_eq!(cart.price.subscriber_count(), before + 1);
        }
        assert_eq!(cart.price.subscriber_count(), before);
    }
}
