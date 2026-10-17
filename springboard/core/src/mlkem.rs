//! ML-KEM (FIPS 203): the module-lattice key encapsulation mechanism, parameter sets ML-KEM-768 and ML-KEM-1024.
//!
//! Integer arithmetic over Z_q with q = 3329: the NTT and its inverse (FIPS 203 Algorithms 9 and 10), CBD and
//! SampleNTT sampling, Compress and Decompress, and the K-PKE core. The hashes come from [`crate::sha3`]: G is
//! SHA3-512, H is SHA3-256, PRF and J are SHAKE256, and the matrix XOF is SHAKE128.
//!
//! The functions take their random seeds from the caller (`d` and `z` for key generation, `m` for encapsulation),
//! so this module draws no randomness; the SDK fills the seeds from the kernel pool.
//!
//! Status: the arithmetic is checked by round trips and a schoolbook multiplication, not by the official FIPS 203
//! known-answer vectors, which are not in the repo yet. Interoperability with other ML-KEM implementations is
//! unverified until those vectors pass.

use crate::sha3::{sha3_256, sha3_512, Shake};

/// The prime modulus q = 3329.
pub const Q: u16 = 3329;
/// Coefficients per polynomial (the ring is Z_q[X] / (X^256 + 1)).
const N: usize = 256;
/// Bound eta of the secret and error distributions. Both parameter sets use eta = 2; the CBD code below assumes it.
const ETA: usize = 2;

/// A polynomial with coefficients kept in `[0, q)`.
type Poly = [u16; N];

/// The two ML-KEM parameter sets, the ones CryptoKit exposes as `MLKEM768` and `MLKEM1024`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Params {
    MlKem768,
    MlKem1024,
}

impl Params {
    /// Rank k of the module: the number of polynomials in a secret or public vector.
    pub fn k(self) -> usize {
        match self {
            Params::MlKem768 => 3,
            Params::MlKem1024 => 4,
        }
    }

    /// Bits kept per coefficient of `u` in a ciphertext.
    fn du(self) -> usize {
        match self {
            Params::MlKem768 => 10,
            Params::MlKem1024 => 11,
        }
    }

    /// Bits kept per coefficient of `v` in a ciphertext.
    fn dv(self) -> usize {
        match self {
            Params::MlKem768 => 4,
            Params::MlKem1024 => 5,
        }
    }

    /// Encapsulation key: `t` at 12 bits per coefficient, then the 32-byte matrix seed `rho`.
    pub fn encapsulation_key_len(self) -> usize {
        384 * self.k() + 32
    }

    /// Decapsulation key: the K-PKE secret, the encapsulation key, `H(ek)` and the implicit-rejection seed `z`.
    pub fn decapsulation_key_len(self) -> usize {
        768 * self.k() + 96
    }

    /// Ciphertext: `u` at `du` bits per coefficient, then `v` at `dv` bits.
    pub fn ciphertext_len(self) -> usize {
        32 * (self.du() * self.k() + self.dv())
    }
}

/// Why a key or a ciphertext was refused. Length checks come first; the two key checks follow FIPS 203.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    EncapsulationKeyLength,
    DecapsulationKeyLength,
    CiphertextLength,
    /// A 12-bit coefficient of the encapsulation key is not below q (the modulus check, FIPS 203 section 7.2).
    EncapsulationKeyModulus,
    /// The `H(ek)` stored in the decapsulation key does not match the key it carries (FIPS 203 section 7.3).
    DecapsulationKeyHash,
}

/// An ML-KEM key pair in the byte layout FIPS 203 gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPair {
    pub encapsulation_key: Vec<u8>,
    pub decapsulation_key: Vec<u8>,
}

fn fadd(a: u16, b: u16) -> u16 {
    ((a as u32 + b as u32) % Q as u32) as u16
}

/// `a - b` in Z_q; `b` must already be reduced.
fn fsub(a: u16, b: u16) -> u16 {
    ((a as u32 + Q as u32 - b as u32) % Q as u32) as u16
}

fn fmul(a: u16, b: u16) -> u16 {
    ((a as u32 * b as u32) % Q as u32) as u16
}

fn poly_add(a: &Poly, b: &Poly) -> Poly {
    let mut r = [0u16; N];
    for i in 0..N {
        r[i] = fadd(a[i], b[i]);
    }
    r
}

