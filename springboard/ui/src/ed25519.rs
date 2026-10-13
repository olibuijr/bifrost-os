//! Ed25519 signatures (RFC 8032), mirroring CryptoKit's `Curve25519.Signing`.
//!
//! Pure std Rust. The field arithmetic is shared with [`crate::x25519`]; the
//! hash is SHA-512 from `bifrost_core`. Points use extended twisted Edwards
//! coordinates with the unified addition formula, and scalar multiplication
//! walks every bit with a masked select, so secret scalars never steer a branch.

use crate::x25519::{
    fe_add, fe_cswap, fe_from_bytes, fe_invert, fe_mul, fe_one, fe_sq, fe_sub, fe_to_bytes,
    fe_zero, Fe,
};
use bifrost_core::sha512::digest512;

/// Errors, named after CryptoKit's `CryptoKitError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoKitError {
    /// A raw key was not 32 bytes long.
    IncorrectKeySize,
    /// The system random source could not be read.
    UnderlyingCoreCryptoError,
}

impl std::fmt::Display for CryptoKitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoKitError::IncorrectKeySize => write!(f, "key must be 32 bytes"),
            CryptoKitError::UnderlyingCoreCryptoError => write!(f, "random source unavailable"),
        }
    }
}

impl std::error::Error for CryptoKitError {}

// ---- field helpers -------------------------------------------------------

fn fe_neg(a: &Fe) -> Fe {
    fe_sub(&fe_zero(), a)
}

fn fe_eq(a: &Fe, b: &Fe) -> bool {
    fe_to_bytes(a) == fe_to_bytes(b)
}

