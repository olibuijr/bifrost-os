//! Thread operational dataset as data: the type-length-value records a commissioner hands to an accessory so that it can
//! attach to a Thread network. This module reads the hex form, checks the length and value of each known record, keeps
//! each record type once, and reports what an attach still lacks. It derives no keys and uses no radio: the border
//! router and the accessory do that work.

use std::collections::BTreeMap;

/// Record types of the operational dataset.
pub mod tlv {
    pub const CHANNEL: u8 = 0;
    pub const PAN_ID: u8 = 1;
    pub const EXTENDED_PAN_ID: u8 = 2;
    pub const NETWORK_NAME: u8 = 3;
    pub const PSKC: u8 = 4;
    pub const NETWORK_MASTER_KEY: u8 = 5;
    pub const NETWORK_KEY_SEQUENCE: u8 = 6;
    pub const MESH_LOCAL_PREFIX: u8 = 7;
    pub const STEERING_DATA: u8 = 8;
    pub const BORDER_AGENT_LOCATOR: u8 = 9;
    pub const COMMISSIONER_SESSION_ID: u8 = 0x0b;
    pub const SECURITY_POLICY: u8 = 0x0c;
    pub const ACTIVE_TIMESTAMP: u8 = 0x0e;
    pub const PENDING_TIMESTAMP: u8 = 0x33;
    pub const DELAY_TIMER: u8 = 0x34;
    pub const CHANNEL_MASK: u8 = 0x35;
}

/// The records an accessory needs before it can attach to the network.
const REQUIRED_TO_ATTACH: [u8; 7] = [
    tlv::ACTIVE_TIMESTAMP,
    tlv::CHANNEL,
    tlv::PAN_ID,
    tlv::EXTENDED_PAN_ID,
    tlv::NETWORK_NAME,
    tlv::NETWORK_MASTER_KEY,
    tlv::MESH_LOCAL_PREFIX,
];

/// The channels of the 2.4 GHz channel page that a Thread network may use.
const CHANNELS: std::ops::RangeInclusive<u16> = 11..=26;

/// The longest network name a dataset may carry, in bytes.
const NETWORK_NAME_MAX: usize = 16;

/// Why a dataset or one of its records was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatasetError {
    /// The hex text has an odd number of digits.
    OddLength,
    /// The text holds a character that is not a hexadecimal digit.
    InvalidHex,
    /// A record header or value runs past the end of the data.
    Truncated,
    /// The record type already appeared in this dataset.
    Duplicate(u8),
    /// The record's length is not the length its type requires.
    BadLength { kind: u8, len: usize },
    /// The record's length is right, but its value is not allowed (a channel outside the range, a name that is not UTF-8).
    BadValue { kind: u8 },
}

/// An operational dataset: at most one value per record type, kept in ascending type order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dataset {
    records: BTreeMap<u8, Vec<u8>>,
}

