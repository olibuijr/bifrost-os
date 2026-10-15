//! Pure helpers behind the thumbnail providers: the representation tiers of a request (`QLThumbnailGenerator.Request`
//! `representationTypes`), the entry count of an archive, and the cards drawn for an archive and for audio or video
//! with no embedded cover. The providers in `crate::thumbs` call these before falling back to the type icon.
//!
//! Not built: the duration badge of audio and video (no demuxer runs in the thumbnail worker) and a video poster frame.
use crate::thumbs::canvas::{Canvas, ACCENT, INK, MUTED};
use bifrost_render::ui::Weight;
use bifrost_ui::thumb_service::ThumbArt;

/// `QLThumbnailRepresentation.RepresentationType`, ordered from the cheapest to the best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RepresentationType {
    Icon,
    LowQualityThumbnail,
    Thumbnail,
}

impl RepresentationType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "icon" => Some(Self::Icon),
            "lowQualityThumbnail" => Some(Self::LowQualityThumbnail),
            "thumbnail" => Some(Self::Thumbnail),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::LowQualityThumbnail => "lowQualityThumbnail",
            Self::Thumbnail => "thumbnail",
        }
    }
}

/// Smallest edge of a low quality thumbnail, in pixels.
pub const LOW_QUALITY_MIN_PX: u32 = 16;

/// The tiers to produce for a request of `px` pixels, cheapest first, without repeats. An empty list asks for all.
/// The icon needs no drawing, so its edge is `px`; the low quality tier is a quarter of the edge.
pub fn tiers(types: &[RepresentationType], px: u32) -> Vec<(RepresentationType, u32)> {
    [RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail]
        .into_iter()
        .filter(|t| types.is_empty() || types.contains(t))
        .map(|t| match t {
            RepresentationType::LowQualityThumbnail => (t, (px / 4).max(LOW_QUALITY_MIN_PX)),
            _ => (t, px),
        })
        .collect()
}

/// Entries of a zip, from the tail of the file (the end-of-central-directory record).
pub fn zip_entries(tail: &[u8]) -> Option<usize> {
    let sig = [0x50, 0x4B, 0x05, 0x06];
    let at = tail.windows(4).rposition(|w| w == sig)?;
    let rec = tail.get(at..at + 22)?;
    Some(u16::from_le_bytes([rec[10], rec[11]]) as usize)
}

/// Entries of an uncompressed tar, from its head: 512-byte headers, each followed by its padded data.
/// `None` for anything that is not a tar; counts only what `head` covers.
pub fn tar_entries(head: &[u8]) -> Option<usize> {
    if head.get(257..262)? != b"ustar" {
        return None;
    }
    let (mut pos, mut n) = (0usize, 0usize);
    while let Some(h) = head.get(pos..pos + 512) {
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = std::str::from_utf8(&h[124..135]).ok().and_then(|s| usize::from_str_radix(s.trim_matches(|c| c == ' ' || c == '\0'), 8).ok())?;
        n += 1;
        pos += 512 + size.div_ceil(512) * 512;
    }
    Some(n)
}

/// Entry count of the archive at `path` (zip or plain tar); `None` for other formats.
pub fn archive_entries(path: &std::path::Path) -> Option<usize> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = Vec::new();
    f.by_ref().take(1 << 20).read_to_end(&mut head).ok()?;
    if head.starts_with(b"PK") {
        f.seek(SeekFrom::Start(len.saturating_sub(70_000))).ok()?;
        let mut tail = Vec::new();
        f.take(70_000).read_to_end(&mut tail).ok()?;
        return zip_entries(&tail);
    }
    tar_entries(&head)
}

/// Bar heights (0.15 to 1.0) of the waveform placeholder, the same for the same name.
pub fn waveform(name: &str, bars: usize) -> Vec<f32> {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    (0..bars)
        .map(|_| {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            0.15 + (h % 850) as f32 / 1000.0
        })
        .collect()
}

const CARD_W: f32 = 200.0;
const CARD_H: f32 = 260.0;

fn canvas_for(px: u32) -> Option<Canvas> {
    let h = px as usize;
    let w = ((px as f32 * CARD_W / CARD_H).round() as usize).max(1);
    let mut c = Canvas::new(w, h, CARD_W)?;
    c.page(6.0);
    Some(c)
}

/// An archive: a stack glyph, the format badge and the entry count.
pub fn archive_card(badge: &str, entries: usize, px: u32) -> Option<ThumbArt> {
    let mut c = canvas_for(px)?;
    for i in 0..3 {
        let o = i as f32 * 8.0;
        c.rrect(50.0 + o, 60.0 + (2 - i) as f32 * 8.0, 90.0, 70.0, 4.0, if i == 2 { ACCENT } else { MUTED });
    }
    c.text_centered(badge, 22.0, Weight::Bold, INK, CARD_W / 2.0, 150.0, CARD_W - 24.0);
    let word = if entries == 1 { "item" } else { "items" };
    c.text_centered(&format!("{entries} {word}"), 13.0, Weight::Regular, MUTED, CARD_W / 2.0, 182.0, CARD_W - 24.0);
    c.into_pixels().map(ThumbArt::Pixels)
}

