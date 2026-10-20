//! Screen Sharing: the files behind `bifrost-wayvnc.service`, as pure text so they can be tested.
//!
//! Settings > Sharing > Screen Sharing starts wayvnc with a configuration that privd writes here. The server is never
//! open without a login and is never unencrypted:
//! - `enable_auth=true` with `enable_pam=true`: a viewer signs in with an account name and password that PAM checks
//!   (service `wayvnc`, `packaging/arch/pam-bifrost-wayvnc`), and that file only lets in the one account the person
//!   picked (`USERS`, read with `pam_listfile`);
//! - `rsa_private_key_file`: the RSA-AES security type, with a key generated once on this computer (`KEY`), so the
//!   password and the screen are encrypted on the wire;
//! - `address` is this computer's local-network address, not `0.0.0.0` and not a public interface;
//! - the unit is not enabled until Settings turns Screen Sharing on, and it needs these files to start at all;
//! - while it is off the configuration is rewritten to the loopback address, so even a manual start is not on the network;
//! - the server runs as the account that may sign in, never as root: privd hands that account the key, the configuration
//!   and the lockout directory, and writes a unit drop-in naming it (`unit_dropin_text`). Only the signed-in account's own
//!   password can be checked by a non-root process, which is why the account that signs in is the account that runs it.
use std::net::Ipv4Addr;

/// Root-owned state of the feature. The directory is readable so Settings can show the account; the key is not.
pub const DIR: &str = "/var/lib/springboard/screen-sharing";
/// The wayvnc configuration `bifrost-wayvnc.service` reads.
pub const CONF: &str = "/var/lib/springboard/screen-sharing/wayvnc.conf";
/// The per-computer RSA key (PEM), mode 0600, never leaves this computer.
pub const KEY: &str = "/var/lib/springboard/screen-sharing/rsa_key.pem";
/// The one account name that may sign in, one line (`pam_listfile item=user`).
pub const USERS: &str = "/var/lib/springboard/screen-sharing/users";
/// The sign-in lockout counters (`pam_faillock dir=`), written by the server's own user, which cannot write `/var/lib/faillock`.
pub const FAILLOCK_DIR: &str = "/var/lib/springboard/screen-sharing/faillock";
/// The drop-in that gives `bifrost-wayvnc.service` its `User=`; the unit does not start without it.
pub const DROPIN_DIR: &str = "/etc/systemd/system/bifrost-wayvnc.service.d";
pub const DROPIN: &str = "/etc/systemd/system/bifrost-wayvnc.service.d/10-user.conf";
/// The address the server is bound to while Screen Sharing is off.
pub const LOOPBACK: &str = "127.0.0.1";
/// The VNC port.
pub const PORT: u16 = 5900;
/// Bits of the generated RSA key.
pub const KEY_BITS: u32 = 3072;

/// The wayvnc configuration: authentication and encryption on, bound to `address`.
pub fn config_text(address: &str, port: u16, key_file: &str) -> String {
    format!(
        "# Written by bifrost-privd (screen_sharing.configure); changes are overwritten.\n\
         address={address}\n\
         port={port}\n\
         enable_auth=true\n\
         enable_pam=true\n\
         rsa_private_key_file={key_file}\n"
    )
}

/// The drop-in text that makes the server run as `user`.
pub fn unit_dropin_text(user: &str) -> String {
    format!("# Written by bifrost-privd (screen_sharing.configure); changes are overwritten.\n[Service]\nUser={user}\n")
}

/// Whether the feature directory may be trusted before files in it are handed to a user: a real directory (not a
/// link), owned by root, writable by nobody else.
pub fn dir_trusted(is_dir: bool, uid: u32, mode: u32) -> bool {
    is_dir && uid == 0 && mode & 0o022 == 0
}

/// The allow list `pam_listfile` reads: the one account that may sign in.
pub fn users_text(user: &str) -> String {
    format!("{user}\n")
}

/// The account in an allow list text, if it holds one.
pub fn user_from(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// The argv that makes the RSA key: PEM format, no passphrase (the unit has nobody to ask), written once.
pub fn keygen_argv(path: &str) -> Vec<String> {
    ["ssh-keygen", "-q", "-m", "pem", "-t", "rsa", "-b", &KEY_BITS.to_string(), "-N", "", "-f", path].iter().map(|s| s.to_string()).collect()
}

/// Whether `a` may be the bind address: one IPv4 address of a network, never all interfaces, loopback or multicast.
pub fn valid_address(a: &str) -> bool {
    a.parse::<Ipv4Addr>().is_ok_and(|ip| !ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast() && !ip.is_broadcast())
}

/// Interfaces that are not the local network: containers, virtual bridges and tunnels.
fn virtual_interface(name: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "tun", "tap", "tailscale", "wg", "lo"].iter().any(|p| name.starts_with(p))
}

