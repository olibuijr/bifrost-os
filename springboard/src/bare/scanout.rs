//! Zero-copy scanout: showing GPU output without the CPU copy into a dumb
//! buffer. `DumbDmabuf`: the GPU renders into an EGLImage made from a KMS dumb
//! buffer exported as a dma-buf (PRIME handle -> fd, then
//! EGL_EXT_image_dma_buf_import).
//!
//! If that does not work the shell keeps the copy path (GPU -> read back ->
//! dumb buffer). Everything that talks to the kernel returns `io::Result`;
//! the decision logic and the format/pitch math are pure and unit tested.

use super::sys::*;
use std::ffi::c_int;
use std::io;

pub const XB24: u32 = u32::from_le_bytes(*b"XB24");
pub const XR24: u32 = u32::from_le_bytes(*b"XR24");
/// Candidate formats, in order of preference. XB24 has the byte order of the
/// shell's frames. The same fourcc is used for the KMS framebuffer and the
/// EGL import, so the channel order agrees whichever one is picked.
pub const FOURCCS: [u32; 2] = [XB24, XR24];

/// Bytes per row of a tightly packed 32-bit `w`-pixel row.
pub fn tight_pitch(w: usize) -> usize {
    w * 4
}

/// Whether a framebuffer of `w` x `h` 32-bit pixels fits a buffer of `len`
/// bytes at `pitch` bytes per row.
pub fn fb_geometry_ok(w: usize, h: usize, pitch: usize, len: usize) -> bool {
    w > 0 && h > 0 && pitch >= tight_pitch(w) && pitch % 4 == 0 && pitch.checked_mul(h).is_some_and(|n| n <= len)
}

/// Page-rounded allocation size for a `pitch` x `h` buffer.
pub fn alloc_size(pitch: usize, h: usize) -> usize {
    (pitch * h).div_ceil(4096) * 4096
}

pub fn fourcc_name(f: u32) -> String {
    f.to_le_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '?' }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// KMS dumb buffer exported as a dma-buf and imported into EGL
    DumbDmabuf,
    /// today's path: render, copy, present
    Copy,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::DumbDmabuf => "dumb-dmabuf",
            Method::Copy => "copy",
        }
    }
}

/// What one (method, fourcc) probe found: the kernel step and the EGL step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Outcome {
    /// the KMS framebuffer (and PRIME step) worked
    pub drm: bool,
    /// EGL imported the dma-buf with this fourcc
    pub egl: bool,
}

