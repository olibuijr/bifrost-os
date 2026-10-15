//! Tooltip timing. A pointer that rests on a control for the delay shows that control's tooltip. Once a tooltip is
//! showing, moving onto a neighbouring control shows its tooltip at once, and a control entered within the grace
//! period after a tooltip closed shows at once too. Time is passed in as seconds, so the state machine is
//! deterministic and needs no clock. The view layer decides which owner is under the pointer and draws the text.

/// How long a resting pointer waits before the tooltip shows, and how long a closed tooltip still lets neighbours
/// show at once. Both are seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub delay: f32,
    pub grace: f32,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { delay: 1.0, grace: 0.3 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    Waiting { owner: u32, since: f32 },
    Showing { owner: u32 },
}

/// The tooltip state of one window. Owners are the view ids of the controls that carry a tooltip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tooltips {
    timing: Timing,
    phase: Phase,
    grace_until: Option<f32>,
}

impl Tooltips {
    pub fn new(timing: Timing) -> Self {
        Tooltips { timing, phase: Phase::Idle, grace_until: None }
    }

    /// The pointer entered `owner` at time `now`. The tooltip shows at once when another tooltip is open or a
    /// tooltip closed within the grace period; otherwise it waits for the delay.
    pub fn enter(&mut self, owner: u32, now: f32) {
        let open = matches!(self.phase, Phase::Showing { .. });
        let within_grace = self.grace_until.is_some_and(|until| now <= until);
        self.phase = if open || within_grace {
            Phase::Showing { owner }
        } else {
            Phase::Waiting { owner, since: now }
        };
    }

    /// The pointer left `owner` at time `now`. Leaving a control that is not the current one changes nothing.
    pub fn leave(&mut self, owner: u32, now: f32) {
        match self.phase {
            Phase::Waiting { owner: current, .. } if current == owner => self.phase = Phase::Idle,
            Phase::Showing { owner: current } if current == owner => {
                self.phase = Phase::Idle;
                self.grace_until = Some(now + self.timing.grace);
            }
            _ => {}
        }
    }

    /// Advances the timer to `now` and returns the owner whose tooltip is showing, if any.
    pub fn tick(&mut self, now: f32) -> Option<u32> {
        if let Phase::Waiting { owner, since } = self.phase {
            if now - since >= self.timing.delay {
                self.phase = Phase::Showing { owner };
            }
        }
        self.visible()
    }

    /// The owner whose tooltip is showing right now.
    pub fn visible(&self) -> Option<u32> {
        match self.phase {
            Phase::Showing { owner } => Some(owner),
            Phase::Idle | Phase::Waiting { .. } => None,
        }
    }
}
