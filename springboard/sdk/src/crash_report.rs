//! Crash reporter extension (the crash-report hook and the crashed-process view): an app that turns a crash into a report
//! reads the crashed process's binary images, symbolicates addresses (with inline frames expanded) and looks symbols
//! up by name.
//!
//! This is the pure model: [`ProcessCrash`] holds the reason, the loaded [`BinaryImageInfo`] list and each image's
//! symbol table, and answers the three lookups. [`CrashReporter`] is the entry point an extension implements. The
//! shell does not yet call an app's reporter when another app crashes, and it does not expose the crashed process's port, so
//! the crashed process's data reaches a reporter only when the caller builds the [`ProcessCrash`] itself (see
//! `docs/parity/w3-theme-14.md`).

use std::fmt;

/// Why the process crashed (the reason text the shell and the core dump tooling record).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CrashReason {
    pub description: String,
}

impl fmt::Display for CrashReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

/// One symbol in an image's table: the image-relative offset where it starts and its names, innermost first. More
/// than one name means the code at that offset is inlined into the functions after it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SymbolEntry {
    offset: u64,
    names: Vec<String>,
}

/// A binary image loaded in the crashed process: its name, where it is loaded and how large it is, and its symbols.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryImageInfo {
    pub name: String,
    pub load_address: u64,
    pub size: u64,
    symbols: Vec<SymbolEntry>,
}

impl BinaryImageInfo {
    pub fn new(name: &str, load_address: u64, size: u64) -> BinaryImageInfo {
        BinaryImageInfo { name: name.into(), load_address, size, symbols: Vec::new() }
    }

    /// Adds a symbol that starts `offset` bytes into the image. `names` lists the inline chain innermost first; an
    /// empty list adds nothing. The table is kept sorted by offset.
    pub fn with_symbol(mut self, offset: u64, names: &[&str]) -> BinaryImageInfo {
        if names.is_empty() {
            return self;
        }
        self.symbols.push(SymbolEntry { offset, names: names.iter().map(|n| n.to_string()).collect() });
        self.symbols.sort_by_key(|s| s.offset);
        self
    }

    /// True when `address` lies inside the image. Written as a subtraction so an image at the top of the address
    /// space cannot overflow.
    pub fn contains(&self, address: u64) -> bool {
        address >= self.load_address && address - self.load_address < self.size
    }
}

/// One frame from symbolication. An address inside an inlined function gives one frame per name: the innermost
/// name has `inline` set, and the last frame is the containing function with `inline` clear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolicatedFrame {
    pub image: String,
    /// `None` when the address lies in an image but before its first symbol.
    pub symbol: Option<String>,
    /// Bytes from the start of the symbol (or from the image start when `symbol` is `None`).
    pub symbol_offset: u64,
    pub address: u64,
    pub inline: bool,
}

/// A crashed process as a reporter sees it: the crash reason and the binary images, with symbol lookups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessCrash {
    pub reason: CrashReason,
    pub binary_images: Vec<BinaryImageInfo>,
}

impl ProcessCrash {
    pub fn new(reason: &str, binary_images: Vec<BinaryImageInfo>) -> ProcessCrash {
        ProcessCrash { reason: CrashReason { description: reason.into() }, binary_images }
    }

    /// Symbolicates one address. Returns no frames when no loaded image contains it.
    pub fn symbolicate_address(&self, address: u64) -> Vec<SymbolicatedFrame> {
        let Some(image) = self.binary_images.iter().find(|i| i.contains(address)) else {
            return Vec::new();
        };
        let offset = address - image.load_address;
        match image.symbols.iter().rev().find(|s| s.offset <= offset) {
            None => vec![SymbolicatedFrame {
                image: image.name.clone(),
                symbol: None,
                symbol_offset: offset,
                address,
                inline: false,
            }],
            Some(entry) => {
                let last = entry.names.len() - 1;
                entry
                    .names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| SymbolicatedFrame {
                        image: image.name.clone(),
                        symbol: Some(name.clone()),
                        symbol_offset: offset - entry.offset,
                        address,
                        inline: i < last,
                    })
                    .collect()
            }
        }
    }

    /// Symbolicates each address, in order.
    pub fn symbolicate_addresses(&self, addresses: &[u64]) -> Vec<Vec<SymbolicatedFrame>> {
        addresses.iter().map(|a| self.symbolicate_address(*a)).collect()
    }

    /// The address of a symbol by name, searched in the named image or in every image when `image_name` is `None`.
    /// Returns `None` when no image has it, or when the address would overflow.
    pub fn symbol_address(&self, image_name: Option<&str>, symbol_name: &str) -> Option<u64> {
        self.binary_images
            .iter()
            .filter(|i| image_name.map_or(true, |n| i.name == n))
            .find_map(|i| {
                i.symbols
                    .iter()
                    .find(|s| s.names.iter().any(|n| n == symbol_name))
                    .and_then(|s| i.load_address.checked_add(s.offset))
            })
    }
}

