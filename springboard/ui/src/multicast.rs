//! IPv4 multicast group membership as a pure model. A connection group joins and leaves group addresses, and it
//! keeps the time to live and loopback settings that its datagrams are sent with. Each group is joined at most once,
//! and a group that is not joined cannot be left. The model opens no socket: the shell joins groups on its own
//! socket, and this type holds the rules an app-facing group must follow before that socket call is made.

use std::net::Ipv4Addr;

/// Why a membership change or option was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipError {
    /// The address is not in 224.0.0.0/4.
    NotMulticast,
    /// The group is already joined.
    AlreadyJoined,
    /// The group is not joined, so it cannot be left.
    NotJoined,
    /// A time to live of zero keeps datagrams on the sending host, which this model does not allow.
    ZeroTimeToLive,
}

/// The set of multicast groups one connection group has joined, with its send options.
///
/// The defaults match the usual socket defaults: a time to live of 1 (the local link) and loopback enabled, so a
/// host that joins a group also receives its own datagrams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MulticastMembership {
    groups: Vec<Ipv4Addr>,
    time_to_live: u8,
    loopback: bool,
}

impl Default for MulticastMembership {
    fn default() -> Self {
        MulticastMembership { groups: Vec::new(), time_to_live: 1, loopback: true }
    }
}

impl MulticastMembership {
    /// An empty membership with the default send options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Joins `group`. Refuses an address outside 224.0.0.0/4 and a group that is already joined.
    pub fn join(&mut self, group: Ipv4Addr) -> Result<(), MembershipError> {
        if !group.is_multicast() {
            return Err(MembershipError::NotMulticast);
        }
        if self.is_member(group) {
            return Err(MembershipError::AlreadyJoined);
        }
        self.groups.push(group);
        Ok(())
    }

    /// Leaves `group`. Refuses a group that is not joined.
    pub fn leave(&mut self, group: Ipv4Addr) -> Result<(), MembershipError> {
        let at = self.groups.iter().position(|g| *g == group).ok_or(MembershipError::NotJoined)?;
        self.groups.remove(at);
        Ok(())
    }

    /// True when `group` is joined.
    pub fn is_member(&self, group: Ipv4Addr) -> bool {
        self.groups.contains(&group)
    }

    /// The joined groups, in the order they were joined.
    pub fn groups(&self) -> &[Ipv4Addr] {
        &self.groups
    }

    /// Sets the time to live of sent datagrams. Zero is refused.
    pub fn set_time_to_live(&mut self, ttl: u8) -> Result<(), MembershipError> {
        if ttl == 0 {
            return Err(MembershipError::ZeroTimeToLive);
        }
        self.time_to_live = ttl;
        Ok(())
    }

    /// The time to live of sent datagrams.
    pub fn time_to_live(&self) -> u8 {
        self.time_to_live
    }

    /// Turns loopback of sent datagrams to the local members on or off.
    pub fn set_loopback(&mut self, on: bool) {
        self.loopback = on;
    }

    /// Whether datagrams sent to a joined group are also delivered to this host.
    pub fn loopback(&self) -> bool {
        self.loopback
    }
}
