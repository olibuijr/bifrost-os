//! Package kinds: the one place that says what a manifest's `kind` means (docs/apps.md, "Package kinds").
//!
//! | kind         | directory | home grid | preinstalled | removable | exec      | provides |
//! |--------------|-----------|-----------|--------------|-----------|-----------|----------|
//! | `app`        | `apps/`   | yes       | no           | yes       | required  | no       |
//! | `system-app` | `apps/`   | yes       | yes          | no        | required  | no       |
//! | `system`     | `system/` | no        | yes          | no        | optional  | yes      |  (spawned on demand when it has an `exec`)
//! | `service`    | `system/` | no        | yes          | no        | required  | no       |
//!
//! `tools/aap.py` and `bifrostd` enforce the same table; keep them in step.

pub const APP: &str = "app";
pub const SYSTEM_APP: &str = "system-app";
pub const SYSTEM: &str = "system";
pub const SERVICE: &str = "service";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    SystemApp,
    System,
    Service,
}

impl Kind {
    /// Strict: `""` reads as `app`, any other unknown string is `None` (the package is rejected).
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "" | APP => Kind::App,
            SYSTEM_APP => Kind::SystemApp,
            SYSTEM => Kind::System,
            SERVICE => Kind::Service,
            _ => return None,
        })
    }

    /// Lenient, for data already on disk: an unknown kind is hidden (`System`), never shown or spawned.
    pub fn of(s: &str) -> Kind {
        Kind::parse(s).unwrap_or(Kind::System)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::App => APP,
            Kind::SystemApp => SYSTEM_APP,
            Kind::System => SYSTEM,
            Kind::Service => SERVICE,
        }
    }

    /// Shown on the home grid and in the switcher, spawned by the shell.
    pub fn on_home_grid(self) -> bool {
        matches!(self, Kind::App | Kind::SystemApp)
    }

    /// Not on the home grid: a data package or an `bifrostd` service. The shell spawns none of them except a `system`
    /// package with an `exec` that `provides` something (the keyboard), on demand.
    pub fn hidden_from_grid(self) -> bool {
        !self.on_home_grid()
    }

    /// Installed in the background and never removed by the user.
    pub fn preinstalled(self) -> bool {
        self != Kind::App
    }

    /// May omit `exec` (a data-only package).
    pub fn exec_optional(self) -> bool {
        self == Kind::System
    }

    /// May declare `provides`.
    pub fn may_provide(self) -> bool {
        self == Kind::System
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_is_strict_and_empty_means_app() {
        assert_eq!(Kind::parse(""), Some(Kind::App));
        assert_eq!(Kind::parse("app"), Some(Kind::App));
        assert_eq!(Kind::parse("widget"), None);
        for k in [Kind::App, Kind::SystemApp, Kind::System, Kind::Service] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn unknown_kinds_are_hidden() {
        assert!(Kind::of("widget").hidden_from_grid());
        assert!(!Kind::of("system-app").hidden_from_grid());
    }

    #[test]
    fn only_system_packages_are_data_only_or_provide() {
        assert!(Kind::System.exec_optional() && Kind::System.may_provide());
        assert!(!Kind::Service.exec_optional() && !Kind::Service.may_provide());
        assert!(!Kind::App.preinstalled() && Kind::SystemApp.preinstalled());
    }
}
