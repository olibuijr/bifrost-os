//! Game Mode: while a game is in front and full screen, the machine gives it the whole CPU.
//!
//! Everything here is pure. The shell feeds `Model::update` the current answer ("is a game in front, full
//! screen, and is Game Mode allowed?") and a clock in milliseconds, and applies the `Effect`s that `plan` lists.
//! Entering takes effect at once; leaving waits `LEAVE_MS` so a briefly unfocused or windowed game does not flap.
//!
//! Effects: the CPU profile goes to Performance (`bare::boost::set_profile`, which writes the governor and clock
//! window), the game's nice and uclamp are raised through bifrostd (`game_priority`), and the background task
//! scheduler does not start new tasks while Game Mode is on (`services::background`). Not built, recorded in
//! `SKIPPED`: the file indexing pause (no indexer runs in the shell) and the Bluetooth sniff-interval change (the
//! shell's mgmt socket has no connection-update opcode).

/// How long the game must be out of front, or out of full screen, before Game Mode leaves.
pub const LEAVE_MS: u64 = 3_000;

/// Things Game Mode would do that this build cannot do yet. Read by the Game Mode page and the docs.
pub const SKIPPED: [&str; 2] = [
    "file indexing pause: no indexer runs in the shell",
    "Bluetooth sniff interval: the shell's mgmt socket has no connection parameter update",
];

/// Settings > Game Mode choice, stored as `game_mode` in settings.ini.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GamePref {
    #[default]
    Automatic,
    Off,
}

impl GamePref {
    pub fn key(self) -> &'static str {
        match self {
            GamePref::Automatic => "automatic",
            GamePref::Off => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GamePref::Automatic => "Automatic",
            GamePref::Off => "Off",
        }
    }

    pub fn parse(s: &str) -> Option<GamePref> {
        [GamePref::Automatic, GamePref::Off].into_iter().find(|p| p.key() == s)
    }
}

/// Whether an installed app counts as a game: its manifest says `game = true`, or it is in the Games category.
pub fn is_game(category: &str, game_flag: bool) -> bool {
    game_flag || category.trim().eq_ignore_ascii_case("games")
}

/// A change of state, which `plan` turns into effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Enter,
    Leave,
}

/// One thing to do on a change. Applied by the shell; `plan` is the only place that decides the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// CPU profile Performance: governor `performance`, clocks held at the top
    CpuPerformance,
    /// the CPU profile the user's Power Mode choice picks again
    CpuFollowPowerMode,
    /// nice and uclamp raised for the game in front (`game_priority`, through bifrostd)
    RaisePriority,
    /// the game's nice and uclamp put back
    RestorePriority,
}

/// The effects for a change of state, in the order they apply.
pub fn plan(change: Change) -> Vec<Effect> {
    match change {
        Change::Enter => vec![Effect::CpuPerformance, Effect::RaisePriority],
        Change::Leave => vec![Effect::CpuFollowPowerMode, Effect::RestorePriority],
    }
}

/// The Game Mode state machine with leave hysteresis.
#[derive(Debug, Default)]
pub struct Model {
    on: bool,
    /// the player turned Game Mode off for the game in front: it stays off until that game leaves the front
    suppressed: bool,
    /// when the game first stopped qualifying, while Game Mode is still on
    leaving_since: Option<u64>,
}

impl Model {
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// `wanted`: Game Mode is allowed and a game is in front, full screen. Returns the change, if any.
    pub fn update(&mut self, wanted: bool, now: u64) -> Option<Change> {
        if !wanted {
            self.suppressed = false;
        }
        let wanted = wanted && !self.suppressed;
        if wanted {
            self.leaving_since = None;
            if !self.on {
                self.on = true;
                return Some(Change::Enter);
            }
            return None;
        }
        if !self.on {
            return None;
        }
        match self.leaving_since {
            None => {
                self.leaving_since = Some(now);
                None
            }
            Some(since) if now.saturating_sub(since) >= LEAVE_MS => {
                self.on = false;
                self.leaving_since = None;
                Some(Change::Leave)
            }
            Some(_) => None,
        }
    }

