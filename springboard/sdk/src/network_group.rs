//! `NWConnectionGroup` parity: an app-facing IPv4 multicast UDP group over `std::net`.
//!
//! `ConnectionGroup::new(port)` binds a UDP socket on the port (0 picks a free one, read back with
//! `local_port`). `join` and `leave` change the group membership on that socket, and the rules come
//! from `bifrost_ui::multicast::MulticastMembership`: a non-multicast address, a repeated join and a
//! leave of a group that is not joined are refused before the socket is touched. `send` writes one
//! datagram to a joined multicast group, and `receive` returns the next datagram or `None` without
//! blocking, so the owner can call it from its run loop.
//!
//! The socket is bound with `SO_REUSEADDR` (set before the bind through the C library, because `std::net`
//! cannot set it), so several groups on one host can share a multicast port and each receives the group
//! datagrams. The option is applied on Linux only; other systems fall back to a plain bind.

use crate::network_listener::Datagram;
use bifrost_ui::multicast::{MembershipError, MulticastMembership};
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

/// Why a group operation was refused.
#[derive(Debug)]
pub enum GroupError {
    /// The membership rules refused the change (not multicast, already joined, not joined, zero TTL).
    Membership(MembershipError),
    /// The socket refused the change or could not be bound. Carries the system message.
    Socket(String),
}

impl From<MembershipError> for GroupError {
    fn from(e: MembershipError) -> Self {
        GroupError::Membership(e)
    }
}

