//! Typed Bluetooth errors for apps: the management status codes that the adapter reports for a pairing or link
//! request, and the Attribute Protocol (ATT) error codes that a remote device returns for a read or write.
//! Both are pure value types with no device access, so the shell and apps can match on them and tests run headless.
//! The shell's status text stays in `bare::bluetooth::status_text`; this module is the typed form of the same codes.

/// A status code from the adapter's management interface (the codes the shell maps to Settings text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MgmtStatus {
    Success,
    UnknownCommand,
    NotConnected,
    Failed,
    ConnectFailed,
    AuthenticationFailed,
    NotPaired,
    NoResources,
    Timeout,
    AlreadyConnected,
    Busy,
    Rejected,
    NotSupported,
    InvalidParameters,
    Disconnected,
    NotPowered,
    Cancelled,
    InvalidIndex,
    RadioBlocked,
    AlreadyPaired,
    PermissionDenied,
    /// a code this build does not name; it is kept so the value can still be shown and compared
    Other(u8),
}

impl MgmtStatus {
    /// Reads the status byte of a management reply.
    pub fn from_code(code: u8) -> MgmtStatus {
        match code {
            0x00 => MgmtStatus::Success,
            0x01 => MgmtStatus::UnknownCommand,
            0x02 => MgmtStatus::NotConnected,
            0x03 => MgmtStatus::Failed,
            0x04 => MgmtStatus::ConnectFailed,
            0x05 => MgmtStatus::AuthenticationFailed,
            0x06 => MgmtStatus::NotPaired,
            0x07 => MgmtStatus::NoResources,
            0x08 => MgmtStatus::Timeout,
            0x09 => MgmtStatus::AlreadyConnected,
            0x0a => MgmtStatus::Busy,
            0x0b => MgmtStatus::Rejected,
            0x0c => MgmtStatus::NotSupported,
            0x0d => MgmtStatus::InvalidParameters,
            0x0e => MgmtStatus::Disconnected,
            0x0f => MgmtStatus::NotPowered,
            0x10 => MgmtStatus::Cancelled,
            0x11 => MgmtStatus::InvalidIndex,
            0x12 => MgmtStatus::RadioBlocked,
            0x13 => MgmtStatus::AlreadyPaired,
            0x14 => MgmtStatus::PermissionDenied,
            other => MgmtStatus::Other(other),
        }
    }

    /// The raw status byte.
    pub fn code(self) -> u8 {
        match self {
            MgmtStatus::Success => 0x00,
            MgmtStatus::UnknownCommand => 0x01,
            MgmtStatus::NotConnected => 0x02,
            MgmtStatus::Failed => 0x03,
            MgmtStatus::ConnectFailed => 0x04,
            MgmtStatus::AuthenticationFailed => 0x05,
            MgmtStatus::NotPaired => 0x06,
            MgmtStatus::NoResources => 0x07,
            MgmtStatus::Timeout => 0x08,
            MgmtStatus::AlreadyConnected => 0x09,
            MgmtStatus::Busy => 0x0a,
            MgmtStatus::Rejected => 0x0b,
            MgmtStatus::NotSupported => 0x0c,
            MgmtStatus::InvalidParameters => 0x0d,
            MgmtStatus::Disconnected => 0x0e,
            MgmtStatus::NotPowered => 0x0f,
            MgmtStatus::Cancelled => 0x10,
            MgmtStatus::InvalidIndex => 0x11,
            MgmtStatus::RadioBlocked => 0x12,
            MgmtStatus::AlreadyPaired => 0x13,
            MgmtStatus::PermissionDenied => 0x14,
            MgmtStatus::Other(code) => code,
        }
    }