impl Outcome {
    pub fn usable(&self) -> bool {
        self.drm && self.egl
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub method: Method,
    pub fourcc: u32,
}

/// Picks the method: exported dumb buffers with the first fourcc that is usable;
/// otherwise the copy path.
pub fn decide(dumb: &[(u32, Outcome)]) -> Decision {
    if let Some((fourcc, _)) = dumb.iter().find(|(_, o)| o.usable()) {
        return Decision { method: Method::DumbDmabuf, fourcc: *fourcc };
    }
    Decision { method: Method::Copy, fourcc: 0 }
}

#[repr(C)]
#[derive(Default)]
struct PrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct GemClose {
    handle: u32,
    pad: u32,
}

const DRM_CLOEXEC: u32 = O_CLOEXEC as u32;

/// DRM_IOCTL_PRIME_FD_TO_HANDLE: the GEM handle for a dma-buf fd.
pub fn prime_fd_to_handle(drm: c_int, fd: c_int) -> io::Result<u32> {
    let mut p = PrimeHandle { fd, ..Default::default() };
    unsafe { ioctl_ptr(drm, iowr::<PrimeHandle>(b'd', 0x2e), &mut p) }?;
    Ok(p.handle)
}

/// DRM_IOCTL_PRIME_HANDLE_TO_FD: a dma-buf fd for a GEM handle.
pub fn prime_handle_to_fd(drm: c_int, handle: u32) -> io::Result<c_int> {
    let mut p = PrimeHandle { handle, flags: DRM_CLOEXEC | O_RDWR as u32, fd: -1 };
    unsafe { ioctl_ptr(drm, iowr::<PrimeHandle>(b'd', 0x2d), &mut p) }?;
    Ok(p.fd)
}

/// DRM_IOCTL_MODE_ADDFB2 for a single-plane 32-bit buffer.
pub fn add_fb2(drm: c_int, w: usize, h: usize, fourcc: u32, handle: u32, pitch: usize) -> io::Result<u32> {
    let mut f = FbCmd2 { width: w as u32, height: h as u32, pixel_format: fourcc, ..Default::default() };
    f.handles[0] = handle;
    f.pitches[0] = pitch as u32;
    unsafe { ioctl_ptr(drm, iowr::<FbCmd2>(b'd', 0xb8), &mut f) }?;
    Ok(f.fb_id)
}

pub fn rm_fb(drm: c_int, fb: u32) {
    let mut id = fb;
    let _ = unsafe { ioctl_ptr(drm, iowr::<u32>(b'd', 0xaf), &mut id) };
}

pub fn gem_close(drm: c_int, handle: u32) {
    let mut c = GemClose { handle, pad: 0 };
    let _ = unsafe { ioctl_ptr(drm, iow::<GemClose>(b'd', 0x09), &mut c) };
}

/// A scanout-capable render buffer: a KMS framebuffer plus the dma-buf the
/// GPU renders into.
pub struct ScanoutBuf {
    pub method: Method,
    pub fb: u32,
    /// dma-buf fd for the EGL import (the exported dumb buffer)
    pub dmabuf_fd: c_int,
    pub pitch: usize,
    pub fourcc: u32,
    pub w: usize,
    pub h: usize,
    /// GEM handle on the DRM fd (freed with `release`)
    pub handle: u32,
}

impl ScanoutBuf {
    /// Frees the framebuffer, GEM handle and exported fd. The caller must
    /// have destroyed the EGL image first and must not be scanning it out.
    pub fn release(self, drm: c_int) {
        rm_fb(drm, self.fb);
        gem_close(drm, self.handle);
        if self.dmabuf_fd >= 0 {
            // SAFETY: the exported dma-buf fd is ours.
            unsafe { close(self.dmabuf_fd) };
        }
    }
}

/// A new dumb buffer exported as a dma-buf.
pub fn alloc_dumb_fb(drm: c_int, w: usize, h: usize, fourcc: u32) -> io::Result<ScanoutBuf> {
    let mut d = CreateDumb { width: w as u32, height: h as u32, bpp: 32, ..Default::default() };
    unsafe { ioctl_ptr(drm, iowr::<CreateDumb>(b'd', 0xb2), &mut d) }?;
    let pitch = d.pitch as usize;
    let step = || -> io::Result<(u32, c_int)> {
        if !fb_geometry_ok(w, h, pitch, d.size as usize) {
            return Err(io::Error::other("bad dumb buffer geometry"));
        }
        let fb = add_fb2(drm, w, h, fourcc, d.handle, pitch)?;
        match prime_handle_to_fd(drm, d.handle) {
            Ok(fd) => Ok((fb, fd)),
            Err(e) => {
                rm_fb(drm, fb);
                Err(e)
            }
        }
    };
    match step() {
        Ok((fb, fd)) => Ok(ScanoutBuf { method: Method::DumbDmabuf, fb, dmabuf_fd: fd, pitch, fourcc, w, h, handle: d.handle }),
        Err(e) => {
            gem_close(drm, d.handle);
            Err(e)
        }
    }
}

pub fn alloc(method: Method, drm: c_int, w: usize, h: usize, fourcc: u32) -> io::Result<ScanoutBuf> {
    match method {
        Method::DumbDmabuf => alloc_dumb_fb(drm, w, h, fourcc),
        Method::Copy => Err(io::Error::other("copy path has no scanout buffers")),
    }
}

/// Result of probing the kernel side of one method for every candidate format.
pub struct DrmProbe {
    pub method: Method,
    /// per fourcc: the kernel step's result, and the buffer kept for the EGL check
    pub tries: Vec<(u32, Result<ScanoutBuf, String>)>,
}

/// Tries to build a framebuffer for each candidate format with `method`
/// (one buffer each; the caller checks EGL import on `dmabuf_fd`, then
/// `release`s them).
pub fn probe_drm(drm: c_int, method: Method, w: usize, h: usize) -> DrmProbe {
    let tries = FOURCCS.iter().map(|&f| (f, alloc(method, drm, w, h, f).map_err(|e| e.to_string()))).collect();
    DrmProbe { method, tries }
}

/// "1" turns Developer > Zero-Copy GPU Output on; anything else (or no pref) is off.
pub fn pref_on(v: Option<&str>) -> bool {
    v == Some("1")
}

/// Which kind of frame the panel shows, for the shell loop: zero-copy GPU frames
/// and CPU frames (dumb buffers) must not be mixed without a full CPU redraw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSwitch {
    /// the scanout ring is live (set after a successful enable, cleared for good on failure)
    pub live: bool,
    /// the panel currently shows a GPU scanout buffer
    pub gpu_shown: bool,
    /// a failure turned it off for this run
    pub dead: bool,
}

impl FrameSwitch {
    pub fn enabled(&mut self) {
        if !self.dead {
            self.live = true;
        }
    }

