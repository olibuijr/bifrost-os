//! Certificate Transparency: reads the signed certificate timestamps (SCTs) a server's certificate carries in the
//! embedded SCT list extension (RFC 6962, OID 1.3.6.1.4.1.11129.2.4.2). Parsing is all this module does. The
//! signatures are not checked yet, and no log list ships, so the TLS client does not enforce Certificate
//! Transparency; [`count_from_logs`] is the policy input once both exist.

/// The DER encoding of the SCT list extension's OID, 1.3.6.1.4.1.11129.2.4.2.
const SCT_LIST_OID: [u8; 12] = [0x06, 0x0a, 0x2b, 0x06, 0x01, 0x04, 0x01, 0xd6, 0x79, 0x02, 0x04, 0x02];

/// One signed certificate timestamp: the log that issued it and when, with the signature it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sct {
    /// The SCT version; version 0 is RFC 6962.
    pub version: u8,
    /// The SHA-256 digest of the log's public key.
    pub log_id: [u8; 32],
    /// Milliseconds since the Unix epoch, when the log accepted the certificate.
    pub timestamp_ms: u64,
    /// The hash and signature algorithm bytes of the signature (RFC 5246, section 7.4.1.4.1).
    pub algorithms: (u8, u8),
    pub signature: Vec<u8>,
}

fn slice(bytes: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    bytes.get(start..start.checked_add(len)?)
}

fn be_u16(bytes: &[u8], at: usize) -> Option<usize> {
    Some(((*bytes.get(at)? as usize) << 8) | *bytes.get(at + 1)? as usize)
}

/// Reads a DER tag and length at `pos`: the tag, the content length and where the content starts.
fn der_header(bytes: &[u8], pos: usize) -> Option<(u8, usize, usize)> {
    let tag = *bytes.get(pos)?;
    let first = *bytes.get(pos + 1)? as usize;
    if first < 0x80 {
        return Some((tag, first, pos + 2));
    }
    let count = first & 0x7f;
    if count == 0 || count > 4 {
        return None;
    }
    let mut len = 0usize;
    for i in 0..count {
        len = (len << 8) | *bytes.get(pos + 2 + i)? as usize;
    }
    Some((tag, len, pos + 2 + count))
}

/// The SCTs in a DER certificate's SCT list extension, or None when the certificate has no such extension or its
/// list is malformed. The extension is found by its OID, which is unique in a certificate; the rest of the
/// certificate is not parsed.
pub fn scts_in_certificate(der: &[u8]) -> Option<Vec<Sct>> {
    let at = der.windows(SCT_LIST_OID.len()).position(|w| w == SCT_LIST_OID)?;
    let mut pos = at + SCT_LIST_OID.len();
    // the extension may be marked critical: BOOLEAN TRUE
    if der.get(pos..pos + 3) == Some(&[0x01, 0x01, 0xff][..]) {
        pos += 3;
    }
    // extnValue is an OCTET STRING holding a DER OCTET STRING, which holds the TLS-encoded list
    let (tag, len, body) = der_header(der, pos)?;
    if tag != 0x04 {
        return None;
    }
    let outer = slice(der, body, len)?;
    let (tag, len, body) = der_header(outer, 0)?;
    if tag != 0x04 {
        return None;
    }
    parse_sct_list(slice(outer, body, len)?)
}

/// Parses a TLS-encoded SCT list (RFC 6962, section 3.3): a 2-byte total length, then each SCT with its own 2-byte
/// length. The list must be exactly that long. None when it is malformed.
pub fn parse_sct_list(list: &[u8]) -> Option<Vec<Sct>> {
    let total = be_u16(list, 0)?;
    if list.len() != 2 + total {
        return None;
    }
    let mut rest = slice(list, 2, total)?;
    let mut scts = Vec::new();
    while !rest.is_empty() {
        let len = be_u16(rest, 0)?;
        scts.push(parse_sct(slice(rest, 2, len)?)?);
        rest = &rest[2 + len..];
    }
    Some(scts)
}

/// Parses one SCT: version, log ID, timestamp, extensions, then the digitally-signed structure. None when a field is
/// missing or the lengths do not add up to the item.
fn parse_sct(item: &[u8]) -> Option<Sct> {
    let version = *item.first()?;
    let log_id: [u8; 32] = item.get(1..33)?.try_into().ok()?;
    let timestamp_ms = u64::from_be_bytes(item.get(33..41)?.try_into().ok()?);
    let ext_len = be_u16(item, 41)?;
    let at = 43 + ext_len;
    let algorithms = (*item.get(at)?, *item.get(at + 1)?);
    let sig_len = be_u16(item, at + 2)?;
    if at + 4 + sig_len != item.len() {
        return None;
    }
    let signature = slice(item, at + 4, sig_len)?.to_vec();
    Some(Sct { version, log_id, timestamp_ms, algorithms, signature })
}

