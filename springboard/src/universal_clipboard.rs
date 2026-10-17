//! Universal Clipboard: a local copy goes to the account's other devices, sealed with the account key, and lives in the
//! cloud for [`TTL_SECS`] at most. This module is the pure part: which copies may leave the device, the payload and its
//! seal, and when a remote copy is still offered. The transport (the watch, `PUT` and long-poll of
//! `/api/cloud/v1/clipboard`) is the shell's; the request builders are `bifrost_app::cloud_clipboard`.
//!
//! Privacy rules, in order: the setting is off until the person turns it on (`Options::universal_clipboard`); a copy
//! carrying a concealed or transient marker (what password managers and secure fields set) is never shared, whatever
//! else it holds; only text, links and PNG or JPEG images travel, at most [`MAX_BYTES`] each; the cloud stores one
//! opaque blob per account and cannot open it.

use crate::cloud_keys::{self, SealError, Sealed};
use bifrost_core::json::{self, Json};
use bifrost_core::plist::{base64_decode, base64_encode};

/// The AEAD additional data of a clipboard blob. Each feature has its own, so no other blob opens as this one.
pub const AAD: &[u8] = b"bifrost.clipboard.v1";
/// The HPKE `info` of a clipboard blob.
pub const INFO: &[u8] = b"clipboard";
/// How long the cloud keeps an item, and how long a device offers it on paste (seconds).
pub const TTL_SECS: i64 = 120;
/// The largest single representation that may be shared (bytes, before base64).
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

/// Text, links and images: the only types that may leave the device.
const SHAREABLE: [&str; 5] = ["public.utf8-plain-text", "public.plain-text", "public.url", "public.png", "public.jpeg"];
/// Type prefixes that mark a copy as secret. Password managers and secure fields set one of these; a copy with any of
/// them is refused whole. `org.nspasteboard.` is the community marker family (concealed, transient, auto-generated).
const CONCEALED_PREFIXES: [&str; 7] = [
    "org.nspasteboard.",
    "com.agilebits.",
    "com.1password.",
    "com.bitwarden.",
    "com.lastpass.",
    "org.keepassxc.",
    "de.petermaurer.",
];

/// Why a copy, or part of it, stayed on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The copy carries a concealed or transient marker: nothing from it is shared.
    Concealed,
    /// Nothing in the copy is a shareable type.
    NothingShareable,
    /// The only shareable types were larger than [`MAX_BYTES`].
    TooLarge,
}

/// The pasteboard item carrying a type marked as concealed or transient, by prefix or by name.
pub fn is_concealed(uti: &str) -> bool {
    CONCEALED_PREFIXES.iter().any(|p| uti.starts_with(p)) || uti.to_ascii_lowercase().contains("concealed")
}

/// Chooses what of a copy may leave the device. Refuses the whole copy when any of its types is concealed; otherwise
/// keeps the shareable representations no larger than [`MAX_BYTES`].
pub fn shareable(items: &[(String, Vec<u8>)]) -> Result<Vec<(String, Vec<u8>)>, Refusal> {
    if items.iter().any(|(uti, _)| is_concealed(uti)) {
        return Err(Refusal::Concealed);
    }
    let mut out = Vec::new();
    let mut too_large = false;
    for (uti, data) in items {
        if !SHAREABLE.contains(&uti.as_str()) {
            continue;
        }
        if data.len() > MAX_BYTES {
            too_large = true;
            continue;
        }
        out.push((uti.clone(), data.clone()));
    }
    if out.is_empty() {
        Err(if too_large { Refusal::TooLarge } else { Refusal::NothingShareable })
    } else {
        Ok(out)
    }
}

/// One copy as it travels: its representations, the device that copied it, and when, in Unix milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    pub types: Vec<(String, Vec<u8>)>,
    pub origin: String,
    pub at_ms: i64,
}

/// The payload as JSON: `types` (each `uti` and base64 `data`), `origin` and `time` (Unix milliseconds).
pub fn encode(p: &Payload) -> Vec<u8> {
    let types = p
        .types
        .iter()
        .map(|(uti, data)| Json::Obj(vec![("uti".into(), Json::Str(uti.clone())), ("data".into(), Json::Str(base64_encode(data)))]))
        .collect();
    Json::Obj(vec![
        ("types".into(), Json::Arr(types)),
        ("origin".into(), Json::Str(p.origin.clone())),
        ("time".into(), Json::Num(p.at_ms as f64)),
    ])
    .encode()
    .into_bytes()
}

