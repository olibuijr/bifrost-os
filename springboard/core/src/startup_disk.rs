//! Startup disk: which firmware boot entry the computer starts from. Reads the `efibootmgr` listing, tells the entries
//! in boot order, and builds the fixed arguments that make one entry the default (`BootOrder`) or the next boot only
//! (`BootNext`).
//!
//! Rules of this model:
//! - An entry number is four hex digits (`0003`). Only an entry that the listing shows and that is active can be chosen,
//!   so a number typed by a caller can never reach the command line unless the firmware lists it.
//! - Making an entry the default moves it to the front of `BootOrder` and keeps the others in their order; an entry that
//!   is not in `BootOrder` is added at the end after it.
//! - Labels are shown as the firmware gives them, without control characters, trimmed and cut at 64 characters.
//!
//! The listing comes from running `efibootmgr` (root). This module runs nothing: `bifrost-privd` runs the argv it
//! builds (`startup_disk.list`, `startup_disk.set`).

/// The most characters of a label kept.
pub const MAX_LABEL: usize = 64;

/// One firmware boot entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootEntry {
    pub num: u16,
    pub label: String,
    /// False for an entry the firmware lists without `*` (it will not be started).
    pub active: bool,
}

impl BootEntry {
    /// The entry number as the firmware prints it: four uppercase hex digits.
    pub fn id(&self) -> String {
        format!("{:04X}", self.num)
    }
}

/// The boot variables of the firmware.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootTable {
    /// The entry the computer started from now (`BootCurrent`).
    pub current: Option<u16>,
    /// The entry chosen for the next start only (`BootNext`).
    pub next: Option<u16>,
    /// The order the firmware tries entries in (`BootOrder`).
    pub order: Vec<u16>,
    /// Every entry, in the order the listing gave them.
    pub entries: Vec<BootEntry>,
}

/// Why a choice was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChooseError {
    /// The text is not four hex digits.
    BadNumber,
    /// The firmware lists no such entry.
    Unknown(u16),
    /// The entry is listed but inactive.
    Inactive(u16),
}