/// This computer's local-network address from `ip -4 -o addr show scope global`: the first address of a real interface,
/// preferring a private (RFC 1918) one. None when there is none, and Screen Sharing then stays off.
pub fn lan_address(ip_output: &str) -> Option<String> {
    let mut found: Vec<(bool, String)> = Vec::new();
    for line in ip_output.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        // "2: enp3s0    inet 192.168.1.12/24 brd 192.168.1.255 scope global dynamic enp3s0"
        let (Some(name), Some(at)) = (f.get(1), f.iter().position(|w| *w == "inet")) else { continue };
        let Some(addr) = f.get(at + 1).and_then(|a| a.split('/').next()) else { continue };
        if virtual_interface(name.trim_end_matches(':')) || !valid_address(addr) {
            continue;
        }
        let private = addr.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_private());
        found.push((private, addr.to_string()));
    }
    found.iter().find(|(private, _)| *private).or_else(|| found.first()).map(|(_, a)| a.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_requires_a_login_and_encrypts() {
        let c = config_text("192.168.1.12", PORT, KEY);
        assert!(c.contains("address=192.168.1.12\n") && c.contains("port=5900\n"));
        assert!(c.contains("enable_auth=true\n") && c.contains("enable_pam=true\n"));
        assert!(c.contains(&format!("rsa_private_key_file={KEY}\n")));
        assert!(!c.contains("enable_auth=false") && !c.contains("0.0.0.0") && !c.contains("relax_encryption"));
    }

    #[test]
    fn the_server_runs_as_the_account_and_is_loopback_while_off() {
        assert_eq!(unit_dropin_text("olof"), "# Written by bifrost-privd (screen_sharing.configure); changes are overwritten.\n[Service]\nUser=olof\n");
        assert!(!unit_dropin_text("olof").contains("User=root"));
        let off = config_text(LOOPBACK, PORT, KEY);
        assert!(off.contains("address=127.0.0.1\n") && off.contains("enable_auth=true\n"));
        assert!(!valid_address(LOOPBACK), "loopback is for the off state only, never a sharing address");
        assert!(FAILLOCK_DIR.starts_with(DIR));
    }

    #[test]
    fn the_unit_and_the_pam_service_match_the_runtime_files() {
        let unit = include_str!("../../../packaging/arch/bifrost-wayvnc.service");
        assert!(unit.contains(&format!("ConditionPathExists={DROPIN}")), "no start without the User= drop-in");
        assert!(!unit.lines().any(|l| l.trim() == "User=root"));
        assert!(unit.contains("ProtectSystem=strict") && unit.contains("CapabilityBoundingSet=\n"));
        assert!(unit.contains(FAILLOCK_DIR), "the lockout directory is writable for the sandboxed server");
        let pam = include_str!("../../../packaging/arch/pam-bifrost-wayvnc");
        assert!(pam.contains(&format!("dir={FAILLOCK_DIR}")) && !pam.contains("dir=/var/lib/faillock"));
    }

    #[test]
    fn files_are_handed_over_only_from_a_root_owned_directory_nobody_else_writes() {
        assert!(dir_trusted(true, 0, 0o755));
        assert!(!dir_trusted(false, 0, 0o755), "a link or a file");
        assert!(!dir_trusted(true, 1000, 0o755), "owned by a user: it could swap a file for a link");
        assert!(!dir_trusted(true, 0, 0o775) && !dir_trusted(true, 0, 0o757));
    }

    #[test]
    fn the_allow_list_holds_one_account() {
        assert_eq!(users_text("olof"), "olof\n");
        assert_eq!(user_from(&users_text("olof")).as_deref(), Some("olof"));
        assert_eq!(user_from("\n  \n"), None);
    }

    #[test]
    fn the_key_is_a_pem_rsa_key_without_a_passphrase_at_the_given_path() {
        let a = keygen_argv("/k/rsa_key.pem");
        assert_eq!(a[0], "ssh-keygen");
        let pair = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(pair("-m", "pem") && pair("-t", "rsa") && pair("-N", ""));
        assert_eq!(a.last().unwrap(), "/k/rsa_key.pem");
    }

    #[test]
    fn only_a_network_address_can_be_bound() {
        assert!(valid_address("192.168.1.12") && valid_address("10.99.0.1"));
        for bad in ["0.0.0.0", "127.0.0.1", "224.0.0.1", "255.255.255.255", "::1", "host", "", "192.168.1.12/24"] {
            assert!(!valid_address(bad), "{bad}");
        }
    }

    #[test]
    fn the_local_network_address_skips_containers_and_prefers_a_private_one() {
        let out = "2: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\n\
                   3: wlan0    inet 203.0.113.9/24 brd 203.0.113.255 scope global dynamic wlan0\n\
                   4: enp3s0    inet 192.168.1.12/24 brd 192.168.1.255 scope global dynamic enp3s0\n\
                   5: tailscale0    inet 100.64.0.1/32 scope global tailscale0\n";
        assert_eq!(lan_address(out).as_deref(), Some("192.168.1.12"));
        assert_eq!(lan_address("5: tailscale0    inet 100.64.0.1/32 scope global tailscale0\n"), None);
        assert_eq!(lan_address("3: wlan0    inet 203.0.113.9/24 scope global wlan0\n").as_deref(), Some("203.0.113.9"));
        assert_eq!(lan_address(""), None);
    }
}
