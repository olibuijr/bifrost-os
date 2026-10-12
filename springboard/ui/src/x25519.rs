//! X25519 key agreement (RFC 7748) with a shared secret type.
//!
//! Pure std Rust. The field is GF(2^255 - 19) held in five 51-bit limbs. The
//! Montgomery ladder runs over all 255 scalar bits with the same sequence of
//! operations for every bit; swaps are masks, not branches.
//!
//! Randomness is the caller's job: `PrivateKey::from_raw` takes the 32 bytes.
//! Key derivation (HKDF and X9.63) needs SHA-256 and is not here yet, so
//! `SharedSecret` exposes the raw agreement output only.

const MASK51: u64 = (1 << 51) - 1;

/// A field element in radix 2^51. Limbs are at most about 2^52 between operations.
#[derive(Clone, Copy)]
struct Fe([u64; 5]);

/// Coefficient (A - 2) / 4 for A = 486662.
const A24: Fe = Fe([121665, 0, 0, 0, 0]);

fn fe_zero() -> Fe {
    Fe([0; 5])
}

fn fe_one() -> Fe {
    Fe([1, 0, 0, 0, 0])
}

/// Carry the wide limbs back into 51-bit limbs, folding 2^255 as 19.
fn reduce(mut t: [u128; 5]) -> Fe {
    let m = MASK51 as u128;
    for i in 0..4 {
        let c = t[i] >> 51;
        t[i] &= m;
        t[i + 1] += c;
    }
    let c = t[4] >> 51;
    t[4] &= m;
    t[0] += c * 19;
    let c = t[0] >> 51;
    t[0] &= m;
    t[1] += c;
    Fe([t[0] as u64, t[1] as u64, t[2] as u64, t[3] as u64, t[4] as u64])
}

fn fe_add(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0u128; 5];
    for i in 0..5 {
        t[i] = a.0[i] as u128 + b.0[i] as u128;
    }
    reduce(t)
}

/// `a - b` computed as `a + 2p - b`, so every limb stays non-negative.
fn fe_sub(a: &Fe, b: &Fe) -> Fe {
    const TWO_P0: u128 = 0xf_ffff_ffff_ffda; // 2 * (2^51 - 19)
    const TWO_PN: u128 = 0xf_ffff_ffff_fffe; // 2 * (2^51 - 1)
    let mut t = [0u128; 5];
    for i in 0..5 {
        let two_p = if i == 0 { TWO_P0 } else { TWO_PN };
        t[i] = a.0[i] as u128 + two_p - b.0[i] as u128;
    }
    reduce(t)
}

fn fe_mul(a: &Fe, b: &Fe) -> Fe {
    let [a0, a1, a2, a3, a4] = a.0.map(|x| x as u128);
    let [b0, b1, b2, b3, b4] = b.0.map(|x| x as u128);
    let (b1_19, b2_19, b3_19, b4_19) = (b1 * 19, b2 * 19, b3 * 19, b4 * 19);
    let t0 = a0 * b0 + a1 * b4_19 + a2 * b3_19 + a3 * b2_19 + a4 * b1_19;
    let t1 = a0 * b1 + a1 * b0 + a2 * b4_19 + a3 * b3_19 + a4 * b2_19;
    let t2 = a0 * b2 + a1 * b1 + a2 * b0 + a3 * b4_19 + a4 * b3_19;
    let t3 = a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0 + a4 * b4_19;
    let t4 = a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0;
    reduce([t0, t1, t2, t3, t4])
}

fn fe_sq(a: &Fe) -> Fe {
    fe_mul(a, a)
}

/// Swap `a` and `b` when `swap` is 1, leave them alone when it is 0.
fn fe_cswap(swap: u64, a: &mut Fe, b: &mut Fe) {
    let mask = 0u64.wrapping_sub(swap);
    for i in 0..5 {
        let x = mask & (a.0[i] ^ b.0[i]);
        a.0[i] ^= x;
        b.0[i] ^= x;
    }
}

/// `a^(p-2)`, the inverse by Fermat. The exponent bytes are `eb ff .. ff 7f`.
fn fe_invert(a: &Fe) -> Fe {
    let mut e = [0xffu8; 32];
    e[0] = 0xeb;
    e[31] = 0x7f;
    let mut result = fe_one();
    for byte in (0..32).rev() {
        for bit in (0..8).rev() {
            result = fe_sq(&result);
            if (e[byte] >> bit) & 1 == 1 {
                result = fe_mul(&result, a);
            }
        }
    }
    result
}

fn le64(s: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&s[..8]);
    u64::from_le_bytes(a)
}

