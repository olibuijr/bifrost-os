//! System content-sharing picker: the selection a person makes in the system's picker before a capture starts. The
//! picker offers five selection modes (one display, one window, one application, several windows, several
//! applications), and the app can narrow them, exclude windows or applications from the list, and say whether the
//! person may change a selection once it is made. `Picker` checks each choice against that configuration before it
//! is accepted. Pure state: nothing is shown here and nothing is captured.

/// A selection mode the picker may offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerMode {
    /// One display.
    SingleDisplay,
    /// One window.
    SingleWindow,
    /// One application, with all its windows.
    SingleApplication,
    /// Any number of windows, at least one.
    MultipleWindows,
    /// Any number of applications, at least one.
    MultipleApplications,
}

impl PickerMode {
    /// Whether the mode takes more than one item.
    pub fn is_multiple(self) -> bool {
        matches!(self, PickerMode::MultipleWindows | PickerMode::MultipleApplications)
    }

    /// Whether an item of this kind can be chosen in this mode.
    pub fn accepts(self, item: &Content) -> bool {
        matches!(
            (self, item),
            (PickerMode::SingleDisplay, Content::Display(_))
                | (PickerMode::SingleWindow | PickerMode::MultipleWindows, Content::Window(_))
                | (PickerMode::SingleApplication | PickerMode::MultipleApplications, Content::Application(_))
        )
    }
}

/// One item the person can pick: a display, a window or an application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// A display, by its id.
    Display(u32),
    /// A window, by its id.
    Window(u32),
    /// An application, by its bundle identifier.
    Application(String),
}

/// What the person picked: a mode and the items chosen in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub mode: PickerMode,
    pub items: Vec<Content>,
}

/// What the app asks of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerConfig {
    /// The modes the picker offers. A choice in any other mode is refused.
    pub allowed_modes: Vec<PickerMode>,
    /// Window ids that must not be chosen, for example the app's own windows.
    pub excluded_windows: Vec<u32>,
    /// Application bundle identifiers that must not be chosen.
    pub excluded_apps: Vec<String>,
    /// Whether the person may change a selection after it is made. When false, a made selection is final.
    pub allows_changing_selection: bool,
}

impl Default for PickerConfig {
    fn default() -> Self {
        PickerConfig {
            allowed_modes: vec![
                PickerMode::SingleDisplay,
                PickerMode::SingleWindow,
                PickerMode::SingleApplication,
                PickerMode::MultipleWindows,
                PickerMode::MultipleApplications,
            ],
            excluded_windows: Vec::new(),
            excluded_apps: Vec::new(),
            allows_changing_selection: true,
        }
    }
}

/// Where the picker stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not shown yet.
    Idle,
    /// Shown and waiting for the person.
    Presented,
    /// The person made a selection that passed every check.
    Selected(Selection),
    /// The person closed the picker without choosing.
    Cancelled,
}

/// Why the picker refused an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerError {
    /// The action needs the picker to be shown first, or it was closed.
    NotPresented,
    /// The chosen mode is not in the allowed modes.
    ModeNotAllowed,
    /// An item's kind does not match the mode, for example a window in the display mode.
    WrongContent,
    /// A single mode got other than one item, or a multiple mode got none.
    CountMismatch,
    /// The same item was chosen twice.
    Duplicate,
    /// An item is on the exclusion list.
    Excluded,
    /// A selection was made and the configuration does not allow changing it.
    SelectionLocked,
}

/// The system picker for one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    config: PickerConfig,
    state: State,
}

impl Picker {
    /// A picker that has not been shown.
    pub fn new(config: PickerConfig) -> Self {
        Picker { config, state: State::Idle }
    }

    /// The configuration the picker checks against.
    pub fn config(&self) -> &PickerConfig {
        &self.config
    }

    /// The current state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The selection, once one is made.
    pub fn selection(&self) -> Option<&Selection> {
        match &self.state {
            State::Selected(sel) => Some(sel),
            _ => None,
        }
    }

    /// Shows the picker. A made selection can be replaced only when the configuration allows changing it.
    pub fn present(&mut self) -> Result<(), PickerError> {
        let locked = matches!(self.state, State::Selected(_)) && !self.config.allows_changing_selection;
        if locked {
            return Err(PickerError::SelectionLocked);
        }
        self.state = State::Presented;
        Ok(())
    }

    /// Accepts the person's choice when the picker is shown and the choice passes every check.
    pub fn choose(&mut self, selection: Selection) -> Result<(), PickerError> {
        if self.state != State::Presented {
            return Err(PickerError::NotPresented);
        }
        self.check(&selection)?;
        self.state = State::Selected(selection);
        Ok(())
    }

    /// The person closed the picker without choosing.
    pub fn cancel(&mut self) -> Result<(), PickerError> {
        if self.state != State::Presented {
            return Err(PickerError::NotPresented);
        }
        self.state = State::Cancelled;
        Ok(())
    }

