//! Composable node actions and keyframe sequences as a pure model. An action is a tree: waits, reports to the caller,
//! sequences that run one child after another, groups that run children at once and finish with the longest, and
//! repeats, either a counted number of times or forever.
//!
//! The runner holds the running actions and advances them by a time step in seconds. It reads no clock and runs no
//! code of its own: a step returns the report ids that fired, in order, and the caller reacts to them. A repeat that
//! never ends stays active until it is removed, except when its body takes no time, in which case it finishes after a
//! single pass so that a step cannot spin.
//!
//! Keyframes map a time to a value by linear interpolation or by holding the previous key.

/// One node action.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Does nothing for the given number of seconds. Negative values count as zero.
    Wait(f64),
    /// Reports the id to the caller at this point in the action. Takes no time.
    Report(u32),
    /// Runs each child after the previous one has finished.
    Sequence(Vec<Action>),
    /// Runs all children at once and finishes when the last of them finishes.
    Group(Vec<Action>),
    /// Runs the body the given number of times. Zero times finishes at once.
    Repeat(Box<Action>, u32),
    /// Runs the body again and again until the action is removed.
    RepeatForever(Box<Action>),
}

impl Action {
    /// The time the action takes in seconds. Infinite for a repeat that never ends, unless its body takes no time.
    pub fn duration(&self) -> f64 {
        match self {
            Action::Wait(d) => d.max(0.0),
            Action::Report(_) => 0.0,
            Action::Sequence(items) => items.iter().map(Action::duration).sum(),
            Action::Group(items) => items.iter().map(Action::duration).fold(0.0, f64::max),
            Action::Repeat(body, count) => {
                if *count == 0 {
                    0.0
                } else {
                    body.duration() * f64::from(*count)
                }
            }
            Action::RepeatForever(body) => {
                if body.duration() > 0.0 {
                    f64::INFINITY
                } else {
                    0.0
                }
            }
        }
    }
}

/// The running state of one action.
#[derive(Clone, Debug)]
enum Running {
    Wait {
        left: f64,
    },
    Report(u32),
    Sequence {
        items: Vec<Action>,
        next: usize,
        current: Option<Box<Running>>,
    },
    Group {
        children: Vec<Running>,
    },
    Repeat {
        body: Action,
        left: u32,
        current: Box<Running>,
    },
    Forever {
        body: Action,
        current: Box<Running>,
    },
}

fn start(action: &Action) -> Running {
    match action {
        Action::Wait(d) => Running::Wait { left: d.max(0.0) },
        Action::Report(id) => Running::Report(*id),
        Action::Sequence(items) => Running::Sequence {
            items: items.clone(),
            next: 0,
            current: None,
        },
        Action::Group(items) => Running::Group {
            children: items.iter().map(start).collect(),
        },
        Action::Repeat(body, count) => {
            if *count == 0 {
                Running::Wait { left: 0.0 }
            } else {
                Running::Repeat {
                    current: Box::new(start(body)),
                    body: (**body).clone(),
                    left: *count,
                }
            }
        }
        Action::RepeatForever(body) => Running::Forever {
            current: Box::new(start(body)),
            body: (**body).clone(),
        },
    }
}

