//! Advertising data: the length, type and value structures of a broadcast payload. The builder packs the flags, local
//! name, service UUIDs and manufacturer data a peripheral sends, within the 31-byte legacy payload limit. The parser
//! reads the same bytes back on the scanning side.
//! Pure model: there is no radio here. The bytes come from a transport and go to one.
use crate::gatt::Uuid;

/// Largest legacy advertising payload in bytes.
pub const LEGACY_PAYLOAD_MAX: usize = 31;

/// Advertising data type codes.
pub mod ad_type {
    pub const FLAGS: u8 = 0x01;
    pub const INCOMPLETE_16: u8 = 0x02;
    pub const COMPLETE_16: u8 = 0x03;
    pub const INCOMPLETE_128: u8 = 0x06;
    pub const COMPLETE_128: u8 = 0x07;
    pub const SHORTENED_NAME: u8 = 0x08;
    pub const COMPLETE_NAME: u8 = 0x09;
    pub const MANUFACTURER: u8 = 0xFF;
}

/// Flag bits of the flags structure.
pub mod flag {
    pub const LE_LIMITED_DISCOVERABLE: u8 = 0x01;
    pub const LE_GENERAL_DISCOVERABLE: u8 = 0x02;
    pub const BR_EDR_NOT_SUPPORTED: u8 = 0x04;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvertisingError {
    /// Adding this structure would take the payload past `LEGACY_PAYLOAD_MAX` bytes.
    PayloadTooLong { max: usize },
    /// One structure holds at most 254 bytes of data after its type byte.
    FieldTooLong,
}

/// An advertising payload being built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdvertisingData {
    bytes: Vec<u8>,
}

impl AdvertisingData {
    pub fn new() -> AdvertisingData {
        AdvertisingData { bytes: Vec::new() }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Appends one structure: length (type and data), type, data.
    pub fn push(&mut self, ad: u8, data: &[u8]) -> Result<(), AdvertisingError> {
        if data.len() > 254 {
            return Err(AdvertisingError::FieldTooLong);
        }
        if self.bytes.len() + data.len() + 2 > LEGACY_PAYLOAD_MAX {
            return Err(AdvertisingError::PayloadTooLong { max: LEGACY_PAYLOAD_MAX });
        }
        self.bytes.push(data.len() as u8 + 1);
        self.bytes.push(ad);
        self.bytes.extend_from_slice(data);
        Ok(())
    }

    pub fn flags(&mut self, flags: u8) -> Result<(), AdvertisingError> {
        self.push(ad_type::FLAGS, &[flags])
    }

    pub fn local_name(&mut self, name: &str) -> Result<(), AdvertisingError> {
        self.push(ad_type::COMPLETE_NAME, name.as_bytes())
    }

    /// Service UUIDs. Uses the 16-bit list when every UUID is a base-UUID alias, otherwise the 128-bit list.
    pub fn service_uuids(&mut self, uuids: &[Uuid], complete: bool) -> Result<(), AdvertisingError> {
        let short: Option<Vec<u16>> = uuids
            .iter()
            .map(|u| u.short().filter(|v| *v <= 0xFFFF).map(|v| v as u16))
            .collect();
        match short {
            Some(list) => {
                let data: Vec<u8> = list.iter().flat_map(|v| v.to_le_bytes()).collect();
                let ad = if complete { ad_type::COMPLETE_16 } else { ad_type::INCOMPLETE_16 };
                self.push(ad, &data)
            }
            None => {
                let data: Vec<u8> = uuids.iter().flat_map(|u| u.to_bytes_le()).collect();
                let ad = if complete { ad_type::COMPLETE_128 } else { ad_type::INCOMPLETE_128 };
                self.push(ad, &data)
            }
        }
    }

    /// Manufacturer data: the 16-bit company identifier (little-endian), then the company's bytes.
    pub fn manufacturer(&mut self, company: u16, data: &[u8]) -> Result<(), AdvertisingError> {
        let mut v = company.to_le_bytes().to_vec();
        v.extend_from_slice(data);
        self.push(ad_type::MANUFACTURER, &v)
    }
}

/// One structure of a received payload, borrowing its data from the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdStructure<'a> {
    pub ad_type: u8,
    pub data: &'a [u8],
}

/// Splits a payload into its structures. A zero length ends the payload (padding), and a structure that runs past
/// the end stops the parse, keeping the structures read so far.
pub fn parse(bytes: &[u8]) -> Vec<AdStructure<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let len = bytes[i] as usize;
        if len == 0 || i + 1 + len > bytes.len() {
            break;
        }
        out.push(AdStructure { ad_type: bytes[i + 1], data: &bytes[i + 2..i + 1 + len] });
        i += 1 + len;
    }
    out
}

/// The flags byte, when the payload carries one.
pub fn flags(bytes: &[u8]) -> Option<u8> {
    parse(bytes)
        .iter()
        .find(|s| s.ad_type == ad_type::FLAGS)
        .and_then(|s| s.data.first().copied())
}

/// The local name. The complete name wins over the shortened one.
pub fn local_name(bytes: &[u8]) -> Option<String> {
    let all = parse(bytes);
    let found = all
        .iter()
        .find(|s| s.ad_type == ad_type::COMPLETE_NAME)
        .or_else(|| all.iter().find(|s| s.ad_type == ad_type::SHORTENED_NAME));
    found.map(|s| String::from_utf8_lossy(s.data).into_owned())
}

