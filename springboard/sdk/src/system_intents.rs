//! App Intents the system settings and apps declare for Search and Shortcuts: starting a Clock timer, turning Reduce
//! Transparency on or off, and turning Do Not Disturb on or off.
//!
//! Like `vpn_intents`, these only describe the actions. The shell serves a call by changing the same state its
//! Settings page changes (the Clock timers, `accessibility::AccessibilitySettings`, the Focus state), so a call here
//! never reports a change the shell did not make.

use crate::intents::{Intent, Param, ParamType};

/// Id of the intent that starts a Clock timer.
pub const TIMER_START_ID: &str = "timer_start";
/// Id of the intent that turns Reduce Transparency on or off.
pub const REDUCE_TRANSPARENCY_ID: &str = "set_reduce_transparency";
/// Id of the intent that turns Do Not Disturb on or off.
pub const DO_NOT_DISTURB_ID: &str = "set_do_not_disturb";

/// Id of the intent that locks the screen now.
pub const LOCK_SCREEN_ID: &str = "lock_screen";

/// Parameter: how long the timer runs, in whole seconds.
pub const DURATION_PARAM: &str = "duration";
/// Parameter: the optional name of the timer.
pub const LABEL_PARAM: &str = "label";
/// Parameter: the new state of a setting, on or off.
pub const ENABLED_PARAM: &str = "enabled";

/// The timer intent: takes a duration in seconds and an optional label.
pub fn timer_intents() -> Vec<Intent> {
    vec![Intent::new(TIMER_START_ID, "Start Timer", "Starts a Clock timer for a number of seconds.")
        .param(Param::new(DURATION_PARAM, "Duration in seconds", ParamType::Int))
        .param(Param::new(LABEL_PARAM, "Label", ParamType::String).optional())]
}

/// The Reduce Transparency intent and the Do Not Disturb intent, in that order.
pub fn setting_intents() -> Vec<Intent> {
    vec![
        Intent::new(REDUCE_TRANSPARENCY_ID, "Set Reduce Transparency", "Turns Reduce Transparency on or off.")
            .param(Param::new(ENABLED_PARAM, "On", ParamType::Bool)),
        Intent::new(DO_NOT_DISTURB_ID, "Set Do Not Disturb", "Turns Do Not Disturb on or off.")
            .param(Param::new(ENABLED_PARAM, "On", ParamType::Bool)),
    ]
}

/// The Lock Screen intent: no parameters, so it is a shortcut. The shell serves it through the `system_intents`
/// service (`springboard/src/shell/services/system_intents.rs`), the same path Lock Screen in the menu takes.
pub fn lock_intents() -> Vec<Intent> {
    vec![Intent::new(LOCK_SCREEN_ID, "Lock Screen", "Locks the screen now, so the password is asked for again.").shortcut()]
}

/// Every intent this module declares, in the order Search lists them.
pub fn system_intents() -> Vec<Intent> {
    let mut all = timer_intents();
    all.extend(setting_intents());
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_in_order() {
        let ids: Vec<String> = system_intents().iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids, [TIMER_START_ID, REDUCE_TRANSPARENCY_ID, DO_NOT_DISTURB_ID]);
    }

    #[test]
    fn methods_use_the_intent_prefix() {
        let list = system_intents();
        assert_eq!(list[0].method(), "intent.timer_start");
        assert_eq!(list[1].method(), "intent.set_reduce_transparency");
        assert_eq!(list[2].method(), "intent.set_do_not_disturb");
    }

    #[test]
    fn timer_takes_a_duration_and_an_optional_label() {
        let timer = &timer_intents()[0];
        assert_eq!(timer.params.len(), 2);
        assert_eq!(timer.params[0].name, DURATION_PARAM);
        assert_eq!(timer.params[0].ty, ParamType::Int);
        assert!(!timer.params[0].optional);
        assert_eq!(timer.params[1].name, LABEL_PARAM);
        assert!(timer.params[1].optional);
    }

    #[test]
    fn the_two_settings_take_a_required_on_off_value() {
        for i in setting_intents() {
            assert_eq!(i.params.len(), 1, "{}", i.id);
            assert_eq!(i.params[0].name, ENABLED_PARAM);
            assert_eq!(i.params[0].ty, ParamType::Bool);
            assert!(!i.params[0].optional);
        }
    }

    #[test]
    fn lock_screen_is_a_shortcut_without_parameters() {
        let list = lock_intents();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, LOCK_SCREEN_ID);
        assert_eq!(list[0].method(), "intent.lock_screen");
        assert!(list[0].params.is_empty());
        assert!(list[0].shortcut);
    }

    #[test]
    fn no_shortcuts_because_each_needs_a_value() {
        for i in system_intents() {
            assert!(!i.shortcut, "{} needs a value, so it is not a shortcut", i.id);
        }
    }
}