fn map_poly(f: &Poly, op: impl Fn(u16) -> u16) -> Poly {
    let mut r = [0u16; N];
    for i in 0..N {
        r[i] = op(f[i]);
    }
    r
}

const fn pow_mod(base: u32, mut exp: u32) -> u32 {
    let mut b = base % Q as u32;
    let mut acc = 1u32;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = acc * b % Q as u32;
        }
        b = b * b % Q as u32;
        exp >>= 1;
    }
    acc
}

const fn bit_reverse7(x: usize) -> usize {
    let mut r = 0;
    let mut i = 0;
    while i < 7 {
        r = (r << 1) | ((x >> i) & 1);
        i += 1;
    }
    r
}

/// `ZETAS[k] = 17^BitRev7(k) mod q`, the FIPS 203 table of powers of the 256th root of unity 17.
const ZETAS: [u16; 128] = {
    let mut t = [0u16; 128];
    let mut k = 0;
    while k < 128 {
        t[k] = pow_mod(17, bit_reverse7(k) as u32) as u16;
        k += 1;
    }
    t
};

/// `GAMMAS[i] = 17^(2 BitRev7(i) + 1) mod q`, the twist used by BaseCaseMultiply (FIPS 203 Algorithm 12).
const GAMMAS: [u16; 128] = {
    let mut t = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        t[i] = pow_mod(17, 2 * bit_reverse7(i) as u32 + 1) as u16;
        i += 1;
    }
    t
};

/// NTT (FIPS 203 Algorithm 9), in place.
fn ntt(f: &mut Poly) {
    let mut k = 1;
    let mut len = 128;
    while len >= 2 {
        for start in (0..N).step_by(2 * len) {
            let zeta = ZETAS[k];
            k += 1;
            for j in start..start + len {
                let t = fmul(zeta, f[j + len]);
                let a = f[j];
                f[j + len] = fsub(a, t);
                f[j] = fadd(a, t);
            }
        }
        len >>= 1;
    }
}

/// Inverse NTT (FIPS 203 Algorithm 10), in place, including the final multiplication by 128^-1 = 3303.
fn inv_ntt(f: &mut Poly) {
    let mut k = 127;
    let mut len = 2;
    while len <= 128 {
        for start in (0..N).step_by(2 * len) {
            let zeta = ZETAS[k];
            k -= 1;
            for j in start..start + len {
                let t = f[j];
                f[j] = fadd(t, f[j + len]);
                f[j + len] = fmul(zeta, fsub(f[j + len], t));
            }
        }
        len <<= 1;
    }
    for c in f.iter_mut() {
        *c = fmul(*c, 3303);
    }
}

/// MultiplyNTTs (FIPS 203 Algorithm 12): the product of two polynomials in the NTT domain, pairwise by BaseCaseMultiply.
fn ntt_mul(a: &Poly, b: &Poly) -> Poly {
    let mut c = [0u16; N];
    for i in 0..N / 2 {
        let (a0, a1, b0, b1) = (a[2 * i], a[2 * i + 1], b[2 * i], b[2 * i + 1]);
        c[2 * i] = fadd(fmul(a0, b0), fmul(fmul(a1, b1), GAMMAS[i]));
        c[2 * i + 1] = fadd(fmul(a0, b1), fmul(a1, b0));
    }
    c
}

/// Sum over `i` of `a[i] * b[i]`, all in the NTT domain.
fn inner_product(a: &[Poly], b: &[Poly]) -> Poly {
    let mut acc = [0u16; N];
    for (x, y) in a.iter().zip(b.iter()) {
        acc = poly_add(&acc, &ntt_mul(x, y));
    }
    acc
}

/// PRF_eta (FIPS 203 section 4.1) for eta = 2: SHAKE256 of the seed and a one-byte counter.
fn prf(seed: &[u8], counter: u8, out: &mut [u8]) {
    let mut h = Shake::new_256();
    h.update(seed);
    h.update(&[counter]);
    h.squeeze(out);
}

/// SamplePolyCBD_eta (FIPS 203 Algorithm 8) for eta = 2: each coefficient is a sum of two bits minus a sum of two
/// other bits, taken from one nibble of the input.
fn cbd(bytes: &[u8; 64 * ETA]) -> Poly {
    let mut f = [0u16; N];
    for i in 0..N {
        let nibble = (bytes[i / 2] >> (4 * (i % 2))) & 0x0f;
        let x = (nibble & 1) + ((nibble >> 1) & 1);
        let y = ((nibble >> 2) & 1) + ((nibble >> 3) & 1);
        f[i] = (x as u16 + Q - y as u16) % Q;
    }
    f
}