    /// The sentence a Settings alert shows for this status.
    pub fn text(self) -> &'static str {
        match self {
            MgmtStatus::Success => "Done",
            MgmtStatus::UnknownCommand => "Bluetooth Command Not Known",
            MgmtStatus::NotConnected => "Device Is Not Connected",
            MgmtStatus::Failed => "Bluetooth Error",
            MgmtStatus::ConnectFailed => "Connection Failed",
            MgmtStatus::AuthenticationFailed => "Authentication Failed",
            MgmtStatus::NotPaired => "Device Is Not Paired",
            MgmtStatus::NoResources => "Bluetooth Is Out of Resources",
            MgmtStatus::Timeout => "Timed Out",
            MgmtStatus::AlreadyConnected => "Already Connected",
            MgmtStatus::Busy => "Device Busy",
            MgmtStatus::Rejected => "Rejected",
            MgmtStatus::NotSupported => "Not Supported by This Device",
            MgmtStatus::InvalidParameters => "Bluetooth Request Was Invalid",
            MgmtStatus::Disconnected => "Device Disconnected",
            MgmtStatus::NotPowered => "Bluetooth Is Off",
            MgmtStatus::Cancelled => "Cancelled",
            MgmtStatus::InvalidIndex => "No Bluetooth Adapter",
            MgmtStatus::RadioBlocked => "Bluetooth Is Blocked",
            MgmtStatus::AlreadyPaired => "Already Paired",
            MgmtStatus::PermissionDenied => "Bluetooth Permission Denied",
            MgmtStatus::Other(_) => "Bluetooth Error",
        }
    }

    /// True when trying the same request again later can succeed: the device was busy, timed out,
    /// dropped the link or the adapter ran short of resources. A refusal or a wrong credential is not retryable.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            MgmtStatus::ConnectFailed | MgmtStatus::NoResources | MgmtStatus::Timeout | MgmtStatus::Busy | MgmtStatus::Disconnected
        )
    }
}

/// An Attribute Protocol error response from a remote device (Bluetooth Core, Volume 3, Part F, 3.4.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttError {
    InvalidHandle,
    ReadNotPermitted,
    WriteNotPermitted,
    InvalidPdu,
    InsufficientAuthentication,
    RequestNotSupported,
    InvalidOffset,
    InsufficientAuthorization,
    PrepareQueueFull,
    AttributeNotFound,
    AttributeNotLong,
    InsufficientEncryptionKeySize,
    InvalidAttributeValueLength,
    UnlikelyError,
    InsufficientEncryption,
    UnsupportedGroupType,
    InsufficientResources,
    /// codes 0x80 to 0x9f, defined by the application that owns the attribute
    Application(u8),
    /// codes 0xe0 to 0xff, defined by a common profile or service specification
    Profile(u8),
}

impl AttError {
    /// Reads an ATT error code. Returns `None` for 0x00 (no error) and for the codes the specification leaves unused.
    pub fn from_code(code: u8) -> Option<AttError> {
        Some(match code {
            0x01 => AttError::InvalidHandle,
            0x02 => AttError::ReadNotPermitted,
            0x03 => AttError::WriteNotPermitted,
            0x04 => AttError::InvalidPdu,
            0x05 => AttError::InsufficientAuthentication,
            0x06 => AttError::RequestNotSupported,
            0x07 => AttError::InvalidOffset,
            0x08 => AttError::InsufficientAuthorization,
            0x09 => AttError::PrepareQueueFull,
            0x0a => AttError::AttributeNotFound,
            0x0b => AttError::AttributeNotLong,
            0x0c => AttError::InsufficientEncryptionKeySize,
            0x0d => AttError::InvalidAttributeValueLength,
            0x0e => AttError::UnlikelyError,
            0x0f => AttError::InsufficientEncryption,
            0x10 => AttError::UnsupportedGroupType,
            0x11 => AttError::InsufficientResources,
            0x80..=0x9f => AttError::Application(code),
            0xe0..=0xff => AttError::Profile(code),
            _ => return None,
        })
    }

    /// The error code on the wire.
    pub fn code(self) -> u8 {
        match self {
            AttError::InvalidHandle => 0x01,
            AttError::ReadNotPermitted => 0x02,
            AttError::WriteNotPermitted => 0x03,
            AttError::InvalidPdu => 0x04,
            AttError::InsufficientAuthentication => 0x05,
            AttError::RequestNotSupported => 0x06,
            AttError::InvalidOffset => 0x07,
            AttError::InsufficientAuthorization => 0x08,
            AttError::PrepareQueueFull => 0x09,
            AttError::AttributeNotFound => 0x0a,
            AttError::AttributeNotLong => 0x0b,
            AttError::InsufficientEncryptionKeySize => 0x0c,
            AttError::InvalidAttributeValueLength => 0x0d,
            AttError::UnlikelyError => 0x0e,
            AttError::InsufficientEncryption => 0x0f,
            AttError::UnsupportedGroupType => 0x10,
            AttError::InsufficientResources => 0x11,
            AttError::Application(code) | AttError::Profile(code) => code,
        }
    }

