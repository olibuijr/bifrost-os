//! Audio files (task 13): reading and writing the linear PCM containers WAV (RIFF, including the extensible
//! header), AIFF / AIFF-C (big endian, with the `sowt`, `fl32` and `fl64` variants) and CAF (the `lpcm` format).
//! The compressed decoders (MP3, AAC, FLAC, Vorbis, MIDI rendering) stay in `springboard/src/media` until their
//! callers switch (see docs/audio/13-toolbox.md); this module takes the container side and the PCM codec.
//!
//! Files are read from memory ([`probe`], [`decode`]) or a path ([`read`]); a [`FileReader`] pulls frames out of a
//! probed file without decoding it whole. Malformed bytes give [`AudioError::Protocol`], valid but unsupported
//! combinations give [`AudioError::BadValue`], file system failures give [`AudioError::Io`].

use crate::converter::{decode_samples, encode_samples, SampleFormat};
use bifrost_audio_types::AudioError;
use std::path::Path;

/// The container of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Wav,
    Aiff,
    Caf,
}

impl Container {
    /// The container for a file extension (any case), if it is one this module handles.
    pub fn from_extension(path: &Path) -> Option<Container> {
        let e = path.extension()?.to_str()?.to_ascii_lowercase();
        match e.as_str() {
            "wav" | "wave" => Some(Container::Wav),
            "aif" | "aiff" | "aifc" => Some(Container::Aiff),
            "caf" => Some(Container::Caf),
            _ => None,
        }
    }
}

/// What a file says about its audio, without the samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileInfo {
    pub container: Container,
    pub rate: u32,
    pub channels: u16,
    pub sample: SampleFormat,
    pub big_endian: bool,
    pub frames: u64,
    /// Where the sample bytes start in the file and how many there are (a whole number of frames).
    pub data_offset: usize,
    pub data_len: usize,
}

/// A decoded file: interleaved floats.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioFile {
    pub container: Container,
    pub rate: u32,
    pub channels: u16,
    /// The encoding the file has (or will be written with).
    pub sample: SampleFormat,
    pub samples: Vec<f32>,
}