fn hex4(s: &str) -> Option<u16> {
    if s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        u16::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

fn clean_label(raw: &str) -> String {
    // `efibootmgr -v` appends a tab and the device path; the label is what comes before the tab
    let text: String = raw.split('\t').next().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    text.trim().chars().take(MAX_LABEL).collect()
}

impl BootTable {
    /// Reads `efibootmgr` output. Lines it does not know are skipped, and so is a `Boot` line without a four-digit
    /// number, so a damaged listing gives fewer entries and never a wrong one.
    pub fn parse(text: &str) -> BootTable {
        let mut t = BootTable::default();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("BootCurrent:") {
                t.current = hex4(v.trim());
            } else if let Some(v) = line.strip_prefix("BootNext:") {
                t.next = hex4(v.trim());
            } else if let Some(v) = line.strip_prefix("BootOrder:") {
                t.order = v.split(',').filter_map(|n| hex4(n.trim())).collect();
            } else if let Some(rest) = line.strip_prefix("Boot") {
                let Some(num) = rest.get(..4).and_then(hex4) else { continue };
                let after = &rest[4..];
                let (active, label) = match after.chars().next() {
                    Some('*') => (true, &after[1..]),
                    Some(' ') => (false, after),
                    _ => continue,
                };
                t.entries.push(BootEntry { num, label: clean_label(label), active });
            }
        }
        t
    }

    pub fn entry(&self, num: u16) -> Option<&BootEntry> {
        self.entries.iter().find(|e| e.num == num)
    }

    /// The entries as the Startup Disk list shows them: those in `BootOrder` first, in that order, then the others.
    pub fn in_boot_order(&self) -> Vec<&BootEntry> {
        let mut out: Vec<&BootEntry> = self.order.iter().filter_map(|&n| self.entry(n)).collect();
        out.extend(self.entries.iter().filter(|e| !self.order.contains(&e.num)));
        out
    }

    /// The entry the computer starts first: the first active entry in `BootOrder`.
    pub fn default_entry(&self) -> Option<&BootEntry> {
        self.order.iter().filter_map(|&n| self.entry(n)).find(|e| e.active)
    }

    fn choose(&self, id: &str) -> Result<u16, ChooseError> {
        let num = hex4(id).ok_or(ChooseError::BadNumber)?;
        match self.entry(num) {
            None => Err(ChooseError::Unknown(num)),
            Some(e) if !e.active => Err(ChooseError::Inactive(num)),
            Some(_) => Ok(num),
        }
    }

    /// The `BootOrder` after `num` is made the default.
    pub fn order_with_default(&self, num: u16) -> Vec<u16> {
        let mut order = vec![num];
        order.extend(self.order.iter().copied().filter(|&n| n != num));
        order
    }

    /// The argv that makes entry `id` the default start (`efibootmgr -o`), after checking it against the listing.
    pub fn set_default_argv(&self, id: &str) -> Result<Vec<String>, ChooseError> {
        let num = self.choose(id)?;
        let order: Vec<String> = self.order_with_default(num).iter().map(|n| format!("{n:04X}")).collect();
        Ok(vec!["efibootmgr".into(), "-o".into(), order.join(",")])
    }

    /// The argv that starts entry `id` on the next start only (`efibootmgr -n`).
    pub fn boot_once_argv(&self, id: &str) -> Result<Vec<String>, ChooseError> {
        let num = self.choose(id)?;
        Ok(vec!["efibootmgr".into(), "-n".into(), format!("{num:04X}")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "BootCurrent: 0003\nTimeout: 1 seconds\nBootOrder: 0003,0001,0000\n\
Boot0000* UEFI: Built-in EFI Shell\tVenMedia(5023b95c)\n\
Boot0001* Windows Boot Manager\tHD(1,GPT,abc)/File(\\EFI\\Microsoft\\bootmgfw.efi)\n\
Boot0003* BifrOSt\tHD(1,GPT,def)/File(\\EFI\\BIFROST\\BOOTx64.efi)\n\
Boot0004  Old Entry\n\
Boot00ZZ* broken\n\
BootNext: 0001\n";

    #[test]
    fn the_listing_gives_the_variables_and_the_entries() {
        let t = BootTable::parse(LISTING);
        assert_eq!((t.current, t.next), (Some(3), Some(1)));
        assert_eq!(t.order, [3, 1, 0]);
        assert_eq!(t.entries.len(), 4, "the broken line is skipped");
        assert_eq!(t.entry(3).unwrap().label, "BifrOSt");
        assert_eq!(t.entry(0).unwrap().label, "UEFI: Built-in EFI Shell");
        assert!(!t.entry(4).unwrap().active);
        assert_eq!(t.entry(4).unwrap().id(), "0004");
    }

    #[test]
    fn entries_list_in_boot_order_then_the_rest() {
        let t = BootTable::parse(LISTING);
        let ids: Vec<String> = t.in_boot_order().iter().map(|e| e.id()).collect();
        assert_eq!(ids, ["0003", "0001", "0000", "0004"]);
        assert_eq!(t.default_entry().unwrap().label, "BifrOSt");
    }

    #[test]
    fn making_an_entry_the_default_moves_it_first_and_keeps_the_rest() {
        let t = BootTable::parse(LISTING);
        assert_eq!(t.set_default_argv("0001").unwrap(), ["efibootmgr", "-o", "0001,0003,0000"]);
        assert_eq!(t.set_default_argv("0003").unwrap(), ["efibootmgr", "-o", "0003,0001,0000"]);
        assert_eq!(t.set_default_argv("0000").unwrap(), ["efibootmgr", "-o", "0000,0003,0001"]);
    }

    #[test]
    fn boot_once_names_the_entry_in_four_uppercase_digits() {
        let mut t = BootTable::parse(LISTING);
        t.entries.push(BootEntry { num: 0x00AB, label: "X".into(), active: true });
        assert_eq!(t.boot_once_argv("00ab").unwrap(), ["efibootmgr", "-n", "00AB"]);
    }

    #[test]
    fn only_listed_active_entries_can_be_chosen() {
        let t = BootTable::parse(LISTING);
        assert_eq!(t.set_default_argv("0009"), Err(ChooseError::Unknown(9)));
        assert_eq!(t.set_default_argv("0004"), Err(ChooseError::Inactive(4)));
        for bad in ["", "3", "00003", "0x03", "0003;reboot", "-o", "../etc"] {
            assert_eq!(t.boot_once_argv(bad), Err(ChooseError::BadNumber), "{bad}");
        }
    }

    #[test]
    fn labels_lose_control_characters_and_are_cut() {
        let long = format!("Boot0007* {}\tPCI()\n", "x".repeat(200));
        let t = BootTable::parse(&format!("Boot0006* A\u{1b}[31mB\tpath\n{long}"));
        assert_eq!(t.entry(6).unwrap().label, "A[31mB");
        assert_eq!(t.entry(7).unwrap().label.chars().count(), MAX_LABEL);
        assert_eq!(BootTable::parse("").entries.len(), 0);
    }
}
