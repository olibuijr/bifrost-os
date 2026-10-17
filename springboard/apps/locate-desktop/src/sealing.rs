//! Sealed locations. A device seals its report with HPKE (RFC 9180 base mode, the `bifrost_ui::hpke` suite) to the
//! account's public key, so only a device that holds the account private key can open it. The cloud stores the two
//! base64 blobs, `sealed` and `enc`, and nothing else about the position.
//!
//! The `info` string binds a blob to what it describes: the device id for a device's own report, the pending device's
//! id for an account key handed to an enrolling device, and the share id for a copy sent to a contact. The caller
//! supplies the sender's ephemeral private key from the random pool, so this module is deterministic and testable.

use bifrost_app::controls::hpke;
use bifrost_app::controls::x25519::PrivateKey;
use bifrost_core::json::{self, Json};
use bifrost_core::plist::{base64_decode, base64_encode};

/// The AEAD additional data for every sealed blob of the locator.
const AAD: &[u8] = b"bifrost.locate.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    /// base64 of the ciphertext (AES-128-GCM, with its tag).
    pub sealed: String,
    /// base64 of the HPKE encapsulated key.
    pub enc: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// A key is not 32 bytes, or it is a low-order point.
    Key,
    /// The blob did not authenticate: wrong key, wrong `info`, or a changed byte.
    Open,
    /// A blob or a payload is not valid base64 or JSON, or lacks a field.
    Payload,
}

/// A position fix as a report carries it: degrees, metres of accuracy when known, the time it was taken (Unix seconds)
/// and this device's battery level when known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: Option<f64>,
    pub at_unix: i64,
    pub battery: Option<u8>,
}

/// Seals `plaintext` to `recipient_public` (32 raw bytes) under `info`, with the sender's `ephemeral` private key.
pub fn seal(recipient_public: &[u8], info: &[u8], plaintext: &[u8], ephemeral: &[u8]) -> Result<Sealed, SealError> {
    let (enc, ct) = hpke::seal_base(recipient_public, info, AAD, plaintext, ephemeral).map_err(|_| SealError::Key)?;
    Ok(Sealed { sealed: base64_encode(&ct), enc: base64_encode(&enc) })
}

/// Opens a blob with the recipient's private key (32 raw bytes) and the `info` it was sealed under.
pub fn open(recipient_private: &[u8], info: &[u8], blob: &Sealed) -> Result<Vec<u8>, SealError> {
    let enc = base64_decode(&blob.enc).ok_or(SealError::Payload)?;
    let ct = base64_decode(&blob.sealed).ok_or(SealError::Payload)?;
    hpke::open_base(&enc, recipient_private, info, AAD, &ct).map_err(|_| SealError::Open)
}

/// Seals a report to the account public key, with `info` the device id.
pub fn seal_report(account_public: &[u8], device_id: &str, report: &Report, ephemeral: &[u8]) -> Result<Sealed, SealError> {
    seal(account_public, device_id.as_bytes(), &encode_report(report), ephemeral)
}

/// Opens a device's sealed report with the account private key.
pub fn open_report(account_private: &[u8], device_id: &str, blob: &Sealed) -> Result<Report, SealError> {
    decode_report(&open(account_private, device_id.as_bytes(), blob)?)
}

/// Seals the account private key to an enrolling device's public key. `pending_id` is the `info`.
pub fn seal_account_key(device_public: &[u8], pending_id: &str, account_private: &[u8], ephemeral: &[u8]) -> Result<Sealed, SealError> {
    seal(device_public, pending_id.as_bytes(), account_private, ephemeral)
}

/// Opens the account private key with the enrolling device's private key.
pub fn open_account_key(device_private: &[u8], pending_id: &str, blob: &Sealed) -> Result<PrivateKey, SealError> {
    let raw = open(device_private, pending_id.as_bytes(), blob)?;
    PrivateKey::from_raw(&raw).map_err(|_| SealError::Key)
}

/// The report as JSON: `lat`, `lon`, `timestamp`, and `accuracy` and `battery` when known.
pub fn encode_report(r: &Report) -> Vec<u8> {
    let mut fields = vec![
        ("lat".to_string(), Json::Num(r.lat)),
        ("lon".to_string(), Json::Num(r.lon)),
        ("timestamp".to_string(), Json::Num(r.at_unix as f64)),
    ];
    if let Some(a) = r.accuracy_m {
        fields.push(("accuracy".to_string(), Json::Num(a)));
    }
    if let Some(b) = r.battery {
        fields.push(("battery".to_string(), Json::Num(b.min(100) as f64)));
    }
    Json::Obj(fields).encode().into_bytes()
}

