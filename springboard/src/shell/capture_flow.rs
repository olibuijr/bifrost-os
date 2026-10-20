//! The screenshot flow around the capture session (`capture.rs`) and the thumbnail (`screenshot.rs`): where a picture
//! comes from, where it is saved and how its overlays reach the screen.
//!
//! On the desktop the window server composes the screen (wallpaper, every window, the menu bar and the Dock); the shell's
//! own frame holds only what the shell draws itself. A capture therefore asks the window server for the composed output
//! (`bifrost-apps --screenshot`, which runs the screen grabber as the session's account, as the lab tooling does) after
//! the shell has taken its own overlays off the screen, and crops the picture it gets. A window capture cuts the window
//! out of that picture and puts it on a transparent ground with its drop shadow. Without a window server (the tablet, the
//! test fixtures) the picture is the shell's own frame, as before.
//!
//! The capture session and the thumbnail are surfaces of their own above the windows (chrome ids 8 and 9, exported by
//! `desktop_chrome_frames`); drawing them into the shell's frame, which sits under the windows, left stale pieces on screen.
//! Pictures go to the Desktop of the session's account (Options > Save To), written for that account.

use super::capture::{self, Session};
use super::screenshot::{self, Overlay};
use super::{photos, Shell};
use crate::gfx::{Image, Surface};
use crate::layout::Rect;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// The tool that prints the composed screen as a PNG on its standard output.
pub const GRAB_TOOL: &str = "/usr/bin/bifrost-apps";
/// How long the shell waits after taking its overlays off the screen before it asks for the picture, so the window server
/// has shown the frame without them.
pub const SETTLE_MS: f64 = 250.0;
/// A grab that has not answered after this long is given up.
pub const GRAB_TIMEOUT_MS: f64 = 15_000.0;
/// How often the shell looks for the grabber's answer.
const POLL_MS: f64 = 30.0;

/// A surface of the shell's own that sits above the windows: its place on the screen and its pixels (premultiplied), and
/// the part of it that takes the pointer (None: all of it; an empty rectangle: none of it).
pub(super) struct Layer {
    pub x: i32,
    pub y: i32,
    pub image: Image,
    pub input: Option<[i32; 4]>,
}

/// A capture waiting for the composed screen.
pub(super) struct Grab {
    /// what to take, in the shell's pixels (a window capture's region includes its shadow)
    region: Rect,
    /// where the pointer is, when it is to be drawn into the picture
    pointer: Option<(f32, f32)>,
    /// the window a window capture cuts out, without its shadow
    window: Option<Rect>,
    /// when the grabber may start (after the overlays are off the screen)
    due: f64,
    /// when it started and where its answer comes
    started: Option<(f64, Receiver<Option<Image>>)>,
}

/// What a waiting grab does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// the overlays are still coming off the screen
    Settling,
    /// time to start the grabber
    Start,
    /// the grabber is working
    Waiting,
    /// it took too long
    TimedOut,
}

/// The phase of a grab due at `due` that started at `started` (None: not yet), at `t`.
pub fn phase(due: f64, started: Option<f64>, t: f64) -> Phase {
    match started {
        None if t < due => Phase::Settling,
        None => Phase::Start,
        Some(at) if t - at > GRAB_TIMEOUT_MS => Phase::TimedOut,
        Some(_) => Phase::Waiting,
    }
}

/// Runs the grabber on a worker and sends the picture it prints (None when it fails).
fn spawn_grab() -> Receiver<Option<Image>> {
    let (tx, rx) = channel();
    crate::bare::sys::spawn_named("screen-grab", move || {
        let png = Command::new(GRAB_TOOL)
            .arg("--screenshot")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout);
        let _ = tx.send(png.and_then(|b| crate::cg::imageio::decode(&b)));
    });
    rx
}

/// The thumbnail animation as a surface of its own, clipped to the part of the screen it covers; None once it has gone.
pub(super) fn shot_layer(o: &Overlay, t: f64) -> Option<Layer> {
    let b = o.bounds(t);
    if b.w <= 0 || b.h <= 0 {
        return None;
    }
    let mut image = Image::new(b.w as usize, b.h as usize);
    o.draw_at(&mut Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 }, t, (b.x, b.y));
    // only the resting thumbnail takes the pointer: the flash and the slide away let it through
    let input = if o.interactive(t) { None } else { Some([0, 0, 0, 0]) };
    Some(Layer { x: b.x, y: b.y, image, input })
}

