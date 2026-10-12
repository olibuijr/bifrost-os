//! AES-XTS (IEEE 1619) for whole 16-byte blocks, as LUKS2's `aes-xts-plain64` uses it: the tweak is the sector number
//! as a little-endian 128-bit value encrypted with the second key. Sectors here are always whole blocks (512 bytes in
//! LUKS2), so ciphertext stealing is not needed and is not implemented.

use super::aes::Aes;

/// Splits an XTS key (two equal halves) into the data key and the tweak key. `None` for odd or unsupported lengths.
pub fn split_key(key: &[u8]) -> Option<(Aes, Aes)> {
    if key.len() % 2 != 0 {
        return None;
    }
    let half = key.len() / 2;
    Some((Aes::new(&key[..half])?, Aes::new(&key[half..])?))
}

/// Multiplies the tweak by x in GF(2^128), little-endian bit order.
fn mul_x(t: &mut [u8; 16]) {
    let mut carry = 0u8;
    for b in t.iter_mut() {
        let next = *b >> 7;
        *b = (*b << 1) | carry;
        carry = next;
    }
    if carry != 0 {
        t[0] ^= 0x87;
    }
}

fn tweak(key2: &Aes, sector: u64) -> [u8; 16] {
    let mut t = [0u8; 16];
    t[..8].copy_from_slice(&sector.to_le_bytes());
    key2.encrypt_block(&mut t);
    t
}

fn xor(a: &mut [u8; 16], b: &[u8; 16]) {
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x ^= y;
    }
}

/// Decrypts one data unit in place. `None` unless the length is a non-zero multiple of 16.
pub fn decrypt_sector(key1: &Aes, key2: &Aes, sector: u64, data: &mut [u8]) -> Option<()> {
    if data.is_empty() || data.len() % 16 != 0 {
        return None;
    }
    let mut t = tweak(key2, sector);
    for chunk in data.chunks_exact_mut(16) {
        let mut b = [0u8; 16];
        b.copy_from_slice(chunk);
        xor(&mut b, &t);
        key1.decrypt_block(&mut b);
        xor(&mut b, &t);
        chunk.copy_from_slice(&b);
        mul_x(&mut t);
    }
    Some(())
}

/// Encrypts one data unit in place. The unlock only decrypts; this is for the round-trip and vector tests.
#[allow(dead_code)]
pub fn encrypt_sector(key1: &Aes, key2: &Aes, sector: u64, data: &mut [u8]) -> Option<()> {
    if data.is_empty() || data.len() % 16 != 0 {
        return None;
    }
    let mut t = tweak(key2, sector);
    for chunk in data.chunks_exact_mut(16) {
        let mut b = [0u8; 16];
        b.copy_from_slice(chunk);
        xor(&mut b, &t);
        key1.encrypt_block(&mut b);
        xor(&mut b, &t);
        chunk.copy_from_slice(&b);
        mul_x(&mut t);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> alloc::string::String {
        bytes.iter().map(|b| alloc::format!("{:02x}", b)).collect()
    }

    #[test]
    fn ieee_1619_vector_1_xts_aes128() {
        // Key 0 (both halves), sector 0, 32 zero bytes of plaintext.
        let (k1, k2) = split_key(&[0u8; 32]).unwrap();
        let mut data = [0u8; 32];
        encrypt_sector(&k1, &k2, 0, &mut data).unwrap();
        assert_eq!(hex(&data), "917cf69ebd68b2ec9b9fe9a3eadda692cd43d2f59598ed858c02c2652fbf922e");
        decrypt_sector(&k1, &k2, 0, &mut data).unwrap();
        assert_eq!(data, [0u8; 32]);
    }

    #[test]
    fn sector_number_changes_the_ciphertext() {
        let (k1, k2) = split_key(&[7u8; 64]).unwrap();
        let mut a = [1u8; 512];
        let mut b = [1u8; 512];
        encrypt_sector(&k1, &k2, 0, &mut a).unwrap();
        encrypt_sector(&k1, &k2, 1, &mut b).unwrap();
        assert_ne!(a[..], b[..]);
    }

    #[test]
    fn round_trip_512_byte_sector_with_aes256() {
        let key: alloc::vec::Vec<u8> = (0..64u8).collect();
        let (k1, k2) = split_key(&key).unwrap();
        let original: alloc::vec::Vec<u8> = (0..512u32).map(|i| (i * 7) as u8).collect();
        let mut data = original.clone();
        encrypt_sector(&k1, &k2, 42, &mut data).unwrap();
        assert_ne!(data, original);
        decrypt_sector(&k1, &k2, 42, &mut data).unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn mul_x_shifts_and_reduces() {
        let mut t = [0u8; 16];
        t[15] = 0x80;
        mul_x(&mut t);
        // The top bit carries out of byte 15 and folds back as 0x87 into byte 0.
        assert_eq!(t[0], 0x87);
        assert_eq!(t[15], 0);
        let mut u = [0u8; 16];
        u[0] = 1;
        mul_x(&mut u);
        assert_eq!(u[0], 2);
    }

    #[test]
    fn rejects_partial_blocks_and_odd_keys() {
        let (k1, k2) = split_key(&[0u8; 32]).unwrap();
        assert!(decrypt_sector(&k1, &k2, 0, &mut [0u8; 15]).is_none());
        assert!(decrypt_sector(&k1, &k2, 0, &mut []).is_none());
        assert!(split_key(&[0u8; 31]).is_none());
    }
}