/// `count` CBD polynomials from consecutive PRF counters starting at `*counter`.
fn cbd_vec(seed: &[u8], counter: &mut u8, count: usize) -> Vec<Poly> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut buf = [0u8; 64 * ETA];
        prf(seed, *counter, &mut buf);
        *counter += 1;
        out.push(cbd(&buf));
    }
    out
}

/// SampleNTT (FIPS 203 Algorithm 7): rejection sampling of uniform coefficients from a SHAKE128 stream.
fn sample_ntt(mut xof: Shake) -> Poly {
    let mut a = [0u16; N];
    let mut j = 0;
    while j < N {
        let mut c = [0u8; 3];
        xof.squeeze(&mut c);
        let d1 = c[0] as u16 | ((c[1] as u16 & 0x0f) << 8);
        let d2 = (c[1] as u16 >> 4) | ((c[2] as u16) << 4);
        if d1 < Q {
            a[j] = d1;
            j += 1;
        }
        if d2 < Q && j < N {
            a[j] = d2;
            j += 1;
        }
    }
    a
}

/// Entry `A[row][col]` of the public matrix, already in the NTT domain: SampleNTT(rho || col || row).
fn matrix_entry(rho: &[u8], row: usize, col: usize) -> Poly {
    let mut xof = Shake::new_128();
    xof.update(rho);
    xof.update(&[col as u8, row as u8]);
    sample_ntt(xof)
}

/// `A * v`, or `Aᵀ * v` when `transposed`, with `v` and the result in the NTT domain.
fn mat_vec(rho: &[u8], v: &[Poly], transposed: bool) -> Vec<Poly> {
    let k = v.len();
    let mut out = vec![[0u16; N]; k];
    for i in 0..k {
        for j in 0..k {
            let a = if transposed { matrix_entry(rho, j, i) } else { matrix_entry(rho, i, j) };
            out[i] = poly_add(&out[i], &ntt_mul(&a, &v[j]));
        }
    }
    out
}

/// ByteDecode without the reduction: each `d`-bit field is read as is, least significant bit first.
fn decode_raw(bytes: &[u8], d: usize) -> Poly {
    let mut f = [0u16; N];
    for (i, c) in f.iter_mut().enumerate() {
        let mut v = 0u16;
        for j in 0..d {
            let pos = i * d + j;
            v |= (((bytes[pos / 8] >> (pos % 8)) & 1) as u16) << j;
        }
        *c = v;
    }
    f
}

/// ByteDecode_d (FIPS 203 Algorithm 5); at d = 12 the values are reduced mod q.
fn byte_decode(bytes: &[u8], d: usize) -> Poly {
    let mut f = decode_raw(bytes, d);
    if d == 12 {
        for c in f.iter_mut() {
            *c %= Q;
        }
    }
    f
}

/// ByteEncode_d (FIPS 203 Algorithm 4): 256 `d`-bit fields packed least significant bit first, 32 * d bytes.
fn byte_encode(f: &Poly, d: usize) -> Vec<u8> {
    let mut out = vec![0u8; 32 * d];
    for (i, &c) in f.iter().enumerate() {
        for j in 0..d {
            let pos = i * d + j;
            out[pos / 8] |= (((c >> j) & 1) as u8) << (pos % 8);
        }
    }
    out
}

/// Compress_d (FIPS 203 section 4.2.1): `round(2^d x / q) mod 2^d`, rounding half up.
fn compress(x: u16, d: usize) -> u16 {
    let scaled = ((x as u32) << d) + Q as u32 / 2;
    ((scaled / Q as u32) & ((1u32 << d) - 1)) as u16
}

/// Decompress_d: `round(q y / 2^d)`, rounding half up.
fn decompress(y: u16, d: usize) -> u16 {
    ((y as u32 * Q as u32 + (1u32 << (d - 1))) >> d) as u16
}

/// FIPS 203 section 7.2 modulus check: every 12-bit coefficient of the encapsulation key must be below q.
pub fn check_encapsulation_key(p: Params, ek: &[u8]) -> Result<(), Error> {
    if ek.len() != p.encapsulation_key_len() {
        return Err(Error::EncapsulationKeyLength);
    }
    for i in 0..p.k() {
        if decode_raw(&ek[384 * i..384 * (i + 1)], 12).iter().any(|&c| c >= Q) {
            return Err(Error::EncapsulationKeyModulus);
        }
    }
    Ok(())
}

