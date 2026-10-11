//! NAL unit plumbing: Annex-B splitting, AVCC/HVCC to Annex-B, access units.

use super::Codec;

pub const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// True when `data` begins with a 3- or 4-byte start code.
pub fn is_annexb(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(&START_CODE)
}

/// Splits an Annex-B stream into NAL units (start codes and trailing zeros removed).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = None;
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            if let Some(s) = start {
                push_nal(&mut out, &data[s..i]);
            }
            i += 3;
            start = Some(i);
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        push_nal(&mut out, &data[s..]);
    }
    out
}

fn push_nal<'a>(out: &mut Vec<&'a [u8]>, nal: &'a [u8]) {
    let end = nal.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    if end > 0 {
        out.push(&nal[..end]);
    }
}

pub fn nal_type(codec: Codec, nal: &[u8]) -> u8 {
    match codec {
        Codec::H264 => nal.first().map_or(0, |b| b & 0x1f),
        Codec::Hevc => nal.first().map_or(0, |b| (b >> 1) & 0x3f),
    }
}

/// A coded slice (as opposed to parameter sets, SEI, delimiters).
pub fn is_vcl(codec: Codec, nal: &[u8]) -> bool {
    let t = nal_type(codec, nal);
    match codec {
        Codec::H264 => (1..=5).contains(&t),
        Codec::Hevc => t < 32,
    }
}

pub fn is_sps(codec: Codec, nal: &[u8]) -> bool {
    nal_type(codec, nal)
        == match codec {
            Codec::H264 => 7,
            Codec::Hevc => 33,
        }
}

pub fn is_parameter_set(codec: Codec, nal: &[u8]) -> bool {
    let t = nal_type(codec, nal);
    match codec {
        Codec::H264 => t == 7 || t == 8,
        Codec::Hevc => (32..=34).contains(&t),
    }
}

/// A slice that starts a new picture (first_mb_in_slice == 0, or
/// first_slice_segment_in_pic_flag set).
pub fn starts_picture(codec: Codec, nal: &[u8]) -> bool {
    is_vcl(codec, nal)
        && match codec {
            Codec::H264 => nal.get(1).is_some_and(|b| b & 0x80 != 0),
            Codec::Hevc => nal.get(2).is_some_and(|b| b & 0x80 != 0),
        }
}

/// The NAL length prefix size declared by avcC/hvcC extradata (4 for Annex-B or unknown).
pub fn length_size(codec: Codec, extra: &[u8]) -> usize {
    if is_annexb(extra) || extra.first() != Some(&1) {
        return 4;
    }
    let idx = match codec {
        Codec::H264 => 4,
        Codec::Hevc => 21,
    };
    extra.get(idx).map_or(4, |b| (b & 3) as usize + 1)
}

/// Parameter sets from avcC/hvcC extradata as an Annex-B stream (Annex-B input is returned as is).
pub fn extradata_to_annexb(codec: Codec, extra: &[u8]) -> Vec<u8> {
    if is_annexb(extra) {
        return extra.to_vec();
    }
    let mut out = Vec::new();
    match codec {
        Codec::H264 => {
            // avcC: ... [5] 0xe0|numSPS, (u16 len, sps)*, numPPS, (u16 len, pps)*
            let n_sps = extra.get(5).map_or(0, |b| b & 0x1f);
            let mut pos = 6;
            for _ in 0..n_sps {
                pos = append_sized(&mut out, extra, pos);
            }
            let n_pps = extra.get(pos).copied().unwrap_or(0);
            pos += 1;
            for _ in 0..n_pps {
                pos = append_sized(&mut out, extra, pos);
            }
        }
        Codec::Hevc => {
            // hvcC: 22 bytes, numOfArrays, per array: type, u16 count, (u16 len, nal)*
            let arrays = extra.get(22).copied().unwrap_or(0);
            let mut pos = 23;
            for _ in 0..arrays {
                let count = u16_at(extra, pos + 1);
                pos += 3;
                for _ in 0..count {
                    pos = append_sized(&mut out, extra, pos);
                }
            }
        }
    }
    out
}

fn u16_at(data: &[u8], pos: usize) -> usize {
    match (data.get(pos), data.get(pos + 1)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]) as usize,
        _ => 0,
    }
}

fn append_sized(out: &mut Vec<u8>, data: &[u8], pos: usize) -> usize {
    let len = u16_at(data, pos);
    if let Some(nal) = data.get(pos + 2..pos + 2 + len) {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
    }
    pos + 2 + len
}

/// A length-prefixed access unit as Annex-B (Annex-B input is returned as is).
pub fn to_annexb(au: &[u8], len_size: usize) -> Vec<u8> {
    if is_annexb(au) {
        return au.to_vec();
    }
    let mut out = Vec::with_capacity(au.len() + 16);
    let mut pos = 0;
    while pos + len_size <= au.len() {
        let len = au[pos..pos + len_size].iter().fold(0usize, |n, &b| n << 8 | b as usize);
        pos += len_size;
        match au.get(pos..pos + len) {
            Some(nal) => {
                out.extend_from_slice(&START_CODE);
                out.extend_from_slice(nal);
            }
            None => break,
        }
        pos += len;
    }
    out
}

/// Joins NAL units into an Annex-B stream.
pub fn join(nals: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nals.iter().map(|n| n.len() + 4).sum());
    for nal in nals {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
    }
    out
}

