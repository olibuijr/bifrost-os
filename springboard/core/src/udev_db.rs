//! udev database records: the properties, device-node symlinks and tags that udev keeps for each device under its data
//! directory (one file per device, named by the device id).
//!
//! Pure: `parse_record` reads the text of one record and `DbId` names the file. `read_in` is the only function that
//! touches the file system, and it only reads. Record types this model does not keep (the version and the link
//! priority) and unknown record types are skipped, so a newer udev that writes more lines still parses.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path};

/// The longest device-node or subsystem name a database id may carry.
const MAX_NAME: usize = 255;

/// Whether a device id names a block or a character device node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Block,
    Char,
}

/// The name of one database record: a device node by its kind and major and minor numbers, or a device that has no
/// node, by subsystem and system name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbId {
    Node { kind: NodeKind, major: u32, minor: u32 },
    Subsystem { subsystem: String, sysname: String },
}

impl DbId {
    /// Parses `b8:0`, `c226:128` or `+input:event3`. Anything else is refused.
    pub fn parse(text: &str) -> Result<DbId, UdevError> {
        let bad = || UdevError::BadId(text.to_string());
        if let Some(rest) = text.strip_prefix('+') {
            let (subsystem, sysname) = rest.split_once(':').ok_or_else(bad)?;
            if !is_name(subsystem) || !is_name(sysname) {
                return Err(bad());
            }
            return Ok(DbId::Subsystem { subsystem: subsystem.to_string(), sysname: sysname.to_string() });
        }
        let (kind, rest) = match text.as_bytes().first() {
            Some(b'b') => (NodeKind::Block, &text[1..]),
            Some(b'c') => (NodeKind::Char, &text[1..]),
            _ => return Err(bad()),
        };
        let (major, minor) = rest.split_once(':').ok_or_else(bad)?;
        Ok(DbId::Node { kind, major: number(major).ok_or_else(bad)?, minor: number(minor).ok_or_else(bad)? })
    }

    /// The file name of the record under the data directory.
    pub fn file_name(&self) -> String {
        match self {
            DbId::Node { kind, major, minor } => {
                let letter = match kind {
                    NodeKind::Block => 'b',
                    NodeKind::Char => 'c',
                };
                format!("{letter}{major}:{minor}")
            }
            DbId::Subsystem { subsystem, sysname } => format!("+{subsystem}:{sysname}"),
        }
    }
}

/// Digits only, no sign and no spaces, so `+5` and `-1` are not numbers here.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A subsystem or system name: letters, digits, `-`, `_` and `.`, never a path step.
fn is_name(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_NAME && text != "." && text != ".." && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// The udev record of one device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// When udev first initialised the device, in microseconds since the epoch (`I:`).
    pub initialized_usec: Option<u64>,
    /// Device properties (`E:KEY=value`), keys in upper case as udev writes them.
    pub properties: BTreeMap<String, String>,
    /// Symlinks under the device directory, relative to it (`S:`).
    pub symlinks: Vec<String>,
    /// Tags, from both the tag list and the current tag list (`G:` and `Q:`).
    pub tags: Vec<String>,
}

impl Record {
    /// The value of one property, if the record has it.
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(|v| v.as_str())
    }

    /// True when the device carries `tag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

/// Why a record could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UdevError {
    /// The text is not a database id (`b8:0`, `c226:128`, `+input:event3`).
    BadId(String),
    /// A line of the record is malformed. `line` counts from one.
    BadLine { line: usize, reason: &'static str },
    /// There is no record for the device, or the database directory is absent (no udev on this system).
    Missing,
    /// The record exists but cannot be read.
    Io(io::ErrorKind),
}

impl fmt::Display for UdevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UdevError::BadId(id) => write!(f, "not a udev database id: {id}"),
            UdevError::BadLine { line, reason } => write!(f, "udev record line {line}: {reason}"),
            UdevError::Missing => write!(f, "no udev record for this device"),
            UdevError::Io(kind) => write!(f, "udev record unreadable: {kind:?}"),
        }
    }
}

impl std::error::Error for UdevError {}

/// Parses the text of one record. Blank lines are skipped.
pub fn parse_record(text: &str) -> Result<Record, UdevError> {
    let mut rec = Record::default();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let bad = |reason: &'static str| UdevError::BadLine { line, reason };
        let (kind, body) = raw.split_once(':').ok_or_else(|| bad("expected a record type and a colon"))?;
        match kind {
            "I" => rec.initialized_usec = Some(body.parse().map_err(|_| bad("initialisation time is not a number"))?),
            "E" => {
                let (key, value) = body.split_once('=').ok_or_else(|| bad("property has no value"))?;
                if key.is_empty() || !key.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
                    return Err(bad("property key must be upper case letters, digits or underscores"));
                }
                rec.properties.insert(key.to_string(), value.to_string());
            }
            "S" => {
                let path = Path::new(body);
                if body.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
                    return Err(bad("symlink must be a relative path inside the device directory"));
                }
                rec.symlinks.push(body.to_string());
            }
            "G" | "Q" => {
                if body.is_empty() {
                    return Err(bad("tag is empty"));
                }
                rec.tags.push(body.to_string());
            }
            _ => {}
        }
    }
    Ok(rec)
}

/// Reads the record of `id` from the data directory `root`.
pub fn read_in(root: &Path, id: &DbId) -> Result<Record, UdevError> {
    let text = fs::read_to_string(root.join(id.file_name())).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => UdevError::Missing,
        kind => UdevError::Io(kind),
    })?;
    parse_record(&text)
}
