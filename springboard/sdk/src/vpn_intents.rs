//! App Intents for the personal VPN: "connect" and "disconnect" descriptors that Search and Shortcuts can list.
//!
//! The intents only describe the actions. Running them needs a tunnel client in the system network service, which
//! BifrOSt does not ship yet (see `springboard/src/shell/pages.rs`, the VPN page). Until then the shell can show the
//! intents but must not report a tunnel as up because one was requested.

use crate::intents::{Intent, Param, ParamType};

/// Id of the intent that brings the named VPN configuration up.
pub const CONNECT_ID: &str = "vpn_connect";
/// Id of the intent that brings the tunnel down.
pub const DISCONNECT_ID: &str = "vpn_disconnect";
/// Name of the parameter that selects a configuration by its display name.
pub const PROFILE_PARAM: &str = "profile";

/// The two VPN intents, in the order Search lists them.
pub fn vpn_intents() -> Vec<Intent> {
    vec![
        Intent::new(CONNECT_ID, "Connect VPN", "Brings up a VPN configuration.")
            .param(Param::new(PROFILE_PARAM, "Configuration", ParamType::String))
            .shortcut(),
        Intent::new(DISCONNECT_ID, "Disconnect VPN", "Brings the VPN tunnel down.").shortcut(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_intents_with_stable_ids() {
        let list = vpn_intents();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, CONNECT_ID);
        assert_eq!(list[1].id, DISCONNECT_ID);
    }

    #[test]
    fn methods_use_the_intent_prefix() {
        let list = vpn_intents();
        assert_eq!(list[0].method(), "intent.vpn_connect");
        assert_eq!(list[1].method(), "intent.vpn_disconnect");
    }

    #[test]
    fn connect_takes_a_profile_and_disconnect_takes_none() {
        let list = vpn_intents();
        assert_eq!(list[0].params.len(), 1);
        assert_eq!(list[0].params[0].name, PROFILE_PARAM);
        assert!(list[1].params.is_empty());
    }

    #[test]
    fn both_are_shortcuts_and_run_in_background() {
        for i in vpn_intents() {
            assert!(i.shortcut, "{} should be a shortcut", i.id);
            assert!(i.supports(crate::intents::IntentMode::Background));
        }
    }
}