/// Groups the NAL units of an elementary stream into access units (one picture
/// each, with its leading parameter sets, SEI and delimiters).
pub fn access_units(codec: Codec, nals: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut units = Vec::new();
    let mut current: Vec<&[u8]> = Vec::new();
    let mut has_slice = false;
    for &nal in nals {
        let new_unit = has_slice && (starts_picture(codec, nal) || !is_vcl(codec, nal));
        if new_unit {
            units.push(join(&current));
            current.clear();
            has_slice = false;
        }
        has_slice |= is_vcl(codec, nal);
        current.push(nal);
    }
    if !current.is_empty() {
        units.push(join(&current));
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x1f];
    const PPS: &[u8] = &[0x68, 0xee, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x33];
    const P_FIRST: &[u8] = &[0x41, 0x9a, 0x02];
    const P_MORE: &[u8] = &[0x41, 0x1a, 0x02]; // first_mb_in_slice != 0

    #[test]
    fn splits_three_and_four_byte_start_codes_and_strips_trailing_zeros() {
        let mut s = vec![0, 0, 0, 1];
        s.extend(SPS);
        s.extend([0, 0, 1]);
        s.extend(PPS);
        s.extend([0, 0]); // trailing_zero_8bits
        s.extend([0, 0, 0, 1]);
        s.extend(IDR);
        assert_eq!(split_annexb(&s), vec![SPS, PPS, IDR]);
        assert!(split_annexb(&[0, 0, 0, 1]).is_empty());
        assert!(split_annexb(b"abc").is_empty());
    }

    #[test]
    fn classifies_h264_and_hevc_nal_units() {
        assert_eq!(nal_type(Codec::H264, SPS), 7);
        assert!(is_sps(Codec::H264, SPS) && !is_sps(Codec::H264, PPS));
        assert!(is_parameter_set(Codec::H264, PPS) && !is_vcl(Codec::H264, PPS));
        assert!(is_vcl(Codec::H264, IDR) && starts_picture(Codec::H264, IDR));
        assert!(starts_picture(Codec::H264, P_FIRST) && !starts_picture(Codec::H264, P_MORE));
        let hevc_sps = [0x42, 0x01, 0x01];
        let hevc_idr = [0x26, 0x01, 0xaf];
        let hevc_idr_more = [0x26, 0x01, 0x2f];
        assert_eq!(nal_type(Codec::Hevc, &hevc_sps), 33);
        assert!(is_sps(Codec::Hevc, &hevc_sps));
        assert!(is_parameter_set(Codec::Hevc, &hevc_sps));
        assert!(starts_picture(Codec::Hevc, &hevc_idr) && !starts_picture(Codec::Hevc, &hevc_idr_more));
    }

    fn avcc() -> Vec<u8> {
        let mut e = vec![1, 0x64, 0x00, 0x1f, 0xff, 0xe1, 0, SPS.len() as u8];
        e.extend(SPS);
        e.push(1);
        e.extend([0, PPS.len() as u8]);
        e.extend(PPS);
        e
    }

    #[test]
    fn converts_avcc_extradata_to_annexb() {
        let out = extradata_to_annexb(Codec::H264, &avcc());
        assert_eq!(split_annexb(&out), vec![SPS, PPS]);
        assert_eq!(length_size(Codec::H264, &avcc()), 4);
        let annexb = join(&[SPS, PPS]);
        assert_eq!(extradata_to_annexb(Codec::H264, &annexb), annexb);
        assert_eq!(length_size(Codec::H264, &annexb), 4);
        assert!(extradata_to_annexb(Codec::H264, &[1, 2]).is_empty());
    }

    #[test]
    fn converts_hvcc_extradata_to_annexb() {
        let vps = [0x40, 0x01, 0x0c];
        let sps = [0x42, 0x01, 0x01];
        let mut e = vec![1u8; 22];
        e[21] = 0x0f; // lengthSizeMinusOne = 3
        e.push(2); // two arrays
        for nal in [&vps[..], &sps[..]] {
            e.push(0xa0);
            e.extend([0, 1, 0, nal.len() as u8]);
            e.extend(nal);
        }
        assert_eq!(split_annexb(&extradata_to_annexb(Codec::Hevc, &e)), vec![&vps[..], &sps[..]]);
        assert_eq!(length_size(Codec::Hevc, &e), 4);
    }

    #[test]
    fn converts_length_prefixed_access_units() {
        let mut au = vec![0, 0, 0, IDR.len() as u8];
        au.extend(IDR);
        au.extend([0, 0, 0, P_MORE.len() as u8]);
        au.extend(P_MORE);
        assert_eq!(split_annexb(&to_annexb(&au, 4)), vec![IDR, P_MORE]);
        let mut short = vec![0, 0, 0, 9];
        short.extend(IDR);
        assert!(to_annexb(&short, 4).is_empty());
        let annexb = join(&[IDR]);
        assert_eq!(to_annexb(&annexb, 4), annexb);
    }

    #[test]
    fn groups_nal_units_into_access_units() {
        let sei = &[0x06, 0x05, 0x80][..];
        let units = access_units(Codec::H264, &[SPS, PPS, IDR, P_MORE, sei, P_FIRST, SPS, PPS, IDR]);
        let split: Vec<Vec<&[u8]>> = units.iter().map(|u| split_annexb(u)).collect();
        assert_eq!(split, vec![vec![SPS, PPS, IDR, P_MORE], vec![sei, P_FIRST], vec![SPS, PPS, IDR]]);
    }
}
