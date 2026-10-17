//! Audio files for the Music app: one `AudioDecoder` trait over MP4/M4A
//! (AAC through the demuxer in `mp4.rs`), MP3 (`mp3.rs`), bare ADTS / LOAS
//! AAC (`aacstream.rs`) and FLAC / Ogg Vorbis / WAV (`symfile.rs`), and the
//! tags they carry (`ilst`, ID3v2, Vorbis comments, RIFF INFO).

use super::mp4::{Codec, Demuxer, Kind};
use super::source::{FileSource, Source};
use super::aacstream::AacStream;
use super::{aac, codec, id3, ilst, mp3, symfile, Error};
use bifrost_core::midi_synth::Renderer;
use bifrost_core::smf::{Smf, SmfError};
use std::path::Path;

/// The largest Standard MIDI File that is read whole (the Music scan skips bigger ones too).
const MAX_MIDI_BYTES: u64 = 16 << 20;
/// Frames per packet the MIDI renderer hands the player (about 85 ms at 48 kHz).
const MIDI_PACKET_FRAMES: usize = 4096;

/// Whether a path names a Standard MIDI File (`.mid` or `.midi`, any case).
pub fn is_midi(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi"))
}

/// A Standard MIDI File rendered by the synth in `bifrost_core::midi_synth` (additive tones, noise drums, no sound
/// bank). It renders at the player's output rate, so the player does not resample it.
pub struct Midi {
    renderer: Renderer,
    duration_ms: f64,
}

impl Midi {
    pub fn open(path: &Path) -> Result<Self, Error> {
        if std::fs::metadata(path)?.len() > MAX_MIDI_BYTES {
            return Err(Error::Invalid("MIDI file is too large"));
        }
        let bytes = std::fs::read(path)?;
        let smf = Smf::parse(&bytes).map_err(midi_error)?;
        Ok(Midi { renderer: Renderer::new(&smf, aac::OUT_RATE), duration_ms: smf.duration() * 1000.0 })
    }
}

fn midi_error(e: SmfError) -> Error {
    match e {
        SmfError::NotMidi => Error::Invalid("not a Standard MIDI File"),
        SmfError::Truncated => Error::Invalid("MIDI file ends early"),
        SmfError::Unsupported(what) => Error::Invalid(what),
    }
}

impl AudioDecoder for Midi {
    fn rate(&self) -> u32 {
        self.renderer.rate()
    }

    fn channels(&self) -> usize {
        bifrost_core::midi_synth::CHANNELS
    }

    fn duration_ms(&self) -> Option<f64> {
        Some(self.duration_ms)
    }

    fn next(&mut self, out: &mut Vec<i16>) -> Result<bool, Error> {
        Ok(self.renderer.next_block(out, MIDI_PACKET_FRAMES) > 0)
    }

    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error> {
        let at = ms.clamp(0.0, self.duration_ms);
        self.renderer.seek(at / 1000.0);
        Ok(at)
    }
}

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
    /// the unsynchronised lyrics text as tagged (ID3 USLT), when the file carries it
    pub lyrics: Option<String>,
    /// the chapters in play order (ID3 CHAP frames, an MP4 chapter list or chapter track); empty when there are none
    pub chapters: Vec<super::chapters::Chapter>,
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
    if is_midi(path) {
        return Ok(Box::new(Midi::open(path)?));
    }
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

/// Writes `tags` into the file at `path`: an ID3v2.4 tag for MP3 and ADTS, the `ilst` for M4A. The audio bytes are kept.
/// The file is replaced atomically: the new bytes go to a temporary file beside it, which is renamed over the original,
/// so a crash leaves either the old file or the new one. Other formats are refused.
pub fn write_tags(path: &Path, tags: &Tags) -> Result<(), Error> {
    let bytes = std::fs::read(path)?;
    let out = match detect(path)? {
        Format::Mp3 | Format::Adts => id3::replace_tag(&bytes, tags)
            .ok_or(Error::Invalid("the file's tag cannot be rewritten without losing frames (ID3v2.2 or compressed frames)"))?,
        Format::M4a => ilst::rewrite(&bytes, tags)?,
        _ => return Err(Error::Invalid("tags are written to MP3 and M4A files only")),
    };
    replace_file(path, &out)
}

/// Replaces the file at `path` with `bytes` through a temporary file in the same directory.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tags-tmp"));
    let perms = std::fs::metadata(path)?.permissions();
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, perms)?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
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

    /// A format 0 Standard MIDI File with one track: 96 ticks a quarter, 120 beats a minute.
    fn midi_bytes(track: &[u8]) -> Vec<u8> {
        let mut b = b"MThd".to_vec();
        b.extend(6u32.to_be_bytes());
        b.extend(0u16.to_be_bytes());
        b.extend(1u16.to_be_bytes());
        b.extend(96u16.to_be_bytes());
        b.extend(b"MTrk");
        b.extend((track.len() as u32).to_be_bytes());
        b.extend(track);
        b
    }

    #[test]
    fn a_midi_file_opens_by_extension_and_renders_at_the_output_rate() {
        // A4 on at 0 s, off at 0.5 s, then the 0.2 s release: about 0.7 s of stereo at 48 kHz.
        let track = [0x00, 0x90, 69, 100, 0x60, 0x80, 69, 0, 0x00, 0xFF, 0x2F, 0x00];
        let path = std::env::temp_dir().join(format!("bifrost-midi-test-{}.MIDI", std::process::id()));
        std::fs::write(&path, midi_bytes(&track)).unwrap();
        assert!(is_midi(&path) && !is_midi(Path::new("x.mp3")));
        let mut d = open(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!((d.rate(), d.channels()), (48_000, 2));
        assert!((d.duration_ms().unwrap() - 500.0).abs() < 1.0, "{:?}", d.duration_ms());
        let pcm = drain(&mut *d);
        let frames = pcm.len() / 2;
        assert!((33_590..=33_610).contains(&frames), "{frames} frames");
        let peak = pcm.iter().map(|s| s.abs()).max().unwrap_or(0);
        assert!(peak > 3_000, "peak {peak}");
        assert!(d.seek_ms(250.0).unwrap() == 250.0);
    }

    #[test]
    fn a_damaged_midi_file_is_an_error_not_a_panic() {
        let path = std::env::temp_dir().join(format!("bifrost-midi-bad-{}.mid", std::process::id()));
        std::fs::write(&path, b"not a midi file at all").unwrap();
        let r = open(&path);
        let _ = std::fs::remove_file(&path);
        assert!(r.is_err());
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

    #[test]
    fn write_tags_keeps_the_audio_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("bifrost-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["tone_id3v24.mp3", "tone.m4a"] {
            let path = dir.join(name);
            std::fs::copy(fixture(name), &path).unwrap();
            let before = {
                let mut d = open(&path).unwrap();
                let mut first = Vec::new();
                assert!(d.next(&mut first).unwrap());
                first
            };
            let mut tags = read_tags(&path).unwrap();
            tags.title = Some("A much longer title written by the tag writer".into());
            tags.track = Some(9);
            write_tags(&path, &tags).unwrap();
            let back = read_tags(&path).unwrap();
            assert_eq!(back.title, tags.title, "{name}");
            assert_eq!(back.track, Some(9), "{name}");
            assert!(!dir.join(format!(".{name}.tags-tmp")).exists(), "the temporary file is renamed away");
            // the first decoded packet is unchanged, so the audio and (for M4A) the chunk offsets still line up
            let mut d = open(&path).unwrap();
            let mut after = Vec::new();
            assert!(d.next(&mut after).unwrap());
            assert_eq!(after, before, "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
