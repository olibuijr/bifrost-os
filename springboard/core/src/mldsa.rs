//! ML-DSA (FIPS 204): the module-lattice digital signature algorithm, parameter sets ML-DSA-65 and ML-DSA-87.
//!
//! Integer arithmetic over Z_q with q = 8380417: the NTT and its inverse (FIPS 204 Algorithms 41 and 42, zeta = 1753),
//! Power2Round, Decompose, MakeHint and UseHint, the bit and hint packings, SampleInBall, and the signing loop with its
//! rejection checks. The hashes come from [`crate::sha3`]: H is SHAKE256 and the matrix XOF is SHAKE128.
//!
//! The functions take their seeds from the caller: `xi` for key generation and `rnd` for signing. A zero `rnd` gives
//! the deterministic variant; a fresh random `rnd` gives the hedged variant. This module draws no randomness.
//!
//! Status: the arithmetic is checked by round trips, a schoolbook product and the size tables of FIPS 204, not by the
//! official FIPS 204 known-answer vectors, which are not in the repo yet. Interoperability with other ML-DSA
//! implementations is unverified until those vectors pass.

use crate::sha3::Shake;

/// The prime modulus q = 8380417 = 2^23 - 2^13 + 1.
const Q: i32 = 8_380_417;
/// Coefficients per polynomial (the ring is Z_q[X] / (X^256 + 1)).
const N: usize = 256;
/// Dropped low bits of the public key: Power2Round uses d = 13.
const D: u32 = 13;
/// Range of the masking vector `y`: coefficients lie in (-GAMMA1, GAMMA1].
const GAMMA1: i32 = 1 << 19;
/// Low-order rounding range of Decompose: GAMMA2 = (q - 1) / 32 for both parameter sets.
const GAMMA2: i32 = (Q - 1) / 32;
/// Decompose modulus alpha = 2 GAMMA2.
const ALPHA: i32 = 2 * GAMMA2;
/// Number of high-order values, m = (q - 1) / alpha = 16.
const M: i32 = (Q - 1) / ALPHA;
/// 256^-1 mod q, the final scale of the inverse NTT.
const N_INV: i32 = 8_347_681;
/// Bounds of the packed `t0`: coefficients lie in (-2^(d-1), 2^(d-1)].
const T0_A: i32 = (1 << (D - 1)) - 1;
const T0_B: i32 = 1 << (D - 1);

/// A polynomial. Values are either canonical in `[0, q)` or small signed values, depending on the step; each function
/// says which one it takes.
type Poly = [i32; N];

/// The two ML-DSA parameter sets of FIPS 204 this module implements (ML-DSA-44 is left out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Params {
    MlDsa65,
    MlDsa87,
}

impl Params {
    /// Rows of the public matrix: the number of polynomials in `t`, `t1` and `t0`.
    pub fn k(self) -> usize {
        match self {
            Params::MlDsa65 => 6,
            Params::MlDsa87 => 8,
        }
    }

    /// Columns of the public matrix: the number of polynomials in `s1`, `y` and `z`.
    pub fn l(self) -> usize {
        match self {
            Params::MlDsa65 => 5,
            Params::MlDsa87 => 7,
        }
    }

    /// Bound eta of the secret distribution.
    fn eta(self) -> i32 {
        match self {
            Params::MlDsa65 => 4,
            Params::MlDsa87 => 2,
        }
    }

    /// Number of nonzero coefficients of the challenge polynomial c.
    fn tau(self) -> usize {
        match self {
            Params::MlDsa65 => 49,
            Params::MlDsa87 => 60,
        }
    }

    /// beta = tau * eta.
    fn beta(self) -> i32 {
        match self {
            Params::MlDsa65 => 196,
            Params::MlDsa87 => 120,
        }
    }

    /// Maximum number of hint bits.
    fn omega(self) -> usize {
        match self {
            Params::MlDsa65 => 55,
            Params::MlDsa87 => 75,
        }
    }

    /// Length of the commitment hash c~ in bytes (lambda / 4).
    fn c_tilde_len(self) -> usize {
        match self {
            Params::MlDsa65 => 48,
            Params::MlDsa87 => 64,
        }
    }

    /// Bytes of one packed secret polynomial: 32 times the bits per coefficient, 4 for eta 4 and 3 for eta 2.
    fn s_poly_len(self) -> usize {
        32 * bitlen((2 * self.eta()) as u32)
    }

    /// Public key: the 32-byte seed `rho`, then `t1` at 10 bits per coefficient.
    pub fn public_key_len(self) -> usize {
        32 + 320 * self.k()
    }

    /// Secret key: `rho`, `K`, `tr`, then `s1`, `s2` and `t0` (13 bits per coefficient).
    pub fn secret_key_len(self) -> usize {
        128 + self.s_poly_len() * (self.k() + self.l()) + 416 * self.k()
    }

    /// Signature: the commitment hash `c~`, then `z` at 20 bits per coefficient, then the hint bytes.
    pub fn signature_len(self) -> usize {
        self.c_tilde_len() + 640 * self.l() + self.omega() + self.k()
    }
}

/// Why a key or a context was refused. Signature verification reports a bad signature as `Ok(false)`, not as an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    PublicKeyLength,
    SecretKeyLength,
    /// The context string is longer than 255 bytes (FIPS 204 Algorithms 2 and 3).
    ContextTooLong,
}