impl Running {
    /// Advances by `dt` seconds. Returns `None` while the action is still running, or `Some(leftover)` when it
    /// finishes, where `leftover` is the part of `dt` it did not use.
    fn advance(&mut self, dt: f64, events: &mut Vec<u32>) -> Option<f64> {
        match self {
            Running::Wait { left } => {
                if dt < *left {
                    *left -= dt;
                    None
                } else {
                    Some(dt - *left)
                }
            }
            Running::Report(id) => {
                events.push(*id);
                Some(dt)
            }
            Running::Sequence {
                items,
                next,
                current,
            } => {
                let mut dt = dt;
                loop {
                    if current.is_none() {
                        if *next >= items.len() {
                            return Some(dt);
                        }
                        *current = Some(Box::new(start(&items[*next])));
                    }
                    let finished = match current.as_mut() {
                        Some(child) => child.advance(dt, events),
                        None => None,
                    };
                    match finished {
                        None => return None,
                        Some(leftover) => {
                            *current = None;
                            *next += 1;
                            dt = leftover;
                        }
                    }
                }
            }
            Running::Group { children } => {
                if children.is_empty() {
                    return Some(dt);
                }
                let mut still = Vec::with_capacity(children.len());
                let mut best: Option<f64> = None;
                for mut child in children.drain(..) {
                    match child.advance(dt, events) {
                        None => still.push(child),
                        // The last child to finish ends the group, and it has the smallest leftover.
                        Some(leftover) => best = Some(best.map_or(leftover, |b| b.min(leftover))),
                    }
                }
                if still.is_empty() {
                    return Some(best.unwrap_or(dt));
                }
                *children = still;
                None
            }
            Running::Repeat {
                body,
                left,
                current,
            } => {
                let mut dt = dt;
                loop {
                    match current.advance(dt, events) {
                        None => return None,
                        Some(leftover) => {
                            if *left <= 1 {
                                return Some(leftover);
                            }
                            *left -= 1;
                            *current = Box::new(start(body));
                            dt = leftover;
                        }
                    }
                }
            }
            Running::Forever { body, current } => {
                let mut dt = dt;
                loop {
                    match current.advance(dt, events) {
                        None => return None,
                        Some(leftover) => {
                            if body.duration() <= 0.0 {
                                return Some(leftover);
                            }
                            *current = Box::new(start(body));
                            dt = leftover;
                        }
                    }
                }
            }
        }
    }
}

/// A handle to an action held by a [`Runner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(u32);

/// Holds running actions and advances them together.
#[derive(Debug, Default)]
pub struct Runner {
    next: u32,
    active: Vec<(Handle, Running)>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an action. It takes effect from the next [`Runner::advance`].
    pub fn run(&mut self, action: &Action) -> Handle {
        self.next = self.next.wrapping_add(1);
        let handle = Handle(self.next);
        self.active.push((handle, start(action)));
        handle
    }

    /// Removes an action before it finishes. Returns whether it was still running.
    pub fn remove(&mut self, handle: Handle) -> bool {
        let before = self.active.len();
        self.active.retain(|(h, _)| *h != handle);
        self.active.len() != before
    }

    /// Removes every running action.
    pub fn remove_all(&mut self) {
        self.active.clear();
    }

    pub fn is_active(&self, handle: Handle) -> bool {
        self.active.iter().any(|(h, _)| *h == handle)
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Advances every running action by `dt` seconds and returns the report ids that fired, in order. Negative and
    /// non-finite steps count as zero.
    pub fn advance(&mut self, dt: f64) -> Vec<u32> {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        let mut events = Vec::new();
        self.active
            .retain_mut(|(_, running)| running.advance(dt, &mut events).is_none());
        events
    }
}

/// How a keyframe sequence fills the time between two keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Interpolates linearly between the two keys around the time.
    Linear,
    /// Holds the value of the earlier key until the next one.
    Hold,
}

/// Values at increasing times. Times outside the keys take the value of the nearest key.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    keys: Vec<(f64, f64)>,
    interpolation: Interpolation,
}

impl Keyframes {
    /// Builds a sequence from `(time, value)` pairs. Returns `None` when there are no keys, a time or value is not
    /// finite, or the times are not strictly increasing.
    pub fn new(interpolation: Interpolation, keys: Vec<(f64, f64)>) -> Option<Self> {
        if keys.is_empty() {
            return None;
        }
        let finite = keys.iter().all(|(t, v)| t.is_finite() && v.is_finite());
        let increasing = keys.windows(2).all(|w| w[0].0 < w[1].0);
        if !finite || !increasing {
            return None;
        }
        Some(Keyframes {
            keys,
            interpolation,
        })
    }

