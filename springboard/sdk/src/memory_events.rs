//! Memory mapping and protection events: the records a monitor gets when a process maps a file into memory, and when it
//! changes the protection of mapped pages.
//!
//! A map event carries the protection in force, the largest protection the mapping may later take, the mapping flags,
//! the file offset and the source file. A protection event carries the start address, the length and the new
//! protection. The model checks what the kernel checks on these fields: the protection must fit inside the maximum,
//! the length must not be zero, and the range must not wrap past the top of the address space.
//!
//! The monitor is notify-only. It records each event and reports a finding when a page is left both writable and
//! executable, which is the condition a code-injection check looks for. No shell path produces these events yet, and no
//! mapping is blocked by this module.
//!
//! Protection bits are the POSIX `PROT_READ` (1), `PROT_WRITE` (2) and `PROT_EXEC` (4) values.

use std::fmt;

/// A set of page protection bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Protection(u32);

impl Protection {
    pub const NONE: Protection = Protection(0);
    pub const READ: Protection = Protection(1);
    pub const WRITE: Protection = Protection(2);
    pub const EXEC: Protection = Protection(4);

    /// The protection with these bits, or `None` when a bit outside read, write and exec is set.
    pub fn from_bits(bits: u32) -> Option<Protection> {
        if bits & !7 == 0 {
            Some(Protection(bits))
        } else {
            None
        }
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn readable(self) -> bool {
        self.0 & 1 != 0
    }

    pub fn writable(self) -> bool {
        self.0 & 2 != 0
    }

    pub fn executable(self) -> bool {
        self.0 & 4 != 0
    }

    /// The bits of both protections.
    pub fn union(self, other: Protection) -> Protection {
        Protection(self.0 | other.0)
    }

    /// True when every bit of `self` is also set in `max`.
    pub fn fits_within(self, max: Protection) -> bool {
        self.0 & !max.0 == 0
    }

    /// True when the pages are writable and executable at once.
    pub fn is_writable_and_executable(self) -> bool {
        self.writable() && self.executable()
    }
}

/// Why an event is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// The protection asks for more than the maximum allows.
    ProtectionExceedsMaximum,
    /// The map event names no source file.
    EmptySource,
    /// The protection range has zero length.
    EmptyRange,
    /// The start address plus the length passes the top of the address space.
    RangeOverflow,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MemoryError::ProtectionExceedsMaximum => "protection is larger than the maximum protection",
            MemoryError::EmptySource => "map event has no source file",
            MemoryError::EmptyRange => "protection range has zero length",
            MemoryError::RangeOverflow => "protection range passes the end of the address space",
        })
    }
}

impl std::error::Error for MemoryError {}

/// A file mapped into memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEvent {
    protection: Protection,
    max_protection: Protection,
    flags: u32,
    file_pos: u64,
    source: String,
}

impl MapEvent {
    pub fn new(
        protection: Protection,
        max_protection: Protection,
        flags: u32,
        file_pos: u64,
        source: &str,
    ) -> Result<MapEvent, MemoryError> {
        if source.is_empty() {
            return Err(MemoryError::EmptySource);
        }
        if !protection.fits_within(max_protection) {
            return Err(MemoryError::ProtectionExceedsMaximum);
        }
        Ok(MapEvent { protection, max_protection, flags, file_pos, source: source.to_string() })
    }

    pub fn protection(&self) -> Protection {
        self.protection
    }

    pub fn max_protection(&self) -> Protection {
        self.max_protection
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn file_pos(&self) -> u64 {
        self.file_pos
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A change of protection on a range of mapped pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtectEvent {
    address: u64,
    size: u64,
    protection: Protection,
}

impl ProtectEvent {
    pub fn new(address: u64, size: u64, protection: Protection) -> Result<ProtectEvent, MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyRange);
        }
        if address.checked_add(size).is_none() {
            return Err(MemoryError::RangeOverflow);
        }
        Ok(ProtectEvent { address, size, protection })
    }

    pub fn address(&self) -> u64 {
        self.address
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// The first address past the range.
    pub fn end(&self) -> u64 {
        self.address + self.size
    }

    pub fn protection(&self) -> Protection {
        self.protection
    }
}

/// One memory event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryEvent {
    Map(MapEvent),
    Protect(ProtectEvent),
}

