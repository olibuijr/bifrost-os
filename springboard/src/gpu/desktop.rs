//! EGL on a desktop (Mesa, NVIDIA, virtio-gpu): the display comes from a GBM device on a DRM
//! render node, else the surfaceless platform, else the default display (`eglGetDisplay(NULL)`
//! finds nothing without a window system, so it is the last resort). The context needs no
//! surface (render targets are FBOs); a pbuffer is made only where the driver demands one.

use super::egl::*;
use super::error::{egl_error, has_extension, sym_error};
use crate::bare::sys;
use std::ffi::{c_char, c_int, c_void, CString};
use std::fs::OpenOptions;
use std::io;
use std::os::fd::IntoRawFd;
use std::ptr;

const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES3_BIT: i32 = 0x40;
const EGL_PBUFFER_BIT: i32 = 1;
/// GLES 3.0: pixels per row of client memory for `glReadPixels` (0 = the read's width)
pub const GL_PACK_ROW_LENGTH: u32 = 0x0D02;
pub const GL_PACK_ALIGNMENT: u32 = 0x0D05;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

type GetPlatformDisplay = unsafe extern "C" fn(u32, P, *const i32) -> P;
/// `glReadPixels(x, y, w, h, format, type, pixels)`
pub type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

fn lookup<T: Copy>(lib: P, name: &str) -> Option<T> {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<P>());
    let c = CString::new(name).ok()?;
    // SAFETY: dlsym on a live handle with a NUL-terminated name; null means missing.
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: T is the documented pointer-sized C signature of the symbol.
    Some(unsafe { std::mem::transmute_copy(&p) })
}

/// `glReadPixels`, only for the desktop render-to-texture fallback.
pub fn read_pixels_fn(gles: P) -> Option<ReadPixels> {
    lookup(gles, "glReadPixels")
}

/// A GBM device on a render node; keeps the node open for as long as the EGL display lives.
pub struct Gbm {
    pub(crate) dev: P,
    fd: c_int,
    destroy: unsafe extern "C" fn(P),
    bo: Option<BoFns>,
}

/// GBM buffer-object calls (libgbm is vendor neutral: Mesa iris/radeonsi today, NVIDIA's GBM backend too).
#[derive(Clone, Copy)]
struct BoFns {
    create: unsafe extern "C" fn(P, u32, u32, u32, u32) -> P,
    get_fd: unsafe extern "C" fn(P) -> c_int,
    get_stride: unsafe extern "C" fn(P) -> u32,
    get_offset: unsafe extern "C" fn(P, c_int) -> u32,
    get_modifier: unsafe extern "C" fn(P) -> u64,
    destroy: unsafe extern "C" fn(P),
}

pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;
pub const GBM_BO_USE_LINEAR: u32 = 1 << 4;
/// DRM_FORMAT_MOD_INVALID: the driver's implicit layout
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// One GBM buffer exported as a dma-buf: a GL render target and a Wayland buffer at once.
pub struct Bo {
    ptr: P,
    pub fd: c_int,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
    destroy: unsafe extern "C" fn(P),
}

impl Drop for Bo {
    fn drop(&mut self) {
        // SAFETY: fd and bo were created by `Gbm::alloc` and are released exactly once.
        unsafe {
            sys::close(self.fd);
            (self.destroy)(self.ptr);
        }
    }
}

impl Gbm {
    fn open() -> io::Result<Gbm> {
        let c = CString::new("libgbm.so.1").map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: plain dlopen of a NUL-terminated name; the result is checked.
        let lib = unsafe { dlopen(c.as_ptr(), 2) };
        if lib.is_null() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "dlopen libgbm.so.1 failed"));
        }
        let create: unsafe extern "C" fn(c_int) -> P = lookup(lib, "gbm_create_device").ok_or_else(|| sym_error("gbm_create_device"))?;
        let destroy = lookup(lib, "gbm_device_destroy").ok_or_else(|| sym_error("gbm_device_destroy"))?;
        let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD")))
            .collect();
        nodes.sort();
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no /dev/dri/renderD* node");
        for node in nodes {
            match OpenOptions::new().read(true).write(true).open(&node) {
                Ok(f) => {
                    let fd = f.into_raw_fd();
                    // SAFETY: fd is an open DRM render node; the device is destroyed in Drop before the fd closes.
                    let dev = unsafe { create(fd) };
                    if !dev.is_null() {
                        let bo = (|| {
                            Some(BoFns {
                                create: lookup(lib, "gbm_bo_create")?,
                                get_fd: lookup(lib, "gbm_bo_get_fd")?,
                                get_stride: lookup(lib, "gbm_bo_get_stride")?,
                                get_offset: lookup(lib, "gbm_bo_get_offset")?,
                                get_modifier: lookup(lib, "gbm_bo_get_modifier")?,
                                destroy: lookup(lib, "gbm_bo_destroy")?,
                            })
                        })();
                        return Ok(Gbm { dev, fd, destroy, bo });
                    }
                    // SAFETY: fd was opened above and nothing else holds it.
                    unsafe { sys::close(fd) };
                    last = io::Error::other(format!("gbm_create_device failed on {}", node.display()));
                }
                Err(e) => last = io::Error::new(e.kind(), format!("{}: {e}", node.display())),
            }
        }
        Err(last)
    }
}

