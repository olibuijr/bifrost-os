//! Screenshots: the frame is saved as `Screenshot-YYYYMMDD-HHMMSS.png`
//! and the bifrost capture animation plays over whatever is on screen: a white
//! flash, the frame shrinking into a bordered thumbnail at the bottom left,
//! a four-second hold (a tap opens it in Photos) and a slide out to the left.

use super::gestures::ease_out_quartic;
use super::photos::library::{stamp, Date};
use super::switcher::draw_scaled;
use crate::gfx::{pack, Image, Surface};
use crate::layout::Rect;
use std::path::{Path, PathBuf};

/// The white flash fades over this long.
pub const FLASH_MS: f64 = 140.0;
/// The frame shrinks into its thumbnail over this long, after the flash begins.
pub const SHRINK_MS: f64 = 420.0;
/// The thumbnail stays until this long after the capture.
pub const HOLD_MS: f64 = 4000.0;
/// Then it slides off the left edge over this long.
pub const OUT_MS: f64 = 260.0;
/// Thumbnail width as a fraction of the screen.
const THUMB_FRACTION: f32 = 0.2;
const MARGIN_PT: f32 = 12.0;
const BOTTOM_PT: f32 = 52.0;
const BORDER_PT: f32 = 2.5;
/// Corner radius in px of the white frame around the thumbnail (rule: at most 6).
const FRAME_RADIUS_PX: f32 = 6.0;
const WHITE: u32 = pack(255, 255, 255, 255);

/// The file name for a capture taken at `d`.
pub fn file_name(d: &Date) -> String {
    format!("Screenshot-{}.png", stamp(d))
}

/// Encodes `frame` as a PNG; the caller writes the bytes (see `write`).
pub fn encode(frame: &Image) -> Vec<u8> {
    crate::pngenc::encode_rgb(frame.w, frame.h, &frame.px)
}

/// Writes PNG `bytes` to `dir/<name>` through a temporary file, dating the
/// file `unix` so the library orders it where the capture happened.
pub fn write(dir: &Path, name: &str, bytes: &[u8], unix: i64) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix.max(0) as u64));
    }
    Ok(path)
}

/// The capture animation over the screen.
pub struct Overlay {
    frame: Image,
    thumb: Image,
    pub path: PathBuf,
    start: f64,
    w: i32,
    h: i32,
    /// px per point
    s: f32,
}

impl Overlay {
    /// Starts the animation at `t` for `frame` (the whole screen) saved at `path`.
    pub fn new(frame: Image, path: PathBuf, s: f32, t: f64) -> Self {
        let (w, h) = (frame.w as i32, frame.h as i32);
        let r = thumb_rect(w, h, s);
        let thumb = frame.resized(r.w as usize, r.h as usize);
        Overlay { frame, thumb, path, start: t, w, h, s }
    }

    /// The captured frame (what Markup edits).
    pub fn frame(&self) -> &Image {
        &self.frame
    }

    /// Where the thumbnail rests.
    pub fn thumb_rect(&self) -> Rect {
        thumb_rect(self.w, self.h, self.s)
    }

    /// The frame's box at `t`: shrinking from the full screen to the thumbnail, then sliding out.
    pub fn rect(&self, t: f64) -> Rect {
        let dt = t - self.start;
        let thumb = self.thumb_rect();
        if dt < FLASH_MS {
            return Rect { x: 0, y: 0, w: self.w, h: self.h };
        }
        if dt < FLASH_MS + SHRINK_MS {
            let k = ease_out_quartic(((dt - FLASH_MS) / SHRINK_MS) as f32);
            let lerp = |a: i32, b: i32| (a as f32 + (b as f32 - a as f32) * k).round() as i32;
            return Rect { x: lerp(0, thumb.x), y: lerp(0, thumb.y), w: lerp(self.w, thumb.w), h: lerp(self.h, thumb.h) };
        }
        if dt >= HOLD_MS {
            let k = ease_out_quartic(((dt - HOLD_MS) / OUT_MS) as f32);
            let gone = -(thumb.x + thumb.w) - (BORDER_PT * self.s) as i32;
            return Rect { x: thumb.x + ((gone - thumb.x) as f32 * k).round() as i32, ..thumb };
        }
        thumb
    }

    /// Opacity of the white flash at `t`.
    pub fn flash(&self, t: f64) -> u32 {
        let k = ((t - self.start) / FLASH_MS).clamp(0.0, 1.0);
        (255.0 * (1.0 - k)) as u32
    }

    pub fn done(&self, t: f64) -> bool {
        t - self.start >= HOLD_MS + OUT_MS
    }

    /// True while something moves (the flash and shrink, the slide out).
    pub fn animating(&self, t: f64) -> bool {
        let dt = t - self.start;
        dt < FLASH_MS + SHRINK_MS || (dt >= HOLD_MS && !self.done(t))
    }