impl AudioFile {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u16be(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32be(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64be(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

const BAD: AudioError = AudioError::Protocol;
const UNSUPPORTED: AudioError = AudioError::BadValue;

/// Reads a file's header from memory.
pub fn probe(b: &[u8]) -> Result<FileInfo, AudioError> {
    match b.get(..4) {
        Some(b"RIFF") => probe_wav(b),
        Some(b"FORM") => probe_aiff(b),
        Some(b"caff") => probe_caf(b),
        _ => Err(BAD),
    }
}

fn finish_info(
    container: Container,
    rate: u32,
    channels: u16,
    sample: SampleFormat,
    big_endian: bool,
    data_offset: usize,
    data_len: usize,
) -> Result<FileInfo, AudioError> {
    if rate == 0 || channels == 0 {
        return Err(BAD);
    }
    let frame = sample.bytes() * channels as usize;
    let data_len = data_len / frame * frame;
    Ok(FileInfo { container, rate, channels, sample, big_endian, frames: (data_len / frame) as u64, data_offset, data_len })
}

fn probe_wav(b: &[u8]) -> Result<FileInfo, AudioError> {
    if b.get(8..12) != Some(b"WAVE") {
        return Err(BAD);
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16, Option<u16>)> = None; // tag, channels, rate, bits, ext subformat
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32le(b, pos + 4).ok_or(BAD)? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if size < 16 || body + size > b.len() {
                return Err(BAD);
            }
            let tag = u16le(b, body).ok_or(BAD)?;
            let channels = u16le(b, body + 2).ok_or(BAD)?;
            let rate = u32le(b, body + 4).ok_or(BAD)?;
            let bits = u16le(b, body + 14).ok_or(BAD)?;
            let sub = if tag == 0xFFFE && size >= 26 { u16le(b, body + 24) } else { None };
            fmt = Some((tag, channels, rate, bits, sub));
        } else if id == b"data" {
            let (tag, channels, rate, bits, sub) = fmt.ok_or(BAD)?;
            // streamed files declare 0 or 0xFFFFFFFF: take the rest of the file
            let avail = b.len() - body;
            let len = if size == 0 || size == 0xFFFF_FFFF || size > avail { avail } else { size };
            let kind = if tag == 0xFFFE { sub.ok_or(BAD)? } else { tag };
            let sample = match (kind, bits) {
                (1, 8) => SampleFormat::U8,
                (1, 16) => SampleFormat::S16,
                (1, 24) => SampleFormat::S24,
                (1, 32) => SampleFormat::S32,
                (3, 32) => SampleFormat::F32,
                (3, 64) => SampleFormat::F64,
                _ => return Err(UNSUPPORTED),
            };
            return finish_info(Container::Wav, rate, channels, sample, false, body, len);
        }
        pos = body + size + (size & 1);
    }
    Err(BAD)
}

/// An 80-bit IEEE extended float, as AIFF stores the sample rate.
fn extended_to_f64(b: &[u8]) -> Option<f64> {
    let se = u16be(b, 0)?;
    let mant = u64be(b, 2)?;
    let exp = (se & 0x7FFF) as i32;
    if exp == 0 && mant == 0 {
        return Some(0.0);
    }
    let v = mant as f64 * 2f64.powi(exp - 16383 - 63);
    Some(if se & 0x8000 != 0 { -v } else { v })
}

fn u32_to_extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let e = 31 - rate.leading_zeros();
    let mant = (rate as u64) << (63 - e);
    let biased = (16383 + e) as u16;
    out[0..2].copy_from_slice(&biased.to_be_bytes());
    out[2..10].copy_from_slice(&mant.to_be_bytes());
    out
}

fn probe_aiff(b: &[u8]) -> Result<FileInfo, AudioError> {
    let form = b.get(8..12).ok_or(BAD)?;
    let aifc = form == b"AIFC";
    if form != b"AIFF" && !aifc {
        return Err(BAD);
    }
    let mut pos = 12;
    let mut comm: Option<(u16, u32, SampleFormat, bool)> = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32be(b, pos + 4).ok_or(BAD)? as usize;
        let body = pos + 8;
        if id == b"COMM" {
            if size < 18 || body + size > b.len() {
                return Err(BAD);
            }
            let channels = u16be(b, body).ok_or(BAD)?;
            let bits = u16be(b, body + 6).ok_or(BAD)?;
            let rate = extended_to_f64(&b[body + 8..]).ok_or(BAD)?;
            if !(rate >= 1.0 && rate <= u32::MAX as f64) {
                return Err(BAD);
            }
            let mut comp: &[u8] = b"NONE";
            if aifc {
                comp = b.get(body + 18..body + 22).ok_or(BAD)?;
            }
            let (sample, be) = match (comp, bits) {
                (b"NONE" | b"twos", 8) => (SampleFormat::S8, true),
                (b"NONE" | b"twos", 16) => (SampleFormat::S16, true),
                (b"NONE" | b"twos", 24) => (SampleFormat::S24, true),
                (b"NONE" | b"twos", 32) => (SampleFormat::S32, true),
                (b"sowt", 16) => (SampleFormat::S16, false),
                (b"sowt", 24) => (SampleFormat::S24, false),
                (b"sowt", 32) => (SampleFormat::S32, false),
                (b"fl32" | b"FL32", 32) => (SampleFormat::F32, true),
                (b"fl64" | b"FL64", 64) => (SampleFormat::F64, true),
                _ => return Err(UNSUPPORTED),
            };
            comm = Some((channels, rate.round() as u32, sample, be));
        } else if id == b"SSND" {
            let (channels, rate, sample, be) = comm.ok_or(BAD)?;
            if size < 8 || body + 8 > b.len() {
                return Err(BAD);
            }
            let offset = u32be(b, body).ok_or(BAD)? as usize;
            let start = body + 8 + offset;
            let end = (body + size).min(b.len());
            if start > end {
                return Err(BAD);
            }
            return finish_info(Container::Aiff, rate, channels, sample, be, start, end - start);
        }
        pos = body + size + (size & 1);
    }
    Err(BAD)
}

