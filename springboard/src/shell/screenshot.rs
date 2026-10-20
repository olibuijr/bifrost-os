//! Screenshots: the picture is saved as `Screenshot-YYYYMMDD-HHMMSS.png`
//! and the bifrost capture animation plays over whatever is on screen: a white
//! flash, the picture shrinking into a bordered thumbnail at the bottom right,
//! a five-second hold (a click opens it in Markup, a drag moves it) and a slide out to the right.

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
pub const HOLD_MS: f64 = 5000.0;
/// Then it slides off the right edge over this long.
pub const OUT_MS: f64 = 260.0;
/// Thumbnail width as a fraction of the screen.
const THUMB_FRACTION: f32 = 0.2;
const MARGIN_PT: f32 = 12.0;
const BOTTOM_PT: f32 = 52.0;
const BORDER_PT: f32 = 2.5;
/// A press on the thumbnail becomes a drag once the pointer has moved this far, in points; before that it is a tap.
pub const DRAG_PT: f32 = 6.0;
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

/// Encodes `frame` as an RGBA PNG: a window capture keeps its transparent border and shadow.
pub fn encode_alpha(frame: &Image) -> Vec<u8> {
    crate::pngenc::encode_rgba(frame.w, frame.h, &frame.px)
}

/// The user and group that own what the shell saves for the session's account: the installed account when the shell runs
/// as root (the session service does), else nobody to hand the file to (the process owns it already).
pub fn account_owner() -> Option<(u32, u32)> {
    #[cfg(target_os = "linux")]
    if crate::appearance_portal::is_root() {
        return crate::bare::account::Account::installed().ok().map(|a| (a.uid, a.gid));
    }
    None
}

/// Writes PNG `bytes` to `dir/<name>` through a temporary file, dating the
/// file `unix` so the library orders it where the capture happened.
pub fn write(dir: &Path, name: &str, bytes: &[u8], unix: i64) -> std::io::Result<PathBuf> {
    write_as(dir, name, bytes, unix, None)
}

/// `write`, handing the folder (when it had to be made) and the file to `owner` (uid, gid): a root shell saving into the
/// account's Desktop must not leave a file the account cannot move or delete.
pub fn write_as(dir: &Path, name: &str, bytes: &[u8], unix: i64, owner: Option<(u32, u32)>) -> std::io::Result<PathBuf> {
    let made = !dir.exists();
    std::fs::create_dir_all(dir)?;
    if let (true, Some((uid, gid))) = (made, owner) {
        let _ = std::os::unix::fs::chown(dir, Some(uid), Some(gid));
    }
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    if let Some((uid, gid)) = owner {
        std::os::unix::fs::chown(&tmp, Some(uid), Some(gid))?;
    }
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
    /// How far the thumbnail has been dragged from its resting place, in px.
    offset: (i32, i32),
    /// The press that may become a drag: where it landed, and the offset it started from.
    grab: Option<(f32, f32, (i32, i32))>,
    /// Whether the press has moved far enough to be a drag.
    moved: bool,
}

impl Overlay {
    /// Starts the animation at `t` for `frame` (the whole screen) saved at `path`.
    pub fn new(frame: Image, path: PathBuf, s: f32, t: f64) -> Self {
        let (w, h) = (frame.w as i32, frame.h as i32);
        let r = thumb_rect(w, h, s);
        let thumb = frame.resized(r.w as usize, r.h as usize);
        Overlay { frame, thumb, path, start: t, w, h, s, offset: (0, 0), grab: None, moved: false }
    }

    /// The captured frame (what Markup edits).
    pub fn frame(&self) -> &Image {
        &self.frame
    }

    /// Where the thumbnail rests: its corner, moved by any drag and kept on the screen.
    pub fn thumb_rect(&self) -> Rect {
        let base = thumb_rect(self.w, self.h, self.s);
        Rect {
            x: (base.x + self.offset.0).clamp(0, (self.w - base.w).max(0)),
            y: (base.y + self.offset.1).clamp(0, (self.h - base.h).max(0)),
            ..base
        }
    }

    /// A press on the resting thumbnail. It is a tap unless the pointer then moves (see `drag_to`).
    pub fn grab(&mut self, x: f32, y: f32) {
        self.grab = Some((x, y, self.offset));
        self.moved = false;
    }

