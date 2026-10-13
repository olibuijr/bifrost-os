//! GPU-first presentation for the Wayland desktop.
//!
//! Every BifrOSt surface renders into GBM buffers that are exported as dma-bufs: the same memory
//! is the GL render target and the `wl_buffer` (linux-dmabuf-v1), so a frame reaches the
//! compositor with no CPU copy and no SHM upload in the compositor. The model is Core Animation's:
//! the CPU raster is one *content layer* per surface, uploaded only where it changed; the GPU
//! composes the wallpaper, the glass materials (a dual-Kawase blur computed once per wallpaper)
//! and the content. Shapes for glass come from `bifrost_render::glass`.
//!
//! The API is shaped like Metal (device, drawables, one render pass per present) so a Vulkan
//! backend can replace GLES without touching callers. Vendor neutral: GBM + EGL + dma-buf are the
//! same on Intel (iris), AMD (radeonsi) and NVIDIA (GBM backend, 495+). Linear buffers by default;
//! `BIFROST_GPU_TILED=1` lets the driver tile and passes the modifier.

use super::desktop::{self, Bo, Gbm, MOD_INVALID};
use super::egl::*;
use super::error::{egl_error, gl_error, has_extension};
use super::health::Health;
use crate::gfx::{Area, Damage, View};
use bifrost_render::glass::Rect as Glass;
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::io;
use std::ptr;

/// DRM_FORMAT_XBGR8888 / ABGR8888: bytes R, G, B, A in memory, the byte order of `gfx` frames
/// and of GL_RGBA, so nothing is swizzled anywhere.
pub const FOURCC_XB24: u32 = 0x3432_4258;
pub const FOURCC_AB24: u32 = 0x3432_4241;

macro_rules! gl {
    ($t:expr, $f:ident($($a:expr),* $(,)?)) => {
        // SAFETY: the Presenter owns the current context on this thread; pointers are live.
        unsafe { ($t.$f)($($a),*) }
    };
}

/// What a Wayland client needs to wrap a drawable in a `wl_buffer`.
pub struct BufferInfo {
    pub fd: i32,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
    pub fourcc: u32,
    pub w: usize,
    pub h: usize,
}

/// What lies under a surface's content.
#[derive(Clone, Copy, Debug)]
pub enum Backdrop {
    /// the wallpaper everywhere (the desktop background); the surface's origin on screen
    Full { x: i32, y: i32 },
    /// the blurred backdrop only under glass shapes (menus, dock, a window's materials); the origin on screen.
    /// `active` is false for an inactive window: materials that follow the active state are drawn opaque.
    GlassOnly { x: i32, y: i32, active: bool },
    /// nothing: a transparent surface (windows, which carry their own shadow)
    None,
}

/// A window shadow drawn by the GPU around the content rect (Golden Gate: the active window's
/// shadow is deeper, so the key window stands out).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// corner radius of the content, px
    pub radius: f32,
    pub focused: bool,
}

struct Tex {
    tex: u32,
    fbo: u32,
    w: usize,
    h: usize,
}

struct Drawable {
    _bo: Bo,
    image: P,
    tex: u32,
    fbo: u32,
}

struct Surf {
    w: usize,
    h: usize,
    /// the content rect inside the drawable (a window's margin holds its shadow)
    cx: usize,
    cy: usize,
    cw: usize,
    ch: usize,
    shadow: Option<Shadow>,
    fx: bifrost_render::glass::Fx,
    opaque: bool,
    ring: Vec<Drawable>,
    owed: Vec<Damage>,
    content: Tex,
    fresh: bool,
    /// content damage not uploaded yet (every drawable was busy)
    pending: Damage,
}

/// The live backdrop: wallpaper plus the windows' content at 1/LIVE_DIV resolution, blurred.
/// Glass samples `up.last()` so menus, bars and the dock blur what is really behind them.
struct Live {
    base: Tex,
    down: Vec<Tex>,
    up: Vec<Tex>,
}

/// One window's behind-window backdrop: the live backdrop without that window and the windows above it.
struct Under {
    live: Option<Live>,
    dirty: bool,
    built: Option<std::time::Instant>,
    /// the 30 Hz cap skipped a wanted rebuild
    wait: bool,
}

/// Live backdrop resolution divisor (a quarter-size render keeps the rebuild cheap).
const LIVE_DIV: usize = 4;

struct Program {
    id: u32,
    view: i32,
    mode: i32,
    half: i32,
    alpha: i32,
    rrect: i32,
    radius: i32,
    glass: i32,
    hole: i32,
    hole_r: i32,
    sat: i32,
    bright: i32,
    lum_amt: i32,
    lum_v: i32,
    rim: i32,
    color: i32,
}

/// Per-present timings for the benchmark log, in milliseconds.
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub upload_ms: f64,
    pub render_ms: f64,
    pub uploaded_px: usize,
    pub drawn_px: usize,
}