    fn check(&self, sel: &Selection) -> Result<(), PickerError> {
        if !self.config.allowed_modes.contains(&sel.mode) {
            return Err(PickerError::ModeNotAllowed);
        }
        let count = sel.items.len();
        let count_ok = if sel.mode.is_multiple() { count >= 1 } else { count == 1 };
        if !count_ok {
            return Err(PickerError::CountMismatch);
        }
        for (i, item) in sel.items.iter().enumerate() {
            if !sel.mode.accepts(item) {
                return Err(PickerError::WrongContent);
            }
            if sel.items[..i].contains(item) {
                return Err(PickerError::Duplicate);
            }
            let excluded = match item {
                Content::Display(_) => false,
                Content::Window(id) => self.config.excluded_windows.contains(id),
                Content::Application(bundle) => self.config.excluded_apps.iter().any(|b| b == bundle),
            };
            if excluded {
                return Err(PickerError::Excluded);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_selection(ids: &[u32]) -> Selection {
        Selection { mode: PickerMode::MultipleWindows, items: ids.iter().map(|&id| Content::Window(id)).collect() }
    }

    fn shown(config: PickerConfig) -> Picker {
        let mut p = Picker::new(config);
        p.present().unwrap();
        p
    }

    #[test]
    fn the_default_offers_all_five_modes_and_allows_changes() {
        let c = PickerConfig::default();
        assert_eq!(c.allowed_modes.len(), 5);
        assert!(c.allowed_modes.contains(&PickerMode::SingleDisplay));
        assert!(c.allows_changing_selection);
        assert!(c.excluded_windows.is_empty());
    }

    #[test]
    fn a_single_window_choice_is_accepted_once_the_picker_is_shown() {
        let mut p = Picker::new(PickerConfig::default());
        assert_eq!(p.choose(Selection { mode: PickerMode::SingleWindow, items: vec![Content::Window(7)] }), Err(PickerError::NotPresented));
        p.present().unwrap();
        let sel = Selection { mode: PickerMode::SingleWindow, items: vec![Content::Window(7)] };
        assert_eq!(p.choose(sel.clone()), Ok(()));
        assert_eq!(p.selection(), Some(&sel));
    }

    #[test]
    fn single_modes_take_exactly_one_item_and_multiple_modes_at_least_one() {
        let mut p = shown(PickerConfig::default());
        let two_in_single = Selection { mode: PickerMode::SingleWindow, items: vec![Content::Window(1), Content::Window(2)] };
        assert_eq!(p.choose(two_in_single), Err(PickerError::CountMismatch));
        let none = Selection { mode: PickerMode::MultipleWindows, items: vec![] };
        assert_eq!(p.choose(none), Err(PickerError::CountMismatch));
        assert_eq!(p.choose(window_selection(&[1, 2, 3])), Ok(()));
    }

    #[test]
    fn a_mode_the_app_did_not_allow_is_refused() {
        let config = PickerConfig { allowed_modes: vec![PickerMode::SingleDisplay], ..PickerConfig::default() };
        let mut p = shown(config);
        assert_eq!(p.choose(window_selection(&[4])), Err(PickerError::ModeNotAllowed));
        assert_eq!(p.choose(Selection { mode: PickerMode::SingleDisplay, items: vec![Content::Display(0)] }), Ok(()));
    }

    #[test]
    fn an_excluded_window_or_application_cannot_be_chosen() {
        let config = PickerConfig {
            excluded_windows: vec![9],
            excluded_apps: vec!["org.example.own".to_string()],
            ..PickerConfig::default()
        };
        let mut p = shown(config);
        assert_eq!(p.choose(window_selection(&[1, 9])), Err(PickerError::Excluded));
        let own = Selection { mode: PickerMode::SingleApplication, items: vec![Content::Application("org.example.own".into())] };
        assert_eq!(p.choose(own), Err(PickerError::Excluded));
        assert_eq!(p.choose(window_selection(&[1])), Ok(()));
    }

    #[test]
    fn items_of_the_wrong_kind_and_repeated_items_are_refused() {
        let mut p = shown(PickerConfig::default());
        let display_as_window = Selection { mode: PickerMode::SingleDisplay, items: vec![Content::Window(3)] };
        assert_eq!(p.choose(display_as_window), Err(PickerError::WrongContent));
        assert_eq!(p.choose(window_selection(&[5, 5])), Err(PickerError::Duplicate));
    }

    #[test]
    fn a_cancelled_picker_must_be_shown_again_before_a_choice() {
        let mut p = shown(PickerConfig::default());
        assert_eq!(p.cancel(), Ok(()));
        assert_eq!(p.state(), &State::Cancelled);
        assert_eq!(p.choose(window_selection(&[1])), Err(PickerError::NotPresented));
        assert_eq!(p.cancel(), Err(PickerError::NotPresented));
    }

    #[test]
    fn a_locked_selection_cannot_be_replaced_when_changes_are_not_allowed() {
        let config = PickerConfig { allows_changing_selection: false, ..PickerConfig::default() };
        let mut p = shown(config);
        p.choose(window_selection(&[1])).unwrap();
        assert_eq!(p.present(), Err(PickerError::SelectionLocked));
        assert_eq!(p.selection(), Some(&window_selection(&[1])));
    }

    #[test]
    fn a_selection_can_be_replaced_when_changes_are_allowed() {
        let mut p = shown(PickerConfig::default());
        p.choose(window_selection(&[1])).unwrap();
        assert_eq!(p.present(), Ok(()));
        assert_eq!(p.selection(), None);
        assert_eq!(p.choose(window_selection(&[2])), Ok(()));
        assert_eq!(p.selection(), Some(&window_selection(&[2])));
    }
}
