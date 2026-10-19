//! The length badge of audio and video thumbnails: a small dark pill with the running time ("3:25", "1:02:03") in the
//! bottom right corner, drawn over the cover, the poster frame or the waveform card. The length comes from the file's
//! own header (the MP4 movie header, the FLAC, WAV and Ogg stream info, the MP3 frame count), read by the media layer
//! without decoding any sound or picture.

use super::canvas::Canvas;
use crate::media::audiofile;
use crate::media::mp4::Demuxer;
use crate::media::source::FileSource;
use bifrost_render::gfx::pack;
use bifrost_render::ui::Weight;
use bifrost_ui::thumb_service::Pixels;
use std::path::Path;

/// Thumbnails whose shorter side is below this many pixels get no badge: the text would not be readable.
pub const MIN_BADGE_EDGE: usize = 48;
/// The longest audio or video file whose length is read; a bigger file keeps its plain thumbnail.
pub const MAX_FILE_BYTES: u64 = 8 << 30;

const PILL: u32 = pack(0, 0, 0, 165);
const TEXT: u32 = pack(255, 255, 255, 255);

/// The running time as the badge writes it: `m:ss` under an hour, `h:mm:ss` from an hour, rounded to the nearest
/// second. `None` for a length that is negative, not finite or longer than 99 hours.
pub fn format_duration(secs: f64) -> Option<String> {
    if !secs.is_finite() || secs < 0.0 || secs > 99.0 * 3600.0 {
        return None;
    }
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    Some(if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") })
}

/// The length in seconds of the audio or video file at `path`, from its header. `None` when the file declares none,
/// cannot be read or is not audio or video.
pub fn duration_secs(path: &Path) -> Option<f64> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let from_container = FileSource::open(path).ok().and_then(|mut src| Demuxer::open(&mut src).ok()).and_then(|d| d.duration_ms());
    let ms = from_container.or_else(|| audiofile::open(path).ok().and_then(|d| d.duration_ms()))?;
    (ms.is_finite() && ms > 0.0).then_some(ms / 1000.0)
}

/// `pixels` with the length badge over the bottom right corner. The picture comes back unchanged when it is too small
/// for a readable badge, the length cannot be written, or there is no font.
pub fn badge(pixels: Pixels, secs: f64) -> Pixels {
    let Some(text) = format_duration(secs) else { return pixels };
    if pixels.w.min(pixels.h) < MIN_BADGE_EDGE {
        return pixels;
    }
    // keep the badge about the same share of the picture at every size: 128 pixels is one point per pixel
    let scale = (pixels.w.min(pixels.h) as f32 / 128.0).max(0.75);
    let kept = pixels.clone();
    let Some(mut c) = Canvas::over(pixels, kept.w as f32 / scale) else { return kept };
    let (pt, pad, margin) = (11.0, 5.0, 5.0);
    let (text_w, pill_h) = (c.width(&text, pt, Weight::Semibold), 16.0);
    let (w, h) = (c.w as f32 / c.s, c.h as f32 / c.s);
    let pill_w = text_w + 2.0 * pad;
    let (x, y) = (w - margin - pill_w, h - margin - pill_h);
    c.rrect(x, y, pill_w, pill_h, pill_h / 2.0, PILL);
    c.text(&text, pt, Weight::Semibold, TEXT, x + pad, y + 2.5, text_w + 1.0);
    c.into_pixels().unwrap_or(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbs::canvas::have_font;

    #[test]
    fn a_length_reads_as_minutes_and_seconds_then_hours() {
        assert_eq!(format_duration(0.0).as_deref(), Some("0:00"));
        assert_eq!(format_duration(7.4).as_deref(), Some("0:07"));
        assert_eq!(format_duration(7.6).as_deref(), Some("0:08"));
        assert_eq!(format_duration(205.0).as_deref(), Some("3:25"));
        assert_eq!(format_duration(3599.4).as_deref(), Some("59:59"));
        assert_eq!(format_duration(3723.0).as_deref(), Some("1:02:03"));
        assert_eq!(format_duration(36_000.0).as_deref(), Some("10:00:00"));
    }

    #[test]
    fn a_length_that_cannot_be_written_has_no_text() {
        for bad in [-1.0, f64::NAN, f64::INFINITY, 100.0 * 3600.0] {
            assert_eq!(format_duration(bad), None, "{bad}");
        }
    }

    /// A mono 16-bit PCM WAV at 8000 Hz: `frames` samples of silence.
    fn wav(frames: u32) -> Vec<u8> {
        let data = frames * 2;
        let mut v = b"RIFF".to_vec();
        v.extend((36 + data).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(8000u32.to_le_bytes());
        v.extend(16000u32.to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend(data.to_le_bytes());
        v.extend(vec![0u8; data as usize]);
        v
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-duration-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_wav_file_gives_its_length_from_the_header() {
        let dir = scratch("wav");
        let file = dir.join("two.wav");
        std::fs::write(&file, wav(16_000)).unwrap();
        let secs = duration_secs(&file).expect("a wav declares its length");
        assert!((secs - 2.0).abs() < 0.01, "{secs}");
    }

    #[test]
    fn a_missing_or_foreign_file_has_no_length() {
        let dir = scratch("none");
        assert_eq!(duration_secs(&dir.join("nope.mp3")), None);
        let junk = dir.join("junk.mp4");
        std::fs::write(&junk, b"not media at all").unwrap();
        assert_eq!(duration_secs(&junk), None);
        assert_eq!(duration_secs(&dir), None, "a folder is not a recording");
    }

    fn grey(w: usize, h: usize) -> Pixels {
        Pixels::new(w, h, vec![pack(120, 120, 120, 255); w * h]).unwrap()
    }

    #[test]
    fn the_badge_marks_the_bottom_right_corner_only() {
        if !have_font() {
            return;
        }
        let before = grey(160, 120);
        let after = badge(before.clone(), 205.0);
        assert_eq!((after.w, after.h), (160, 120));
        assert_eq!(after.px[0], before.px[0], "the top left is untouched");
        let corner = after.px[(120 - 14) * 160 + (160 - 30)];
        assert_ne!(corner, before.px[0], "the pill darkens the picture under it");
        let changed = after.px.iter().zip(&before.px).filter(|(a, b)| a != b).count();
        assert!(changed > 200 && changed < 160 * 120 / 4, "a small pill, not a wash: {changed}");
    }

    #[test]
    fn a_thumbnail_too_small_for_text_and_an_unwritable_length_stay_plain() {
        let tiny = grey(32, 32);
        assert_eq!(badge(tiny.clone(), 60.0), tiny);
        let big = grey(128, 128);
        assert_eq!(badge(big.clone(), f64::NAN), big);
    }
}