/// Bumped by the shell whenever it installs a new wallpaper, so a new picture that reuses the
/// freed allocation of the old one (same address and size) is still uploaded (PR-5).
static WALLPAPER_GEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn bump_wallpaper_generation() {
    WALLPAPER_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub struct Presenter {
    egl: Egl,
    gl: Gles,
    ext: EglExt,
    dpy: P,
    ctx: P,
    surf: P,
    gbm: Gbm,
    linear: bool,
    has_modifiers: bool,
    prog: Program,
    vao: u32,
    vbo: u32,
    surfaces: HashMap<u32, Surf>,
    wallpaper: Option<(Tex, i32, i32)>,
    blur: Vec<Tex>,
    /// windows in z order (bottom first): presenter key and the content origin in screen px
    stack: Vec<(u32, i32, i32)>,
    live: Option<Live>,
    live_dirty: bool,
    /// Backdrops of windows whose behind-window materials look active, by presenter key: as `live`, without the
    /// window itself and the windows above it (a sidebar shows the windows behind its window, not its own content).
    under: HashMap<u32, Under>,
    /// windows whose backdrop changed since the caller last asked: their materials owe a redraw
    under_changed: Vec<u32>,
    /// a backdrop rebuild was wanted but the 30 Hz cap skipped it: `retry_in_ms` wakes the loop for it
    live_wait: bool,
    /// when the live backdrop was last rebuilt: rebuilds are capped at ~30 Hz so a video behind glass stays cheap
    live_built: Option<std::time::Instant>,
    /// a window changed since the chrome last drew: glass surfaces owe their shapes
    live_changed: bool,
    live_on: bool,
    /// glass rects were owed: the next present draws even with empty content damage
    flush: bool,
    wallpaper_key: (usize, usize, i32, u32),
    pub renderer: String,
    pub stats: Stats,
    /// the driver syncs shared dma-bufs itself (Mesa); otherwise each pass is waited for
    implicit_sync: bool,
    /// the first GPU failure (lost context, GL error, a pass that never finished): once set, nothing is drawn and
    /// the shell moves to the SHM path (`failure`)
    health: Health,
}

const VS: &str = "#version 300 es
layout(location=0) in vec2 a_pos;
layout(location=1) in vec2 a_uv;
uniform vec2 u_view;
out vec2 v_uv;
out vec2 v_px;
void main() {
    v_uv = a_uv;
    v_px = a_pos;
    gl_Position = vec4(a_pos / u_view * 2.0 - 1.0, 0.0, 1.0);
}";

// mode 0: textured quad (optionally clipped to a rounded rect); 1: Kawase downsample; 2: upsample;
// 3: rounded-rect shadow; 4: solid rounded rect (an inactive window's material)
const FS: &str = "#version 300 es
precision mediump float;
uniform sampler2D u_tex;
uniform int u_mode;
uniform vec2 u_half;
uniform float u_alpha;
uniform vec4 u_rrect;
uniform float u_radius;
uniform float u_glass;
uniform vec4 u_hole;
uniform float u_hole_r;
uniform float u_sat;
uniform float u_bright;
uniform float u_lum_amt;
uniform vec4 u_lum_v;
uniform float u_rim;
uniform vec4 u_color;
in vec2 v_uv;
in vec2 v_px;
out vec4 o;
float rr(vec2 p, vec4 r, float rad) {
    vec2 c = r.xy + r.zw * 0.5;
    vec2 q = abs(p - c) - (r.zw * 0.5 - vec2(rad));
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rad;
}
void main() {
    vec2 h = u_half;
    if (u_mode == 1) {
        vec4 s = texture(u_tex, v_uv) * 4.0;
        s += texture(u_tex, v_uv - h);
        s += texture(u_tex, v_uv + h);
        s += texture(u_tex, v_uv + vec2(h.x, -h.y));
        s += texture(u_tex, v_uv - vec2(h.x, -h.y));
        o = s / 8.0;
        return;
    }
    if (u_mode == 3) {
        // soft rounded-rect shadow: u_half.x is the blur, u_alpha the strength; premultiplied black
        float d = rr(v_px, u_rrect, u_radius);
        float b = max(u_half.x, 1.0);
        float a = u_alpha * (1.0 - smoothstep(-0.35 * b, b, d));
        // the window's own rect is not shadowed: a translucent (vibrant) window shows the blur, not a dark slab
        if (u_hole.z > 0.0) {
            a *= clamp(rr(v_px, u_hole, u_hole_r) + 1.0, 0.0, 1.0);
        }
        o = vec4(0.0, 0.0, 0.0, a);
        return;
    }
    if (u_mode == 4) {
        o = u_color * clamp(0.5 - rr(v_px, u_rrect, u_radius), 0.0, 1.0);
        return;
    }
    if (u_mode == 2) {
        vec4 s = texture(u_tex, v_uv + vec2(-h.x * 2.0, 0.0));
        s += texture(u_tex, v_uv + vec2(-h.x, h.y)) * 2.0;
        s += texture(u_tex, v_uv + vec2(0.0, h.y * 2.0));
        s += texture(u_tex, v_uv + vec2(h.x, h.y)) * 2.0;
        s += texture(u_tex, v_uv + vec2(h.x * 2.0, 0.0));
        s += texture(u_tex, v_uv + vec2(h.x, -h.y)) * 2.0;
        s += texture(u_tex, v_uv + vec2(0.0, -h.y * 2.0));
        s += texture(u_tex, v_uv + vec2(-h.x, -h.y)) * 2.0;
        o = s / 12.0;
        return;
    }
    vec4 c = texture(u_tex, v_uv) * u_alpha;
    if (u_radius >= 0.0) {
        float d = rr(v_px, u_rrect, u_radius);
        if (u_glass > 0.0) {
            // vibrancy: the material lifts the saturation of what is behind it (Rec. 709 luminance)
            float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
            c.rgb = clamp(mix(vec3(l), c.rgb, u_sat), 0.0, c.a);
            // the material recipe's luminance mapping (a cubic curve through four control values, pulled towards by the
            // amount) and brightness offset; render/src/glass.rs `Look::apply` is the reference arithmetic
            if (u_lum_amt != 0.0) {
                float m = clamp(dot(c.rgb, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
                float w = 1.0 - m;
                float cv = w * w * w * u_lum_v.x + 3.0 * w * w * m * u_lum_v.y + 3.0 * w * m * m * u_lum_v.z + m * m * m * u_lum_v.w;
                c.rgb = clamp(c.rgb + u_lum_amt * (cv - m), 0.0, c.a);
            }
            c.rgb = clamp(c.rgb + u_bright, 0.0, c.a);
            // Liquid Glass edge: a thin specular rim lit from the top left, its opposite edge catching a
            // weaker reflection; the light falls off with the angle between the edge normal and the light
            if (u_rim > 0.0) {
                vec2 g = vec2(dFdx(d), dFdy(d));
                vec2 n = g / max(length(g), 1e-4);
                float facing = dot(n, vec2(-0.7071, -0.7071));
                float lit = max(facing, 0.0) + 0.45 * max(-facing, 0.0);
                float band = 1.0 - smoothstep(0.0, 1.6, -d);
                c.rgb += vec3(band * (0.05 + 0.22 * lit) * u_rim) * c.a;
            }
            // dither: the blur is upscaled from a small texture, so smooth gradients band in 8 bits
            float noise = fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
            c.rgb = clamp(c.rgb + (noise - 0.5) / 255.0 * c.a, 0.0, c.a);
        }
        c *= clamp(0.5 - d, 0.0, 1.0);
    }
    o = c;
}";

/// The first stack position where `old` and `new` differ (a window moved, appeared, left or changed order).
/// A window's backdrop reads only the windows below it, so only the windows above that position need a new one.
fn stack_change_from(old: &[(u32, i32, i32)], new: &[(u32, i32, i32)]) -> usize {
    old.iter().zip(new).position(|(a, b)| a != b).unwrap_or(old.len().min(new.len()))
}

/// Marks dirty the backdrops of the windows at stack position `from` and above, and lists them as changed.
fn under_dirty_from(stack: &[(u32, i32, i32)], under: &mut HashMap<u32, Under>, changed: &mut Vec<u32>, from: usize) {
    for &(k, ..) in stack.iter().skip(from) {
        if let Some(u) = under.get_mut(&k) {
            u.dirty = true;
            if !changed.contains(&k) {
                changed.push(k);
            }
        }
    }
}

/// Blur pyramid depth: full -> 1/2 -> ... -> 1/2^LEVELS, then back up to 1/2.
const LEVELS: usize = 4;

/// Presenter keys at or above this are windows (`GPU_WINDOW + id`); below are the background and chrome.
pub const WINDOW_KEY_BASE: u32 = 0x1000_0000;

fn is_window(id: u32) -> bool {
    (WINDOW_KEY_BASE..u32::MAX).contains(&id)
}

impl Presenter {
    /// Opens the GPU on its render node (GBM display), compiles the pipeline. Fails, never panics,
    /// when there is no GBM/EGL/dma-buf import: the caller keeps the SHM path.
    pub fn open() -> io::Result<Presenter> {
        let libs = open_libs()?;
        let egl = Egl::load(libs.egl)?;
        let gl = Gles::load(libs.gles)?;
        let d = desktop::open_display(&egl)?;
        let gbm = d.gbm.ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, format!("EGL display is {}, not GBM", d.kind)))?;
        // SAFETY: the display is initialised; the string is driver-owned.
        // SAFETY: eglQueryString returns null or a driver-owned NUL-terminated string.
        let exts = unsafe { cstr((egl.query_string)(d.dpy, EGL_EXTENSIONS)) };
        if !has_extension(&exts, EXT_DMA_BUF) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "EGL_EXT_image_dma_buf_import missing"));
        }
        let ext = EglExt::load(&egl, &exts)?;
        let (ctx, surf) = desktop::open_context(&egl, d.dpy)?;
        let prog = link(&gl)?;
        let (mut vao, mut vbo) = (0u32, 0u32);
        gl!(gl, gen_vertex_arrays(1, &mut vao));
        gl!(gl, bind_vertex_array(vao));
        gl!(gl, gen_buffers(1, &mut vbo));
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, vbo));
        gl!(gl, enable_vertex_attrib_array(0));
        gl!(gl, vertex_attrib_pointer(0, 2, GL_FLOAT, 0, 16, ptr::null()));
        gl!(gl, enable_vertex_attrib_array(1));
        gl!(gl, vertex_attrib_pointer(1, 2, GL_FLOAT, 0, 16, 8 as *const c_void));
        gl!(gl, pixel_storei(GL_UNPACK_ALIGNMENT, 4));
        let renderer = gl!(gl, get_string(GL_RENDERER));
        // SAFETY: glGetString returns null or a driver-owned NUL-terminated string.
        let renderer = unsafe { cstr(renderer) };
        let e = gl!(gl, get_error());
        if e != 0 {
            return Err(gl_error("presenter setup", e));
        }
        let linear = std::env::var_os("BIFROST_GPU_TILED").is_none();
        let implicit_sync = renderer.contains("Mesa") && std::env::var_os("BIFROST_GPU_WAIT").is_none();
        let has_modifiers = has_extension(&exts, "EGL_EXT_image_dma_buf_import_modifiers");
        Ok(Presenter {
            has_modifiers,
            egl,
            gl,
            ext,
            dpy: d.dpy,
            ctx,
            surf,
            gbm,
            linear,
            prog,
            vao,
            vbo,
            surfaces: HashMap::new(),
            wallpaper: None,
            blur: Vec::new(),
            stack: Vec::new(),
            live: None,
            live_dirty: false,
            under: HashMap::new(),
            under_changed: Vec::new(),
            live_wait: false,
            live_built: None,
            live_changed: false,
            live_on: true,
            flush: false,
            wallpaper_key: (0, 0, 0, 0),
            renderer,
            stats: Stats::default(),
            implicit_sync,
            health: Health::default(),
        })
    }

    /// The reason the presenter stopped drawing, once it did. The caller then leaves the GPU for the SHM path.
    pub fn failure(&self) -> Option<&str> {
        self.health.failure()
    }

    /// Creates (or recreates) surface `id` with `n` drawables; returns them for `wl_buffer`s.
    pub fn create_surface(&mut self, id: u32, w: usize, h: usize, opaque: bool, n: usize) -> io::Result<Vec<BufferInfo>> {
        self.create_framed(id, w, h, (0, 0, w, h), opaque, n)
    }

    /// `create_surface` with the content at `(cx, cy, cw, ch)` inside a larger `w` x `h` drawable.
    pub fn create_framed(&mut self, id: u32, w: usize, h: usize, (cx, cy, cw, ch): (usize, usize, usize, usize), opaque: bool, n: usize) -> io::Result<Vec<BufferInfo>> {
        self.current();
        self.destroy_surface(id);
        let fourcc = if opaque { FOURCC_XB24 } else { FOURCC_AB24 };
        let mut ring = Vec::new();
        let mut infos = Vec::new();
        for _ in 0..n {
            let built = self.gbm.alloc(w, h, fourcc, self.linear).and_then(|bo| self.import(&bo, w, h, fourcc).map(|r| (bo, r)));
            let (bo, (image, tex, fbo)) = match built {
                Ok(v) => v,
                Err(e) => {
                    // PR-8: free the drawables already built, `Drawable` has no Drop
                    for d in ring.drain(..) {
                        self.release(d);
                    }
                    return Err(e);
                }
            };
            infos.push(BufferInfo { fd: bo.fd, stride: bo.stride, offset: bo.offset, modifier: bo.modifier, fourcc, w, h });
            ring.push(Drawable { _bo: bo, image, tex, fbo });
        }
        let content = self.texture(cw, ch, GL_NEAREST, None)?;
        let mut all = Damage::new(w, h);
        all.all();
        self.surfaces.insert(id, Surf { w, h, cx, cy, cw, ch, shadow: None, fx: bifrost_render::glass::Fx::IDENTITY, opaque, owed: vec![all; n], ring, content, fresh: true, pending: Damage::new(cw, ch) });
        Ok(infos)
    }

    /// Every drawable of `id` must be redrawn in full (e.g. after it moved to the lock surface).
    pub fn invalidate(&mut self, id: u32) {
        if let Some(s) = self.surfaces.get_mut(&id) {
            for d in &mut s.owed {
                d.all();
            }
            s.fresh = true;
        }
    }

    /// Records content damage that could not be presented yet; it is uploaded with the next frame.
    pub fn owe(&mut self, id: u32, damage: &Damage) {
        if let Some(s) = self.surfaces.get_mut(&id) {
            s.pending.union(damage);
        }
    }

    /// Sets the window shadow; a change redraws the whole drawable.
    pub fn set_shadow(&mut self, id: u32, shadow: Option<Shadow>) {
        let Some(s) = self.surfaces.get_mut(&id) else { return };
        if s.shadow != shadow {
            s.shadow = shadow;
            for d in &mut s.owed {
                d.all();
            }
        }
    }

    /// Sets the surface transform (open, close, minimize effects); a change redraws the whole drawable.
    /// Returns true while the transform is not the identity (the caller keeps animating).
    pub fn set_fx(&mut self, id: u32, fx: bifrost_render::glass::Fx) -> bool {
        let Some(s) = self.surfaces.get_mut(&id) else { return false };
        if s.fx != fx {
            s.fx = fx;
            for d in &mut s.owed {
                d.all();
            }
        }
        !fx.is_identity()
    }

    /// Turns the live backdrop on or off (off under Reduce Transparency: glass is then not drawn at all,
    /// and on slow software paths: glass falls back to the static wallpaper blur).
    pub fn set_live(&mut self, on: bool) {
        if self.live_on != on {
            self.live_on = on;
            self.live_dirty = on;
            self.live_changed = true;
            self.under_dirty_from(0);
        }
    }

    /// The windows behind the glass, bottom first, with the screen position of each content rect.
    /// A change rebuilds the backdrop with the next draw.
    pub fn set_window_stack(&mut self, stack: Vec<(u32, i32, i32)>) {
        if self.stack != stack {
            let from = stack_change_from(&self.stack, &stack);
            self.stack = stack;
            self.live_dirty = true;
            self.live_changed = true;
            let keep: Vec<u32> = self.stack.iter().map(|t| t.0).collect();
            let gone: Vec<u32> = self.under.keys().copied().filter(|k| !keep.contains(k)).collect();
            for k in gone {
                if let Some(l) = self.under.remove(&k).and_then(|u| u.live) {
                    self.free_live(l);
                }
            }
            self.under_dirty_from(from + 1);
        }
    }

    /// Marks dirty the backdrops of every window at or above stack position `from` (a window at `from - 1` or a
    /// position change below them altered what they show).
    fn under_dirty_from(&mut self, from: usize) {
        under_dirty_from(&self.stack, &mut self.under, &mut self.under_changed, from);
    }

    fn free_live(&self, l: Live) {
        self.free(l.base);
        for t in l.down.into_iter().chain(l.up) {
            self.free(t);
        }
    }

    /// Ms until a backdrop rebuild the 30 Hz cap skipped comes due (the main loop's one-shot timeout), or None.
    pub fn retry_in_ms(&self) -> Option<i32> {
        let left = |at: Option<std::time::Instant>| 33i32.saturating_sub(at.map_or(33, |t| t.elapsed().as_millis().min(33) as i32));
        std::iter::once((self.live_wait, self.live_built)).chain(self.under.values().map(|u| (u.wait, u.built))).filter(|w| w.0).map(|w| left(w.1)).min()
    }

    /// True once a skipped rebuild is due: the glass that samples it owes a redraw (`take_live_changed`,
    /// `take_under_changed`), which rebuilds the backdrop when it draws.
    pub fn retry_now(&mut self) -> bool {
        if self.retry_in_ms() != Some(0) {
            return false;
        }
        self.live_changed |= std::mem::take(&mut self.live_wait);
        for (&k, u) in self.under.iter_mut() {
            if std::mem::take(&mut u.wait) && !self.under_changed.contains(&k) {
                self.under_changed.push(k);
            }
        }
        true
    }

    /// The windows (presenter keys) whose backdrop changed since the last call: the caller marks their material
    /// rects owed (`owe_rects`) so they pick up the new backdrop.
    pub fn take_under_changed(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.under_changed)
    }

    /// True once after any window content or position changed: the caller redraws the glass shapes
    /// (see `owe_rects`) so they pick up the new backdrop.
    pub fn take_live_changed(&mut self) -> bool {
        std::mem::take(&mut self.live_changed)
    }

    /// True once after `owe_rects`: the caller presents even when the content has no damage.
    pub fn take_flush(&mut self) -> bool {
        std::mem::take(&mut self.flush)
    }

    /// Marks `rects` (px, in the drawable of surface `id`) as owed, without touching the content.
    pub fn owe_rects(&mut self, id: u32, rects: &[[f32; 4]]) {
        self.flush |= !rects.is_empty();
        let Some(s) = self.surfaces.get_mut(&id) else { return };
        for d in &mut s.owed {
            for r in rects {
                let (x0, y0) = (r[0].max(0.0) as i32, r[1].max(0.0) as i32);
                let (x1, y1) = (((r[0] + r[2]).ceil() as i32).min(s.w as i32), ((r[1] + r[3]).ceil() as i32).min(s.h as i32));
                if x1 > x0 && y1 > y0 {
                    d.add(x0, y0, x1 - x0, y1 - y0);
                }
            }
        }
    }

    /// Renders wallpaper + windows into the quarter-size base and blurs it (dual Kawase, 3 down, 2 up).
    /// Runs at most once per change; the pyramid textures are kept between rebuilds. `under` builds that
    /// window's backdrop, which leaves the window and those above it out.
    fn rebuild_live(&mut self, under: Option<u32>) {
        if under.is_none() {
            self.live_dirty = false;
        }
        let Some((wp, wx, wy)) = &self.wallpaper else { return };
        let (sw, sh, wx, wy) = (wp.w, wp.h, *wx, *wy);
        let (lw, lh) = ((sw / LIVE_DIV).max(2), (sh / LIVE_DIV).max(2));
        let mut slot = match under {
            Some(k) => self.under.get_mut(&k).and_then(|u| u.live.take()),
            None => self.live.take(),
        };
        if slot.as_ref().is_none_or(|l| l.base.w != lw || l.base.h != lh) {
            if let Some(old) = slot.take() {
                self.free_live(old);
            }
            let Ok(fresh) = self.alloc_live(lw, lh) else { return };
            slot = Some(fresh);
        }
        let Some(live) = slot else { return };
        let windows = match under {
            Some(k) => self.stack.iter().position(|t| t.0 == k).unwrap_or(0),
            None => self.stack.len(),
        };
        let gl = self.gl;
        let k = lw as f32 / sw as f32;
        gl!(gl, disable(GL_SCISSOR_TEST));
        gl!(gl, use_program(self.prog.id));
        gl!(gl, bind_vertex_array(self.vao));
        gl!(gl, active_texture(GL_TEXTURE0));
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, live.base.fbo));
        gl!(gl, viewport(0, 0, lw as i32, lh as i32));
        gl!(gl, uniform2f(self.prog.view, lw as f32, lh as f32));
        gl!(gl, uniform1i(self.prog.mode, 0));
        gl!(gl, uniform1f(self.prog.glass, 0.0));
        gl!(gl, disable(GL_BLEND));
        self.quad(self.wallpaper.as_ref().map_or(0, |w| w.0.tex), [wx as f32 * k, wy as f32 * k, sw as f32 * k, sh as f32 * k], 1.0, None);
        gl!(gl, enable(GL_BLEND));
        gl!(gl, blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA));
        for &(id, x, y) in &self.stack[..windows] {
            let Some(s) = self.surfaces.get(&id) else { continue };
            let f = s.fx;
            if f.alpha <= 0.0 {
                continue;
            }
            let (w, h) = (s.cw as f32 * f.scale, s.ch as f32 * f.scale);
            let r = [(x as f32 + (s.cw as f32 - w) / 2.0 + f.dx) * k, (y as f32 + (s.ch as f32 - h) / 2.0 + f.dy) * k, w * k, h * k];
            // content is sampled 1:1 (nearest) on screen; the quarter-size copy filters it, or text shimmers
            // through the glass as it scrolls
            gl!(gl, bind_texture(GL_TEXTURE_2D, s.content.tex));
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR));
            self.quad(s.content.tex, r, f.alpha, None);
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST));
        }
        gl!(gl, disable(GL_BLEND));
        let mut src = (live.base.tex, live.base.w, live.base.h);
        for d in &live.down {
            self.pass(d, src, 1);
            src = (d.tex, d.w, d.h);
        }
        for u in &live.up {
            self.pass(u, src, 2);
            src = (u.tex, u.w, u.h);
        }
        match under {
            Some(k) => match self.under.get_mut(&k) {
                Some(u) => u.live = Some(live),
                None => self.free_live(live),
            },
            None => self.live = Some(live),
        }
    }

    /// Allocates a backdrop pyramid: a `lw` x `lh` base, three halvings down and two back up. A failure part way
    /// frees what was already made, so a GPU out of memory costs no textures.
    fn alloc_live(&self, lw: usize, lh: usize) -> io::Result<Live> {
        let base = self.texture(lw, lh, GL_LINEAR, None)?;
        let mut down: Vec<Tex> = Vec::with_capacity(3);
        let (mut w, mut h) = (lw, lh);
        for _ in 0..3 {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            match self.texture(w, h, GL_LINEAR, None) {
                Ok(t) => down.push(t),
                Err(e) => {
                    self.free_live(Live { base, down, up: Vec::new() });
                    return Err(e);
                }
            }
        }
        let mut up: Vec<Tex> = Vec::with_capacity(2);
        for i in (0..2).rev() {
            let (uw, uh) = (down[i].w, down[i].h);
            match self.texture(uw, uh, GL_LINEAR, None) {
                Ok(t) => up.push(t),
                Err(e) => {
                    self.free_live(Live { base, down, up });
                    return Err(e);
                }
            }
        }
        Ok(Live { base, down, up })
    }

    pub fn has_surface(&self, id: u32) -> bool {
        self.surfaces.contains_key(&id)
    }

    /// Allocates one more drawable for surface `id` (owing a full redraw); returns it for a `wl_buffer`.
    pub fn add_drawable(&mut self, id: u32) -> io::Result<BufferInfo> {
        self.current();
        let Some(s) = self.surfaces.get(&id) else { return Err(io::Error::other("no such surface")) };
        let (w, h, opaque) = (s.w, s.h, s.opaque);
        let fourcc = if opaque { FOURCC_XB24 } else { FOURCC_AB24 };
        let bo = self.gbm.alloc(w, h, fourcc, self.linear)?;
        let (image, tex, fbo) = self.import(&bo, w, h, fourcc)?;
        let info = BufferInfo { fd: bo.fd, stride: bo.stride, offset: bo.offset, modifier: bo.modifier, fourcc, w, h };
        let s = self.surfaces.get_mut(&id).unwrap();
        let mut all = Damage::new(w, h);
        all.all();
        s.ring.push(Drawable { _bo: bo, image, tex, fbo });
        s.owed.push(all);
        Ok(info)
    }

    /// Frees the last drawable of `id` (the caller destroyed its `wl_buffer` and it is not on screen).
    pub fn pop_drawable(&mut self, id: u32) {
        self.current();
        let Some(s) = self.surfaces.get_mut(&id) else { return };
        if let Some(d) = s.ring.pop() {
            s.owed.pop();
            self.release(d);
        }
    }

    fn release(&self, d: Drawable) {
        gl!(self.gl, delete_framebuffers(1, &d.fbo));
        gl!(self.gl, delete_textures(1, &d.tex));
        // SAFETY: the image was created on this display and its GL users are gone.
        unsafe { (self.ext.destroy_image)(self.dpy, d.image) };
    }

    pub fn destroy_surface(&mut self, id: u32) {
        let Some(s) = self.surfaces.remove(&id) else { return };
        self.current();
        for d in s.ring {
            self.release(d);
        }
        self.free(s.content);
    }

    /// Uploads the wallpaper (once per change) at screen position (`x`, `y`) and blurs it on the GPU.
    pub fn set_wallpaper(&mut self, wp: View, x: i32, y: i32) {
        self.current();
        let key = (wp.px.as_ptr() as usize, wp.px.len(), x, WALLPAPER_GEN.load(std::sync::atomic::Ordering::Relaxed));
        if key == self.wallpaper_key && self.wallpaper.is_some() {
            return;
        }
        self.wallpaper_key = key;
        if let Some((t, ..)) = self.wallpaper.take() {
            self.free(t);
        }
        for t in std::mem::take(&mut self.blur) {
            self.free(t);
        }
        let Ok(t) = self.texture(wp.w, wp.h, GL_LINEAR, Some(wp.px)) else { return };
        // the blur pyramid: down LEVELS times, up LEVELS-1 times; the last texture is the material
        let mut sizes = Vec::new();
        let (mut w, mut h) = (wp.w, wp.h);
        for _ in 0..LEVELS {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            sizes.push((w, h));
        }
        let mut chain: Vec<Tex> = Vec::new();
        for &(w, h) in &sizes {
            match self.texture(w, h, GL_LINEAR, None) {
                Ok(c) => chain.push(c),
                Err(_) => {
                    // free the source and the part of the chain already made: nothing is kept on failure
                    for c in chain {
                        self.free(c);
                    }
                    self.free(t);
                    return;
                }
            }
        }
        let gl = self.gl;
        gl!(gl, disable(GL_SCISSOR_TEST));
        gl!(gl, disable(GL_BLEND));
        gl!(gl, use_program(self.prog.id));
        gl!(gl, uniform1i(self.prog.mode, 1));
        let mut src = (t.tex, t.w, t.h);
        for c in &chain {
            self.pass(c, src, 1);
            src = (c.tex, c.w, c.h);
        }
        for i in (0..LEVELS - 1).rev() {
            let (w, h) = (chain[i].w, chain[i].h);
            if let Ok(up) = self.texture(w, h, GL_LINEAR, None) {
                self.pass(&up, src, 2);
                src = (up.tex, up.w, up.h);
                self.blur.push(up);
            }
        }
        for c in chain {
            self.free(c);
        }
        self.wallpaper = Some((t, x, y));
        for s in self.surfaces.values_mut() {
            for d in &mut s.owed {
                d.all();
            }
        }
    }

    /// Renders surface `id` into drawable `slot`: uploads the content's `damage`, then redraws what
    /// that drawable still owes. Returns the areas to report as buffer damage (empty: nothing new).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, id: u32, slot: usize, content: &[u32], stride: usize, damage: &Damage, back: Backdrop, glass: &[Glass]) -> Vec<Area> {
        // a shell surface (not a client window) may carry glass over the live backdrop
        let windowed = !is_window(id);
        self.current();
        if !self.health.is_ok() {
            return Vec::new();
        }
        let t0 = std::time::Instant::now();
        let gl = self.gl;
        let Some(s) = self.surfaces.get_mut(&id) else { return Vec::new() };
        if slot >= s.ring.len() || content.len() < stride * (s.ch.saturating_sub(1)) + s.cw {
            return Vec::new();
        }
        // 1. the content layer: only the damaged rectangles cross the bus
        gl!(gl, bind_texture(GL_TEXTURE_2D, s.content.tex));
        gl!(gl, pixel_storei(GL_UNPACK_ROW_LENGTH, stride as i32));
        let mut up = 0;
        let full = [Area { x0: 0, y0: 0, x1: s.cw, y1: s.ch }];
        s.pending.union(damage);
        let areas: Vec<Area> = if s.fresh { full.to_vec() } else { s.pending.areas().to_vec() };
        s.pending.clear();
        for a in &areas {
            let (x1, y1) = (a.x1.min(s.cw), a.y1.min(s.ch));
            if a.x0 >= x1 || a.y0 >= y1 {
                continue;
            }
            let p = content[a.y0 * stride + a.x0..].as_ptr() as *const c_void;
            gl!(gl, tex_sub_image_2d(GL_TEXTURE_2D, 0, a.x0 as i32, a.y0 as i32, (x1 - a.x0) as i32, (y1 - a.y0) as i32, GL_RGBA, GL_UNSIGNED_BYTE, p));
            up += (x1 - a.x0) * (y1 - a.y0);
        }
        gl!(gl, pixel_storei(GL_UNPACK_ROW_LENGTH, 0));
        for d in &mut s.owed {
            for a in &areas {
                d.add((a.x0 + s.cx) as i32, (a.y0 + s.cy) as i32, (a.x1 - a.x0) as i32, (a.y1 - a.y0) as i32);
            }
        }
        s.fresh = false;
        let t1 = std::time::Instant::now();
        if is_window(id) && (!areas.is_empty() || s.fx != bifrost_render::glass::Fx::IDENTITY) {
            self.live_dirty = self.live_on;
            self.live_changed = self.live_on;
            // the windows above this one blur it
            if self.live_on {
                if let Some(i) = self.stack.iter().position(|t| t.0 == id) {
                    under_dirty_from(&self.stack, &mut self.under, &mut self.under_changed, i + 1);
                }
            }
        }
        // 2. one render pass over what this drawable owes
        let owed: Vec<Area> = s.owed[slot].areas().to_vec();
        s.owed[slot].clear();
        if owed.is_empty() {
            return owed;
        }
        let (w, h, opaque) = (s.w, s.h, s.opaque);
        let crect = [s.cx as f32, s.cy as f32, s.cw as f32, s.ch as f32];
        let shadow = s.shadow;
        let fx = s.fx;
        let margin = (s.cx.min(s.cy)) as f32;
        // the content rect after the surface transform: scaled about its centre, then shifted
        let (dw, dh) = (crect[2] * fx.scale, crect[3] * fx.scale);
        let drect = [crect[0] + (crect[2] - dw) / 2.0 + fx.dx, crect[1] + (crect[3] - dh) / 2.0 + fx.dy, dw, dh];
        let fbo = s.ring[slot].fbo;
        let content_tex = s.content.tex;
        // glass over shell surfaces blurs the live backdrop (windows behind it), rebuilt once per change
        let due = |at: Option<std::time::Instant>| at.is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(33));
        if windowed && self.live_on && self.live_dirty && !glass.is_empty() {
            if due(self.live_built) {
                self.live_built = Some(std::time::Instant::now());
                self.live_wait = false;
                self.rebuild_live(None);
            } else {
                self.live_wait = true;
            }
        }
        // a window's behind-window materials blur the windows under it, when any of them looks active (an
        // inactive window's materials that follow its state are opaque plates and need no backdrop)
        let active = matches!(back, Backdrop::GlassOnly { active: true, .. });
        let looks_active = |g: &Glass| g.active.unwrap_or(active || !g.kind.style().follows_active);
        let under_glass = !windowed && self.live_on && self.stack.iter().any(|t| t.0 == id) && glass.iter().any(looks_active);
        if under_glass {
            let u = self.under.entry(id).or_insert(Under { live: None, dirty: true, built: None, wait: false });
            if u.dirty {
                if due(u.built) {
                    u.built = Some(std::time::Instant::now());
                    u.wait = false;
                    u.dirty = false;
                    self.rebuild_live(Some(id));
                } else {
                    u.wait = true;
                }
            }
        } else if !windowed {
            // no active-looking material any more (the window went inactive): drop its backdrop
            if let Some(l) = self.under.remove(&id).and_then(|u| u.live) {
                self.free_live(l);
            }
        }
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
        gl!(gl, viewport(0, 0, w as i32, h as i32));
        gl!(gl, use_program(self.prog.id));
        gl!(gl, uniform2f(self.prog.view, w as f32, h as f32));
        gl!(gl, uniform1i(self.prog.mode, 0));
        gl!(gl, uniform4f(self.prog.hole, 0.0, 0.0, 0.0, 0.0));
        gl!(gl, active_texture(GL_TEXTURE0));
        gl!(gl, bind_vertex_array(self.vao));
        gl!(gl, blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA));
        gl!(gl, enable(GL_SCISSOR_TEST));
        let (ox, oy, full_wp) = match back {
            Backdrop::Full { x, y } => (x, y, true),
            Backdrop::GlassOnly { x, y, .. } => (x, y, false),
            Backdrop::None => (0, 0, false),
        };
        let glass = if matches!(back, Backdrop::None) { &[][..] } else { glass };
        let mut drawn = 0;
        for a in &owed {
            gl!(gl, scissor(a.x0 as i32, a.y0 as i32, (a.x1 - a.x0) as i32, (a.y1 - a.y0) as i32));
            drawn += a.pixels();
            gl!(gl, disable(GL_BLEND));
            if opaque {
                gl!(gl, clear_color(0.0, 0.0, 0.0, 1.0));
            } else {
                gl!(gl, clear_color(0.0, 0.0, 0.0, 0.0));
            }
            gl!(gl, clear(GL_COLOR_BUFFER_BIT));
            gl!(gl, enable(GL_BLEND));
            if let Some((wp, wx, wy)) = &self.wallpaper {
                let dst = [(*wx - ox) as f32, (*wy - oy) as f32, wp.w as f32, wp.h as f32];
                if full_wp {
                    self.quad(wp.tex, dst, 1.0, None);
                }
                let live_tex = match () {
                    _ if !self.live_on || self.stack.is_empty() => None,
                    _ if windowed => self.live.as_ref().and_then(|l| l.up.last()),
                    _ if under_glass => self.under.get(&id).and_then(|u| u.live.as_ref()).and_then(|l| l.up.last()),
                    _ => None,
                };
                if let Some(b) = live_tex.or(self.blur.last()) {
                    for g in glass {
                        let r = [g.x - ox as f32, g.y - oy as f32, g.w, g.h];
                        if r[0] >= a.x1 as f32 || r[1] >= a.y1 as f32 || r[0] + r[2] <= a.x0 as f32 || r[1] + r[3] <= a.y0 as f32 {
                            continue;
                        }
                        let style = g.kind.style();
                        // `.active` / `.inactive` fix the look; `.followsWindowActiveState` takes the window's
                        if !looks_active(g) {
                            // `.followsWindowActiveState`: an inactive window's material is an opaque plate
                            let c = g.kind.inactive();
                            let ch = |sh: u32| ((c >> sh) & 255) as f32 / 255.0;
                            gl!(gl, uniform1i(self.prog.mode, 4));
                            gl!(gl, uniform4f(self.prog.color, ch(0), ch(8), ch(16), ch(24)));
                            gl!(gl, uniform4f(self.prog.rrect, r[0], r[1], r[2], r[3]));
                            gl!(gl, uniform1f(self.prog.radius, g.r.min(r[2] / 2.0).min(r[3] / 2.0)));
                            self.quad_raw(r);
                            gl!(gl, uniform1i(self.prog.mode, 0));
                            continue;
                        }
                        // only the glass shape's bounds are shaded, with the matching part of the blur
                        let uv = [(r[0] - dst[0]) / dst[2], (r[1] - dst[1]) / dst[3], (r[0] + r[2] - dst[0]) / dst[2], (r[1] + r[3] - dst[1]) / dst[3]];
                        gl!(gl, uniform1f(self.prog.glass, 1.0));
                        gl!(gl, uniform1f(self.prog.sat, style.saturation));
                        gl!(gl, uniform1f(self.prog.bright, style.brightness));
                        gl!(gl, uniform1f(self.prog.lum_amt, style.lum_amount));
                        gl!(gl, uniform4f(self.prog.lum_v, style.lum_values[0], style.lum_values[1], style.lum_values[2], style.lum_values[3]));
                        gl!(gl, uniform1f(self.prog.rim, style.rim));
                        // Liquid Glass clarity: clear lets the sharp wallpaper show through the blur
                        // (only when the sharp wallpaper was drawn underneath), tinted is fully blurred.
                        let mix = if full_wp { 0.5 + 0.5 * bifrost_render::glass::clarity() } else { 1.0 };
                        self.quad_uv(b.tex, r, uv, mix, Some((r, g.r)));
                        gl!(gl, uniform1f(self.prog.glass, 0.0));
                    }
                }
            }
            if let Some(sh) = shadow {
                self.shadow(drect, margin, sh, fx.shadow * fx.alpha, fx.scale);
            }
            self.quad(content_tex, drect, fx.alpha, None);
        }
        gl!(gl, disable(GL_SCISSOR_TEST));
        self.finish();
        if !self.health.is_ok() {
            // the pass failed: the drawable holds no complete frame, so nothing is reported as presented
            return Vec::new();
        }
        let t2 = std::time::Instant::now();
        self.stats = Stats {
            upload_ms: (t1 - t0).as_secs_f64() * 1e3,
            render_ms: (t2 - t1).as_secs_f64() * 1e3,
            uploaded_px: up,
            drawn_px: drawn,
        };
        owed
    }

    /// Two soft layers (ambient and key light, offset down) under the content rect, then the 1 px
    /// dark hairline just outside it. Fitted to the Golden Gate Files capture (2x, margin ~52 pt):
    /// the key window's shadow is ~30% under the bottom edge, ~18% at the sides, ~7% above, and
    /// fades out ~40 pt from the edge (bottom further, it sits ~16 pt down). Inactive windows get
    /// about half the depth and reach.
    fn shadow(&self, r: [f32; 4], m: f32, sh: Shadow, k: f32, scale: f32) {
        let gl = self.gl;
        let m = m.max(1.0);
        let layers = if sh.focused {
            [(m * 0.80, m * 0.30, 0.20), (m * 0.20, m * 0.30, 0.10), (1.0, 0.0, 0.16)]
        } else {
            [(m * 0.45, m * 0.12, 0.10), (m * 0.15, m * 0.12, 0.05), (1.0, 0.0, 0.12)]
        };
        gl!(gl, uniform1i(self.prog.mode, 3));
        for (blur, dy, a) in layers {
            gl!(gl, uniform2f(self.prog.half, blur, 0.0));
            gl!(gl, uniform4f(self.prog.rrect, r[0], r[1] + dy, r[2], r[3]));
            gl!(gl, uniform1f(self.prog.radius, sh.radius * scale));
            gl!(gl, uniform1f(self.prog.alpha, a * k));
            // layers that reach inside the rect are masked by the true content rect
            gl!(gl, uniform4f(self.prog.hole, r[0], r[1], r[2], r[3]));
            gl!(gl, uniform1f(self.prog.hole_r, sh.radius * scale));
            let (x0, y0, x1, y1) = (r[0] - m, r[1] - m, r[0] + r[2] + m, r[1] + r[3] + m);
            let v: [f32; 24] = [x0, y0, 0., 0., x1, y0, 1., 0., x0, y1, 0., 1., x1, y0, 1., 0., x1, y1, 1., 1., x0, y1, 0., 1.];
            gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
            gl!(gl, buffer_data(GL_ARRAY_BUFFER, 96, v.as_ptr() as *const c_void, GL_STREAM_DRAW));
            gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
        }
        gl!(gl, uniform4f(self.prog.hole, 0.0, 0.0, 0.0, 0.0));
        gl!(gl, uniform1i(self.prog.mode, 0));
    }

    /// Makes the presenter's context current (another EGL user on the thread may have switched it). A context that
    /// will not become current is a lost GPU: it is recorded as the failure.
    fn current(&mut self) {
        // SAFETY: dpy/ctx are live; surf is null (surfaceless) or the pbuffer made with the context.
        let ok = unsafe { (self.egl.make_current)(self.dpy, self.surf, self.surf, self.ctx) } != 0;
        if !ok {
            self.health.fail(|| "eglMakeCurrent failed: the GPU context is lost".to_string());
        }
    }

    /// Waits until the GPU finished the pass (vendor neutral: implicit sync is not assumed). A GL error flag, or a
    /// pass that does not finish within the wait, is recorded in `health` instead of blocking the shell for good.
    fn finish(&mut self) {
        if self.implicit_sync {
            // Mesa drivers fence dma-bufs implicitly: the compositor's reads wait for this pass
            gl!(self.gl, flush());
        } else {
            let waited = self.ext.khr.and_then(|k| {
                // SAFETY: dpy is initialised and the context current; the sync is destroyed after the wait.
                unsafe {
                    let sync = (k.create_sync)(self.dpy, EGL_SYNC_FENCE_KHR, ptr::null());
                    if sync.is_null() {
                        return None;
                    }
                    let r = (k.client_wait)(self.dpy, sync, EGL_SYNC_FLUSH_COMMANDS_BIT_KHR, 100_000_000);
                    (k.destroy_sync)(self.dpy, sync);
                    Some(r)
                }
            });
            match waited {
                Some(r) => {
                    self.health.fence("presenter pass", r);
                }
                None => gl!(self.gl, finish()),
            }
        }
        let e = gl!(self.gl, get_error());
        if let Some(msg) = self.health.gl_flag("presenter pass", e) {
            bifrost_core::log::shell_logger("presenter").error(msg);
        }
    }

    /// Draws `tex` over `dst` (x, y, w, h in target pixels), clipped to a rounded rect when given.
    fn quad(&self, tex: u32, dst: [f32; 4], alpha: f32, clip: Option<([f32; 4], f32)>) {
        self.quad_uv(tex, dst, [0.0, 0.0, 1.0, 1.0], alpha, clip);
    }

    /// Draws the rectangle `dst` with the current mode and uniforms (no texture, no clip uniforms).
    fn quad_raw(&self, dst: [f32; 4]) {
        let gl = self.gl;
        let [x, y, w, h] = dst;
        let v: [f32; 24] = [x, y, 0., 0., x + w, y, 1., 0., x, y + h, 0., 1., x + w, y, 1., 0., x + w, y + h, 1., 1., x, y + h, 0., 1.];
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
        gl!(gl, buffer_data(GL_ARRAY_BUFFER, 96, v.as_ptr() as *const c_void, GL_STREAM_DRAW));
        gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
    }

    /// `quad` with explicit texture coordinates (u0, v0, u1, v1).
    fn quad_uv(&self, tex: u32, dst: [f32; 4], uv: [f32; 4], alpha: f32, clip: Option<([f32; 4], f32)>) {
        let gl = self.gl;
        let [x, y, w, h] = dst;
        let [u0, v0, u1, v1] = uv;
        let v: [f32; 24] = [x, y, u0, v0, x + w, y, u1, v0, x, y + h, u0, v1, x + w, y, u1, v0, x + w, y + h, u1, v1, x, y + h, u0, v1];
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
        gl!(gl, buffer_data(GL_ARRAY_BUFFER, 96, v.as_ptr() as *const c_void, GL_STREAM_DRAW));
        gl!(gl, bind_texture(GL_TEXTURE_2D, tex));
        gl!(gl, uniform1f(self.prog.alpha, alpha));
        match clip {
            Some((r, rad)) => {
                gl!(gl, uniform4f(self.prog.rrect, r[0], r[1], r[2], r[3]));
                gl!(gl, uniform1f(self.prog.radius, rad.min(r[2] / 2.0).min(r[3] / 2.0)));
            }
            None => gl!(gl, uniform1f(self.prog.radius, -1.0)),
        }
        gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
    }

    /// The final-frame zoom (Settings > Accessibility > Zoom): draws the composed frame `src` magnified into the
    /// framebuffer `dst_fbo`, showing only the normalised rect `uv` = (u0, v0, u1, v1) of it. The shell supplies
    /// the rect from `shell::accessibility::zoom`; with zoom off the caller skips this pass.
    pub fn zoom_into(&self, src: u32, dst_fbo: u32, dst_w: usize, dst_h: usize, uv: [f32; 4]) {
        self.current();
        let gl = self.gl;
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, dst_fbo));
        gl!(gl, viewport(0, 0, dst_w as i32, dst_h as i32));
        gl!(gl, uniform2f(self.prog.view, dst_w as f32, dst_h as f32));
        gl!(gl, uniform1i(self.prog.mode, 0));
        self.quad_uv(src, [0.0, 0.0, dst_w as f32, dst_h as f32], uv, 1.0, None);
        self.finish();
    }

    /// One blur pass from `src` into `dst`.
    fn pass(&self, dst: &Tex, src: (u32, usize, usize), mode: i32) {
        let gl = self.gl;
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, dst.fbo));
        gl!(gl, viewport(0, 0, dst.w as i32, dst.h as i32));
        gl!(gl, uniform2f(self.prog.view, dst.w as f32, dst.h as f32));
        gl!(gl, uniform1i(self.prog.mode, mode));
        // the downsample reads the source's half texel; the upsample the destination's
        let (hw, hh) = if mode == 1 { (0.5 / src.1 as f32, 0.5 / src.2 as f32) } else { (0.5 / dst.w as f32, 0.5 / dst.h as f32) };
        gl!(gl, uniform2f(self.prog.half, hw, hh));
        self.quad(src.0, [0.0, 0.0, dst.w as f32, dst.h as f32], 1.0, None);
        gl!(gl, uniform1i(self.prog.mode, 0));
    }

    fn texture(&self, w: usize, h: usize, filter: i32, px: Option<&[u32]>) -> io::Result<Tex> {
        let gl = self.gl;
        let (mut tex, mut fbo) = (0u32, 0u32);
        gl!(gl, gen_textures(1, &mut tex));
        gl!(gl, bind_texture(GL_TEXTURE_2D, tex));
        gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter));
        gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter));
        gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32));
        gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32));
        let data = px.map_or(ptr::null(), |p| p.as_ptr() as *const c_void);
        gl!(gl, tex_image_2d(GL_TEXTURE_2D, 0, GL_RGBA as i32, w as i32, h as i32, 0, GL_RGBA, GL_UNSIGNED_BYTE, data));
        gl!(gl, gen_framebuffers(1, &mut fbo));
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
        gl!(gl, framebuffer_texture_2d(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0));
        let st = gl!(gl, check_framebuffer_status(GL_FRAMEBUFFER));
        if st != GL_FRAMEBUFFER_COMPLETE {
            gl!(gl, delete_framebuffers(1, &fbo));
            gl!(gl, delete_textures(1, &tex));
            return Err(super::error::fbo_error(st));
        }
        Ok(Tex { tex, fbo, w, h })
    }

    fn free(&self, t: Tex) {
        gl!(self.gl, delete_framebuffers(1, &t.fbo));
        gl!(self.gl, delete_textures(1, &t.tex));
    }

    /// Imports a GBM buffer as an EGL image bound to a texture and an FBO.
    fn import(&self, bo: &Bo, w: usize, h: usize, fourcc: u32) -> io::Result<(P, u32, u32)> {
        let mut attrs = vec![
            EGL_WIDTH,
            w as i32,
            EGL_HEIGHT,
            h as i32,
            EGL_LINUX_DRM_FOURCC_EXT,
            fourcc as i32,
            EGL_DMA_BUF_PLANE0_FD_EXT,
            bo.fd,
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            bo.offset as i32,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            bo.stride as i32,
        ];
        if bo.modifier != MOD_INVALID && self.has_modifiers {
            attrs.extend([EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, bo.modifier as u32 as i32, EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, (bo.modifier >> 32) as u32 as i32]);
        }
        attrs.push(EGL_NONE);
        // SAFETY: attrs is EGL_NONE terminated; the fd stays open for the call.
        let image = unsafe { (self.ext.create_image)(self.dpy, ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, ptr::null_mut(), attrs.as_ptr()) };
        if image.is_null() {
            // SAFETY: eglGetError has no preconditions.
            return Err(egl_error("eglCreateImageKHR(dma-buf)", unsafe { (self.egl.get_error)() }));
        }
        let gl = self.gl;
        let (mut tex, mut fbo) = (0u32, 0u32);
        gl!(gl, gen_textures(1, &mut tex));
        gl!(gl, bind_texture(GL_TEXTURE_2D, tex));
        // SAFETY: the image is live and the texture bound on the current context.
        unsafe { (self.ext.image_target_texture)(GL_TEXTURE_2D, image) };
        gl!(gl, gen_framebuffers(1, &mut fbo));
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
        gl!(gl, framebuffer_texture_2d(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0));
        let st = gl!(gl, check_framebuffer_status(GL_FRAMEBUFFER));
        if st != GL_FRAMEBUFFER_COMPLETE {
            gl!(gl, delete_framebuffers(1, &fbo));
            gl!(gl, delete_textures(1, &tex));
            // SAFETY: created above on this display.
            unsafe { (self.ext.destroy_image)(self.dpy, image) };
            return Err(super::error::fbo_error(st));
        }
        Ok((image, tex, fbo))
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        let ids: Vec<u32> = self.surfaces.keys().copied().collect();
        for id in ids {
            self.destroy_surface(id);
        }
        if let Some((t, ..)) = self.wallpaper.take() {
            self.free(t);
        }
        for t in std::mem::take(&mut self.blur) {
            self.free(t);
        }
        let unders: Vec<Live> = self.under.drain().filter_map(|(_, u)| u.live).collect();
        for l in self.live.take().into_iter().chain(unders) {
            self.free_live(l);
        }
        gl!(self.gl, delete_buffers(1, &self.vbo));
        gl!(self.gl, delete_vertex_arrays(1, &self.vao));
        gl!(self.gl, delete_program(self.prog.id));
        // SAFETY: handles were created in `open`; release the context first, then its objects.
        unsafe {
            (self.egl.make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            if !self.surf.is_null() {
                (self.egl.destroy_surface)(self.dpy, self.surf);
            }
            (self.egl.destroy_context)(self.dpy, self.ctx);
        }
    }
}