/// Binds a UDP socket on `0.0.0.0:port` with `SO_REUSEADDR` set before the bind.
#[cfg(target_os = "linux")]
fn bind_shared(port: u16) -> io::Result<UdpSocket> {
    use std::os::fd::FromRawFd;

    /// `struct sockaddr_in` (Linux, IPv4): family, port and address in network byte order, then padding.
    #[repr(C)]
    struct SockAddrIn {
        sin_family: u16,
        sin_port: u16,
        sin_addr: u32,
        sin_zero: [u8; 8],
    }

    extern "C" {
        fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
        fn bind(fd: i32, addr: *const SockAddrIn, len: u32) -> i32;
        fn close(fd: i32) -> i32;
    }
    const AF_INET: i32 = 2;
    const SOCK_DGRAM: i32 = 2;
    const SOCK_CLOEXEC: i32 = 0o2_000_000;
    const SOL_SOCKET: i32 = 1;
    const SO_REUSEADDR: i32 = 2;

    // SAFETY: plain C calls on a descriptor this function owns; the address struct is a live local
    // with the Linux `sockaddr_in` layout, and every descriptor is closed on the error paths.
    unsafe {
        let fd = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let on: i32 = 1;
        if setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &on, std::mem::size_of::<i32>() as u32) != 0 {
            let e = io::Error::last_os_error();
            close(fd);
            return Err(e);
        }
        let addr = SockAddrIn { sin_family: AF_INET as u16, sin_port: port.to_be(), sin_addr: 0, sin_zero: [0; 8] };
        if bind(fd, &addr, std::mem::size_of::<SockAddrIn>() as u32) != 0 {
            let e = io::Error::last_os_error();
            close(fd);
            return Err(e);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

/// Other systems: a plain bind on `0.0.0.0:port`.
#[cfg(not(target_os = "linux"))]
fn bind_shared(port: u16) -> io::Result<UdpSocket> {
    UdpSocket::bind(("0.0.0.0", port))
}

/// `NWConnectionGroup`: one UDP socket and the multicast groups it has joined.
pub struct ConnectionGroup {
    membership: MulticastMembership,
    socket: UdpSocket,
}

impl ConnectionGroup {
    /// Binds the group's socket on `0.0.0.0:port` and applies the default send options.
    pub fn new(port: u16) -> Result<Self, GroupError> {
        let socket = bind_shared(port).map_err(|e| GroupError::Socket(e.to_string()))?;
        socket.set_nonblocking(true).map_err(|e| GroupError::Socket(e.to_string()))?;
        let membership = MulticastMembership::new();
        socket.set_multicast_loop_v4(membership.loopback()).map_err(|e| GroupError::Socket(e.to_string()))?;
        socket.set_multicast_ttl_v4(u32::from(membership.time_to_live())).map_err(|e| GroupError::Socket(e.to_string()))?;
        Ok(ConnectionGroup { membership, socket })
    }

    /// The port the socket is bound to.
    pub fn local_port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// The membership and send options this group holds.
    pub fn membership(&self) -> &MulticastMembership {
        &self.membership
    }

    /// Joins `group` on the socket. The model checks the address and the duplicate first; a socket
    /// failure leaves the membership unchanged.
    pub fn join(&mut self, group: Ipv4Addr) -> Result<(), GroupError> {
        let mut next = self.membership.clone();
        next.join(group)?;
        self.socket
            .join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| GroupError::Socket(e.to_string()))?;
        self.membership = next;
        Ok(())
    }

    /// Leaves `group`. Refuses a group that is not joined.
    pub fn leave(&mut self, group: Ipv4Addr) -> Result<(), GroupError> {
        if !self.membership.is_member(group) {
            return Err(MembershipError::NotJoined.into());
        }
        self.socket
            .leave_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| GroupError::Socket(e.to_string()))?;
        self.membership.leave(group)?;
        Ok(())
    }

    /// Sets the time to live of sent datagrams. Zero is refused by the model.
    pub fn set_time_to_live(&mut self, ttl: u8) -> Result<(), GroupError> {
        let mut next = self.membership.clone();
        next.set_time_to_live(ttl)?;
        self.socket
            .set_multicast_ttl_v4(u32::from(ttl))
            .map_err(|e| GroupError::Socket(e.to_string()))?;
        self.membership = next;
        Ok(())
    }

    /// Turns loopback of sent datagrams to the local members on or off.
    pub fn set_loopback(&mut self, on: bool) -> Result<(), GroupError> {
        self.socket.set_multicast_loop_v4(on).map_err(|e| GroupError::Socket(e.to_string()))?;
        self.membership.set_loopback(on);
        Ok(())
    }

    /// Sends one datagram to `group` on `port`. Refuses an address outside 224.0.0.0/4.
    pub fn send(&self, data: &[u8], group: Ipv4Addr, port: u16) -> Result<usize, GroupError> {
        if !group.is_multicast() {
            return Err(MembershipError::NotMulticast.into());
        }
        self.socket
            .send_to(data, SocketAddr::V4(SocketAddrV4::new(group, port)))
            .map_err(|e| GroupError::Socket(e.to_string()))
    }

    /// Returns the next received datagram, or `None` when none is waiting. Never blocks.
    pub fn receive(&self) -> Result<Option<Datagram>, GroupError> {
        let mut buf = vec![0u8; 65_536];
        match self.socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                buf.truncate(n);
                Ok(Some(Datagram { from, data: buf }))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(GroupError::Socket(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_groups_share_one_port() {
        let first = ConnectionGroup::new(0).unwrap();
        let second = ConnectionGroup::new(first.local_port()).unwrap();
        assert_eq!(second.local_port(), first.local_port());
    }

    #[test]
    fn a_new_group_binds_a_port_with_default_options() {
        let g = ConnectionGroup::new(0).unwrap();
        assert_ne!(g.local_port(), 0);
        assert!(g.membership().groups().is_empty());
        assert_eq!(g.membership().time_to_live(), 1);
        assert!(g.membership().loopback());
    }

    #[test]
    fn joining_a_unicast_address_is_refused_before_the_socket() {
        let mut g = ConnectionGroup::new(0).unwrap();
        let err = g.join(Ipv4Addr::new(192, 168, 1, 5)).unwrap_err();
        assert!(matches!(err, GroupError::Membership(MembershipError::NotMulticast)));
        assert!(g.membership().groups().is_empty());
    }

    #[test]
    fn leaving_a_group_that_is_not_joined_is_refused() {
        let mut g = ConnectionGroup::new(0).unwrap();
        let err = g.leave(Ipv4Addr::new(239, 255, 20, 20)).unwrap_err();
        assert!(matches!(err, GroupError::Membership(MembershipError::NotJoined)));
    }

    #[test]
    fn a_zero_time_to_live_is_refused_and_leaves_the_option_unchanged() {
        let mut g = ConnectionGroup::new(0).unwrap();
        assert!(matches!(g.set_time_to_live(0), Err(GroupError::Membership(MembershipError::ZeroTimeToLive))));
        assert_eq!(g.membership().time_to_live(), 1);
        g.set_time_to_live(4).unwrap();
        assert_eq!(g.membership().time_to_live(), 4);
    }

    #[test]
    fn sending_to_a_unicast_address_is_refused() {
        let g = ConnectionGroup::new(0).unwrap();
        let err = g.send(b"hi", Ipv4Addr::LOCALHOST, 9).unwrap_err();
        assert!(matches!(err, GroupError::Membership(MembershipError::NotMulticast)));
    }

    #[test]
    fn receive_returns_none_when_nothing_is_waiting() {
        let g = ConnectionGroup::new(0).unwrap();
        assert!(g.receive().unwrap().is_none());
    }

    #[test]
    fn loopback_setting_is_kept_in_the_model() {
        let mut g = ConnectionGroup::new(0).unwrap();
        g.set_loopback(false).unwrap();
        assert!(!g.membership().loopback());
    }
}
