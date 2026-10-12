//! How a notification is presented: its interruption level, the app's own delivery settings and the Focus that is on
//! decide whether it is only stored (list and badge), shown as a banner that leaves on its own, or shown as an alert
//! that stays until the user answers it, and whether it plays a sound, wakes a dark screen or gets past a Focus.
//! Pure policy: nothing here draws, plays or reads a clock, so the shell passes the app's settings and the Focus state
//! and acts on the returned `Decision`.

/// The interruption level an app marks a notification with, from least to most intrusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Kept in Notification Center without a banner, a sound or a screen wake.
    Passive,
    /// The normal level: a banner, or an alert when the app uses that style.
    Active,
    /// Breaks through a Focus when the app is allowed to mark its notifications time-sensitive.
    TimeSensitive,
    /// Always presented as an alert with a sound, even under a Focus or a muted app.
    Critical,
}

/// Where the notification is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// Only stored: listed in Notification Center and counted on the badge.
    Stored,
    /// A banner that goes away on its own.
    Banner,
    /// An alert that stays on screen until the user answers it.
    Alert,
}

/// The delivery settings the user chose for one app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppPolicy {
    /// Banners and alerts are allowed at all (the app's "Allow Notifications" switch).
    pub banners: bool,
    /// The app uses the alert style instead of the banner style.
    pub alerts: bool,
    /// Notifications play a sound.
    pub sounds: bool,
    /// The app may mark notifications time-sensitive, so they can get past a Focus.
    pub time_sensitive: bool,
}

/// The system state the decision depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    /// A Focus is on.
    pub focus_on: bool,
    /// The active Focus lets this app's notifications through.
    pub focus_allows_app: bool,
}

/// What the shell does with one notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub presentation: Presentation,
    pub sound: bool,
    /// A dark screen wakes to show it. Only presented notifications wake the screen.
    pub wakes_screen: bool,
    /// The notification got past a Focus that would otherwise hold it back.
    pub breaks_focus: bool,
}

/// Decides how one notification at `level` is presented for an app with `app` settings while `ctx` holds.
pub fn decide(level: Level, app: AppPolicy, ctx: Context) -> Decision {
    let through_focus = level == Level::Critical || (level == Level::TimeSensitive && app.time_sensitive);
    let held_by_focus = ctx.focus_on && !ctx.focus_allows_app && !through_focus;
    let breaks_focus = ctx.focus_on && !ctx.focus_allows_app && through_focus;

    let presentation = if level == Level::Passive || held_by_focus {
        Presentation::Stored
    } else if level == Level::Critical {
        Presentation::Alert
    } else if !app.banners {
        Presentation::Stored
    } else if app.alerts {
        Presentation::Alert
    } else {
        Presentation::Banner
    };

    let shown = presentation != Presentation::Stored;
    Decision {
        presentation,
        sound: shown && (app.sounds || level == Level::Critical),
        wakes_screen: shown,
        breaks_focus: shown && breaks_focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: AppPolicy = AppPolicy { banners: true, alerts: false, sounds: true, time_sensitive: false };
    const NO_FOCUS: Context = Context { focus_on: false, focus_allows_app: false };
    const FOCUS: Context = Context { focus_on: true, focus_allows_app: false };

    #[test]
    fn passive_is_only_stored_and_never_sounds_or_wakes() {
        let d = decide(Level::Passive, ON, NO_FOCUS);
        assert_eq!(d.presentation, Presentation::Stored);
        assert!(!d.sound && !d.wakes_screen && !d.breaks_focus);
    }

    #[test]
    fn active_is_a_banner_unless_the_app_uses_alerts() {
        assert_eq!(decide(Level::Active, ON, NO_FOCUS).presentation, Presentation::Banner);
        let alerts = AppPolicy { alerts: true, ..ON };
        assert_eq!(decide(Level::Active, alerts, NO_FOCUS).presentation, Presentation::Alert);
    }

    #[test]
    fn an_app_with_banners_off_only_stores_except_critical() {
        let off = AppPolicy { banners: false, ..ON };
        assert_eq!(decide(Level::Active, off, NO_FOCUS).presentation, Presentation::Stored);
        assert_eq!(decide(Level::Critical, off, NO_FOCUS).presentation, Presentation::Alert);
    }

    #[test]
    fn a_focus_holds_active_notifications_back() {
        let d = decide(Level::Active, ON, FOCUS);
        assert_eq!(d.presentation, Presentation::Stored);
        assert!(!d.sound && !d.wakes_screen && !d.breaks_focus);
    }

    #[test]
    fn time_sensitive_breaks_a_focus_only_for_apps_allowed_to_use_it() {
        let d = decide(Level::TimeSensitive, ON, FOCUS);
        assert_eq!(d.presentation, Presentation::Stored, "app not allowed: held");
        let allowed = AppPolicy { time_sensitive: true, ..ON };
        let d = decide(Level::TimeSensitive, allowed, FOCUS);
        assert_eq!(d.presentation, Presentation::Banner);
        assert!(d.breaks_focus && d.sound && d.wakes_screen);
    }

    #[test]
    fn critical_breaks_a_focus_and_sounds_even_when_muted_in_sound() {
        let quiet = AppPolicy { sounds: false, ..ON };
        let d = decide(Level::Critical, quiet, FOCUS);
        assert_eq!(d.presentation, Presentation::Alert);
        assert!(d.breaks_focus && d.sound && d.wakes_screen);
    }

    #[test]
    fn an_app_the_focus_allows_is_not_a_focus_break() {
        let allowed = Context { focus_on: true, focus_allows_app: true };
        let d = decide(Level::Active, ON, allowed);
        assert_eq!(d.presentation, Presentation::Banner);
        assert!(!d.breaks_focus);
    }

    #[test]
    fn sound_follows_the_app_setting_for_shown_notifications() {
        let silent = AppPolicy { sounds: false, ..ON };
        let d = decide(Level::Active, silent, NO_FOCUS);
        assert_eq!(d.presentation, Presentation::Banner);
        assert!(!d.sound && d.wakes_screen);
    }
}