fn compile(gl: &Gles, kind: u32, src: &str) -> io::Result<u32> {
    let c = CString::new(src).map_err(|_| io::ErrorKind::InvalidInput)?;
    let sh = gl!(gl, create_shader(kind));
    let p = c.as_ptr();
    gl!(gl, shader_source(sh, 1, &p, ptr::null()));
    gl!(gl, compile_shader(sh));
    let mut ok = 0;
    gl!(gl, get_shaderiv(sh, GL_COMPILE_STATUS, &mut ok));
    if ok == 0 {
        let mut buf = vec![0u8; 2048];
        let mut n = 0;
        gl!(gl, get_shader_info_log(sh, buf.len() as i32, &mut n, buf.as_mut_ptr() as *mut _));
        buf.truncate(n.max(0) as usize);
        return Err(io::Error::other(format!("presenter shader: {}", String::from_utf8_lossy(&buf))));
    }
    Ok(sh)
}

fn link(gl: &Gles) -> io::Result<Program> {
    let vs = compile(gl, GL_VERTEX_SHADER, VS)?;
    let fs = compile(gl, GL_FRAGMENT_SHADER, FS)?;
    let id = gl!(gl, create_program());
    gl!(gl, attach_shader(id, vs));
    gl!(gl, attach_shader(id, fs));
    gl!(gl, link_program(id));
    gl!(gl, delete_shader(vs));
    gl!(gl, delete_shader(fs));
    let mut ok = 0;
    gl!(gl, get_programiv(id, GL_LINK_STATUS, &mut ok));
    if ok == 0 {
        return Err(io::Error::other("presenter program link failed"));
    }
    gl!(gl, use_program(id));
    let loc = |n: &str| {
        let c = CString::new(n).unwrap_or_default();
        gl!(gl, get_uniform_location(id, c.as_ptr()))
    };
    let tex = loc("u_tex");
    gl!(gl, uniform1i(tex, 0));
    Ok(Program { id, view: loc("u_view"), mode: loc("u_mode"), half: loc("u_half"), alpha: loc("u_alpha"), rrect: loc("u_rrect"), radius: loc("u_radius"), glass: loc("u_glass"), hole: loc("u_hole"), hole_r: loc("u_hole_r"), sat: loc("u_sat"), bright: loc("u_bright"), lum_amt: loc("u_lum_amt"), lum_v: loc("u_lum_v"), rim: loc("u_rim"), color: loc("u_color") })
}

