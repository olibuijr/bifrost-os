//! Pure helpers for Preview style previews: text wrapping, image zoom and
//! pan limits, bounded-decode sizing, and which preview a file gets.
//! Audio never plays by itself: the preview offers a Play button and only an
//! explicit tap hands the file to the music player.

use super::fs::Kind;

/// What opening a file shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    Text,
    Image,
    /// a card with a Play button; nothing plays until it is tapped
    Audio,
    /// first page rendered with the Books PDF engine
    Pdf,
    /// size and item count of a folder
    FolderSummary,
    Video,
    /// no content preview: just the info card
    Info,
}

pub fn is_pdf(name: &str) -> bool {
    super::fs::extension(name).as_deref() == Some("pdf")
}

pub fn preview_for(name: &str, kind: Kind) -> Preview {
    if is_pdf(name) {
        return Preview::Pdf;
    }
    match kind {
        Kind::Folder => Preview::FolderSummary,
        Kind::Text => Preview::Text,
        Kind::Image => Preview::Image,
        Kind::Audio => Preview::Audio,
        Kind::Video => Preview::Video,
        _ => Preview::Info,
    }
}

/// Wraps `text` to `cols` characters per line: at spaces where possible,
/// hard-breaking words longer than a line. Existing line breaks stay, and
/// blank lines survive. Returns (display line, source line number) pairs so
/// a gutter can number the source lines once.
pub fn wrap_lines(text: &str, cols: usize) -> Vec<(String, usize)> {
    let cols = cols.max(1);
    let mut out = vec![];
    for (n, line) in text.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() <= cols {
            out.push((line.to_string(), n + 1));
            continue;
        }
        let mut start = 0;
        while start < chars.len() {
            let end = (start + cols).min(chars.len());
            if end == chars.len() {
                out.push((chars[start..].iter().collect(), n + 1));
                break;
            }
            // break after the last space inside the window, if there is one past the start
            let cut = chars[start..=end].iter().rposition(|c| *c == ' ').filter(|&i| i > 0).map(|i| start + i);
            match cut {
                Some(c) => {
                    out.push((chars[start..c].iter().collect(), n + 1));
                    start = c + 1;
                }
                None => {
                    out.push((chars[start..end].iter().collect(), n + 1));
                    start = end;
                }
            }
        }
    }
    out
}

pub const MIN_ZOOM: f32 = 1.0;
pub const MAX_ZOOM: f32 = 8.0;

/// A zoomed image: scale relative to fit-to-view and the offset of its
/// centre from the view's centre, in view points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub scale: f32,
    pub pan: (f32, f32),
}

impl Default for Zoom {
    fn default() -> Self {
        Zoom { scale: 1.0, pan: (0.0, 0.0) }
    }
}