/// How many distinct logs in `logs` issued one of `scts`. A certificate is logged when enough distinct logs
/// issued its SCTs; this counts them and does not check the signatures.
pub fn count_from_logs(scts: &[Sct], logs: &[[u8; 32]]) -> usize {
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for sct in scts {
        if logs.contains(&sct.log_id) && !seen.contains(&sct.log_id) {
            seen.push(sct.log_id);
        }
    }
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if body.len() < 0x80 {
            out.push(body.len() as u8);
        } else if body.len() < 0x100 {
            out.extend_from_slice(&[0x81, body.len() as u8]);
        } else {
            out.extend_from_slice(&[0x82, (body.len() >> 8) as u8, body.len() as u8]);
        }
        out.extend_from_slice(body);
        out
    }

    /// One SCT of version 0 from the log whose ID is `log` bytes, with no extensions and an ECDSA SHA-256 signature.
    fn sct(log: u8, timestamp: u64, signature: &[u8]) -> Vec<u8> {
        let mut item = vec![0u8];
        item.extend_from_slice(&[log; 32]);
        item.extend_from_slice(&timestamp.to_be_bytes());
        item.extend_from_slice(&[0, 0]);
        item.extend_from_slice(&[4, 3]);
        item.extend_from_slice(&(signature.len() as u16).to_be_bytes());
        item.extend_from_slice(signature);
        let mut out = (item.len() as u16).to_be_bytes().to_vec();
        out.extend(item);
        out
    }

    fn list(entries: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = entries.concat();
        let mut out = (body.len() as u16).to_be_bytes().to_vec();
        out.extend(body);
        out
    }

    /// A stand-in certificate: some DER, then the critical SCT list extension, then more DER.
    fn certificate(list: &[u8]) -> Vec<u8> {
        let mut der = vec![0x30, 0x03, 0x02, 0x01, 0x05];
        der.extend_from_slice(&SCT_LIST_OID);
        der.extend_from_slice(&[0x01, 0x01, 0xff]);
        der.extend(tlv(0x04, &tlv(0x04, list)));
        der.extend_from_slice(&[0x05, 0x00]);
        der
    }

    #[test]
    fn scts_are_read_from_the_certificate_extension() {
        let der = certificate(&list(&[sct(1, 1000, &[9, 9]), sct(2, 2000, &[7])]));
        let scts = scts_in_certificate(&der).unwrap();
        assert_eq!(scts.len(), 2);
        assert_eq!(scts[0].log_id, [1; 32]);
        assert_eq!(scts[0].timestamp_ms, 1000);
        assert_eq!(scts[0].algorithms, (4, 3));
        assert_eq!(scts[0].signature, vec![9, 9]);
        assert_eq!(scts[1].log_id, [2; 32]);
        assert_eq!(scts[1].version, 0);
    }

    #[test]
    fn a_certificate_without_the_extension_has_no_scts() {
        assert!(scts_in_certificate(&[0x30, 0x03, 0x02, 0x01, 0x05]).is_none());
    }

    #[test]
    fn a_list_with_trailing_bytes_is_refused() {
        let mut bytes = list(&[sct(1, 1000, &[9])]);
        bytes.push(0);
        assert!(parse_sct_list(&bytes).is_none());
    }

    #[test]
    fn a_truncated_list_is_refused() {
        let mut bytes = list(&[sct(1, 1000, &[9])]);
        bytes.truncate(bytes.len() - 1);
        assert!(parse_sct_list(&bytes).is_none());
    }

    #[test]
    fn an_sct_whose_signature_runs_past_its_item_is_refused() {
        let mut entry = sct(1, 1000, &[9, 9]);
        entry.pop();
        let bytes = list(&[entry]);
        assert!(parse_sct_list(&bytes).is_none());
    }

    #[test]
    fn distinct_known_logs_are_counted_once_each() {
        let scts = parse_sct_list(&list(&[sct(1, 1, &[]), sct(1, 2, &[]), sct(2, 3, &[])])).unwrap();
        assert_eq!(count_from_logs(&scts, &[]), 0);
        assert_eq!(count_from_logs(&scts, &[[1; 32]]), 1);
        assert_eq!(count_from_logs(&scts, &[[1; 32], [2; 32]]), 2);
        assert_eq!(count_from_logs(&scts, &[[3; 32]]), 0);
    }
}