impl Gbm {
    /// Allocates a `w` x `h` render target in `fourcc` and exports it. `linear` forces the linear
    /// layout (modifier 0), which every compositor and GPU can import; otherwise the driver picks
    /// its tiling and reports the modifier.
    pub fn alloc(&self, w: usize, h: usize, fourcc: u32, linear: bool) -> io::Result<Bo> {
        let f = self.bo.ok_or_else(|| sym_error("gbm_bo_*"))?;
        let flags = GBM_BO_USE_RENDERING | if linear { GBM_BO_USE_LINEAR } else { 0 };
        // SAFETY: dev is a live gbm_device; the result is checked.
        let ptr = unsafe { (f.create)(self.dev, w as u32, h as u32, fourcc, flags) };
        if ptr.is_null() {
            return Err(io::Error::other(format!("gbm_bo_create {w}x{h} failed")));
        }
        // SAFETY: ptr is a live bo; each getter only reads it.
        let (fd, stride, offset, modifier) = unsafe { ((f.get_fd)(ptr), (f.get_stride)(ptr), (f.get_offset)(ptr, 0), (f.get_modifier)(ptr)) };
        if fd < 0 {
            // SAFETY: the bo was created above and nothing else holds it.
            unsafe { (f.destroy)(ptr) };
            return Err(io::Error::other("gbm_bo_get_fd failed"));
        }
        Ok(Bo { ptr, fd, stride, offset, modifier: if linear { 0 } else { modifier }, destroy: f.destroy })
    }
}

impl Drop for Gbm {
    fn drop(&mut self) {
        // SAFETY: dev and fd were created in `open` and are released exactly once.
        unsafe {
            (self.destroy)(self.dev);
            sys::close(self.fd);
        }
    }
}

/// An initialised EGL display and what keeps it alive.
pub struct Display {
    pub dpy: P,
    pub gbm: Option<Gbm>,
    /// which route worked: "gbm", "surfaceless" or "default"
    pub kind: &'static str,
}

fn initialize(egl: &Egl, dpy: P) -> io::Result<()> {
    let (mut maj, mut min) = (0, 0);
    // SAFETY: out pointers are live locals; a null display just fails.
    if dpy.is_null() || unsafe { (egl.initialize)(dpy, &mut maj, &mut min) } == 0 {
        // SAFETY: eglGetError has no preconditions.
        return Err(egl_error("eglInitialize", unsafe { (egl.get_error)() }));
    }
    Ok(())
}