/// K-PKE.KeyGen (FIPS 203 Algorithm 13): returns `(ek_PKE, dk_PKE)` from the seed `d`.
fn k_pke_keygen(p: Params, d: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let k = p.k();
    let mut seed = d.to_vec();
    seed.push(k as u8);
    let g = sha3_512(&seed);
    let (rho, sigma) = g.split_at(32);

    let mut counter = 0u8;
    let mut s = cbd_vec(sigma, &mut counter, k);
    let mut e = cbd_vec(sigma, &mut counter, k);
    for poly in s.iter_mut().chain(e.iter_mut()) {
        ntt(poly);
    }

    let a_s = mat_vec(rho, &s, false);
    let mut ek = Vec::with_capacity(p.encapsulation_key_len());
    for (a_s_i, e_i) in a_s.iter().zip(e.iter()) {
        ek.extend(byte_encode(&poly_add(a_s_i, e_i), 12));
    }
    ek.extend_from_slice(rho);

    let mut dk = Vec::with_capacity(384 * k);
    for poly in &s {
        dk.extend(byte_encode(poly, 12));
    }
    (ek, dk)
}

/// K-PKE.Encrypt (FIPS 203 Algorithm 14): encrypts the 32-byte message `m` with coins `r`.
fn k_pke_encrypt(p: Params, ek: &[u8], m: &[u8; 32], r: &[u8]) -> Vec<u8> {
    let k = p.k();
    let (du, dv) = (p.du(), p.dv());
    let t_hat: Vec<Poly> = (0..k).map(|i| byte_decode(&ek[384 * i..384 * (i + 1)], 12)).collect();
    let rho = &ek[384 * k..];

    let mut counter = 0u8;
    let mut y = cbd_vec(r, &mut counter, k);
    let e1 = cbd_vec(r, &mut counter, k);
    let mut e2_buf = [0u8; 64 * ETA];
    prf(r, counter, &mut e2_buf);
    let e2 = cbd(&e2_buf);
    for poly in y.iter_mut() {
        ntt(poly);
    }

    // u = NTT^-1(Aᵀ y) + e1
    let mut u = mat_vec(rho, &y, true);
    for (u_i, e1_i) in u.iter_mut().zip(e1.iter()) {
        inv_ntt(u_i);
        *u_i = poly_add(u_i, e1_i);
    }

    // v = NTT^-1(tᵀ y) + e2 + Decompress_q(m)
    let mut v = inner_product(&t_hat, &y);
    inv_ntt(&mut v);
    let mu = map_poly(&byte_decode(m, 1), |x| decompress(x, 1));
    for i in 0..N {
        v[i] = fadd(fadd(v[i], e2[i]), mu[i]);
    }

    let mut c = Vec::with_capacity(32 * (du * k + dv));
    for u_i in &u {
        c.extend(byte_encode(&map_poly(u_i, |x| compress(x, du)), du));
    }
    c.extend(byte_encode(&map_poly(&v, |x| compress(x, dv)), dv));
    c
}

/// K-PKE.Decrypt (FIPS 203 Algorithm 15): recovers the 32-byte message from `c`.
fn k_pke_decrypt(p: Params, dk_pke: &[u8], c: &[u8]) -> [u8; 32] {
    let k = p.k();
    let (du, dv) = (p.du(), p.dv());
    let mut u_hat: Vec<Poly> = (0..k)
        .map(|i| {
            let chunk = &c[32 * du * i..32 * du * (i + 1)];
            map_poly(&byte_decode(chunk, du), |x| decompress(x, du))
        })
        .collect();
    let v = map_poly(&byte_decode(&c[32 * du * k..], dv), |x| decompress(x, dv));
    let s_hat: Vec<Poly> = (0..k).map(|i| byte_decode(&dk_pke[384 * i..384 * (i + 1)], 12)).collect();

    for poly in u_hat.iter_mut() {
        ntt(poly);
    }
    let mut inner = inner_product(&s_hat, &u_hat);
    inv_ntt(&mut inner);
    let mut w = [0u16; N];
    for i in 0..N {
        w[i] = fsub(v[i], inner[i]);
    }
    let bits = byte_encode(&map_poly(&w, |x| compress(x, 1)), 1);
    let mut m = [0u8; 32];
    m.copy_from_slice(&bits);
    m
}

