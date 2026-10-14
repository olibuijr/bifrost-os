//! `UISpringLoadedInteraction` for BifrOSt apps: attach a handler to a drag target, feed it the drag updates, and the
//! handler runs once when a resting drag has spring-loaded the target (see `bifrost_ui::spring_load` for the model).
//!
//! An app that handles `dnd::Event::Dest` calls [`SpringLoadedInteraction::drag_updated`] for every `Entered` and `Updated`
//! with the id of the element under the pointer, and [`SpringLoadedInteraction::tick`] from its frame timer so a still
//! pointer still activates. [`SpringLoadedInteraction::progress`] drives the hover effect, and
//! [`SpringLoadedInteraction::finish`] runs when the session ends or the pointer leaves the window.
//!
//! The shell does not yet tick drag sessions by itself, so the tick is the app's job until the shell forwards it.

pub use bifrost_ui::spring_load::{Config, Event, State};
use bifrost_ui::spring_load::SpringLoad;

pub struct SpringLoadedInteraction {
    model: SpringLoad,
    handler: Box<dyn FnMut(u64)>,
}

impl SpringLoadedInteraction {
    /// `init(interactionBehavior:interactionEffect:activationHandler:)`: `handler` gets the id of the activated target.
    pub fn new(config: Config, handler: impl FnMut(u64) + 'static) -> Self {
        SpringLoadedInteraction { model: SpringLoad::new(config), handler: Box::new(handler) }
    }

    /// `init(activationHandler:)`: the default delay.
    pub fn with_default(handler: impl FnMut(u64) + 'static) -> Self {
        Self::new(Config::default(), handler)
    }

    /// The drag is over `target` (None over nothing) at `now_ms`; `allowed` is the behavior's answer for that target.
    /// Returns true when this update activated the target.
    pub fn drag_updated(&mut self, target: Option<u64>, allowed: bool, now_ms: f64) -> bool {
        let event = self.model.update(target, allowed, now_ms);
        self.dispatch(event)
    }

    /// The periodic check while a drag rests over a target. Returns true when it activated the target.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        let event = self.model.tick(now_ms);
        self.dispatch(event)
    }

    /// Hover ramp from 0 to 1 for the visual effect.
    pub fn progress(&self, now_ms: f64) -> f32 {
        self.model.progress(now_ms)
    }

    pub fn state(&self) -> State {
        self.model.state()
    }

    /// The drag ended or was cancelled.
    pub fn finish(&mut self) {
        self.model.finish();
    }

    fn dispatch(&mut self, event: Option<Event>) -> bool {
        match event {
            Some(Event::Activated(id)) => {
                (self.handler)(id);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn handler_runs_once_with_the_activated_target() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let mut s = SpringLoadedInteraction::new(Config { delay_ms: 100.0 }, move |id| sink.borrow_mut().push(id));
        assert!(!s.drag_updated(Some(4), true, 0.0));
        assert!(!s.tick(50.0));
        assert!(s.tick(100.0));
        assert!(!s.drag_updated(Some(4), true, 300.0));
        assert_eq!(*seen.borrow(), vec![4]);
        assert_eq!(s.state(), State::Activated);
    }

    #[test]
    fn finish_before_the_delay_never_calls_the_handler() {
        let seen = Rc::new(RefCell::new(0));
        let sink = seen.clone();
        let mut s = SpringLoadedInteraction::with_default(move |_| *sink.borrow_mut() += 1);
        s.drag_updated(Some(1), true, 0.0);
        s.finish();
        assert!(!s.tick(60_000.0));
        assert_eq!(*seen.borrow(), 0);
    }

    #[test]
    fn progress_is_exposed_for_the_effect() {
        let mut s = SpringLoadedInteraction::new(Config { delay_ms: 200.0 }, |_| {});
        s.drag_updated(Some(1), true, 0.0);
        assert!((s.progress(100.0) - 0.5).abs() < 1e-6);
    }
}
