//! Game Mode: while a game is in front and full screen, the machine gives it the whole CPU.
//!
//! Everything here is pure. The shell feeds `Model::update` the current answer ("is a game in front, full
//! screen, and is Game Mode allowed?") and a clock in milliseconds, and applies the `Effect`s that `plan` lists.
//! Entering takes effect at once; leaving waits `LEAVE_MS` so a briefly unfocused or windowed game does not flap.
//!
//! Effects that exist today: the CPU profile goes to Performance (`bare::boost::set_profile`, which already writes
//! the governor and clock window). Not built yet, and recorded in `SKIPPED`: the background task and file indexing
//! pause (no such scheduler exists in the shell), raising the game's scheduling priority (no privileged request in
//! bifrostd), and the Bluetooth sniff-interval change (the shell's mgmt socket has no connection-update opcode).

/// How long the game must be out of front, or out of full screen, before Game Mode leaves.
pub const LEAVE_MS: u64 = 3_000;

/// Things Game Mode would do that this build cannot do yet. Read by the Game Mode page and the docs.
pub const SKIPPED: [&str; 3] = [
    "background task and file indexing pause: the shell has no background scheduler to pause",
    "game process priority: needs a privileged request, bifrostd has no knob for it yet",
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
}

/// The effects for a change of state, in the order they apply.
pub fn plan(change: Change) -> Vec<Effect> {
    match change {
        Change::Enter => vec![Effect::CpuPerformance],
        Change::Leave => vec![Effect::CpuFollowPowerMode],
    }
}

/// The Game Mode state machine with leave hysteresis.
#[derive(Debug, Default)]
pub struct Model {
    on: bool,
    /// when the game first stopped qualifying, while Game Mode is still on
    leaving_since: Option<u64>,
    notified: bool,
}

impl Model {
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// `wanted`: Game Mode is allowed and a game is in front, full screen. Returns the change, if any.
    pub fn update(&mut self, wanted: bool, now: u64) -> Option<Change> {
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

    /// When `update` next has to run to let Game Mode leave, if it is waiting to.
    pub fn leave_deadline(&self) -> Option<u64> {
        self.leaving_since.map(|since| since + LEAVE_MS)
    }

    /// True the first time Game Mode turns on in this session, so the shell can post its one notification.
    pub fn take_first_notice(&mut self) -> bool {
        let first = !self.notified;
        self.notified = true;
        first
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
    fn the_first_notice_comes_once() {
        let mut m = Model::default();
        assert!(m.take_first_notice());
        assert!(!m.take_first_notice());
    }

    #[test]
    fn effects_follow_the_change() {
        assert_eq!(plan(Change::Enter), vec![Effect::CpuPerformance]);
        assert_eq!(plan(Change::Leave), vec![Effect::CpuFollowPowerMode]);
        assert_eq!(SKIPPED.len(), 3, "every unbuilt effect is named");
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
