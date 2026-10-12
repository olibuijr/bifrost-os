//! RFCOMM frames: the serial-style channel layer over Bluetooth BR/EDR (std only).
//!
//! An RFCOMM frame is an address byte (DLCI, command/response bit, extension bit), a control
//! byte (frame kind and poll/final bit), a length field of one or two bytes, the information
//! field, and one frame check sequence (FCS) byte. The FCS is the reflected CRC-8 of the
//! Bluetooth RFCOMM specification (the TS 07.10 frame check): it covers address, control and
//! length, except for UIH frames, where it covers only address and control, so a damaged
//! information byte in a UIH frame is not caught by the FCS.
//! This module encodes and decodes those bytes. It opens no socket and no link; the caller
//! owns the transport and feeds it the bytes it received.

/// SABM: set asynchronous balanced mode, opens a DLCI.
const CTRL_SABM: u8 = 0x2F;
/// UA: unnumbered acknowledgement.
const CTRL_UA: u8 = 0x63;
/// DM: disconnected mode, refuses or reports a closed DLCI.
const CTRL_DM: u8 = 0x0F;
/// DISC: disconnect a DLCI.
const CTRL_DISC: u8 = 0x43;
/// UIH: unnumbered information with header check, carries data.
const CTRL_UIH: u8 = 0xEF;
/// The poll/final bit of the control byte.
const PF: u8 = 0x10;
/// Largest DLCI the six-bit address field holds.
const MAX_DLCI: u8 = 63;
/// Largest information field the two-byte length field holds (15 bits).
const MAX_INFO: usize = 0x7FFF;
/// Largest information field the one-byte length field holds (7 bits).
const SHORT_INFO: usize = 127;
/// Reflected CRC-8 polynomial of the RFCOMM frame check sequence.
const CRC_POLY: u8 = 0xE0;
/// Value the running FCS is complemented against: `0xFF - fcs` on output.
const FCS_INIT: u8 = 0xFF;
/// The valid check value after the received FCS is folded in: a frame passes when the
/// running value over its covered bytes plus the received FCS equals this.
const FCS_CHECK: u8 = 0xCF;

/// The frame kind carried by the control byte, without the poll/final bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Set asynchronous balanced mode: open a DLCI.
    Sabm,
    /// Unnumbered acknowledgement.
    Ua,
    /// Disconnected mode.
    Dm,
    /// Disconnect.
    Disc,
    /// Unnumbered information with header check: data.
    Uih,
}

/// One decoded or to-be-encoded RFCOMM frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Data link connection identifier, 0 to 63 (server channel times two, plus direction).
    pub dlci: u8,
    /// The command/response bit: set for a command.
    pub command: bool,
    /// The frame kind.
    pub kind: Kind,
    /// The poll (command) or final (response) bit.
    pub poll_final: bool,
    /// The information field. Empty for SABM, UA, DM and DISC.
    pub info: Vec<u8>,
}

/// Why bytes could not be encoded or decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The DLCI is above 63.
    BadDlci,
    /// The information field is longer than 32767 bytes.
    TooLong,
    /// Fewer bytes arrived than the header and FCS need.
    TooShort,
    /// The address byte does not end with the extension bit set.
    BadAddress,
    /// The control byte is not one of the five frame kinds.
    BadControl(u8),
    /// The length field does not match the bytes that arrived.
    BadLength,
    /// The frame check sequence does not match the covered bytes.
    BadFcs,
}

/// The table entry for one byte: eight shift-and-XOR steps of the reflected polynomial.
fn crc_step(byte: u8) -> u8 {
    let mut c = byte;
    for _ in 0..8 {
        c = if c & 1 != 0 { (c >> 1) ^ CRC_POLY } else { c >> 1 };
    }
    c
}

/// The frame check sequence of `bytes`: the running CRC complemented against 0xFF.
fn fcs(bytes: &[u8]) -> u8 {
    let mut running = FCS_INIT;
    for &b in bytes {
        running = crc_step(running ^ b);
    }
    FCS_INIT - running
}

/// Encodes a frame to its wire bytes.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, Error> {
    if frame.dlci > MAX_DLCI {
        return Err(Error::BadDlci);
    }
    let len = frame.info.len();
    if len > MAX_INFO {
        return Err(Error::TooLong);
    }
    let base = match frame.kind {
        Kind::Sabm => CTRL_SABM,
        Kind::Ua => CTRL_UA,
        Kind::Dm => CTRL_DM,
        Kind::Disc => CTRL_DISC,
        Kind::Uih => CTRL_UIH,
    };
    let control = if frame.poll_final { base | PF } else { base };
    let address = (frame.dlci << 2) | if frame.command { 0x02 } else { 0x00 } | 0x01;
    let mut out = vec![address, control];
    if len <= SHORT_INFO {
        out.push(((len as u8) << 1) | 0x01);
    } else {
        out.push(((len & 0x7F) as u8) << 1);
        out.push((len >> 7) as u8);
    }
    let covered = if frame.kind == Kind::Uih { 2 } else { out.len() };
    let check = fcs(&out[..covered]);
    out.extend_from_slice(&frame.info);
    out.push(check);
    Ok(out)
}