fn probe_caf(b: &[u8]) -> Result<FileInfo, AudioError> {
    if u16be(b, 4) != Some(1) {
        return Err(UNSUPPORTED);
    }
    let mut pos = 8;
    let mut desc: Option<(u32, u16, SampleFormat, bool)> = None;
    while pos + 12 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u64be(b, pos + 4).ok_or(BAD)? as i64;
        let body = pos + 12;
        if id == b"desc" {
            if size < 32 || body + 32 > b.len() {
                return Err(BAD);
            }
            let rate = f64::from_bits(u64be(b, body).ok_or(BAD)?);
            if b.get(body + 8..body + 12) != Some(b"lpcm") {
                return Err(UNSUPPORTED);
            }
            let flags = u32be(b, body + 12).ok_or(BAD)?;
            let bytes_per_packet = u32be(b, body + 16).ok_or(BAD)?;
            let channels = u32be(b, body + 24).ok_or(BAD)?;
            let bits = u32be(b, body + 28).ok_or(BAD)?;
            if !(rate >= 1.0 && rate <= u32::MAX as f64) || channels == 0 || channels > u16::MAX as u32 {
                return Err(BAD);
            }
            let float = flags & 1 != 0;
            let be = flags & 2 == 0;
            let container = bytes_per_packet / channels;
            let sample = match (float, bits, container) {
                (true, 32, 4) => SampleFormat::F32,
                (true, 64, 8) => SampleFormat::F64,
                (false, 8, 1) => SampleFormat::S8,
                (false, 16, 2) => SampleFormat::S16,
                (false, 24, 3) => SampleFormat::S24,
                (false, 32, 4) => SampleFormat::S32,
                _ => return Err(UNSUPPORTED),
            };
            desc = Some((rate.round() as u32, channels as u16, sample, be));
        } else if id == b"data" {
            let (rate, channels, sample, be) = desc.ok_or(BAD)?;
            // a size of -1 means "to the end of the file"; the first four bytes are the edit count
            let end = if size < 0 { b.len() } else { (body + size as usize).min(b.len()) };
            let start = body + 4;
            if start > end {
                return Err(BAD);
            }
            return finish_info(Container::Caf, rate, channels, sample, be, start, end - start);
        }
        if size < 0 {
            break;
        }
        pos = body + size as usize;
    }
    Err(BAD)
}

/// Decodes a whole file from memory.
pub fn decode(b: &[u8]) -> Result<AudioFile, AudioError> {
    let info = probe(b)?;
    let mut samples = Vec::new();
    decode_samples(&b[info.data_offset..info.data_offset + info.data_len], info.sample, info.big_endian, &mut samples);
    Ok(AudioFile { container: info.container, rate: info.rate, channels: info.channels, sample: info.sample, samples })
}

/// Reads and decodes a file.
pub fn read(path: &Path) -> Result<AudioFile, AudioError> {
    let bytes = std::fs::read(path).map_err(|_| AudioError::Io)?;
    decode(&bytes)
}

/// Encodes a file in memory with `f.container` and `f.sample`. WAV takes U8, S16, S24, S32, F32 and F64; AIFF
/// takes S8, S16, S24, S32, F32 and F64 (the float forms as AIFF-C); CAF takes S8, S16, S24, S32, F32 and F64.
pub fn encode(f: &AudioFile) -> Result<Vec<u8>, AudioError> {
    if f.channels == 0 || f.rate == 0 || f.samples.len() % f.channels as usize != 0 {
        return Err(AudioError::BadValue);
    }
    match f.container {
        Container::Wav => encode_wav(f),
        Container::Aiff => encode_aiff(f),
        Container::Caf => encode_caf(f),
    }
}

/// Encodes and writes a file.
pub fn write(path: &Path, f: &AudioFile) -> Result<(), AudioError> {
    let bytes = encode(f)?;
    std::fs::write(path, bytes).map_err(|_| AudioError::Io)
}

