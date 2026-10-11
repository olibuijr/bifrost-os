//! Service environment of the bare shell: where data lives, and which window layout is used.
//! Chosen at start-up, never at build time.

/// Where settings, notes, wallpapers and the rest live when no directory is given on the command line:
/// /var/lib/springboard for root (the OS shell) and `$XDG_DATA_HOME/springboard`
/// (else `~/.local/share/springboard`) for a user.
pub fn data_dir(root: bool, xdg_data_home: Option<&str>, home: Option<&str>) -> String {
    if root {
        return "/var/lib/springboard".to_string();
    }
    match (xdg_data_home.filter(|s| !s.is_empty()), home.filter(|s| !s.is_empty())) {
        (Some(x), _) => format!("{}/springboard", x.trim_end_matches('/')),
        (None, Some(h)) => format!("{}/.local/share/springboard", h.trim_end_matches('/')),
        (None, None) => "/var/lib/springboard".to_string(),
    }
}

/// Window and navigation layout, independent of architecture and service paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation { Bifrost, Desktop }
impl Presentation {
    pub fn pick(layout: Option<&str>, legacy_desk: Option<&str>) -> Self {
        match layout.map(str::trim) {
            Some("bifrost") => Self::Bifrost,
            Some("desktop") => Self::Desktop,
            _ => match legacy_desk {
                Some("0" | "false") => Self::Bifrost,
                Some(_) => Self::Desktop,
                None => Self::Desktop,
            },
        }
    }
    pub fn detect() -> Self {
        let layout = std::env::var("BIFROST_LAYOUT").ok();
        let desk = std::env::var("SPRINGBOARD_DESK").ok();
        Self::pick(layout.as_deref(), desk.as_deref())
    }
    pub fn is_desktop(self) -> bool { self == Self::Desktop }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_env_wins() {
        assert_eq!(Presentation::pick(Some("bifrost"), None), Presentation::Bifrost);
        assert_eq!(Presentation::pick(Some("desktop"), Some("0")), Presentation::Desktop);
        assert_eq!(Presentation::pick(None, Some("0")), Presentation::Bifrost);
        assert_eq!(Presentation::pick(None, None), Presentation::Desktop);
    }

    #[test]
    fn data_dirs() {
        assert_eq!(data_dir(true, Some("/x"), Some("/h")), "/var/lib/springboard");
        assert_eq!(data_dir(false, Some("/x/"), Some("/h")), "/x/springboard");
        assert_eq!(data_dir(false, None, Some("/home/u")), "/home/u/.local/share/springboard");
        assert_eq!(data_dir(false, Some(""), Some("")), "/var/lib/springboard");
    }
}