    /// The pointer moved while it holds the thumbnail: the thumbnail follows it, kept on the screen.
    pub fn drag_to(&mut self, x: f32, y: f32) {
        let Some((gx, gy, start)) = self.grab else { return };
        let (dx, dy) = ((x - gx).round() as i32, (y - gy).round() as i32);
        if !self.moved && dx.abs().max(dy.abs()) as f32 <= DRAG_PT * self.s {
            return;
        }
        self.moved = true;
        let base = thumb_rect(self.w, self.h, self.s);
        let ox = (start.0 + dx).clamp(-base.x, (self.w - base.w - base.x).max(-base.x));
        let oy = (start.1 + dy).clamp(-base.y, (self.h - base.h - base.y).max(-base.y));
        self.offset = (ox, oy);
    }

    /// Ends the press. Returns true when it was a drag (so it is not a tap).
    pub fn release(&mut self) -> bool {
        let moved = self.moved;
        self.grab = None;
        self.moved = false;
        moved
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
            // the frame ends just past the right edge, border included
            let gone = self.w + (BORDER_PT * self.s).ceil() as i32;
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

    /// Whether the thumbnail rests where a click or a drag can take it (not while it flashes and shrinks, nor once it slides away).
    pub fn interactive(&self, t: f64) -> bool {
        let dt = t - self.start;
        dt >= FLASH_MS + SHRINK_MS && dt < HOLD_MS
    }

    /// A tap on the resting thumbnail (opens the capture in Markup).
    pub fn hit(&self, x: f32, y: f32, t: f64) -> bool {
        self.interactive(t) && self.thumb_rect().contains(x, y)
    }

    /// The part of the screen the animation covers at `t`, clipped to the screen: all of it while the flash and the shrink
    /// run, then the thumbnail and its border. The desktop shows the animation as its own surface of this size, so nothing
    /// outside it is touched and nothing is left behind once it is gone.
    pub fn bounds(&self, t: f64) -> Rect {
        if t - self.start < FLASH_MS + SHRINK_MS {
            return Rect { x: 0, y: 0, w: self.w, h: self.h };
        }
        let r = self.rect(t);
        let b = (BORDER_PT * self.s).ceil() as i32;
        let (x0, y0) = ((r.x - b).max(0), (r.y - b).max(0));
        let (x1, y1) = ((r.x + r.w + b).min(self.w), (r.y + r.h + b).min(self.h));
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }

    /// Draws the whole animation on a screen-sized surface.
    pub fn draw(&self, s: &mut Surface, t: f64) {
        self.draw_at(s, t, (0, 0));
    }

    /// Draws the animation on a surface whose top left is the screen point `origin` (see `bounds`).
    pub fn draw_at(&self, s: &mut Surface, t: f64, origin: (i32, i32)) {
        let mut r = self.rect(t);
        let full = r.w == self.w && r.h == self.h;
        r.x -= origin.0;
        r.y -= origin.1;
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
            s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, pack(255, 255, 255, 255) & 0x00ff_ffff | flash << 24 & 0xff00_0000);
        }
    }
}