/// Reads a payload back. `None` for anything that is not one.
pub fn decode(bytes: &[u8]) -> Option<Payload> {
    let j = json::parse(std::str::from_utf8(bytes).ok()?).ok()?;
    let types = j
        .get("types")?
        .as_array()?
        .iter()
        .map(|t| Some((t.str_field("uti")?, base64_decode(&t.str_field("data")?)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(Payload { types, origin: j.str_field("origin")?, at_ms: j.get("time")?.number()? as i64 })
}

/// Seals a payload to the account public key (32 raw bytes) with the caller's ephemeral private key.
pub fn seal(account_public: &[u8], p: &Payload, ephemeral: &[u8]) -> Result<Sealed, SealError> {
    cloud_keys::seal(account_public, AAD, INFO, &encode(p), ephemeral)
}

/// Opens a blob the cloud returned, with the account private key. `None` when it does not open or is not a payload.
pub fn open(account_private: &[u8], blob: &Sealed) -> Option<Payload> {
    decode(&cloud_keys::open(account_private, AAD, INFO, blob).ok()?)
}

/// Whether the cloud still holds an item: it deletes it at `expires_at` (Unix seconds).
pub fn live(expires_at_secs: i64, now_secs: i64) -> bool {
    now_secs < expires_at_secs
}

/// Whether a remote copy is offered on paste now: it is no more than [`TTL_SECS`] old, and it is newer than the last
/// copy made on this device (`local_at_ms`, `None` when this device has copied nothing yet).
pub fn offer(remote_at_ms: i64, local_at_ms: Option<i64>, now_ms: i64) -> bool {
    now_ms - remote_at_ms <= TTL_SECS * 1000 && local_at_ms.map_or(true, |local| remote_at_ms > local)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(uti: &str, data: &[u8]) -> (String, Vec<u8>) {
        (uti.to_string(), data.to_vec())
    }

    #[test]
    fn text_and_links_are_shareable_and_other_types_are_left_out() {
        let kept = shareable(&[item("public.utf8-plain-text", b"hi"), item("public.html", b"<b>hi</b>"), item("public.url", b"https://x")]).unwrap();
        assert_eq!(kept, vec![item("public.utf8-plain-text", b"hi"), item("public.url", b"https://x")]);
    }

    #[test]
    fn a_concealed_copy_is_refused_whole() {
        let copy = [item("public.utf8-plain-text", b"hunter2"), item("org.nspasteboard.ConcealedType", b"")];
        assert_eq!(shareable(&copy), Err(Refusal::Concealed));
        let manager = [item("com.agilebits.onepassword", b""), item("public.utf8-plain-text", b"pw")];
        assert_eq!(shareable(&manager), Err(Refusal::Concealed));
        assert_eq!(shareable(&[item("org.nspasteboard.TransientType", b"")]), Err(Refusal::Concealed));
    }

    #[test]
    fn a_file_copy_is_not_shareable() {
        assert_eq!(shareable(&[item("public.file-url", b"file:///x")]), Err(Refusal::NothingShareable));
        assert_eq!(shareable(&[]), Err(Refusal::NothingShareable));
    }

    #[test]
    fn an_oversize_image_is_dropped_and_the_rest_kept() {
        let big = vec![0u8; MAX_BYTES + 1];
        assert_eq!(shareable(&[item("public.png", &big)]), Err(Refusal::TooLarge));
        let kept = shareable(&[item("public.png", &big), item("public.utf8-plain-text", b"ok")]).unwrap();
        assert_eq!(kept, vec![item("public.utf8-plain-text", b"ok")]);
        let at_limit = vec![0u8; MAX_BYTES];
        assert!(shareable(&[item("public.png", &at_limit)]).is_ok());
    }

    fn payload() -> Payload {
        Payload { types: vec![item("public.utf8-plain-text", b"copied text")], origin: "dev-1".into(), at_ms: 1_700_000_000_123 }
    }

    #[test]
    fn a_payload_survives_the_json_codec() {
        assert_eq!(decode(&encode(&payload())), Some(payload()));
        assert_eq!(decode(b"{\"types\":3}"), None);
    }

    #[test]
    fn a_payload_round_trips_through_the_account_key_only() {
        let account_private = [0x11u8; 32];
        let account_public = cloud_keys::public_key(&account_private).unwrap();
        let blob = seal(&account_public, &payload(), &[0x22; 32]).unwrap();
        assert!(!blob.sealed.contains("copied"), "the text is not in the clear");
        assert_eq!(open(&account_private, &blob), Some(payload()));
        assert_eq!(open(&[0x33u8; 32], &blob), None, "another key does not open it");
    }

    #[test]
    fn an_item_is_live_until_its_ttl_runs_out() {
        assert!(live(1_000 + TTL_SECS, 1_000));
        assert!(live(1_000 + TTL_SECS, 1_000 + TTL_SECS - 1));
        assert!(!live(1_000 + TTL_SECS, 1_000 + TTL_SECS));
    }

    #[test]
    fn a_remote_copy_is_offered_only_when_fresh_and_newer() {
        let now = 1_000_000;
        assert!(offer(now - 10_000, None, now), "fresh and this device has copied nothing");
        assert!(offer(now - 10_000, Some(now - 20_000), now), "newer than the last local copy");
        assert!(!offer(now - 10_000, Some(now - 5_000), now), "older than the last local copy");
        assert!(!offer(now - TTL_SECS * 1000 - 1, None, now), "older than two minutes");
        assert!(offer(now - TTL_SECS * 1000, None, now), "exactly two minutes is still offered");
    }
}