    /// A GPU frame was flipped to the panel.
    pub fn gpu_frame(&mut self) {
        self.gpu_shown = self.live;
    }

    /// A CPU frame is about to be drawn: true when the panel showed a GPU frame, so the
    /// shell must redraw in full (its frame buffer does not hold what is on screen).
    pub fn cpu_frame(&mut self) -> bool {
        std::mem::take(&mut self.gpu_shown)
    }

    /// Scanout failed or vanished: stays off for the run.
    pub fn kill(&mut self) {
        self.live = false;
        self.dead = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Outcome {
        Outcome { drm: true, egl: true }
    }

    #[test]
    fn fourcc_values() {
        assert_eq!(XB24, 0x3432_4258);
        assert_eq!(XR24, 0x3432_5258);
        assert_eq!(fourcc_name(XB24), "XB24");
    }

    #[test]
    fn pitch_and_size_math() {
        assert_eq!(tight_pitch(1200), 4800);
        assert_eq!(alloc_size(4800, 1920), 4800 * 1920);
        assert_eq!(alloc_size(40, 10), 4096);
        assert!(fb_geometry_ok(1200, 1920, 4800, 4800 * 1920));
        assert!(!fb_geometry_ok(1200, 1920, 4796, 1 << 30), "pitch below the row size");
        assert!(!fb_geometry_ok(1200, 1920, 4802, 1 << 30), "pitch not a multiple of 4");
        assert!(!fb_geometry_ok(1200, 1920, 4800, 4800 * 1919), "buffer too small");
        assert!(!fb_geometry_ok(0, 10, 0, 100));
        assert!(!fb_geometry_ok(10, 10, usize::MAX, usize::MAX), "overflow is rejected");
    }

    #[test]
    fn decision_table() {
        let bad = Outcome::default();
        let drm_only = Outcome { drm: true, egl: false };
        let egl_only = Outcome { drm: false, egl: true };
        // everything works: the first format wins
        assert_eq!(decide(&[(XB24, ok()), (XR24, ok())]), Decision { method: Method::DumbDmabuf, fourcc: XB24 });
        // first format fails, second works
        assert_eq!(decide(&[(XB24, bad), (XR24, ok())]), Decision { method: Method::DumbDmabuf, fourcc: XR24 });
        // a half-working method is not usable
        assert_eq!(decide(&[(XB24, drm_only), (XR24, egl_only)]).method, Method::Copy);
        // nothing probed or nothing works: the copy path
        assert_eq!(decide(&[]), Decision { method: Method::Copy, fourcc: 0 });
        assert_eq!(Method::Copy.name(), "copy");
    }

    #[test]
    fn uapi_struct_sizes_and_numbers() {
        use std::mem::size_of;
        assert_eq!(size_of::<PrimeHandle>(), 12);
        assert_eq!(size_of::<FbCmd2>(), 104);
        assert_eq!(size_of::<CreateDumb>(), 32);
        assert_eq!(size_of::<GemClose>(), 8);
        // DRM_IOCTL_PRIME_HANDLE_TO_FD = DRM_IOWR(0x2d, struct drm_prime_handle)
        assert_eq!(iowr::<PrimeHandle>(b'd', 0x2d), 0xc00c_642d);
        assert_eq!(iowr::<PrimeHandle>(b'd', 0x2e), 0xc00c_642e);
        // DRM_IOCTL_MODE_ADDFB2
        assert_eq!(iowr::<FbCmd2>(b'd', 0xb8), 0xc068_64b8);
        // DRM_IOCTL_MODE_RMFB
        assert_eq!(iowr::<u32>(b'd', 0xaf), 0xc004_64af);
    }

    #[test]
    fn pref_is_on_only_for_one() {
        assert!(pref_on(Some("1")));
        assert!(!pref_on(Some("0")) && !pref_on(Some("")) && !pref_on(Some("true")) && !pref_on(None));
    }

    #[test]
    fn frame_switch_demands_a_full_cpu_redraw_after_gpu_frames() {
        let mut s = FrameSwitch::default();
        s.gpu_frame();
        assert!(!s.gpu_shown, "not live: gpu frames do not count");
        assert!(!s.cpu_frame());
        s.enabled();
        assert!(!s.cpu_frame(), "cpu to cpu: incremental");
        s.gpu_frame();
        s.gpu_frame();
        assert!(s.cpu_frame(), "gpu to cpu: full redraw");
        assert!(!s.cpu_frame(), "once");
        s.gpu_frame();
        s.kill();
        assert!(s.cpu_frame() && !s.live && s.dead);
        s.enabled();
        assert!(!s.live, "never again this run");
    }
}