/// Decode 32 little-endian bytes. Bit 255 is ignored, as RFC 7748 requires.
fn fe_from_bytes(b: &[u8; 32]) -> Fe {
    Fe([
        le64(&b[0..8]) & MASK51,
        (le64(&b[6..14]) >> 3) & MASK51,
        (le64(&b[12..20]) >> 6) & MASK51,
        (le64(&b[19..27]) >> 1) & MASK51,
        (le64(&b[24..32]) >> 12) & MASK51,
    ])
}

/// Encode canonically (fully reduced modulo p) as 32 little-endian bytes.
fn fe_to_bytes(a: &Fe) -> [u8; 32] {
    let mut h = reduce(a.0.map(|x| x as u128)).0;
    // q is 1 when h >= p, else 0.
    let mut q = (h[0] + 19) >> 51;
    q = (h[1] + q) >> 51;
    q = (h[2] + q) >> 51;
    q = (h[3] + q) >> 51;
    q = (h[4] + q) >> 51;
    h[0] += 19 * q;
    for i in 0..4 {
        let c = h[i] >> 51;
        h[i] &= MASK51;
        h[i + 1] += c;
    }
    h[4] &= MASK51;

    let mut out = [0u8; 32];
    let mut acc: u128 = 0;
    let mut bits: u32 = 0;
    let mut idx = 0;
    for limb in h {
        acc |= (limb as u128) << bits;
        bits += 51;
        while bits >= 8 {
            out[idx] = acc as u8;
            idx += 1;
            acc >>= 8;
            bits -= 8;
        }
    }
    // 255 bits in total: 31 full bytes above, then 7 bits left for byte 31.
    out[idx] = acc as u8;
    out
}

/// The X25519 function of RFC 7748: `scalar` times the point with u-coordinate `u`.
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let x1 = fe_from_bytes(u);
    let mut x2 = fe_one();
    let mut z2 = fe_zero();
    let mut x3 = x1;
    let mut z3 = fe_one();
    let mut swap: u64 = 0;

    for t in (0..255).rev() {
        let kt = ((k[t >> 3] >> (t & 7)) & 1) as u64;
        swap ^= kt;
        fe_cswap(swap, &mut x2, &mut x3);
        fe_cswap(swap, &mut z2, &mut z3);
        swap = kt;

        let a = fe_add(&x2, &z2);
        let aa = fe_sq(&a);
        let b = fe_sub(&x2, &z2);
        let bb = fe_sq(&b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x3, &z3);
        let d = fe_sub(&x3, &z3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        x3 = fe_sq(&fe_add(&da, &cb));
        z3 = fe_mul(&x1, &fe_sq(&fe_sub(&da, &cb)));
        x2 = fe_mul(&aa, &bb);
        z2 = fe_mul(&e, &fe_add(&aa, &fe_mul(&A24, &e)));
    }
    fe_cswap(swap, &mut x2, &mut x3);
    fe_cswap(swap, &mut z2, &mut z3);

    fe_to_bytes(&fe_mul(&x2, &fe_invert(&z2)))
}

/// The u-coordinate of the standard base point (9).
pub fn base_point() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
}

/// Why a key or an agreement was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// A key was not 32 bytes long. Holds the length that was given.
    BadLength(usize),
    /// The peer key has small order, so the shared secret is all zero.
    LowOrderPoint,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::BadLength(n) => write!(f, "key must be 32 bytes, got {n}"),
            KeyError::LowOrderPoint => {
                write!(f, "shared secret is all zero: the peer key has small order")
            }
        }
    }
}

impl std::error::Error for KeyError {}

