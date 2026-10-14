//! Spring-loaded interaction (`UISpringLoadedInteraction`): a drag that rests over a target for a while activates it once.
//! The target is any id the app chooses (a button, a folder row, a tab). The behavior decides per target whether it may
//! spring-load (`allowed`), the effect reads [`SpringLoad::progress`] to draw the hover ramp, and the activation is
//! reported once per hover: moving to another target restarts the timer, and leaving the target cancels it.
//!
//! Pure model. The caller feeds the current target and a timestamp in milliseconds on each drag update and on a
//! periodic tick (a drag that does not move sends no update, so the tick is what lets a still pointer activate). Nothing
//! here reads a clock or a device; pointer and keyboard drags both feed the same calls.

/// Delay before a resting drag activates its target, and the effect ramp is the same length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub delay_ms: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { delay_ms: 700.0 }
    }
}

/// Where the interaction is for the current target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No target, or the target does not allow spring loading.
    Idle,
    /// A target is under the drag and its delay runs.
    Hovering,
    /// The delay elapsed; the target was activated for this hover.
    Activated,
}

/// The result of an update that changed the interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The target was spring-loaded now. Reported once per hover.
    Activated(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringLoad {
    config: Config,
    target: Option<u64>,
    since: f64,
    state: State,
}

impl SpringLoad {
    pub fn new(config: Config) -> Self {
        SpringLoad { config, target: None, since: 0.0, state: State::Idle }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The target under the drag, when one is spring-loading or has been activated.
    pub fn target(&self) -> Option<u64> {
        self.target
    }

    /// Feeds the target under the drag at time `t_ms`. `allowed` is the behavior's answer for that target (the
    /// `shouldAllow` decision). Returns [`Event::Activated`] on the update where the delay has run out.
    pub fn update(&mut self, target: Option<u64>, allowed: bool, t_ms: f64) -> Option<Event> {
        let target = target.filter(|_| allowed);
        if target != self.target {
            self.target = target;
            self.since = t_ms;
            self.state = if target.is_some() { State::Hovering } else { State::Idle };
        }
        self.tick(t_ms)
    }

    /// Re-checks the delay at time `t_ms` for the current target. Call it on a timer while a drag is over a target.
    pub fn tick(&mut self, t_ms: f64) -> Option<Event> {
        match (self.state, self.target) {
            (State::Hovering, Some(id)) if t_ms - self.since >= self.config.delay_ms => {
                self.state = State::Activated;
                Some(Event::Activated(id))
            }
            _ => None,
        }
    }

    /// The hover ramp from 0 to 1 over the delay, for the effect to draw. 1 once activated, 0 when idle.
    pub fn progress(&self, t_ms: f64) -> f32 {
        match self.state {
            State::Idle => 0.0,
            State::Activated => 1.0,
            State::Hovering => {
                if self.config.delay_ms <= 0.0 {
                    return 1.0;
                }
                (((t_ms - self.since) / self.config.delay_ms) as f32).clamp(0.0, 1.0)
            }
        }
    }

    /// The drag ended or was cancelled: the interaction returns to idle.
    pub fn finish(&mut self) {
        self.target = None;
        self.state = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> SpringLoad {
        SpringLoad::new(Config { delay_ms: 500.0 })
    }

    #[test]
    fn default_delay_is_positive() {
        assert!(Config::default().delay_ms > 0.0);
    }

    #[test]
    fn activates_only_after_the_delay() {
        let mut s = model();
        assert_eq!(s.update(Some(7), true, 0.0), None);
        assert_eq!(s.state(), State::Hovering);
        assert_eq!(s.tick(499.0), None);
        assert_eq!(s.tick(500.0), Some(Event::Activated(7)));
        assert_eq!(s.state(), State::Activated);
    }

    #[test]
    fn activation_is_reported_once_per_hover() {
        let mut s = model();
        s.update(Some(7), true, 0.0);
        s.tick(600.0);
        assert_eq!(s.update(Some(7), true, 900.0), None);
        assert_eq!(s.tick(2000.0), None);
    }

    #[test]
    fn moving_to_another_target_restarts_the_timer() {
        let mut s = model();
        s.update(Some(1), true, 0.0);
        assert_eq!(s.update(Some(2), true, 400.0), None);
        assert_eq!(s.target(), Some(2));
        assert_eq!(s.tick(800.0), None, "the new target has only had 400 ms");
        assert_eq!(s.tick(900.0), Some(Event::Activated(2)));
    }

    #[test]
    fn leaving_the_target_cancels_the_hover() {
        let mut s = model();
        s.update(Some(1), true, 0.0);
        assert_eq!(s.update(None, true, 300.0), None);
        assert_eq!(s.state(), State::Idle);
        assert_eq!(s.tick(1000.0), None);
    }

    #[test]
    fn a_target_that_refuses_spring_loading_never_activates() {
        let mut s = model();
        assert_eq!(s.update(Some(9), false, 0.0), None);
        assert_eq!(s.state(), State::Idle);
        assert_eq!(s.update(Some(9), false, 5000.0), None);
        assert_eq!(s.target(), None);
    }

    #[test]
    fn progress_ramps_across_the_delay() {
        let mut s = model();
        s.update(Some(1), true, 1000.0);
        assert_eq!(s.progress(1000.0), 0.0);
        assert!((s.progress(1250.0) - 0.5).abs() < 1e-6);
        assert_eq!(s.progress(9000.0), 1.0);
        s.tick(1500.0);
        assert_eq!(s.progress(1500.0), 1.0);
        s.finish();
        assert_eq!(s.progress(2000.0), 0.0);
    }

    #[test]
    fn zero_delay_activates_on_the_first_update() {
        let mut s = SpringLoad::new(Config { delay_ms: 0.0 });
        assert_eq!(s.update(Some(3), true, 10.0), Some(Event::Activated(3)));
    }

    #[test]
    fn finish_drops_the_target() {
        let mut s = model();
        s.update(Some(1), true, 0.0);
        s.finish();
        assert_eq!(s.target(), None);
        assert_eq!(s.tick(10_000.0), None);
    }
}