fn encode_wav(f: &AudioFile) -> Result<Vec<u8>, AudioError> {
    let kind: u16 = match f.sample {
        SampleFormat::U8 | SampleFormat::S16 | SampleFormat::S24 | SampleFormat::S32 => 1,
        SampleFormat::F32 | SampleFormat::F64 => 3,
        _ => return Err(UNSUPPORTED),
    };
    let mut data = Vec::new();
    encode_samples(&f.samples, f.sample, false, &mut data);
    let extensible = f.channels > 2 || f.sample.bits() > 16 && kind == 1;
    let fmt_len: u32 = if extensible { 40 } else { 16 };
    let pad = data.len() & 1;
    let total = 4 + (8 + fmt_len as usize) + (8 + data.len() + pad);
    if total > u32::MAX as usize {
        return Err(AudioError::BadValue);
    }
    let bytes = f.sample.bytes() as u16;
    let block = bytes * f.channels;
    let mut o = Vec::with_capacity(total + 8);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&(total as u32).to_le_bytes());
    o.extend_from_slice(b"WAVE");
    o.extend_from_slice(b"fmt ");
    o.extend_from_slice(&fmt_len.to_le_bytes());
    o.extend_from_slice(&(if extensible { 0xFFFEu16 } else { kind }).to_le_bytes());
    o.extend_from_slice(&f.channels.to_le_bytes());
    o.extend_from_slice(&f.rate.to_le_bytes());
    o.extend_from_slice(&(f.rate * block as u32).to_le_bytes());
    o.extend_from_slice(&block.to_le_bytes());
    o.extend_from_slice(&(bytes * 8).to_le_bytes());
    if extensible {
        o.extend_from_slice(&22u16.to_le_bytes());
        o.extend_from_slice(&(bytes * 8).to_le_bytes()); // valid bits
        o.extend_from_slice(&0u32.to_le_bytes()); // channel mask: unspecified
        // sub-format GUID: the format tag, then 00000000-0010-8000-00AA-00389B71
        o.extend_from_slice(&kind.to_le_bytes());
        o.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    }
    o.extend_from_slice(b"data");
    o.extend_from_slice(&(data.len() as u32).to_le_bytes());
    o.extend_from_slice(&data);
    if pad == 1 {
        o.push(0);
    }
    Ok(o)
}

fn encode_aiff(f: &AudioFile) -> Result<Vec<u8>, AudioError> {
    let (comp, name): (&[u8; 4], &[u8]) = match f.sample {
        SampleFormat::S8 | SampleFormat::S16 | SampleFormat::S24 | SampleFormat::S32 => (b"NONE", b"not compressed"),
        SampleFormat::F32 => (b"fl32", b"32-bit float"),
        SampleFormat::F64 => (b"fl64", b"64-bit float"),
        _ => return Err(UNSUPPORTED),
    };
    let aifc = f.sample.is_float();
    let mut data = Vec::new();
    encode_samples(&f.samples, f.sample, true, &mut data);
    let frames = (f.samples.len() / f.channels as usize) as u32;
    // AIFF-C: compression type, then a pascal string padded to an even length
    let mut comm_tail = Vec::new();
    if aifc {
        comm_tail.extend_from_slice(comp);
        comm_tail.push(name.len() as u8);
        comm_tail.extend_from_slice(name);
        if (1 + name.len()) & 1 == 1 {
            comm_tail.push(0);
        }
    }
    let comm_len = 18 + comm_tail.len();
    let pad = data.len() & 1;
    let ssnd_len = 8 + data.len();
    let fver = if aifc { 12 } else { 0 };
    let total = 4 + fver + (8 + comm_len) + (8 + ssnd_len + pad);
    if total > u32::MAX as usize {
        return Err(AudioError::BadValue);
    }
    let mut o = Vec::with_capacity(total + 8);
    o.extend_from_slice(b"FORM");
    o.extend_from_slice(&(total as u32).to_be_bytes());
    o.extend_from_slice(if aifc { b"AIFC" } else { b"AIFF" });
    if aifc {
        o.extend_from_slice(b"FVER");
        o.extend_from_slice(&4u32.to_be_bytes());
        o.extend_from_slice(&0xA280_5140u32.to_be_bytes());
    }
    o.extend_from_slice(b"COMM");
    o.extend_from_slice(&(comm_len as u32).to_be_bytes());
    o.extend_from_slice(&f.channels.to_be_bytes());
    o.extend_from_slice(&frames.to_be_bytes());
    o.extend_from_slice(&(f.sample.bits() as u16).to_be_bytes());
    o.extend_from_slice(&u32_to_extended(f.rate));
    o.extend_from_slice(&comm_tail);
    o.extend_from_slice(b"SSND");
    o.extend_from_slice(&(ssnd_len as u32).to_be_bytes());
    o.extend_from_slice(&[0u8; 8]); // offset and block size
    o.extend_from_slice(&data);
    if pad == 1 {
        o.push(0);
    }
    Ok(o)
}

