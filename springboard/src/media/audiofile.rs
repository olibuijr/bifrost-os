//! Audio files for the Music app: one `AudioDecoder` trait over MP4/M4A
//! (AAC through the demuxer in `mp4.rs`), MP3 (`mp3.rs`), bare ADTS / LOAS
//! AAC (`aacstream.rs`) and FLAC / Ogg Vorbis / WAV (`symfile.rs`), and the
//! tags they carry (`ilst`, ID3v2, Vorbis comments, RIFF INFO).

use super::mp4::{Codec, Demuxer, Kind};
use super::source::{FileSource, Source};
use super::aacstream::AacStream;
use super::{aac, codec, id3, ilst, mp3, symfile, Error};
use std::path::Path;

/// What a file says about itself; every field is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track: Option<u32>,
    pub year: Option<u32>,
    /// the cover picture, still encoded (JPEG or PNG)
    pub cover: Option<Vec<u8>>,
}

/// A decoded audio stream: interleaved S16 at its own rate and channel
/// count, one packet at a time.
pub trait AudioDecoder: Send {
    fn rate(&self) -> u32;
    fn channels(&self) -> usize;
    /// The stream's length, when the file declares one.
    fn duration_ms(&self) -> Option<f64>;
    /// Decodes the next packet into `out` (replaced); false at the end. A
    /// corrupt packet yields an empty `out` and true, so the stream goes on.
    fn next(&mut self, out: &mut Vec<i16>) -> Result<bool, Error>;
    /// Repositions at (or just before) `ms`; returns where it landed.
    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error>;
}

/// The container / codec of an audio file (see `codec` for the sniffing).
pub use super::codec::Kind as Format;

/// The format by extension; None for files the Music app does not play.
pub fn format_of(path: &Path) -> Option<Format> {
    codec::from_extension(path)
}

/// What the file really is: its first bytes decide, the extension is the
/// fallback (a mislabelled `.aac` that is MP4 still plays).
fn detect(path: &Path) -> Result<Format, Error> {
    let head = read_head(path, 4096)?;
    codec::sniff(None, &head).or_else(|| format_of(path)).ok_or(Error::Invalid("unsupported file"))
}

/// Opens a file for decoding.
pub fn open(path: &Path) -> Result<Box<dyn AudioDecoder>, Error> {
    match detect(path)? {
        Format::Mp3 => Ok(Box::new(mp3::Mp3::open(path)?)),
        Format::M4a => Ok(Box::new(M4a::open(Box::new(FileSource::open(path)?))?)),
        aac @ (Format::Adts | Format::Latm) => Ok(Box::new(AacStream::open(Box::new(mp3::Skip::open(path)?), aac.aac_framing().expect("an AAC kind"), true)?)),
        kind @ (Format::Flac | Format::Ogg | Format::Wav) => Ok(Box::new(symfile::SymFile::open(kind, path)?)),
    }
}

/// Reads a file's tags (an empty `Tags` when it has none).
pub fn read_tags(path: &Path) -> Result<Tags, Error> {
    match detect(path)? {
        // bare AAC files carry an ID3v2 tag in front like MP3 does
        Format::Mp3 | Format::Adts | Format::Latm => {
            let head = read_head(path, 10)?;
            let len = id3::tag_len(&head);
            if len == 0 {
                return Ok(Tags::default());
            }
            Ok(id3::parse(&read_head(path, len)?).unwrap_or_default())
        }
        Format::M4a => ilst::read(&mut FileSource::open(path)?),
        kind @ (Format::Flac | Format::Ogg | Format::Wav) => symfile::read_tags(kind, path),
    }
}

/// The first `n` bytes of the file (fewer at a short file).
fn read_head(path: &Path, n: usize) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(n);
    std::fs::File::open(path)?.take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// AAC in an MP4 container.
pub struct M4a {
    demuxer: Demuxer,
    source: Box<dyn Source>,
    decoder: aac::Decoder,
    duration_ms: Option<f64>,
}

impl M4a {
    pub fn open(mut source: Box<dyn Source>) -> Result<Self, Error> {
        let demuxer = Demuxer::open(&mut *source)?;
        let Some(Codec::Aac { asc, .. }) = demuxer.audio().map(|t| t.codec.clone()) else { return Err(Error::Invalid("no AAC track")) };
        let decoder = aac::Decoder::new(&asc)?;
        let duration_ms = demuxer.duration_ms();
        Ok(M4a { demuxer, source, decoder, duration_ms })
    }
}

impl AudioDecoder for M4a {
    fn rate(&self) -> u32 {
        self.decoder.rate()
    }

    fn channels(&self) -> usize {
        self.decoder.channels()
    }

    fn duration_ms(&self) -> Option<f64> {
        self.duration_ms
    }

    fn next(&mut self, out: &mut Vec<i16>) -> Result<bool, Error> {
        match self.demuxer.next(Kind::Audio, &mut *self.source)? {
            Some(sample) => {
                self.decoder.decode(&sample.data, out)?;
                Ok(true)
            }
            None => {
                out.clear();
                Ok(false)
            }
        }
    }

    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error> {
        let at = self.demuxer.seek_ms(ms, &mut *self.source)?;
        self.decoder.reset();
        Ok(at)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::source::MemSource;
    use super::*;

    pub const M4A: &[u8] = include_bytes!("testdata/tone.m4a");

    /// Where the fixtures live on disk, for the path-based API.
    pub fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/media/testdata").join(name)
    }

