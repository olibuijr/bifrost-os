//! Custom actions the shell itself offers on its elements. Each open window is an element (`os/windows/<app>`) with the
//! actions a user can take on a window from its title bar: Zoom (maximize or restore), Minimize and Close. An agent lists them
//! with `ax actions <window>` and runs one with `ax action <window> :: <name>`. `ax press` on a window refuses: a window has
//! actions, not a press.

use crate::shell::Shell;
use crate::shell::Effect;

/// The action names a window element offers, in the order they are listed.
pub const WINDOW_ACTIONS: [&str; 3] = ["Zoom", "Minimize", "Close"];

/// The window index a window element's handle names (`win:<index>`).
pub fn window_index(handle: &str) -> Option<usize> {
    handle.strip_prefix("win:")?.parse().ok()
}

impl Shell {
    /// Runs the window action `name` on window `i` (its index in the desk). Refused when the window is gone or the action is
    /// unknown. Close and Minimize refocus the desk the way a title-bar click does.
    #[cfg(feature = "desktop-wayland")]
    pub(in crate::shell) fn window_action(&mut self, i: usize, name: &str, t: f64) -> Result<Vec<Effect>, String> {
        let app = self.desk.as_ref().and_then(|d| d.wins.get(i)).map(|w| w.app).ok_or("that window is gone")?;
        match name {
            "Zoom" => self.desk_zoom_window(i),
            "Minimize" => self.desk_minimize_window(i, t),
            "Close" => self.desk_close_app(app),
            _ => return Err(format!("no window action {name:?}")),
        }
        Ok(Vec::new())
    }

    #[cfg(not(feature = "desktop-wayland"))]
    pub(in crate::shell) fn window_action(&mut self, _i: usize, _name: &str, _t: f64) -> Result<Vec<Effect>, String> {
        Err("there are no windows to act on".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_handle_names_its_index() {
        assert_eq!(window_index("win:0"), Some(0));
        assert_eq!(window_index("win:12"), Some(12));
        assert_eq!(window_index("win:-1"), None);
        assert_eq!(window_index("menu:1:2"), None);
        assert_eq!(window_index("77"), None, "an app node handle is not a window");
    }

    #[test]
    fn the_window_actions_are_listed_once_each() {
        let mut names = WINDOW_ACTIONS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), WINDOW_ACTIONS.len());
        assert_eq!(WINDOW_ACTIONS[0], "Zoom");
    }
}