fn encode_caf(f: &AudioFile) -> Result<Vec<u8>, AudioError> {
    match f.sample {
        SampleFormat::S8 | SampleFormat::S16 | SampleFormat::S24 | SampleFormat::S32 | SampleFormat::F32 | SampleFormat::F64 => {}
        _ => return Err(UNSUPPORTED),
    }
    let mut data = Vec::new();
    encode_samples(&f.samples, f.sample, false, &mut data);
    let bytes = f.sample.bytes() as u32;
    let mut o = Vec::with_capacity(data.len() + 64);
    o.extend_from_slice(b"caff");
    o.extend_from_slice(&1u16.to_be_bytes());
    o.extend_from_slice(&0u16.to_be_bytes());
    o.extend_from_slice(b"desc");
    o.extend_from_slice(&32i64.to_be_bytes());
    o.extend_from_slice(&(f.rate as f64).to_bits().to_be_bytes());
    o.extend_from_slice(b"lpcm");
    // flags: bit 0 float, bit 1 little endian
    o.extend_from_slice(&((f.sample.is_float() as u32) | 2).to_be_bytes());
    o.extend_from_slice(&(bytes * f.channels as u32).to_be_bytes());
    o.extend_from_slice(&1u32.to_be_bytes());
    o.extend_from_slice(&(f.channels as u32).to_be_bytes());
    o.extend_from_slice(&f.sample.bits().to_be_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&((data.len() + 4) as i64).to_be_bytes());
    o.extend_from_slice(&0u32.to_be_bytes()); // edit count
    o.extend_from_slice(&data);
    Ok(o)
}

/// Pulls frames out of a probed file in memory without decoding it whole.
pub struct FileReader {
    bytes: Vec<u8>,
    info: FileInfo,
    cursor: u64,
}

impl FileReader {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<FileReader, AudioError> {
        let info = probe(&bytes)?;
        Ok(FileReader { bytes, info, cursor: 0 })
    }

    pub fn open(path: &Path) -> Result<FileReader, AudioError> {
        FileReader::from_bytes(std::fs::read(path).map_err(|_| AudioError::Io)?)
    }

    pub fn info(&self) -> &FileInfo {
        &self.info
    }

    /// The next frame to be read.
    pub fn position(&self) -> u64 {
        self.cursor
    }

    /// Moves to `frame` (clamped to the end); returns where it landed.
    pub fn seek(&mut self, frame: u64) -> u64 {
        self.cursor = frame.min(self.info.frames);
        self.cursor
    }