/// Constant-time equality of two equal-length byte strings: the loop always runs to the end.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0 && a.len() == b.len()
}

/// ML-KEM.KeyGen_internal (FIPS 203 Algorithm 16) from the 32-byte seeds `d` and `z`.
pub fn keygen(p: Params, d: &[u8; 32], z: &[u8; 32]) -> KeyPair {
    let (ek, dk_pke) = k_pke_keygen(p, d);
    let h = sha3_256(&ek);
    let mut dk = dk_pke;
    dk.extend_from_slice(&ek);
    dk.extend_from_slice(&h);
    dk.extend_from_slice(z);
    KeyPair { encapsulation_key: ek, decapsulation_key: dk }
}

/// ML-KEM.Encaps_internal (FIPS 203 Algorithm 17) with the 32-byte random message `m`. Returns the shared secret
/// and the ciphertext.
pub fn encapsulate(p: Params, ek: &[u8], m: &[u8; 32]) -> Result<([u8; 32], Vec<u8>), Error> {
    check_encapsulation_key(p, ek)?;
    let h = sha3_256(ek);
    let g = sha3_512(&[&m[..], &h[..]].concat());
    let (shared, r) = g.split_at(32);
    let c = k_pke_encrypt(p, ek, m, r);
    let mut k = [0u8; 32];
    k.copy_from_slice(shared);
    Ok((k, c))
}