#[cfg(test)]
mod stack_tests {
    use super::*;

    fn under_entry() -> Under {
        Under { live: None, dirty: false, built: None, wait: false }
    }

    /// A foreign toplevel is never in the stack and a stale key must not matter: marking dirty touches only
    /// windows at or above `from` that have a backdrop, and never panics.
    #[test]
    fn dirty_from_marks_only_windows_above() {
        let k = |n: u32| WINDOW_KEY_BASE + n;
        let stack = vec![(k(1), 0, 0), (k(2), 10, 10), (k(3), 20, 20)];
        let mut under: HashMap<u32, Under> = HashMap::new();
        under.insert(k(2), under_entry());
        under.insert(k(3), under_entry());
        under.insert(k(9), under_entry());
        let mut changed = Vec::new();
        under_dirty_from(&stack, &mut under, &mut changed, 2);
        assert_eq!(changed, vec![k(3)]);
        assert!(under[&k(3)].dirty && !under[&k(2)].dirty && !under[&k(9)].dirty);
        under_dirty_from(&stack, &mut under, &mut changed, 0);
        assert_eq!(changed, vec![k(3), k(2)]);
    }

    #[test]
    fn a_stack_change_only_dirties_the_windows_above_it() {
        let a = (WINDOW_KEY_BASE + 1, 0, 0);
        let b = (WINDOW_KEY_BASE + 2, 10, 10);
        let c = (WINDOW_KEY_BASE + 3, 20, 20);
        // nothing changed: the first position is past both stacks
        assert_eq!(stack_change_from(&[a, b, c], &[a, b, c]), 3);
        // the top window moved: only the position of the top changed
        assert_eq!(stack_change_from(&[a, b, c], &[a, b, (c.0, 30, 30)]), 2);
        // the bottom window moved: everything above it may see a new backdrop
        assert_eq!(stack_change_from(&[a, b, c], &[(a.0, 1, 0), b, c]), 0);
        // a window added on top changes nothing below it
        assert_eq!(stack_change_from(&[a, b], &[a, b, c]), 2);
    }

    #[test]
    fn window_keys_are_windows() {
        assert!(is_window(WINDOW_KEY_BASE) && !is_window(5) && !is_window(u32::MAX));
    }
}
