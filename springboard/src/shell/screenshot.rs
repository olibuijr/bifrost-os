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
/// as root (the session service does), else nobody to hand the file to (the process owns it already). The account is
/// looked up once it exists and then remembered (an account's uid and gid never change); before setup it is looked
/// up again on each call, so the account created by setup is found without a restart.
///
/// None does not mean the shell is unprivileged: a root shell before setup, or one whose account file cannot be read,
/// gets None too. A caller that would otherwise act as root must tell the two apart itself; the app control socket
/// dial (`intents::dial_identity`) then refuses every socket that is not root's in a root folder instead of dialling
/// it as root.
pub fn account_owner() -> Option<(u32, u32)> {
    #[cfg(target_os = "linux")]
    if crate::appearance_portal::is_root() {
        static OWNER: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
        if let Some(owner) = OWNER.get() {
            return Some(*owner);
        }
        let found = crate::bare::account::Account::installed().ok().map(|a| (a.uid, a.gid))?;
        return Some(*OWNER.get_or_init(|| found));
    }
    None
}

/// The home folder of the installed account (None off Linux or before an account exists).
#[cfg(target_os = "linux")]
fn account_home() -> Option<PathBuf> {
    crate::bare::account::Account::installed().ok().map(|a| a.home)
}

#[cfg(not(target_os = "linux"))]
fn account_home() -> Option<PathBuf> {
    None
}

/// Writes PNG `bytes` to `dir/<name>` through a temporary file, dating the file `unix` so the library orders it where
/// the capture happened. A root shell hands the file to the session account (see `write_as`).
pub fn write(dir: &Path, name: &str, bytes: &[u8], unix: i64) -> std::io::Result<PathBuf> {
    write_as(dir, name, bytes, unix, account_owner())
}

/// `write`, handing the folder (when it had to be made) and the file to `owner` (uid, gid): a root shell saving into the
/// account's Desktop must not leave a file the account cannot move or delete.
///
/// The folders under the account's home belong to the account, so a process running as the account can plant links
/// there. Nothing below the home is therefore reached by a path: the home is opened, and each folder, the temporary
/// file and the final rename go through directory descriptors that refuse to leave the home (`openat2` with
/// `RESOLVE_BENEATH`), create the temporary file exclusively without following a link, and hand ownership over by
/// descriptor, never by path.
pub fn write_as(dir: &Path, name: &str, bytes: &[u8], unix: i64, owner: Option<(u32, u32)>) -> std::io::Result<PathBuf> {
    let home = owner.and_then(|_| account_home()).filter(|h| dir.starts_with(h));
    match home {
        Some(home) => {
            let rel = dir.strip_prefix(&home).map(Path::to_path_buf).unwrap_or_default();
            safe_write::write_beneath(&home, &rel, name, bytes, unix, owner)?;
        }
        None => {
            // not under the account's home: the shell chose the folder, not the account
            let made = !dir.exists();
            std::fs::create_dir_all(dir)?;
            if let (true, Some((uid, gid))) = (made, owner) {
                let _ = std::os::unix::fs::lchown(dir, Some(uid), Some(gid));
            }
            safe_write::write_beneath(dir, Path::new(""), name, bytes, unix, owner)?;
        }
    }
    Ok(dir.join(name))
}