impl Dataset {
    /// An empty dataset.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a dataset from its hex text. Whitespace is ignored and either letter case is accepted.
    pub fn from_hex(text: &str) -> Result<Self, DatasetError> {
        let mut nibbles = Vec::new();
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            nibbles.push(c.to_digit(16).ok_or(DatasetError::InvalidHex)? as u8);
        }
        if nibbles.len() % 2 != 0 {
            return Err(DatasetError::OddLength);
        }
        let bytes: Vec<u8> = nibbles.chunks(2).map(|pair| (pair[0] << 4) | pair[1]).collect();
        Self::from_bytes(&bytes)
    }

    /// Reads a dataset from its encoded bytes. A length byte of 255 announces a two-byte big-endian length after it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DatasetError> {
        let mut dataset = Dataset::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            if rest.len() < 2 {
                return Err(DatasetError::Truncated);
            }
            let kind = rest[0];
            let (len, header) = if rest[1] == 0xff {
                if rest.len() < 4 {
                    return Err(DatasetError::Truncated);
                }
                (usize::from(u16::from_be_bytes([rest[2], rest[3]])), 4)
            } else {
                (usize::from(rest[1]), 2)
            };
            let end = header + len;
            if rest.len() < end {
                return Err(DatasetError::Truncated);
            }
            dataset.insert(kind, rest[header..end].to_vec())?;
            rest = &rest[end..];
        }
        Ok(dataset)
    }

    /// Stores one record after checking its length and value. A type already present is refused, so a dataset never
    /// holds two values for one record type.
    pub fn insert(&mut self, kind: u8, value: Vec<u8>) -> Result<(), DatasetError> {
        check(kind, &value)?;
        if self.records.contains_key(&kind) {
            return Err(DatasetError::Duplicate(kind));
        }
        self.records.insert(kind, value);
        Ok(())
    }

    /// The value of one record type, if the dataset has it.
    pub fn get(&self, kind: u8) -> Option<&[u8]> {
        self.records.get(&kind).map(Vec::as_slice)
    }

    /// The channel the network uses.
    pub fn channel(&self) -> Option<u16> {
        self.get(tlv::CHANNEL).map(|v| u16::from_be_bytes([v[1], v[2]]))
    }

    /// The 16-bit PAN identifier.
    pub fn pan_id(&self) -> Option<u16> {
        self.get(tlv::PAN_ID).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }

    /// The network name, when it is valid UTF-8.
    pub fn network_name(&self) -> Option<&str> {
        self.get(tlv::NETWORK_NAME).and_then(|v| std::str::from_utf8(v).ok())
    }

    /// The record types an attach needs and the dataset lacks, in the order of the attach list.
    pub fn missing_for_attach(&self) -> Vec<u8> {
        REQUIRED_TO_ATTACH.iter().copied().filter(|kind| !self.records.contains_key(kind)).collect()
    }

    /// Whether the dataset carries every record an attach needs.
    pub fn is_complete(&self) -> bool {
        self.missing_for_attach().is_empty()
    }

    /// The encoded bytes, records in ascending type order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, value) in &self.records {
            out.push(*kind);
            if value.len() < 255 {
                out.push(value.len() as u8);
            } else {
                out.push(0xff);
                out.extend_from_slice(&(value.len() as u16).to_be_bytes());
            }
            out.extend_from_slice(value);
        }
        out
    }

    /// The hex text of the encoded bytes, in lower case.
    pub fn to_hex(&self) -> String {
        self.to_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Checks one record against the length and value its type requires. Types outside the list are kept as they come.
fn check(kind: u8, value: &[u8]) -> Result<(), DatasetError> {
    let bad_length = || DatasetError::BadLength { kind, len: value.len() };
    if value.len() > usize::from(u16::MAX) {
        return Err(bad_length());
    }
    let fixed = |want: usize| if value.len() == want { Ok(()) } else { Err(bad_length()) };
    match kind {
        tlv::CHANNEL => {
            fixed(3)?;
            let channel = u16::from_be_bytes([value[1], value[2]]);
            if value[0] != 0 || !CHANNELS.contains(&channel) {
                return Err(DatasetError::BadValue { kind });
            }
            Ok(())
        }
        tlv::PAN_ID => fixed(2),
        tlv::EXTENDED_PAN_ID => fixed(8),
        tlv::NETWORK_KEY_SEQUENCE | tlv::DELAY_TIMER => fixed(4),
        tlv::NETWORK_NAME => {
            if value.is_empty() || value.len() > NETWORK_NAME_MAX {
                return Err(bad_length());
            }
            std::str::from_utf8(value).map(|_| ()).map_err(|_| DatasetError::BadValue { kind })
        }
        tlv::PSKC | tlv::NETWORK_MASTER_KEY => fixed(16),
        tlv::MESH_LOCAL_PREFIX | tlv::ACTIVE_TIMESTAMP | tlv::PENDING_TIMESTAMP => fixed(8),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: u8, value: &[u8]) -> Vec<u8> {
        let mut out = vec![kind, value.len() as u8];
        out.extend_from_slice(value);
        out
    }

    #[test]
    fn a_channel_record_reads_the_page_and_the_channel() {
        let mut d = Dataset::new();
        d.insert(tlv::CHANNEL, vec![0, 0, 15]).unwrap();
        assert_eq!(d.channel(), Some(15));
    }

    #[test]
    fn a_channel_outside_the_two_point_four_band_is_refused() {
        let mut d = Dataset::new();
        assert_eq!(d.insert(tlv::CHANNEL, vec![0, 0, 10]), Err(DatasetError::BadValue { kind: tlv::CHANNEL }));
        assert_eq!(d.insert(tlv::CHANNEL, vec![0, 0, 27]), Err(DatasetError::BadValue { kind: tlv::CHANNEL }));
        assert_eq!(d.insert(tlv::CHANNEL, vec![1, 0, 15]), Err(DatasetError::BadValue { kind: tlv::CHANNEL }));
        assert_eq!(d.insert(tlv::CHANNEL, vec![0, 15]), Err(DatasetError::BadLength { kind: tlv::CHANNEL, len: 2 }));
    }

    #[test]
    fn a_network_name_is_one_to_sixteen_utf8_bytes() {
        let mut d = Dataset::new();
        assert_eq!(d.insert(tlv::NETWORK_NAME, vec![]), Err(DatasetError::BadLength { kind: tlv::NETWORK_NAME, len: 0 }));
        assert_eq!(
            d.insert(tlv::NETWORK_NAME, vec![b'a'; 17]),
            Err(DatasetError::BadLength { kind: tlv::NETWORK_NAME, len: 17 })
        );
        assert_eq!(d.insert(tlv::NETWORK_NAME, vec![0xff, 0xfe]), Err(DatasetError::BadValue { kind: tlv::NETWORK_NAME }));
        d.insert(tlv::NETWORK_NAME, b"Hus-net".to_vec()).unwrap();
        assert_eq!(d.network_name(), Some("Hus-net"));
    }

    #[test]
    fn fixed_length_records_must_have_their_length() {
        let mut d = Dataset::new();
        assert_eq!(d.insert(tlv::NETWORK_MASTER_KEY, vec![0; 15]), Err(DatasetError::BadLength { kind: tlv::NETWORK_MASTER_KEY, len: 15 }));
        assert_eq!(d.insert(tlv::PAN_ID, vec![0; 3]), Err(DatasetError::BadLength { kind: tlv::PAN_ID, len: 3 }));
        assert_eq!(d.insert(tlv::EXTENDED_PAN_ID, vec![0; 7]), Err(DatasetError::BadLength { kind: tlv::EXTENDED_PAN_ID, len: 7 }));
        assert_eq!(d.insert(tlv::NETWORK_KEY_SEQUENCE, vec![0; 5]), Err(DatasetError::BadLength { kind: tlv::NETWORK_KEY_SEQUENCE, len: 5 }));
        assert_eq!(d.insert(tlv::ACTIVE_TIMESTAMP, vec![0; 7]), Err(DatasetError::BadLength { kind: tlv::ACTIVE_TIMESTAMP, len: 7 }));
        d.insert(tlv::PAN_ID, vec![0x12, 0x34]).unwrap();
        assert_eq!(d.pan_id(), Some(0x1234));
    }

    #[test]
    fn a_record_type_appears_once() {
        let mut d = Dataset::new();
        d.insert(tlv::PAN_ID, vec![0, 1]).unwrap();
        assert_eq!(d.insert(tlv::PAN_ID, vec![0, 2]), Err(DatasetError::Duplicate(tlv::PAN_ID)));
        assert_eq!(d.pan_id(), Some(1));
    }

    #[test]
    fn hex_text_is_read_with_spaces_and_either_case() {
        let mut bytes = record(tlv::PAN_ID, &[0xab, 0xcd]);
        bytes.extend(record(tlv::CHANNEL, &[0, 0, 26]));
        let text: String = bytes.iter().map(|b| format!("{b:02X} ")).collect();
        let d = Dataset::from_hex(&text).unwrap();
        assert_eq!(d.pan_id(), Some(0xabcd));
        assert_eq!(d.channel(), Some(26));
    }

    #[test]
    fn odd_digit_counts_and_non_hex_text_are_refused() {
        assert_eq!(Dataset::from_hex("0e08f"), Err(DatasetError::OddLength));
        assert_eq!(Dataset::from_hex("0g"), Err(DatasetError::InvalidHex));
    }

    #[test]
    fn a_record_that_runs_past_the_end_is_truncated() {
        assert_eq!(Dataset::from_bytes(&[tlv::PAN_ID, 2, 0]), Err(DatasetError::Truncated));
        assert_eq!(Dataset::from_bytes(&[tlv::PAN_ID]), Err(DatasetError::Truncated));
        assert_eq!(Dataset::from_bytes(&[tlv::CHANNEL_MASK, 0xff, 0x01]), Err(DatasetError::Truncated));
    }

    #[test]
    fn a_long_record_uses_the_extended_length_and_round_trips() {
        let mut d = Dataset::new();
        d.insert(tlv::CHANNEL_MASK, vec![0x55; 300]).unwrap();
        let bytes = d.to_bytes();
        assert_eq!(&bytes[..4], &[tlv::CHANNEL_MASK, 0xff, 0x01, 0x2c]);
        assert_eq!(Dataset::from_bytes(&bytes).unwrap(), d);
    }

    #[test]
    fn the_hex_form_writes_records_in_ascending_type_order() {
        let mut d = Dataset::new();
        d.insert(tlv::NETWORK_NAME, b"net".to_vec()).unwrap();
        d.insert(tlv::CHANNEL, vec![0, 0, 11]).unwrap();
        assert_eq!(d.to_hex(), "000300000b03036e6574");
        assert_eq!(Dataset::from_hex(&d.to_hex()).unwrap(), d);
    }

    #[test]
    fn a_dataset_without_the_attach_records_lists_what_is_missing() {
        let mut d = Dataset::new();
        d.insert(tlv::CHANNEL, vec![0, 0, 15]).unwrap();
        d.insert(tlv::PAN_ID, vec![0x12, 0x34]).unwrap();
        assert!(!d.is_complete());
        assert_eq!(
            d.missing_for_attach(),
            vec![
                tlv::ACTIVE_TIMESTAMP,
                tlv::EXTENDED_PAN_ID,
                tlv::NETWORK_NAME,
                tlv::NETWORK_MASTER_KEY,
                tlv::MESH_LOCAL_PREFIX,
            ]
        );
    }

    #[test]
    fn a_dataset_with_every_attach_record_is_complete() {
        let mut d = Dataset::new();
        d.insert(tlv::ACTIVE_TIMESTAMP, vec![0; 8]).unwrap();
        d.insert(tlv::CHANNEL, vec![0, 0, 20]).unwrap();
        d.insert(tlv::PAN_ID, vec![0, 7]).unwrap();
        d.insert(tlv::EXTENDED_PAN_ID, vec![1; 8]).unwrap();
        d.insert(tlv::NETWORK_NAME, b"hub".to_vec()).unwrap();
        d.insert(tlv::NETWORK_MASTER_KEY, vec![2; 16]).unwrap();
        d.insert(tlv::MESH_LOCAL_PREFIX, vec![3; 8]).unwrap();
        assert!(d.is_complete());
        assert!(d.missing_for_attach().is_empty());
    }
}
