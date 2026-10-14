//! L2CAP addressing: the protocol/service multiplexer (PSM) values a BR/EDR channel opens on, the fixed channel
//! identifiers (CIDs) of an LE link, and the rules a PSM must follow. Pure: no socket and no radio. The shell holds
//! one link open per connected device through [`keep_alive_endpoint`]; apps open no channel of their own.

/// The Service Discovery Protocol PSM.
pub const PSM_SDP: u16 = 0x0001;
/// The RFCOMM PSM.
pub const PSM_RFCOMM: u16 = 0x0003;
/// The Audio/Video Distribution Transport Protocol PSM.
pub const PSM_AVDTP: u16 = 0x0019;
/// The first PSM in the range a device assigns dynamically (0x1001 to 0xFFFF).
pub const PSM_DYNAMIC_FIRST: u16 = 0x1001;

/// The signalling channel of a BR/EDR link.
pub const CID_SIGNALING_BREDR: u16 = 0x0001;
/// The Attribute Protocol channel of an LE link.
pub const CID_ATT: u16 = 0x0004;
/// The signalling channel of an LE link.
pub const CID_SIGNALING_LE: u16 = 0x0005;
/// The Security Manager channel of an LE link.
pub const CID_SMP: u16 = 0x0006;

/// Why a PSM value is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsmError {
    /// The least significant bit of the least significant octet is 0. Every PSM is odd.
    EvenLowOctet,
    /// The least significant bit of the most significant octet is 1. A PSM's upper octet is even.
    OddHighOctet,
}

/// Checks a PSM against the rules of the Bluetooth core specification: the least significant bit of the least
/// significant octet must be 1 and the least significant bit of the most significant octet must be 0.
pub fn validate_psm(psm: u16) -> Result<u16, PsmError> {
    if psm & 0x0001 == 0 {
        return Err(PsmError::EvenLowOctet);
    }
    if psm & 0x0100 != 0 {
        return Err(PsmError::OddHighOctet);
    }
    Ok(psm)
}

/// True when the PSM is in the dynamic range, where a device assigns it to a service at run time rather than the
/// fixed assignments above. The value must already be valid.
pub fn is_dynamic(psm: u16) -> bool {
    psm >= PSM_DYNAMIC_FIRST
}

/// The kind of link a channel runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Classic (BR/EDR) link: channels open on a PSM.
    BrEdr,
    /// Low energy link: channels are fixed, identified by a CID.
    LowEnergy,
}

/// Where a channel is opened on a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A dynamic channel on a BR/EDR link, named by its PSM.
    Psm(u16),
    /// A fixed channel, named by its CID.
    FixedCid(u16),
}

/// The endpoint the shell holds open to keep a device connected: the signalling PSM on BR/EDR, the Attribute
/// Protocol channel on LE. No data moves over it.
pub fn keep_alive_endpoint(transport: Transport) -> Endpoint {
    match transport {
        Transport::BrEdr => Endpoint::Psm(1),
        Transport::LowEnergy => Endpoint::FixedCid(CID_ATT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_psms_are_odd_and_valid() {
        for psm in [PSM_SDP, PSM_RFCOMM, PSM_AVDTP] {
            assert_eq!(validate_psm(psm), Ok(psm));
        }
    }

    #[test]
    fn an_even_low_octet_is_refused() {
        assert_eq!(validate_psm(0x0000), Err(PsmError::EvenLowOctet));
        assert_eq!(validate_psm(0x0002), Err(PsmError::EvenLowOctet));
    }

    #[test]
    fn a_set_high_octet_bit_is_refused() {
        assert_eq!(validate_psm(0x0101), Err(PsmError::OddHighOctet));
        assert_eq!(validate_psm(0x0301), Err(PsmError::OddHighOctet));
    }

    #[test]
    fn the_dynamic_range_starts_at_0x1001() {
        assert!(!is_dynamic(PSM_AVDTP));
        assert!(is_dynamic(PSM_DYNAMIC_FIRST));
        assert_eq!(validate_psm(PSM_DYNAMIC_FIRST), Ok(PSM_DYNAMIC_FIRST));
        assert!(is_dynamic(0xFFFF));
    }

    #[test]
    fn the_fixed_cids_follow_the_specification() {
        assert_eq!(CID_SIGNALING_BREDR, 1);
        assert_eq!(CID_ATT, 4);
        assert_eq!(CID_SIGNALING_LE, 5);
        assert_eq!(CID_SMP, 6);
    }

    #[test]
    fn the_keep_alive_endpoint_is_psm_one_on_classic_links_and_the_attribute_channel_on_le() {
        assert_eq!(keep_alive_endpoint(Transport::BrEdr), Endpoint::Psm(1));
        assert_eq!(keep_alive_endpoint(Transport::LowEnergy), Endpoint::FixedCid(4));
    }
}