    /// The player turns Game Mode off from the menu bar: it is off until the game in front leaves. Returns whether
    /// it was on.
    pub fn turn_off(&mut self) -> bool {
        let was_on = self.on;
        self.on = false;
        self.leaving_since = None;
        self.suppressed = true;
        was_on
    }

    /// When `update` next has to run to let Game Mode leave, if it is waiting to.
    pub fn leave_deadline(&self) -> Option<u64> {
        self.leaving_since.map(|since| since + LEAVE_MS)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enters_at_once_and_reports_the_change_once() {
        let mut m = Model::default();
        assert_eq!(m.update(true, 0), Some(Change::Enter));
        assert!(m.is_on());
        assert_eq!(m.update(true, 100), None);
    }

    #[test]
    fn leaves_only_after_the_hysteresis_passes() {
        let mut m = Model::default();
        m.update(true, 0);
        assert_eq!(m.update(false, 1_000), None, "the first miss only starts the timer");
        assert_eq!(m.leave_deadline(), Some(1_000 + LEAVE_MS));
        assert_eq!(m.update(false, 1_000 + LEAVE_MS - 1), None);
        assert_eq!(m.update(false, 1_000 + LEAVE_MS), Some(Change::Leave));
        assert!(!m.is_on());
        assert_eq!(m.leave_deadline(), None);
    }

    #[test]
    fn a_game_that_comes_back_inside_the_window_stays_on() {
        let mut m = Model::default();
        m.update(true, 0);
        m.update(false, 500);
        assert_eq!(m.update(true, 2_000), None);
        // the timer restarts from the next miss, not from the first one
        assert_eq!(m.update(false, 10_000), None);
        assert_eq!(m.update(false, 10_000 + LEAVE_MS - 1), None);
        assert_eq!(m.update(false, 10_000 + LEAVE_MS), Some(Change::Leave));
    }

    #[test]
    fn nothing_happens_while_off() {
        let mut m = Model::default();
        assert_eq!(m.update(false, 0), None);
        assert_eq!(m.update(false, 10 * LEAVE_MS), None);
        assert!(!m.is_on());
    }

    #[test]
    fn turning_it_off_holds_until_the_game_leaves() {
        let mut m = Model::default();
        m.update(true, 0);
        assert!(m.turn_off());
        assert!(!m.is_on());
        assert_eq!(m.update(true, 100), None, "the same game does not turn it back on");
        assert_eq!(m.update(false, 200), None, "the game leaves");
        assert_eq!(m.update(true, 300), Some(Change::Enter), "the next game gets it again");
        assert!(!Model::default().turn_off(), "off already: nothing to turn off");
    }

    #[test]
    fn effects_follow_the_change() {
        assert_eq!(plan(Change::Enter), vec![Effect::CpuPerformance, Effect::RaisePriority]);
        assert_eq!(plan(Change::Leave), vec![Effect::CpuFollowPowerMode, Effect::RestorePriority]);
        assert_eq!(SKIPPED.len(), 2, "every unbuilt effect is named");
    }

    #[test]
    fn games_are_the_games_category_or_a_game_flag() {
        assert!(is_game("Games", false));
        assert!(is_game(" games ", false));
        assert!(is_game("Utilities", true));
        assert!(!is_game("Utilities", false));
        assert!(!is_game("", false));
    }

    #[test]
    fn the_preference_round_trips_and_rejects_unknown_values() {
        for p in [GamePref::Automatic, GamePref::Off] {
            assert_eq!(GamePref::parse(p.key()), Some(p));
        }
        assert_eq!(GamePref::parse("on"), None);
        assert_eq!(GamePref::default(), GamePref::Automatic);
    }
}