/// Reads a report back. Coordinates must be in range; the battery is clamped to 100 percent.
pub fn decode_report(bytes: &[u8]) -> Result<Report, SealError> {
    let text = std::str::from_utf8(bytes).map_err(|_| SealError::Payload)?;
    let j = json::parse(text).map_err(|_| SealError::Payload)?;
    let num = |key: &str| j.get(key).and_then(Json::number).filter(|n| n.is_finite());
    let lat = num("lat").filter(|v| v.abs() <= 90.0).ok_or(SealError::Payload)?;
    let lon = num("lon").filter(|v| v.abs() <= 180.0).ok_or(SealError::Payload)?;
    let at_unix = num("timestamp").ok_or(SealError::Payload)? as i64;
    Ok(Report {
        lat,
        lon,
        accuracy_m: num("accuracy").filter(|a| *a >= 0.0),
        at_unix,
        battery: num("battery").map(|b| b.clamp(0.0, 100.0) as u8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed test keys: the private scalar is any 32 bytes, the public key is derived from it.
    fn keys(seed: u8) -> ([u8; 32], [u8; 32]) {
        let private = [seed; 32];
        let public = PrivateKey::from_raw(&private).unwrap().public_key().raw();
        (private, public)
    }

    fn report() -> Report {
        Report { lat: 64.1466, lon: -21.9426, accuracy_m: Some(12.0), at_unix: 1_700_000_000, battery: Some(82) }
    }

    #[test]
    fn a_report_round_trips_through_the_account_key() {
        let (account_private, account_public) = keys(0x11);
        let blob = seal_report(&account_public, "dev-1", &report(), &[0x22; 32]).unwrap();
        assert_eq!(open_report(&account_private, "dev-1", &blob).unwrap(), report());
    }

    #[test]
    fn the_blob_is_opaque_and_only_the_account_key_opens_it() {
        let (_, account_public) = keys(0x11);
        let (other_private, _) = keys(0x33);
        let blob = seal_report(&account_public, "dev-1", &report(), &[0x22; 32]).unwrap();
        assert!(!blob.sealed.contains("64.14"), "the coordinates are not in the clear");
        assert_eq!(open_report(&other_private, "dev-1", &blob), Err(SealError::Open));
    }

    #[test]
    fn the_info_binds_the_blob_to_its_device() {
        let (account_private, account_public) = keys(0x11);
        let blob = seal_report(&account_public, "dev-1", &report(), &[0x22; 32]).unwrap();
        assert_eq!(open_report(&account_private, "dev-2", &blob), Err(SealError::Open));
    }

    #[test]
    fn a_changed_byte_fails_to_open() {
        let (account_private, account_public) = keys(0x11);
        let mut blob = seal_report(&account_public, "dev-1", &report(), &[0x22; 32]).unwrap();
        let mut ct = base64_decode(&blob.sealed).unwrap();
        ct[0] ^= 1;
        blob.sealed = base64_encode(&ct);
        assert_eq!(open_report(&account_private, "dev-1", &blob), Err(SealError::Open));
    }

    #[test]
    fn a_blob_that_is_not_base64_is_a_payload_error() {
        let (account_private, _) = keys(0x11);
        let bad = Sealed { sealed: "!!".into(), enc: "!!".into() };
        assert_eq!(open(&account_private, b"dev-1", &bad), Err(SealError::Payload));
    }

    #[test]
    fn the_account_key_is_handed_to_an_enrolling_device_only() {
        let (account_private, _) = keys(0x11);
        let (device_private, device_public) = keys(0x44);
        let blob = seal_account_key(&device_public, "p1", &account_private, &[0x55; 32]).unwrap();
        let opened = open_account_key(&device_private, "p1", &blob).unwrap();
        assert_eq!(opened.raw(), account_private);
        let (stranger_private, _) = keys(0x66);
        assert!(open_account_key(&stranger_private, "p1", &blob).is_err());
        assert!(open_account_key(&device_private, "p2", &blob).is_err(), "bound to the pending id");
    }

    #[test]
    fn a_report_without_a_position_or_with_a_bad_one_is_refused() {
        assert_eq!(decode_report(br#"{"lon":1,"timestamp":1}"#), Err(SealError::Payload));
        assert_eq!(decode_report(br#"{"lat":95,"lon":1,"timestamp":1}"#), Err(SealError::Payload));
        assert_eq!(decode_report(b"not json"), Err(SealError::Payload));
    }

    #[test]
    fn the_battery_is_clamped_and_optional_fields_stay_absent() {
        let r = decode_report(br#"{"lat":1,"lon":2,"timestamp":3,"battery":250}"#).unwrap();
        assert_eq!(r.battery, Some(100));
        assert_eq!(r.accuracy_m, None);
        let bare = Report { battery: None, accuracy_m: None, ..report() };
        let text = String::from_utf8(encode_report(&bare)).unwrap();
        assert!(!text.contains("battery") && !text.contains("accuracy"), "{text}");
    }
}
