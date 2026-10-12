//! NAT port mapping over NAT-PMP (RFC 6886), wire format and lease bookkeeping (std only).
//!
//! This is the pure model behind a port mapping call. It encodes the external address request
//! and the map request (UDP or TCP, with a lifetime where zero deletes), decodes the gateway's
//! replies, and tracks the lease a grant creates. It opens no socket: the caller sends the bytes
//! this module encodes to the gateway on `GATEWAY_PORT` and passes back the bytes it receives.

use std::fmt;
use std::net::Ipv4Addr;

/// UDP port the gateway listens on.
pub const GATEWAY_PORT: u16 = 5351;
/// Protocol version this module speaks.
pub const VERSION: u8 = 0;

/// Opcode: request the external IPv4 address.
pub const OP_EXTERNAL_ADDRESS: u8 = 0;
/// Opcode: map a UDP port.
pub const OP_MAP_UDP: u8 = 1;
/// Opcode: map a TCP port.
pub const OP_MAP_TCP: u8 = 2;
/// Bit set on the opcode of every reply.
const REPLY_BIT: u8 = 0x80;

/// Length of the external address request.
pub const EXTERNAL_REQUEST_LEN: usize = 2;
/// Length of the map request.
pub const MAP_REQUEST_LEN: usize = 12;
/// Length of an external address reply.
pub const EXTERNAL_REPLY_LEN: usize = 12;
/// Length of a map reply.
pub const MAP_REPLY_LEN: usize = 16;

/// Transport a mapping applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Udp,
    Tcp,
}

impl Protocol {
    fn opcode(self) -> u8 {
        match self {
            Protocol::Udp => OP_MAP_UDP,
            Protocol::Tcp => OP_MAP_TCP,
        }
    }
}

/// Result codes a gateway returns, by their RFC names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultCode {
    Success,
    UnsupportedVersion,
    NotAuthorized,
    NetworkFailure,
    OutOfResources,
    UnsupportedOpcode,
}

impl ResultCode {
    /// Maps a wire result code to its name. Codes above 5 are not defined by the RFC.
    pub fn from_code(code: u16) -> Option<ResultCode> {
        match code {
            0 => Some(ResultCode::Success),
            1 => Some(ResultCode::UnsupportedVersion),
            2 => Some(ResultCode::NotAuthorized),
            3 => Some(ResultCode::NetworkFailure),
            4 => Some(ResultCode::OutOfResources),
            5 => Some(ResultCode::UnsupportedOpcode),
            _ => None,
        }
    }
}

/// Why a reply could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The buffer is shorter than the reply it claims to be.
    Short,
    /// The reply names a protocol version this module does not speak.
    WrongVersion(u8),
    /// The reply does not have the reply bit set, so it is a request.
    NotAReply,
    /// The reply names an opcode this module does not know.
    UnknownOpcode(u8),
    /// The gateway refused the request with a defined result code.
    Gateway(ResultCode),
    /// The gateway returned a result code the RFC does not define.
    UnknownResult(u16),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Short => write!(f, "reply is shorter than its opcode needs"),
            Error::WrongVersion(v) => write!(f, "unsupported NAT-PMP version {}", v),
            Error::NotAReply => write!(f, "packet is a request, not a reply"),
            Error::UnknownOpcode(op) => write!(f, "unknown NAT-PMP opcode {}", op),
            Error::Gateway(code) => write!(f, "gateway refused the request: {:?}", code),
            Error::UnknownResult(code) => write!(f, "unknown NAT-PMP result code {}", code),
        }
    }
}

impl std::error::Error for Error {}

/// A decoded reply from the gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The gateway's external IPv4 address, with its epoch in seconds since it started.
    ExternalAddress { epoch: u32, address: Ipv4Addr },
    /// A granted mapping. A lifetime of zero means the mapping was deleted.
    Mapping {
        epoch: u32,
        protocol: Protocol,
        internal_port: u16,
        external_port: u16,
        lifetime: u32,
    },
}

/// The request for the gateway's external IPv4 address.
pub fn external_address_request() -> [u8; EXTERNAL_REQUEST_LEN] {
    [VERSION, OP_EXTERNAL_ADDRESS]
}

