//! Spectral band replication for HE-AAC (ISO/IEC 14496-3 clause 4.6.18), pure
//! Rust. symphonia decodes the AAC-LC core of an HE-AAC stream at half the
//! output rate and ignores the SBR payload; this module takes that core PCM
//! plus the raw access unit and rebuilds the missing upper half of the
//! spectrum, doubling the sample rate.
//!
//! Pipeline per frame (1024 core samples in, 2048 samples out):
//!
//! 1. [`fill_candidates`] finds the `ID_FIL` element holding an
//!    `EXT_SBR_DATA`/`EXT_SBR_DATA_CRC` extension. symphonia does not expose
//!    the raw data block elements and a full `raw_data_block` parser would
//!    need the whole spectral decoder, so the elements are located from the
//!    back: the block ends with `ID_END` and 0-7 zero padding bits, before
//!    it come only fill elements (the SBR one, then the encoder's padding:
//!    `EXT_FILL_DATA` bytes or empty ones), and a fill element's `count` field
//!    must land exactly on the start of the next one. The few look-alikes this
//!    leaves are weeded out by actually parsing the payload (`Ctl::parse`),
//!    which must succeed and consume the payload to within its alignment
//!    bits. (Checked against libfdk-aac output: every frame of the
//!    `he_aac_tone.adts` fixture yields its SBR element, one of them with a
//!    spurious second candidate behind it.)
//! 2. `Ctl::parse` reads the SBR header and the single or paired channel
//!    element: frequency tables, time/frequency grid, delta coded envelope
//!    and noise floor, inverse filtering modes, added sinusoids.
//! 3. 64-band complex QMF analysis of the core, HF generation by
//!    copy-up patches with complex linear prediction (inverse filtering),
//!    envelope estimation, gain/limiter calculation, HF assembly (noise and
//!    sinusoids, gain smoothing) and 64-band QMF synthesis.
//!
//! The structure follows FFmpeg's `aacsbr` (LGPL) closely, since the standard
//! text carries its quirks (timing offsets, borrowed time slots) only
//! implicitly. The QMF banks are plain matrix products, O(n^2) instead of
//! FFmpeg's shuffled MDCT (cheap enough: about 30 M multiply-adds a second
//! for stereo 44.1 kHz). Floating point throughout, on the i16 scale.
//!
//! Verification: the algorithm was prototyped in Python (a line by line
//! mirror of this module) and compared with FFmpeg's and faad2's output on
//! libfdk-aac encoded HE-AAC streams (mono and stereo, 32 to 48 kHz, all
//! four frame classes, coupled and independent channels): 72-75 dB SNR, at
//! most 2 LSB difference. The AAC-LC core for those runs came from the same
//! streams with the SBR fill elements cut out.
//!
//! Not implemented: parametric stereo (HE-AAC v2). For SBR+PS streams the
//! mono core gets SBR and stays mono. Corrupt or missing SBR data falls back
//! to the plain core, upsampled through the same QMF chain (so no click).

use super::sbr_tables::{HUFF, HUFF_OFFSET, NOISE, QMF_WINDOW};
use super::sbr_tables::{F_ENV_1_5, F_ENV_3_0, F_ENV_BAL_1_5, F_ENV_BAL_3_0, T_ENV_1_5, T_ENV_3_0, T_ENV_BAL_1_5, T_ENV_BAL_3_0, T_NOISE_3_0, T_NOISE_BAL_3_0};
use std::sync::OnceLock;

/// Core samples per AAC frame (and per channel, per `process` call).
pub const CORE_FRAME: usize = 1024;
/// SBR time slots per frame.
const TIME_SLOTS: usize = 16;
/// QMF time slots per frame (two per SBR time slot).
const SLOTS: usize = 32;
/// Time slots the HF generator looks back (`tHFGen`).
const T_HF_GEN: usize = 8;
/// Offset of the envelope adjuster in the time axis (`tHFAdj`).
const T_HF_ADJ: usize = 2;
const NOISE_FLOOR_OFFSET: i32 = 6;

const FIXFIX: u32 = 0;
const FIXVAR: u32 = 1;
const VARFIX: u32 = 2;

/// `ceil(log2(n + 1))` for the envelope counts of a frame.
const CEIL_LOG2: [usize; 6] = [0, 1, 2, 2, 3, 3];

/// Start frequency offsets per SBR sample rate class (table 4.69..4.74).
const START_OFFSET: [[i32; 16]; 6] = [
    [-8, -7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7],
    [-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13],
    [-5, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16],
    [-6, -4, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16],
    [-4, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16, 20],
    [-2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16, 20, 24],
];

/// Core rates SBR can double (the SBR rate must have a frequency table).
pub fn supported(core_rate: u32) -> bool {
    matches!(core_rate * 2, 16000 | 22050 | 24000 | 32000 | 44100 | 48000 | 64000 | 88200 | 96000)
}

// ---------------------------------------------------------------- bits

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], pos: usize, end: usize) -> Self {
        Bits { data, pos, end: end.min(data.len() * 8) }
    }

    fn left(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }

    fn bits(&mut self, n: usize) -> Option<u32> {
        if n > self.left() {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..n {
            let b = (self.data[self.pos >> 3] >> (7 - (self.pos & 7))) & 1;
            v = v << 1 | b as u32;
            self.pos += 1;
        }
        Some(v)
    }

    fn bit(&mut self) -> Option<bool> {
        self.bits(1).map(|b| b == 1)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        if n > self.left() {
            return None;
        }
        self.pos += n;
        Some(())
    }
}

/// `n` bits at bit offset `pos` of `data`, None past the end.
fn peek(data: &[u8], pos: usize, n: usize) -> Option<u32> {
    Bits::new(data, pos, data.len() * 8).bits(n)
}

// ------------------------------------------------------------- huffman

/// A canonical Huffman table: codes of one length are consecutive.
struct Huff {
    first: [u32; 21],
    count: [u32; 21],
    base: [u32; 21],
    syms: Vec<i32>,
}

impl Huff {
    fn new(table: &[(u8, u8)], offset: i32) -> Huff {
        let mut h = Huff { first: [0; 21], count: [0; 21], base: [0; 21], syms: Vec::with_capacity(table.len()) };
        let mut code = 0u32;
        let mut prev = table[0].1 as u32;
        for (i, &(sym, len)) in table.iter().enumerate() {
            let len = len as u32;
            if i > 0 {
                code = (code + 1) << (len - prev);
            }
            if h.count[len as usize] == 0 {
                h.first[len as usize] = code;
                h.base[len as usize] = i as u32;
            }
            h.count[len as usize] += 1;
            h.syms.push(sym as i32 + offset);
            prev = len;
        }
        h
    }

    fn read(&self, r: &mut Bits<'_>) -> Option<i32> {
        let mut code = 0u32;
        for len in 1..=20 {
            code = code << 1 | r.bit()? as u32;
            let c = self.count[len];
            if c > 0 && code >= self.first[len] && code - self.first[len] < c {
                return Some(self.syms[(self.base[len] + code - self.first[len]) as usize]);
            }
        }
        None
    }
}

fn huff(table: usize) -> &'static Huff {
    static TABLES: OnceLock<Vec<Huff>> = OnceLock::new();
    &TABLES.get_or_init(|| (0..10).map(|i| Huff::new(HUFF[i], HUFF_OFFSET[i])).collect())[table]
}

// --------------------------------------------------------- fill file_manager

/// Locates SBR extension payloads in a raw data block: returns, nearest the
/// end first, `(bit offset after the extension type nibble, payload bits,
/// crc present)`. See the module doc for why it works from the back.
pub fn fill_candidates(block: &[u8]) -> Vec<(usize, usize, bool)> {
    let total = block.len() * 8;
    let mut found = Vec::new();
    for pad in 0..8usize {
        if total < pad + 3 + 7 {
            break;
        }
        let end = total - pad - 3;
        // ID_END (111) then zero bits up to the byte boundary
        if peek(block, end, 3) != Some(7) || (pad > 0 && peek(block, end + 3, pad) != Some(0)) {
            continue;
        }
        // reach[p]: a chain of fill elements leads from bit p up to ID_END
        let mut reach = vec![false; end + 1];
        reach[end] = true;
        for s in (0..end).rev() {
            if peek(block, s, 3) != Some(6) {
                continue;
            }
            let Some(mut count) = peek(block, s + 3, 4).map(|c| c as usize) else { continue };
            let mut at = s + 7;
            if count == 15 {
                let Some(esc) = peek(block, at, 8) else { continue };
                count = count + esc as usize - 1;
                at += 8;
            }
            let after = at + 8 * count;
            if after > end || !reach[after] {
                continue;
            }
            reach[s] = true;
            if count == 0 {
                continue; // an empty fill element, only padding
            }
            if let Some(t @ (0xd | 0xe)) = peek(block, at, 4) {
                found.push((at + 4, 8 * count - 4, t == 0xe));
            }
        }
    }
    found
}

// -------------------------------------------------------- bitstream state

/// Per-channel data from the bitstream (the standard's `bs_*` elements) and
/// the values derived from it that persist from frame to frame.
#[derive(Clone)]
struct Bs {
    frame_class: u32,
    add_harmonic_flag: bool,
    num_env: usize,
    /// resolution of envelope `i`, index 0 is the last of the previous frame
    freq_res: [usize; 8],
    num_noise: usize,
    df_env: [bool; 8],
    df_noise: [bool; 2],
    /// inverse filtering mode, `[0]` current, `[1]` previous frame
    invf: [[usize; 5]; 2],
    add_harmonic: [bool; 64],
    amp_res: bool,
    /// `l_APrev`, `l_A`
    e_a: [i32; 2],
    env_q: [[i32; 64]; 6],
    env: [[f32; 64]; 6],
    noise_q: [[i32; 5]; 3],
    noise: [[f32; 5]; 3],
    t_env: [i32; 7],
    t_env_old: i32,
    t_q: [i32; 3],
}