/// `a^e` for a little-endian exponent.
fn fe_pow(a: &Fe, e: &[u8; 32]) -> Fe {
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

fn fe_small(n: u64) -> Fe {
    Fe([n, 0, 0, 0, 0])
}

/// Edwards curve constant d = -121665 / 121666.
fn curve_d() -> Fe {
    fe_neg(&fe_mul(&fe_small(121665), &fe_invert(&fe_small(121666))))
}

/// A square root of -1: 2^((p-1)/4).
fn sqrt_m1() -> Fe {
    let mut e = [0xffu8; 32];
    e[0] = 0xfb;
    e[31] = 0x1f;
    fe_pow(&fe_small(2), &e)
}

// ---- points --------------------------------------------------------------

#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

fn identity() -> Point {
    Point { x: fe_zero(), y: fe_one(), z: fe_one(), t: fe_zero() }
}

/// Unified addition (also doubles), for a = -1.
fn point_add(p: &Point, q: &Point) -> Point {
    let d2 = fe_add(&curve_d(), &curve_d());
    let a = fe_mul(&fe_sub(&p.y, &p.x), &fe_sub(&q.y, &q.x));
    let b = fe_mul(&fe_add(&p.y, &p.x), &fe_add(&q.y, &q.x));
    let c = fe_mul(&fe_mul(&p.t, &d2), &q.t);
    let zz = fe_mul(&p.z, &q.z);
    let d = fe_add(&zz, &zz);
    let e = fe_sub(&b, &a);
    let f = fe_sub(&d, &c);
    let g = fe_add(&d, &c);
    let h = fe_add(&b, &a);
    Point { x: fe_mul(&e, &f), y: fe_mul(&g, &h), z: fe_mul(&f, &g), t: fe_mul(&e, &h) }
}

fn point_neg(p: &Point) -> Point {
    Point { x: fe_neg(&p.x), y: p.y, z: p.z, t: fe_neg(&p.t) }
}

fn point_cswap(swap: u64, a: &mut Point, b: &mut Point) {
    fe_cswap(swap, &mut a.x, &mut b.x);
    fe_cswap(swap, &mut a.y, &mut b.y);
    fe_cswap(swap, &mut a.z, &mut b.z);
    fe_cswap(swap, &mut a.t, &mut b.t);
}

/// `scalar * p` over all 256 bits, selecting with masks.
fn scalar_mul(scalar: &[u8; 32], p: &Point) -> Point {
    let mut acc = identity();
    for i in (0..256).rev() {
        acc = point_add(&acc, &acc);
        let mut sum = point_add(&acc, p);
        let bit = ((scalar[i / 8] >> (i % 8)) & 1) as u64;
        point_cswap(bit, &mut acc, &mut sum);
    }
    acc
}

fn encode(p: &Point) -> [u8; 32] {
    let zi = fe_invert(&p.z);
    let x = fe_to_bytes(&fe_mul(&p.x, &zi));
    let mut out = fe_to_bytes(&fe_mul(&p.y, &zi));
    out[31] |= (x[0] & 1) << 7;
    out
}

/// Decode a compressed point; `None` when it is not on the curve or not canonical.
fn decode(bytes: &[u8; 32]) -> Option<Point> {
    let sign = bytes[31] >> 7;
    let mut yb = *bytes;
    yb[31] &= 0x7f;
    let y = fe_from_bytes(&yb);
    if fe_to_bytes(&y) != yb {
        return None;
    }
    let y2 = fe_sq(&y);
    let u = fe_sub(&y2, &fe_one());
    let v = fe_add(&fe_mul(&curve_d(), &y2), &fe_one());
    let v3 = fe_mul(&fe_sq(&v), &v);
    let v7 = fe_mul(&fe_sq(&v3), &v);
    let mut e = [0xffu8; 32];
    e[0] = 0xfd;
    e[31] = 0x0f;
    let mut x = fe_mul(&fe_mul(&u, &v3), &fe_pow(&fe_mul(&u, &v7), &e));
    let vx2 = fe_mul(&v, &fe_sq(&x));
    if !fe_eq(&vx2, &u) {
        if !fe_eq(&vx2, &fe_neg(&u)) {
            return None;
        }
        x = fe_mul(&x, &sqrt_m1());
    }
    let xb = fe_to_bytes(&x);
    if xb == [0u8; 32] && sign == 1 {
        return None;
    }
    if (xb[0] & 1) != sign {
        x = fe_neg(&x);
    }
    let t = fe_mul(&x, &y);
    Some(Point { x, y, z: fe_one(), t })
}

fn base_point() -> Point {
    let mut b = [0x66u8; 32];
    b[0] = 0x58;
    decode(&b).expect("the base point encoding is valid")
}

// ---- scalars mod L -------------------------------------------------------

/// The group order L = 2^252 + 27742317777372353535851937790883648493, as u64 limbs.
const L: [u64; 4] = [0x5812631a5cf5d3ed, 0x14def9dea2f79cd6, 0, 0x1000000000000000];

/// `acc - L` with the final borrow (1 when `acc < L`).
fn sub_l(acc: &[u64; 4]) -> ([u64; 4], u64) {
    let mut out = [0u64; 4];
    let mut borrow = 0u64;
    for i in 0..4 {
        let (d1, b1) = acc[i].overflowing_sub(L[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        out[i] = d2;
        borrow = (b1 | b2) as u64;
    }
    (out, borrow)
}

/// Reduce a 512-bit little-endian number modulo L, bit by bit.
fn reduce_mod_l(wide: &[u8; 64]) -> [u8; 32] {
    let mut acc = [0u64; 4];
    for i in (0..512).rev() {
        let mut carry = ((wide[i / 8] >> (i % 8)) & 1) as u64;
        for limb in acc.iter_mut() {
            let next = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        let (sub, borrow) = sub_l(&acc);
        let mask = borrow.wrapping_sub(1); // all ones when acc >= L
        for k in 0..4 {
            acc[k] = (acc[k] & !mask) | (sub[k] & mask);
        }
    }
    let mut out = [0u8; 32];
    for (k, limb) in acc.iter().enumerate() {
        out[k * 8..k * 8 + 8].copy_from_slice(&limb.to_le_bytes());
    }
    out
}

fn limbs(b: &[u8; 32]) -> [u64; 4] {
    let mut l = [0u64; 4];
    for k in 0..4 {
        let mut w = [0u8; 8];
        w.copy_from_slice(&b[k * 8..k * 8 + 8]);
        l[k] = u64::from_le_bytes(w);
    }
    l
}

/// `(r + k * a) mod L`.
fn mul_add_mod_l(k: &[u8; 32], a: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let (kl, al, rl) = (limbs(k), limbs(a), limbs(r));
    let mut prod = [0u64; 9];
    for i in 0..4 {
        let mut carry = 0u128;
        for j in 0..4 {
            let cur = prod[i + j] as u128 + kl[i] as u128 * al[j] as u128 + carry;
            prod[i + j] = cur as u64;
            carry = cur >> 64;
        }
        prod[i + 4] = carry as u64;
    }
    let mut carry = 0u128;
    for i in 0..9 {
        let add = if i < 4 { rl[i] as u128 } else { 0 };
        let cur = prod[i] as u128 + add + carry;
        prod[i] = cur as u64;
        carry = cur >> 64;
    }
    let mut wide = [0u8; 64];
    for i in 0..8 {
        wide[i * 8..i * 8 + 8].copy_from_slice(&prod[i].to_le_bytes());
    }
    reduce_mod_l(&wide)
}

/// True when the 32-byte little-endian `s` is below L.
fn is_canonical_scalar(s: &[u8; 32]) -> bool {
    sub_l(&limbs(s)).1 == 1
}

fn hash_scalar(parts: &[&[u8]]) -> [u8; 32] {
    let mut buf = Vec::new();
    for p in parts {
        buf.extend_from_slice(p);
    }
    reduce_mod_l(&digest512(&buf))
}

fn key_bytes(raw: &[u8]) -> Result<[u8; 32], CryptoKitError> {
    <[u8; 32]>::try_from(raw).map_err(|_| CryptoKitError::IncorrectKeySize)
}

// ---- Curve25519.Signing --------------------------------------------------

/// `Curve25519.Signing.PublicKey`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PublicKey {
    raw: [u8; 32],
}

impl PublicKey {
    /// `init(rawRepresentation:)`. Only the length is checked here; a key that
    /// is not a curve point never validates a signature.
    pub fn from_raw_representation(raw: &[u8]) -> Result<Self, CryptoKitError> {
        Ok(PublicKey { raw: key_bytes(raw)? })
    }

    /// `rawRepresentation`.
    pub fn raw_representation(&self) -> [u8; 32] {
        self.raw
    }

    /// `isValidSignature(_:for:)`.
    pub fn is_valid_signature(&self, signature: &[u8], data: &[u8]) -> bool {
        let Ok(sig) = <[u8; 64]>::try_from(signature) else {
            return false;
        };
        let mut r_bytes = [0u8; 32];
        let mut s_bytes = [0u8; 32];
        r_bytes.copy_from_slice(&sig[..32]);
        s_bytes.copy_from_slice(&sig[32..]);
        if !is_canonical_scalar(&s_bytes) {
            return false;
        }
        let (Some(a), Some(r)) = (decode(&self.raw), decode(&r_bytes)) else {
            return false;
        };
        let k = hash_scalar(&[&r_bytes, &self.raw, data]);
        // [S]B == R + [k]A  <=>  [S]B - [k]A == R
        let lhs = point_add(&scalar_mul(&s_bytes, &base_point()), &point_neg(&scalar_mul(&k, &a)));
        encode(&lhs) == encode(&r)
    }
}

/// `Curve25519.Signing.PrivateKey`.
#[derive(Clone)]
pub struct PrivateKey {
    seed: [u8; 32],
}

impl PrivateKey {
    /// `init()`: a fresh key from the system random source.
    pub fn generate() -> Result<Self, CryptoKitError> {
        use std::io::Read;
        let mut seed = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut seed))
            .map_err(|_| CryptoKitError::UnderlyingCoreCryptoError)?;
        Ok(PrivateKey { seed })
    }

    /// `init(rawRepresentation:)`: the 32-byte seed.
    pub fn from_raw_representation(raw: &[u8]) -> Result<Self, CryptoKitError> {
        Ok(PrivateKey { seed: key_bytes(raw)? })
    }

    /// `rawRepresentation`.
    pub fn raw_representation(&self) -> [u8; 32] {
        self.seed
    }

    fn expanded(&self) -> ([u8; 32], [u8; 32]) {
        let h = digest512(&self.seed);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..32]);
        a[0] &= 248;
        a[31] &= 127;
        a[31] |= 64;
        let mut prefix = [0u8; 32];
        prefix.copy_from_slice(&h[32..]);
        (a, prefix)
    }

    /// `publicKey`.
    pub fn public_key(&self) -> PublicKey {
        let (a, _) = self.expanded();
        PublicKey { raw: encode(&scalar_mul(&a, &base_point())) }
    }

    /// `signature(for:)`: the 64-byte signature R || S.
    pub fn signature_for(&self, data: &[u8]) -> [u8; 64] {
        let (a, prefix) = self.expanded();
        let public = self.public_key().raw;
        let r = hash_scalar(&[&prefix, data]);
        let r_bytes = encode(&scalar_mul(&r, &base_point()));
        let k = hash_scalar(&[&r_bytes, &public, data]);
        let s = mul_add_mod_l(&k, &a, &r);
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&r_bytes);
        sig[32..].copy_from_slice(&s);
        sig
    }
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrivateKey(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    fn check(sk: &str, pk: Option<&str>, msg: &[u8], sig: &str) {
        let key = PrivateKey::from_raw_representation(&hex(sk)).unwrap();
        if let Some(pk) = pk {
            assert_eq!(key.public_key().raw_representation().to_vec(), hex(pk));
        }
        let signature = key.signature_for(msg);
        assert_eq!(signature.to_vec(), hex(sig));
        assert!(key.public_key().is_valid_signature(&signature, msg));
    }

    #[test]
    fn rfc8032_test_1_empty_message() {
        check(
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            Some("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
            b"",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
    }

    #[test]
    fn rfc8032_test_2_one_byte() {
        check(
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            Some("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"),
            &[0x72],
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        );
    }

    #[test]
    fn rfc8032_test_3_two_bytes() {
        check(
            "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
            None,
            &[0xaf, 0x82],
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        );
    }

    #[test]
    fn rfc8032_test_sha_abc() {
        let msg = digest512(b"abc");
        check(
            "833fe62409237b9d62ec77587520911e9a759cec1d19755b7da901b96dca3d42",
            Some("ec172b93ad5e563bf4932c70e1245034c35467ef2efd4d64ebf819683467e2bf"),
            &msg,
            "dc2a4459e7369633a52b1bf277839a00201009a3efbf3ecb69bea2186c26b58909351fc9ac90b3ecfdfbc7c66431e0303dca179c138ac17ad9bef1177331a704",
        );
    }

    #[test]
    fn tampered_message_is_rejected() {
        let key = PrivateKey::from_raw_representation(&[7u8; 32]).unwrap();
        let sig = key.signature_for(b"update 1.0");
        assert!(!key.public_key().is_valid_signature(&sig, b"update 1.1"));
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let key = PrivateKey::from_raw_representation(&[9u8; 32]).unwrap();
        let mut sig = key.signature_for(b"m");
        sig[40] ^= 1;
        assert!(!key.public_key().is_valid_signature(&sig, b"m"));
        assert!(!key.public_key().is_valid_signature(&sig[..63], b"m"));
    }

    #[test]
    fn other_key_is_rejected() {
        let a = PrivateKey::from_raw_representation(&[1u8; 32]).unwrap();
        let b = PrivateKey::from_raw_representation(&[2u8; 32]).unwrap();
        let sig = a.signature_for(b"m");
        assert!(!b.public_key().is_valid_signature(&sig, b"m"));
    }

    #[test]
    fn non_canonical_s_is_rejected() {
        let key = PrivateKey::from_raw_representation(&[3u8; 32]).unwrap();
        let mut sig = key.signature_for(b"m");
        sig[63] = 0xff; // S >= L
        assert!(!key.public_key().is_valid_signature(&sig, b"m"));
    }

    #[test]
    fn bad_length_raw_representation_is_refused() {
        assert_eq!(PrivateKey::from_raw_representation(&[0u8; 31]).unwrap_err(), CryptoKitError::IncorrectKeySize);
        assert_eq!(PublicKey::from_raw_representation(&[0u8; 33]).unwrap_err(), CryptoKitError::IncorrectKeySize);
    }

    #[test]
    fn invalid_public_key_never_verifies() {
        let key = PublicKey::from_raw_representation(&[0xffu8; 32]).unwrap();
        assert!(!key.is_valid_signature(&[0u8; 64], b"m"));
    }

    #[test]
    fn raw_representation_round_trips() {
        let key = PrivateKey::from_raw_representation(&[5u8; 32]).unwrap();
        assert_eq!(key.raw_representation(), [5u8; 32]);
        let public = key.public_key();
        assert_eq!(PublicKey::from_raw_representation(&public.raw_representation()).unwrap(), public);
    }

    #[test]
    fn generated_keys_sign_and_verify() {
        let key = PrivateKey::generate().unwrap();
        let sig = key.signature_for(b"hello");
        assert!(key.public_key().is_valid_signature(&sig, b"hello"));
    }
}