/// An ML-DSA key pair in the byte layout FIPS 204 gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPair {
    pub public_key: Vec<u8>,
    pub secret_key: Vec<u8>,
}

/// Canonical addition in Z_q; both inputs are in `[0, q)`.
fn fadd(a: i32, b: i32) -> i32 {
    (a + b) % Q
}

/// Canonical subtraction in Z_q; both inputs are in `[0, q)`.
fn fsub(a: i32, b: i32) -> i32 {
    (a - b).rem_euclid(Q)
}

/// Canonical product in Z_q; both inputs are in `[0, q)`.
fn fmul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) % Q as i64) as i32
}

/// The centered representative of `x` mod q, in `(-q/2, q/2]`.
fn centered(x: i32) -> i32 {
    let r = x.rem_euclid(Q);
    if r > Q / 2 {
        r - Q
    } else {
        r
    }
}

/// The largest absolute centered coefficient over a vector of polynomials (the infinity norm).
fn max_abs(polys: &[Poly]) -> i32 {
    polys.iter().flat_map(|p| p.iter()).map(|&x| centered(x).abs()).max().unwrap_or(0)
}

fn poly_add(a: &Poly, b: &Poly) -> Poly {
    std::array::from_fn(|i| fadd(a[i], b[i]))
}

fn poly_sub(a: &Poly, b: &Poly) -> Poly {
    std::array::from_fn(|i| fsub(a[i], b[i]))
}

/// Pointwise product in the NTT domain.
fn pointwise(a: &Poly, b: &Poly) -> Poly {
    std::array::from_fn(|i| fmul(a[i], b[i]))
}

const fn pow_mod(base: u64, mut exp: u64) -> u64 {
    let q = Q as u64;
    let mut b = base % q;
    let mut acc = 1u64;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = acc * b % q;
        }
        b = b * b % q;
        exp >>= 1;
    }
    acc
}

const fn bit_reverse8(x: usize) -> usize {
    let mut r = 0;
    let mut i = 0;
    while i < 8 {
        r = (r << 1) | ((x >> i) & 1);
        i += 1;
    }
    r
}

/// `ZETAS[k] = 1753^BitRev8(k) mod q`, the twiddle factors of FIPS 204 Algorithms 41 and 42.
const ZETAS: [i32; N] = {
    let mut t = [0i32; N];
    let mut k = 0;
    while k < N {
        t[k] = pow_mod(1753, bit_reverse8(k) as u64) as i32;
        k += 1;
    }
    t
};

/// NTT (FIPS 204 Algorithm 41), in place. The input may be any `i32` values: they are reduced into `[0, q)` first.
fn ntt(f: &mut Poly) {
    for c in f.iter_mut() {
        *c = c.rem_euclid(Q);
    }
    let mut m = 0usize;
    let mut len = N / 2;
    while len >= 1 {
        let mut start = 0;
        while start < N {
            m += 1;
            let zeta = ZETAS[m];
            for j in start..start + len {
                let t = fmul(zeta, f[j + len]);
                let a = f[j];
                f[j + len] = fsub(a, t);
                f[j] = fadd(a, t);
            }
            start += 2 * len;
        }
        len >>= 1;
    }
}

