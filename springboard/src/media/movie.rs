//! Movie model for apps: a read-only summary of a movie file (its duration and each track's id, kind, codec and
//! duration), built from the demuxer's parsed tracks. It is a view of the file, not an editor. Movie editing (inserting
//! or removing time ranges, writing a new movie) needs a writer that does not exist yet.

use super::mp4::{Codec, Demuxer, Kind, Track};

/// One track of a movie.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSummary {
    pub id: u32,
    pub kind: Kind,
    /// Codec name: `h264`, `hevc` or `aac`.
    pub codec: &'static str,
    /// Track duration in milliseconds, None when the file does not declare one.
    pub duration_ms: Option<f64>,
}

/// The movie: its duration and its tracks in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    /// Movie duration in milliseconds, None when the file does not declare one.
    pub duration_ms: Option<f64>,
    pub tracks: Vec<TrackSummary>,
}

fn codec_name(codec: &Codec) -> &'static str {
    match codec {
        Codec::H264 { .. } => "h264",
        Codec::Hevc { .. } => "hevc",
        Codec::Aac { .. } => "aac",
    }
}

fn summarize(track: &Track) -> TrackSummary {
    let duration_ms = (track.duration > 0 && track.timescale > 0)
        .then(|| track.duration as f64 * 1000.0 / track.timescale as f64);
    TrackSummary { id: track.id, kind: track.kind, codec: codec_name(&track.codec), duration_ms }
}

impl Movie {
    /// Summarizes a demuxed file.
    pub fn from_demuxer(demuxer: &Demuxer) -> Movie {
        Movie { duration_ms: demuxer.duration_ms(), tracks: demuxer.tracks().iter().map(summarize).collect() }
    }

    /// The first track of `kind`, if the movie has one.
    pub fn track(&self, kind: Kind) -> Option<&TrackSummary> {
        self.tracks.iter().find(|t| t.kind == kind)
    }

    /// True when the movie has no video track.
    pub fn is_audio_only(&self) -> bool {
        self.track(Kind::Video).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::super::source::MemSource;
    use super::*;

    const BASELINE: &[u8] = include_bytes!("testdata/tiny_baseline.mp4");
    const TONE_M4A: &[u8] = include_bytes!("testdata/tone.m4a");

    fn movie(bytes: &[u8]) -> Movie {
        let mut src = MemSource::new(bytes.to_vec());
        let demuxer = Demuxer::open(&mut src).unwrap();
        Movie::from_demuxer(&demuxer)
    }

    #[test]
    fn a_video_file_lists_its_video_and_audio_tracks() {
        let m = movie(BASELINE);
        let video = m.track(Kind::Video).expect("video track");
        assert_eq!(video.codec, "h264");
        assert!(m.track(Kind::Audio).is_some());
        assert!(!m.is_audio_only());
        assert_eq!(m.tracks.len(), 2);
        assert_ne!(m.tracks[0].id, m.tracks[1].id);
    }

    #[test]
    fn the_movie_duration_is_positive_when_declared() {
        let m = movie(BASELINE);
        let duration = m.duration_ms.expect("duration");
        assert!(duration > 0.0, "{duration}");
        let video = m.track(Kind::Video).unwrap();
        assert!(video.duration_ms.unwrap() > 0.0);
    }

    #[test]
    fn an_audio_file_is_audio_only() {
        let m = movie(TONE_M4A);
        assert!(m.is_audio_only());
        assert_eq!(m.track(Kind::Audio).unwrap().codec, "aac");
    }
}
