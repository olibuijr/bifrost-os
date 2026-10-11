//! WAV output: 16-bit PCM with TPDF dither (seeded, so the same song renders to the same bytes), or 32-bit float.
use crate::dsp::Rng;
use std::io::Write;

/// Encodes stereo f32 samples as a 16-bit PCM WAV with TPDF dither.
pub fn encode16(left: &[f32], right: &[f32], sr: u32, seed: u64) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let mut rng = Rng::new(seed);
    let mut data = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        for x in [left[i], right[i]] {
            let d = (rng.unit() - rng.unit()) / 32768.0;
            let s = ((x + d) * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
    }
    header(data, sr, 2, 16, 1)
}

/// Encodes stereo f32 samples as 32-bit float WAV.
pub fn encode_f32(left: &[f32], right: &[f32], sr: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let mut data = Vec::with_capacity(frames * 8);
    for i in 0..frames {
        data.extend_from_slice(&left[i].to_le_bytes());
        data.extend_from_slice(&right[i].to_le_bytes());
    }
    header(data, sr, 2, 32, 3)
}

fn header(data: Vec<u8>, sr: u32, channels: u16, bits: u16, format: u16) -> Vec<u8> {
    let block = channels * bits / 8;
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * block as u32).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// Writes a WAV file atomically (temporary file, then rename).
pub fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("wav.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_sizes_are_consistent() {
        let l = vec![0.25f32; 100];
        let bytes = encode16(&l, &l, 48_000, 1);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 100 * 4);
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(riff as usize, bytes.len() - 8);
    }
}
