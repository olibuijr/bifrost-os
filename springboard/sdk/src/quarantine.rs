//! Download quarantine records (`LSQuarantine` properties) for files an app saves from the network.
//!
//! A record says where a file came from and which app wrote it. It is kept in the extended attribute
//! [`QUARANTINE_XATTR`] of the file, so it travels with the file on the same filesystem and a copy made with
//! [`file_metadata::copy_xattrs`] keeps it. The value is four `;`-separated fields, as the system writes them:
//! flags in hex, the download time in seconds in hex, the agent name, and a UUID.
//!
//! ```ignore
//! let record = Quarantine::new(0, now_secs, "BifrOSt Browser", "5b1e...");
//! quarantine::set_quarantine(&path, &record)?;   // the file now opens with a first-run warning in the shell
//! quarantine::quarantine_of(&path)?;             // Some(record), or None when the file has no record
//! quarantine::clear_quarantine(&path)?;          // the user approved the file
//! ```
//!
//! The record is read and written by apps only through this module. The shell decides what a record means for
//! opening the file; the record format here is the shared part. Nothing in the shell reads the attribute yet.

use std::fmt;
use std::io;
use std::path::Path;

use crate::file_metadata;

/// The attribute that holds the quarantine record (`com.apple.quarantine` in the system's layout).
pub const QUARANTINE_XATTR: &str = "user.bifrost.quarantine";

/// A quarantine record: the flags, the download time, the agent and the record UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quarantine {
    /// Raw flag bits. Their meaning belongs to the shell; this module stores them unchanged.
    pub flags: u32,
    /// Seconds since the Unix epoch when the file was written.
    pub timestamp: u64,
    /// The app that wrote the file, as shown to the user.
    pub agent: String,
    /// An identifier for the record, unique per download.
    pub uuid: String,
}

/// `QuarantineError`: a record that cannot be written or cannot be read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuarantineError {
    /// The agent or UUID contains a `;` or a line break, which the format does not allow.
    BadField(&'static str),
    /// The value does not have four fields, or a number field does not parse.
    Malformed,
}

impl fmt::Display for QuarantineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuarantineError::BadField(name) => write!(f, "the quarantine {name} contains a separator or a line break"),
            QuarantineError::Malformed => write!(f, "the quarantine record is malformed"),
        }
    }
}

impl Quarantine {
    /// A record with the given fields.
    pub fn new(flags: u32, timestamp: u64, agent: &str, uuid: &str) -> Self {
        Quarantine { flags, timestamp, agent: agent.into(), uuid: uuid.into() }
    }

    /// The attribute value: `flags;timestamp;agent;uuid`, with the numbers in lowercase hex.
    pub fn encode(&self) -> Result<String, QuarantineError> {
        for (name, field) in [("agent", &self.agent), ("uuid", &self.uuid)] {
            if field.contains(';') || field.contains('\n') || field.contains('\r') {
                return Err(QuarantineError::BadField(name));
            }
        }
        Ok(format!("{:04x};{:08x};{};{}", self.flags, self.timestamp, self.agent, self.uuid))
    }

    /// Reads an attribute value written by [`Quarantine::encode`].
    pub fn parse(value: &str) -> Result<Self, QuarantineError> {
        let mut parts = value.splitn(4, ';');
        let (flags, timestamp, agent, uuid) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(f), Some(t), Some(a), Some(u)) => (f, t, a, u),
            _ => return Err(QuarantineError::Malformed),
        };
        let flags = u32::from_str_radix(flags, 16).map_err(|_| QuarantineError::Malformed)?;
        let timestamp = u64::from_str_radix(timestamp, 16).map_err(|_| QuarantineError::Malformed)?;
        Ok(Quarantine { flags, timestamp, agent: agent.into(), uuid: uuid.into() })
    }
}

/// The record on `path`, or `None` when the file has none. A link is not followed.
pub fn quarantine_of(path: &Path) -> io::Result<Option<Quarantine>> {
    match file_metadata::xattr(path, QUARANTINE_XATTR)? {
        None => Ok(None),
        Some(bytes) => {
            let text = String::from_utf8(bytes).map_err(|_| invalid("the quarantine record is not UTF-8"))?;
            Quarantine::parse(&text).map(Some).map_err(|e| invalid(&e.to_string()))
        }
    }
}

/// Writes `record` on `path`, replacing any record already there.
pub fn set_quarantine(path: &Path, record: &Quarantine) -> io::Result<()> {
    let value = record.encode().map_err(|e| invalid(&e.to_string()))?;
    file_metadata::set_xattr(path, QUARANTINE_XATTR, value.as_bytes())
}

/// Removes the record from `path`. A file without a record is left as it is.
pub fn clear_quarantine(path: &Path) -> io::Result<()> {
    file_metadata::remove_xattr(path, QUARANTINE_XATTR)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_record_encodes_to_four_fields_with_hex_numbers() {
        let r = Quarantine::new(0x0040, 1_700_000_000, "BifrOSt Browser", "A1B2");
        assert_eq!(r.encode().unwrap(), "0040;6553f100;BifrOSt Browser;A1B2");
    }

    #[test]
    fn a_record_parses_back_to_the_same_fields() {
        let r = Quarantine::new(3, 42, "Mail", "uuid-1");
        assert_eq!(Quarantine::parse(&r.encode().unwrap()), Ok(r));
    }

    #[test]
    fn a_separator_in_the_agent_or_uuid_is_refused() {
        assert_eq!(Quarantine::new(0, 1, "a;b", "u").encode(), Err(QuarantineError::BadField("agent")));
        assert_eq!(Quarantine::new(0, 1, "a", "u\n").encode(), Err(QuarantineError::BadField("uuid")));
    }

    #[test]
    fn a_short_or_non_hex_value_is_malformed() {
        assert_eq!(Quarantine::parse("0040;1;agent"), Err(QuarantineError::Malformed));
        assert_eq!(Quarantine::parse("zz;1;agent;u"), Err(QuarantineError::Malformed));
        assert_eq!(Quarantine::parse("0;zz;agent;u"), Err(QuarantineError::Malformed));
    }

    #[test]
    fn a_uuid_may_be_empty_and_an_agent_keeps_spaces() {
        let r = Quarantine::parse("0;0;Some App;").unwrap();
        assert_eq!((r.agent.as_str(), r.uuid.as_str()), ("Some App", ""));
    }

    #[test]
    fn a_record_is_set_read_and_cleared_on_a_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("quarantine-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("download.bin");
        fs::write(&file, b"data").unwrap();
        if file_metadata::set_xattr(&file, "user.bifrost.probe", b"1").is_err() {
            // The filesystem has no user extended attributes; nothing to test here.
            fs::remove_dir_all(&dir).ok();
            return;
        }
        file_metadata::remove_xattr(&file, "user.bifrost.probe").ok();
        assert_eq!(quarantine_of(&file).unwrap(), None);
        let r = Quarantine::new(0, 7, "Browser", "u-7");
        set_quarantine(&file, &r).unwrap();
        assert_eq!(quarantine_of(&file).unwrap(), Some(r));
        clear_quarantine(&file).unwrap();
        assert_eq!(quarantine_of(&file).unwrap(), None);
        clear_quarantine(&file).unwrap();
        fs::remove_dir_all(&dir).ok();
    }
}