/// ML-KEM.Decaps_internal (FIPS 203 Algorithm 18). A ciphertext that does not re-encrypt to itself gets the
/// implicit-rejection secret `J(z || c)`, so a bad ciphertext is not an error and reveals nothing about the key.
pub fn decapsulate(p: Params, dk: &[u8], c: &[u8]) -> Result<[u8; 32], Error> {
    if dk.len() != p.decapsulation_key_len() {
        return Err(Error::DecapsulationKeyLength);
    }
    if c.len() != p.ciphertext_len() {
        return Err(Error::CiphertextLength);
    }
    let pke_len = 384 * p.k();
    let ek_len = p.encapsulation_key_len();
    let ek = &dk[pke_len..pke_len + ek_len];
    let h_start = pke_len + ek_len;
    let h = &dk[h_start..h_start + 32];
    let z = &dk[h_start + 32..];
    if sha3_256(ek).as_slice() != h {
        return Err(Error::DecapsulationKeyHash);
    }

    let m = k_pke_decrypt(p, &dk[..pke_len], c);
    let g = sha3_512(&[&m[..], h].concat());
    let (k_prime, r) = g.split_at(32);
    let c_check = k_pke_encrypt(p, ek, &m, r);

    let mut j_in = z.to_vec();
    j_in.extend_from_slice(c);
    let mut k_bar = [0u8; 32];
    let mut j = Shake::new_256();
    j.update(&j_in);
    j.squeeze(&mut k_bar);

    let accept = ct_eq(c, &c_check);
    let mask = 0u8.wrapping_sub(accept as u8);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = (k_prime[i] & mask) | (k_bar[i] & !mask);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(tag: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        for (i, b) in s.iter_mut().enumerate() {
            *b = tag.wrapping_add((i as u8).wrapping_mul(7));
        }
        s
    }

    const BOTH: [Params; 2] = [Params::MlKem768, Params::MlKem1024];

    fn sample_poly(salt: u32) -> Poly {
        let mut f = [0u16; N];
        for (i, c) in f.iter_mut().enumerate() {
            *c = ((i as u32 * 37 + salt) % Q as u32) as u16;
        }
        f
    }

    /// Reference product in Z_q[X] / (X^256 + 1), computed the slow way.
    fn schoolbook(a: &Poly, b: &Poly) -> Poly {
        let q = Q as u32;
        let mut c = [0u32; N];
        for i in 0..N {
            for j in 0..N {
                let prod = a[i] as u32 * b[j] as u32 % q;
                let k = i + j;
                if k < N {
                    c[k] = (c[k] + prod) % q;
                } else {
                    c[k - N] = (c[k - N] + q - prod) % q;
                }
            }
        }
        let mut out = [0u16; N];
        for i in 0..N {
            out[i] = c[i] as u16;
        }
        out
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
        let mut c = ntt_mul(&ah, &bh);
        inv_ntt(&mut c);
        assert_eq!(c, schoolbook(&a, &b));
    }

    #[test]
    fn byte_codec_round_trips() {
        for d in [1usize, 4, 5, 10, 11, 12] {
            let mut f = sample_poly(9);
            for c in f.iter_mut() {
                *c &= ((1u32 << d) - 1) as u16;
            }
            let bytes = byte_encode(&f, d);
            assert_eq!(bytes.len(), 32 * d);
            assert_eq!(byte_decode(&bytes, d), f);
        }
    }

    #[test]
    fn compress_inverts_decompress() {
        for d in [4usize, 5, 10, 11] {
            for y in 0..(1u16 << d) {
                assert_eq!(compress(decompress(y, d), d), y);
            }
        }
    }

    #[test]
    fn key_and_ciphertext_lengths_match_fips_203() {
        assert_eq!(Params::MlKem768.encapsulation_key_len(), 1184);
        assert_eq!(Params::MlKem768.decapsulation_key_len(), 2400);
        assert_eq!(Params::MlKem768.ciphertext_len(), 1088);
        assert_eq!(Params::MlKem1024.encapsulation_key_len(), 1568);
        assert_eq!(Params::MlKem1024.decapsulation_key_len(), 3168);
        assert_eq!(Params::MlKem1024.ciphertext_len(), 1568);
        for p in BOTH {
            let kp = keygen(p, &seed(1), &seed(2));
            assert_eq!(kp.encapsulation_key.len(), p.encapsulation_key_len());
            assert_eq!(kp.decapsulation_key.len(), p.decapsulation_key_len());
            let (_, c) = encapsulate(p, &kp.encapsulation_key, &seed(3)).unwrap();
            assert_eq!(c.len(), p.ciphertext_len());
        }
    }

    #[test]
    fn encapsulation_round_trip_for_both_sizes() {
        for p in BOTH {
            let kp = keygen(p, &seed(10), &seed(20));
            assert_eq!(keygen(p, &seed(10), &seed(20)), kp, "key generation is deterministic");
            let (k_enc, c) = encapsulate(p, &kp.encapsulation_key, &seed(30)).unwrap();
            let k_dec = decapsulate(p, &kp.decapsulation_key, &c).unwrap();
            assert_eq!(k_enc, k_dec);
        }
    }

    #[test]
    fn tampered_ciphertext_gets_implicit_rejection_secret() {
        for p in BOTH {
            let kp = keygen(p, &seed(40), &seed(50));
            let (k, mut c) = encapsulate(p, &kp.encapsulation_key, &seed(60)).unwrap();
            c[0] ^= 1;
            let rejected = decapsulate(p, &kp.decapsulation_key, &c).unwrap();
            assert_ne!(rejected, k);
            assert_eq!(rejected, decapsulate(p, &kp.decapsulation_key, &c).unwrap());
        }
    }

    #[test]
    fn encapsulation_key_modulus_is_checked() {
        let p = Params::MlKem768;
        let mut kp = keygen(p, &seed(1), &seed(2));
        // the first 12-bit coefficient becomes 0xfff = 4095, which is not below q
        kp.encapsulation_key[0] = 0xff;
        kp.encapsulation_key[1] |= 0x0f;
        assert_eq!(encapsulate(p, &kp.encapsulation_key, &seed(3)), Err(Error::EncapsulationKeyModulus));
    }

    #[test]
    fn wrong_lengths_and_tampered_hash_are_refused() {
        for p in BOTH {
            let kp = keygen(p, &seed(7), &seed(8));
            let (_, c) = encapsulate(p, &kp.encapsulation_key, &seed(9)).unwrap();
            assert_eq!(encapsulate(p, &kp.encapsulation_key[1..], &seed(9)), Err(Error::EncapsulationKeyLength));
            assert_eq!(decapsulate(p, &kp.decapsulation_key[1..], &c), Err(Error::DecapsulationKeyLength));
            assert_eq!(decapsulate(p, &kp.decapsulation_key, &c[1..]), Err(Error::CiphertextLength));
            let mut dk = kp.decapsulation_key.clone();
            let h_at = dk.len() - 64;
            dk[h_at] ^= 1;
            assert_eq!(decapsulate(p, &dk, &c), Err(Error::DecapsulationKeyHash));
        }
    }
}
