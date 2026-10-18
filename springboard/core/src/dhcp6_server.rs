//! A DHCPv6 server for one network segment, served by dnsmasq: the lease range, the name servers and the router
//! advertisement a shared connection hands to the computers behind it. This module checks the settings and writes the
//! dnsmasq configuration; `bifrost-privd` (`dhcp6.serve`) stores it and starts `bifrost-dhcp6@<interface>.service`.
//!
//! Rules of this model:
//! - The interface is a Linux interface name: 1 to 15 characters of letters, digits, `_`, `-` and `.`, not `.` or `..`.
//! - The segment is a /64. `prefix` is its network address (the low 64 bits must be zero); leases come from
//!   `prefix + first` to `prefix + last`, both inside the /64, `first <= last`, at most [`MAX_RANGE`] addresses.
//! - A lease lasts [`MIN_LEASE_SECS`] to [`MAX_LEASE_SECS`] seconds.
//! - Name servers are IPv6 addresses, at most [`MAX_DNS`]. Link-local and unspecified addresses are refused: a client
//!   cannot be told to ask a resolver it cannot route to.
//! - DNS service of dnsmasq itself is off (`port=0`): this is the address-assignment half of a shared connection.
//!
//! Nothing here opens a socket or runs a program, so it holds no secret and sends nothing off the computer.

use std::net::Ipv6Addr;

/// Shortest lease, in seconds.
pub const MIN_LEASE_SECS: u32 = 120;
/// Longest lease, in seconds (7 days).
pub const MAX_LEASE_SECS: u32 = 7 * 24 * 3600;
/// Most addresses one range may hold.
pub const MAX_RANGE: u64 = 65_536;
/// Most name servers announced.
pub const MAX_DNS: usize = 3;
/// Directory (under `/run`) the configuration and lease files live in.
pub const RUN_DIR: &str = "/run/bifrost/dhcp6";

/// Why a setting was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dhcp6Error {
    BadInterface,
    /// The prefix is not a /64 network address.
    BadPrefix,
    BadRange,
    BadLease,
    BadDns,
}

/// The settings of one served segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dhcp6Server {
    pub interface: String,
    /// The segment's network address; the low 64 bits are zero.
    pub prefix: Ipv6Addr,
    /// First and last host part of the lease range (the low 64 bits of the address).
    pub first: u64,
    pub last: u64,
    pub lease_secs: u32,
    pub dns: Vec<Ipv6Addr>,
}