/// Decodes one complete frame. `bytes` must hold exactly one frame, FCS included.
pub fn decode(bytes: &[u8]) -> Result<Frame, Error> {
    if bytes.len() < 4 {
        return Err(Error::TooShort);
    }
    let address = bytes[0];
    if address & 0x01 == 0 {
        return Err(Error::BadAddress);
    }
    let control = bytes[1];
    let (len, header) = if bytes[2] & 0x01 == 1 {
        ((bytes[2] >> 1) as usize, 3)
    } else {
        if bytes.len() < 5 {
            return Err(Error::TooShort);
        }
        (((bytes[2] >> 1) as usize) | ((bytes[3] as usize) << 7), 4)
    };
    let kind = match control & !PF {
        CTRL_SABM => Kind::Sabm,
        CTRL_UA => Kind::Ua,
        CTRL_DM => Kind::Dm,
        CTRL_DISC => Kind::Disc,
        CTRL_UIH => Kind::Uih,
        _ => return Err(Error::BadControl(control)),
    };
    let end = header + len;
    if bytes.len() != end + 1 {
        return Err(Error::BadLength);
    }
    let covered = if kind == Kind::Uih { 2 } else { header };
    if bytes[end] != fcs(&bytes[..covered]) {
        return Err(Error::BadFcs);
    }
    Ok(Frame {
        dlci: address >> 2,
        command: address & 0x02 != 0,
        kind,
        poll_final: control & PF != 0,
        info: bytes[header..end].to_vec(),
    })
}

/// True when `bytes` passes the RFCOMM check: the running CRC over every byte, FCS included,
/// lands on the check value. This holds for SABM, UA, DM and DISC frames, whose FCS covers
/// the whole header. A UIH frame's FCS covers only its first two bytes, so this check does
/// not apply to it; use `decode` for those.
pub fn check_passes(bytes: &[u8]) -> bool {
    let mut running = FCS_INIT;
    for &b in bytes {
        running = crc_step(running ^ b);
    }
    running == FCS_CHECK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sabm(dlci: u8) -> Frame {
        Frame { dlci, command: true, kind: Kind::Sabm, poll_final: true, info: vec![] }
    }

    #[test]
    fn sabm_on_dlci_zero_has_the_specified_fcs() {
        // 03 3F 01 is address, control (SABM with P=1), length 0; the FCS is 0x1C.
        assert_eq!(encode(&sabm(0)).unwrap(), vec![0x03, 0x3F, 0x01, 0x1C]);
    }

    #[test]
    fn ua_fcs_on_dlci_zero_matches_the_worked_value() {
        // Address 03, control 73 (UA with F=1), length 01: the FCS is 0xD7.
        assert_eq!(fcs(&[0x03, 0x73, 0x01]), 0xD7);
    }

    #[test]
    fn data_frame_round_trips_through_decode() {
        let f = Frame { dlci: 3, command: true, kind: Kind::Uih, poll_final: false, info: b"hello".to_vec() };
        let bytes = encode(&f).unwrap();
        // 2 header + 1 length + 5 data + 1 FCS = 9 bytes; address is 3 << 2 | 2 | 1 = 0x0F.
        assert_eq!(bytes.len(), 9);
        assert_eq!(bytes[0], 0x0F);
        assert_eq!(decode(&bytes).unwrap(), f);
    }

    #[test]
    fn long_information_uses_the_two_byte_length_field() {
        let f = Frame { dlci: 1, command: true, kind: Kind::Uih, poll_final: false, info: vec![7; 200] };
        let bytes = encode(&f).unwrap();
        // 200 = 72 << 1 in the first byte (extension bit clear) and 1 in the second.
        assert_eq!(bytes[2], 0x90);
        assert_eq!(bytes[3], 0x01);
        assert_eq!(decode(&bytes).unwrap(), f);
    }

    #[test]
    fn uih_fcs_ignores_the_information_field() {
        let f = Frame { dlci: 2, command: true, kind: Kind::Uih, poll_final: false, info: vec![1, 2, 3] };
        let mut bytes = encode(&f).unwrap();
        bytes[3] = 9;
        let back = decode(&bytes).unwrap();
        assert_eq!(back.info, vec![9, 2, 3]);
    }

    #[test]
    fn a_damaged_sabm_is_refused_with_bad_fcs() {
        let mut bytes = encode(&sabm(0)).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        assert_eq!(decode(&bytes), Err(Error::BadFcs));
    }

    #[test]
    fn unknown_control_and_short_input_are_refused() {
        assert_eq!(decode(&[0x03, 0x01, 0x01, 0x00]), Err(Error::BadControl(0x01)));
        assert_eq!(decode(&[0x03, 0x3F]), Err(Error::TooShort));
        assert_eq!(encode(&Frame { dlci: 64, command: true, kind: Kind::Sabm, poll_final: true, info: vec![] }), Err(Error::BadDlci));
    }

    #[test]
    fn check_passes_for_a_frame_with_its_fcs_appended() {
        // Address, control and length followed by the FCS: the running CRC over all of it
        // equals the check value, which is how a receiver tests a whole frame.
        let bytes = encode(&sabm(0)).unwrap();
        assert!(check_passes(&bytes));
        let mut bad = bytes.clone();
        bad[1] ^= 0x10;
        assert!(!check_passes(&bad));
    }
}
