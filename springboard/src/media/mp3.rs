//! MP3 decoding through `symphonia-bundle-mp3` (pure Rust): the ID3v2 tag
//! is skipped by our own reader, the Xing/LAME header gives the length and
//! the gapless trims, which the decoder applies to each packet.

use super::audiofile::AudioDecoder;
use super::{id3, Error};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use symphonia_bundle_mp3::{MpaDecoder, MpaReader};
use symphonia_core::codecs::audio::{AudioDecoder as _, AudioDecoderOptions};
use symphonia_core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia_core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia_core::units::Time;

/// A file viewed from `base` onwards, so the demuxer never sees the tag.
pub(super) struct Skip {
    file: File,
    base: u64,
    len: u64,
}

impl Skip {
    /// The file at `path` past its ID3v2 tag, if any.
    pub(super) fn open(path: &Path) -> Result<Self, Error> {
        let mut file = File::open(path)?;
        let mut head = [0u8; 10];
        let n = file.read(&mut head)?;
        let base = id3::tag_len(&head[..n]) as u64;
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(base))?;
        Ok(Skip { file, base, len })
    }
}

impl Read for Skip {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for Skip {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(p) => SeekFrom::Start(p + self.base),
            other => other,
        };
        Ok(self.file.seek(pos)?.saturating_sub(self.base))
    }
}

impl MediaSource for Skip {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len.saturating_sub(self.base))
    }
}

pub struct Mp3 {
    reader: MpaReader<'static>,
    decoder: MpaDecoder,
    rate: u32,
    channels: usize,
    duration_ms: Option<f64>,
}

fn invalid<T>(what: &'static str) -> Result<T, Error> {
    Err(Error::Invalid(what))
}

impl Mp3 {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Self::from_source(Box::new(Skip::open(path)?))
    }

    /// Decodes MP3 frames from any source, a live stream included (the
    /// first frame found starts the stream; there is no length then).
    pub fn from_source(source: Box<dyn MediaSource>) -> Result<Self, Error> {
        let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let reader = MpaReader::try_new(mss, FormatOptions::default()).or_else(|_| invalid("MP3 stream"))?;
        let track = reader.tracks().first().ok_or(Error::Invalid("MP3 track"))?;
        let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or(Error::Invalid("MP3 codec"))?;
        let rate = params.sample_rate.unwrap_or(44_100);
        let channels = params.channels.as_ref().map_or(2, |c| c.count());
        let duration_ms = track.num_frames.map(|f| f as f64 * 1000.0 / rate as f64);
        let decoder = MpaDecoder::try_new(params, &AudioDecoderOptions::default()).or_else(|_| invalid("MP3 decoder"))?;
        Ok(Mp3 { reader, decoder, rate, channels, duration_ms })
    }
}

impl Mp3 {
    /// Forgets the length the stream's first frame declared (a live stream
    /// that begins with a Xing header still has none).
    pub fn without_length(mut self) -> Self {
        self.duration_ms = None;
        self
    }
}

impl AudioDecoder for Mp3 {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn duration_ms(&self) -> Option<f64> {
        self.duration_ms
    }

    fn next(&mut self, out: &mut Vec<i16>) -> Result<bool, Error> {
        out.clear();
        let packet = match self.reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(false),
            // trailing junk (an ID3v1 tag, a truncated frame) ends the stream
            Err(_) => return Ok(false),
        };
        match self.decoder.decode_ref(&packet.as_packet_ref()) {
            Ok(buf) => {
                let spec = buf.spec();
                self.rate = spec.rate();
                self.channels = spec.channels().count();
                buf.copy_to_vec_interleaved(out);
                Ok(true)
            }
            Err(symphonia_core::errors::Error::DecodeError(_) | symphonia_core::errors::Error::IoError(_)) => Ok(true),
            Err(_) => invalid("MP3 packet"),
        }
    }

    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error> {
        let time = Time::from_millis_u64(ms.max(0.0) as u64);
        let seeked = self.reader.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: None }).or_else(|_| invalid("MP3 seek"))?;
        self.decoder.reset();
        Ok(seeked.actual_ts.get().max(0) as f64 * 1000.0 / self.rate as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::super::audiofile::tests::{drain, fixture, power};
    use super::*;

    #[test]
    fn decodes_the_stereo_fixture_with_its_tag_skipped() {
        let mut d = Mp3::open(&fixture("tone_id3v23.mp3")).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        let dur = d.duration_ms().unwrap();
        assert!((dur - 1200.0).abs() < 40.0, "lame header duration {dur}");
        let pcm = drain(&mut d);
        let frames = pcm.len() / 2;
        assert!((frames as f64 - 1.2 * 44100.0).abs() < 1200.0, "{frames} frames: gapless trims applied");
        let tail = &pcm[pcm.len() / 4..];
        assert!(power(tail, 2, 44100, 440.0) > 20.0 * power(tail, 2, 44100, 1000.0).max(1e-9));
        assert!(tail.iter().any(|&s| s.abs() > 2000));
        let mut chunk = Vec::new();
        assert!(!d.next(&mut chunk).unwrap());
    }

    #[test]
    fn mono_48k_fixture_and_seeking() {
        let mut d = Mp3::open(&fixture("tone_id3v24.mp3")).unwrap();
        assert_eq!((d.rate(), d.channels()), (48000, 1));
        let all = drain(&mut d).len();
        let at = d.seek_ms(600.0).unwrap();
        assert!(at <= 600.0 && at > 500.0, "landed at {at}");
        let rest = drain(&mut d).len();
        assert!(rest > all * 4 / 10 && rest < all * 6 / 10, "{rest} of {all}");
        assert_eq!(d.seek_ms(0.0).unwrap(), 0.0);
        assert_eq!(drain(&mut d).len(), all);
    }

    #[test]
    fn rejects_non_mp3_bytes() {
        assert!(Mp3::open(&fixture("tiny_baseline.mp4")).is_err());
        assert!(Mp3::open(&fixture("nope.mp3")).is_err());
    }
}