/// A map request. The gateway may use another external port than `suggested_external`. A lifetime
/// of zero asks it to delete the mapping.
pub fn map_request(
    protocol: Protocol,
    internal_port: u16,
    suggested_external: u16,
    lifetime: u32,
) -> [u8; MAP_REQUEST_LEN] {
    let mut b = [0u8; MAP_REQUEST_LEN];
    b[0] = VERSION;
    b[1] = protocol.opcode();
    // Bytes 2 and 3 are reserved and stay zero.
    b[4..6].copy_from_slice(&internal_port.to_be_bytes());
    b[6..8].copy_from_slice(&suggested_external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime.to_be_bytes());
    b
}

/// The request that deletes a mapping: a map request with a lifetime of zero.
pub fn delete_request(protocol: Protocol, internal_port: u16) -> [u8; MAP_REQUEST_LEN] {
    map_request(protocol, internal_port, 0, 0)
}

/// Reads a gateway reply. The header (version, opcode, result, epoch) comes first, so a refusal
/// is reported even when the body that follows it is missing.
pub fn decode_reply(buf: &[u8]) -> Result<Reply, Error> {
    if buf.len() < 8 {
        return Err(Error::Short);
    }
    if buf[0] != VERSION {
        return Err(Error::WrongVersion(buf[0]));
    }
    if buf[1] & REPLY_BIT == 0 {
        return Err(Error::NotAReply);
    }
    let op = buf[1] & !REPLY_BIT;
    let result = u16::from_be_bytes([buf[2], buf[3]]);
    let epoch = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if result != 0 {
        return Err(match ResultCode::from_code(result) {
            Some(code) => Error::Gateway(code),
            None => Error::UnknownResult(result),
        });
    }
    match op {
        OP_EXTERNAL_ADDRESS => {
            if buf.len() < EXTERNAL_REPLY_LEN {
                return Err(Error::Short);
            }
            Ok(Reply::ExternalAddress {
                epoch,
                address: Ipv4Addr::new(buf[8], buf[9], buf[10], buf[11]),
            })
        }
        OP_MAP_UDP | OP_MAP_TCP => {
            if buf.len() < MAP_REPLY_LEN {
                return Err(Error::Short);
            }
            let protocol = if op == OP_MAP_TCP { Protocol::Tcp } else { Protocol::Udp };
            Ok(Reply::Mapping {
                epoch,
                protocol,
                internal_port: u16::from_be_bytes([buf[8], buf[9]]),
                external_port: u16::from_be_bytes([buf[10], buf[11]]),
                lifetime: u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]),
            })
        }
        other => Err(Error::UnknownOpcode(other)),
    }
}

/// A mapping the gateway granted, with the epoch it was granted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub protocol: Protocol,
    pub internal_port: u16,
    pub external_port: u16,
    pub lifetime: u32,
    /// The gateway's epoch at the grant.
    pub epoch: u32,
}

impl Lease {
    /// Builds a lease from a mapping reply. A reply with a zero lifetime deleted the mapping, so it
    /// makes no lease and the result is `None`.
    pub fn from_reply(reply: Reply) -> Option<Lease> {
        match reply {
            Reply::Mapping { epoch, protocol, internal_port, external_port, lifetime } => {
                if lifetime == 0 {
                    None
                } else {
                    Some(Lease { protocol, internal_port, external_port, lifetime, epoch })
                }
            }
            Reply::ExternalAddress { .. } => None,
        }
    }

    /// Seconds after the grant at which to ask for the mapping again. This is half the lifetime,
    /// a choice of this module, not a rule of the RFC.
    pub fn renew_after(&self) -> u32 {
        self.lifetime / 2
    }

    /// Seconds left on the lease, given the seconds elapsed since the grant.
    pub fn remaining(&self, elapsed: u32) -> u32 {
        self.lifetime.saturating_sub(elapsed)
    }
}

