//! Typed errors for the low energy central role: `BleError` for transaction failures reported to an app, and
//! `AttCode` for the codes a remote attribute server returns with a failed request.
//! Case names follow the official error reference (`corebluetooth/cberror-swift.struct` and
//! `corebluetooth/cbatterror-swift.struct`, see `docs/parity/w5-core-bluetooth.md`). The numeric ATT codes are the
//! attribute protocol error values from the Bluetooth core specification.

use std::fmt;

/// A failed Bluetooth transaction, as an app receives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BleError {
    Unknown,
    InvalidParameters,
    InvalidHandle,
    NotConnected,
    OutOfSpace,
    OperationCancelled,
    ConnectionTimeout,
    PeripheralDisconnected,
    UuidNotAllowed,
    AlreadyAdvertising,
    ConnectionFailed,
    ConnectionLimitReached,
    OperationNotSupported,
    UnknownDevice,
    EncryptionTimedOut,
    PeerRemovedPairingInformation,
    TooManyPairedDevices,
    GattNearBackgroundNotificationLimit,
    GattExceededBackgroundNotificationLimit,
    ChannelSoundingConfigurationFailed,
    ChannelSoundingProcedureFailed,
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            BleError::Unknown => "an unknown error occurred",
            BleError::InvalidParameters => "the parameters are invalid",
            BleError::InvalidHandle => "the attribute handle is invalid",
            BleError::NotConnected => "the device is not connected",
            BleError::OutOfSpace => "the device has run out of space",
            BleError::OperationCancelled => "the operation was cancelled",
            BleError::ConnectionTimeout => "the connection timed out",
            BleError::PeripheralDisconnected => "the peripheral disconnected",
            BleError::UuidNotAllowed => "the UUID is not permitted",
            BleError::AlreadyAdvertising => "the peripheral is already advertising",
            BleError::ConnectionFailed => "the connection failed",
            BleError::ConnectionLimitReached => "the device has the maximum number of connections",
            BleError::OperationNotSupported => "the operation is not supported",
            BleError::UnknownDevice => "the device is unknown",
            BleError::EncryptionTimedOut => "encryption timed out",
            BleError::PeerRemovedPairingInformation => "the peer removed its pairing information",
            BleError::TooManyPairedDevices => "too many low energy devices are paired",
            BleError::GattNearBackgroundNotificationLimit => "near the background notification limit",
            BleError::GattExceededBackgroundNotificationLimit => "the background notification limit was exceeded",
            BleError::ChannelSoundingConfigurationFailed => "channel sounding configuration failed",
            BleError::ChannelSoundingProcedureFailed => "channel sounding procedure failed",
        };
        f.write_str(text)
    }
}

/// An error code from a remote attribute server. Code 0 means success and is not an error, so it has no variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttCode {
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
}

impl AttCode {
    /// The protocol value of this code.
    pub fn code(self) -> u8 {
        match self {
            AttCode::InvalidHandle => 0x01,
            AttCode::ReadNotPermitted => 0x02,
            AttCode::WriteNotPermitted => 0x03,
            AttCode::InvalidPdu => 0x04,
            AttCode::InsufficientAuthentication => 0x05,
            AttCode::RequestNotSupported => 0x06,
            AttCode::InvalidOffset => 0x07,
            AttCode::InsufficientAuthorization => 0x08,
            AttCode::PrepareQueueFull => 0x09,
            AttCode::AttributeNotFound => 0x0A,
            AttCode::AttributeNotLong => 0x0B,
            AttCode::InsufficientEncryptionKeySize => 0x0C,
            AttCode::InvalidAttributeValueLength => 0x0D,
            AttCode::UnlikelyError => 0x0E,
            AttCode::InsufficientEncryption => 0x0F,
            AttCode::UnsupportedGroupType => 0x10,
            AttCode::InsufficientResources => 0x11,
        }
    }

