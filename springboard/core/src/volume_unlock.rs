//! Encrypted volumes that are still locked after login: which LUKS devices have no open mapping, and the unlock request
//! the password prompt answers with.
//!
//! Pure. The caller reads `lsblk -rpno NAME,TYPE,FSTYPE,PKNAME` and passes its text. A LUKS container (`crypto_LUKS`)
//! is locked when no `crypt` device has it as its parent, which is what an open mapping looks like in that listing.
//! The privileged helper opens a volume with the typed password on stdin; this module only decides what is locked and
//! checks the names it will pass to the command.

/// The privileged method that opens a locked volume with a password.
pub const M_UNLOCK_VOLUME: &str = "disk.unlock_volume";

/// One line of the device listing, with the fields this module reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub kind: String,
    pub fstype: String,
    pub parent: String,
}

/// Parses `lsblk -rpno NAME,TYPE,FSTYPE,PKNAME` output. Short or blank lines are skipped; a missing field is empty.
pub fn parse(listing: &str) -> Vec<Row> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.to_string();
            Some(Row {
                name,
                kind: fields.next().unwrap_or("").to_string(),
                fstype: fields.next().unwrap_or("").to_string(),
                parent: fields.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// The LUKS devices with no open mapping, in listing order.
pub fn locked(listing: &str) -> Vec<String> {
    let rows = parse(listing);
    rows.iter()
        .filter(|r| r.fstype == "crypto_LUKS")
        .filter(|r| !rows.iter().any(|m| m.kind == "crypt" && m.parent == r.name))
        .map(|r| r.name.clone())
        .collect()
}

/// Why a device name cannot be unlocked, or None when it may be passed to the helper. Only `/dev/` paths made of
/// letters, digits, `/`, `-`, `_` and `.` are accepted, so the name cannot carry arguments or escape `/dev`.
pub fn check_device(device: &str) -> Result<(), &'static str> {
    if !device.starts_with("/dev/") || device.len() <= "/dev/".len() {
        return Err("the volume must be a /dev path");
    }
    if device.contains("..") || !device.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.')) {
        return Err("the volume name has characters the helper does not accept");
    }
    Ok(())
}

/// The name the opened mapping gets: `bifrost-` plus the device's last path part with other characters replaced.
pub fn mapper_name(device: &str) -> String {
    let base = device.rsplit('/').next().unwrap_or("volume");
    let clean: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("bifrost-{clean}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "/dev/sda1 part vfat \n\
/dev/sda2 part crypto_LUKS \n\
/dev/sdb1 part crypto_LUKS \n\
/dev/mapper/cryptdata crypt ext4 /dev/sdb1\n\
/dev/sdc disk \n";

    #[test]
    fn a_luks_container_with_no_mapping_is_locked() {
        assert_eq!(locked(LISTING), vec!["/dev/sda2".to_string()]);
    }

    #[test]
    fn an_open_mapping_means_the_container_is_not_locked() {
        let text = "/dev/sdb1 part crypto_LUKS\n/dev/mapper/x crypt ext4 /dev/sdb1\n";
        assert!(locked(text).is_empty());
    }

    #[test]
    fn short_and_blank_lines_are_skipped() {
        let rows = parse("\n/dev/sdz\n\n/dev/sdy part\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fstype, "");
        assert_eq!(rows[1].kind, "part");
    }

    #[test]
    fn device_names_are_checked_before_use() {
        assert!(check_device("/dev/sda2").is_ok());
        assert!(check_device("/dev/nvme0n1p3").is_ok());
        assert!(check_device("sda2").is_err());
        assert!(check_device("/dev/").is_err());
        assert!(check_device("/dev/../etc/shadow").is_err());
        assert!(check_device("/dev/sda2 --batch").is_err());
    }

    #[test]
    fn the_mapping_name_is_a_safe_word() {
        assert_eq!(mapper_name("/dev/sda2"), "bifrost-sda2");
        assert_eq!(mapper_name("/dev/disk/by-uuid/abc:def"), "bifrost-abc_def");
    }
}