impl MemoryEvent {
    /// Stable lower-case name, used in logs.
    pub fn name(&self) -> &'static str {
        match self {
            MemoryEvent::Map(_) => "mmap",
            MemoryEvent::Protect(_) => "mprotect",
        }
    }

    /// The protection the event sets on its pages.
    pub fn protection(&self) -> Protection {
        match self {
            MemoryEvent::Map(m) => m.protection,
            MemoryEvent::Protect(p) => p.protection,
        }
    }
}

/// A condition a monitor reports about an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// The event leaves pages writable and executable at once.
    WritableAndExecutable { event: &'static str },
}

/// Records memory events and reports findings about them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryMonitor {
    events: Vec<MemoryEvent>,
}

impl MemoryMonitor {
    pub fn new() -> MemoryMonitor {
        MemoryMonitor::default()
    }

    /// Records the event and returns the findings it raises. An event that sets write and execute together raises one.
    pub fn record(&mut self, event: MemoryEvent) -> Vec<Finding> {
        let mut findings = Vec::new();
        if event.protection().is_writable_and_executable() {
            findings.push(Finding::WritableAndExecutable { event: event.name() });
        }
        self.events.push(event);
        findings
    }

    pub fn events(&self) -> &[MemoryEvent] {
        &self.events
    }

    pub fn count(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_bits_and_predicates() {
        let rx = Protection::from_bits(5).unwrap();
        assert!(rx.readable() && rx.executable() && !rx.writable());
        assert_eq!(rx.bits(), 5);
        assert!(Protection::from_bits(8).is_none());
        assert!(!rx.is_writable_and_executable());
    }

    #[test]
    fn a_map_event_protection_must_fit_its_maximum() {
        let read_only_max = Protection::READ;
        let err = MapEvent::new(Protection::READ.union(Protection::EXEC), read_only_max, 0, 0, "/lib/x.so");
        assert_eq!(err, Err(MemoryError::ProtectionExceedsMaximum));
        let ok = MapEvent::new(Protection::READ, Protection::READ.union(Protection::WRITE), 1, 4096, "/lib/x.so").unwrap();
        assert_eq!(ok.file_pos(), 4096);
        assert_eq!(ok.source(), "/lib/x.so");
        assert_eq!(MapEvent::new(Protection::READ, Protection::READ, 0, 0, ""), Err(MemoryError::EmptySource));
    }

    #[test]
    fn a_protect_range_must_be_non_empty_and_must_not_wrap() {
        assert_eq!(ProtectEvent::new(4096, 0, Protection::READ), Err(MemoryError::EmptyRange));
        assert_eq!(ProtectEvent::new(u64::MAX - 1, 4, Protection::READ), Err(MemoryError::RangeOverflow));
        let p = ProtectEvent::new(4096, 8192, Protection::READ).unwrap();
        assert_eq!(p.end(), 12288);
        assert_eq!(p.size(), 8192);
        assert_eq!(p.address(), 4096);
    }

    #[test]
    fn the_monitor_flags_a_range_that_becomes_writable_and_executable() {
        let mut m = MemoryMonitor::new();
        let bad = MemoryEvent::Protect(ProtectEvent::new(0x1000, 0x1000, Protection::WRITE.union(Protection::EXEC)).unwrap());
        assert_eq!(m.record(bad), vec![Finding::WritableAndExecutable { event: "mprotect" }]);
        let good = MemoryEvent::Protect(ProtectEvent::new(0x2000, 0x1000, Protection::READ.union(Protection::EXEC)).unwrap());
        assert!(m.record(good).is_empty());
        assert_eq!(m.count(), 2);
    }

    #[test]
    fn a_map_that_is_writable_and_executable_is_reported_as_a_map() {
        let mut m = MemoryMonitor::new();
        let max = Protection::READ.union(Protection::WRITE).union(Protection::EXEC);
        let ev = MapEvent::new(max, max, 0, 0, "/tmp/payload").unwrap();
        let out = m.record(MemoryEvent::Map(ev));
        assert_eq!(out, vec![Finding::WritableAndExecutable { event: "mmap" }]);
        assert_eq!(m.events()[0].name(), "mmap");
    }

    #[test]
    fn event_names_and_protections_are_reported_per_kind() {
        let p = MemoryEvent::Protect(ProtectEvent::new(0, 1, Protection::NONE).unwrap());
        assert_eq!(p.name(), "mprotect");
        assert_eq!(p.protection(), Protection::NONE);
        assert!(Protection::NONE.fits_within(Protection::NONE));
        assert!(!Protection::WRITE.fits_within(Protection::READ));
    }
}
