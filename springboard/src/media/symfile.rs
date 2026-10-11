//! FLAC, Ogg Vorbis (and FLAC in Ogg) and PCM WAV files through symphonia
//! (pure Rust), as `AudioDecoder`s, plus their tags: Vorbis comments (FLAC,
//! Ogg) and RIFF INFO (WAV), cover pictures from FLAC PICTURE blocks.

use super::audiofile::{AudioDecoder, Tags};
use super::codec::Kind;
use super::Error;
use std::fs::File;
use std::path::Path;
use symphonia_bundle_flac::{FlacDecoder, FlacReader};
use symphonia_codec_pcm::PcmDecoder;
use symphonia_codec_vorbis::VorbisDecoder;
use symphonia_core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_VORBIS};
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder as SymDecoder, AudioDecoderOptions};
use symphonia_core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia_core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia_core::meta::{MetadataRevision, StandardVisualKey};
use symphonia_core::units::Time;
use symphonia_format_ogg::OggReader;
use symphonia_format_riff::WavReader;

pub struct SymFile {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn SymDecoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    duration_ms: Option<f64>,
}

/// The container reader for `kind` over `src` (not for MP3 / AAC kinds).
fn container(kind: Kind, src: Box<dyn MediaSource>) -> Result<Box<dyn FormatReader>, Error> {
    let mss = MediaSourceStream::new(src, MediaSourceStreamOptions::default());
    let opts = FormatOptions::default();
    let bad = |_| Error::Invalid(match kind {
        Kind::Flac => "FLAC file",
        Kind::Ogg => "Ogg file",
        _ => "WAV file",
    });
    let reader: Box<dyn FormatReader> = match kind {
        Kind::Flac => Box::new(FlacReader::try_new(mss, opts).map_err(bad)?),
        Kind::Ogg => Box::new(OggReader::try_new(mss, opts).map_err(bad)?),
        Kind::Wav => Box::new(WavReader::try_new(mss, opts).map_err(bad)?),
        _ => return Err(Error::Invalid("not a symphonia container")),
    };
    Ok(reader)
}

/// The decoder for a track's codec: FLAC, Vorbis or PCM.
fn decoder_for(params: &AudioCodecParameters) -> Result<Box<dyn SymDecoder>, Error> {
    let opts = AudioDecoderOptions::default();
    let bad = |_| Error::Invalid("audio codec");
    let decoder: Box<dyn SymDecoder> = if params.codec == CODEC_ID_FLAC {
        Box::new(FlacDecoder::try_new(params, &opts).map_err(bad)?)
    } else if params.codec == CODEC_ID_VORBIS {
        Box::new(VorbisDecoder::try_new(params, &opts).map_err(bad)?)
    } else {
        // PCM in its many sample formats; anything else (Opus in Ogg, ADPCM in WAV) is refused here
        Box::new(PcmDecoder::try_new(params, &opts).map_err(bad)?)
    };
    Ok(decoder)
}

impl SymFile {
    pub fn open(kind: Kind, path: &Path) -> Result<Self, Error> {
        Self::from_source(kind, Box::new(File::open(path)?))
    }

    pub fn from_source(kind: Kind, src: Box<dyn MediaSource>) -> Result<Self, Error> {
        let reader = container(kind, src)?;
        let track = reader.first_track(TrackType::Audio).ok_or(Error::Invalid("audio track"))?;
        let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or(Error::Invalid("audio codec"))?;
        let rate = params.sample_rate.unwrap_or(44_100);
        let channels = params.channels.as_ref().map_or(2, |c| c.count());
        let duration_ms = track.num_frames.map(|f| f as f64 * 1000.0 / rate as f64);
        let decoder = decoder_for(params)?;
        let track_id = track.id;
        Ok(SymFile { reader, decoder, track_id, rate, channels, duration_ms })
    }
}

impl SymFile {
    /// Forgets any length the stream declared (a live stream has none).
    pub fn without_length(mut self) -> Self {
        self.duration_ms = None;
        self
    }
}

