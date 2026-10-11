//! Matter setup codes and commissionable discovery (std only).
//!
//! A Matter accessory is added with a setup payload: the QR code ("MT:" then
//! Base38) or the 11 or 21 digit manual pairing code. Accessories waiting to be
//! commissioned advertise themselves on the local network with DNS-SD, and the
//! instance name and TXT record carry the discriminator and product identity.
//! This module decodes both forms of the payload and parses the service names
//! and TXT records. It does no radio, network or certificate work.

use std::fmt;

/// Base38 alphabet of the QR setup payload. Index equals digit value.
const BASE38: &[u8; 38] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-.";
const QR_PREFIX: &str = "MT:";
/// The payload is 88 bits, packed little-endian.
const QR_BYTES: usize = 11;

/// Verhoeff multiplication table (dihedral group D5).
const VERHOEFF_D: [[u8; 10]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
    [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
    [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
    [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
    [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
    [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
    [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
    [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
    [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
];
/// Verhoeff permutation, applied with the position modulo 8.
const VERHOEFF_P: [[u8; 10]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
    [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
    [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
    [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
    [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
    [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
    [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
];
/// Verhoeff inverse.
const VERHOEFF_INV: [u8; 10] = [0, 4, 3, 2, 1, 5, 6, 7, 8, 9];

/// Why a setup code, instance name or TXT record was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// The QR text does not start with "MT:".
    Prefix,
    /// A character or group size outside Base38.
    Base38,
    /// The QR payload is not 11 bytes.
    Length,
    /// The payload version is not 0.
    Version,
    /// The commissioning flow value is 3, which is reserved.
    Flow,
    /// The padding bits are not zero.
    Padding,
    /// The passcode is out of range or on the invalid list.
    Passcode,
    /// The manual code has the wrong number of digits or a non-digit.
    Digits,
    /// The manual code Verhoeff check digit does not match.
    Check,
    /// A manual code chunk is out of range for its field.
    Range,
    /// A DNS-SD name or TXT value is malformed.
    Txt,
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SetupError::Prefix => "setup code must start with MT:",
            SetupError::Base38 => "setup code has a character or group size outside Base38",
            SetupError::Length => "setup payload is not 11 bytes",
            SetupError::Version => "unsupported setup payload version",
            SetupError::Flow => "commissioning flow 3 is reserved",
            SetupError::Padding => "setup payload padding is not zero",
            SetupError::Passcode => "setup passcode is invalid",
            SetupError::Digits => "manual pairing code must be 11 or 21 digits",
            SetupError::Check => "manual pairing code check digit does not match",
            SetupError::Range => "manual pairing code field is out of range",
            SetupError::Txt => "malformed Matter service name or TXT record",
        };
        f.write_str(text)
    }
}

impl std::error::Error for SetupError {}

/// How the accessory is put into commissioning mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommissioningFlow {
    Standard,
    UserIntent,
    Custom,
}

/// The decoded QR setup payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupPayload {
    pub vendor_id: u16,
    pub product_id: u16,
    pub flow: CommissioningFlow,
    /// Discovery capability bits: 0x01 soft access point, 0x02 BLE, 0x04 on network.
    pub discovery: u8,
    /// The full 12-bit discriminator.
    pub discriminator: u16,
    pub passcode: u32,
}

impl SetupPayload {
    pub fn soft_ap(&self) -> bool {
        self.discovery & 0x01 != 0
    }
    pub fn ble(&self) -> bool {
        self.discovery & 0x02 != 0
    }
    pub fn on_network(&self) -> bool {
        self.discovery & 0x04 != 0
    }
    /// The upper 4 bits of the discriminator, which the manual code carries.
    pub fn short_discriminator(&self) -> u8 {
        (self.discriminator >> 8) as u8
    }
}

/// A decoded manual pairing code. Vendor and product are present only in the
/// 21 digit form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManualCode {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    /// The 4-bit short discriminator.
    pub short_discriminator: u8,
    pub passcode: u32,
}

/// Whether a passcode is allowed. Zero, values with one repeated digit, and
/// 12345678 and 87654321 are refused, as are values above 99999998.
pub fn passcode_is_valid(passcode: u32) -> bool {
    if passcode == 0 || passcode > 99_999_998 {
        return false;
    }
    if passcode == 12_345_678 || passcode == 87_654_321 {
        return false;
    }
    !(0..=9).any(|d| passcode == d * 11_111_111)
}

/// Decodes Base38 text. Each group of 5 characters is 3 bytes, 4 characters
/// is 2 bytes and 2 characters is 1 byte, all little-endian.
pub fn base38_decode(text: &str) -> Result<Vec<u8>, SetupError> {
    let mut out = Vec::with_capacity(text.len() / 5 * 3 + 2);
    for group in text.as_bytes().chunks(5) {
        let width: usize = match group.len() {
            5 => 3,
            4 => 2,
            2 => 1,
            _ => return Err(SetupError::Base38),
        };
        let mut value: u64 = 0;
        for &c in group.iter().rev() {
            let digit = BASE38.iter().position(|&a| a == c).ok_or(SetupError::Base38)? as u64;
            value = value * 38 + digit;
        }
        if value >= 1u64 << (8 * width as u32) {
            return Err(SetupError::Base38);
        }
        out.extend_from_slice(&value.to_le_bytes()[..width]);
    }
    Ok(out)
}

/// Reads one field from the low end of the packed payload bits.
fn take(bits: &mut u128, n: u32) -> u64 {
    let value = (*bits & ((1u128 << n) - 1)) as u64;
    *bits >>= n;
    value
}

/// Parses a QR setup payload such as "MT:-24J0AFN00KA0648G00".
pub fn parse_qr(text: &str) -> Result<SetupPayload, SetupError> {
    let body = text.strip_prefix(QR_PREFIX).ok_or(SetupError::Prefix)?;
    let bytes = base38_decode(body)?;
    if bytes.len() != QR_BYTES {
        return Err(SetupError::Length);
    }
    let mut raw = [0u8; 16];
    raw[..QR_BYTES].copy_from_slice(&bytes);
    let mut bits = u128::from_le_bytes(raw);
    // Field order and widths: version 3, vendor 16, product 16, flow 2,
    // discovery 8, discriminator 12, passcode 27, padding 4 (88 bits).
    let version = take(&mut bits, 3);
    let vendor_id = take(&mut bits, 16) as u16;
    let product_id = take(&mut bits, 16) as u16;
    let flow = match take(&mut bits, 2) {
        0 => CommissioningFlow::Standard,
        1 => CommissioningFlow::UserIntent,
        2 => CommissioningFlow::Custom,
        _ => return Err(SetupError::Flow),
    };
    let discovery = take(&mut bits, 8) as u8;
    let discriminator = take(&mut bits, 12) as u16;
    let passcode = take(&mut bits, 27) as u32;
    let padding = take(&mut bits, 4);
    if version != 0 {
        return Err(SetupError::Version);
    }
    if padding != 0 {
        return Err(SetupError::Padding);
    }
    if !passcode_is_valid(passcode) {
        return Err(SetupError::Passcode);
    }
    Ok(SetupPayload { vendor_id, product_id, flow, discovery, discriminator, passcode })
}

/// Verhoeff check digit for a run of decimal digits (values 0 to 9).
pub fn verhoeff_check(digits: &[u8]) -> u8 {
    let mut c = 0usize;
    for (i, &d) in digits.iter().rev().enumerate() {
        c = VERHOEFF_D[c][VERHOEFF_P[(i + 1) % 8][d as usize] as usize] as usize;
    }
    VERHOEFF_INV[c]
}

fn chunk(digits: &[u8]) -> u32 {
    digits.iter().fold(0u32, |acc, &d| acc * 10 + d as u32)
}

/// Parses a manual pairing code. Spaces and hyphens are ignored, so
/// "3497 011 2332" and "34970112332" are the same code.
pub fn parse_manual_code(text: &str) -> Result<ManualCode, SetupError> {
    let mut digits: Vec<u8> = Vec::with_capacity(21);
    for c in text.chars() {
        match c {
            '0'..='9' => digits.push(c as u8 - b'0'),
            ' ' | '-' => {}
            _ => return Err(SetupError::Digits),
        }
    }
    let long = match digits.len() {
        11 => false,
        21 => true,
        _ => return Err(SetupError::Digits),
    };
    let (body, check) = digits.split_at(digits.len() - 1);
    if verhoeff_check(body) != check[0] {
        return Err(SetupError::Check);
    }
    // The first digit holds the vendor/product flag (bit 2) and the top two
    // bits of the short discriminator (bits 0 and 1).
    let first = digits[0] as u32;
    if ((first >> 2) & 1 == 1) != long {
        return Err(SetupError::Digits);
    }
    let (vendor_id, product_id, rest) = if long {
        let vid = chunk(&digits[1..6]);
        let pid = chunk(&digits[6..11]);
        if vid > u16::MAX as u32 || pid > u16::MAX as u32 {
            return Err(SetupError::Range);
        }
        (Some(vid as u16), Some(pid as u16), 11)
    } else {
        (None, None, 1)
    };
    // The second chunk holds the low two bits of the short discriminator
    // (bits 14 and 15) and the low 14 bits of the passcode; the third holds
    // the top 13 bits of the passcode.
    let c2 = chunk(&digits[rest..rest + 5]);
    let c3 = chunk(&digits[rest + 5..rest + 9]);
    if c2 > 0xFFFF || c3 > 0x1FFF {
        return Err(SetupError::Range);
    }
    let passcode = (c3 << 14) | (c2 & 0x3FFF);
    if !passcode_is_valid(passcode) {
        return Err(SetupError::Passcode);
    }
    let short_discriminator = (((first & 3) << 2) | ((c2 >> 14) & 3)) as u8;
    Ok(ManualCode { vendor_id, product_id, short_discriminator, passcode })
}

/// DNS-SD service kinds used by Matter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    /// "_matterc._udp": an accessory waiting to be commissioned.
    Commissionable,
    /// "_matter._tcp": a commissioned node on a fabric.
    Operational,
}

/// Recognises a service name, with or without a trailing dot or ".local".
pub fn service_kind(service: &str) -> Option<ServiceKind> {
    let name = service.trim_end_matches('.');
    let name = name.strip_suffix(".local").unwrap_or(name);
    if name.eq_ignore_ascii_case("_matterc._udp") {
        Some(ServiceKind::Commissionable)
    } else if name.eq_ignore_ascii_case("_matter._tcp") {
        Some(ServiceKind::Operational)
    } else {
        None
    }
}

/// An accessory advertising itself for commissioning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commissionable {
    /// The random 16 hex digit instance name.
    pub instance: String,
    /// "D", the 12-bit discriminator.
    pub discriminator: Option<u16>,
    /// "VP", the vendor ID before the plus sign.
    pub vendor_id: Option<u16>,
    /// "VP", the product ID after the plus sign.
    pub product_id: Option<u16>,
    /// "CM", the commissioning mode.
    pub commissioning_mode: Option<u8>,
    /// "DN", the device name.
    pub device_name: Option<String>,
    /// "PH", the pairing hint.
    pub pairing_hint: Option<u16>,
}

fn parse_num<T: std::str::FromStr>(text: &str) -> Result<T, SetupError> {
    text.parse::<T>().map_err(|_| SetupError::Txt)
}

fn is_hex16(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parses the instance name and TXT strings ("key=value") of a commissionable
/// service. Unknown keys and attributes without a value are skipped.
pub fn commissionable_from_txt(instance: &str, txt: &[&str]) -> Result<Commissionable, SetupError> {
    if !is_hex16(instance) {
        return Err(SetupError::Txt);
    }
    let mut out = Commissionable {
        instance: instance.to_string(),
        discriminator: None,
        vendor_id: None,
        product_id: None,
        commissioning_mode: None,
        device_name: None,
        pairing_hint: None,
    };
    for entry in txt {
        let (key, value) = match entry.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match key {
            "D" => {
                let d: u16 = parse_num(value)?;
                if d > 0x0FFF {
                    return Err(SetupError::Txt);
                }
                out.discriminator = Some(d);
            }
            "VP" => {
                let (vid, pid) = match value.split_once('+') {
                    Some((v, p)) => (v, Some(p)),
                    None => (value, None),
                };
                out.vendor_id = Some(parse_num(vid)?);
                out.product_id = match pid {
                    Some(p) => Some(parse_num(p)?),
                    None => None,
                };
            }
            "CM" => out.commissioning_mode = Some(parse_num(value)?),
            "DN" => out.device_name = Some(value.to_string()),
            "PH" => out.pairing_hint = Some(parse_num(value)?),
            _ => {}
        }
    }
    Ok(out)
}

/// A commissioned node, named "<compressed fabric>-<node>" in hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operational {
    pub compressed_fabric: u64,
    pub node: u64,
}

pub fn operational_from_instance(instance: &str) -> Result<Operational, SetupError> {
    let (fabric, node) = instance.split_once('-').ok_or(SetupError::Txt)?;
    if !is_hex16(fabric) || !is_hex16(node) {
        return Err(SetupError::Txt);
    }
    Ok(Operational {
        compressed_fabric: u64::from_str_radix(fabric, 16).map_err(|_| SetupError::Txt)?,
        node: u64::from_str_radix(node, 16).map_err(|_| SetupError::Txt)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs the 88 payload bits in the order the setup payload defines.
    fn pack(version: u128, vid: u128, pid: u128, flow: u128, caps: u128, disc: u128, passcode: u128, pad: u128) -> u128 {
        version | vid << 3 | pid << 19 | flow << 35 | caps << 37 | disc << 45 | passcode << 57 | pad << 84
    }

    /// Base38 encoder for the tests, the inverse of base38_decode.
    fn base38_encode(bytes: &[u8]) -> String {
        let mut out = String::new();
        for group in bytes.chunks(3) {
            let mut raw = [0u8; 4];
            raw[..group.len()].copy_from_slice(group);
            let mut value = u32::from_le_bytes(raw);
            let chars = match group.len() {
                3 => 5,
                2 => 4,
                _ => 2,
            };
            for _ in 0..chars {
                out.push(BASE38[(value % 38) as usize] as char);
                value /= 38;
            }
        }
        out
    }

    fn qr_from_bits(bits: u128) -> String {
        format!("MT:{}", base38_encode(&bits.to_le_bytes()[..QR_BYTES]))
    }

    #[test]
    fn qr_vector_decodes_to_the_known_fields() {
        let p = parse_qr("MT:-24J0AFN00KA0648G00").unwrap();
        assert_eq!(p.vendor_id, 0xFFF1);
        assert_eq!(p.product_id, 0x8001);
        assert_eq!(p.discriminator, 3840);
        assert_eq!(p.passcode, 20_202_021);
        assert_eq!(p.flow, CommissioningFlow::Standard);
        assert!(p.on_network() && !p.ble() && !p.soft_ap());
        assert_eq!(p.short_discriminator(), 15);
        assert_eq!(base38_encode(&base38_decode("-24J0AFN00KA0648G00").unwrap()), "-24J0AFN00KA0648G00");
    }

    #[test]
    fn qr_fields_round_trip_through_base38() {
        let bits = pack(0, 0x1234, 0x5678, 1, 0x03, 0x0ABC, 12_344_321, 0);
        let p = parse_qr(&qr_from_bits(bits)).unwrap();
        assert_eq!(
            (p.vendor_id, p.product_id, p.flow, p.discovery, p.discriminator, p.passcode),
            (0x1234, 0x5678, CommissioningFlow::UserIntent, 0x03, 0x0ABC, 12_344_321)
        );
        assert!(p.ble() && p.soft_ap() && !p.on_network());
    }

    #[test]
    fn qr_rejects_bad_prefix_version_padding_flow_and_passcode() {
        let ok = pack(0, 1, 2, 0, 4, 100, 20_202_021, 0);
        let qr = qr_from_bits(ok);
        assert!(parse_qr(&qr).is_ok());
        let body = &qr[3..];
        assert_eq!(parse_qr(&format!("mt:{body}")), Err(SetupError::Prefix));
        assert_eq!(parse_qr(&qr_from_bits(pack(1, 1, 2, 0, 4, 100, 20_202_021, 0))), Err(SetupError::Version));
        assert_eq!(parse_qr(&qr_from_bits(pack(0, 1, 2, 0, 4, 100, 20_202_021, 1))), Err(SetupError::Padding));
        assert_eq!(parse_qr(&qr_from_bits(pack(0, 1, 2, 3, 4, 100, 20_202_021, 0))), Err(SetupError::Flow));
        assert_eq!(parse_qr(&qr_from_bits(pack(0, 1, 2, 0, 4, 100, 12_345_678, 0))), Err(SetupError::Passcode));
    }

    #[test]
    fn base38_rejects_bad_characters_and_group_sizes() {
        assert_eq!(base38_decode("ab"), Err(SetupError::Base38));
        assert_eq!(base38_decode("0"), Err(SetupError::Base38));
        assert_eq!(base38_decode("00000"), Ok(vec![0, 0, 0]));
        // 38^5 - 1 does not fit in three bytes.
        assert_eq!(base38_decode("ZZZZZ"), Err(SetupError::Base38));
    }

    #[test]
    fn passcode_validity_follows_the_setup_rules() {
        for bad in [0, 11_111_111, 99_999_999, 12_345_678, 87_654_321, 100_000_000] {
            assert!(!passcode_is_valid(bad), "{bad}");
        }
        for good in [1, 20_202_021, 99_999_998] {
            assert!(passcode_is_valid(good), "{good}");
        }
    }

    #[test]
    fn verhoeff_matches_the_standard_example() {
        assert_eq!(verhoeff_check(&[2, 3, 6]), 3);
    }

    #[test]
    fn manual_code_decodes_the_eleven_digit_vector() {
        let m = parse_manual_code("3497 011 2332").unwrap();
        assert_eq!((m.vendor_id, m.product_id), (None, None));
        assert_eq!(m.short_discriminator, 15);
        assert_eq!(m.passcode, 20_202_021);
    }

    #[test]
    fn manual_code_rejects_check_digit_length_and_non_digits() {
        assert_eq!(parse_manual_code("34970112333"), Err(SetupError::Check));
        assert_eq!(parse_manual_code("3497011233"), Err(SetupError::Digits));
        assert_eq!(parse_manual_code("3497O112332"), Err(SetupError::Digits));
    }

    #[test]
    fn manual_code_long_form_carries_vendor_and_product() {
        let passcode = 20_202_021u32;
        let short_disc = 15u32;
        // Vendor and product present (bit 2), then the discriminator top bits.
        let c1 = 4 | (short_disc >> 2);
        let c2 = ((short_disc & 3) << 14) | (passcode & 0x3FFF);
        let c3 = passcode >> 14;
        let body = format!("{c1}{:05}{:05}{c2:05}{c3:04}", 65_521, 32_769);
        let digits: Vec<u8> = body.bytes().map(|b| b - b'0').collect();
        let text = format!("{body}{}", verhoeff_check(&digits));
        let m = parse_manual_code(&text).unwrap();
        assert_eq!(
            (m.vendor_id, m.product_id, m.short_discriminator, m.passcode),
            (Some(65_521), Some(32_769), 15, passcode)
        );
    }

    #[test]
    fn commissionable_txt_records_parse_known_keys_and_skip_unknown() {
        let c = commissionable_from_txt(
            "0123456789ABCDEF",
            &["D=3840", "VP=65521+32769", "CM=1", "DN=Lamp", "PH=33", "flag", "XX=9"],
        )
        .unwrap();
        assert_eq!(c.discriminator, Some(3840));
        assert_eq!((c.vendor_id, c.product_id), (Some(65_521), Some(32_769)));
        assert_eq!(c.commissioning_mode, Some(1));
        assert_eq!(c.device_name.as_deref(), Some("Lamp"));
        assert_eq!(c.pairing_hint, Some(33));
    }

    #[test]
    fn commissionable_txt_rejects_bad_discriminator_and_instance() {
        assert_eq!(commissionable_from_txt("0123456789ABCDEF", &["D=4096"]), Err(SetupError::Txt));
        assert_eq!(commissionable_from_txt("0123", &["D=1"]), Err(SetupError::Txt));
        assert_eq!(commissionable_from_txt("0123456789ABCDEG", &[]), Err(SetupError::Txt));
    }

    #[test]
    fn service_names_and_operational_instances_parse() {
        assert_eq!(service_kind("_matterc._udp.local."), Some(ServiceKind::Commissionable));
        assert_eq!(service_kind("_MATTER._tcp"), Some(ServiceKind::Operational));
        assert_eq!(service_kind("_http._tcp"), None);
        let o = operational_from_instance("0102030405060708-00000000000000AB").unwrap();
        assert_eq!((o.compressed_fabric, o.node), (0x0102_0304_0506_0708, 0xAB));
        assert_eq!(operational_from_instance("0102030405060708"), Err(SetupError::Txt));
    }
}