/// The capture session (dim, selection, countdown, toolbar) as a screen-sized surface of its own.
pub(super) fn capture_layer(c: &Session, ui: &super::ui::Ui, frames: &[Rect], w: i32, h: i32, t: f64) -> Option<Layer> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut image = Image::new(w as usize, h as usize);
    c.draw(&mut Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 }, ui, frames, w, h, t);
    Some(Layer { x: 0, y: 0, image, input: None })
}

impl Shell {
    /// Whether the shell's overlays are surfaces of their own above the windows (the desktop under the window server).
    pub(super) fn overlays_on_desk(&self) -> bool {
        cfg!(feature = "desktop-wayland") && self.desk_active()
    }

    /// The Dock's frame, which the capture toolbar keeps clear of; None without a desktop.
    pub(super) fn capture_dock(&self) -> Option<Rect> {
        #[cfg(feature = "desktop-wayland")]
        if self.desk_active() {
            let r = self.dock_overlay_rect();
            return Some(Rect { x: r.x, y: r.y, w: r.w, h: r.h });
        }
        None
    }

    /// Opens a capture session over the screen, its toolbar placed from the Dock.
    pub(super) fn capture_open(&mut self, mut session: Session) {
        session.dock = self.capture_dock();
        self.capture = Some(session);
        self.dirty = true;
    }

    /// Shift-Cmd-3: the whole screen, at once, with no timer.
    pub(super) fn capture_screen_now(&mut self, t: f64) {
        self.capture = None;
        let whole = Rect { x: 0, y: 0, w: self.w, h: self.h };
        self.begin_capture(whole, None, None, t);
    }

    /// Starts a capture of `region`. With a window server the shell takes its overlays off the screen and asks it for the
    /// composed picture a moment later (`grab_tick`); without one the picture is the shell's own frame.
    pub(super) fn begin_capture(&mut self, region: Rect, pointer: Option<(f32, f32)>, window: Option<Rect>, t: f64) {
        if !(self.overlays_on_desk() && Path::new(GRAB_TOOL).exists()) {
            self.take_capture(region, pointer, t);
            return;
        }
        self.shot = None;
        self.capture = None;
        self.grab = Some(Grab { region, pointer, window, due: t + SETTLE_MS, started: None });
        self.dirty = true;
    }

    /// Moves a waiting grab along: starts the grabber when the overlays are off the screen, and saves the picture when it
    /// arrives. A grab that fails is reported, not replaced by a picture that would not show the screen.
    pub(super) fn grab_tick(&mut self, t: f64) {
        let Some(g) = self.grab.as_mut() else { return };
        match phase(g.due, g.started.as_ref().map(|(at, _)| *at), t) {
            Phase::Settling => {}
            Phase::Start => g.started = Some((t, spawn_grab())),
            Phase::TimedOut => {
                self.grab = None;
                eprintln!("springboard: screenshot: the window server did not answer");
            }
            Phase::Waiting => {
                let answer = g.started.as_ref().map(|(_, rx)| rx.try_recv());
                match answer {
                    Some(Ok(Some(shot))) => {
                        if let Some(g) = self.grab.take() {
                            self.finish_grab(g, shot, t);
                        }
                    }
                    Some(Ok(None)) | Some(Err(TryRecvError::Disconnected)) => {
                        self.grab = None;
                        eprintln!("springboard: screenshot: the screen could not be read from the window server");
                    }
                    _ => {}
                }
            }
        }
    }

    /// When a waiting grab needs the loop again (ms): when its settling time ends, then often until the answer comes.
    pub(super) fn grab_wake(&self, t: f64) -> Option<f64> {
        let g = self.grab.as_ref()?;
        Some(match g.started {
            None => g.due.max(t),
            Some(_) => t + POLL_MS,
        })
    }

