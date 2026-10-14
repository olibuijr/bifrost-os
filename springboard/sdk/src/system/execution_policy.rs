//! Launch refusals for app policy: why the shell refuses to start an app whose manifest asks for something it cannot
//! grant. The shell decides; this module maps the refusal to a domain and a numeric code, so an app or a tool can
//! report the same reason the shell logged.

use springboard::bare::app_policy::Policy;
use std::io;

/// The error domain of every launch refusal.
pub const DOMAIN: &str = "is.olibuijr.bifrost.execution-policy";

/// Why a launch was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchRefusal {
    /// The manifest names a permission this system does not grant, or grants to another app.
    UnsupportedPermission(String),
    /// The remote-desktop permission was declared without the network and storage permissions it needs.
    RemoteNeedsNetworkAndStorage,
    /// A refusal this model does not name. The shell's message is kept as written.
    Unclassified(String),
}

impl LaunchRefusal {
    /// The numeric code within [`DOMAIN`].
    pub fn code(&self) -> i32 {
        match self {
            LaunchRefusal::UnsupportedPermission(_) => 1,
            LaunchRefusal::RemoteNeedsNetworkAndStorage => 2,
            LaunchRefusal::Unclassified(_) => 3,
        }
    }

    /// Maps the error that [`Policy::from_permissions`] returns to a refusal.
    pub fn from_error(error: &io::Error) -> LaunchRefusal {
        let message = error.to_string();
        if let Some(permission) = message.strip_prefix("Unsupported app permission: ") {
            LaunchRefusal::UnsupportedPermission(permission.to_string())
        } else if message == "Remote sharing requires network and storage" {
            LaunchRefusal::RemoteNeedsNetworkAndStorage
        } else {
            LaunchRefusal::Unclassified(message)
        }
    }
}

/// Checks the permissions an app declares, as the shell does before it launches the app.
pub fn check(app_id: &str, permissions: &[String]) -> Result<Policy, LaunchRefusal> {
    Policy::from_permissions(app_id, permissions).map_err(|error| LaunchRefusal::from_error(&error))
}
