//! SHA-3 (FIPS 202): the Keccak-f[1600] sponge with the SHA3-256, SHA3-384 and SHA3-512 parameters.
//! Incremental like [`crate::sha256`] and [`crate::sha512`]; the sponge state is 25 little-endian lanes.

const ROUND_CONSTANTS: [u64; 24] = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000, 0x000000000000808b, 0x0000000080000001,
    0x8000000080008081, 0x8000000000008009, 0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
    0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003, 0x8000000000008002, 0x8000000000000080,
    0x000000000000800a, 0x800000008000000a, 0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
];

/// Rho rotation offsets, indexed by lane `x + 5 * y`.
const ROTATIONS: [u32; 25] = [
    0, 1, 62, 28, 27, //
    36, 44, 6, 55, 20, //
    3, 10, 43, 25, 39, //
    41, 45, 15, 21, 8, //
    18, 2, 61, 56, 14,
];

fn keccak_f(a: &mut [u64; 25]) {
    for &rc in ROUND_CONSTANTS.iter() {
        // theta
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                a[x + 5 * y] ^= d;
            }
        }
        // rho and pi: lane (x, y) moves to (y, 2x + 3y) and is rotated by its offset
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                let i = x + 5 * y;
                let nx = y;
                let ny = (2 * x + 3 * y) % 5;
                b[nx + 5 * ny] = a[i].rotate_left(ROTATIONS[i]);
            }
        }
        // chi
        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] = b[x + 5 * y] ^ (!b[(x + 1) % 5 + 5 * y] & b[(x + 2) % 5 + 5 * y]);
            }
        }
        // iota
        a[0] ^= rc;
    }
}

/// Incremental SHA3-256, SHA3-384 or SHA3-512 hash. Create with [`Sha3::new_256`],
/// [`Sha3::new_384`] or [`Sha3::new_512`].
#[derive(Clone)]
pub struct Sha3 {
    state: [u64; 25],
    buf: [u8; 200],
    fill: usize,
    rate: usize,
    out: usize,
}

impl Sha3 {
    fn with(rate: usize, out: usize) -> Self {
        Sha3 { state: [0; 25], buf: [0; 200], fill: 0, rate, out }
    }

    /// SHA3-256: rate 136 bytes, 32-byte digest.
    pub fn new_256() -> Self {
        Self::with(136, 32)
    }

    /// SHA3-384: rate 104 bytes, 48-byte digest.
    pub fn new_384() -> Self {
        Self::with(104, 48)
    }

    /// SHA3-512: rate 72 bytes, 64-byte digest.
    pub fn new_512() -> Self {
        Self::with(72, 64)
    }

    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let n = (self.rate - self.fill).min(data.len());
            self.buf[self.fill..self.fill + n].copy_from_slice(&data[..n]);
            self.fill += n;
            data = &data[n..];
            if self.fill == self.rate {
                self.absorb();
            }
        }
    }

    /// XORs one full rate block into the state and permutes it.
    fn absorb(&mut self) {
        for i in 0..self.rate / 8 {
            let mut lane = [0u8; 8];
            lane.copy_from_slice(&self.buf[8 * i..8 * i + 8]);
            self.state[i] ^= u64::from_le_bytes(lane);
        }
        keccak_f(&mut self.state);
        self.fill = 0;
    }

    /// Pads with the SHA-3 domain suffix (0x06, then 0x80 in the last rate byte) and returns the digest.
    pub fn finish(mut self) -> Vec<u8> {
        for b in self.buf[self.fill..self.rate].iter_mut() {
            *b = 0;
        }
        self.buf[self.fill] = 0x06;
        self.buf[self.rate - 1] |= 0x80;
        self.absorb();
        // every digest here is shorter than the rate, so one squeeze of the first lanes suffices
        let bytes: Vec<u8> = self.state.iter().flat_map(|l| l.to_le_bytes()).collect();
        bytes[..self.out].to_vec()
    }
}

/// Absorbs one full rate block (`block.len()` bytes, a multiple of 8) into the sponge and permutes it.
fn absorb_block(state: &mut [u64; 25], block: &[u8]) {
    for (i, lane) in block.chunks_exact(8).enumerate() {
        let mut l = [0u8; 8];
        l.copy_from_slice(lane);
        state[i] ^= u64::from_le_bytes(l);
    }
    keccak_f(state);
}

/// SHAKE128 or SHAKE256 (FIPS 202): the extendable-output functions on the same sponge, with the 0x1F domain suffix.
/// Absorb with [`Shake::update`], then read any number of bytes with [`Shake::squeeze`]. Input after the first
/// squeeze is not allowed.
#[derive(Clone)]
pub struct Shake {
    state: [u64; 25],
    buf: [u8; 200],
    fill: usize,
    rate: usize,
    squeezing: bool,
    pos: usize,
}

impl Shake {
    fn with(rate: usize) -> Self {
        Shake { state: [0; 25], buf: [0; 200], fill: 0, rate, squeezing: false, pos: 0 }
    }

    /// SHAKE128: rate 168 bytes.
    pub fn new_128() -> Self {
        Self::with(168)
    }

    /// SHAKE256: rate 136 bytes.
    pub fn new_256() -> Self {
        Self::with(136)
    }