fn key_bytes(raw: &[u8]) -> Result<[u8; 32], KeyError> {
    if raw.len() != 32 {
        return Err(KeyError::BadLength(raw.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(raw);
    Ok(out)
}

/// An X25519 public key: a 32-byte u-coordinate.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey {
    raw: [u8; 32],
}

impl PublicKey {
    /// Wrap 32 raw bytes. Any 32 bytes are accepted; small-order keys are
    /// refused later, when the agreement is computed.
    pub fn from_raw(raw: &[u8]) -> Result<Self, KeyError> {
        Ok(PublicKey { raw: key_bytes(raw)? })
    }

    pub fn raw(&self) -> [u8; 32] {
        self.raw
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey(")?;
        for b in self.raw {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

/// An X25519 private key: a 32-byte scalar, clamped when used.
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKey {
    raw: [u8; 32],
}

impl PrivateKey {
    /// Wrap 32 raw bytes from the caller's random source.
    pub fn from_raw(raw: &[u8]) -> Result<Self, KeyError> {
        Ok(PrivateKey { raw: key_bytes(raw)? })
    }

    pub fn raw(&self) -> [u8; 32] {
        self.raw
    }

    /// The matching public key: the scalar times the base point.
    pub fn public_key(&self) -> PublicKey {
        PublicKey {
            raw: x25519(&self.raw, &base_point()),
        }
    }

    /// The shared secret with `peer`. Fails when `peer` has small order.
    pub fn shared_secret(&self, peer: &PublicKey) -> Result<SharedSecret, KeyError> {
        let out = x25519(&self.raw, &peer.raw);
        if out.iter().fold(0u8, |acc, b| acc | b) == 0 {
            return Err(KeyError::LowOrderPoint);
        }
        Ok(SharedSecret { raw: out })
    }
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrivateKey(..)")
    }
}

/// The 32-byte output of a key agreement. Equality runs in constant time.
#[derive(Clone)]
pub struct SharedSecret {
    raw: [u8; 32],
}

impl SharedSecret {
    pub fn bytes(&self) -> &[u8; 32] {
        &self.raw
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

impl PartialEq for SharedSecret {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for i in 0..32 {
            diff |= self.raw[i] ^ other.raw[i];
        }
        diff == 0
    }
}

impl Eq for SharedSecret {}

impl std::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedSecret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    // RFC 7748 section 5.2, first test vector.
    #[test]
    fn rfc7748_vector_one() {
        let k = hex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
        let u = hex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
        let out = x25519(&k, &u);
        assert_eq!(
            out,
            hex32("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        );
    }

    // RFC 7748 section 5.2: one iteration of the function starting from 9.
    #[test]
    fn rfc7748_one_iteration_from_base_point() {
        let b = base_point();
        assert_eq!(
            x25519(&b, &b),
            hex32("422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079")
        );
    }

    // RFC 7748 section 6.1: both parties derive the same secret.
    #[test]
    fn diffie_hellman_both_sides_agree() {
        let alice = PrivateKey::from_raw(&hex32(
            "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
        ))
        .unwrap();
        let bob = PrivateKey::from_raw(&hex32(
            "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
        ))
        .unwrap();
        assert_eq!(
            alice.public_key().raw(),
            hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            bob.public_key().raw(),
            hex32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let expected = hex32("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        let ab = alice.shared_secret(&bob.public_key()).unwrap();
        let ba = bob.shared_secret(&alice.public_key()).unwrap();
        assert_eq!(ab.bytes(), &expected);
        assert_eq!(ab, ba);
    }

    // The all-zero u-coordinate has small order, so agreement must fail.
    #[test]
    fn zero_peer_key_is_refused() {
        let me = PrivateKey::from_raw(&[7u8; 32]).unwrap();
        let peer = PublicKey::from_raw(&[0u8; 32]).unwrap();
        assert_eq!(me.shared_secret(&peer), Err(KeyError::LowOrderPoint));
    }

    #[test]
    fn keys_must_be_32_bytes() {
        assert_eq!(PrivateKey::from_raw(&[0u8; 31]).err(), Some(KeyError::BadLength(31)));
        assert_eq!(PublicKey::from_raw(&[0u8; 33]).err(), Some(KeyError::BadLength(33)));
        assert_eq!(
            KeyError::BadLength(31).to_string(),
            "key must be 32 bytes, got 31"
        );
    }

    // The field prime p = 2^255 - 19 encodes as bytes ed ff .. ff 7f and reduces to zero.
    #[test]
    fn field_encoding_reduces_p_to_zero() {
        let mut p = [0xffu8; 32];
        p[0] = 0xed;
        p[31] = 0x7f;
        assert_eq!(fe_to_bytes(&fe_from_bytes(&p)), [0u8; 32]);
        let mut one = [0u8; 32];
        one[0] = 1;
        assert_eq!(fe_to_bytes(&fe_from_bytes(&one)), one);
    }

    // 2 * 3 = 6 and 6 - 6 = 0 in the field, checked through the byte encoding.
    #[test]
    fn field_arithmetic_small_values() {
        let mut two = [0u8; 32];
        two[0] = 2;
        let mut three = [0u8; 32];
        three[0] = 3;
        let mut six = [0u8; 32];
        six[0] = 6;
        let prod = fe_mul(&fe_from_bytes(&two), &fe_from_bytes(&three));
        assert_eq!(fe_to_bytes(&prod), six);
        let diff = fe_sub(&prod, &fe_from_bytes(&six));
        assert_eq!(fe_to_bytes(&diff), [0u8; 32]);
        // 0 - 1 is p - 1, which encodes as ec ff .. ff 7f.
        let neg_one = fe_sub(&fe_zero(), &fe_one());
        let mut p_minus_1 = [0xffu8; 32];
        p_minus_1[0] = 0xec;
        p_minus_1[31] = 0x7f;
        assert_eq!(fe_to_bytes(&neg_one), p_minus_1);
    }
}