    /// Cuts the picture out of the composed screen `shot` and saves it.
    fn finish_grab(&mut self, g: Grab, shot: Image, t: f64) {
        let from = (self.w, self.h);
        let (kx, ky) = (shot.w as f32 / self.w.max(1) as f32, shot.h as f32 / self.h.max(1) as f32);
        let (img, alpha) = match g.window {
            Some(win) => {
                let pad = (capture::SHADOW_PT * self.ui.s * kx).round() as i32;
                // a window that fills the screen's width (maximized) has square corners
                let radius = if win.w >= self.w { 0.0 } else { self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) * kx };
                (capture::window_image(&self.ui, &shot, win, radius, pad, (kx, ky)), true)
            }
            None => {
                let Some(r) = capture::region_in(g.region, from, (shot.w, shot.h)) else {
                    eprintln!("springboard: screenshot: nothing of the chosen area is on the screen");
                    return;
                };
                let mut img = if (r.w as usize, r.h as usize) == (shot.w, shot.h) { shot } else { shot.crop(r.x as usize, r.y as usize, r.w as usize, r.h as usize) };
                if let Some((x, y)) = g.pointer {
                    capture::draw_pointer(&mut img, (x - g.region.x as f32) * kx, (y - g.region.y as f32) * ky, self.ui.s * kx);
                }
                (img, false)
            }
        };
        self.save_capture(img, alpha, t);
    }

    /// Captures `region` of the shell's own frame (the whole frame when it is all of it), draws the pointer into it when
    /// `pointer` names where it is, and saves it with the capture animation. The frame is drawn without the toolbar or
    /// the old thumbnail. This is the picture without a window server.
    pub(super) fn take_capture(&mut self, region: Rect, pointer: Option<(f32, f32)>, t: f64) {
        self.shot = None;
        // the toolbar is set aside while the frame is drawn, so it stays out of the picture
        let open = self.capture.take();
        let full = self.capture_frame(t);
        self.capture = open;
        let whole = region.x == 0 && region.y == 0 && region.w as usize == full.w && region.h as usize == full.h;
        let mut img = if whole { full } else { full.crop(region.x as usize, region.y as usize, region.w as usize, region.h as usize) };
        if let Some((x, y)) = pointer {
            capture::draw_pointer(&mut img, x - region.x as f32, y - region.y as f32, self.ui.s);
        }
        self.save_capture(img, false, t);
    }

    /// The folder a picture is saved in: the Options menu's choice (the Desktop by default) in the home folder of the
    /// session's account, which the installed account gives even when the shell has no `$HOME`.
    pub(super) fn capture_dir(&self) -> PathBuf {
        self.shot_dir.clone().unwrap_or_else(|| self.capture_prefs.save_to.dir(&super::dock_stack::home_dir()))
    }

    /// Saves a captured picture (with its alpha when it is a window) and starts the thumbnail's animation over it. A
    /// picture saved into the Photos library's folders joins the library.
    pub(super) fn save_capture(&mut self, img: Image, alpha: bool, t: f64) {
        let (unix, offset) = self.wall_now(t);
        let name = screenshot::file_name(&photos::library::civil(unix, offset));
        let dir = self.capture_dir();
        let bytes = if alpha { screenshot::encode_alpha(&img) } else { screenshot::encode(&img) };
        let path = dir.join(&name);
        self.shot_saved.push(path.clone());
        let photo = photos::library::Photo { path: path.clone(), mtime: unix, size: bytes.len() as u64 };
        let in_library = self.photo_dirs.iter().any(|d| dir.starts_with(d));
        let owner = screenshot::account_owner();
        crate::bare::sys::spawn_named("photo-save", move || {
            crate::gfx::affinity::background_thread();
            if let Err(e) = screenshot::write_as(&dir, &name, &bytes, unix, owner) {
                eprintln!("springboard: screenshot: {e}");
            }
        });
        if in_library {
            if self.photos.is_none() {
                self.ensure_photos();
            }
            if let Some(p) = &mut self.photos {
                p.add_photo(&self.ui, photo, img.clone());
            }
        }
        self.shot = Some(Overlay::new(img, path, self.ui.s, t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grab_settles_then_starts_then_waits_then_gives_up() {
        assert_eq!(phase(1250.0, None, 1000.0), Phase::Settling, "the overlays are still coming off the screen");
        assert_eq!(phase(1250.0, None, 1250.0), Phase::Start);
        assert_eq!(phase(1250.0, Some(1250.0), 1300.0), Phase::Waiting);
        assert_eq!(phase(1250.0, Some(1250.0), 1250.0 + GRAB_TIMEOUT_MS), Phase::Waiting);
        assert_eq!(phase(1250.0, Some(1250.0), 1251.0 + GRAB_TIMEOUT_MS), Phase::TimedOut);
    }

    #[test]
    fn the_grabber_is_the_tool_the_lab_uses_for_the_composed_screen() {
        assert_eq!(GRAB_TOOL, "/usr/bin/bifrost-apps");
        assert!(SETTLE_MS >= 100.0, "long enough for the window server to show the frame without the overlays");
    }

    #[test]
    fn pictures_are_saved_for_the_account_in_its_desktop_not_in_the_filesystem_root() {
        let mut sh = crate::shell::tests::shell();
        let home = Path::new("/home/ola");
        sh.shot_dir = None;
        let dir = sh.capture_prefs.save_to.dir(home);
        assert_eq!(dir, PathBuf::from("/home/ola/Desktop"));
        // without an override the shell resolves the folder from the account, so it is never `/Pictures`
        let resolved = sh.capture_dir();
        assert!(resolved.ends_with("Desktop") && resolved.parent().is_some_and(|p| p != Path::new("/")), "{resolved:?}");
        sh.shot_dir = Some(PathBuf::from("/x/y"));
        assert_eq!(sh.capture_dir(), PathBuf::from("/x/y"), "an override wins (the tests use it)");
    }

    #[test]
    fn the_thumbnail_layer_covers_the_screen_while_it_flashes_then_only_the_thumbnail_and_then_nothing() {
        let frame = Image { w: 1200, h: 1920, px: vec![crate::gfx::pack(1, 2, 3, 255); 1200 * 1920] };
        let o = Overlay::new(frame, PathBuf::from("/x.png"), 1200.0 / 768.0, 1000.0);
        let flash = shot_layer(&o, 1000.0).unwrap();
        assert_eq!((flash.image.w, flash.image.h), (1200, 1920));
        assert_eq!(flash.input, Some([0, 0, 0, 0]), "the flash does not take the pointer");
        let rest = 1000.0 + screenshot::FLASH_MS + screenshot::SHRINK_MS + 100.0;
        let thumb = shot_layer(&o, rest).unwrap();
        assert!(thumb.image.w < 400 && thumb.x > 600, "the thumbnail at the bottom right: {}x{} at {}", thumb.image.w, thumb.image.h, thumb.x);
        assert_eq!(thumb.input, None, "the resting thumbnail takes the pointer");
        assert!(thumb.image.px.iter().any(|&p| p >> 24 != 0), "it shows the picture");
        assert!(shot_layer(&o, 1000.0 + screenshot::HOLD_MS + screenshot::OUT_MS + 1.0).is_none(), "nothing is left once it has slid away");
    }

    /// The desktop exports the capture session and the thumbnail as surfaces above the windows and takes them away again.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_desktop_exports_the_overlays_and_leaves_nothing_behind() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-capture-flow"));
        let _ = std::fs::remove_dir_all(&dir);
        sh.shot_dir = Some(dir.join("Desktop"));
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; w * h];
        let mut frames = |sh: &mut Shell, t: f64| {
            sh.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, t);
            sh.desktop_chrome_frames(&buf, w, h).into_iter().map(|f| (f.id, f.x, f.y, f.width, f.height)).collect::<Vec<_>>()
        };
        // the session is a screen-sized surface, its toolbar clear of the Dock
        sh.capture_open(Session::new());
        let dock = sh.capture.as_ref().and_then(|c| c.dock).expect("the toolbar is placed from the Dock's frame");
        let list = frames(&mut sh, 100.0);
        assert!(list.contains(&(8, 0, 0, w, h)), "the session is a surface of its own: {list:?}");
        let (bar, _) = sh.capture.as_ref().unwrap().toolbar(&sh.ui, sh.w, sh.h);
        assert!(bar.y + bar.h <= dock.y, "the toolbar {bar:?} is above the Dock {dock:?}");
        sh.capture = None;
        sh.dirty = true;
        let list = frames(&mut sh, 200.0);
        assert!(list.iter().all(|f| f.0 != 8), "closing the session removes its surface: {list:?}");
        // the thumbnail is a surface at the bottom right, gone when it has slid away
        sh.take_capture(Rect { x: 0, y: 0, w: sh.w, h: sh.h }, None, 300.0);
        let rest = 300.0 + screenshot::FLASH_MS + screenshot::SHRINK_MS + 100.0;
        let list = frames(&mut sh, rest);
        let thumb = list.iter().find(|f| f.0 == 9).expect("the thumbnail is a surface of its own");
        assert!(thumb.1 > w as i32 / 2 && thumb.2 > h as i32 / 2, "at the bottom right: {thumb:?}");
        let gone = 300.0 + screenshot::HOLD_MS + screenshot::OUT_MS + 10.0;
        sh.tick(gone);
        let list = frames(&mut sh, gone);
        assert!(list.iter().all(|f| f.0 != 9), "nothing of the capture is left on screen: {list:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