    pub fn update(&mut self, mut data: &[u8]) {
        debug_assert!(!self.squeezing, "SHAKE input after output");
        while !data.is_empty() {
            let n = (self.rate - self.fill).min(data.len());
            self.buf[self.fill..self.fill + n].copy_from_slice(&data[..n]);
            self.fill += n;
            data = &data[n..];
            if self.fill == self.rate {
                absorb_block(&mut self.state, &self.buf[..self.rate]);
                self.fill = 0;
            }
        }
    }

    /// Pads the partial block with the SHAKE suffix (0x1F, then 0x80 in the last rate byte) and absorbs it.
    fn pad(&mut self) {
        for b in self.buf[self.fill..self.rate].iter_mut() {
            *b = 0;
        }
        self.buf[self.fill] = 0x1f;
        self.buf[self.rate - 1] |= 0x80;
        absorb_block(&mut self.state, &self.buf[..self.rate]);
        self.squeezing = true;
        self.pos = 0;
    }

    /// Fills `out` with the next bytes of the output stream. Calls can be split at any point: the result is the same
    /// as one call for the whole length.
    pub fn squeeze(&mut self, out: &mut [u8]) {
        if !self.squeezing {
            self.pad();
        }
        for b in out.iter_mut() {
            if self.pos == self.rate {
                keccak_f(&mut self.state);
                self.pos = 0;
            }
            *b = (self.state[self.pos / 8] >> (8 * (self.pos % 8))) as u8;
            self.pos += 1;
        }
    }
}

/// SHA3-256 of `data` in one call.
pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha3::new_256();
    s.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&s.finish());
    out
}

/// SHA3-384 of `data` in one call.
pub fn sha3_384(data: &[u8]) -> [u8; 48] {
    let mut s = Sha3::new_384();
    s.update(data);
    let mut out = [0u8; 48];
    out.copy_from_slice(&s.finish());
    out
}

/// SHA3-512 of `data` in one call.
pub fn sha3_512(data: &[u8]) -> [u8; 64] {
    let mut s = Sha3::new_512();
    s.update(data);
    let mut out = [0u8; 64];
    out.copy_from_slice(&s.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::hex;

    #[test]
    fn sha3_256_known_vectors() {
        assert_eq!(hex(&sha3_256(b"")), "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a");
        assert_eq!(hex(&sha3_256(b"abc")), "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532");
    }

    #[test]
    fn sha3_512_empty_vector() {
        assert_eq!(
            hex(&sha3_512(b"")),
            "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a615b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26"
        );
    }

    #[test]
    fn output_lengths() {
        assert_eq!(Sha3::new_256().finish().len(), 32);
        assert_eq!(Sha3::new_384().finish().len(), 48);
        assert_eq!(Sha3::new_512().finish().len(), 64);
    }

    #[test]
    fn chunked_updates_match_one_shot() {
        // 1000 bytes crosses the 136-byte rate several times; the chunk size does not divide the rate
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut s = Sha3::new_256();
        for c in data.chunks(37) {
            s.update(c);
        }
        assert_eq!(s.finish(), sha3_256(&data).to_vec());
        let mut t = Sha3::new_512();
        for c in data.chunks(71) {
            t.update(c);
        }
        assert_eq!(t.finish(), sha3_512(&data).to_vec());
    }

    #[test]
    fn padding_at_the_rate_boundary() {
        // 135 bytes leaves one byte of padding room: domain byte and the final 0x80 share it
        let data = vec![0xa5u8; 135];
        let mut s = Sha3::new_256();
        s.update(&data[..100]);
        s.update(&data[100..]);
        assert_eq!(s.finish(), sha3_256(&data).to_vec());
    }

    #[test]
    fn shake_split_squeeze_matches_one_call() {
        // 400 bytes spans more than two SHAKE128 rates (168) and one SHAKE256 rate (136)
        for make in [Shake::new_128, Shake::new_256] {
            let mut whole = make();
            whole.update(b"ML-KEM seed material");
            let mut all = vec![0u8; 400];
            whole.squeeze(&mut all);

            let mut parts = make();
            parts.update(b"ML-KEM ");
            parts.update(b"seed material");
            let mut got = Vec::new();
            for n in [1usize, 7, 168, 200, 24] {
                let mut chunk = vec![0u8; n];
                parts.squeeze(&mut chunk);
                got.extend_from_slice(&chunk);
            }
            assert_eq!(got, all[..got.len()].to_vec());
        }
    }

    #[test]
    fn shake_of_empty_input_matches_the_fips_202_vectors() {
        let mut out = [0u8; 32];
        Shake::new_128().squeeze(&mut out);
        assert_eq!(hex(&out), "7f9c2ba4e88f827d616045507605853ed73b8093f6efbc88eb1a6eacfa66ef26");
        Shake::new_256().squeeze(&mut out);
        assert_eq!(hex(&out), "46b9dd2b0ba88d13233b3feb743eeb243fcd52ea62b81b82b50c27646ed5762f");
    }

    #[test]
    fn shake128_and_shake256_differ() {
        let mut a = Shake::new_128();
        let mut b = Shake::new_256();
        let (mut x, mut y) = ([0u8; 32], [0u8; 32]);
        a.squeeze(&mut x);
        b.squeeze(&mut y);
        assert_ne!(x, y);
    }

    #[test]
    fn million_a_sha3_256() {
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha3_256(&million)), "5c8875ae474a3634ba4fd55ec85bffd661f32aca75c6d699d0cdcb6c115891c1");
    }
}