impl AudioDecoder for SymFile {
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
        let packet = loop {
            match self.reader.next_packet() {
                Ok(Some(p)) if p.track_id == self.track_id => break p,
                Ok(Some(_)) => {}
                // the end, or trailing junk, ends the stream
                Ok(None) | Err(_) => return Ok(false),
            }
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
            Err(_) => Err(Error::Invalid("audio packet")),
        }
    }

    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error> {
        let time = Time::from_millis_u64(ms.max(0.0) as u64);
        let seeked = self.reader.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) }).map_err(|_| Error::Invalid("seek"))?;
        self.decoder.reset();
        Ok(seeked.actual_ts.get().max(0) as f64 * 1000.0 / self.rate as f64)
    }
}

/// Reads a file's tags (an empty `Tags` when it has none).
pub fn read_tags(kind: Kind, path: &Path) -> Result<Tags, Error> {
    if kind == Kind::Wav {
        // Symphonia's WAV reader does not surface LIST/INFO, so read it directly
        return Ok(riff_info(&std::fs::read(path)?));
    }
    let mut reader = container(kind, Box::new(File::open(path)?))?;
    Ok(reader.metadata().current().map(tags_of).unwrap_or_default())
}

/// Tags from the LIST/INFO chunk of a RIFF/WAVE file (empty when absent).
pub fn riff_info(data: &[u8]) -> Tags {
    let mut tags = Tags::default();
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return tags;
    }
    let mut at = 12;
    while at + 8 <= data.len() {
        let size = u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        let body = &data[(at + 8).min(data.len())..(at + 8 + size).min(data.len())];
        if &data[at..at + 4] == b"LIST" && body.starts_with(b"INFO") {
            let mut i = 4;
            while i + 8 <= body.len() {
                let n = u32::from_le_bytes([body[i + 4], body[i + 5], body[i + 6], body[i + 7]]) as usize;
                let text = &body[(i + 8).min(body.len())..(i + 8 + n).min(body.len())];
                let text = String::from_utf8_lossy(text).trim_end_matches('\0').trim().to_string();
                if !text.is_empty() {
                    match &body[i..i + 4] {
                        b"INAM" => tags.title.get_or_insert(text),
                        b"IART" => tags.artist.get_or_insert(text),
                        b"IPRD" => tags.album.get_or_insert(text),
                        _ => &mut String::new(),
                    };
                }
                i += 8 + n + (n & 1);
            }
        }
        at += 8 + size + (size & 1);
    }
    tags
}