impl Bs {
    fn new() -> Bs {
        Bs {
            frame_class: 0,
            add_harmonic_flag: false,
            num_env: 0,
            freq_res: [0; 8],
            num_noise: 0,
            df_env: [false; 8],
            df_noise: [false; 2],
            invf: [[0; 5]; 2],
            add_harmonic: [false; 64],
            amp_res: false,
            e_a: [0, -1],
            env_q: [[0; 64]; 6],
            env: [[0.0; 64]; 6],
            noise_q: [[0; 5]; 3],
            noise: [[0.0; 5]; 3],
            t_env: [0; 7],
            t_env_old: 0,
            t_q: [0; 3],
        }
    }
}

/// Header, derived frequency tables and both channels' bitstream data.
#[derive(Clone)]
struct Ctl {
    sample_rate: i32,
    start: bool,
    reset: bool,
    /// (start_freq, stop_freq, xover_band, freq_scale, alter_scale, noise_bands)
    params: Option<[u32; 6]>,
    amp_res_header: bool,
    limiter_bands: usize,
    limiter_gains: usize,
    interpol_freq: bool,
    smoothing_mode: bool,
    coupling: bool,
    k: [i32; 3],
    /// first SBR subband, `[0]` = previous frame
    kx: [i32; 2],
    /// number of SBR subbands, `[0]` = previous frame
    m: [i32; 2],
    n_master: usize,
    n: [usize; 2],
    n_q: usize,
    n_lim: usize,
    f_master: [i32; 64],
    f_low: [i32; 32],
    f_high: [i32; 64],
    f_noise: [i32; 8],
    f_lim: [i32; 64],
    num_patches: usize,
    patch_num_subbands: [i32; 8],
    patch_start_subband: [i32; 8],
    data: [Bs; 2],
    channels: usize,
}