    /// Decodes everything to one interleaved buffer.
    pub fn drain(d: &mut dyn AudioDecoder) -> Vec<i16> {
        let (mut all, mut chunk) = (Vec::new(), Vec::new());
        while d.next(&mut chunk).unwrap() {
            all.extend_from_slice(&chunk);
        }
        all
    }

    /// Power of `hz` in channel 0 of an interleaved signal (Goertzel).
    pub fn power(pcm: &[i16], channels: usize, rate: u32, hz: f64) -> f64 {
        let n = pcm.len() / channels;
        let k = 2.0 * std::f64::consts::PI * hz / rate as f64;
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for i in 0..n {
            let s = pcm[i * channels] as f64 / 32768.0 + 2.0 * k.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * k.cos() * s1 * s2) / n as f64
    }

    #[test]
    fn formats_by_extension() {
        assert_eq!(format_of(Path::new("/a/b.MP3")), Some(Format::Mp3));
        assert_eq!(format_of(Path::new("x.m4a")), Some(Format::M4a));
        assert_eq!(format_of(Path::new("x.mp4")), Some(Format::M4a));
        assert_eq!(format_of(Path::new("x.flac")), Some(Format::Flac));
        assert_eq!(format_of(Path::new("x.aac")), Some(Format::Adts));
        assert_eq!(format_of(Path::new("x.opus")), None);
        assert_eq!(format_of(Path::new("noext")), None);
    }

    #[test]
    fn m4a_decodes_the_fixture_tone_and_seeks() {
        let mut d = M4a::open(Box::new(MemSource::new(M4A.to_vec()))).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        assert!((d.duration_ms().unwrap() - 1200.0).abs() < 60.0, "{:?}", d.duration_ms());
        let pcm = drain(&mut d);
        let frames = pcm.len() / 2;
        assert!(frames > 44100 && frames < 44100 * 13 / 10 + 4096, "{frames} frames");
        let tail = &pcm[pcm.len() / 2..];
        assert!(power(tail, 2, 44100, 440.0) > 20.0 * power(tail, 2, 44100, 1000.0).max(1e-9));
        let mut chunk = Vec::new();
        assert!(!d.next(&mut chunk).unwrap() && chunk.is_empty(), "stays at the end");
        let at = d.seek_ms(600.0).unwrap();
        assert!(at <= 600.0 && at > 500.0, "landed at {at}");
        let rest = drain(&mut d);
        assert!(rest.len() < pcm.len() * 6 / 10 && rest.len() > pcm.len() * 4 / 10, "{} of {}", rest.len(), pcm.len());
        assert!(M4a::open(Box::new(MemSource::new(include_bytes!("testdata/tiny_baseline.mp4").to_vec()))).is_ok(), "a video file's audio track plays too");
    }

    #[test]
    fn opens_and_tags_by_path() {
        let mut d = open(&fixture("tone.m4a")).unwrap();
        assert_eq!(d.rate(), 44100);
        assert!(!drain(&mut *d).is_empty());
        let t = read_tags(&fixture("tone.m4a")).unwrap();
        assert_eq!(t.title.as_deref(), Some("Tone M4A"));
        let t = read_tags(&fixture("tone_id3v23.mp3")).unwrap();
        assert_eq!(t.title.as_deref(), Some("Tone Song"));
        assert!(t.cover.is_some());
        assert!(open(&fixture("missing.mp3")).is_err());
        assert!(open(&fixture("tiny_frag.txt")).is_err());
        assert!(read_tags(&fixture("x.wav")).is_err());
    }

    #[test]
    fn opens_and_tags_flac_ogg_and_wav_by_path_and_sniffs_past_the_extension() {
        for (name, rate, title) in [("tone.flac", 44100, "Tone FLAC"), ("tone.ogg", 44100, "Tone Ogg"), ("tone.wav", 22050, "Tone WAV")] {
            let mut d = open(&fixture(name)).unwrap();
            assert_eq!(d.rate(), rate, "{name}");
            assert!(!drain(&mut *d).is_empty(), "{name}");
            assert_eq!(read_tags(&fixture(name)).unwrap().title.as_deref(), Some(title), "{name}");
        }
        // a FLAC file named .mp3 is still FLAC
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("sb-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(fixture("tone.flac"), dir.join("liar.mp3")).unwrap();
        assert!(!drain(&mut *open(&dir.join("liar.mp3")).unwrap()).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bare_adts_files_play_with_a_length() {
        let (_, frames) = super::super::aacstream::tests::raw_frames();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("sb-adts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.aac"), super::super::aacstream::tests::adts(&frames)).unwrap();
        let mut d = open(&dir.join("t.aac")).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        assert!(d.duration_ms().is_some());
        assert!(drain(&mut *d).len() / 2 > 44100);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
