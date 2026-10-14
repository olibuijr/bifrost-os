//! The stage strip: a model of the recent app groups that a desktop keeps at one edge while a window is in front.
//!
//! Each stage is one app with the windows it has open, most recent first. Focusing a window moves its app's stage to
//! the front and makes that window the stage's front window. The strip keeps at most `MAX_STAGES` stages; the stages
//! pushed past that limit are returned to the caller, which minimizes their windows. Closing the last window of a stage
//! removes the stage. The model is pure: it holds ids, not windows, and the shell draws it and moves the windows.

/// Most stages the strip keeps on screen at once. A choice of this shell, not a value taken from another system.
pub const MAX_STAGES: usize = 5;

/// One app group in the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub app: u32,
    /// Window ids of this app, the front window first.
    pub windows: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageStrip {
    /// The most recent stage first.
    stages: Vec<Stage>,
}

impl StageStrip {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stages, the most recent first.
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// The stage that holds `window`, if any.
    pub fn stage_of(&self, window: u32) -> Option<&Stage> {
        self.stages.iter().find(|s| s.windows.contains(&window))
    }

    /// The app of the front stage, if the strip is not empty.
    pub fn front_app(&self) -> Option<u32> {
        self.stages.first().map(|s| s.app)
    }

    /// `window` of `app` came to the front. Its stage moves to the front and the window becomes that stage's front
    /// window. Returns the windows of the stages pushed past `MAX_STAGES`, so the caller can minimize them.
    pub fn focus(&mut self, app: u32, window: u32) -> Vec<u32> {
        // a window belongs to one stage: take it out of wherever it was
        self.remove_window(window);
        match self.stages.iter().position(|s| s.app == app) {
            Some(i) => {
                let mut stage = self.stages.remove(i);
                stage.windows.insert(0, window);
                self.stages.insert(0, stage);
            }
            None => self.stages.insert(0, Stage { app, windows: vec![window] }),
        }
        let mut evicted = Vec::new();
        while self.stages.len() > MAX_STAGES {
            if let Some(old) = self.stages.pop() {
                evicted.extend(old.windows);
            }
        }
        evicted
    }

    /// `window` closed or left the desktop. An emptied stage is removed. Returns true if the window was in the strip.
    pub fn remove_window(&mut self, window: u32) -> bool {
        let mut found = false;
        for stage in &mut self.stages {
            if let Some(i) = stage.windows.iter().position(|&w| w == window) {
                stage.windows.remove(i);
                found = true;
                break;
            }
        }
        self.stages.retain(|s| !s.windows.is_empty());
        found
    }

    /// Every window of `app` left the desktop (the app quit). Returns how many windows were removed.
    pub fn remove_app(&mut self, app: u32) -> usize {
        let before: usize = self.stages.iter().map(|s| s.windows.len()).sum();
        self.stages.retain(|s| s.app != app);
        before - self.stages.iter().map(|s| s.windows.len()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_groups_windows_of_one_app_into_one_stage() {
        let mut strip = StageStrip::new();
        assert!(strip.focus(1, 10).is_empty());
        assert!(strip.focus(2, 20).is_empty());
        assert!(strip.focus(1, 11).is_empty());
        assert_eq!(strip.stages().len(), 2);
        assert_eq!(strip.stages()[0], Stage { app: 1, windows: vec![11, 10] });
        assert_eq!(strip.front_app(), Some(1));
    }

    #[test]
    fn focusing_an_older_stage_brings_it_to_the_front() {
        let mut strip = StageStrip::new();
        strip.focus(1, 10);
        strip.focus(2, 20);
        strip.focus(1, 10);
        assert_eq!(strip.front_app(), Some(1));
        assert_eq!(strip.stages()[1].app, 2);
    }

    #[test]
    fn a_window_moves_out_of_its_old_stage_when_its_app_changes() {
        let mut strip = StageStrip::new();
        strip.focus(1, 10);
        strip.focus(1, 11);
        strip.focus(2, 11);
        assert_eq!(strip.stage_of(11).map(|s| s.app), Some(2));
        assert_eq!(strip.stage_of(10).map(|s| s.app), Some(1));
        assert_eq!(strip.stages().iter().map(|s| s.windows.len()).sum::<usize>(), 2);
    }

    #[test]
    fn the_strip_keeps_only_max_stages_and_returns_the_rest() {
        let mut strip = StageStrip::new();
        for app in 1..=MAX_STAGES as u32 {
            assert!(strip.focus(app, app * 10).is_empty());
        }
        let evicted = strip.focus(99, 990);
        assert_eq!(evicted, vec![10], "the oldest stage leaves the strip");
        assert_eq!(strip.stages().len(), MAX_STAGES);
        assert_eq!(strip.front_app(), Some(99));
        assert!(strip.stage_of(10).is_none());
    }

    #[test]
    fn closing_the_last_window_removes_the_stage() {
        let mut strip = StageStrip::new();
        strip.focus(1, 10);
        strip.focus(2, 20);
        assert!(strip.remove_window(20));
        assert_eq!(strip.stages().len(), 1);
        assert!(!strip.remove_window(20), "a closed window is not found twice");
    }

    #[test]
    fn quitting_an_app_removes_all_its_stage_windows() {
        let mut strip = StageStrip::new();
        strip.focus(1, 10);
        strip.focus(1, 11);
        strip.focus(2, 20);
        assert_eq!(strip.remove_app(1), 2);
        assert_eq!(strip.stages(), &[Stage { app: 2, windows: vec![20] }]);
        assert_eq!(strip.remove_app(7), 0);
    }
}