    /// The code for a protocol value. Returns `None` for 0 (success) and for values the protocol does not assign.
    pub fn from_code(code: u8) -> Option<AttCode> {
        Some(match code {
            0x01 => AttCode::InvalidHandle,
            0x02 => AttCode::ReadNotPermitted,
            0x03 => AttCode::WriteNotPermitted,
            0x04 => AttCode::InvalidPdu,
            0x05 => AttCode::InsufficientAuthentication,
            0x06 => AttCode::RequestNotSupported,
            0x07 => AttCode::InvalidOffset,
            0x08 => AttCode::InsufficientAuthorization,
            0x09 => AttCode::PrepareQueueFull,
            0x0A => AttCode::AttributeNotFound,
            0x0B => AttCode::AttributeNotLong,
            0x0C => AttCode::InsufficientEncryptionKeySize,
            0x0D => AttCode::InvalidAttributeValueLength,
            0x0E => AttCode::UnlikelyError,
            0x0F => AttCode::InsufficientEncryption,
            0x10 => AttCode::UnsupportedGroupType,
            0x11 => AttCode::InsufficientResources,
            _ => return None,
        })
    }
}

impl fmt::Display for AttCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            AttCode::InvalidHandle => "the attribute handle is invalid",
            AttCode::ReadNotPermitted => "reading the attribute is not permitted",
            AttCode::WriteNotPermitted => "writing the attribute is not permitted",
            AttCode::InvalidPdu => "the attribute PDU is invalid",
            AttCode::InsufficientAuthentication => "the attribute needs authentication",
            AttCode::RequestNotSupported => "the request is not supported",
            AttCode::InvalidOffset => "the offset is past the end of the value",
            AttCode::InsufficientAuthorization => "the attribute needs authorization",
            AttCode::PrepareQueueFull => "the prepare queue is full",
            AttCode::AttributeNotFound => "the attribute was not found",
            AttCode::AttributeNotLong => "the attribute cannot be read with a long read",
            AttCode::InsufficientEncryptionKeySize => "the encryption key size is too small",
            AttCode::InvalidAttributeValueLength => "the attribute value length is invalid",
            AttCode::UnlikelyError => "the request failed with an unlikely error",
            AttCode::InsufficientEncryption => "the attribute needs encryption",
            AttCode::UnsupportedGroupType => "the attribute type is not a supported grouping type",
            AttCode::InsufficientResources => "the server has insufficient resources",
        };
        f.write_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn att_codes_use_the_protocol_values() {
        assert_eq!(AttCode::from_code(0x01), Some(AttCode::InvalidHandle));
        assert_eq!(AttCode::from_code(0x0A), Some(AttCode::AttributeNotFound));
        assert_eq!(AttCode::from_code(0x11), Some(AttCode::InsufficientResources));
        assert_eq!(AttCode::AttributeNotFound.code(), 0x0A);
    }

    #[test]
    fn success_and_unassigned_values_are_not_errors() {
        assert_eq!(AttCode::from_code(0x00), None);
        assert_eq!(AttCode::from_code(0x12), None);
        assert_eq!(AttCode::from_code(0xFF), None);
    }

    #[test]
    fn every_att_code_round_trips_through_its_value() {
        for raw in 0x01..=0x11u8 {
            let code = AttCode::from_code(raw).expect("assigned value");
            assert_eq!(code.code(), raw);
        }
    }

    #[test]
    fn att_code_text_names_the_failure() {
        assert_eq!(AttCode::AttributeNotFound.to_string(), "the attribute was not found");
        assert_eq!(AttCode::WriteNotPermitted.to_string(), "writing the attribute is not permitted");
    }

    #[test]
    fn ble_error_text_is_a_short_sentence() {
        assert_eq!(BleError::ConnectionTimeout.to_string(), "the connection timed out");
        assert_eq!(BleError::ConnectionFailed.to_string(), "the connection failed");
        assert_ne!(BleError::Unknown.to_string(), BleError::OperationCancelled.to_string());
    }
}