    /// The sentence an app shows when the device refuses a read or a write.
    pub fn text(self) -> &'static str {
        match self {
            AttError::InvalidHandle => "The device does not have that value",
            AttError::ReadNotPermitted => "The device does not allow reading this value",
            AttError::WriteNotPermitted => "The device does not allow writing this value",
            AttError::InvalidPdu => "The device rejected a malformed request",
            AttError::InsufficientAuthentication => "Pair with the device to continue",
            AttError::RequestNotSupported => "The device does not support this request",
            AttError::InvalidOffset => "The value offset is outside the value",
            AttError::InsufficientAuthorization => "The device does not authorise this request",
            AttError::PrepareQueueFull => "The device has no room for more queued writes",
            AttError::AttributeNotFound => "The device has no value at that position",
            AttError::AttributeNotLong => "The value cannot be read in parts",
            AttError::InsufficientEncryptionKeySize => "The link encryption key is too short",
            AttError::InvalidAttributeValueLength => "The value has the wrong length",
            AttError::UnlikelyError => "The device could not complete the request",
            AttError::InsufficientEncryption => "Encrypt the link to continue",
            AttError::UnsupportedGroupType => "The device does not group values this way",
            AttError::InsufficientResources => "The device is out of resources",
            AttError::Application(_) => "The device refused the request",
            AttError::Profile(_) => "The device reported a service error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_management_code_round_trips_through_its_variant() {
        for code in 0x00..=0x14u8 {
            assert_eq!(MgmtStatus::from_code(code).code(), code);
        }
        assert_eq!(MgmtStatus::from_code(0x7e), MgmtStatus::Other(0x7e));
        assert_eq!(MgmtStatus::from_code(0x7e).code(), 0x7e);
    }

    #[test]
    fn the_shell_status_codes_keep_their_text() {
        assert_eq!(MgmtStatus::from_code(4).text(), "Connection Failed");
        assert_eq!(MgmtStatus::from_code(5).text(), "Authentication Failed");
        assert_eq!(MgmtStatus::from_code(8).text(), "Timed Out");
        assert_eq!(MgmtStatus::from_code(9).text(), "Already Connected");
        assert_eq!(MgmtStatus::from_code(10).text(), "Device Busy");
        assert_eq!(MgmtStatus::from_code(11).text(), "Rejected");
        assert_eq!(MgmtStatus::from_code(15).text(), "Bluetooth Is Off");
        assert_eq!(MgmtStatus::from_code(18).text(), "Bluetooth Is Blocked");
        assert_eq!(MgmtStatus::from_code(19).text(), "Already Paired");
        assert_eq!(MgmtStatus::from_code(0x7e).text(), "Bluetooth Error");
    }

    #[test]
    fn only_transient_failures_are_worth_retrying() {
        assert!(MgmtStatus::Timeout.is_retryable());
        assert!(MgmtStatus::Busy.is_retryable());
        assert!(MgmtStatus::Disconnected.is_retryable());
        assert!(!MgmtStatus::Rejected.is_retryable());
        assert!(!MgmtStatus::AuthenticationFailed.is_retryable());
        assert!(!MgmtStatus::Success.is_retryable());
    }

    #[test]
    fn success_and_unused_att_codes_are_not_errors() {
        assert_eq!(AttError::from_code(0x00), None);
        assert_eq!(AttError::from_code(0x12), None);
        assert_eq!(AttError::from_code(0xa0), None);
    }

    #[test]
    fn the_specified_att_codes_map_to_their_names() {
        assert_eq!(AttError::from_code(0x01), Some(AttError::InvalidHandle));
        assert_eq!(AttError::from_code(0x03), Some(AttError::WriteNotPermitted));
        assert_eq!(AttError::from_code(0x05), Some(AttError::InsufficientAuthentication));
        assert_eq!(AttError::from_code(0x0a), Some(AttError::AttributeNotFound));
        assert_eq!(AttError::from_code(0x0f), Some(AttError::InsufficientEncryption));
        assert_eq!(AttError::from_code(0x11), Some(AttError::InsufficientResources));
    }

    #[test]
    fn application_and_profile_codes_keep_their_number() {
        assert_eq!(AttError::from_code(0x80), Some(AttError::Application(0x80)));
        assert_eq!(AttError::from_code(0x9f), Some(AttError::Application(0x9f)));
        assert_eq!(AttError::from_code(0xe0), Some(AttError::Profile(0xe0)));
        assert_eq!(AttError::Profile(0xe5).code(), 0xe5);
        assert_eq!(AttError::Application(0x85).code(), 0x85);
    }

    #[test]
    fn every_specified_att_code_round_trips() {
        for code in 0x01..=0x11u8 {
            let error = AttError::from_code(code).expect("specified code");
            assert_eq!(error.code(), code);
            assert!(!error.text().is_empty());
        }
    }

    #[test]
    fn an_insufficient_authentication_error_asks_for_pairing() {
        assert_eq!(AttError::InsufficientAuthentication.text(), "Pair with the device to continue");
    }
}
