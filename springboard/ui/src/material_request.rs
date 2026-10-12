//! Material requests: a surface says which reference material it is (`NativeVisualEffectView.Material`
//! kind), where, with which corner radius and active state, instead of drawing its own blur and tint.
//! The renderer maps the request to its glass shape and the named material colours; there are no
//! per-surface blur or tint numbers. Built-in shell surfaces and app windows use the same kinds.
//! Minimal on purpose: the visual-effect API branch owns the full request type and this is reconciled with it.

/// The reference material kinds (the same set apps declare through the SDK).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    Menu,
    Popover,
    Sheet,
    HudWindow,
    ToolTip,
    FullScreenUi,
    Sidebar,
    Titlebar,
    HeaderView,
    UnderWindowBackground,
}

/// Whether the material follows its window's active state or is pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MaterialState {
    #[default]
    FollowsWindowActiveState,
    Active,
    Inactive,
}

/// One translucent surface of the shell itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellSurface {
    Spotlight,
    MenuBar,
    MenuBarMenu,
    ContextMenu,
    Dock,
    ControlCentre,
    NotificationPanel,
    NotificationBanner,
    Popover,
    Sheet,
    Tooltip,
    SettingsSidebar,
    SettingsToolbar,
    OverviewVeil,
    OverviewPill,
    AppGridVeil,
}

impl ShellSurface {
    pub const ALL: [ShellSurface; 16] = [
        ShellSurface::Spotlight,
        ShellSurface::MenuBar,
        ShellSurface::MenuBarMenu,
        ShellSurface::ContextMenu,
        ShellSurface::Dock,
        ShellSurface::ControlCentre,
        ShellSurface::NotificationPanel,
        ShellSurface::NotificationBanner,
        ShellSurface::Popover,
        ShellSurface::Sheet,
        ShellSurface::Tooltip,
        ShellSurface::SettingsSidebar,
        ShellSurface::SettingsToolbar,
        ShellSurface::OverviewVeil,
        ShellSurface::OverviewPill,
        ShellSurface::AppGridVeil,
    ];

    /// The reference material this surface is.
    pub fn material(self) -> MaterialKind {
        use MaterialKind as M;
        match self {
            ShellSurface::Spotlight | ShellSurface::ControlCentre | ShellSurface::NotificationPanel | ShellSurface::OverviewPill => M::HudWindow,
            ShellSurface::MenuBar | ShellSurface::SettingsToolbar => M::HeaderView,
            ShellSurface::MenuBarMenu | ShellSurface::ContextMenu => M::Menu,
            ShellSurface::Dock => M::UnderWindowBackground,
            ShellSurface::NotificationBanner | ShellSurface::Popover => M::Popover,
            ShellSurface::Sheet => M::Sheet,
            ShellSurface::Tooltip => M::ToolTip,
            ShellSurface::SettingsSidebar => M::Sidebar,
            ShellSurface::OverviewVeil | ShellSurface::AppGridVeil => M::FullScreenUi,
        }
    }
}

/// A material request for one rectangle (logical pixels of the surface that carries it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialRequest {
    pub kind: MaterialKind,
    pub rect: (f32, f32, f32, f32),
    pub radius: f32,
    pub state: MaterialState,
}

impl MaterialRequest {
    pub fn for_surface(surface: ShellSurface, rect: (f32, f32, f32, f32), radius: f32) -> Self {
        MaterialRequest { kind: surface.material(), rect, radius, state: MaterialState::FollowsWindowActiveState }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MaterialKind as M;

    /// Every translucent shell surface maps to one reference material kind.
    #[test]
    fn every_shell_surface_maps_to_a_reference_material() {
        let table = [
            (ShellSurface::Spotlight, M::HudWindow),
            (ShellSurface::MenuBar, M::HeaderView),
            (ShellSurface::MenuBarMenu, M::Menu),
            (ShellSurface::ContextMenu, M::Menu),
            (ShellSurface::Dock, M::UnderWindowBackground),
            (ShellSurface::ControlCentre, M::HudWindow),
            (ShellSurface::NotificationPanel, M::HudWindow),
            (ShellSurface::NotificationBanner, M::Popover),
            (ShellSurface::Popover, M::Popover),
            (ShellSurface::Sheet, M::Sheet),
            (ShellSurface::Tooltip, M::ToolTip),
            (ShellSurface::SettingsSidebar, M::Sidebar),
            (ShellSurface::SettingsToolbar, M::HeaderView),
            (ShellSurface::OverviewVeil, M::FullScreenUi),
            (ShellSurface::OverviewPill, M::HudWindow),
            (ShellSurface::AppGridVeil, M::FullScreenUi),
        ];
        assert_eq!(table.len(), ShellSurface::ALL.len());
        for s in ShellSurface::ALL {
            let (_, want) = table.iter().find(|(t, _)| *t == s).expect("surface in table");
            assert_eq!(s.material(), *want, "{s:?}");
        }
    }

    #[test]
    fn request_carries_the_surface_material() {
        let r = MaterialRequest::for_surface(ShellSurface::Spotlight, (1.0, 2.0, 3.0, 4.0), 12.0);
        assert_eq!(r.kind, M::HudWindow);
        assert_eq!(r.state, MaterialState::FollowsWindowActiveState);
    }
}
