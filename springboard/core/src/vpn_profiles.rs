//! Saved VPN profiles: a store of named tunnel server profiles, one of which can be the active profile, with a text
//! form that the system keeps on disk.
//!
//! The store holds no secrets, as [`VpnProfile`] does. Names are unique without regard to ASCII case, and every
//! profile is checked before it is added or replaced, so a stored profile always passes [`VpnProfile::validate`].
//! Writing the file and reading it back is the caller's job; [`VpnProfileStore::to_text`] and
//! [`VpnProfileStore::from_text`] only convert between the store and its text.
//!
//! The text form has a header line, one `profile` line per profile with tab-separated fields (name, server, port,
//! protocol key, on-demand flag as 0 or 1), and an optional `active` line that names the active profile. Names
//! cannot contain tabs or line breaks, so the fields never need escaping.

use crate::tunnel::{TunnelError, VpnProfile, VpnProtocol};
use std::fmt::Write as _;

const HEADER: &str = "bifrost-vpn-profiles 1";

/// Named VPN profiles and the one that is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnProfileStore {
    profiles: Vec<VpnProfile>,
    active: Option<String>,
}

fn refused(why: &str) -> TunnelError {
    TunnelError::InvalidConfiguration(why.to_string())
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn check_profile(profile: &VpnProfile) -> Result<(), TunnelError> {
    if profile.name.chars().any(char::is_control) {
        return Err(refused("the profile name cannot contain control characters"));
    }
    profile.validate()
}

fn protocol_key(protocol: VpnProtocol) -> &'static str {
    match protocol {
        VpnProtocol::Ikev2 => "ikev2",
        VpnProtocol::IpsecIkev1 => "ikev1",
        VpnProtocol::Tunnel => "tunnel",
    }
}

fn protocol_from_key(key: &str) -> Result<VpnProtocol, TunnelError> {
    match key {
        "ikev2" => Ok(VpnProtocol::Ikev2),
        "ikev1" => Ok(VpnProtocol::IpsecIkev1),
        "tunnel" => Ok(VpnProtocol::Tunnel),
        _ => Err(refused("the profile protocol is not known")),
    }
}

impl VpnProfileStore {
    /// An empty store with no active profile.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// The profiles sorted by name, ignoring ASCII case.
    pub fn list(&self) -> Vec<&VpnProfile> {
        let mut sorted: Vec<&VpnProfile> = self.profiles.iter().collect();
        sorted.sort_by_key(|profile| profile.name.to_ascii_lowercase());
        sorted
    }

    /// The profile with this name, matched without regard to ASCII case.
    pub fn get(&self, name: &str) -> Option<&VpnProfile> {
        self.profiles.iter().find(|profile| same_name(&profile.name, name))
    }

    /// Add a profile. It is refused when it fails validation or when another profile already has its name.
    pub fn add(&mut self, profile: VpnProfile) -> Result<(), TunnelError> {
        check_profile(&profile)?;
        if self.get(&profile.name).is_some() {
            return Err(refused("a profile with this name already exists"));
        }
        self.profiles.push(profile);
        Ok(())
    }

    /// Replace the profile called `name`. A rename keeps the profile active when it was active, and a new name that
    /// another profile uses is refused, leaving the store unchanged.
    pub fn replace(&mut self, name: &str, profile: VpnProfile) -> Result<(), TunnelError> {
        check_profile(&profile)?;
        let index = self.index_of(name).ok_or_else(|| refused("no profile has this name"))?;
        let clashes = self
            .profiles
            .iter()
            .enumerate()
            .any(|(i, other)| i != index && same_name(&other.name, &profile.name));
        if clashes {
            return Err(refused("a profile with this name already exists"));
        }
        if self.active.as_deref().is_some_and(|active| same_name(active, name)) {
            self.active = Some(profile.name.clone());
        }
        self.profiles[index] = profile;
        Ok(())
    }

    /// Remove the profile called `name` and return it. Removing the active profile leaves no profile active.
    pub fn remove(&mut self, name: &str) -> Option<VpnProfile> {
        let index = self.index_of(name)?;
        if self.active.as_deref().is_some_and(|active| same_name(active, name)) {
            self.active = None;
        }
        Some(self.profiles.remove(index))
    }

    /// The active profile, if one is set.
    pub fn active(&self) -> Option<&VpnProfile> {
        self.get(self.active.as_deref()?)
    }

    /// Make the profile called `name` active, or clear the active profile with `None`. An unknown name is refused.
    pub fn set_active(&mut self, name: Option<&str>) -> Result<(), TunnelError> {
        self.active = match name {
            None => None,
            Some(name) => Some(self.get(name).ok_or_else(|| refused("no profile has this name"))?.name.clone()),
        };
        Ok(())
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.profiles.iter().position(|profile| same_name(&profile.name, name))
    }

    /// Render the store as the text form described in the module documentation.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = writeln!(out, "{HEADER}");
        for profile in &self.profiles {
            let _ = writeln!(
                out,
                "profile\t{}\t{}\t{}\t{}\t{}",
                profile.name,
                profile.server,
                profile.port,
                protocol_key(profile.protocol),
                u8::from(profile.on_demand)
            );
        }
        if let Some(active) = &self.active {
            let _ = writeln!(out, "active\t{active}");
        }
        out
    }

    /// Read the text form back. Every profile is added through [`VpnProfileStore::add`], so a file that holds an
    /// invalid profile, a duplicate name or an unknown active name is refused as a whole.
    pub fn from_text(text: &str) -> Result<Self, TunnelError> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(refused("the profile file has an unknown header"));
        }
        let mut store = VpnProfileStore::new();
        let mut active: Option<String> = None;
        for line in lines.filter(|line| !line.trim().is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["profile", name, server, port, protocol, on_demand] => {
                    let port = port.parse::<u16>().map_err(|_| refused("a profile port is not a number"))?;
                    let on_demand = match *on_demand {
                        "0" => false,
                        "1" => true,
                        _ => return Err(refused("the on-demand flag must be 0 or 1")),
                    };
                    store.add(VpnProfile {
                        name: name.to_string(),
                        server: server.to_string(),
                        port,
                        protocol: protocol_from_key(protocol)?,
                        on_demand,
                    })?;
                }
                ["active", name] => active = Some(name.to_string()),
                _ => return Err(refused("a line of the profile file has the wrong shape")),
            }
        }
        store.set_active(active.as_deref())?;
        Ok(store)
    }
}
