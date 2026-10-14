//! App permissions from the package manifest, and the check an app makes before it uses a feature.
//!
//! The shell reads the manifest when it launches the app and enforces the grants in its services (the launch policy is
//! in `springboard/src/bare/app_policy.rs`). This model reads the same manifest names so the app's own code can tell
//! which features are available and show the rest as unavailable. It never prompts the user and never changes a grant.
//! First-party entitlements (the remote desktop permission and the system journal) are not manifest permissions and are
//! not represented here.

use std::collections::BTreeSet;

/// A permission a manifest may request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Network,
    Storage,
    Audio,
    Notifications,
    Documents,
    Keychain,
    Accessibility,
    DisplayConfig,
}

impl Permission {
    /// Every permission, in manifest order.
    pub const ALL: [Permission; 8] = [
        Permission::Network,
        Permission::Storage,
        Permission::Audio,
        Permission::Notifications,
        Permission::Documents,
        Permission::Keychain,
        Permission::Accessibility,
        Permission::DisplayConfig,
    ];

    /// The name the manifest uses for the permission.
    pub fn manifest_name(self) -> &'static str {
        match self {
            Permission::Network => "network",
            Permission::Storage => "storage",
            Permission::Audio => "audio",
            Permission::Notifications => "notifications",
            Permission::Documents => "documents",
            Permission::Keychain => "keychain",
            Permission::Accessibility => "accessibility",
            Permission::DisplayConfig => "display-config",
        }
    }

    /// The permission a manifest name stands for; None for an unknown name.
    pub fn from_manifest_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|permission| permission.manifest_name() == name)
    }
}

/// A manifest permission name this system does not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownPermission(pub String);

/// A feature was used without its permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Denied(pub Permission);

/// The permissions an app holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    granted: BTreeSet<Permission>,
}

impl Grants {
    /// The grants from manifest permission names. A repeated name counts once. An unknown name is an error, so a
    /// misspelt permission is never silently dropped.
    pub fn from_manifest<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, UnknownPermission> {
        let mut granted = BTreeSet::new();
        for name in names {
            let permission = Permission::from_manifest_name(name).ok_or_else(|| UnknownPermission(name.to_string()))?;
            granted.insert(permission);
        }
        Ok(Self { granted })
    }

    /// Whether the app holds the permission.
    pub fn allows(&self, permission: Permission) -> bool {
        self.granted.contains(&permission)
    }

    /// Ok when the app holds the permission, else the denial to show.
    pub fn require(&self, permission: Permission) -> Result<(), Denied> {
        if self.allows(permission) {
            Ok(())
        } else {
            Err(Denied(permission))
        }
    }

    /// The held permissions, in manifest order.
    pub fn granted(&self) -> impl Iterator<Item = Permission> + '_ {
        self.granted.iter().copied()
    }
}