/// Every service UUID listed in the payload, in order, from both the 16-bit and the 128-bit lists.
pub fn service_uuids(bytes: &[u8]) -> Vec<Uuid> {
    let mut out = Vec::new();
    for s in parse(bytes) {
        match s.ad_type {
            ad_type::COMPLETE_16 | ad_type::INCOMPLETE_16 => {
                for c in s.data.chunks_exact(2) {
                    out.push(Uuid::from_16(u16::from_le_bytes([c[0], c[1]])));
                }
            }
            ad_type::COMPLETE_128 | ad_type::INCOMPLETE_128 => {
                for c in s.data.chunks_exact(16) {
                    let mut b = [0u8; 16];
                    b.copy_from_slice(c);
                    out.push(Uuid::from_bytes_le(b));
                }
            }
            _ => {}
        }
    }
    out
}

/// The manufacturer-specific bytes that follow the company identifier, when the payload carries that company.
pub fn manufacturer_data(bytes: &[u8], company: u16) -> Option<&[u8]> {
    parse(bytes)
        .into_iter()
        .find(|s| {
            s.ad_type == ad_type::MANUFACTURER
                && s.data.len() >= 2
                && u16::from_le_bytes([s.data[0], s.data[1]]) == company
        })
        .map(|s| &s.data[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_flags_and_name_in_order() {
        let mut ad = AdvertisingData::new();
        ad.flags(flag::LE_GENERAL_DISCOVERABLE | flag::BR_EDR_NOT_SUPPORTED).unwrap();
        ad.local_name("Hi").unwrap();
        assert_eq!(ad.as_bytes(), &[2, 1, 6, 3, 9, b'H', b'i'][..]);
    }

    #[test]
    fn refuses_to_pass_the_legacy_payload_limit() {
        let mut ad = AdvertisingData::new();
        ad.push(0xF0, &[0u8; 20]).unwrap(); // 22 bytes so far
        // 22 + 10 + 2 = 34 > 31
        assert_eq!(ad.push(0xF0, &[0u8; 10]), Err(AdvertisingError::PayloadTooLong { max: 31 }));
        // 22 + 7 + 2 = 31 fits exactly
        ad.push(0xF0, &[0u8; 7]).unwrap();
        assert_eq!(ad.as_bytes().len(), 31);
        assert_eq!(AdvertisingData::new().push(0xF0, &[0u8; 255]), Err(AdvertisingError::FieldTooLong));
    }

    #[test]
    fn parse_reads_back_what_the_builder_wrote() {
        let bytes = [2u8, 1, 6, 3, 9, b'H', b'i'];
        let all = parse(&bytes);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1], AdStructure { ad_type: 9, data: &[b'H', b'i'][..] });
        assert_eq!(flags(&bytes), Some(6));
        assert_eq!(local_name(&bytes), Some("Hi".to_string()));
    }

    #[test]
    fn truncated_and_zero_length_structures_stop_the_parse() {
        assert!(parse(&[5, 9, b'a']).is_empty(), "a length past the end yields nothing");
        assert!(parse(&[0, 1, 2]).is_empty(), "a zero length ends the payload");
        // A complete structure followed by a truncated one keeps the first.
        assert_eq!(parse(&[2, 1, 6, 4, 9, b'x']).len(), 1);
    }

    #[test]
    fn complete_name_wins_over_shortened() {
        let bytes = [4u8, 8, b'a', b'b', b'c', 3, 9, b'A', b'B'];
        assert_eq!(local_name(&bytes), Some("AB".to_string()));
        let short_only = [3u8, 8, b'x', b'y'];
        assert_eq!(local_name(&short_only), Some("xy".to_string()));
        assert_eq!(local_name(&[]), None);
    }

    #[test]
    fn service_uuids_round_trip_in_both_lists() {
        let mut ad = AdvertisingData::new();
        let uuids = [Uuid::parse("180D").unwrap(), Uuid::parse("180F").unwrap()];
        ad.service_uuids(&uuids, true).unwrap();
        assert_eq!(ad.as_bytes(), &[5, 3, 0x0D, 0x18, 0x0F, 0x18][..]);
        assert_eq!(service_uuids(ad.as_bytes()), uuids.to_vec());

        let long = Uuid::parse("6E400001-B5A3-F393-E0A9-E50E24DCCA9E").unwrap();
        let mut wide = AdvertisingData::new();
        wide.service_uuids(&[long], false).unwrap();
        assert_eq!(wide.as_bytes()[1], ad_type::INCOMPLETE_128);
        assert_eq!(wide.as_bytes().len(), 2 + 16);
        assert_eq!(service_uuids(wide.as_bytes()), vec![long]);
    }

    #[test]
    fn manufacturer_data_matches_the_company_identifier() {
        let mut ad = AdvertisingData::new();
        ad.manufacturer(0x1234, &[0xAB]).unwrap();
        assert_eq!(ad.as_bytes(), &[4, 0xFF, 0x34, 0x12, 0xAB][..]);
        assert_eq!(manufacturer_data(ad.as_bytes(), 0x1234), Some(&[0xABu8][..]));
        assert_eq!(manufacturer_data(ad.as_bytes(), 0x0001), None);
    }
}