/// The thumbnail's resting box: bottom right, clear of the screen edge by the margin.
fn thumb_rect(w: i32, h: i32, s: f32) -> Rect {
    let tw = (w as f32 * THUMB_FRACTION).round() as i32;
    let th = (tw as f32 * h as f32 / w as f32).round() as i32;
    Rect { x: w - (MARGIN_PT * s) as i32 - tw, y: h - (BOTTOM_PT * s) as i32 - th, w: tw, h: th }
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
    fn a_drag_moves_the_thumbnail_and_keeps_it_on_screen() {
        let frame = Image { w: 1200, h: 1920, px: vec![pack(1, 2, 3, 255); 1200 * 1920] };
        let mut o = Overlay::new(frame, PathBuf::from("/x.png"), 1200.0 / 768.0, 0.0);
        let rest = o.thumb_rect();
        o.grab(100.0, 100.0);
        o.drag_to(102.0, 101.0);
        assert!(!o.release(), "a small move is still a tap");
        o.grab(100.0, 100.0);
        o.drag_to(40.0, 90.0);
        assert!(o.release(), "a move past the slop is a drag");
        let moved = o.thumb_rect();
        assert_eq!((moved.x - rest.x, moved.y - rest.y), (-60, -10));
        o.grab(0.0, 0.0);
        o.drag_to(-5000.0, 5000.0);
        o.release();
        let far = o.thumb_rect();
        assert!(far.x >= 0 && far.y >= 0 && far.x + far.w <= 1200 && far.y + far.h <= 1920, "kept on screen: {far:?}");
        assert_eq!(far.x, 0);
        assert_eq!(far.y + far.h, 1920);
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
        assert!(th.y + th.h < 1920 && th.x + th.w < 1200, "the thumbnail rests above the bottom edge and clear of the right one");
        assert!(th.x > 1200 / 2, "at the bottom right, not the bottom left");
        assert!(o.hit(th.x as f32 + 5.0, th.y as f32 + 5.0, rest));
        assert!(!o.hit(th.x as f32 + 5.0, th.y as f32 + 5.0, 1100.0), "not while shrinking");
        assert!(!o.hit(600.0, 600.0, rest));
        let out = 1000.0 + HOLD_MS + OUT_MS / 2.0;
        assert!(o.animating(out) && o.rect(out).x > th.x, "it slides to the right");
        assert!(o.done(1000.0 + HOLD_MS + OUT_MS));
        assert!(o.rect(1000.0 + HOLD_MS + OUT_MS).x >= 1200, "off screen at the right");
        assert_eq!(HOLD_MS, 5000.0, "the thumbnail slides away after about five seconds");
        assert_eq!(o.next_wake(1000.0 + HOLD_MS + OUT_MS), None);
        let mut buf = vec![0u32; 1200 * 1920];
        for t in [1000.0, 1300.0, rest, out] {
            o.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        }
        let inside = buf[(th.y + th.h / 2) as usize * 1200 + (th.x + th.w / 2) as usize];
        assert_ne!(inside, 0);
    }

    #[test]
    fn the_desktop_surface_covers_the_screen_while_it_flashes_then_only_the_thumbnail() {
        let frame = Image { w: 1200, h: 1920, px: vec![pack(1, 2, 3, 255); 1200 * 1920] };
        let o = Overlay::new(frame, PathBuf::from("/x.png"), 1200.0 / 768.0, 1000.0);
        assert_eq!(o.bounds(1000.0), Rect { x: 0, y: 0, w: 1200, h: 1920 });
        let rest = 1000.0 + FLASH_MS + SHRINK_MS + 100.0;
        let b = o.bounds(rest);
        let th = o.thumb_rect();
        assert!(b.x <= th.x && b.y <= th.y && b.x + b.w >= th.x + th.w && b.y + b.h >= th.y + th.h, "{b:?} holds {th:?}");
        assert!(b.w < 400 && b.h < 600, "only the thumbnail and its border: {b:?}");
        // sliding out, the surface is clipped to the screen and ends empty
        let end = o.bounds(1000.0 + HOLD_MS + OUT_MS);
        assert!(end.w <= 0 || end.x >= 1200 - 1, "nothing is left on screen once it has slid away: {end:?}");
        // the thumbnail drawn on its own surface matches the same pixels of the full-screen drawing
        let mut full = vec![0u32; 1200 * 1920];
        o.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut full, y0: 0 }, rest);
        let mut local = vec![0u32; b.w as usize * b.h as usize];
        o.draw_at(&mut Surface { w: b.w as usize, h: b.h as usize, stride: b.w as usize, px: &mut local, y0: 0 }, rest, (b.x, b.y));
        let (cx, cy) = (th.x + th.w / 2, th.y + th.h / 2);
        assert_eq!(local[(cy - b.y) as usize * b.w as usize + (cx - b.x) as usize], full[cy as usize * 1200 + cx as usize]);
        // and everything outside the surface stays clear in the full drawing's neighbours
        assert_eq!(full[0], 0);
    }

    #[test]
    fn a_capture_is_written_for_the_account_into_its_folder() {
        let dir = crate::shell::photos::library::tests::scratch("screenshot-as").join("Desktop");
        // no owner: the folder is made and the file lands in it
        let path = write_as(&dir, "Screenshot-20261020-014807.png", &[1, 2, 3], 1_791_073_395, None).unwrap();
        assert_eq!(path, dir.join("Screenshot-20261020-014807.png"));
        assert_eq!(std::fs::read(&path).unwrap(), vec![1, 2, 3]);
        assert!(!dir.join("Screenshot-20261020-014807.tmp").exists(), "the temporary file is gone");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_window_capture_encodes_with_alpha() {
        let img = Image { w: 2, h: 1, px: vec![0, pack(10, 20, 30, 255)] };
        assert_eq!(encode_alpha(&img)[25], 6);
        assert_eq!(encode(&img)[25], 2);
    }
}