    /// When the next frame is due: now while moving, else when the hold ends.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.done(t) {
            None
        } else if self.animating(t) {
            Some(t)
        } else {
            Some(self.start + HOLD_MS)
        }
    }

    /// A tap on the resting thumbnail (opens the capture in Photos).
    pub fn hit(&self, x: f32, y: f32, t: f64) -> bool {
        let dt = t - self.start;
        dt >= FLASH_MS + SHRINK_MS && dt < HOLD_MS && self.thumb_rect().contains(x, y)
    }

    pub fn draw(&self, s: &mut Surface, t: f64) {
        let r = self.rect(t);
        let full = r.w == self.w && r.h == self.h;
        let border = BORDER_PT * self.s;
        let radius = if full { 0.0 } else { (FRAME_RADIUS_PX - border).max(0.0) };
        if !full {
            s.fill_rrect(r.x as f32 - border, r.y as f32 - border, r.w as f32 + 2.0 * border, r.h as f32 + 2.0 * border, radius + border, WHITE);
        }
        if (r.w as usize, r.h as usize) == (self.thumb.w, self.thumb.h) {
            draw_scaled(s, &self.thumb, r, radius, 255);
        } else {
            draw_scaled(s, &self.frame, r, radius, 255);
        }
        let flash = self.flash(t);
        if flash > 0 {
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, pack(255, 255, 255, 255) & 0x00ff_ffff | flash << 24 & 0xff00_0000);
        }
    }
}

/// The thumbnail's resting box: bottom left, above the home indicator.
fn thumb_rect(w: i32, h: i32, s: f32) -> Rect {
    let tw = (w as f32 * THUMB_FRACTION).round() as i32;
    let th = (tw as f32 * h as f32 / w as f32).round() as i32;
    Rect { x: (MARGIN_PT * s) as i32, y: h - (BOTTOM_PT * s) as i32 - th, w: tw, h: th }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::photos::library::civil;

    #[test]
    fn captures_are_named_by_the_local_time_and_written_dated() {
        let d = civil(1_791_073_395, 0);
        assert_eq!(file_name(&d), "Screenshot-20261004-002315.png");
        let dir = crate::shell::photos::library::tests::scratch("screenshot");
        let img = Image { w: 5, h: 3, px: vec![pack(9, 8, 7, 255); 15] };
        let bytes = encode(&img);
        let path = write(&dir, &file_name(&d), &bytes, 1_791_073_395).unwrap();
        assert!(path.ends_with("Screenshot-20261004-002315.png"));
        let back = crate::cg::imageio::decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((back.w, back.h, back.px[7]), (5, 3, pack(9, 8, 7, 255)));
        let photo = crate::shell::photos::library::Photo::from_path(path).unwrap();
        assert_eq!(photo.mtime, 1_791_073_395);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_overlay_flashes_shrinks_holds_and_slides_out() {
        let frame = Image { w: 1200, h: 1920, px: vec![pack(1, 2, 3, 255); 1200 * 1920] };
        let o = Overlay::new(frame, PathBuf::from("/x.png"), 1200.0 / 768.0, 1000.0);
        assert_eq!(o.rect(1000.0), Rect { x: 0, y: 0, w: 1200, h: 1920 });
        assert_eq!(o.flash(1000.0), 255);
        assert_eq!(o.flash(1000.0 + FLASH_MS), 0);
        assert!(o.animating(1050.0));
        let mid = o.rect(1000.0 + FLASH_MS + SHRINK_MS / 2.0);
        assert!(mid.w < 1200 && mid.w > o.thumb_rect().w, "shrinking: {mid:?}");
        let rest = 1000.0 + FLASH_MS + SHRINK_MS + 100.0;
        assert_eq!(o.rect(rest), o.thumb_rect());
        assert!(!o.animating(rest));
        assert_eq!(o.next_wake(rest), Some(1000.0 + HOLD_MS), "nothing to draw until the hold ends");
        let th = o.thumb_rect();
        assert_eq!(th.w, 240);
        assert_eq!(th.h, 384);
        assert!(th.y + th.h < 1920 && th.x > 0);
        assert!(o.hit(th.x as f32 + 5.0, th.y as f32 + 5.0, rest));
        assert!(!o.hit(th.x as f32 + 5.0, th.y as f32 + 5.0, 1100.0), "not while shrinking");
        assert!(!o.hit(600.0, 600.0, rest));
        let out = 1000.0 + HOLD_MS + OUT_MS / 2.0;
        assert!(o.animating(out) && o.rect(out).x < th.x);
        assert!(o.done(1000.0 + HOLD_MS + OUT_MS));
        assert!(o.rect(1000.0 + HOLD_MS + OUT_MS).x + th.w < 0, "off screen");
        assert_eq!(o.next_wake(1000.0 + HOLD_MS + OUT_MS), None);
        let mut buf = vec![0u32; 1200 * 1920];
        for t in [1000.0, 1300.0, rest, out] {
            o.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        }
        let inside = buf[(th.y + th.h / 2) as usize * 1200 + (th.x + th.w / 2) as usize];
        assert_ne!(inside, 0);
    }
}