impl Zoom {
    /// Applies a pinch factor about `focus` (view points from the view
    /// centre), keeping the point under the fingers still, then re-limits.
    pub fn pinch(&mut self, factor: f32, focus: (f32, f32), fitted: (f32, f32), view: (f32, f32)) {
        let new = (self.scale * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = new / self.scale;
        self.pan = (focus.0 - (focus.0 - self.pan.0) * k, focus.1 - (focus.1 - self.pan.1) * k);
        self.scale = new;
        self.limit(fitted, view);
    }

    pub fn drag(&mut self, dx: f32, dy: f32, fitted: (f32, f32), view: (f32, f32)) {
        self.pan.0 += dx;
        self.pan.1 += dy;
        self.limit(fitted, view);
    }

    /// Double tap: zoom in to 2.5x about `focus`, or back to fit.
    pub fn toggle(&mut self, focus: (f32, f32), fitted: (f32, f32), view: (f32, f32)) {
        if self.scale > 1.01 {
            *self = Zoom::default();
        } else {
            self.pinch(2.5, focus, fitted, view);
        }
    }

    /// Keeps the picture covering the view along any axis where it is larger,
    /// and centred where it is smaller.
    pub fn limit(&mut self, fitted: (f32, f32), view: (f32, f32)) {
        let scale = self.scale;
        let clamp = |pan: f32, content: f32, port: f32| {
            let over = (content * scale - port) / 2.0;
            if over <= 0.0 {
                0.0
            } else {
                pan.clamp(-over, over)
            }
        };
        self.pan = (clamp(self.pan.0, fitted.0, view.0), clamp(self.pan.1, fitted.1, view.1));
    }
}

/// Largest decode a preview allows, in pixels.
pub const MAX_PREVIEW_PIXELS: u64 = 16_000_000;
/// Largest image file a preview reads.
pub const MAX_IMAGE_BYTES: u64 = 40 << 20;

/// Whether an image of `w` x `h` pixels in a `bytes`-long file is safe to decode.
pub fn decode_allowed(w: u32, h: u32, bytes: u64) -> bool {
    w > 0 && h > 0 && (w as u64) * (h as u64) <= MAX_PREVIEW_PIXELS && bytes <= MAX_IMAGE_BYTES
}

/// PNG/JPEG pixel size from the file header, without decoding (None when unknown).
pub fn image_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() >= 24 && head[..8] == [0x89, b'P', b'N', b'G', 13, 10, 26, 10] {
        let w = u32::from_be_bytes(head[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(head[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if head.len() > 4 && head[0] == 0xFF && head[1] == 0xD8 {
        let mut p = 2;
        while p + 9 < head.len() {
            if head[p] != 0xFF {
                p += 1;
                continue;
            }
            let m = head[p + 1];
            if m == 0xFF {
                p += 1;
                continue;
            }
            if (0xC0..=0xCF).contains(&m) && ![0xC4, 0xC8, 0xCC].contains(&m) {
                let h = u16::from_be_bytes([head[p + 5], head[p + 6]]) as u32;
                let w = u16::from_be_bytes([head[p + 7], head[p + 8]]) as u32;
                return Some((w, h));
            }
            if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
                p += 2;
                continue;
            }
            let len = u16::from_be_bytes([head[p + 2], head[p + 3]]) as usize;
            p += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_preview() {
        assert_eq!(preview_for("a.txt", Kind::Text), Preview::Text);
        assert_eq!(preview_for("a.PDF", Kind::Other), Preview::Pdf);
        assert_eq!(preview_for("song.mp3", Kind::Audio), Preview::Audio);
        assert_eq!(preview_for("d", Kind::Folder), Preview::FolderSummary);
        assert_eq!(preview_for("x.bin", Kind::Other), Preview::Info);
        assert_eq!(preview_for("p.png", Kind::Image), Preview::Image);
    }

    #[test]
    fn wraps_at_spaces_and_hard_breaks() {
        let w = |s: &str, c| wrap_lines(s, c).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(w("the quick brown fox", 10), ["the quick", "brown fox"]);
        assert_eq!(w("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        assert_eq!(w("a\n\nb", 5), ["a", "", "b"]);
        assert_eq!(w("short", 80), ["short"]);
        assert_eq!(w("aaaa bbbb cccc", 9), ["aaaa bbbb", "cccc"]);
        assert_eq!(w("", 5), [""]);
    }

    #[test]
    fn wrapped_lines_keep_source_numbers() {
        let l = wrap_lines("one two three four\nfive", 8);
        let nums: Vec<usize> = l.iter().map(|(_, n)| *n).collect();
        assert_eq!(*nums.last().unwrap(), 2);
        assert!(nums[..nums.len() - 1].iter().all(|&n| n == 1));
        // no text is lost
        let joined: String = l.iter().filter(|(_, n)| *n == 1).map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(" ");
        assert_eq!(joined, "one two three four");
    }

    #[test]
    fn zoom_limits_and_focus() {
        let (fit, view) = ((400.0, 300.0), (500.0, 500.0));
        let mut z = Zoom::default();
        z.pinch(20.0, (0.0, 0.0), fit, view);
        assert_eq!(z.scale, MAX_ZOOM);
        z.pinch(0.001, (0.0, 0.0), fit, view);
        assert_eq!(z, Zoom::default());
        // smaller than the view: stays centred
        z.drag(50.0, 50.0, fit, view);
        assert_eq!(z.pan, (0.0, 0.0));
        // zoomed: pan is bounded by the overhang (400*2 - 500)/2 = 150
        z.pinch(2.0, (0.0, 0.0), fit, view);
        z.drag(1000.0, 0.0, fit, view);
        assert_eq!(z.pan.0, 150.0);
        assert_eq!(z.pan.1, 0.0);
    }

    #[test]
    fn double_tap_toggles() {
        let (fit, view) = ((400.0, 400.0), (400.0, 400.0));
        let mut z = Zoom::default();
        z.toggle((0.0, 0.0), fit, view);
        assert_eq!(z.scale, 2.5);
        z.toggle((0.0, 0.0), fit, view);
        assert_eq!(z, Zoom::default());
    }

    #[test]
    fn decode_bounds() {
        assert!(decode_allowed(4000, 3000, 5 << 20));
        assert!(!decode_allowed(10_000, 10_000, 5 << 20));
        assert!(!decode_allowed(100, 100, 100 << 20));
        assert!(!decode_allowed(0, 10, 1));
    }

    #[test]
    fn reads_sizes_from_headers() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));
        // JPEG: SOI, APP0 (len 4), SOF0 with 300 high, 200 wide
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 1, 44, 0, 200, 3, 0, 0, 0];
        assert_eq!(image_size(&jpg), Some((200, 300)));
        assert_eq!(image_size(b"nope"), None);
    }
}