/// Writing a file under a folder that another user can reorganise.
mod safe_write {
    use std::ffi::{c_char, c_int, c_long, c_uint, CString, OsStr};
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Component, Path, PathBuf};

    const SYS_OPENAT2: c_long = 437;
    const O_RDONLY: c_int = 0;
    const O_WRONLY: c_int = 1;
    const O_CREAT: c_int = 0o100;
    const O_EXCL: c_int = 0o200;
    const O_CLOEXEC: c_int = 0o2000000;
    // the directory and no-follow bits differ on the ARM ABIs
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    const O_DIRECTORY: c_int = 0o40000;
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    const O_NOFOLLOW: c_int = 0o100000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
    const O_DIRECTORY: c_int = 0o200000;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
    const O_NOFOLLOW: c_int = 0o400000;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_BENEATH: u64 = 0x08;

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }

    extern "C" {
        fn syscall(n: c_long, ...) -> c_long;
        fn openat(dirfd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
        fn mkdirat(dirfd: c_int, path: *const c_char, mode: c_uint) -> c_int;
        fn renameat(olddir: c_int, old: *const c_char, newdir: c_int, new: *const c_char) -> c_int;
        fn unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int;
    }

    fn invalid(what: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, what.to_string())
    }

    fn cstr(s: &OsStr) -> io::Result<CString> {
        CString::new(s.as_bytes()).map_err(|_| invalid("path holds a NUL"))
    }

    /// A file name: one plain component.
    fn valid_name(name: &str) -> bool {
        !name.is_empty() && name.len() <= 200 && name != "." && name != ".." && !name.contains(['/', '\0'])
    }

    /// Opens the folder `rel` below the folder `dir`, following only links that stay below `dir`.
    fn open_beneath(dir: RawFd, rel: &Path) -> io::Result<OwnedFd> {
        let c = cstr(rel.as_os_str())?;
        let how = OpenHow { flags: (O_RDONLY | O_DIRECTORY | O_CLOEXEC) as u64, mode: 0, resolve: RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS };
        // SAFETY: `c` is NUL-terminated and `how` is a live OpenHow of the size passed.
        let fd = unsafe { syscall(SYS_OPENAT2, dir as c_long, c.as_ptr(), &how as *const OpenHow, std::mem::size_of::<OpenHow>() as c_long) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the kernel returned a new descriptor that nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
    }

    /// The folder `rel` below `root` as a descriptor. A missing folder is made and handed to `owner`.
    fn open_dir(root: &File, rel: &Path, owner: Option<(u32, u32)>) -> io::Result<OwnedFd> {
        let mut cur = OwnedFd::from(root.try_clone()?);
        let mut prefix = PathBuf::new();
        for part in rel.components() {
            let name = match part {
                Component::Normal(n) => n,
                Component::CurDir => continue,
                _ => return Err(invalid("folder path leaves its root")),
            };
            prefix.push(name);
            match open_beneath(root.as_raw_fd(), &prefix) {
                Ok(fd) => cur = fd,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let c = cstr(name)?;
                    // SAFETY: `cur` is an open descriptor and `c` is NUL-terminated; mkdirat never follows a final link.
                    let made = unsafe { mkdirat(cur.as_raw_fd(), c.as_ptr(), 0o755) } == 0;
                    if !made {
                        let e = io::Error::last_os_error();
                        if e.kind() != io::ErrorKind::AlreadyExists {
                            return Err(e);
                        }
                    }
                    let fd = open_beneath(root.as_raw_fd(), &prefix)?;
                    if let (true, Some((uid, gid))) = (made, owner) {
                        std::os::unix::fs::fchown(&fd, Some(uid), Some(gid))?;
                    }
                    cur = fd;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(cur)
    }

    fn unlink(dir: RawFd, name: &CString) {
        // SAFETY: `name` is NUL-terminated and `dir` is an open descriptor; unlinkat removes the entry, not a target.
        let _ = unsafe { unlinkat(dir, name.as_ptr(), 0) };
    }

    /// Writes `bytes` as `rel/name` below the folder `root`. `root` itself is opened as given; everything below it is
    /// reached through descriptors and cannot lead out of it. The file is made under a hidden temporary name (created
    /// exclusively, never through a link), owned by `owner` and dated `unix`, then renamed over `name`: a link at
    /// `name` is replaced, never written through.
    pub fn write_beneath(root: &Path, rel: &Path, name: &str, bytes: &[u8], unix: i64, owner: Option<(u32, u32)>) -> io::Result<()> {
        if !valid_name(name) {
            return Err(invalid("not a plain file name"));
        }
        let root = File::open(root)?;
        let dir = open_dir(&root, rel, owner)?;
        let dirfd = dir.as_raw_fd();
        let final_name = CString::new(name).map_err(|_| invalid("name holds a NUL"))?;
        for attempt in 0..32u32 {
            let tmp = CString::new(format!(".{name}.{}.{attempt}.tmp", std::process::id())).map_err(|_| invalid("name holds a NUL"))?;
            // SAFETY: `tmp` is NUL-terminated and `dirfd` is open; O_EXCL with O_NOFOLLOW never opens an existing entry.
            let fd = unsafe { openat(dirfd, tmp.as_ptr(), O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600 as c_uint) };
            if fd < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(e);
            }
            // SAFETY: openat returned a new descriptor that nothing else owns.
            let mut file = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
            let made = (|| -> io::Result<()> {
                file.write_all(bytes)?;
                if let Some((uid, gid)) = owner {
                    std::os::unix::fs::fchown(&file, Some(uid), Some(gid))?;
                }
                file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
                file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix.max(0) as u64))
            })();
            drop(file);
            if let Err(e) = made {
                unlink(dirfd, &tmp);
                return Err(e);
            }
            // SAFETY: both names are NUL-terminated and `dirfd` is open.
            if unsafe { renameat(dirfd, tmp.as_ptr(), dirfd, final_name.as_ptr()) } != 0 {
                let e = io::Error::last_os_error();
                unlink(dirfd, &tmp);
                return Err(e);
            }
            return Ok(());
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary name"))
    }
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
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_linked_folder_that_leaves_the_home_is_refused() {
        let base = crate::shell::photos::library::tests::scratch("screenshot-link-dir");
        let (home, outside) = (base.join("home"), base.join("outside"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("Desktop")).unwrap();
        assert!(safe_write::write_beneath(&home, Path::new("Desktop"), "Shot.png", &[1], 0, None).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing was written outside the home");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_link_in_the_name_or_the_temporary_file_is_replaced_not_followed() {
        let base = crate::shell::photos::library::tests::scratch("screenshot-link-file");
        let (home, outside) = (base.join("home"), base.join("outside"));
        let desk = home.join("Desktop");
        std::fs::create_dir_all(&desk).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, desk.join("Shot.png")).unwrap();
        std::os::unix::fs::symlink(&victim, desk.join(format!(".Shot.png.{}.0.tmp", std::process::id()))).unwrap();
        safe_write::write_beneath(&home, Path::new("Desktop"), "Shot.png", &[1, 2, 3], 1_791_073_395, None).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep", "the link target was not written");
        assert_eq!(std::fs::read(desk.join("Shot.png")).unwrap(), vec![1, 2, 3]);
        assert!(!std::fs::symlink_metadata(desk.join("Shot.png")).unwrap().file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_link_that_stays_inside_the_home_is_followed_and_missing_folders_are_made() {
        let base = crate::shell::photos::library::tests::scratch("screenshot-inside");
        let home = base.join("home");
        std::fs::create_dir_all(home.join("Pictures")).unwrap();
        std::os::unix::fs::symlink("Pictures", home.join("Desktop")).unwrap();
        safe_write::write_beneath(&home, Path::new("Desktop"), "A.png", &[7], 0, None).unwrap();
        assert_eq!(std::fs::read(home.join("Pictures/A.png")).unwrap(), vec![7]);
        safe_write::write_beneath(&home, Path::new("Pictures/Screens/2026"), "B.png", &[8], 0, None).unwrap();
        assert_eq!(std::fs::read(home.join("Pictures/Screens/2026/B.png")).unwrap(), vec![8]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_path_with_parent_parts_and_a_bad_file_name_are_refused() {
        let base = crate::shell::photos::library::tests::scratch("screenshot-escape");
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        assert!(safe_write::write_beneath(&home, Path::new("../x"), "a.png", &[1], 0, None).is_err());
        assert!(safe_write::write_beneath(&home, Path::new(""), "../a.png", &[1], 0, None).is_err());
        assert!(safe_write::write_beneath(&home, Path::new(""), "", &[1], 0, None).is_err());
        assert!(!base.join("x").exists() && !base.join("a.png").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_window_capture_encodes_with_alpha() {
        let img = Image { w: 2, h: 1, px: vec![0, pack(10, 20, 30, 255)] };
        assert_eq!(encode_alpha(&img)[25], 6);
        assert_eq!(encode(&img)[25], 2);
    }
}