/// Audio or video with no cover: a waveform of bars and the file name.
pub fn waveform_card(name: &str, px: u32) -> Option<ThumbArt> {
    let mut c = canvas_for(px)?;
    for (i, h) in waveform(name, 20).iter().enumerate() {
        let bh = 110.0 * h;
        c.rrect(16.0 + i as f32 * 8.8, 110.0 - bh / 2.0, 5.0, bh, 2.0, ACCENT);
    }
    c.text_centered(name, 12.0, Weight::Regular, MUTED, CARD_W / 2.0, 200.0, CARD_W - 24.0);
    c.into_pixels().map(ThumbArt::Pixels)
}

/// Headless model tests: the tiers of a request are pure values, with no drawing and no file read.
#[cfg(test)]
mod headless_tier_tests {
    use super::*;

    #[test]
    fn an_empty_request_asks_for_every_tier_cheapest_first() {
        assert_eq!(tiers(&[], 128), [(RepresentationType::Icon, 128), (RepresentationType::LowQualityThumbnail, 32), (RepresentationType::Thumbnail, 128)]);
    }

    #[test]
    fn the_low_quality_tier_is_a_quarter_of_the_edge_and_never_below_its_minimum() {
        assert_eq!(tiers(&[RepresentationType::LowQualityThumbnail], 40), [(RepresentationType::LowQualityThumbnail, LOW_QUALITY_MIN_PX)]);
        assert_eq!(tiers(&[RepresentationType::LowQualityThumbnail], 256), [(RepresentationType::LowQualityThumbnail, 64)]);
    }

    #[test]
    fn a_request_gets_each_asked_tier_once_in_the_fixed_order() {
        let asked = [RepresentationType::Thumbnail, RepresentationType::Icon, RepresentationType::Thumbnail];
        assert_eq!(tiers(&asked, 64), [(RepresentationType::Icon, 64), (RepresentationType::Thumbnail, 64)]);
    }

    #[test]
    fn representation_names_round_trip_and_order_from_cheapest() {
        for t in [RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail] {
            assert_eq!(RepresentationType::parse(t.as_str()), Some(t));
        }
        assert_eq!(RepresentationType::parse("poster"), None);
        assert!(RepresentationType::Icon < RepresentationType::Thumbnail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbs::canvas::have_font;

    fn zip_with(entries: u16) -> Vec<u8> {
        let mut v = vec![1, 2, 3];
        v.extend([0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
        v.extend(entries.to_le_bytes());
        v.extend(entries.to_le_bytes());
        v.extend([0u8; 10]);
        v
    }

    fn tar_header(size: usize) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..1].copy_from_slice(b"a");
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[257..262].copy_from_slice(b"ustar");
        h
    }

    #[test]
    fn representation_types_round_trip_by_reference_name() {
        for t in [RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail] {
            assert_eq!(RepresentationType::parse(t.as_str()), Some(t));
        }
        assert_eq!(RepresentationType::parse("poster"), None);
    }

    #[test]
    fn representation_types_limit_the_tiers() {
        assert_eq!(tiers(&[], 128).len(), 3);
        assert_eq!(tiers(&[RepresentationType::Thumbnail], 128), vec![(RepresentationType::Thumbnail, 128)]);
        let t = tiers(&[RepresentationType::Thumbnail, RepresentationType::LowQualityThumbnail], 128);
        assert_eq!(t, vec![(RepresentationType::LowQualityThumbnail, 32), (RepresentationType::Thumbnail, 128)]);
        assert_eq!(tiers(&[RepresentationType::LowQualityThumbnail], 32)[0].1, 16);
    }

    #[test]
    fn a_zip_gives_its_entry_count() {
        assert_eq!(zip_entries(&zip_with(12)), Some(12));
        assert_eq!(zip_entries(b"no record here"), None);
    }

    #[test]
    fn a_tar_gives_its_entry_count() {
        let mut t = tar_header(600);
        t.extend(vec![7u8; 1024]);
        t.extend(tar_header(0));
        t.extend(vec![0u8; 1024]);
        assert_eq!(tar_entries(&t), Some(2));
        assert_eq!(tar_entries(&[1u8; 600]), None);
    }

    #[test]
    fn archives_are_counted_from_disk() {
        let d = std::env::temp_dir().join(format!("bifrost-thumbprov-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut z = b"PK".to_vec();
        z.extend(zip_with(3));
        std::fs::write(d.join("a.zip"), z).unwrap();
        assert_eq!(archive_entries(&d.join("a.zip")), Some(3));
        std::fs::write(d.join("b.bin"), b"xyz").unwrap();
        assert_eq!(archive_entries(&d.join("b.bin")), None);
    }

    #[test]
    fn the_waveform_is_stable_and_bounded() {
        let a = waveform("song.mp3", 20);
        assert_eq!(a, waveform("song.mp3", 20));
        assert_ne!(a, waveform("other.mp3", 20));
        assert!(a.iter().all(|h| (0.15..=1.0).contains(h)));
    }

    #[test]
    fn cards_have_pixels_of_the_requested_height() {
        if !have_font() {
            return;
        }
        for art in [archive_card("ZIP", 5, 128), waveform_card("a.mp3", 128)] {
            let Some(ThumbArt::Pixels(p)) = art else { panic!("pixels") };
            assert_eq!(p.h, 128);
            assert!(p.px.iter().any(|&v| v != 0));
        }
    }
}
