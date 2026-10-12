//! The volume key handed to the initramfs (task 10): one EFI variable, volatile (no NV attribute, so it is gone at
//! power-off), written after a successful unlock and deleted by the initramfs hook `bifrost-unlock` before the root
//! is mounted. Payload: the data segment's byte offset (u64, little endian), then the volume key (32 or 64 bytes).

use crate::luks::Volume;
use alloc::vec::Vec;
use uefi::cstr16;
use uefi::runtime::{VariableAttributes, VariableVendor};

/// The variable's vendor GUID (BifrOSt's own; the hook reads `BifrostVolumeKey-<guid>` from efivarfs).
const VENDOR: VariableVendor = VariableVendor(uefi::guid!("b9a7c3e1-3f2d-4e8a-9c61-0b5e7d2a4f10"));

/// Writes the one-time variable. Its contents are never logged.
pub fn publish(volume: &Volume) -> Result<(), uefi::Error> {
    let payload = encode(volume);
    uefi::runtime::set_variable(
        cstr16!("BifrostVolumeKey"),
        &VENDOR,
        VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS,
        &payload,
    )
}

/// The payload bytes for `volume`.
pub fn encode(volume: &Volume) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + volume.key.len());
    out.extend_from_slice(&volume.offset.to_le_bytes());
    out.extend_from_slice(&volume.key);
    out
}

/// Reads a payload back: `(segment offset, volume key)`. `None` for a key length that is not 32 or 64 bytes.
/// The loader only encodes; the initramfs hook parses the same layout, and the tests pin it.
#[allow(dead_code)]
pub fn decode(payload: &[u8]) -> Option<(u64, &[u8])> {
    if payload.len() < 8 {
        return None;
    }
    let mut offset = [0u8; 8];
    offset.copy_from_slice(&payload[..8]);
    let key = &payload[8..];
    if key.len() != 32 && key.len() != 64 {
        return None;
    }
    Some((u64::from_le_bytes(offset), key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn round_trips_an_aes256_xts_key() {
        let volume = Volume { key: (0..64u8).collect(), offset: 0x0102_0304_0506 };
        let payload = encode(&volume);
        assert_eq!(payload.len(), 8 + 64);
        let (offset, key) = decode(&payload).unwrap();
        assert_eq!(offset, 0x0102_0304_0506);
        assert_eq!(key, &volume.key[..]);
    }

    #[test]
    fn offset_is_little_endian_first() {
        let volume = Volume { key: vec![0xaa; 32], offset: 0x0102 };
        let payload = encode(&volume);
        assert_eq!(&payload[..8], &[0x02, 0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(payload[8], 0xaa);
    }

    #[test]
    fn rejects_short_and_odd_length_payloads() {
        assert_eq!(decode(&[0u8; 7]), None);
        assert_eq!(decode(&[0u8; 8 + 16]), None);
        assert_eq!(decode(&[0u8; 8 + 33]), None);
        assert_eq!(decode(&[0u8; 8 + 32]).map(|(_, k)| k.len()), Some(32));
    }
}
