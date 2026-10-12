//! Next-item proposals as pure state: the offer to play the item after the current one. The offer appears once the
//! content time reaches the transition time. It is accepted when the viewer chooses it, or automatically once the
//! acceptance interval has passed since playback ended. The caller draws the offer and feeds in the clock; nothing
//! here sleeps or draws.

/// The proposed item and when it is offered.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// Content time, in seconds, at which the offer appears.
    pub transition_at: f64,
    pub title: String,
    pub url: Option<String>,
    /// Seconds between the end of playback and automatic acceptance.
    pub automatic_acceptance_interval: f64,
}

/// Where an offer is in its life. `Accepted` and `Rejected` are final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Offered,
    Accepted,
    Rejected,
}

/// One proposal being played out against a player.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    proposal: Proposal,
    phase: Phase,
    ended_at: Option<f64>,
}

impl Session {
    /// A session for `proposal`. `None` when the transition time or the interval is not a finite non-negative
    /// number of seconds.
    pub fn new(proposal: Proposal) -> Option<Session> {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        if !ok(proposal.transition_at) || !ok(proposal.automatic_acceptance_interval) {
            return None;
        }
        Some(Session { proposal, phase: Phase::Waiting, ended_at: None })
    }

    pub fn proposal(&self) -> &Proposal {
        &self.proposal
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The content time moved to `content_time` seconds. Returns true on the call that makes the offer appear.
    pub fn observe(&mut self, content_time: f64) -> bool {
        if self.phase == Phase::Waiting && content_time.is_finite() && content_time >= self.proposal.transition_at {
            self.phase = Phase::Offered;
            return true;
        }
        false
    }

    /// Playback of the current item ended at `now` seconds on the caller's clock. An offer that has not appeared yet
    /// is shown now. Only the first end counts.
    pub fn playback_ended(&mut self, now: f64) {
        if !now.is_finite() || self.ended_at.is_some() {
            return;
        }
        self.ended_at = Some(now);
        if self.phase == Phase::Waiting {
            self.phase = Phase::Offered;
        }
    }

    /// Seconds left until automatic acceptance, while an offer is showing after playback ended.
    pub fn remaining(&self, now: f64) -> Option<f64> {
        match (self.phase, self.ended_at) {
            (Phase::Offered, Some(ended)) => {
                Some((ended + self.proposal.automatic_acceptance_interval - now).max(0.0))
            }
            _ => None,
        }
    }

    /// Advances the clock to `now`. Accepts the offer when it is showing, playback has ended and the interval has
    /// passed. Returns true on the call that accepts it.
    pub fn tick(&mut self, now: f64) -> bool {
        if self.phase != Phase::Offered {
            return false;
        }
        match self.remaining(now) {
            Some(left) if left <= 0.0 => {
                self.phase = Phase::Accepted;
                true
            }
            _ => false,
        }
    }

    /// The viewer takes (`true`) or declines (`false`) the offer. Only a showing offer can be answered.
    pub fn choose(&mut self, accept: bool) -> bool {
        if self.phase != Phase::Offered {
            return false;
        }
        self.phase = if accept { Phase::Accepted } else { Phase::Rejected };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(transition_at: f64, interval: f64) -> Proposal {
        Proposal {
            transition_at,
            title: String::from("Next episode"),
            url: Some(String::from("https://example.test/next")),
            automatic_acceptance_interval: interval,
        }
    }

    #[test]
    fn rejects_values_that_are_not_seconds() {
        assert!(Session::new(proposal(f64::NAN, 10.0)).is_none());
        assert!(Session::new(proposal(30.0, -1.0)).is_none());
        assert!(Session::new(proposal(-5.0, 10.0)).is_none());
        let s = Session::new(proposal(30.0, 10.0)).expect("valid");
        assert_eq!(s.phase(), Phase::Waiting);
        assert_eq!(s.proposal().title, "Next episode");
    }

    #[test]
    fn offer_appears_at_the_transition_time_once() {
        let mut s = Session::new(proposal(30.0, 10.0)).expect("valid");
        assert!(!s.observe(29.9));
        assert_eq!(s.phase(), Phase::Waiting);
        assert!(s.observe(30.0));
        assert_eq!(s.phase(), Phase::Offered);
        assert!(!s.observe(31.0), "it appears only once");
    }

    #[test]
    fn playback_ending_first_shows_the_offer_and_counts_down() {
        let mut s = Session::new(proposal(30.0, 10.0)).expect("valid");
        s.playback_ended(100.0);
        assert_eq!(s.phase(), Phase::Offered);
        assert_eq!(s.remaining(104.0), Some(6.0));
        assert_eq!(s.remaining(150.0), Some(0.0), "never negative");
        s.playback_ended(120.0);
        assert_eq!(s.remaining(104.0), Some(6.0), "only the first end counts");
    }

    #[test]
    fn automatic_acceptance_after_the_interval() {
        let mut s = Session::new(proposal(30.0, 10.0)).expect("valid");
        assert!(s.observe(30.0));
        s.playback_ended(100.0);
        assert!(!s.tick(109.9));
        assert_eq!(s.phase(), Phase::Offered);
        assert!(s.tick(110.0));
        assert_eq!(s.phase(), Phase::Accepted);
        assert!(!s.tick(200.0), "final once accepted");
    }

    #[test]
    fn viewer_choice_is_final_and_needs_a_showing_offer() {
        let mut s = Session::new(proposal(30.0, 10.0)).expect("valid");
        assert!(!s.choose(true), "no offer yet");
        assert_eq!(s.phase(), Phase::Waiting);
        assert!(s.observe(30.0));
        assert!(s.choose(false));
        assert_eq!(s.phase(), Phase::Rejected);
        assert!(!s.choose(true));
        s.playback_ended(100.0);
        assert!(!s.tick(200.0), "a declined offer is not accepted later");
    }
}