fn lrint(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// Splits `start..stop` into `num` bands of geometrically growing width and
/// stores the widths (14496-3 4.6.18.3.2).
fn make_bands(bands: &mut [i32], start: i32, stop: i32, num: usize) {
    let base = (stop as f32 / start as f32).powf(1.0 / num as f32);
    let mut prod = start as f32;
    let mut previous = start;
    for b in bands.iter_mut().take(num - 1) {
        prod *= base;
        let present = lrint(prod);
        *b = present - previous;
        previous = present;
    }
    bands[num - 1] = stop - previous;
}

impl Ctl {
    fn new(channels: usize, core_rate: u32) -> Ctl {
        let mut c = Ctl {
            sample_rate: (core_rate * 2) as i32,
            start: false,
            reset: false,
            params: None,
            amp_res_header: false,
            limiter_bands: 2,
            limiter_gains: 2,
            interpol_freq: true,
            smoothing_mode: true,
            coupling: false,
            k: [0; 3],
            kx: [32; 2],
            m: [0; 2],
            n_master: 0,
            n: [0; 2],
            n_q: 0,
            n_lim: 0,
            f_master: [0; 64],
            f_low: [0; 32],
            f_high: [0; 64],
            f_noise: [0; 8],
            f_lim: [0; 64],
            num_patches: 0,
            patch_num_subbands: [0; 8],
            patch_start_subband: [0; 8],
            data: [Bs::new(), Bs::new()],
            channels,
        };
        c.turnoff();
        c
    }

    /// Pure upsampling mode until the next header (`sbr_turnoff`).
    fn turnoff(&mut self) {
        self.start = false;
        self.kx[1] = 32;
        self.m[1] = 0;
        self.data[0].e_a[1] = -1;
        self.data[1].e_a[1] = -1;
        self.params = None;
    }

    /// Parses one SBR extension payload; Some(true) when the frame's data
    /// was read, Some(false) when it is well formed but carries no data we
    /// can use yet (no header seen), None when it is not valid SBR.
    fn parse(&mut self, block: &[u8], at: usize, bits: usize, crc: bool) -> Option<bool> {
        let mut r = Bits::new(block, at, at + bits);
        self.reset = false;
        if crc {
            r.skip(10)?; // bs_sbr_crc_bits, not checked
        }
        if r.bit()? {
            self.read_header(&mut r)?;
        }
        if self.reset {
            self.make_tables()?;
        }
        if !self.start {
            return Some(false);
        }
        self.read_data(&mut r)?;
        // what is left is the byte alignment of the extension payload
        if r.left() >= 8 {
            return None;
        }
        Some(true)
    }

    fn read_header(&mut self, r: &mut Bits<'_>) -> Option<()> {
        let old_limiter = self.limiter_bands;
        self.start = true;
        self.amp_res_header = r.bit()?;
        let start_freq = r.bits(4)?;
        let stop_freq = r.bits(4)?;
        let xover = r.bits(3)?;
        r.skip(2)?;
        let extra1 = r.bit()?;
        let extra2 = r.bit()?;
        let (freq_scale, alter_scale, noise_bands) = if extra1 { (r.bits(2)?, r.bits(1)?, r.bits(2)?) } else { (2, 1, 2) };
        let params = [start_freq, stop_freq, xover, freq_scale, alter_scale, noise_bands];
        if self.params != Some(params) {
            self.reset = true;
        }
        self.params = Some(params);
        if extra2 {
            self.limiter_bands = r.bits(2)? as usize;
            self.limiter_gains = r.bits(2)? as usize;
            self.interpol_freq = r.bit()?;
            self.smoothing_mode = r.bit()?;
        } else {
            self.limiter_bands = 2;
            self.limiter_gains = 2;
            self.interpol_freq = true;
            self.smoothing_mode = true;
        }
        if self.limiter_bands != old_limiter && !self.reset {
            self.make_f_lim();
        }
        Some(())
    }

    /// Builds all frequency tables from the header (`sbr_reset`); on failure
    /// SBR is switched off.
    fn make_tables(&mut self) -> Option<()> {
        let ok = self.make_f_master().is_some() && self.make_f_derived().is_some();
        if !ok {
            self.turnoff();
            return None;
        }
        Some(())
    }

    /// Master frequency band table (4.6.18.3.2).
    fn make_f_master(&mut self) -> Option<()> {
        let [start_freq, stop_freq, xover, freq_scale, alter_scale, _] = self.params?;
        let (start_freq, stop_freq, xover) = (start_freq as usize, stop_freq as usize, xover as usize);
        let sr = self.sample_rate;
        let offsets = &START_OFFSET[match sr {
            16000 => 0,
            22050 => 1,
            24000 => 2,
            32000 => 3,
            44100 | 48000 | 64000 => 4,
            88200 | 96000 | 128000 | 176400 | 192000 => 5,
            _ => return None,
        }];
        let temp = if sr < 32000 {
            3000
        } else if sr < 64000 {
            4000
        } else {
            5000
        };
        let start_min = ((temp << 7) + (sr >> 1)) / sr;
        let stop_min = ((temp << 8) + (sr >> 1)) / sr;
        self.k[0] = start_min + offsets[start_freq];
        if stop_freq < 14 {
            let mut stop_dk = [0i32; 13];
            make_bands(&mut stop_dk, stop_min, 64, 13);
            stop_dk.sort_unstable();
            self.k[2] = stop_min + stop_dk[..stop_freq].iter().sum::<i32>();
        } else if stop_freq == 14 {
            self.k[2] = 2 * self.k[0];
        } else {
            self.k[2] = 3 * self.k[0];
        }
        self.k[2] = self.k[2].min(64);
        let max_subbands = if sr <= 32000 {
            48
        } else if sr == 44100 {
            35
        } else {
            32
        };
        let (k0, k2) = (self.k[0], self.k[2]);
        if k0 <= 0 || k2 - k0 > max_subbands || k2 <= k0 {
            return None;
        }
        let check = |n: i32| (n > 0 && n <= 48 && xover < n as usize).then_some(n as usize);
        if freq_scale == 0 {
            let dk = alter_scale as i32 + 1;
            let n_master = check(((k2 - k0 + (dk & 2)) >> dk) << 1)?;
            self.n_master = n_master;
            for k in 1..=n_master {
                self.f_master[k] = dk;
            }
            let k2diff = k2 - k0 - n_master as i32 * dk;
            if k2diff < 0 {
                self.f_master[1] -= 1;
                self.f_master[2] -= (k2diff < -1) as i32;
            } else if k2diff > 0 {
                self.f_master[n_master] += 1;
            }
            self.f_master[0] = k0;
            for k in 1..=n_master {
                self.f_master[k] += self.f_master[k - 1];
            }
            return Some(());
        }
        let half_bands = 7 - freq_scale as i32;
        let two_regions = 49 * k2 > 110 * k0;
        let k1 = if two_regions { 2 * k0 } else { k2 };
        self.k[1] = k1;
        let num_bands_0 = lrint(half_bands as f32 * (k1 as f32 / k0 as f32).log2()) * 2;
        if num_bands_0 <= 0 || num_bands_0 > 48 {
            return None;
        }
        let nb0 = num_bands_0 as usize;
        let mut vk0 = vec![0i32; nb0 + 1];
        make_bands(&mut vk0[1..], k0, k1, nb0);
        vk0[1..].sort_unstable();
        let vdk0_max = vk0[nb0];
        vk0[0] = k0;
        for k in 1..=nb0 {
            if vk0[k] <= 0 {
                return None;
            }
            vk0[k] += vk0[k - 1];
        }
        if !two_regions {
            self.n_master = check(num_bands_0)?;
            self.f_master[..=nb0].copy_from_slice(&vk0);
            return Some(());
        }
        let invwarp = if alter_scale == 1 { 0.76923077f32 } else { 1.0 };
        let num_bands_1 = lrint(half_bands as f32 * invwarp * (k2 as f32 / k1 as f32).log2()) * 2;
        if num_bands_1 <= 0 || num_bands_1 > 48 {
            return None;
        }
        let nb1 = num_bands_1 as usize;
        let mut vk1 = vec![0i32; nb1 + 1];
        make_bands(&mut vk1[1..], k1, k2, nb1);
        let vdk1_min = *vk1[1..].iter().min()?;
        if vdk1_min < vdk0_max {
            vk1[1..].sort_unstable();
            let change = (vdk0_max - vk1[1]).min((vk1[nb1] - vk1[1]) >> 1);
            vk1[1] += change;
            vk1[nb1] -= change;
        }
        vk1[1..].sort_unstable();
        vk1[0] = k1;
        for k in 1..=nb1 {
            if vk1[k] <= 0 {
                return None;
            }
            vk1[k] += vk1[k - 1];
        }
        self.n_master = check(num_bands_0 + num_bands_1)?;
        self.f_master[..=nb0].copy_from_slice(&vk0);
        self.f_master[nb0 + 1..=nb0 + nb1].copy_from_slice(&vk1[1..]);
        Some(())
    }

    /// High/low resolution, noise and limiter tables (4.6.18.3.2).
    fn make_f_derived(&mut self) -> Option<()> {
        let [_, _, xover, _, _, noise_bands] = self.params?;
        let xover = xover as usize;
        self.n[1] = self.n_master - xover;
        self.n[0] = (self.n[1] + 1) >> 1;
        let n1 = self.n[1];
        self.f_high[..=n1].copy_from_slice(&self.f_master[xover..=xover + n1]);
        self.m[1] = self.f_high[n1] - self.f_high[0];
        self.kx[1] = self.f_high[0];
        if self.kx[1] + self.m[1] > 64 || self.kx[1] > 32 || self.kx[1] <= 0 || self.m[1] <= 0 {
            return None;
        }
        self.f_low[0] = self.f_high[0];
        let odd = n1 & 1;
        for k in 1..=self.n[0] {
            self.f_low[k] = self.f_high[2 * k - odd];
        }
        let n_q = lrint(noise_bands as f32 * (self.k[2] as f32 / self.kx[1] as f32).log2()).max(1);
        if n_q > 5 {
            return None;
        }
        self.n_q = n_q as usize;
        self.f_noise[0] = self.f_low[0];
        let mut temp = 0usize;
        for k in 1..=self.n_q {
            temp += (self.n[0] - temp) / (self.n_q + 1 - k);
            self.f_noise[k] = self.f_low[temp];
        }
        self.calc_patches()?;
        self.make_f_lim();
        Some(())
    }

    /// HF generation patch construction (4.6.18.6.3).
    fn calc_patches(&mut self) -> Option<()> {
        let (k0, kx1, m1) = (self.k[0], self.kx[1], self.m[1]);
        let mut num_patches = 0usize;
        let (mut last_k, mut last_msb) = (-1i32, -1i32);
        let mut sb: i32;
        let mut msb = k0;
        let mut usb = kx1;
        let goal_sb = ((1000 << 11) + (self.sample_rate >> 1)) / self.sample_rate;
        let mut k = if goal_sb < kx1 + m1 {
            let mut k = 0usize;
            while self.f_master[k] < goal_sb {
                k += 1;
                if k > self.n_master {
                    return None;
                }
            }
            k
        } else {
            self.n_master
        };
        loop {
            if k as i32 == last_k && msb == last_msb {
                return None;
            }
            last_k = k as i32;
            last_msb = msb;
            let mut odd;
            let mut i = k as i32;
            loop {
                sb = self.f_master[i as usize];
                odd = (sb + k0) & 1;
                i -= 1;
                if !(sb > k0 - 1 + msb - odd) {
                    break;
                }
                if i < 0 {
                    return None;
                }
            }
            if num_patches > 5 {
                return None;
            }
            let num_sub = (sb - usb).max(0);
            self.patch_num_subbands[num_patches] = num_sub;
            self.patch_start_subband[num_patches] = k0 - odd - num_sub;
            if num_sub > 0 {
                usb = sb;
                msb = sb;
                num_patches += 1;
            } else {
                msb = kx1;
            }
            if self.f_master[k] - sb < 3 {
                k = self.n_master;
            }
            if sb == kx1 + m1 {
                break;
            }
        }
        if num_patches > 1 && self.patch_num_subbands[num_patches - 1] < 3 {
            num_patches -= 1;
        }
        self.num_patches = num_patches;
        Some(())
    }

    /// Limiter frequency band table (4.6.18.3.2).
    fn make_f_lim(&mut self) {
        let n0 = self.n[0];
        if self.limiter_bands == 0 {
            self.f_lim[0] = self.f_low[0];
            self.f_lim[1] = self.f_low[n0];
            self.n_lim = 1;
            return;
        }
        let warped = [1.327_151_8f32, 1.185_092_8, 1.119_871_6][self.limiter_bands - 1];
        let np = self.num_patches;
        let mut borders = [0i32; 8];
        borders[0] = self.kx[1];
        for k in 1..=np {
            borders[k] = borders[k - 1] + self.patch_num_subbands[k - 1];
        }
        let in_borders = |v: i32| borders[..=np].contains(&v);
        let mut t: Vec<i32> = self.f_low[..=n0].to_vec();
        if np > 1 {
            t.extend_from_slice(&borders[1..np]);
        }
        t.sort_unstable();
        let mut n_lim = n0 + np - 1;
        let (mut i, mut out) = (1usize, 0usize);
        while out < n_lim && i < t.len() {
            if t[i] as f32 >= t[out] as f32 * warped {
                out += 1;
                t[out] = t[i];
                i += 1;
            } else if t[i] == t[out] || !in_borders(t[i]) {
                i += 1;
                n_lim -= 1;
            } else if !in_borders(t[out]) {
                t[out] = t[i];
                i += 1;
                n_lim -= 1;
            } else {
                out += 1;
                t[out] = t[i];
                i += 1;
            }
        }
        self.n_lim = n_lim.min(t.len() - 1).min(62);
        self.f_lim[..=self.n_lim].copy_from_slice(&t[..=self.n_lim]);
    }

    // ------------------------------------------------------ channel data

    fn read_grid(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let amp_res_header = self.amp_res_header;
        let d = &mut self.data[ch];
        let old_num_env = d.num_env;
        d.freq_res[0] = d.freq_res[old_num_env];
        d.amp_res = amp_res_header;
        d.t_env_old = d.t_env[old_num_env];
        let mut abs_bord_trail = TIME_SLOTS as i32;
        let class = r.bits(2)?;
        let mut pointer = 0usize;
        let num_env;
        match class {
            FIXFIX => {
                num_env = 1usize << r.bits(2)?;
                if num_env > 5 {
                    return None;
                }
                d.num_env = num_env;
                if num_env == 1 {
                    d.amp_res = false;
                }
                d.t_env[0] = 0;
                d.t_env[num_env] = abs_bord_trail;
                abs_bord_trail = (abs_bord_trail + (num_env as i32 >> 1)) / num_env as i32;
                for i in 0..num_env - 1 {
                    d.t_env[i + 1] = d.t_env[i] + abs_bord_trail;
                }
                d.freq_res[1] = r.bits(1)? as usize;
                for i in 1..num_env {
                    d.freq_res[i + 1] = d.freq_res[1];
                }
            }
            FIXVAR => {
                abs_bord_trail += r.bits(2)? as i32;
                let num_rel_trail = r.bits(2)? as usize;
                num_env = num_rel_trail + 1;
                d.num_env = num_env;
                d.t_env[0] = 0;
                d.t_env[num_env] = abs_bord_trail;
                for i in 0..num_rel_trail {
                    d.t_env[num_env - 1 - i] = d.t_env[num_env - i] - 2 * r.bits(2)? as i32 - 2;
                }
                pointer = r.bits(CEIL_LOG2[num_env])? as usize;
                for i in 0..num_env {
                    d.freq_res[num_env - i] = r.bits(1)? as usize;
                }
            }
            VARFIX => {
                d.t_env[0] = r.bits(2)? as i32;
                let num_rel_lead = r.bits(2)? as usize;
                num_env = num_rel_lead + 1;
                d.num_env = num_env;
                d.t_env[num_env] = abs_bord_trail;
                for i in 0..num_rel_lead {
                    d.t_env[i + 1] = d.t_env[i] + 2 * r.bits(2)? as i32 + 2;
                }
                pointer = r.bits(CEIL_LOG2[num_env])? as usize;
                for i in 0..num_env {
                    d.freq_res[1 + i] = r.bits(1)? as usize;
                }
            }
            _ => {
                d.t_env[0] = r.bits(2)? as i32;
                abs_bord_trail += r.bits(2)? as i32;
                let num_rel_lead = r.bits(2)? as usize;
                let num_rel_trail = r.bits(2)? as usize;
                num_env = num_rel_lead + num_rel_trail + 1;
                if num_env > 5 {
                    return None;
                }
                d.num_env = num_env;
                d.t_env[num_env] = abs_bord_trail;
                for i in 0..num_rel_lead {
                    d.t_env[i + 1] = d.t_env[i] + 2 * r.bits(2)? as i32 + 2;
                }
                for i in 0..num_rel_trail {
                    d.t_env[num_env - 1 - i] = d.t_env[num_env - i] - 2 * r.bits(2)? as i32 - 2;
                }
                pointer = r.bits(CEIL_LOG2[num_env])? as usize;
                for i in 0..num_env {
                    d.freq_res[1 + i] = r.bits(1)? as usize;
                }
            }
        }
        d.frame_class = class;
        if pointer > num_env + 1 {
            return None;
        }
        for i in 1..=num_env {
            if d.t_env[i - 1] >= d.t_env[i] {
                return None;
            }
        }
        if d.t_env[num_env] > TIME_SLOTS as i32 + 3 || d.t_env[0] < 0 {
            return None;
        }
        d.num_noise = (num_env > 1) as usize + 1;
        d.t_q[0] = d.t_env[0];
        d.t_q[d.num_noise] = d.t_env[num_env];
        if d.num_noise > 1 {
            let idx = if class == FIXFIX {
                num_env >> 1
            } else if class & 1 == 1 {
                num_env - pointer.saturating_sub(1).max(1)
            } else if pointer == 0 {
                1
            } else if pointer == 1 {
                num_env - 1
            } else {
                pointer - 1
            };
            d.t_q[1] = d.t_env[idx];
        }
        d.e_a[0] = -((d.e_a[1] != old_num_env as i32) as i32);
        d.e_a[1] = -1;
        if class & 1 == 1 && pointer != 0 {
            d.e_a[1] = (num_env + 1 - pointer) as i32;
        } else if class == VARFIX && pointer > 1 {
            d.e_a[1] = pointer as i32 - 1;
        }
        Some(())
    }

    /// Channel 1 shares channel 0's grid (coupled stereo).
    fn copy_grid(&mut self) {
        let src = self.data[0].clone();
        let dst = &mut self.data[1];
        dst.freq_res[0] = dst.freq_res[dst.num_env];
        dst.t_env_old = dst.t_env[dst.num_env];
        dst.e_a[0] = -((dst.e_a[1] != dst.num_env as i32) as i32);
        dst.freq_res[1..].copy_from_slice(&src.freq_res[1..]);
        dst.t_env = src.t_env;
        dst.t_q = src.t_q;
        dst.num_env = src.num_env;
        dst.amp_res = src.amp_res;
        dst.num_noise = src.num_noise;
        dst.frame_class = src.frame_class;
        dst.e_a[1] = src.e_a[1];
    }

    fn read_dtdf(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let d = &mut self.data[ch];
        for i in 0..d.num_env {
            d.df_env[i] = r.bit()?;
        }
        for i in 0..d.num_noise {
            d.df_noise[i] = r.bit()?;
        }
        Some(())
    }

    fn read_invf(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let n_q = self.n_q;
        let d = &mut self.data[ch];
        d.invf[1] = d.invf[0];
        for i in 0..n_q {
            d.invf[0][i] = r.bits(2)? as usize;
        }
        Some(())
    }

    fn read_envelope(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let (coupling, n) = (self.coupling, self.n);
        let d = &mut self.data[ch];
        let delta = (ch == 1 && coupling) as i32 + 1;
        let odd = n[1] & 1;
        let (bits, t_huff, f_huff) = match (coupling && ch == 1, d.amp_res) {
            (true, true) => (5, huff(T_ENV_BAL_3_0), huff(F_ENV_BAL_3_0)),
            (true, false) => (6, huff(T_ENV_BAL_1_5), huff(F_ENV_BAL_1_5)),
            (false, true) => (6, huff(T_ENV_3_0), huff(F_ENV_3_0)),
            (false, false) => (7, huff(T_ENV_1_5), huff(F_ENV_1_5)),
        };
        for i in 0..d.num_env {
            let res = d.freq_res[i + 1];
            let bands = n[res];
            if d.df_env[i] {
                // coded against the previous envelope (index 0: last frame)
                for j in 0..bands {
                    let k = if d.freq_res[i] == res {
                        j
                    } else if res == 1 {
                        (j + odd) >> 1
                    } else if j > 0 {
                        2 * j - odd
                    } else {
                        0
                    };
                    let v = d.env_q[i][k] + delta * t_huff.read(r)?;
                    if !(0..=127).contains(&v) {
                        return None;
                    }
                    d.env_q[i + 1][j] = v;
                }
            } else {
                d.env_q[i + 1][0] = delta * r.bits(bits)? as i32;
                for j in 1..bands {
                    let v = d.env_q[i + 1][j - 1] + delta * f_huff.read(r)?;
                    if !(0..=127).contains(&v) {
                        return None;
                    }
                    d.env_q[i + 1][j] = v;
                }
            }
        }
        d.env_q[0] = d.env_q[d.num_env];
        Some(())
    }

    fn read_noise(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let (coupling, n_q) = (self.coupling, self.n_q);
        let d = &mut self.data[ch];
        let delta = (ch == 1 && coupling) as i32 + 1;
        let (t_huff, f_huff) = if coupling && ch == 1 { (huff(T_NOISE_BAL_3_0), huff(F_ENV_BAL_3_0)) } else { (huff(T_NOISE_3_0), huff(F_ENV_3_0)) };
        for i in 0..d.num_noise {
            if d.df_noise[i] {
                for j in 0..n_q {
                    let v = d.noise_q[i][j] + delta * t_huff.read(r)?;
                    if !(0..=30).contains(&v) {
                        return None;
                    }
                    d.noise_q[i + 1][j] = v;
                }
            } else {
                d.noise_q[i + 1][0] = delta * r.bits(5)? as i32;
                for j in 1..n_q {
                    let v = d.noise_q[i + 1][j - 1] + delta * f_huff.read(r)?;
                    if !(0..=30).contains(&v) {
                        return None;
                    }
                    d.noise_q[i + 1][j] = v;
                }
            }
        }
        d.noise_q[0] = d.noise_q[d.num_noise];
        Some(())
    }

    fn read_harmonics(&mut self, r: &mut Bits<'_>, ch: usize) -> Option<()> {
        let n1 = self.n[1];
        let d = &mut self.data[ch];
        d.add_harmonic_flag = r.bit()?;
        if d.add_harmonic_flag {
            for i in 0..n1 {
                d.add_harmonic[i] = r.bit()?;
            }
        }
        Some(())
    }

    /// `sbr_single_channel_element` / `sbr_channel_pair_element`, then the
    /// extension data (skipped: it only carries parametric stereo), then
    /// dequantisation.
    fn read_data(&mut self, r: &mut Bits<'_>) -> Option<()> {
        if self.channels == 1 {
            if r.bit()? {
                r.skip(4)?;
            }
            self.read_grid(r, 0)?;
            self.read_dtdf(r, 0)?;
            self.read_invf(r, 0)?;
            self.read_envelope(r, 0)?;
            self.read_noise(r, 0)?;
            self.read_harmonics(r, 0)?;
        } else {
            if r.bit()? {
                r.skip(8)?;
            }
            self.coupling = r.bit()?;
            if self.coupling {
                self.read_grid(r, 0)?;
                self.copy_grid();
                self.read_dtdf(r, 0)?;
                self.read_dtdf(r, 1)?;
                self.read_invf(r, 0)?;
                self.data[1].invf[1] = self.data[1].invf[0];
                self.data[1].invf[0] = self.data[0].invf[0];
                self.read_envelope(r, 0)?;
                self.read_noise(r, 0)?;
                self.read_envelope(r, 1)?;
                self.read_noise(r, 1)?;
            } else {
                self.read_grid(r, 0)?;
                self.read_grid(r, 1)?;
                self.read_dtdf(r, 0)?;
                self.read_dtdf(r, 1)?;
                self.read_invf(r, 0)?;
                self.read_invf(r, 1)?;
                self.read_envelope(r, 0)?;
                self.read_envelope(r, 1)?;
                self.read_noise(r, 0)?;
                self.read_noise(r, 1)?;
            }
            self.read_harmonics(r, 0)?;
            self.read_harmonics(r, 1)?;
        }
        if r.bit()? {
            // bs_extended_data: size in bytes, content skipped
            let mut size = r.bits(4)? as usize;
            if size == 15 {
                size += r.bits(8)? as usize;
            }
            r.skip(size * 8)?;
        }
        self.dequant();
        Some(())
    }

    /// Dequantisation and stereo decoding (4.6.18.3.5).
    fn dequant(&mut self) {
        let exp2 = |x: i32| 2f32.powi(x);
        let root2 = [1.0f32, std::f32::consts::SQRT_2];
        let n = self.n;
        let n_q = self.n_q;
        if self.channels == 2 && self.coupling {
            let amp_res = self.data[0].amp_res;
            let pan_offset = if amp_res { 12 } else { 24 };
            for e in 1..=self.data[0].num_env {
                for k in 0..n[self.data[0].freq_res[e]] {
                    let (q0, q1) = (self.data[0].env_q[e][k], self.data[1].env_q[e][k]);
                    let (mut temp1, temp2) = if amp_res {
                        (exp2(q0 + 7), exp2(pan_offset - q1))
                    } else {
                        (exp2((q0 >> 1) + 7) * root2[(q0 & 1) as usize], exp2((pan_offset - q1) >> 1) * root2[((pan_offset - q1) & 1) as usize])
                    };
                    if temp1 > 1e20 {
                        temp1 = 1.0;
                    }
                    let fac = temp1 / (1.0 + temp2);
                    self.data[0].env[e][k] = fac;
                    self.data[1].env[e][k] = fac * temp2;
                }
            }
            for e in 1..=self.data[0].num_noise {
                for k in 0..n_q {
                    let temp1 = exp2(NOISE_FLOOR_OFFSET - self.data[0].noise_q[e][k] + 1);
                    let temp2 = exp2(12 - self.data[1].noise_q[e][k]);
                    let fac = temp1 / (1.0 + temp2);
                    self.data[0].noise[e][k] = fac;
                    self.data[1].noise[e][k] = fac * temp2;
                }
            }
        } else {
            for ch in 0..self.channels {
                let d = &mut self.data[ch];
                for e in 1..=d.num_env {
                    for k in 0..n[d.freq_res[e]] {
                        let q = d.env_q[e][k];
                        let mut v = if d.amp_res { exp2(q + 6) } else { exp2((q >> 1) + 6) * root2[(q & 1) as usize] };
                        if v > 1e20 {
                            v = 1.0;
                        }
                        d.env[e][k] = v;
                    }
                }
                for e in 1..=d.num_noise {
                    for k in 0..n_q {
                        d.noise[e][k] = exp2(NOISE_FLOOR_OFFSET - d.noise_q[e][k]);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ DSP state

/// Transform matrices of the two QMF banks, f32, built once.
struct Trig {
    /// 2 cos(pi/64 (k+1/2)(2n-1/2)), `[k][n]`, 32 x 64
    ana_cos: Vec<f32>,
    ana_sin: Vec<f32>,
    /// cos(pi/128 (k+1/2)(2n-255)) / 64, `[n][k]`, 128 x 64
    syn_cos: Vec<f32>,
    syn_sin: Vec<f32>,
}

fn trig() -> &'static Trig {
    static T: OnceLock<Trig> = OnceLock::new();
    T.get_or_init(|| {
        let pi = std::f64::consts::PI;
        let (mut ac, mut as_) = (vec![0f32; 32 * 64], vec![0f32; 32 * 64]);
        for k in 0..32 {
            for n in 0..64 {
                let a = pi / 64.0 * (k as f64 + 0.5) * (2.0 * n as f64 - 0.5);
                ac[k * 64 + n] = (2.0 * a.cos()) as f32;
                as_[k * 64 + n] = (2.0 * a.sin()) as f32;
            }
        }
        let (mut sc, mut ss) = (vec![0f32; 128 * 64], vec![0f32; 128 * 64]);
        for n in 0..128 {
            for k in 0..64 {
                let a = pi / 128.0 * (k as f64 + 0.5) * (2.0 * n as f64 - 255.0);
                sc[n * 64 + k] = (a.cos() / 64.0) as f32;
                ss[n * 64 + k] = (a.sin() / 64.0) as f32;
            }
        }
        Trig { ana_cos: ac, ana_sin: as_, syn_cos: sc, syn_sin: ss }
    })
}

type Cx = [f32; 2];

/// Everything one channel keeps between frames.
struct Chan {
    /// last 320 input samples, oldest first
    ana: [f32; 320],
    /// analysis output `[buffer][slot][band]`, double buffered by `ypos`
    w: Box<[[[Cx; 32]; SLOTS]; 2]>,
    /// HF adjuster output `[buffer][slot][band]`
    y: Box<[[[Cx; 64]; 38]; 2]>,
    ypos: usize,
    /// synthesis delay line, newest block first
    syn: [f32; 1280],
    /// chirp factors
    bw: [f32; 5],
    index_noise: usize,
    index_sine: usize,
    g_temp: [[f32; 48]; 42],
    q_temp: [[f32; 48]; 42],
    /// sinusoid flags per envelope + 1 (row 0: last envelope of the last frame)
    s_idx: [[bool; 48]; 8],
}

impl Chan {
    fn new() -> Chan {
        Chan {
            ana: [0.0; 320],
            w: Box::new([[[[0.0; 2]; 32]; SLOTS]; 2]),
            y: Box::new([[[[0.0; 2]; 64]; 38]; 2]),
            ypos: 0,
            syn: [0.0; 1280],
            bw: [0.0; 5],
            index_noise: 0,
            index_sine: 0,
            g_temp: [[0.0; 48]; 42],
            q_temp: [[0.0; 48]; 42],
            s_idx: [[false; 48]; 8],
        }
    }
}

/// Scratch space of one frame, shared by both channels.
struct Work {
    x_low: Box<[[Cx; 40]; 32]>,
    x_high: Box<[[Cx; 40]; 64]>,
    alpha0: [Cx; 64],
    alpha1: [Cx; 64],
    /// reconstructed QMF values `[re/im][slot][band]`
    x: Box<[[[f32; 64]; 38]; 2]>,
    e_orig: [[f32; 48]; 8],
    q_map: [[f32; 48]; 8],
    s_map: [[bool; 48]; 8],
    e_curr: [[f32; 48]; 8],
    q_m: [[f32; 48]; 8],
    s_m: [[f32; 48]; 8],
    gain: [[f32; 48]; 8],
}

impl Work {
    fn new() -> Work {
        Work {
            x_low: Box::new([[[0.0; 2]; 40]; 32]),
            x_high: Box::new([[[0.0; 2]; 40]; 64]),
            alpha0: [[0.0; 2]; 64],
            alpha1: [[0.0; 2]; 64],
            x: Box::new([[[0.0; 64]; 38]; 2]),
            e_orig: [[0.0; 48]; 8],
            q_map: [[0.0; 48]; 8],
            s_map: [[false; 48]; 8],
            e_curr: [[0.0; 48]; 8],
            q_m: [[0.0; 48]; 8],
            s_m: [[0.0; 48]; 8],
            gain: [[0.0; 48]; 8],
        }
    }
}

/// The SBR decoder of one stream.
pub struct Sbr {
    channels: usize,
    core_rate: u32,
    ctl: Ctl,
    chan: [Chan; 2],
    work: Work,
    /// SBR data has been seen in this stream: output is at twice the core rate
    engaged: bool,
}

impl Default for Sbr {
    fn default() -> Self {
        Sbr::new()
    }
}

impl Sbr {
    pub fn new() -> Sbr {
        Sbr { channels: 0, core_rate: 0, ctl: Ctl::new(1, 0), chan: [Chan::new(), Chan::new()], work: Work::new(), engaged: false }
    }

    /// Forgets all state (after a seek or a config change).
    pub fn reset(&mut self) {
        *self = Sbr::new();
    }

    /// Forgets the signal history (after a seek) but keeps the stream's SBR
    /// header and tables, which a file sends only once.
    pub fn flush(&mut self) {
        self.chan = [Chan::new(), Chan::new()];
        self.work = Work::new();
    }

    /// True once the stream has shown valid SBR data; `process` then keeps
    /// producing twice the core rate.
    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Runs SBR for one access unit. `block` is the raw data block, `pcm` its
    /// decoded AAC-LC core (interleaved S16, `CORE_FRAME` frames, or empty
    /// when the packet was corrupt). Returns false, leaving `out` empty, while
    /// the stream has not shown SBR data: the caller then uses `pcm` as is.
    /// Otherwise `out` receives the interleaved S16 at twice the core rate.
    pub fn process(&mut self, block: &[u8], pcm: &[i16], channels: usize, core_rate: u32, out: &mut Vec<i16>) -> bool {
        out.clear();
        if !(1..=2).contains(&channels) || !supported(core_rate) {
            return false;
        }
        if (self.channels, self.core_rate) != (channels, core_rate) {
            *self = Sbr::new();
            self.channels = channels;
            self.core_rate = core_rate;
            self.ctl = Ctl::new(channels, core_rate);
        }
        if !pcm.is_empty() && pcm.len() != CORE_FRAME * channels {
            return false;
        }
        self.ctl.kx[0] = self.ctl.kx[1];
        self.ctl.m[0] = self.ctl.m[1];
        let mut got_data = false;
        for (at, bits, crc) in fill_candidates(block) {
            let mut trial = self.ctl.clone();
            match trial.parse(block, at, bits, crc) {
                Some(true) => {
                    self.ctl = trial;
                    got_data = true;
                    break;
                }
                _ => {}
            }
        }
        if got_data && !self.engaged {
            self.engaged = true;
        }
        if !self.engaged {
            return false;
        }
        if !got_data {
            self.ctl.turnoff();
        }
        if pcm.is_empty() {
            return true;
        }
        if self.ctl.reset {
            for c in &mut self.chan {
                c.index_noise = 0;
            }
        }
        self.render(pcm, out);
        true
    }

    /// QMF analysis, SBR synthesis of the high band and QMF synthesis of
    /// one frame (the part after the bitstream is parsed).
    fn render(&mut self, pcm: &[i16], out: &mut Vec<i16>) {
        let nch = self.channels;
        out.resize(2 * CORE_FRAME * nch, 0);
        for c in 0..nch {
            analysis(&mut self.chan[c], pcm, nch, c);
            lf_gen(&self.ctl, &self.chan[c], &mut self.work.x_low);
            self.chan[c].ypos ^= 1;
            if self.ctl.start {
                self.high_band(c);
            }
            x_gen(&self.ctl, &self.ctl.data[c], &self.chan[c], &self.work.x_low, &mut self.work.x);
            let kmax = ((self.ctl.kx[1] + self.ctl.m[1]).max(self.ctl.kx[0] + self.ctl.m[0]).max(32) as usize).min(64);
            synthesis(&mut self.chan[c], &self.work.x, kmax, out, nch, c);
        }
    }

    /// HF generation and adjustment of channel `c` into its `Y` buffer.
    fn high_band(&mut self, c: usize) {
        let (ctl, ch, w) = (&self.ctl, &mut self.chan[c], &mut self.work);
        let d = &ctl.data[c];
        inverse_filter(&w.x_low, ctl.k[0] as usize, &mut w.alpha0, &mut w.alpha1);
        chirp(ctl, d, ch);
        let ok = hf_gen(ctl, d, w, ch) && mapping(ctl, ch, d, w);
        if ok {
            env_estimate(ctl, d, w);
            gain_calc(ctl, d, w, ch);
            hf_assemble(ctl, d, w, ch);
        } else {
            self.ctl.turnoff();
        }
    }
}

// ------------------------------------------------------------ QMF banks

/// 64-band complex QMF analysis (4.6.18.4.1) of channel `c`'s 1024 samples
/// into `w[ypos]`.
fn analysis(ch: &mut Chan, pcm: &[i16], nch: usize, c: usize) {
    let t = trig();
    for slot in 0..SLOTS {
        ch.ana.copy_within(32.., 0);
        for j in 0..32 {
            ch.ana[288 + j] = pcm[(slot * 32 + j) * nch + c] as f32;
        }
        // z[i] = x[i] * window[2i] with x newest first, then the 5-fold sum
        let mut u = [0f32; 64];
        for (n, un) in u.iter_mut().enumerate() {
            let mut s = 0.0f32;
            for j in 0..5 {
                let i = n + 64 * j;
                s += QMF_WINDOW[2 * i] * ch.ana[319 - i];
            }
            *un = s;
        }
        for k in 0..32 {
            let (cs, sn) = (&t.ana_cos[k * 64..k * 64 + 64], &t.ana_sin[k * 64..k * 64 + 64]);
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for n in 0..64 {
                re += u[n] * cs[n];
                im += u[n] * sn[n];
            }
            ch.w[ch.ypos][slot][k] = [re, im];
        }
    }
}

/// 64-band QMF synthesis (4.6.18.4.2) of the frame's 32 slots of `x`,
/// interleaved into `out`.
fn synthesis(ch: &mut Chan, x: &[[[f32; 64]; 38]; 2], kmax: usize, out: &mut [i16], nch: usize, c: usize) {
    let t = trig();
    for slot in 0..SLOTS {
        ch.syn.copy_within(0..1280 - 128, 128);
        let (xr, xi) = (&x[0][slot], &x[1][slot]);
        for n in 0..128 {
            let (cs, sn) = (&t.syn_cos[n * 64..n * 64 + kmax], &t.syn_sin[n * 64..n * 64 + kmax]);
            let mut a = 0.0f32;
            for k in 0..kmax {
                a += xr[k] * cs[k] - xi[k] * sn[k];
            }
            ch.syn[n] = a;
        }
        for n in 0..64 {
            let mut s = 0.0f32;
            for j in 0..5 {
                s += ch.syn[256 * j + n] * QMF_WINDOW[128 * j + n];
                s += ch.syn[256 * j + 192 + n] * QMF_WINDOW[128 * j + 64 + n];
            }
            out[(slot * 64 + n) * nch + c] = s.round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// The low band as seen by the HF generator: 8 slots of history from the
/// previous frame plus the new frame (4.6.18.6.1).
fn lf_gen(ctl: &Ctl, ch: &Chan, x_low: &mut [[Cx; 40]; 32]) {
    for row in x_low.iter_mut() {
        *row = [[0.0; 2]; 40];
    }
    let (cur, prev) = (ch.ypos, 1 - ch.ypos);
    for k in 0..ctl.kx[1] as usize {
        for i in T_HF_GEN..SLOTS + T_HF_GEN {
            x_low[k][i] = ch.w[cur][i - T_HF_GEN][k];
        }
    }
    for k in 0..ctl.kx[0] as usize {
        for i in 0..T_HF_GEN {
            x_low[k][i] = ch.w[prev][i + SLOTS - T_HF_GEN][k];
        }
    }
}

/// Merges low band, generated high band and the previous frame's tail into
/// the QMF values to synthesise (4.6.18.7).
fn x_gen(ctl: &Ctl, d: &Bs, ch: &Chan, x_low: &[[Cx; 40]; 32], x: &mut [[[f32; 64]; 38]; 2]) {
    let (y0, y1) = (&ch.y[1 - ch.ypos], &ch.y[ch.ypos]);
    let i_temp = (2 * d.t_env_old - SLOTS as i32).max(0) as usize;
    *x = [[[0.0; 64]; 38]; 2];
    let (kx0, kx1) = (ctl.kx[0] as usize, ctl.kx[1] as usize);
    let (m0, m1) = (ctl.m[0] as usize, ctl.m[1] as usize);
    for k in 0..kx0 {
        for i in 0..i_temp {
            x[0][i][k] = x_low[k][i + T_HF_ADJ][0];
            x[1][i][k] = x_low[k][i + T_HF_ADJ][1];
        }
    }
    for k in kx0..kx0 + m0 {
        for i in 0..i_temp {
            x[0][i][k] = y0[i + SLOTS][k][0];
            x[1][i][k] = y0[i + SLOTS][k][1];
        }
    }
    for k in 0..kx1 {
        for i in i_temp..38 {
            x[0][i][k] = x_low[k][i + T_HF_ADJ][0];
            x[1][i][k] = x_low[k][i + T_HF_ADJ][1];
        }
    }
    for k in kx1..kx1 + m1 {
        for i in i_temp..SLOTS {
            x[0][i][k] = y1[i][k][0];
            x[1][i][k] = y1[i][k][1];
        }
    }
}

// ------------------------------------------------------- HF generation

/// Complex LPC coefficients of every low band subband (4.6.18.6.2).
fn inverse_filter(x_low: &[[Cx; 40]; 32], k0: usize, alpha0: &mut [Cx; 64], alpha1: &mut [Cx; 64]) {
    for k in 0..k0.min(32) {
        let phi = autocorrelate(&x_low[k]);
        let dk = phi[2][1][0] * phi[1][0][0] - (phi[1][1][0] * phi[1][1][0] + phi[1][1][1] * phi[1][1][1]) / 1.000001;
        if dk == 0.0 {
            alpha1[k] = [0.0, 0.0];
        } else {
            let re = phi[0][0][0] * phi[1][1][0] - phi[0][0][1] * phi[1][1][1] - phi[0][1][0] * phi[1][0][0];
            let im = phi[0][0][0] * phi[1][1][1] + phi[0][0][1] * phi[1][1][0] - phi[0][1][1] * phi[1][0][0];
            alpha1[k] = [re / dk, im / dk];
        }
        if phi[1][0][0] == 0.0 {
            alpha0[k] = [0.0, 0.0];
        } else {
            let re = phi[0][0][0] + alpha1[k][0] * phi[1][1][0] + alpha1[k][1] * phi[1][1][1];
            let im = phi[0][0][1] + alpha1[k][1] * phi[1][1][0] - alpha1[k][0] * phi[1][1][1];
            alpha0[k] = [-re / phi[1][0][0], -im / phi[1][0][0]];
        }
        let mag = |a: Cx| a[0] * a[0] + a[1] * a[1];
        if mag(alpha1[k]) >= 16.0 || mag(alpha0[k]) >= 16.0 {
            alpha1[k] = [0.0, 0.0];
            alpha0[k] = [0.0, 0.0];
        }
    }
}

/// Covariance matrix terms `phi[i][j]` of one subband over its 40 slots.
fn autocorrelate(x: &[Cx; 40]) -> [[Cx; 2]; 3] {
    let mut real_sum2 = x[0][0] * x[2][0] + x[0][1] * x[2][1];
    let mut imag_sum2 = x[0][0] * x[2][1] - x[0][1] * x[2][0];
    let (mut real_sum1, mut imag_sum1, mut real_sum0) = (0.0f32, 0.0f32, 0.0f32);
    for i in 1..38 {
        real_sum0 += x[i][0] * x[i][0] + x[i][1] * x[i][1];
        real_sum1 += x[i][0] * x[i + 1][0] + x[i][1] * x[i + 1][1];
        imag_sum1 += x[i][0] * x[i + 1][1] - x[i][1] * x[i + 1][0];
        real_sum2 += x[i][0] * x[i + 2][0] + x[i][1] * x[i + 2][1];
        imag_sum2 += x[i][0] * x[i + 2][1] - x[i][1] * x[i + 2][0];
    }
    let mut phi = [[[0.0f32; 2]; 2]; 3];
    phi[0][1] = [real_sum2, imag_sum2];
    phi[2][1][0] = real_sum0 + x[0][0] * x[0][0] + x[0][1] * x[0][1];
    phi[1][0][0] = real_sum0 + x[38][0] * x[38][0] + x[38][1] * x[38][1];
    phi[1][1] = [real_sum1 + x[0][0] * x[1][0] + x[0][1] * x[1][1], imag_sum1 + x[0][0] * x[1][1] - x[0][1] * x[1][0]];
    phi[0][0] = [real_sum1 + x[38][0] * x[39][0] + x[38][1] * x[39][1], imag_sum1 + x[38][0] * x[39][1] - x[38][1] * x[39][0]];
    phi
}

/// Chirp factors from the inverse filtering modes (4.6.18.6.2).
fn chirp(ctl: &Ctl, d: &Bs, ch: &mut Chan) {
    const BW: [f32; 4] = [0.0, 0.75, 0.9, 0.98];
    for i in 0..ctl.n_q {
        let mut new_bw = if d.invf[0][i] + d.invf[1][i] == 1 { 0.6 } else { BW[d.invf[0][i]] };
        if new_bw < ch.bw[i] {
            new_bw = 0.75 * new_bw + 0.25 * ch.bw[i];
        } else {
            new_bw = 0.90625 * new_bw + 0.09375 * ch.bw[i];
        }
        ch.bw[i] = if new_bw < 0.015625 { 0.0 } else { new_bw };
    }
}

/// Copies the low band up into the patches, predicting each HF slot from
/// the two before it (4.6.18.6.3). False if a subband has no noise band.
fn hf_gen(ctl: &Ctl, d: &Bs, w: &mut Work, ch: &Chan) -> bool {
    for row in w.x_high.iter_mut() {
        *row = [[0.0; 2]; 40];
    }
    let (start, end) = (2 * d.t_env[0] as usize, 2 * d.t_env[d.num_env] as usize);
    let mut g = 0usize;
    let mut k = ctl.kx[1] as usize;
    for j in 0..ctl.num_patches {
        for x in 0..ctl.patch_num_subbands[j] as usize {
            let p = (ctl.patch_start_subband[j] + x as i32) as usize;
            if p >= 32 || k >= 64 {
                return false;
            }
            while g <= ctl.n_q && k as i32 >= ctl.f_noise[g] {
                g += 1;
            }
            if g == 0 {
                return false;
            }
            g -= 1;
            let bw = ch.bw.get(g).copied().unwrap_or(0.0);
            let (a0, a1) = (w.alpha0[p], w.alpha1[p]);
            let a = [a1[0] * bw * bw, a1[1] * bw * bw, a0[0] * bw, a0[1] * bw];
            let lo = &w.x_low[p];
            let hi = &mut w.x_high[k];
            for i in start..end {
                let (l2, l1, l0) = (lo[i], lo[i + 1], lo[i + 2]);
                hi[i + T_HF_ADJ] = [l2[0] * a[0] - l2[1] * a[1] + l1[0] * a[2] - l1[1] * a[3] + l0[0], l2[1] * a[0] + l2[0] * a[1] + l1[1] * a[2] + l1[0] * a[3] + l0[1]];
            }
            k += 1;
        }
    }
    true
}

// ------------------------------------------------- envelope adjustment

/// Maps envelope, noise and sinusoid data onto the individual HF subbands
/// (4.6.18.7.3). False when the tables do not match the frame.
fn mapping(ctl: &Ctl, ch: &mut Chan, d: &Bs, w: &mut Work) -> bool {
    let kx1 = ctl.kx[1];
    for row in ch.s_idx[1..].iter_mut() {
        *row = [false; 48];
    }
    for e in 0..d.num_env {
        let res = d.freq_res[e + 1];
        let ilim = ctl.n[res];
        let table: &[i32] = if res == 1 { &ctl.f_high } else { &ctl.f_low };
        if kx1 != table[0] {
            return false;
        }
        for i in 0..ilim {
            for m in table[i]..table[i + 1] {
                w.e_orig[e][(m - kx1) as usize] = d.env[e + 1][i];
            }
        }
        let k = (d.num_noise > 1 && d.t_env[e] >= d.t_q[1]) as usize;
        for i in 0..ctl.n_q {
            for m in ctl.f_noise[i]..ctl.f_noise[i + 1] {
                w.q_map[e][(m - kx1) as usize] = d.noise[k + 1][i];
            }
        }
        for i in 0..ctl.n[1] {
            if d.add_harmonic_flag {
                let mid = ((ctl.f_high[i] + ctl.f_high[i + 1]) >> 1) - kx1;
                let mid = mid as usize;
                ch.s_idx[e + 1][mid] = d.add_harmonic[i] && (e as i32 >= d.e_a[1] || ch.s_idx[0][mid]);
            }
        }
        for i in 0..ilim {
            let present = (table[i]..table[i + 1]).any(|m| ch.s_idx[e + 1][(m - kx1) as usize]);
            for m in table[i]..table[i + 1] {
                w.s_map[e][(m - kx1) as usize] = present;
            }
        }
    }
    ch.s_idx[0] = ch.s_idx[d.num_env];
    true
}

/// Mean energy of the generated HF signal per envelope (4.6.18.7.4).
fn env_estimate(ctl: &Ctl, d: &Bs, w: &mut Work) {
    let kx1 = ctl.kx[1] as usize;
    let m1 = ctl.m[1] as usize;
    let sum_sq = |row: &[Cx; 40], lo: usize, hi: usize| -> f32 { row[lo..hi].iter().map(|c| c[0] * c[0] + c[1] * c[1]).sum() };
    for e in 0..d.num_env {
        let ilb = d.t_env[e] as usize * 2 + T_HF_ADJ;
        let iub = d.t_env[e + 1] as usize * 2 + T_HF_ADJ;
        if ilb >= 40 || iub > 40 {
            return;
        }
        if ctl.interpol_freq {
            let recip = 0.5 / (d.t_env[e + 1] - d.t_env[e]) as f32;
            for m in 0..m1 {
                w.e_curr[e][m] = sum_sq(&w.x_high[m + kx1], ilb, iub) * recip;
            }
        } else {
            let res = d.freq_res[e + 1];
            let table: &[i32] = if res == 1 { &ctl.f_high } else { &ctl.f_low };
            let env_size = 2 * (d.t_env[e + 1] - d.t_env[e]);
            for p in 0..ctl.n[res] {
                let mut sum = 0.0f32;
                for k in table[p]..table[p + 1] {
                    sum += sum_sq(&w.x_high[k as usize], ilb, iub);
                }
                sum /= (env_size * (table[p + 1] - table[p])) as f32;
                for k in table[p]..table[p + 1] {
                    w.e_curr[e][k as usize - kx1] = sum;
                }
            }
        }
    }
}

/// Gain, limiter and boost per subband and envelope (4.6.18.7.5).
fn gain_calc(ctl: &Ctl, d: &Bs, w: &mut Work, ch: &Chan) {
    const LIMGAIN: [f32; 4] = [0.70795, 1.0, 1.41254, 10_000_000_000.0];
    let kx1 = ctl.kx[1];
    for e in 0..d.num_env {
        let delta = !(e as i32 == d.e_a[1] || e as i32 == d.e_a[0]);
        for k in 0..ctl.n_lim {
            let band = (ctl.f_lim[k] - kx1) as usize..(ctl.f_lim[k + 1] - kx1) as usize;
            for m in band.clone() {
                let temp = w.e_orig[e][m] / (1.0 + w.q_map[e][m]);
                w.q_m[e][m] = (temp * w.q_map[e][m]).sqrt();
                w.s_m[e][m] = if ch.s_idx[e + 1][m] { temp.sqrt() } else { 0.0 };
                w.gain[e][m] = if !w.s_map[e][m] {
                    (w.e_orig[e][m] / ((1.0 + w.e_curr[e][m]) * (1.0 + w.q_map[e][m] * delta as i32 as f32))).sqrt()
                } else {
                    (w.e_orig[e][m] * w.q_map[e][m] / ((1.0 + w.e_curr[e][m]) * (1.0 + w.q_map[e][m]))).sqrt()
                };
                w.gain[e][m] += f32::MIN_POSITIVE;
            }
            let (mut s0, mut s1) = (0.0f32, 0.0f32);
            for m in band.clone() {
                s0 += w.e_orig[e][m];
                s1 += w.e_curr[e][m];
            }
            let gain_max = (LIMGAIN[ctl.limiter_gains] * ((f32::EPSILON + s0) / (f32::EPSILON + s1)).sqrt()).min(100_000.0);
            for m in band.clone() {
                let q_m_max = w.q_m[e][m] * gain_max / w.gain[e][m];
                w.q_m[e][m] = w.q_m[e][m].min(q_m_max);
                w.gain[e][m] = w.gain[e][m].min(gain_max);
            }
            let (mut s0, mut s1) = (0.0f32, 0.0f32);
            for m in band.clone() {
                s0 += w.e_orig[e][m];
                s1 += w.e_curr[e][m] * w.gain[e][m] * w.gain[e][m] + w.s_m[e][m] * w.s_m[e][m] + if delta && w.s_m[e][m] == 0.0 { w.q_m[e][m] * w.q_m[e][m] } else { 0.0 };
            }
            let boost = (((f32::EPSILON + s0) / (f32::EPSILON + s1)).sqrt()).min(1.584_893_2);
            for m in band {
                w.gain[e][m] *= boost;
                w.q_m[e][m] *= boost;
                w.s_m[e][m] *= boost;
            }
        }
    }
}

/// Assembles the adjusted HF signal: smoothed gains on the generated
/// subbands, plus noise or added sinusoids (4.6.18.7.6).
fn hf_assemble(ctl: &Ctl, d: &Bs, w: &mut Work, ch: &mut Chan) {
    const H_SMOOTH: [f32; 5] = [0.333_333_33, 0.301_502_83, 0.218_169_5, 0.115_163_83, 0.031_830_5];
    let h_sl = if ctl.smoothing_mode { 0 } else { 4 };
    let kx = ctl.kx[1] as usize;
    let m_max = ctl.m[1] as usize;
    let t0 = 2 * d.t_env[0] as usize;
    if ctl.reset {
        for i in 0..h_sl {
            ch.g_temp[i + t0][..m_max].copy_from_slice(&w.gain[0][..m_max]);
            ch.q_temp[i + t0][..m_max].copy_from_slice(&w.q_m[0][..m_max]);
        }
    } else if h_sl > 0 {
        let old = 2 * d.t_env_old as usize;
        for i in 0..4 {
            ch.g_temp[i + t0] = ch.g_temp[i + old];
            ch.q_temp[i + t0] = ch.q_temp[i + old];
        }
    }
    for e in 0..d.num_env {
        for i in 2 * d.t_env[e] as usize..2 * d.t_env[e + 1] as usize {
            ch.g_temp[h_sl + i][..m_max].copy_from_slice(&w.gain[e][..m_max]);
            ch.q_temp[h_sl + i][..m_max].copy_from_slice(&w.q_m[e][..m_max]);
        }
    }
    let y = &mut ch.y[ch.ypos];
    let (mut index_noise, mut index_sine) = (ch.index_noise, ch.index_sine);
    for e in 0..d.num_env {
        let plain = e as i32 != d.e_a[0] && e as i32 != d.e_a[1];
        for i in 2 * d.t_env[e] as usize..2 * d.t_env[e + 1] as usize {
            let mut g_filt = [0f32; 48];
            let mut q_filt = [0f32; 48];
            if h_sl > 0 && plain {
                for m in 0..m_max {
                    for (j, h) in H_SMOOTH.iter().enumerate().take(h_sl + 1) {
                        g_filt[m] += ch.g_temp[i + h_sl - j][m] * h;
                        q_filt[m] += ch.q_temp[i + h_sl - j][m] * h;
                    }
                }
            } else {
                g_filt[..m_max].copy_from_slice(&ch.g_temp[i + h_sl][..m_max]);
                q_filt[..m_max].copy_from_slice(&ch.q_temp[i + h_sl][..m_max]);
            }
            for m in 0..m_max {
                let xh = w.x_high[kx + m][i + T_HF_ADJ];
                y[i][kx + m] = [xh[0] * g_filt[m], xh[1] * g_filt[m]];
            }
            if plain {
                let phi_sign = 1.0 - 2.0 * (kx & 1) as f32;
                let (p0, mut p1) = match index_sine {
                    0 => (1.0, 0.0),
                    1 => (0.0, phi_sign),
                    2 => (-1.0, 0.0),
                    _ => (0.0, -phi_sign),
                };
                let mut noise = index_noise;
                for m in 0..m_max {
                    noise = (noise + 1) & 0x1ff;
                    let s = w.s_m[e][m];
                    if s != 0.0 {
                        y[i][kx + m][0] += s * p0;
                        y[i][kx + m][1] += s * p1;
                    } else {
                        y[i][kx + m][0] += q_filt[m] * NOISE[noise][0];
                        y[i][kx + m][1] += q_filt[m] * NOISE[noise][1];
                    }
                    p1 = -p1;
                }
            } else {
                let idx = index_sine & 1;
                let a = 1 - ((index_sine + (kx & 1)) & 2) as i32;
                let b = if idx == 0 { a } else { -a };
                for m in (0..m_max).step_by(2) {
                    y[i][kx + m][idx] += w.s_m[e][m] * a as f32;
                    if m + 1 < m_max {
                        y[i][kx + m + 1][idx] += w.s_m[e][m + 1] * b as f32;
                    }
                }
            }
            index_noise = (index_noise + m_max) & 0x1ff;
            index_sine = (index_sine + 1) & 3;
        }
    }
    ch.index_noise = index_noise;
    ch.index_sine = index_sine;
}

#[cfg(test)]
mod tests {
    use super::super::aacstream::{adts_frame, Parsed};
    use super::*;

    /// libfdk-aac HE-AAC, 44.1 kHz stereo, 22.05 kHz core (see the
    /// aacstream test that decodes it end to end).
    const HE_TONE: &[u8] = include_bytes!("testdata/he_aac_tone.adts");

    fn fixture_frames() -> Vec<Vec<u8>> {
        let (mut off, mut out) = (0, Vec::new());
        while off < HE_TONE.len() {
            match adts_frame(&HE_TONE[off..], true) {
                Parsed::Frame(f) => {
                    off += f.consumed;
                    out.push(f.payload);
                }
                Parsed::Skip(n) => off += n,
                Parsed::NeedMore => break,
            }
        }
        out
    }

    /// Appends `n` bits of `v`.
    fn put(bits: &mut Vec<bool>, v: u32, n: usize) {
        for i in (0..n).rev() {
            bits.push(v >> i & 1 == 1);
        }
    }

    fn to_bytes(bits: &[bool]) -> Vec<u8> {
        bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &x)| b | (x as u8) << (7 - i))).collect()
    }

    /// `prefix` junk bits, an SBR fill element of `payload` bytes (type
    /// nibble included) and optionally a padding fill element, then
    /// `ID_END` and the byte alignment.
    fn block_with_fill(prefix: usize, ext_type: u32, payload: usize, padding_fill: bool) -> (Vec<u8>, usize) {
        let mut bits = Vec::new();
        for i in 0..prefix {
            bits.push(i % 3 == 0);
        }
        put(&mut bits, 6, 3);
        if payload < 15 {
            put(&mut bits, payload as u32, 4);
        } else {
            put(&mut bits, 15, 4);
            put(&mut bits, (payload - 14) as u32, 8);
        }
        let payload_at = bits.len();
        put(&mut bits, ext_type, 4);
        for i in 0..payload * 8 - 4 {
            bits.push(i % 5 < 2);
        }
        if padding_fill {
            put(&mut bits, 6, 3);
            put(&mut bits, 3, 4);
            put(&mut bits, 1, 4); // EXT_FILL_DATA
            for _ in 0..20 {
                bits.push(true);
            }
            put(&mut bits, 6, 3);
            put(&mut bits, 0, 4); // an empty fill element
        }
        put(&mut bits, 7, 3);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        (to_bytes(&bits), payload_at)
    }

    #[test]
    fn huffman_codes_are_canonical() {
        // t_huffman_env_1_5dB: 00 -> 0, 01 -> -1, 100 -> +1, 101 -> -2
        let data = [0b0001_1001, 0b0100_0000];
        let mut r = Bits::new(&data, 0, 16);
        let h = huff(T_ENV_1_5);
        let got: Vec<i32> = (0..4).map(|_| h.read(&mut r).unwrap()).collect();
        assert_eq!(got, [0, -1, 1, -2]);
        assert_eq!(r.pos, 10);
    }

    #[test]
    fn every_huffman_table_is_complete_and_decodes_each_of_its_codes() {
        for (t, table) in HUFF.iter().enumerate() {
            // a transcription error in a table breaks the Kraft sum
            let kraft: f64 = table.iter().map(|&(_, len)| 0.5f64.powi(len as i32)).sum();
            assert!((kraft - 1.0).abs() < 1e-9, "table {t}: {kraft}");
            // encode every symbol with the canonical code and read it back
            let h = huff(t);
            let mut code = 0u32;
            let mut prev = table[0].1 as u32;
            for (i, &(sym, len)) in table.iter().enumerate() {
                let len = len as u32;
                if i > 0 {
                    code = (code + 1) << (len - prev);
                }
                prev = len;
                let mut bits = Vec::new();
                put(&mut bits, code, len as usize);
                let bytes = to_bytes(&bits);
                let mut r = Bits::new(&bytes, 0, bytes.len() * 8);
                assert_eq!(h.read(&mut r), Some(sym as i32 + HUFF_OFFSET[t]), "table {t} symbol {sym}");
                assert_eq!(r.pos, len as usize);
            }
        }
    }

    #[test]
    fn fill_elements_are_found_from_the_back() {
        // plain
        let (block, at) = block_with_fill(21, 0xd, 4, false);
        assert!(fill_candidates(&block).contains(&(at + 4, 28, false)));
        // with the encoder's padding behind it
        let (block, at) = block_with_fill(37, 0xd, 9, true);
        assert!(fill_candidates(&block).contains(&(at + 4, 68, false)));
        // CRC variant
        let (block, at) = block_with_fill(5, 0xe, 6, true);
        assert!(fill_candidates(&block).contains(&(at + 4, 44, true)));
        // escaped length
        let (block, at) = block_with_fill(13, 0xd, 20, false);
        assert!(fill_candidates(&block).contains(&(at + 4, 156, false)));
        // other extension types are not SBR
        let (block, _) = block_with_fill(8, 0x1, 9, false);
        assert!(fill_candidates(&block).iter().all(|c| c.1 != 68));
        assert!(fill_candidates(&[0u8; 40]).is_empty());
        assert!(fill_candidates(&[]).is_empty());
    }

    #[test]
    fn every_fixture_frame_has_an_sbr_element() {
        let frames = fixture_frames();
        assert_eq!(frames.len(), 25);
        for (i, f) in frames.iter().enumerate() {
            assert!(!fill_candidates(f).is_empty(), "frame {i}");
        }
        assert_eq!(fill_candidates(&frames[0])[0], (72, 100, false));
    }

    #[test]
    fn the_first_header_builds_the_expected_tables() {
        let frames = fixture_frames();
        let mut c = Ctl::new(2, 22050);
        let (at, bits, crc) = fill_candidates(&frames[0])[0];
        assert_eq!(c.parse(&frames[0], at, bits, crc), Some(true));
        // start 12, stop 9, xover 0, freq_scale 2, alter_scale 1, noise bands 3
        assert_eq!(c.params, Some([12, 9, 0, 2, 1, 3]));
        assert_eq!(c.k, [23, 47, 47]);
        assert_eq!((c.kx[1], c.m[1]), (23, 24));
        assert_eq!((c.n, c.n_q, c.n_lim), ([5, 10], 3, 3));
        assert_eq!(c.f_high[..=10], [23, 24, 26, 28, 30, 32, 35, 38, 41, 44, 47]);
        assert_eq!(c.f_low[..=5], [23, 26, 30, 35, 41, 47]);
        assert_eq!(c.f_noise[..=3], [23, 26, 35, 47]);
        assert_eq!(c.f_lim[..=3], [23, 30, 44, 47]);
        assert_eq!(c.num_patches, 2);
        assert_eq!(c.patch_num_subbands[..2], [21, 3]);
        assert_eq!(c.patch_start_subband[..2], [1, 20]);
        assert!(c.start && c.reset, "the first header starts and resets");
        assert_eq!((c.limiter_bands, c.limiter_gains, c.interpol_freq, c.smoothing_mode), (2, 2, true, true));
    }

    #[test]
    fn every_fixture_frame_parses_in_sequence() {
        let mut c = Ctl::new(2, 22050);
        for (i, f) in fixture_frames().iter().enumerate() {
            c.kx[0] = c.kx[1];
            c.m[0] = c.m[1];
            let mut ok = false;
            for (at, bits, crc) in fill_candidates(f) {
                let mut trial = c.clone();
                if trial.parse(f, at, bits, crc) == Some(true) {
                    c = trial;
                    ok = true;
                    break;
                }
            }
            assert!(ok, "frame {i}");
            assert_eq!(c.reset, i == 0, "only the first header changes the tables");
            for d in &c.data {
                assert!((1..=5).contains(&d.num_env), "frame {i}");
            }
        }
    }

    #[test]
    fn a_stream_without_sbr_data_stays_out_of_the_way() {
        let mut s = Sbr::new();
        let mut out = vec![1i16];
        assert!(!s.process(&[1, 2, 3], &vec![0i16; 2 * CORE_FRAME], 2, 22050, &mut out));
        assert!(out.is_empty());
        assert!(!s.engaged());
        // unsupported core rate or channel count
        assert!(!s.process(&[1, 2, 3], &vec![0i16; CORE_FRAME], 1, 7350, &mut out));
        assert!(!s.process(&[1, 2, 3], &vec![0i16; 6 * CORE_FRAME], 6, 22050, &mut out));
        assert!(supported(22050) && supported(24000) && supported(16000));
        assert!(!supported(7350) && !supported(100_000));
    }

    #[test]
    fn the_qmf_chain_upsamples_a_tone_without_distortion() {
        // no SBR data: the core goes through analysis and synthesis only
        let mut s = Sbr::new();
        s.channels = 1;
        s.core_rate = 24000;
        s.ctl = Ctl::new(1, 24000);
        let mut out = Vec::new();
        let mut all = Vec::new();
        let w = 2.0 * std::f64::consts::PI * 1000.0 / 24000.0;
        for f in 0..12 {
            let pcm: Vec<i16> = (0..CORE_FRAME).map(|i| (8000.0 * (w * (f * CORE_FRAME + i) as f64).sin()).round() as i16).collect();
            s.render(&pcm, &mut out);
            assert_eq!(out.len(), 2 * CORE_FRAME);
            all.extend_from_slice(&out);
        }
        // 6 frames of 2048 samples at 48 kHz: a whole number of 1 kHz periods
        let tail = &all[all.len() / 2..];
        let w = 2.0 * std::f64::consts::PI * 1000.0 / 48000.0;
        let (mut sc, mut ss) = (0.0f64, 0.0f64);
        for (i, &x) in tail.iter().enumerate() {
            sc += x as f64 * (w * i as f64).cos();
            ss += x as f64 * (w * i as f64).sin();
        }
        let n = tail.len() as f64;
        let (c, sn) = (2.0 * sc / n, 2.0 * ss / n);
        let amp = c.hypot(sn);
        assert!((amp - 8000.0).abs() < 160.0, "amplitude {amp}");
        let resid = (tail.iter().enumerate().map(|(i, &x)| (x as f64 - c * (w * i as f64).cos() - sn * (w * i as f64).sin()).powi(2)).sum::<f64>() / n).sqrt();
        assert!(resid < 0.02 * amp, "residual {resid} against {amp}");
    }

    #[test]
    fn flush_keeps_the_stream_header_but_not_the_signal() {
        let frames = fixture_frames();
        let mut s = Sbr::new();
        s.channels = 2;
        s.core_rate = 22050;
        s.ctl = Ctl::new(2, 22050);
        let (at, bits, crc) = fill_candidates(&frames[0])[0];
        assert_eq!(s.ctl.parse(&frames[0], at, bits, crc), Some(true));
        s.chan[0].ana[5] = 1.0;
        s.flush();
        assert_eq!(s.chan[0].ana[5], 0.0);
        assert!(s.ctl.start);
        assert_eq!(s.ctl.kx[1], 23);
    }
}