/// The leading number of "3", "3/12", "2024-05-01".
fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Maps a metadata revision (Vorbis comments, RIFF INFO) to our tags; the
/// first value of each field wins.
pub fn tags_of(rev: &MetadataRevision) -> Tags {
    let mut t = Tags::default();
    let all = rev.media.tags.iter().chain(rev.per_track.iter().flat_map(|p| p.metadata.tags.iter()));
    for tag in all {
        let value = tag.raw.value.to_string();
        let value = value.lines().next().unwrap_or("").trim().to_string();
        if value.is_empty() {
            continue;
        }
        let key = tag.raw.key.to_ascii_uppercase();
        let slot = match key.as_str() {
            "TITLE" | "INAM" => &mut t.title,
            "ARTIST" | "IART" => &mut t.artist,
            "ALBUM" | "IPRD" => &mut t.album,
            "ALBUMARTIST" | "ALBUM ARTIST" => &mut t.album_artist,
            "TRACKNUMBER" | "TRACK" | "ITRK" | "IPRT" => {
                t.track = t.track.or(leading_number(&value));
                continue;
            }
            "DATE" | "YEAR" | "ICRD" => {
                t.year = t.year.or(leading_number(&value).filter(|y| *y > 0));
                continue;
            }
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }
    let visuals = rev.media.visuals.iter().chain(rev.per_track.iter().flat_map(|p| p.metadata.visuals.iter()));
    let visuals: Vec<_> = visuals.collect();
    let pick = visuals.iter().find(|v| v.usage == Some(StandardVisualKey::FrontCover)).or(visuals.first());
    t.cover = pick.map(|v| v.data.to_vec());
    t
}

#[cfg(test)]
mod tests {
    use super::super::audiofile::tests::{drain, fixture, power};
    use super::*;
    use symphonia_core::meta::{MetadataBuilder, MetadataInfo, RawTag, Tag};

    fn check_tone(path: &str, kind: Kind, rate: u32, channels: usize) {
        let mut d = SymFile::open(kind, &fixture(path)).unwrap();
        assert_eq!((d.rate(), d.channels()), (rate, channels), "{path}");
        let pcm = drain(&mut d);
        let frames = pcm.len() / channels;
        assert!((frames as f64 - 1.2 * rate as f64).abs() < 0.1 * rate as f64, "{path}: {frames} frames");
        let tail = &pcm[pcm.len() / 4..];
        assert!(power(tail, channels, rate, 440.0) > 20.0 * power(tail, channels, rate, 1000.0).max(1e-9), "{path}: 440 Hz dominates");
        assert!(tail.iter().any(|&s| s.abs() > 2000), "{path}: has amplitude");
        let mut chunk = Vec::new();
        assert!(!d.next(&mut chunk).unwrap(), "{path}: stays at the end");
        let at = d.seek_ms(600.0).unwrap();
        assert!(at <= 600.0 && at > 400.0, "{path}: landed at {at}");
        let rest = drain(&mut d).len();
        assert!(rest > pcm.len() * 3 / 10 && rest < pcm.len() * 7 / 10, "{path}: {rest} of {}", pcm.len());
    }

    #[test]
    fn flac_decodes_with_length_and_seeks() {
        check_tone("tone.flac", Kind::Flac, 44100, 2);
        let d = SymFile::open(Kind::Flac, &fixture("tone.flac")).unwrap();
        assert!((d.duration_ms().unwrap() - 1200.0).abs() < 20.0, "{:?}", d.duration_ms());
    }

    #[test]
    fn ogg_vorbis_decodes_and_seeks() {
        check_tone("tone.ogg", Kind::Ogg, 44100, 2);
    }

    #[test]
    fn wav_pcm_decodes_mono_22k_and_seeks() {
        check_tone("tone.wav", Kind::Wav, 22050, 1);
    }

    #[test]
    fn wrong_container_or_codec_is_refused() {
        assert!(SymFile::open(Kind::Flac, &fixture("tone.ogg")).is_err());
        assert!(SymFile::open(Kind::Wav, &fixture("tone.flac")).is_err());
        assert!(SymFile::open(Kind::Ogg, &fixture("missing.ogg")).is_err());
    }

    #[test]
    fn tags_come_from_vorbis_comments_and_riff_info() {
        let t = read_tags(Kind::Flac, &fixture("tone.flac")).unwrap();
        assert_eq!((t.title.as_deref(), t.artist.as_deref(), t.album.as_deref()), (Some("Tone FLAC"), Some("Tester"), Some("Fixtures")));
        assert_eq!((t.album_artist.as_deref(), t.track, t.year), (Some("Various"), Some(3), Some(2024)));
        let t = read_tags(Kind::Ogg, &fixture("tone.ogg")).unwrap();
        assert_eq!((t.title.as_deref(), t.track, t.year), (Some("Tone Ogg"), Some(2), Some(2023)));
        let t = read_tags(Kind::Wav, &fixture("tone.wav")).unwrap();
        assert_eq!((t.title.as_deref(), t.artist.as_deref()), (Some("Tone WAV"), Some("Tester")));
    }

    #[test]
    fn tag_mapping_is_case_insensitive_first_wins_and_reads_number_prefixes() {
        let info = MetadataInfo { metadata: symphonia_core::meta::METADATA_ID_NULL, short_name: "t", long_name: "t" };
        let mut b = MetadataBuilder::new(info);
        for (k, v) in [("title", "One"), ("TITLE", "Two"), ("TrackNumber", "7/12"), ("DATE", "1999-12-31"), ("ARTIST", "")] {
            b.add_tag(Tag::new(RawTag::new(k, v)));
        }
        let t = tags_of(&b.build());
        assert_eq!((t.title.as_deref(), t.track, t.year, t.artist), (Some("One"), Some(7), Some(1999), None));
        assert_eq!(leading_number("x"), None);
    }
}
