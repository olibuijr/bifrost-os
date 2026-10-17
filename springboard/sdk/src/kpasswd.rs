//! The kpasswd wire framing of the Kerberos password service (`com.apple.Kerberos.kpasswdd`), as RFC 3244 defines it.
//!
//! A request is a 2-byte total length, a 2-byte version (`0x0001` change password, `0xff80` set password), a 2-byte
//! length of the AP-REQ, the AP-REQ, then the KRB-PRIV that carries the new password. A reply is a 2-byte total length,
//! a 2-byte result code, then the KRB-PRIV (on success) or the KRB-ERROR (on failure). All lengths and codes are big
//! endian and the total lengths count the header.
//!
//! This module frames and unframes those bytes. It does not read or check the tickets: the AP-REQ and KRB-PRIV are
//! opaque here, so the Kerberos library (or the krb5 tools) must verify them before the password changes.

use std::fmt;

pub const VERSION_CHANGE: u16 = 0x0001;
pub const VERSION_SET: u16 = 0xff80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// The caller changes its own password and proves knowledge of the old one through its ticket.
    Change,
    /// An administrator sets another principal's password.
    Set,
}

impl Version {
    fn wire(self) -> u16 {
        match self {
            Version::Change => VERSION_CHANGE,
            Version::Set => VERSION_SET,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCode {
    Success,
    Malformed,
    HardError,
    AuthError,
    SoftError,
    AccessDenied,
    BadVersion,
    InitialFlagNeeded,
}

impl ResultCode {
    pub fn wire(self) -> u16 {
        match self {
            ResultCode::Success => 0,
            ResultCode::Malformed => 1,
            ResultCode::HardError => 2,
            ResultCode::AuthError => 3,
            ResultCode::SoftError => 4,
            ResultCode::AccessDenied => 5,
            ResultCode::BadVersion => 6,
            ResultCode::InitialFlagNeeded => 7,
        }
    }

    pub fn from_wire(code: u16) -> Result<ResultCode, KpasswdError> {
        Ok(match code {
            0 => ResultCode::Success,
            1 => ResultCode::Malformed,
            2 => ResultCode::HardError,
            3 => ResultCode::AuthError,
            4 => ResultCode::SoftError,
            5 => ResultCode::AccessDenied,
            6 => ResultCode::BadVersion,
            7 => ResultCode::InitialFlagNeeded,
            other => return Err(KpasswdError::UnknownResult(other)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub version: Version,
    pub ap_req: Vec<u8>,
    pub krb_priv: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub result: ResultCode,
    /// The KRB-PRIV on success, the KRB-ERROR otherwise. Opaque to this module.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KpasswdError {
    Truncated,
    LengthMismatch { declared: usize, actual: usize },
    UnknownVersion(u16),
    UnknownResult(u16),
    TooLong(usize),
}

impl fmt::Display for KpasswdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KpasswdError::Truncated => write!(f, "kpasswd message is shorter than its header"),
            KpasswdError::LengthMismatch { declared, actual } => {
                write!(f, "kpasswd message declares {declared} bytes but has {actual}")
            }
            KpasswdError::UnknownVersion(v) => write!(f, "kpasswd version {v:#06x} is not known"),
            KpasswdError::UnknownResult(c) => write!(f, "kpasswd result code {c} is not known"),
            KpasswdError::TooLong(n) => write!(f, "{n} bytes do not fit a kpasswd length field"),
        }
    }
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn len16(n: usize) -> Result<u16, KpasswdError> {
    u16::try_from(n).map_err(|_| KpasswdError::TooLong(n))
}

/// Frames a request. The AP-REQ and the total must each fit the 2-byte length field.
pub fn encode_request(req: &Request) -> Result<Vec<u8>, KpasswdError> {
    let total = 6 + req.ap_req.len() + req.krb_priv.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&len16(total)?.to_be_bytes());
    out.extend_from_slice(&req.version.wire().to_be_bytes());
    out.extend_from_slice(&len16(req.ap_req.len())?.to_be_bytes());
    out.extend_from_slice(&req.ap_req);
    out.extend_from_slice(&req.krb_priv);
    Ok(out)
}

/// Unframes a request. The declared length must match the bytes given, and the AP-REQ must fit inside them.
pub fn decode_request(bytes: &[u8]) -> Result<Request, KpasswdError> {
    if bytes.len() < 6 {
        return Err(KpasswdError::Truncated);
    }
    let declared = be16(bytes) as usize;
    if declared != bytes.len() {
        return Err(KpasswdError::LengthMismatch { declared, actual: bytes.len() });
    }
    let version = match be16(&bytes[2..]) {
        VERSION_CHANGE => Version::Change,
        VERSION_SET => Version::Set,
        other => return Err(KpasswdError::UnknownVersion(other)),
    };
    let ap_len = be16(&bytes[4..]) as usize;
    let rest = &bytes[6..];
    if ap_len > rest.len() {
        return Err(KpasswdError::Truncated);
    }
    Ok(Request { version, ap_req: rest[..ap_len].to_vec(), krb_priv: rest[ap_len..].to_vec() })
}

/// Frames a reply.
pub fn encode_reply(reply: &Reply) -> Result<Vec<u8>, KpasswdError> {
    let total = 4 + reply.payload.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&len16(total)?.to_be_bytes());
    out.extend_from_slice(&reply.result.wire().to_be_bytes());
    out.extend_from_slice(&reply.payload);
    Ok(out)
}

/// Unframes a reply.
pub fn decode_reply(bytes: &[u8]) -> Result<Reply, KpasswdError> {
    if bytes.len() < 4 {
        return Err(KpasswdError::Truncated);
    }
    let declared = be16(bytes) as usize;
    if declared != bytes.len() {
        return Err(KpasswdError::LengthMismatch { declared, actual: bytes.len() });
    }
    Ok(Reply { result: ResultCode::from_wire(be16(&bytes[2..]))?, payload: bytes[4..].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(version: Version) -> Request {
        Request { version, ap_req: vec![0x6e, 0x01, 0x02], krb_priv: vec![0x75, 0x09, 0xaa] }
    }

    #[test]
    fn a_change_request_has_the_rfc_header() {
        let bytes = encode_request(&request(Version::Change)).unwrap();
        assert_eq!(bytes, [0x00, 0x0c, 0x00, 0x01, 0x00, 0x03, 0x6e, 0x01, 0x02, 0x75, 0x09, 0xaa]);
    }

    #[test]
    fn a_request_round_trips_for_both_versions() {
        for v in [Version::Change, Version::Set] {
            let req = request(v);
            assert_eq!(decode_request(&encode_request(&req).unwrap()).unwrap(), req);
        }
    }

    #[test]
    fn the_set_version_is_0xff80() {
        let bytes = encode_request(&request(Version::Set)).unwrap();
        assert_eq!(&bytes[2..4], &[0xff, 0x80]);
    }

    #[test]
    fn a_length_that_does_not_match_the_bytes_is_refused() {
        let mut bytes = encode_request(&request(Version::Change)).unwrap();
        bytes.push(0);
        assert_eq!(decode_request(&bytes), Err(KpasswdError::LengthMismatch { declared: 12, actual: 13 }));
    }

    #[test]
    fn an_ap_req_longer_than_the_message_is_refused() {
        let bytes = [0x00, 0x07, 0x00, 0x01, 0x00, 0x05, 0x01];
        assert_eq!(decode_request(&bytes), Err(KpasswdError::Truncated));
    }

    #[test]
    fn an_unknown_version_is_refused() {
        let bytes = [0x00, 0x06, 0x00, 0x02, 0x00, 0x00];
        assert_eq!(decode_request(&bytes), Err(KpasswdError::UnknownVersion(2)));
    }

    #[test]
    fn a_reply_round_trips_and_keeps_its_result_code() {
        let reply = Reply { result: ResultCode::AuthError, payload: vec![1, 2, 3] };
        let bytes = encode_reply(&reply).unwrap();
        assert_eq!(bytes, [0x00, 0x07, 0x00, 0x03, 1, 2, 3]);
        assert_eq!(decode_reply(&bytes).unwrap(), reply);
    }

    #[test]
    fn every_result_code_has_its_wire_number() {
        let all = [
            ResultCode::Success,
            ResultCode::Malformed,
            ResultCode::HardError,
            ResultCode::AuthError,
            ResultCode::SoftError,
            ResultCode::AccessDenied,
            ResultCode::BadVersion,
            ResultCode::InitialFlagNeeded,
        ];
        for (i, code) in all.iter().enumerate() {
            assert_eq!(code.wire() as usize, i);
            assert_eq!(ResultCode::from_wire(i as u16).unwrap(), *code);
        }
        assert_eq!(ResultCode::from_wire(8), Err(KpasswdError::UnknownResult(8)));
    }

    #[test]
    fn a_short_reply_is_refused() {
        assert_eq!(decode_reply(&[0x00, 0x02]), Err(KpasswdError::Truncated));
        assert_eq!(decode_reply(&[0x00, 0x05, 0x00, 0x00]), Err(KpasswdError::LengthMismatch { declared: 5, actual: 4 }));
    }

    #[test]
    fn a_field_too_big_for_its_length_is_refused() {
        let req = Request { version: Version::Change, ap_req: vec![0; 70_000], krb_priv: vec![] };
        assert!(matches!(encode_request(&req), Err(KpasswdError::TooLong(_))));
    }
}