/// True for a usable interface name.
pub fn valid_interface(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name != "."
        && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl Dhcp6Server {
    /// A segment with the usual range (`::100` to `::1ff`), a 12 hour lease and no name servers.
    pub fn new(interface: &str, prefix: Ipv6Addr) -> Result<Dhcp6Server, Dhcp6Error> {
        let s = Dhcp6Server { interface: interface.into(), prefix, first: 0x100, last: 0x1ff, lease_secs: 12 * 3600, dns: Vec::new() };
        s.validate()?;
        Ok(s)
    }

    /// Checks every setting against the rules above.
    pub fn validate(&self) -> Result<(), Dhcp6Error> {
        if !valid_interface(&self.interface) {
            return Err(Dhcp6Error::BadInterface);
        }
        if u128::from(self.prefix) as u64 != 0 || self.prefix.is_unspecified() {
            return Err(Dhcp6Error::BadPrefix);
        }
        if self.first == 0 || self.first > self.last || self.last - self.first >= MAX_RANGE {
            return Err(Dhcp6Error::BadRange);
        }
        if !(MIN_LEASE_SECS..=MAX_LEASE_SECS).contains(&self.lease_secs) {
            return Err(Dhcp6Error::BadLease);
        }
        let link_local = |a: &Ipv6Addr| (a.segments()[0] & 0xffc0) == 0xfe80;
        if self.dns.len() > MAX_DNS || self.dns.iter().any(|a| a.is_unspecified() || a.is_multicast() || link_local(a)) {
            return Err(Dhcp6Error::BadDns);
        }
        Ok(())
    }

    fn at(&self, host: u64) -> Ipv6Addr {
        Ipv6Addr::from(u128::from(self.prefix) | host as u128)
    }

    /// The first and last address of the lease range.
    pub fn range(&self) -> (Ipv6Addr, Ipv6Addr) {
        (self.at(self.first), self.at(self.last))
    }

    /// The path of this segment's configuration file.
    pub fn conf_path(&self) -> String {
        format!("{RUN_DIR}/{}.conf", self.interface)
    }

    /// The dnsmasq configuration: DHCPv6 leases on the interface and router advertisements that tell clients to ask
    /// for them. Refuses settings that do not validate.
    pub fn dnsmasq_conf(&self) -> Result<String, Dhcp6Error> {
        self.validate()?;
        let (a, b) = self.range();
        let mut out = String::from("# written by bifrost-privd (dhcp6.serve); changes are overwritten\n");
        out += "port=0\nbind-interfaces\nno-hosts\nno-resolv\n";
        out += &format!("interface={}\n", self.interface);
        out += &format!("dhcp-range={a},{b},64,{}\n", self.lease_secs);
        out += &format!("dhcp-leasefile={RUN_DIR}/{}.leases\n", self.interface);
        out += "enable-ra\n";
        out += &format!("ra-param={},60,1800\n", self.interface);
        if !self.dns.is_empty() {
            let list: Vec<String> = self.dns.iter().map(|d| format!("[{d}]")).collect();
            out += &format!("dhcp-option=option6:dns-server,{}\n", list.join(","));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net() -> Ipv6Addr {
        "fd42:1:2:3::".parse().unwrap()
    }

    #[test]
    fn a_default_segment_leases_from_100_to_1ff_for_twelve_hours() {
        let s = Dhcp6Server::new("eth1", net()).unwrap();
        let (a, b) = s.range();
        assert_eq!((a.to_string(), b.to_string()), ("fd42:1:2:3::100".to_string(), "fd42:1:2:3::1ff".to_string()));
        let conf = s.dnsmasq_conf().unwrap();
        assert!(conf.contains("interface=eth1\n"));
        assert!(conf.contains("dhcp-range=fd42:1:2:3::100,fd42:1:2:3::1ff,64,43200\n"));
        assert!(conf.contains("port=0\n") && conf.contains("enable-ra\n") && conf.contains("ra-param=eth1,60,1800\n"));
        assert!(conf.contains("dhcp-leasefile=/run/bifrost/dhcp6/eth1.leases\n"));
        assert!(!conf.contains("dns-server"), "no name server announced unless one is set");
        assert_eq!(s.conf_path(), "/run/bifrost/dhcp6/eth1.conf");
    }

    #[test]
    fn name_servers_are_announced_in_brackets() {
        let mut s = Dhcp6Server::new("br0", net()).unwrap();
        s.dns = vec!["fd42:1:2:3::1".parse().unwrap(), "2606:4700:4700::1111".parse().unwrap()];
        assert!(s.dnsmasq_conf().unwrap().contains("dhcp-option=option6:dns-server,[fd42:1:2:3::1],[2606:4700:4700::1111]\n"));
    }

    #[test]
    fn interface_names_cannot_carry_config_lines_or_paths() {
        for bad in ["", "a b", "eth0\ninterface=lo", "../x", "a/b", "..", ".", "averyveryverylongname", "eth0;x"] {
            assert!(!valid_interface(bad), "{bad:?}");
            assert_eq!(Dhcp6Server::new(bad, net()), Err(Dhcp6Error::BadInterface), "{bad:?}");
        }
        assert!(valid_interface("enp3s0") && valid_interface("wlan0.10") && valid_interface("br-lan_1"));
    }

    #[test]
    fn the_prefix_must_be_a_64_network_address() {
        assert_eq!(Dhcp6Server::new("eth1", "fd42:1:2:3::5".parse().unwrap()), Err(Dhcp6Error::BadPrefix));
        assert_eq!(Dhcp6Server::new("eth1", Ipv6Addr::UNSPECIFIED), Err(Dhcp6Error::BadPrefix));
    }

    #[test]
    fn the_range_lease_and_name_servers_are_bounded() {
        let ok = Dhcp6Server::new("eth1", net()).unwrap();
        let with = |f: &dyn Fn(&mut Dhcp6Server)| {
            let mut s = ok.clone();
            f(&mut s);
            s.validate()
        };
        assert_eq!(with(&|s| s.first = 0), Err(Dhcp6Error::BadRange));
        assert_eq!(with(&|s| (s.first, s.last) = (9, 8)), Err(Dhcp6Error::BadRange));
        assert_eq!(with(&|s| s.last = s.first + MAX_RANGE), Err(Dhcp6Error::BadRange));
        assert_eq!(with(&|s| s.last = s.first + MAX_RANGE - 1), Ok(()));
        assert_eq!(with(&|s| s.lease_secs = 60), Err(Dhcp6Error::BadLease));
        assert_eq!(with(&|s| s.lease_secs = MAX_LEASE_SECS + 1), Err(Dhcp6Error::BadLease));
        assert_eq!(with(&|s| s.dns = vec!["fe80::1".parse().unwrap()]), Err(Dhcp6Error::BadDns));
        assert_eq!(with(&|s| s.dns = vec![Ipv6Addr::UNSPECIFIED]), Err(Dhcp6Error::BadDns));
        assert_eq!(with(&|s| s.dns = vec!["ff02::1".parse().unwrap()]), Err(Dhcp6Error::BadDns));
        assert_eq!(with(&|s| s.dns = vec!["fd00::1".parse().unwrap(); 4]), Err(Dhcp6Error::BadDns));
    }

    #[test]
    fn invalid_settings_never_produce_a_config() {
        let mut s = Dhcp6Server::new("eth1", net()).unwrap();
        s.interface = "x\ny".into();
        assert_eq!(s.dnsmasq_conf(), Err(Dhcp6Error::BadInterface));
    }
}