/// Whether the gateway restarted between two readings, so that every mapping must be requested
/// again. `last_epoch` and `new_epoch` are the gateway's epochs. `local_elapsed` is the number of
/// local seconds between the two readings. A smaller epoch, or an epoch step that differs from the
/// local elapsed time by more than two seconds, means a restart.
pub fn gateway_restarted(last_epoch: u32, new_epoch: u32, local_elapsed: u32) -> bool {
    if new_epoch < last_epoch {
        return true;
    }
    (new_epoch - last_epoch).abs_diff(local_elapsed) > 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn map_request_lays_out_the_fields_in_network_order() {
        // 8080 is 0x1F90, 3600 is 0x0E10.
        assert_eq!(
            map_request(Protocol::Udp, 8080, 8080, 3600),
            [0, 1, 0, 0, 0x1F, 0x90, 0x1F, 0x90, 0, 0, 0x0E, 0x10]
        );
        assert_eq!(map_request(Protocol::Tcp, 1, 2, 0)[1], 2);
    }

    #[test]
    fn external_address_request_is_two_bytes() {
        assert_eq!(external_address_request(), [0, 0]);
        assert_eq!(delete_request(Protocol::Udp, 8080), map_request(Protocol::Udp, 8080, 0, 0));
    }

    #[test]
    fn decodes_an_external_address_reply() {
        let buf = [0, 0x80, 0, 0, 0, 0, 0x0E, 0x10, 203, 0, 113, 7];
        assert_eq!(
            decode_reply(&buf),
            Ok(Reply::ExternalAddress { epoch: 3600, address: Ipv4Addr::new(203, 0, 113, 7) })
        );
    }

    #[test]
    fn decodes_a_mapping_reply() {
        // Epoch 100, internal 8080, external 50000 (0xC350), lifetime 7200 (0x1C20).
        let buf = [0, 0x81, 0, 0, 0, 0, 0, 100, 0x1F, 0x90, 0xC3, 0x50, 0, 0, 0x1C, 0x20];
        assert_eq!(
            decode_reply(&buf),
            Ok(Reply::Mapping {
                epoch: 100,
                protocol: Protocol::Udp,
                internal_port: 8080,
                external_port: 50000,
                lifetime: 7200,
            })
        );
    }

    #[test]
    fn a_refusal_is_reported_from_the_header_alone() {
        let buf = [0, 0x81, 0, 2, 0, 0, 0, 9];
        assert_eq!(decode_reply(&buf), Err(Error::Gateway(ResultCode::NotAuthorized)));
        let unknown = [0, 0x81, 0, 9, 0, 0, 0, 9];
        assert_eq!(decode_reply(&unknown), Err(Error::UnknownResult(9)));
    }

    #[test]
    fn short_and_mistyped_replies_are_rejected() {
        // A mapping reply needs 16 bytes, this has 12.
        let short = [0, 0x82, 0, 0, 0, 0, 0, 1, 0x1F, 0x90, 0xC3, 0x50];
        assert_eq!(decode_reply(&short), Err(Error::Short));
        assert_eq!(decode_reply(&[0, 0x01, 0, 0, 0, 0, 0, 1]), Err(Error::NotAReply));
        assert_eq!(decode_reply(&[1, 0x80, 0, 0, 0, 0, 0, 1]), Err(Error::WrongVersion(1)));
        assert_eq!(decode_reply(&[0, 0x85, 0, 0, 0, 0, 0, 1]), Err(Error::UnknownOpcode(5)));
    }

    #[test]
    fn a_zero_lifetime_reply_makes_no_lease() {
        let granted = Reply::Mapping {
            epoch: 100,
            protocol: Protocol::Tcp,
            internal_port: 22,
            external_port: 2222,
            lifetime: 600,
        };
        let lease = Lease::from_reply(granted).expect("a lease");
        assert_eq!(lease.renew_after(), 300);
        assert_eq!(lease.remaining(500), 100);
        assert_eq!(lease.remaining(900), 0);
        let deleted = Reply::Mapping {
            epoch: 100,
            protocol: Protocol::Tcp,
            internal_port: 22,
            external_port: 2222,
            lifetime: 0,
        };
        assert_eq!(Lease::from_reply(deleted), None);
        assert_eq!(Lease::from_reply(Reply::ExternalAddress { epoch: 1, address: Ipv4Addr::LOCALHOST }), None);
    }

    #[test]
    fn restart_is_seen_from_a_backward_epoch_or_a_skewed_step() {
        assert!(!gateway_restarted(1000, 1010, 10));
        assert!(!gateway_restarted(1000, 1002, 0));
        assert!(gateway_restarted(1000, 1010, 4));
        assert!(gateway_restarted(1000, 900, 0));
    }
}