    /// The value at time `t`.
    pub fn value_at(&self, t: f64) -> f64 {
        let first = self.keys[0];
        let last = self.keys[self.keys.len() - 1];
        if t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        // Index of the first key after `t`; the key before it is the segment start.
        let after = self.keys.partition_point(|k| k.0 <= t);
        let (t0, v0) = self.keys[after - 1];
        let (t1, v1) = self.keys[after];
        match self.interpolation {
            Interpolation::Hold => v0,
            Interpolation::Linear => {
                let frac = (t - t0) / (t1 - t0);
                v0 + (v1 - v0) * frac
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn wait(d: f64) -> Action {
        Action::Wait(d)
    }

    #[test]
    fn durations_compose() {
        let a = Action::Sequence(vec![
            wait(1.0),
            Action::Group(vec![wait(2.0), wait(3.0)]),
            Action::Repeat(Box::new(wait(0.5)), 4),
        ]);
        assert!(near(a.duration(), 6.0));
        assert!(near(Action::Repeat(Box::new(wait(1.0)), 0).duration(), 0.0));
        assert!(near(Action::Wait(-2.0).duration(), 0.0));
        assert!(Action::RepeatForever(Box::new(wait(0.1)))
            .duration()
            .is_infinite());
        assert!(near(
            Action::RepeatForever(Box::new(Action::Report(1))).duration(),
            0.0
        ));
    }

    #[test]
    fn sequence_reports_in_order() {
        let mut runner = Runner::new();
        let h = runner.run(&Action::Sequence(vec![
            Action::Report(1),
            wait(0.5),
            Action::Report(2),
        ]));
        assert_eq!(runner.advance(0.2), vec![1]);
        assert!(runner.is_active(h));
        assert_eq!(runner.advance(0.4), vec![2]);
        assert!(!runner.is_active(h));
        assert_eq!(runner.active_count(), 0);
    }

    #[test]
    fn group_finishes_with_longest_child() {
        let mut runner = Runner::new();
        runner.run(&Action::Group(vec![
            Action::Sequence(vec![wait(0.2), Action::Report(1)]),
            wait(0.5),
            Action::Report(2),
        ]));
        assert_eq!(runner.advance(0.3), vec![1, 2]);
        assert_eq!(runner.active_count(), 1);
        assert_eq!(runner.advance(0.3), Vec::<u32>::new());
        assert_eq!(runner.active_count(), 0);
    }

    #[test]
    fn counted_repeat_finishes_after_last_pass() {
        let mut runner = Runner::new();
        let h = runner.run(&Action::Repeat(Box::new(wait(0.25)), 3));
        runner.advance(0.5);
        assert!(runner.is_active(h));
        runner.advance(0.5);
        assert!(!runner.is_active(h));
    }

    #[test]
    fn forever_repeat_reports_each_pass() {
        let mut runner = Runner::new();
        let h = runner.run(&Action::RepeatForever(Box::new(Action::Sequence(vec![
            Action::Report(5),
            wait(0.1),
        ]))));
        assert_eq!(runner.advance(0.35), vec![5, 5, 5, 5]);
        assert!(runner.is_active(h));
        assert!(runner.remove(h));
        assert_eq!(runner.active_count(), 0);
    }

    #[test]
    fn forever_repeat_with_no_time_finishes_once() {
        let mut runner = Runner::new();
        runner.run(&Action::RepeatForever(Box::new(Action::Report(9))));
        assert_eq!(runner.advance(0.1), vec![9]);
        assert_eq!(runner.active_count(), 0);
    }

    #[test]
    fn invalid_steps_count_as_zero() {
        let mut runner = Runner::new();
        runner.run(&wait(0.1));
        assert_eq!(runner.advance(f64::NAN), Vec::<u32>::new());
        assert_eq!(runner.advance(-1.0), Vec::<u32>::new());
        assert_eq!(runner.active_count(), 1);
        runner.remove_all();
        assert_eq!(runner.active_count(), 0);
    }

    #[test]
    fn keyframes_linear_and_hold() {
        let lin = Keyframes::new(Interpolation::Linear, vec![(0.0, 0.0), (1.0, 10.0)]).unwrap();
        assert!(near(lin.value_at(0.5), 5.0));
        assert!(near(lin.value_at(-1.0), 0.0));
        assert!(near(lin.value_at(2.0), 10.0));

        let hold = Keyframes::new(
            Interpolation::Hold,
            vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)],
        )
        .unwrap();
        assert!(near(hold.value_at(0.5), 1.0));
        assert!(near(hold.value_at(1.0), 2.0));
        assert!(near(hold.value_at(1.9), 2.0));
    }

    #[test]
    fn keyframes_reject_bad_input() {
        assert!(Keyframes::new(Interpolation::Linear, vec![]).is_none());
        assert!(Keyframes::new(Interpolation::Linear, vec![(1.0, 0.0), (1.0, 2.0)]).is_none());
        assert!(Keyframes::new(Interpolation::Hold, vec![(0.0, f64::NAN)]).is_none());
    }
}
