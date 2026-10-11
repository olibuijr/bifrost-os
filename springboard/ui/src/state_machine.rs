//! Finite state machines for game logic: named states, each with its own rules for what may come next.
//! A machine owns its states and enters one at a time. The current state can refuse a transition,
//! run hooks on entry and exit, and run a per-frame update.

/// The logic of one state. Override only the hooks the state needs.
pub trait State {
    /// Whether the machine may leave this state for `next`. Asked before each transition out of this state.
    fn is_valid_next(&self, _next: &str) -> bool {
        true
    }

    /// Runs after the machine has entered this state. `from` is the state it came from, if any.
    fn did_enter(&mut self, _from: Option<&str>) {}

    /// Runs just before the machine leaves this state for `next`.
    fn will_exit(&mut self, _next: &str) {}

    /// Per-frame work, run by `StateMachine::update` while this state is current.
    fn update(&mut self, _dt: f64) {}
}

/// A machine over named states. Names should be unique; the first state with a given name is used.
pub struct StateMachine {
    states: Vec<(&'static str, Box<dyn State>)>,
    current: Option<usize>,
}

impl StateMachine {
    /// Starts with no current state. The first successful `enter` sets one.
    pub fn new(states: Vec<(&'static str, Box<dyn State>)>) -> Self {
        Self {
            states,
            current: None,
        }
    }

    pub fn current_state(&self) -> Option<&'static str> {
        self.current.map(|i| self.states[i].0)
    }

    /// Whether `enter(name)` would succeed right now. Nothing is changed.
    pub fn can_enter(&self, name: &str) -> bool {
        if self.index_of(name).is_none() {
            return false;
        }
        self.current
            .map_or(true, |from| self.states[from].1.is_valid_next(name))
    }

    /// Moves to the named state if the machine allows it. Returns false and changes nothing otherwise.
    pub fn enter(&mut self, name: &str) -> bool {
        let Some(to) = self.index_of(name) else {
            return false;
        };
        let from = self.current;
        if let Some(f) = from {
            if !self.states[f].1.is_valid_next(name) {
                return false;
            }
            self.states[f].1.will_exit(name);
        }
        let from_name = from.map(|f| self.states[f].0);
        self.current = Some(to);
        self.states[to].1.did_enter(from_name);
        true
    }

    /// Runs the current state's update. Does nothing before the first entry.
    pub fn update(&mut self, dt: f64) {
        if let Some(i) = self.current {
            self.states[i].1.update(dt);
        }
    }

    /// The logic of a named state, for reading or changing its data.
    pub fn state_mut(&mut self, name: &str) -> Option<&mut dyn State> {
        let i = self.index_of(name)?;
        Some(&mut *self.states[i].1)
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|(n, _)| *n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    /// A state that writes its hooks to a shared log and accepts only the names it is given.
    struct Recorder {
        name: &'static str,
        log: Log,
        allowed: Vec<&'static str>,
    }

    impl State for Recorder {
        fn is_valid_next(&self, next: &str) -> bool {
            self.allowed.iter().any(|a| *a == next)
        }

        fn did_enter(&mut self, from: Option<&str>) {
            self.log
                .borrow_mut()
                .push(format!("enter {} from {}", self.name, from.unwrap_or("nothing")));
        }

        fn will_exit(&mut self, next: &str) {
            self.log
                .borrow_mut()
                .push(format!("exit {} to {next}", self.name));
        }

        fn update(&mut self, dt: f64) {
            self.log.borrow_mut().push(format!("tick {} {dt}", self.name));
        }
    }

    fn recorder(name: &'static str, log: &Log, allowed: Vec<&'static str>) -> (&'static str, Box<dyn State>) {
        let boxed: Box<dyn State> = Box::new(Recorder {
            name,
            log: log.clone(),
            allowed,
        });
        (name, boxed)
    }

    fn machine(log: &Log) -> StateMachine {
        StateMachine::new(vec![
            recorder("idle", log, vec!["walk"]),
            recorder("walk", log, vec!["idle", "jump"]),
            recorder("jump", log, vec![]),
        ])
    }

    #[test]
    fn the_first_entry_always_works_and_unknown_names_are_refused() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut m = machine(&log);
        assert_eq!(m.current_state(), None);
        assert!(!m.enter("fly"));
        assert!(m.can_enter("idle"));
        assert!(m.enter("idle"));
        assert_eq!(m.current_state(), Some("idle"));
    }

    #[test]
    fn transitions_follow_the_rules_of_the_current_state() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut m = machine(&log);
        m.enter("idle");
        assert!(!m.can_enter("jump"));
        assert!(!m.enter("jump"));
        assert_eq!(m.current_state(), Some("idle"), "a refused transition changes nothing");
        assert!(m.enter("walk"));
        assert!(m.enter("jump"));
        assert!(!m.can_enter("idle"), "jump allows no exits");
        assert_eq!(m.current_state(), Some("jump"));
    }

    #[test]
    fn exit_and_enter_hooks_run_in_order_and_refusals_run_nothing() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut m = machine(&log);
        m.enter("idle");
        m.enter("walk");
        m.enter("jump");
        assert!(!m.enter("idle"));
        assert_eq!(
            *log.borrow(),
            vec![
                "enter idle from nothing",
                "exit idle to walk",
                "enter walk from idle",
                "exit walk to jump",
                "enter jump from walk",
            ]
        );
    }

    #[test]
    fn update_only_reaches_the_current_state() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut m = machine(&log);
        m.update(0.5);
        assert!(log.borrow().is_empty(), "no state before the first entry");
        m.enter("idle");
        m.update(0.5);
        m.enter("walk");
        m.update(1.0);
        let ticks: Vec<String> = log.borrow().iter().filter(|l| l.starts_with("tick")).cloned().collect();
        assert_eq!(ticks, vec!["tick idle 0.5", "tick walk 1"]);
    }

    #[test]
    fn state_mut_reaches_a_state_by_name_only() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut m = machine(&log);
        assert!(m.state_mut("swim").is_none());
        let walk = m.state_mut("walk").expect("walk exists");
        walk.update(0.25);
        assert_eq!(log.borrow().last().map(String::as_str), Some("tick walk 0.25"));
    }
}
