//! The phase of one game (playing, won, lost) as a named-state machine.
//! Each phase is a `State` with its own rules for what may come next, and the machine from
//! `bifrost_app::controls::state_machine` enforces them. A game in progress may end; a game that is
//! won may still end later; an ended game can only be restarted; any game may start over.

use bifrost_app::controls::state_machine::{State, StateMachine};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Playing => "playing",
            Phase::Won => "won",
            Phase::Lost => "lost",
        }
    }

    fn from_name(name: &str) -> Option<Phase> {
        match name {
            "playing" => Some(Phase::Playing),
            "won" => Some(Phase::Won),
            "lost" => Some(Phase::Lost),
            _ => None,
        }
    }
}

/// The rules of one phase. The machine asks the current phase before every transition.
struct Rules(Phase);

impl State for Rules {
    fn is_valid_next(&self, next: &str) -> bool {
        match (self.0, next) {
            // a new game may start from any phase
            (_, "playing") => true,
            (Phase::Playing, "won") | (Phase::Playing, "lost") => true,
            // a won game can still be lost (no moves left)
            (Phase::Won, "lost") => true,
            _ => false,
        }
    }
}

/// The phase of one board. It starts in `Playing`.
pub struct Flow {
    fsm: StateMachine,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    pub fn new() -> Self {
        let states: Vec<(&'static str, Box<dyn State>)> = vec![
            ("playing", Box::new(Rules(Phase::Playing))),
            ("won", Box::new(Rules(Phase::Won))),
            ("lost", Box::new(Rules(Phase::Lost))),
        ];
        let mut fsm = StateMachine::new(states);
        fsm.enter(Phase::Playing.name());
        Self { fsm }
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.fsm.current_state().and_then(Phase::from_name).unwrap_or(Phase::Playing)
    }

    /// Moves to `to` if the rules allow it. Returns false and changes nothing otherwise.
    pub fn go(&mut self, to: Phase) -> bool {
        self.fsm.enter(to.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_flow_is_playing() {
        assert_eq!(Flow::new().phase(), Phase::Playing);
    }

    #[test]
    fn a_game_in_progress_can_be_won_or_lost() {
        let mut f = Flow::new();
        assert!(f.go(Phase::Won));
        assert_eq!(f.phase(), Phase::Won);
        let mut f = Flow::new();
        assert!(f.go(Phase::Lost));
        assert_eq!(f.phase(), Phase::Lost);
    }

    #[test]
    fn a_won_game_can_still_be_lost_later() {
        let mut f = Flow::new();
        assert!(f.go(Phase::Won));
        assert!(f.go(Phase::Lost));
        assert_eq!(f.phase(), Phase::Lost);
    }

    #[test]
    fn an_ended_game_cannot_be_won_or_lost_again() {
        let mut f = Flow::new();
        assert!(f.go(Phase::Lost));
        assert!(!f.go(Phase::Won));
        assert!(!f.go(Phase::Lost));
        assert_eq!(f.phase(), Phase::Lost);

        let mut f = Flow::new();
        assert!(f.go(Phase::Won));
        assert!(!f.go(Phase::Won));
        assert_eq!(f.phase(), Phase::Won);
    }

    #[test]
    fn a_new_game_starts_from_any_phase() {
        for end in [Phase::Won, Phase::Lost] {
            let mut f = Flow::new();
            assert!(f.go(end));
            assert!(f.go(Phase::Playing));
            assert_eq!(f.phase(), Phase::Playing);
        }
        let mut f = Flow::new();
        assert!(f.go(Phase::Playing));
        assert_eq!(f.phase(), Phase::Playing);
    }
}