/// Opens the first display that initialises; the error lists why each route failed.
pub fn open_display(egl: &Egl) -> io::Result<Display> {
    // SAFETY: a null display asks for the client extension string; it is driver-owned.
    // SAFETY: eglQueryString returns null or a driver-owned NUL-terminated string.
    let client = unsafe { cstr((egl.query_string)(ptr::null_mut(), EGL_EXTENSIONS)) };
    let platform: Option<GetPlatformDisplay> = if has_extension(&client, "EGL_EXT_platform_base") {
        let c = CString::new("eglGetPlatformDisplayEXT").map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: eglGetProcAddress with a NUL-terminated name; null means unsupported.
        let p = unsafe { (egl.get_proc)(c.as_ptr()) };
        // SAFETY: eglGetPlatformDisplayEXT has exactly this C signature.
        (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<P, GetPlatformDisplay>(&p) })
    } else {
        None
    };
    let mut why = Vec::new();
    if let Some(get) = platform {
        if has_extension(&client, "EGL_KHR_platform_gbm") || has_extension(&client, "EGL_MESA_platform_gbm") {
            match Gbm::open() {
                Ok(g) => {
                    // SAFETY: g.dev is a live gbm_device, the native display of the GBM platform.
                    let dpy = unsafe { get(EGL_PLATFORM_GBM_KHR, g.dev, ptr::null()) };
                    match initialize(egl, dpy) {
                        Ok(()) => return Ok(Display { dpy, gbm: Some(g), kind: "gbm" }),
                        Err(e) => why.push(format!("gbm: {e}")),
                    }
                }
                Err(e) => why.push(format!("gbm: {e}")),
            }
        } else {
            why.push("gbm: EGL_KHR_platform_gbm missing".into());
        }
        if has_extension(&client, "EGL_MESA_platform_surfaceless") {
            // SAFETY: the surfaceless platform takes no native display.
            let dpy = unsafe { get(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null()) };
            match initialize(egl, dpy) {
                Ok(()) => return Ok(Display { dpy, gbm: None, kind: "surfaceless" }),
                Err(e) => why.push(format!("surfaceless: {e}")),
            }
        } else {
            why.push("surfaceless: EGL_MESA_platform_surfaceless missing".into());
        }
    } else {
        why.push("eglGetPlatformDisplayEXT missing".into());
    }
    // SAFETY: EGL_DEFAULT_DISPLAY is a null native display.
    let dpy = unsafe { (egl.get_display)(ptr::null_mut()) };
    match initialize(egl, dpy) {
        Ok(()) => Ok(Display { dpy, gbm: None, kind: "default" }),
        Err(e) => {
            why.push(format!("default: {e}"));
            Err(io::Error::other(format!("no EGL display ({})", why.join("; "))))
        }
    }
}

/// An ES 3 context that renders to FBOs only: no surface when the driver allows a surfaceless
/// make-current, else a small pbuffer. Returns (context, surface or null).
// EGL handles are opaque: the driver validates them (EGL_BAD_DISPLAY etc.); no Rust memory is read through them.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn open_context(egl: &Egl, dpy: P) -> io::Result<(P, P)> {
    let ctx_attrs = [0x3098, 3, EGL_NONE];
    // SAFETY: eglBindAPI has no pointer arguments.
    unsafe { (egl.bind_api)(EGL_OPENGL_ES_API) };
    let pick = |surface_type: i32| -> io::Result<P> {
        let attrs = [EGL_SURFACE_TYPE, surface_type, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT, EGL_NONE];
        let (mut cfg, mut n) = (ptr::null_mut(), 0);
        // SAFETY: attribute list is EGL_NONE terminated; out pointers are live locals.
        let ok = unsafe { (egl.choose_config)(dpy, attrs.as_ptr(), &mut cfg, 1, &mut n) };
        if ok == 0 || n < 1 {
            // SAFETY: eglGetError has no preconditions.
            return Err(egl_error("eglChooseConfig", unsafe { (egl.get_error)() }));
        }
        Ok(cfg)
    };
    let mut why = Vec::new();
    // surfaceless: any config will do, FBO targets never touch the surface
    match pick(0) {
        Ok(cfg) => {
            // SAFETY: dpy and cfg are valid; the attribute list is EGL_NONE terminated.
            let ctx = unsafe { (egl.create_context)(dpy, cfg, ptr::null_mut(), ctx_attrs.as_ptr()) };
            // SAFETY: a null surface pair is EGL_NO_SURFACE.
            if !ctx.is_null() && unsafe { (egl.make_current)(dpy, ptr::null_mut(), ptr::null_mut(), ctx) } != 0 {
                return Ok((ctx, ptr::null_mut()));
            }
            // SAFETY: eglGetError has no preconditions.
            why.push(format!("surfaceless: {}", egl_error("make current", unsafe { (egl.get_error)() })));
            if !ctx.is_null() {
                // SAFETY: ctx was just created on dpy and is not current.
                unsafe { (egl.destroy_context)(dpy, ctx) };
            }
        }
        Err(e) => why.push(format!("surfaceless: {e}")),
    }
    let cfg = pick(EGL_PBUFFER_BIT).map_err(|e| io::Error::other(format!("{e} ({})", why.join("; "))))?;
    let pb_attrs = [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE];
    // SAFETY: dpy/cfg are valid; attribute lists are EGL_NONE terminated.
    let (ctx, surf) = unsafe { ((egl.create_context)(dpy, cfg, ptr::null_mut(), ctx_attrs.as_ptr()), (egl.create_pbuffer)(dpy, cfg, pb_attrs.as_ptr())) };
    // SAFETY: eglGetError has no preconditions.
    if ctx.is_null() || surf.is_null() || unsafe { (egl.make_current)(dpy, surf, surf, ctx) } == 0 {
        return Err(egl_error("create context", unsafe { (egl.get_error)() }));
    }
    Ok((ctx, surf))
}