/// An app's crash reporter. The shell calls [`CrashReporter::process_crash_report`] with the crashed process when a
/// report is ready; the implementation writes whatever report it builds from the process data.
pub trait CrashReporter {
    fn process_crash_report(&mut self, process: &ProcessCrash);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> BinaryImageInfo {
        BinaryImageInfo::new("app", 0x1000, 0x100)
            .with_symbol(0x40, &["parse"])
            .with_symbol(0x0, &["start"])
            .with_symbol(0x80, &["inner_helper", "outer_call"])
    }

    #[test]
    fn contains_is_the_half_open_range_of_the_image() {
        let img = BinaryImageInfo::new("a", 0x1000, 0x100);
        assert!(img.contains(0x1000));
        assert!(img.contains(0x10ff));
        assert!(!img.contains(0x1100));
        assert!(!img.contains(0xfff));
    }

    #[test]
    fn contains_does_not_overflow_at_the_top_of_the_address_space() {
        let img = BinaryImageInfo::new("top", u64::MAX - 1, 10);
        assert!(img.contains(u64::MAX));
        assert!(!img.contains(0));
    }

    #[test]
    fn symbolicate_picks_the_symbol_that_starts_last_before_the_address() {
        let p = ProcessCrash::new("SIGSEGV", vec![app()]);
        let frames = p.symbolicate_address(0x1050);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].symbol.as_deref(), Some("parse"));
        assert_eq!(frames[0].symbol_offset, 0x10);
        assert_eq!(frames[0].image, "app");
        assert!(!frames[0].inline);
    }

    #[test]
    fn inline_chain_expands_to_one_frame_per_name_innermost_first() {
        let p = ProcessCrash::new("SIGABRT", vec![app()]);
        let frames = p.symbolicate_address(0x1085);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].symbol.as_deref(), Some("inner_helper"));
        assert!(frames[0].inline);
        assert_eq!(frames[1].symbol.as_deref(), Some("outer_call"));
        assert!(!frames[1].inline);
        assert_eq!(frames[0].symbol_offset, 5);
        assert_eq!(frames[1].symbol_offset, 5);
        assert_eq!(frames[0].address, 0x1085);
    }

    #[test]
    fn address_outside_every_image_gives_no_frames() {
        let p = ProcessCrash::new("SIGBUS", vec![app()]);
        assert!(p.symbolicate_address(0x2000).is_empty());
        assert!(p.symbolicate_address(0x0fff).is_empty());
    }

    #[test]
    fn address_before_the_first_symbol_reports_the_image_offset() {
        let img = BinaryImageInfo::new("late", 0x5000, 0x100).with_symbol(0x20, &["begin"]);
        let p = ProcessCrash::new("SIGILL", vec![img]);
        let frames = p.symbolicate_address(0x5008);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].symbol, None);
        assert_eq!(frames[0].symbol_offset, 8);
    }

    #[test]
    fn batch_symbolication_keeps_the_order_of_the_addresses() {
        let p = ProcessCrash::new("SIGSEGV", vec![app()]);
        let out = p.symbolicate_addresses(&[0x1000, 0x9999, 0x1041]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0][0].symbol.as_deref(), Some("start"));
        assert!(out[1].is_empty());
        assert_eq!(out[2][0].symbol.as_deref(), Some("parse"));
        assert_eq!(out[2][0].symbol_offset, 1);
    }

    #[test]
    fn symbol_address_finds_a_name_in_one_image_or_in_all_of_them() {
        let other = BinaryImageInfo::new("lib", 0x9000, 0x40).with_symbol(0x8, &["helper"]);
        let p = ProcessCrash::new("SIGSEGV", vec![app(), other]);
        assert_eq!(p.symbol_address(Some("app"), "parse"), Some(0x1040));
        assert_eq!(p.symbol_address(None, "helper"), Some(0x9008));
        assert_eq!(p.symbol_address(Some("app"), "helper"), None);
        assert_eq!(p.symbol_address(None, "missing"), None);
        assert_eq!(p.symbol_address(None, "outer_call"), Some(0x1080));
    }

    struct Count(usize);

    impl CrashReporter for Count {
        fn process_crash_report(&mut self, process: &ProcessCrash) {
            self.0 += process.binary_images.len();
        }
    }

    #[test]
    fn a_reporter_receives_the_process_and_keeps_the_reason() {
        let p = ProcessCrash::new("SIGSEGV at 0x0", vec![app()]);
        let mut r = Count(0);
        r.process_crash_report(&p);
        r.process_crash_report(&p);
        assert_eq!(r.0, 2);
        assert_eq!(p.reason.to_string(), "SIGSEGV at 0x0");
    }
}