    /// Appends up to `frames` frames of interleaved floats to `out`; returns how many were read (0 at the end).
    pub fn read(&mut self, frames: usize, out: &mut Vec<f32>) -> usize {
        let n = (self.info.frames - self.cursor).min(frames as u64) as usize;
        let fb = self.info.sample.bytes() * self.info.channels as usize;
        let from = self.info.data_offset + self.cursor as usize * fb;
        decode_samples(&self.bytes[from..from + n * fb], self.info.sample, self.info.big_endian, out);
        self.cursor += n as u64;
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(container: Container, sample: SampleFormat, channels: u16, samples: Vec<f32>) -> AudioFile {
        AudioFile { container, rate: 44100, channels, sample, samples }
    }

    #[test]
    fn wav_header_is_the_canonical_44_bytes() {
        let f = file(Container::Wav, SampleFormat::S16, 2, vec![0.5, -0.5]);
        let b = encode(&f).unwrap();
        assert_eq!(b.len(), 44 + 4);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32le(&b, 4), Some(40)); // file length minus 8
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u32le(&b, 16), Some(16));
        assert_eq!(u16le(&b, 20), Some(1));
        assert_eq!(u16le(&b, 22), Some(2));
        assert_eq!(u32le(&b, 24), Some(44100));
        assert_eq!(u32le(&b, 28), Some(176400)); // 44100 * 4
        assert_eq!(u16le(&b, 32), Some(4));
        assert_eq!(u16le(&b, 34), Some(16));
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32le(&b, 40), Some(4));
        // 0.5 -> 16384 = 0x4000, -0.5 -> -16384 = 0xC000
        assert_eq!(&b[44..48], &[0x00, 0x40, 0x00, 0xC0]);
    }

    #[test]
    fn wav_round_trips_every_sample_format() {
        for s in [SampleFormat::U8, SampleFormat::S16, SampleFormat::S24, SampleFormat::S32, SampleFormat::F32, SampleFormat::F64] {
            for ch in [1u16, 2, 6] {
                let samples: Vec<f32> = (0..ch as usize * 5).map(|i| (i as f32 - 15.0) / 32.0).collect();
                let f = file(Container::Wav, s, ch, samples);
                let back = decode(&encode(&f).unwrap()).unwrap();
                assert_eq!(back, f, "{s:?} x{ch}");
            }
        }
    }

    #[test]
    fn wav_odd_data_is_padded_and_chunks_are_skipped() {
        let f = file(Container::Wav, SampleFormat::U8, 1, vec![0.0, 0.5, -0.5]);
        let b = encode(&f).unwrap();
        // 3 data bytes plus a pad byte; the RIFF length counts the pad
        assert_eq!(b.len(), 48);
        assert_eq!(u32le(&b, 4), Some(40));
        assert_eq!(u32le(&b, 40), Some(3));
        // a LIST chunk before data must be skipped
        let mut with_list = Vec::new();
        with_list.extend_from_slice(&b[..36]);
        with_list.extend_from_slice(b"LIST");
        with_list.extend_from_slice(&3u32.to_le_bytes());
        with_list.extend_from_slice(&[1, 2, 3, 0]);
        with_list.extend_from_slice(&b[36..]);
        let back = decode(&with_list).unwrap();
        assert_eq!(back.samples, f.samples);
    }

    #[test]
    fn wav_extensible_float_and_surround_headers() {
        let f = file(Container::Wav, SampleFormat::F32, 6, vec![0.25; 12]);
        let b = encode(&f).unwrap();
        assert_eq!(u32le(&b, 16), Some(40));
        assert_eq!(u16le(&b, 20), Some(0xFFFE));
        assert_eq!(u16le(&b, 44), Some(3)); // sub-format tag: float
        let info = probe(&b).unwrap();
        assert_eq!((info.channels, info.frames, info.sample), (6, 2, SampleFormat::F32));
    }

    #[test]
    fn wav_streamed_sizes_take_the_rest_of_the_file() {
        let f = file(Container::Wav, SampleFormat::S16, 1, vec![0.0, 0.25, 0.5, 0.75]);
        let mut b = encode(&f).unwrap();
        b[40..44].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(probe(&b).unwrap().frames, 4);
    }

    #[test]
    fn aiff_header_and_extended_rate() {
        // 44100 as an 80-bit extended float is 400E AC44 0000 0000 0000
        assert_eq!(u32_to_extended(44100), [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        for r in [8000u32, 44100, 48000, 96000, 192000, 1] {
            assert_eq!(extended_to_f64(&u32_to_extended(r)), Some(r as f64));
        }
        let f = file(Container::Aiff, SampleFormat::S16, 2, vec![0.5, -0.5]);
        let b = encode(&f).unwrap();
        assert_eq!(&b[0..4], b"FORM");
        assert_eq!(u32be(&b, 4), Some(b.len() as u32 - 8));
        assert_eq!(&b[8..12], b"AIFF");
        assert_eq!(&b[12..16], b"COMM");
        assert_eq!(u32be(&b, 16), Some(18));
        assert_eq!(u16be(&b, 20), Some(2));
        assert_eq!(u32be(&b, 22), Some(1)); // frames
        assert_eq!(u16be(&b, 26), Some(16));
        // big-endian samples: 0x4000, 0xC000
        assert_eq!(&b[b.len() - 4..], &[0x40, 0x00, 0xC0, 0x00]);
    }

    #[test]
    fn aiff_round_trips_and_aifc_carries_floats() {
        for s in [SampleFormat::S8, SampleFormat::S16, SampleFormat::S24, SampleFormat::S32, SampleFormat::F32, SampleFormat::F64] {
            let samples: Vec<f32> = (0..8).map(|i| (i as f32 - 4.0) / 8.0).collect();
            let f = file(Container::Aiff, s, 2, samples);
            let b = encode(&f).unwrap();
            assert_eq!(&b[8..12], if s.is_float() { b"AIFC" } else { b"AIFF" });
            assert_eq!(decode(&b).unwrap(), f, "{s:?}");
        }
    }

    #[test]
    fn aifc_little_endian_sowt_reads() {
        // build an AIFC with 'sowt' by hand: one mono frame of 0x4000
        let mut b = Vec::new();
        b.extend_from_slice(b"FORM");
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"AIFC");
        b.extend_from_slice(b"COMM");
        b.extend_from_slice(&24u32.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&16u16.to_be_bytes());
        b.extend_from_slice(&u32_to_extended(48000));
        b.extend_from_slice(b"sowt");
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(b"SSND");
        b.extend_from_slice(&10u32.to_be_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(&[0x00, 0x40]);
        let f = decode(&b).unwrap();
        assert_eq!((f.rate, f.channels, f.samples.clone()), (48000, 1, vec![0.5]));
    }

    #[test]
    fn caf_header_and_round_trip() {
        let f = file(Container::Caf, SampleFormat::S16, 2, vec![0.5, -0.5]);
        let b = encode(&f).unwrap();
        assert_eq!(&b[0..8], b"caff\x00\x01\x00\x00");
        assert_eq!(&b[8..12], b"desc");
        assert_eq!(u64be(&b, 12), Some(32));
        assert_eq!(f64::from_bits(u64be(&b, 20).unwrap()), 44100.0);
        assert_eq!(&b[28..32], b"lpcm");
        assert_eq!(u32be(&b, 32), Some(2)); // little-endian flag
        assert_eq!(u32be(&b, 36), Some(4)); // bytes per packet
        assert_eq!(&b[52..56], b"data");
        for s in [SampleFormat::S8, SampleFormat::S16, SampleFormat::S24, SampleFormat::S32, SampleFormat::F32, SampleFormat::F64] {
            let samples: Vec<f32> = (0..6).map(|i| (i as f32 - 3.0) / 8.0).collect();
            let f = file(Container::Caf, s, 3, samples);
            assert_eq!(decode(&encode(&f).unwrap()).unwrap(), f, "{s:?}");
        }
    }

    #[test]
    fn caf_big_endian_ints_read() {
        let f = file(Container::Caf, SampleFormat::S16, 1, vec![0.5]);
        let mut b = encode(&f).unwrap();
        b[32..36].copy_from_slice(&0u32.to_be_bytes()); // clear the little-endian flag
        let n = b.len();
        b[n - 2..].copy_from_slice(&[0x40, 0x00]);
        assert_eq!(decode(&b).unwrap().samples, vec![0.5]);
    }

    #[test]
    fn malformed_and_unsupported_files() {
        assert_eq!(probe(b"nope"), Err(AudioError::Protocol));
        assert_eq!(probe(b""), Err(AudioError::Protocol));
        let f = file(Container::Wav, SampleFormat::S16, 1, vec![0.0; 4]);
        let b = encode(&f).unwrap();
        // no data chunk
        assert_eq!(probe(&b[..36]), Err(AudioError::Protocol));
        // an unsupported WAV format tag (ADPCM = 2)
        let mut adpcm = b.clone();
        adpcm[20] = 2;
        assert_eq!(probe(&adpcm), Err(AudioError::BadValue));
        // a sample format a container cannot hold
        assert_eq!(encode(&file(Container::Aiff, SampleFormat::U8, 1, vec![0.0])), Err(AudioError::BadValue));
        // samples that are not whole frames
        assert_eq!(encode(&file(Container::Wav, SampleFormat::S16, 2, vec![0.0; 3])), Err(AudioError::BadValue));
    }

    #[test]
    fn reader_streams_and_seeks() {
        let samples: Vec<f32> = (0..10).map(|i| i as f32 / 32.0).collect();
        let f = file(Container::Wav, SampleFormat::F32, 2, samples.clone());
        let mut r = FileReader::from_bytes(encode(&f).unwrap()).unwrap();
        assert_eq!(r.info().frames, 5);
        let mut out = Vec::new();
        assert_eq!(r.read(3, &mut out), 3);
        assert_eq!(out, samples[..6]);
        assert_eq!(r.read(3, &mut out), 2);
        assert_eq!(r.read(3, &mut out), 0);
        assert_eq!(r.seek(1), 1);
        out.clear();
        r.read(1, &mut out);
        assert_eq!(out, samples[2..4]);
        assert_eq!(r.seek(99), 5);
    }

    #[test]
    fn extension_lookup() {
        assert_eq!(Container::from_extension(Path::new("a.WAV")), Some(Container::Wav));
        assert_eq!(Container::from_extension(Path::new("a.aiff")), Some(Container::Aiff));
        assert_eq!(Container::from_extension(Path::new("a.caf")), Some(Container::Caf));
        assert_eq!(Container::from_extension(Path::new("a.mp3")), None);
    }

    #[test]
    fn files_on_disk_round_trip() {
        let dir = std::env::temp_dir().join(format!("bifrost-toolbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.wav");
        let f = file(Container::Wav, SampleFormat::S16, 1, vec![0.25, -0.25]);
        write(&p, &f).unwrap();
        assert_eq!(read(&p).unwrap(), f);
        assert_eq!(read(&dir.join("missing.wav")), Err(AudioError::Io));
        std::fs::remove_dir_all(&dir).ok();
    }
}