/// Inverse NTT (FIPS 204 Algorithm 42), in place, with the final scale by 256^-1. The output is canonical.
fn inv_ntt(f: &mut Poly) {
    let mut m = N;
    let mut len = 1;
    while len < N {
        let mut start = 0;
        while start < N {
            m -= 1;
            let zeta = fsub(0, ZETAS[m]);
            for j in start..start + len {
                let t = f[j];
                f[j] = fadd(t, f[j + len]);
                f[j + len] = fmul(zeta, fsub(t, f[j + len]));
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    for c in f.iter_mut() {
        *c = fmul(*c, N_INV);
    }
}

fn ntt_vec(v: &[Poly]) -> Vec<Poly> {
    v.iter()
        .map(|p| {
            let mut f = *p;
            ntt(&mut f);
            f
        })
        .collect()
}

fn inv_ntt_vec(v: &[Poly]) -> Vec<Poly> {
    v.iter()
        .map(|p| {
            let mut f = *p;
            inv_ntt(&mut f);
            f
        })
        .collect()
}

/// `A * v` with the matrix and the vector in the NTT domain: row `i` is the sum over `j` of `A[i][j] * v[j]`.
fn mul_matrix(a: &[Vec<Poly>], v_hat: &[Poly]) -> Vec<Poly> {
    a.iter()
        .map(|row| {
            let mut acc = [0i32; N];
            for (x, y) in row.iter().zip(v_hat) {
                acc = poly_add(&acc, &pointwise(x, y));
            }
            acc
        })
        .collect()
}

/// Number of bits needed to write `x` (0 for 0).
fn bitlen(x: u32) -> usize {
    (32 - x.leading_zeros()) as usize
}

/// SHAKE256 of the concatenation of `parts`, written to `out`.
fn shake256(parts: &[&[u8]], out: &mut [u8]) {
    let mut h = Shake::new_256();
    for part in parts {
        h.update(part);
    }
    h.squeeze(out);
}

/// Writes `values` (each in `[0, 2^bits)`) at `bits` bits per value, least significant bit first.
fn pack_bits(values: &Poly, bits: usize, out: &mut Vec<u8>) {
    let mut buf = vec![0u8; N * bits / 8];
    for (i, &v) in values.iter().enumerate() {
        for j in 0..bits {
            let pos = i * bits + j;
            buf[pos / 8] |= (((v >> j) & 1) as u8) << (pos % 8);
        }
    }
    out.extend_from_slice(&buf);
}

/// Reads one polynomial written by [`pack_bits`]; `bytes` must hold exactly `N * bits / 8` bytes.
fn unpack_bits(bytes: &[u8], bits: usize) -> Poly {
    let mut f = [0i32; N];
    for (i, c) in f.iter_mut().enumerate() {
        let mut v = 0i32;
        for j in 0..bits {
            let pos = i * bits + j;
            v |= (((bytes[pos / 8] >> (pos % 8)) & 1) as i32) << j;
        }
        *c = v;
    }
    f
}

/// SimpleBitPack (FIPS 204 Algorithm 16): coefficients in `[0, b]`, `bitlen(b)` bits each.
fn simple_bit_pack(p: &Poly, b: i32, out: &mut Vec<u8>) {
    pack_bits(p, bitlen(b as u32), out);
}

/// SimpleBitUnpack (FIPS 204 Algorithm 18).
fn simple_bit_unpack(bytes: &[u8], b: i32) -> Poly {
    unpack_bits(bytes, bitlen(b as u32))
}

/// BitPack (FIPS 204 Algorithm 17): coefficients in `[-a, b]` stored as `b - w` in `bitlen(a + b)` bits.
fn bit_pack(p: &Poly, a: i32, b: i32, out: &mut Vec<u8>) {
    let w: Poly = std::array::from_fn(|i| b - p[i]);
    pack_bits(&w, bitlen((a + b) as u32), out);
}

/// BitUnpack (FIPS 204 Algorithm 19).
fn bit_unpack(bytes: &[u8], a: i32, b: i32) -> Poly {
    let v = unpack_bits(bytes, bitlen((a + b) as u32));
    std::array::from_fn(|i| b - v[i])
}

/// HintBitPack (FIPS 204 Algorithm 20): the positions of the ones of each hint polynomial, then one end index per
/// polynomial. The result is `omega + k` bytes. The caller has already checked that there are at most `omega` ones.
fn hint_bit_pack(h: &[Poly], omega: usize, out: &mut Vec<u8>) {
    let k = h.len();
    let mut y = vec![0u8; omega + k];
    let mut index = 0usize;
    for (i, poly) in h.iter().enumerate() {
        for (j, &bit) in poly.iter().enumerate() {
            if bit != 0 {
                y[index] = j as u8;
                index += 1;
            }
        }
        y[omega + i] = index as u8;
    }
    out.extend_from_slice(&y);
}

/// HintBitUnpack (FIPS 204 Algorithm 21). `None` for a malformed encoding: unsorted or repeated positions, end indices
/// that go backwards or past `omega`, or nonzero padding.
fn hint_bit_unpack(y: &[u8], k: usize, omega: usize) -> Option<Vec<Poly>> {
    let mut h = vec![[0i32; N]; k];
    let mut index = 0usize;
    for i in 0..k {
        let end = y[omega + i] as usize;
        if end < index || end > omega {
            return None;
        }
        let first = index;
        while index < end {
            if index > first && y[index - 1] >= y[index] {
                return None;
            }
            h[i][y[index] as usize] = 1;
            index += 1;
        }
    }
    if y[index..omega].iter().any(|&b| b != 0) {
        return None;
    }
    Some(h)
}

/// Power2Round (FIPS 204 Algorithm 35) of a canonical value: `r = r1 * 2^d + r0` with `r0` centered.
fn power2round(r: i32) -> (i32, i32) {
    let rp = r.rem_euclid(Q);
    let mut r0 = rp & ((1 << D) - 1);
    if r0 > (1 << (D - 1)) {
        r0 -= 1 << D;
    }
    ((rp - r0) >> D, r0)
}

/// Decompose (FIPS 204 Algorithm 36): `r = r1 * alpha + r0` with `r0` centered mod alpha, and `r1` in `[0, m)`.
fn decompose(r: i32) -> (i32, i32) {
    let rp = r.rem_euclid(Q);
    let mut r0 = rp.rem_euclid(ALPHA);
    if r0 > GAMMA2 {
        r0 -= ALPHA;
    }
    if rp - r0 == Q - 1 {
        (0, r0 - 1)
    } else {
        ((rp - r0) / ALPHA, r0)
    }
}

fn high_bits(r: i32) -> i32 {
    decompose(r).0
}

fn low_bits(r: i32) -> i32 {
    decompose(r).1
}

/// MakeHint (FIPS 204 Algorithm 39) for canonical `z` and `r`: true when adding `z` changes the high bits of `r`.
fn make_hint(z: i32, r: i32) -> bool {
    high_bits(r) != high_bits(fadd(r, z))
}

/// UseHint (FIPS 204 Algorithm 40): moves the high bits of `r` by one step when the hint is set.
fn use_hint(hint: bool, r: i32) -> i32 {
    let (r1, r0) = decompose(r);
    if !hint {
        r1
    } else if r0 > 0 {
        (r1 + 1).rem_euclid(M)
    } else {
        (r1 - 1).rem_euclid(M)
    }
}

/// RejNTTPoly (FIPS 204 Algorithm 30): uniform coefficients in `[0, q)` from 3-byte chunks of a SHAKE128 stream. The
/// top bit of the third byte is cleared, which keeps the value below 2^23, and values not below q are skipped.
fn rej_ntt_poly(xof: &mut Shake) -> Poly {
    let mut a = [0i32; N];
    let mut j = 0;
    while j < N {
        let mut b = [0u8; 3];
        xof.squeeze(&mut b);
        let z = b[0] as i32 | ((b[1] as i32) << 8) | (((b[2] & 0x7f) as i32) << 16);
        if z < Q {
            a[j] = z;
            j += 1;
        }
    }
    a
}

/// `A` in the NTT domain (FIPS 204 Algorithm 32): entry `[i][j]` is RejNTTPoly(rho || j || i).
fn expand_a(rho: &[u8], k: usize, l: usize) -> Vec<Vec<Poly>> {
    (0..k)
        .map(|i| {
            (0..l)
                .map(|j| {
                    let mut xof = Shake::new_128();
                    xof.update(rho);
                    xof.update(&[j as u8, i as u8]);
                    rej_ntt_poly(&mut xof)
                })
                .collect()
        })
        .collect()
}

/// CoeffFromHalfByte (FIPS 204 Algorithm 15): a value in `[-eta, eta]`, or `None` for a rejected nibble.
fn coeff_from_half_byte(b: u8, eta: i32) -> Option<i32> {
    let b = b as i32;
    match eta {
        2 if b < 15 => Some(2 - b % 5),
        4 if b < 9 => Some(4 - b),
        _ => None,
    }
}

/// RejBoundedPoly (FIPS 204 Algorithm 31): coefficients in `[-eta, eta]` from SHAKE256 of `seed || nonce`.
fn rej_bounded_poly(seed: &[u8], nonce: u16, eta: i32) -> Poly {
    let mut xof = Shake::new_256();
    xof.update(seed);
    xof.update(&nonce.to_le_bytes());
    let mut a = [0i32; N];
    let mut j = 0;
    let mut byte = [0u8; 1];
    while j < N {
        xof.squeeze(&mut byte);
        for nibble in [byte[0] & 0x0f, byte[0] >> 4] {
            if j < N {
                if let Some(c) = coeff_from_half_byte(nibble, eta) {
                    a[j] = c;
                    j += 1;
                }
            }
        }
    }
    a
}

/// ExpandS (FIPS 204 Algorithm 33): `s1` (l polynomials) and `s2` (k polynomials) from the seed `rho'`.
fn expand_s(p: Params, rho_p: &[u8]) -> (Vec<Poly>, Vec<Poly>) {
    let eta = p.eta();
    let s1 = (0..p.l()).map(|r| rej_bounded_poly(rho_p, r as u16, eta)).collect();
    let s2 = (0..p.k()).map(|r| rej_bounded_poly(rho_p, (p.l() + r) as u16, eta)).collect();
    (s1, s2)
}

/// ExpandMask (FIPS 204 Algorithm 34): the masking vector `y` for the attempt that starts at counter `kappa`.
fn expand_mask(p: Params, rho_pp: &[u8], kappa: u16) -> Vec<Poly> {
    (0..p.l())
        .map(|r| {
            let nonce = kappa.wrapping_add(r as u16).to_le_bytes();
            let mut v = [0u8; 640];
            shake256(&[rho_pp, &nonce[..]], &mut v);
            bit_unpack(&v, GAMMA1 - 1, GAMMA1)
        })
        .collect()
}

/// SampleInBall (FIPS 204 Algorithm 29): the challenge polynomial with tau coefficients in {-1, 1}.
fn sample_in_ball(c_tilde: &[u8], tau: usize) -> Poly {
    let mut xof = Shake::new_256();
    xof.update(c_tilde);
    let mut sign_bytes = [0u8; 8];
    xof.squeeze(&mut sign_bytes);
    let mut signs = u64::from_le_bytes(sign_bytes);
    let mut c = [0i32; N];
    for i in (N - tau)..N {
        let mut b = [0u8; 1];
        loop {
            xof.squeeze(&mut b);
            if b[0] as usize <= i {
                break;
            }
        }
        let j = b[0] as usize;
        c[i] = c[j];
        c[j] = if signs & 1 == 1 { -1 } else { 1 };
        signs >>= 1;
    }
    c
}

/// The challenge in the NTT domain, with its small coefficients reduced mod q.
fn challenge_hat(c_tilde: &[u8], tau: usize) -> Poly {
    let mut c = sample_in_ball(c_tilde, tau);
    ntt(&mut c);
    c
}

/// `t = A s1 + s2` split by Power2Round into `(t1, t0)`.
fn split_t(p: Params, rho: &[u8], s1: &[Poly], s2: &[Poly]) -> (Vec<Poly>, Vec<Poly>) {
    let a = expand_a(rho, p.k(), p.l());
    let as1 = inv_ntt_vec(&mul_matrix(&a, &ntt_vec(s1)));
    let mut t1 = Vec::with_capacity(p.k());
    let mut t0 = Vec::with_capacity(p.k());
    for i in 0..p.k() {
        let t: Poly = std::array::from_fn(|c| fadd(as1[i][c], s2[i][c].rem_euclid(Q)));
        let mut hi = [0i32; N];
        let mut lo = [0i32; N];
        for c in 0..N {
            let (a1, a0) = power2round(t[c]);
            hi[c] = a1;
            lo[c] = a0;
        }
        t1.push(hi);
        t0.push(lo);
    }
    (t1, t0)
}

fn encode_public_key(rho: &[u8], t1: &[Poly]) -> Vec<u8> {
    let mut pk = rho.to_vec();
    for poly in t1 {
        simple_bit_pack(poly, 1023, &mut pk);
    }
    pk
}

/// skDecode (FIPS 204 Algorithm 25): `(rho, K, tr, s1, s2, t0)`. The length must already be checked.
fn decode_secret_key(p: Params, sk: &[u8]) -> (&[u8], &[u8], &[u8], Vec<Poly>, Vec<Poly>, Vec<Poly>) {
    let (k, l, eta) = (p.k(), p.l(), p.eta());
    let (rho, rest) = sk.split_at(32);
    let (key, rest) = rest.split_at(32);
    let (tr, rest) = rest.split_at(64);
    let s_len = p.s_poly_len();
    let s1: Vec<Poly> = (0..l).map(|i| bit_unpack(&rest[i * s_len..(i + 1) * s_len], eta, eta)).collect();
    let rest = &rest[l * s_len..];
    let s2: Vec<Poly> = (0..k).map(|i| bit_unpack(&rest[i * s_len..(i + 1) * s_len], eta, eta)).collect();
    let rest = &rest[k * s_len..];
    let t0: Vec<Poly> = (0..k).map(|i| bit_unpack(&rest[i * 416..(i + 1) * 416], T0_A, T0_B)).collect();
    (rho, key, tr, s1, s2, t0)
}

fn check_public_key(p: Params, pk: &[u8]) -> Result<(), Error> {
    if pk.len() != p.public_key_len() {
        return Err(Error::PublicKeyLength);
    }
    Ok(())
}

fn check_secret_key(p: Params, sk: &[u8]) -> Result<(), Error> {
    if sk.len() != p.secret_key_len() {
        return Err(Error::SecretKeyLength);
    }
    Ok(())
}

/// ML-DSA.KeyGen_internal (FIPS 204 Algorithm 6) from the 32-byte seed `xi`.
pub fn keygen(p: Params, xi: &[u8; 32]) -> KeyPair {
    let (k, l) = (p.k(), p.l());
    let kb = [k as u8];
    let lb = [l as u8];
    let mut h = [0u8; 128];
    shake256(&[&xi[..], &kb[..], &lb[..]], &mut h);
    let (rho, rest) = h.split_at(32);
    let (rho_p, key) = rest.split_at(64);

    let (s1, s2) = expand_s(p, rho_p);
    let (t1, t0) = split_t(p, rho, &s1, &s2);
    let pk = encode_public_key(rho, &t1);

    let mut tr = [0u8; 64];
    shake256(&[&pk[..]], &mut tr);
    let mut sk = Vec::with_capacity(p.secret_key_len());
    sk.extend_from_slice(rho);
    sk.extend_from_slice(key);
    sk.extend_from_slice(&tr);
    let eta = p.eta();
    for poly in s1.iter().chain(s2.iter()) {
        bit_pack(poly, eta, eta, &mut sk);
    }
    for poly in &t0 {
        bit_pack(poly, T0_A, T0_B, &mut sk);
    }
    KeyPair { public_key: pk, secret_key: sk }
}

/// The public key that belongs to a secret key, recomputed from `rho`, `s1` and `s2` (the secret key holds no `t1`).
pub fn public_key_from_secret_key(p: Params, sk: &[u8]) -> Result<Vec<u8>, Error> {
    check_secret_key(p, sk)?;
    let (rho, _key, _tr, s1, s2, _t0) = decode_secret_key(p, sk);
    let (t1, _t0) = split_t(p, rho, &s1, &s2);
    Ok(encode_public_key(rho, &t1))
}

/// ML-DSA.Sign_internal (FIPS 204 Algorithm 7) over the already-built message `m_prime`.
fn sign_internal(p: Params, sk: &[u8], m_prime: &[u8], rnd: &[u8; 32]) -> Vec<u8> {
    let (k, l) = (p.k(), p.l());
    let (beta, omega, tau) = (p.beta(), p.omega(), p.tau());
    let (rho, key, tr, s1, s2, t0) = decode_secret_key(p, sk);
    let s1_hat = ntt_vec(&s1);
    let s2_hat = ntt_vec(&s2);
    let t0_hat = ntt_vec(&t0);
    let a = expand_a(rho, k, l);

    let mut mu = [0u8; 64];
    shake256(&[tr, m_prime], &mut mu);
    let mut rho_pp = [0u8; 64];
    shake256(&[key, &rnd[..], &mu[..]], &mut rho_pp);

    let mut kappa: u16 = 0;
    loop {
        let y = expand_mask(p, &rho_pp, kappa);
        kappa = kappa.wrapping_add(l as u16);
        let w = inv_ntt_vec(&mul_matrix(&a, &ntt_vec(&y)));

        let mut w1_bytes = Vec::with_capacity(k * 128);
        for wi in &w {
            let w1: Poly = std::array::from_fn(|c| high_bits(wi[c]));
            simple_bit_pack(&w1, 15, &mut w1_bytes);
        }
        let mut c_tilde = vec![0u8; p.c_tilde_len()];
        shake256(&[&mu[..], &w1_bytes[..]], &mut c_tilde);
        let c_hat = challenge_hat(&c_tilde, tau);

        let cs1 = inv_ntt_vec(&s1_hat.iter().map(|s| pointwise(&c_hat, s)).collect::<Vec<Poly>>());
        let cs2 = inv_ntt_vec(&s2_hat.iter().map(|s| pointwise(&c_hat, s)).collect::<Vec<Poly>>());
        let z: Vec<Poly> = (0..l).map(|i| std::array::from_fn(|c| centered(y[i][c] + cs1[i][c]))).collect();
        if max_abs(&z) >= GAMMA1 - beta {
            continue;
        }

        let r0_too_big = (0..k).any(|i| (0..N).any(|c| low_bits(fsub(w[i][c], cs2[i][c])).abs() >= GAMMA2 - beta));
        if r0_too_big {
            continue;
        }

        let ct0 = inv_ntt_vec(&t0_hat.iter().map(|t| pointwise(&c_hat, t)).collect::<Vec<Poly>>());
        if max_abs(&ct0) >= GAMMA2 {
            continue;
        }

        let mut h = vec![[0i32; N]; k];
        let mut ones = 0usize;
        for i in 0..k {
            for c in 0..N {
                let neg_ct0 = fsub(0, ct0[i][c]);
                let r = fadd(fsub(w[i][c], cs2[i][c]), ct0[i][c]);
                if make_hint(neg_ct0, r) {
                    h[i][c] = 1;
                    ones += 1;
                }
            }
        }
        if ones > omega {
            continue;
        }

        let mut sig = c_tilde;
        for poly in &z {
            bit_pack(poly, GAMMA1 - 1, GAMMA1, &mut sig);
        }
        hint_bit_pack(&h, omega, &mut sig);
        return sig;
    }
}

/// ML-DSA.Verify_internal (FIPS 204 Algorithm 8) over the already-built message `m_prime`. Malformed encodings are
/// `false`.
fn verify_internal(p: Params, pk: &[u8], m_prime: &[u8], sig: &[u8]) -> bool {
    if sig.len() != p.signature_len() {
        return false;
    }
    let (k, l) = (p.k(), p.l());
    let (beta, omega, tau) = (p.beta(), p.omega(), p.tau());
    let c_len = p.c_tilde_len();
    let rho = &pk[..32];
    let t1: Vec<Poly> = (0..k).map(|i| simple_bit_unpack(&pk[32 + 320 * i..32 + 320 * (i + 1)], 1023)).collect();

    let (c_tilde, rest) = sig.split_at(c_len);
    let (z_bytes, h_bytes) = rest.split_at(640 * l);
    let z: Vec<Poly> = (0..l).map(|i| bit_unpack(&z_bytes[640 * i..640 * (i + 1)], GAMMA1 - 1, GAMMA1)).collect();
    if max_abs(&z) >= GAMMA1 - beta {
        return false;
    }
    let h = match hint_bit_unpack(h_bytes, k, omega) {
        Some(h) => h,
        None => return false,
    };

    let mut tr = [0u8; 64];
    shake256(&[pk], &mut tr);
    let mut mu = [0u8; 64];
    shake256(&[&tr[..], m_prime], &mut mu);
    let c_hat = challenge_hat(c_tilde, tau);

    // w'_approx = A z - c * t1 * 2^d, in the NTT domain before the inverse transform
    let a = expand_a(rho, k, l);
    let az = mul_matrix(&a, &ntt_vec(&z));
    let t1_scaled: Vec<Poly> = t1.iter().map(|t| std::array::from_fn(|c| t[c] << D)).collect();
    let ct1: Vec<Poly> = ntt_vec(&t1_scaled).iter().map(|t| pointwise(&c_hat, t)).collect();
    let w_approx = inv_ntt_vec(&(0..k).map(|i| poly_sub(&az[i], &ct1[i])).collect::<Vec<Poly>>());

    let mut w1_bytes = Vec::with_capacity(k * 128);
    for i in 0..k {
        let w1: Poly = std::array::from_fn(|c| use_hint(h[i][c] == 1, w_approx[i][c]));
        simple_bit_pack(&w1, 15, &mut w1_bytes);
    }
    let mut expected = vec![0u8; c_len];
    shake256(&[&mu[..], &w1_bytes[..]], &mut expected);
    c_tilde == expected.as_slice()
}

/// The message representative of the pure (external) interface: `0 || |ctx| || ctx || M` (FIPS 204 Algorithm 2).
fn message_prime(message: &[u8], context: &[u8]) -> Result<Vec<u8>, Error> {
    if context.len() > 255 {
        return Err(Error::ContextTooLong);
    }
    let mut m_prime = Vec::with_capacity(2 + context.len() + message.len());
    m_prime.push(0);
    m_prime.push(context.len() as u8);
    m_prime.extend_from_slice(context);
    m_prime.extend_from_slice(message);
    Ok(m_prime)
}

/// ML-DSA.Sign (FIPS 204 Algorithm 2), pure interface. `rnd` is 32 random bytes for hedged signing, or all zeros for
/// the deterministic variant. A wrong secret key length or a context over 255 bytes is an error.
pub fn sign(p: Params, sk: &[u8], message: &[u8], context: &[u8], rnd: &[u8; 32]) -> Result<Vec<u8>, Error> {
    check_secret_key(p, sk)?;
    let m_prime = message_prime(message, context)?;
    Ok(sign_internal(p, sk, &m_prime, rnd))
}

/// ML-DSA.Verify (FIPS 204 Algorithm 3), pure interface. `Ok(false)` for a bad signature or a signature of the wrong
/// length; `Err` for a wrong public key length or a context over 255 bytes.
pub fn verify(p: Params, pk: &[u8], message: &[u8], context: &[u8], sig: &[u8]) -> Result<bool, Error> {
    check_public_key(p, pk)?;
    let m_prime = message_prime(message, context)?;
    Ok(verify_internal(p, pk, &m_prime, sig))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Params; 2] = [Params::MlDsa65, Params::MlDsa87];

    fn seed(tag: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        for (i, b) in s.iter_mut().enumerate() {
            *b = tag.wrapping_add((i as u8).wrapping_mul(13));
        }
        s
    }

    fn sample_poly(salt: i32) -> Poly {
        std::array::from_fn(|i| (i as i32 * 37 + salt) % Q)
    }

    /// Reference product in Z_q[X] / (X^256 + 1), computed the slow way.
    fn schoolbook(a: &Poly, b: &Poly) -> Poly {
        let mut c = [0i64; N];
        for i in 0..N {
            for j in 0..N {
                let prod = a[i] as i64 * b[j] as i64;
                if i + j < N {
                    c[i + j] += prod;
                } else {
                    c[i + j - N] -= prod;
                }
            }
        }
        std::array::from_fn(|i| c[i].rem_euclid(Q as i64) as i32)
    }

    #[test]
    fn sizes_match_fips_204() {
        assert_eq!(Params::MlDsa65.public_key_len(), 1952);
        assert_eq!(Params::MlDsa65.secret_key_len(), 4032);
        assert_eq!(Params::MlDsa65.signature_len(), 3309);
        assert_eq!(Params::MlDsa87.public_key_len(), 2592);
        assert_eq!(Params::MlDsa87.secret_key_len(), 4896);
        assert_eq!(Params::MlDsa87.signature_len(), 4627);
    }

    #[test]
    fn generated_keys_have_the_fips_204_sizes() {
        for p in BOTH {
            let kp = keygen(p, &seed(1));
            assert_eq!(kp.public_key.len(), p.public_key_len());
            assert_eq!(kp.secret_key.len(), p.secret_key_len());
            assert_eq!(public_key_from_secret_key(p, &kp.secret_key).unwrap(), kp.public_key);
        }
    }

    #[test]
    fn ntt_round_trip_is_identity() {
        let f = sample_poly(5);
        let mut g = f;
        ntt(&mut g);
        assert_ne!(g, f);
        inv_ntt(&mut g);
        assert_eq!(g, f);
    }

    #[test]
    fn ntt_multiplication_matches_negacyclic_product() {
        let a = sample_poly(3);
        let b = sample_poly(101);
        let (mut ah, mut bh) = (a, b);
        ntt(&mut ah);
        ntt(&mut bh);
        let mut c = pointwise(&ah, &bh);
        inv_ntt(&mut c);
        assert_eq!(c, schoolbook(&a, &b));
    }

    #[test]
    fn power2round_recombines() {
        for r in [0, 1, 4095, 4096, 4097, Q - 1, 123_456, 8_000_000] {
            let (r1, r0) = power2round(r);
            assert_eq!((r1 * (1 << D) + r0).rem_euclid(Q), r);
            assert!((0..1024).contains(&r1));
            assert!(r0 > -(1 << (D - 1)) && r0 <= 1 << (D - 1));
        }
    }

    #[test]
    fn decompose_recombines_and_stays_in_range() {
        for r in [0, 1, GAMMA2, GAMMA2 + 1, Q - 1, Q - 2, 4_000_000, 7_654_321] {
            let (r1, r0) = decompose(r);
            assert!((0..M).contains(&r1));
            assert_eq!((r1 * ALPHA + r0).rem_euclid(Q), r.rem_euclid(Q));
        }
        // the corner case of FIPS 204 Algorithm 36: r = q - 1 gives r1 = 0
        assert_eq!(decompose(Q - 1).0, 0);
    }

    #[test]
    fn hint_recovers_the_high_bits() {
        for r in [0, 1000, 5_000_000, Q - 1, 3_141_592] {
            for z in [-1000i32, -1, 0, 1, 777] {
                let rz = fadd(r, z.rem_euclid(Q));
                let hint = make_hint(z.rem_euclid(Q), r);
                assert_eq!(use_hint(hint, rz), high_bits(r), "r {r} z {z}");
            }
        }
    }

    #[test]
    fn sample_in_ball_has_tau_signed_ones() {
        for p in BOTH {
            let seed64 = [7u8; 64];
            let c = sample_in_ball(&seed64[..p.c_tilde_len()], p.tau());
            assert!(c.iter().all(|&x| (-1..=1).contains(&x)));
            assert_eq!(c.iter().filter(|&&x| x != 0).count(), p.tau());
        }
    }

    #[test]
    fn bit_packings_round_trip() {
        let mut t0 = sample_poly(9);
        for c in t0.iter_mut() {
            *c = centered(*c) % (1 << (D - 1));
        }
        let mut buf = Vec::new();
        bit_pack(&t0, T0_A, T0_B, &mut buf);
        assert_eq!(buf.len(), 416);
        assert_eq!(bit_unpack(&buf, T0_A, T0_B), t0);

        let z: Poly = std::array::from_fn(|i| (i as i32 * 1999) % (GAMMA1 - 1) - GAMMA1 / 2);
        let mut zb = Vec::new();
        bit_pack(&z, GAMMA1 - 1, GAMMA1, &mut zb);
        assert_eq!(zb.len(), 640);
        assert_eq!(bit_unpack(&zb, GAMMA1 - 1, GAMMA1), z);

        let w1: Poly = std::array::from_fn(|i| (i as i32) % 16);
        let mut wb = Vec::new();
        simple_bit_pack(&w1, 15, &mut wb);
        assert_eq!(wb.len(), 128);
        assert_eq!(simple_bit_unpack(&wb, 15), w1);
    }

    #[test]
    fn hint_packing_round_trips_and_refuses_bad_encodings() {
        let k = 6;
        let omega = 55;
        let mut h = vec![[0i32; N]; k];
        h[0][3] = 1;
        h[0][200] = 1;
        h[5][0] = 1;
        h[2][255] = 1;
        let mut buf = Vec::new();
        hint_bit_pack(&h, omega, &mut buf);
        assert_eq!(buf.len(), omega + k);
        assert_eq!(hint_bit_unpack(&buf, k, omega), Some(h));
        // an end index that goes backwards is refused
        let mut bad = buf.clone();
        bad[omega + 1] = 0;
        assert_eq!(hint_bit_unpack(&bad, k, omega), None);
        // nonzero padding after the last end index is refused
        let mut pad = buf.clone();
        pad[omega - 1] = 9;
        assert_eq!(hint_bit_unpack(&pad, k, omega), None);
    }

    #[test]
    fn sign_and_verify_round_trip_for_both_sets() {
        for p in BOTH {
            let kp = keygen(p, &seed(11));
            let msg = b"the quick brown fox";
            let ctx = b"bifrost";
            let sig = sign(p, &kp.secret_key, msg, ctx, &seed(12)).unwrap();
            assert_eq!(sig.len(), p.signature_len());
            assert!(verify(p, &kp.public_key, msg, ctx, &sig).unwrap());
        }
    }

    #[test]
    fn wrong_message_context_or_signature_fails() {
        for p in BOTH {
            let kp = keygen(p, &seed(21));
            let ctx = b"app";
            let sig = sign(p, &kp.secret_key, b"hello", ctx, &seed(22)).unwrap();
            assert!(!verify(p, &kp.public_key, b"hellO", ctx, &sig).unwrap());
            assert!(!verify(p, &kp.public_key, b"hello", b"apq", &sig).unwrap());
            let mut flipped = sig.clone();
            flipped[0] ^= 1;
            assert!(!verify(p, &kp.public_key, b"hello", ctx, &flipped).unwrap());
            let mut tail = sig.clone();
            let last = tail.len() - 1;
            tail[last] ^= 0x80;
            assert!(!verify(p, &kp.public_key, b"hello", ctx, &tail).unwrap());
            assert!(!verify(p, &kp.public_key, b"hello", ctx, &sig[1..]).unwrap());
        }
    }

    #[test]
    fn deterministic_signing_is_reproducible_and_hedged_differs() {
        for p in BOTH {
            let kp = keygen(p, &seed(31));
            let zero = [0u8; 32];
            let a = sign(p, &kp.secret_key, b"same", b"", &zero).unwrap();
            let b = sign(p, &kp.secret_key, b"same", b"", &zero).unwrap();
            assert_eq!(a, b);
            let hedged = sign(p, &kp.secret_key, b"same", b"", &seed(32)).unwrap();
            assert_ne!(hedged, a);
            assert!(verify(p, &kp.public_key, b"same", b"", &hedged).unwrap());
        }
    }

    #[test]
    fn length_and_context_limits_are_refused() {
        let p = Params::MlDsa65;
        let kp = keygen(p, &seed(41));
        assert_eq!(sign(p, &kp.secret_key, b"m", &[0u8; 256], &[0u8; 32]), Err(Error::ContextTooLong));
        assert!(sign(p, &kp.secret_key, b"m", &[0u8; 255], &[0u8; 32]).is_ok());
        assert_eq!(sign(p, &kp.secret_key[1..], b"m", b"", &[0u8; 32]), Err(Error::SecretKeyLength));
        assert_eq!(verify(p, &kp.public_key[1..], b"m", b"", &[]), Err(Error::PublicKeyLength));
        assert_eq!(verify(p, &kp.public_key, b"m", &[0u8; 256], &[]), Err(Error::ContextTooLong));
        assert_eq!(verify(p, &kp.public_key, b"m", b"", &[]), Ok(false));
    }
}
