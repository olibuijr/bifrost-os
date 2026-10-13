//! GPU rendering for bare mode: EGL + GLES 3.x on Mesa through GBM, drawing into a
//! render-to-texture target. `present` reads back only the damaged rows, unless the runtime
//! probe in `scanout` found a zero-copy method (a dumb buffer exported as a dma-buf): then
//! `present_scanout` hands back a framebuffer to page-flip and no CPU copy happens.
//!
//! Pure parts (rect math, damage merging, vertex batching, shader source,
//! error mapping, texture bookkeeping) are host-testable; the rest needs a GPU.
//! Pixel formats: `gfx::Image` is premultiplied RGBA (R in the low byte of each u32) and
//! uploads unchanged; the XBGR target has the byte order of `gfx` frames.

pub mod batch;
pub mod damage;
pub mod error;
pub mod health;
pub mod nv12;
pub mod overlay;
pub mod plan;
pub mod quad;
pub mod shaders;
pub mod texture;

#[cfg(target_os = "linux")]
pub mod egl;
#[cfg(target_os = "linux")]
pub mod desktop;
#[cfg(target_os = "linux")]
pub mod layer;
#[cfg(target_os = "linux")]
pub mod scanout;
#[cfg(target_os = "linux")]
pub mod target;
#[cfg(target_os = "linux")]
pub mod wl;

/// A texture owned by a `Gpu`; ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub(crate) u32);

#[cfg(target_os = "linux")]
pub use gpu::{FrameFence, Gpu};

#[cfg(target_os = "linux")]
mod gpu {
    use super::batch::{Batch, ATTRIBS, VERTEX_BYTES};
    use super::desktop::{self, Gbm, ReadPixels};
    use super::egl::*;
    use super::error::*;
    use super::target::Target;
    use super::texture::{self, Entry, Textures};
    use super::{damage, shaders, TextureId};
    use crate::bare::scanout::ScanoutBuf;
    use crate::bare::sys;
    use crate::gfx::{Damage, Image};
    use crate::layout::Rect;
    use std::ffi::{c_int, c_void, CString};
    use std::io;
    use std::ptr;

    /// Calls a GL function from a table.
    // SAFETY (all uses): the Gpu owns the EGL context and made it current on
    // the creating thread; Gpu is !Send, so every GL call runs there, and each
    // call site passes pointers to live data of the size the call reads.
    macro_rules! gl {
        ($t:expr, $f:ident($($a:expr),* $(,)?)) => {
            unsafe { ($t.$f)($($a),*) }
        };
    }

    /// How long `present` waits for the GPU before copying anyway.
    const FENCE_TIMEOUT_MS: i32 = 200;

    /// Completion of a frame's GPU work: an native fence fd, or
    /// already complete when the frame was finished with `glFinish`.
    pub struct FrameFence {
        fd: c_int,
    }

    impl FrameFence {
        fn signaled() -> Self {
            FrameFence { fd: -1 }
        }

        pub fn is_signaled(&self) -> bool {
            self.fd < 0
        }

        /// Blocks until the GPU is done (or `timeout_ms` passes); true when done.
        pub fn wait(self, timeout_ms: i32) -> bool {
            self.fd < 0 || sys::wait(&[self.fd], timeout_ms).first().copied().unwrap_or(false)
        }
    }

    impl Drop for FrameFence {
        fn drop(&mut self) {
            if self.fd >= 0 {
                // SAFETY: the fd came from eglDupNativeFenceFDANDROID and is ours alone.
                unsafe { sys::close(self.fd) };
            }
        }
    }

    /// Direct-scanout render targets: `slots[cur]` is empty because that
    /// target lives in `Gpu::target` (so the layer code needs no changes).
    struct ScanRing {
        slots: Vec<Option<Target>>,
        bufs: Vec<ScanoutBuf>,
        cur: usize,
        /// what each target still lacks of the frames shown since it was last drawn
        owed: Vec<Damage>,
        /// the render-to-texture target, parked while scanout is on
        copy_target: Option<Target>,
    }

    pub struct Gpu {
        // the layer compositor (`super::layer`) issues its own GL calls on this context
        pub(in crate::gpu) egl: Egl,
        pub(in crate::gpu) gl: Gles,
        pub(in crate::gpu) ext: EglExt,
        pub(in crate::gpu) dpy: P,
        ctx: P,
        surf: P,
        pub(in crate::gpu) target: Option<Target>,
        program: u32,
        view_loc: i32,
        vao: u32,
        vbo: u32,
        batch: Batch,
        textures: Textures,
        w: usize,
        h: usize,
        swap_rb: bool,
        scan: Option<ScanRing>,
        /// framebuffer a scanout-mode `present` left to page-flip (see `take_flip`)
        flip_fb: Option<u32>,
        /// desktop render-to-texture fallback: reads the damage back when there is no dma-buf scanout
        read_px: Option<ReadPixels>,
        /// keeps the GBM device (and its render node) behind a desktop display alive
        _gbm: Option<Gbm>,
    }

    fn compile(gl: &Gles, kind: u32, src: &str) -> io::Result<u32> {
        let csrc = CString::new(src).map_err(|_| io::ErrorKind::InvalidInput)?;
        let sh = gl!(gl, create_shader(kind));
        let p = csrc.as_ptr();
        gl!(gl, shader_source(sh, 1, &p, ptr::null()));
        gl!(gl, compile_shader(sh));
        let mut ok = 0;
        gl!(gl, get_shaderiv(sh, GL_COMPILE_STATUS, &mut ok));
        if ok == 0 {
            let mut buf = vec![0u8; 2048];
            let mut n = 0;
            gl!(gl, get_shader_info_log(sh, buf.len() as i32, &mut n, buf.as_mut_ptr() as *mut _));
            buf.truncate(n.max(0) as usize);
            gl!(gl, delete_shader(sh));
            return Err(io::Error::other(format!("shader compile: {}", String::from_utf8_lossy(&buf))));
        }
        Ok(sh)
    }

    fn link(gl: &Gles) -> io::Result<(u32, i32)> {
        let vs = compile(gl, GL_VERTEX_SHADER, &shaders::vertex_source())?;
        let fs = compile(gl, GL_FRAGMENT_SHADER, &shaders::fragment_source())?;
        let prog = gl!(gl, create_program());
        gl!(gl, attach_shader(prog, vs));
        gl!(gl, attach_shader(prog, fs));
        gl!(gl, link_program(prog));
        gl!(gl, delete_shader(vs));
        gl!(gl, delete_shader(fs));
        let mut ok = 0;
        gl!(gl, get_programiv(prog, GL_LINK_STATUS, &mut ok));
        if ok == 0 {
            let mut buf = vec![0u8; 2048];
            let mut n = 0;
            gl!(gl, get_program_info_log(prog, buf.len() as i32, &mut n, buf.as_mut_ptr() as *mut _));
            buf.truncate(n.max(0) as usize);
            gl!(gl, delete_program(prog));
            return Err(io::Error::other(format!("program link: {}", String::from_utf8_lossy(&buf))));
        }
        gl!(gl, use_program(prog));
        let view = CString::new(shaders::UNIFORM_VIEW).unwrap_or_default();
        let tex = CString::new(shaders::UNIFORM_TEX).unwrap_or_default();
        let view_loc = gl!(gl, get_uniform_location(prog, view.as_ptr()));
        let tex_loc = gl!(gl, get_uniform_location(prog, tex.as_ptr()));
        gl!(gl, uniform1i(tex_loc, 0));
        Ok((prog, view_loc))
    }

    impl Gpu {
        /// Opens the GBM/EGL display, makes a context and allocates a
        /// `w` x `h` render target. Fails (never panics) when the GPU or
        /// a required extension is missing.
        pub fn new(w: usize, h: usize) -> io::Result<Gpu> {
            if w == 0 || h == 0 || w > texture::MAX_DIM || h > texture::MAX_DIM {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad render target size"));
            }
            let libs = open_libs()?;
            let egl = Egl::load(libs.egl)?;
            let gles = Gles::load(libs.gles)?;
            let d = desktop::open_display(&egl)?;
            // SAFETY: the display is initialised; the string is driver-owned.
            // SAFETY: eglQueryString returns null or a driver-owned NUL-terminated string.
            let exts = unsafe { cstr((egl.query_string)(d.dpy, EGL_EXTENSIONS)) };
            if !has_extension(&exts, EXT_DMA_BUF) {
                eprintln!("springboard: gpu: EGL_EXT_image_dma_buf_import missing, zero-copy scanout unavailable");
            }
            let ext = EglExt::load(&egl, &exts)?;
            let (ctx, surf) = desktop::open_context(&egl, d.dpy)?;
            eprintln!("springboard: gpu: desktop EGL on the {} display", d.kind);
            let (dpy, gbm, read_px) = (d.dpy, d.gbm, desktop::read_pixels_fn(libs.gles));
            if read_px.is_none() {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "glReadPixels missing"));
            }
            let target = Target::new_texture(&gles, w, h)?;
            let (program, view_loc) = link(&gles)?;
            let (mut vao, mut vbo) = (0u32, 0u32);
            gl!(gles, gen_vertex_arrays(1, &mut vao));
            gl!(gles, bind_vertex_array(vao));
            gl!(gles, gen_buffers(1, &mut vbo));
            gl!(gles, bind_buffer(GL_ARRAY_BUFFER, vbo));
            for (loc, size, off) in ATTRIBS {
                gl!(gles, enable_vertex_attrib_array(loc));
                gl!(gles, vertex_attrib_pointer(loc, size, GL_FLOAT, 0, VERTEX_BYTES as i32, off as *const c_void));
            }
            gl!(gles, pixel_storei(GL_UNPACK_ALIGNMENT, 4));
            let err = gl!(gles, get_error());
            if err != 0 {
                return Err(gl_error("GPU setup", err));
            }
            Ok(Gpu {
                egl,
                gl: gles,
                ext,
                dpy,
                ctx,
                surf,
                target: Some(target),
                program,
                view_loc,
                vao,
                vbo,
                batch: Batch::new(),
                textures: Textures::new(),
                w,
                h,
                swap_rb: false,
                scan: None,
                flip_fb: None,
                read_px,
                _gbm: gbm,
            })
        }

        pub fn size(&self) -> (usize, usize) {
            (self.w, self.h)
        }

        /// Whether frames end with a native fence (else `glFinish`).
        pub fn has_native_fence(&self) -> bool {
            self.ext.fence.is_some()
        }

        /// `present` copies as is by default (the target is already in `gfx` byte order);
        /// pass true to swap red and blue while copying.
        pub fn set_swap_rb(&mut self, swap: bool) {
            self.swap_rb = swap;
        }

        /// Binds the target and sets the state for this frame. The target keeps
        /// its pixels between frames: draw only what changed (see `clip`).
        pub fn begin_frame(&mut self) {
            let Some(t) = &self.target else { return };
            let gl = &self.gl;
            gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, t.fbo));
            gl!(gl, viewport(0, 0, self.w as i32, self.h as i32));
            gl!(gl, disable(GL_SCISSOR_TEST));
            gl!(gl, enable(GL_BLEND));
            gl!(gl, blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA));
            gl!(gl, use_program(self.program));
            gl!(gl, uniform2f(self.view_loc, self.w as f32, self.h as f32));
            gl!(gl, active_texture(GL_TEXTURE0));
            gl!(gl, bind_vertex_array(self.vao));
            self.batch.clear();
        }

        /// Restricts drawing to `rect` (None lifts the restriction); what was
        /// queued before is drawn first.
        pub fn clip(&mut self, rect: Option<Rect>) {
            self.flush();
            let gl = &self.gl;
            match rect.and_then(|r| damage::clip(r, self.w, self.h)) {
                Some(a) => {
                    gl!(gl, enable(GL_SCISSOR_TEST));
                    gl!(gl, scissor(a.x0 as i32, a.y0 as i32, (a.x1 - a.x0) as i32, (a.y1 - a.y0) as i32));
                }
                None if rect.is_some() => {
                    gl!(gl, enable(GL_SCISSOR_TEST));
                    gl!(gl, scissor(0, 0, 0, 0));
                }
                None => gl!(gl, disable(GL_SCISSOR_TEST)),
            }
        }

        /// Overwrites the clip area (or the whole target) with `color`.
        pub fn clear(&mut self, color: [f32; 4]) {
            self.flush();
            let gl = &self.gl;
            gl!(gl, clear_color(color[0] * color[3], color[1] * color[3], color[2] * color[3], color[3]));
            gl!(gl, clear(GL_COLOR_BUFFER_BIT));
        }

        pub fn fill_rrect(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
            self.room();
            self.batch.push_fill(rect, radius, color);
        }

        pub fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: [f32; 4]) {
            self.room();
            self.batch.push_shadow(rect, radius, blur, color);
        }

        /// Uploads `img` (premultiplied RGBA) as a linear-filtered texture.
        pub fn upload(&mut self, img: &Image) -> io::Result<TextureId> {
            texture::validate(img).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
            let gl = &self.gl;
            let mut tex = 0u32;
            gl!(gl, gen_textures(1, &mut tex));
            gl!(gl, bind_texture(GL_TEXTURE_2D, tex));
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR));
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR));
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE));
            gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE));
            // validate() checked px.len() == w * h; each u32 is the four bytes R, G, B, A
            gl!(
                gl,
                tex_image_2d(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as i32,
                    img.w as i32,
                    img.h as i32,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    img.px.as_ptr() as *const c_void
                )
            );
            let err = gl!(gl, get_error());
            if err != 0 {
                gl!(gl, delete_textures(1, &tex));
                return Err(gl_error("texture upload", err));
            }
            Ok(self.textures.insert(Entry { gl: tex, w: img.w, h: img.h }))
        }

        /// Size of an uploaded texture.
        pub fn texture_size(&self, t: TextureId) -> Option<(usize, usize)> {
            self.textures.get(t).map(|e| (e.w, e.h))
        }

        /// `src` is in image pixels, `dst` in target pixels (scaled to fit).
        pub fn draw_texture(&mut self, t: TextureId, src: Rect, dst: Rect, radius: f32, alpha: f32) {
            let Some(e) = self.textures.get(t) else { return };
            self.room();
            self.batch.push_texture(t, texture::uv(src, e.w, e.h), dst, radius, alpha);
        }

        /// Frees the texture now; unknown or already freed ids are ignored.
        pub fn free(&mut self, t: TextureId) {
            if self.batch.uses(t) {
                self.flush();
            }
            if let Some(e) = self.textures.remove(t) {
                gl!(self.gl, delete_textures(1, &e.gl));
            }
        }

        /// Draws what is queued, then returns the frame's completion fence.
        pub fn end_frame(&mut self) -> FrameFence {
            self.flush();
            let gl = &self.gl;
            if let Some(f) = self.ext.fence {
                // SAFETY: dpy is initialised and the context current; the sync is destroyed right after dup.
                let fd = unsafe {
                    let sync = (f.create_sync)(self.dpy, EGL_SYNC_NATIVE_FENCE_ANDROID, ptr::null());
                    if sync.is_null() {
                        EGL_NO_NATIVE_FENCE_FD_ANDROID
                    } else {
                        (gl.flush)();
                        let fd = (f.dup_fence_fd)(self.dpy, sync);
                        (f.destroy_sync)(self.dpy, sync);
                        fd
                    }
                };
                if fd >= 0 {
                    return FrameFence { fd };
                }
            }
            if let Some(k) = self.ext.khr {
                // SAFETY: dpy is initialised and the context current; the sync is destroyed right after the wait.
                let done = unsafe {
                    let sync = (k.create_sync)(self.dpy, EGL_SYNC_FENCE_KHR, ptr::null());
                    !sync.is_null() && {
                        let r = (k.client_wait)(self.dpy, sync, EGL_SYNC_FLUSH_COMMANDS_BIT_KHR, FENCE_TIMEOUT_MS as u64 * 1_000_000);
                        (k.destroy_sync)(self.dpy, sync);
                        r == EGL_CONDITION_SATISFIED_KHR
                    }
                };
                if done {
                    return FrameFence::signaled();
                }
            }
            gl!(gl, finish());
            FrameFence::signaled()
        }

        /// Which wait ends a frame: "native-fence", "khr-fence" or "glFinish".
        pub fn fence_kind(&self) -> &'static str {
            match (self.ext.fence.is_some(), self.ext.khr.is_some()) {
                (true, _) => "native-fence",
                (false, true) => "khr-fence",
                _ => "glFinish",
            }
        }

        /// Whether EGL can import a dma-buf as a render target in this format
        /// (the image is destroyed again). The context must be current.
        pub fn can_import(&self, w: usize, h: usize, fd: c_int, pitch: usize, fourcc: u32) -> Result<(), String> {
            let attrs = super::target::image_attribs_fourcc(w, h, fd, pitch, fourcc);
            // SAFETY: attrs is EGL_NONE terminated; the fd is open for the call.
            let image = unsafe { (self.ext.create_image)(self.dpy, ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, ptr::null_mut(), attrs.as_ptr()) };
            if image.is_null() {
                // SAFETY: eglGetError has no preconditions.
                return Err(egl_error("eglCreateImageKHR", unsafe { (self.egl.get_error)() }).to_string());
            }
            // SAFETY: image was just created on this display.
            unsafe { (self.ext.destroy_image)(self.dpy, image) };
            Ok(())
        }

        /// The framebuffer the last scanout-mode frame must be flipped to, once.
        pub fn take_flip(&mut self) -> Option<u32> {
            self.flip_fb.take()
        }

        pub fn scanout_active(&self) -> bool {
            self.scan.is_some()
        }

        /// Switches rendering to `bufs` (2 or more scanout buffers, 3 advised). The caller
        /// keeps no other reference to them; they come back from `disable_scanout`. Every
        /// target is blank, so the next frame must redraw in full. On error the copy path
        /// stays in force and the buffers are returned in the error's second field.
        pub fn enable_scanout(&mut self, bufs: Vec<ScanoutBuf>) -> Result<(), (io::Error, Vec<ScanoutBuf>)> {
            if self.scan.is_some() || bufs.len() < 2 || self.target.is_none() {
                return Err((io::Error::other("scanout already on or too few buffers"), bufs));
            }
            let mut targets = Vec::new();
            for b in &bufs {
                match Target::from_dmabuf(&self.gl, &self.egl, &self.ext, self.dpy, b.w, b.h, b.dmabuf_fd, b.pitch, b.fourcc) {
                    Ok(t) if t.w == self.w && t.h == self.h => targets.push(t),
                    Ok(t) => {
                        t.destroy(&self.gl, &self.ext, self.dpy);
                        for t in targets {
                            t.destroy(&self.gl, &self.ext, self.dpy);
                        }
                        return Err((io::Error::other("scanout buffer size differs from the render target"), bufs));
                    }
                    Err(e) => {
                        for t in targets {
                            t.destroy(&self.gl, &self.ext, self.dpy);
                        }
                        return Err((e, bufs));
                    }
                }
            }
            let mut owed = Damage::new(self.w, self.h);
            owed.all();
            let n = targets.len();
            let mut slots: Vec<Option<Target>> = targets.into_iter().map(Some).collect();
            let first = slots[0].take();
            let copy_target = std::mem::replace(&mut self.target, first);
            self.scan = Some(ScanRing { slots, bufs, cur: 0, owed: vec![owed; n], copy_target });
            Ok(())
        }

        /// Back to the copy path; returns the scanout buffers for the caller to
        /// release. The display must not be showing one of them.
        pub fn disable_scanout(&mut self) -> Vec<ScanoutBuf> {
            self.flip_fb = None;
            let Some(mut ring) = self.scan.take() else { return Vec::new() };
            if let Some(t) = std::mem::replace(&mut self.target, ring.copy_target.take()) {
                t.destroy(&self.gl, &self.ext, self.dpy);
            }
            for t in ring.slots.drain(..).flatten() {
                t.destroy(&self.gl, &self.ext, self.dpy);
            }
            ring.bufs
        }

        /// Scanout counterpart of `present`: waits for the fence, then makes the next
        /// target current, bringing it up to date with a GPU blit of the damage it
        /// still owes. Returns the framebuffer id to page-flip (the one just drawn),
        /// or None when nothing changed (or scanout is off).
        pub fn present_scanout(&mut self, fence: FrameFence, damage: &[Rect]) -> Option<u32> {
            fence.wait(FENCE_TIMEOUT_MS);
            let areas = damage::merge(damage, self.w, self.h);
            let ring = self.scan.as_mut()?;
            if areas.is_empty() {
                return None;
            }
            for d in &mut ring.owed {
                for a in &areas {
                    d.add(a.x0 as i32, a.y0 as i32, (a.x1 - a.x0) as i32, (a.y1 - a.y0) as i32);
                }
            }
            let cur = ring.cur;
            ring.owed[cur].clear();
            let fb = ring.bufs[cur].fb;
            let next = (cur + 1) % ring.slots.len();
            let (Some(src), Some(dst)) = (self.target.as_ref(), ring.slots[next].as_ref()) else { return Some(fb) };
            let gl = &self.gl;
            gl!(gl, disable(GL_SCISSOR_TEST));
            gl!(gl, bind_framebuffer(GL_READ_FRAMEBUFFER, src.fbo));
            gl!(gl, bind_framebuffer(GL_DRAW_FRAMEBUFFER, dst.fbo));
            for a in ring.owed[next].areas() {
                let (x0, y0, x1, y1) = (a.x0 as i32, a.y0 as i32, a.x1 as i32, a.y1 as i32);
                gl!(gl, blit_framebuffer(x0, y0, x1, y1, x0, y0, x1, y1, GL_COLOR_BUFFER_BIT, GL_NEAREST));
            }
            gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, dst.fbo));
            ring.owed[next].clear();
            let nxt = ring.slots[next].take();
            let old = std::mem::replace(&mut self.target, nxt);
            ring.slots[cur] = old;
            ring.cur = next;
            Some(fb)
        }

        /// Waits for the fence, then copies the damaged rows of the target into
        /// `dst` (`dst_stride_px` pixels per row). Returns the pixels copied.
        ///
        /// While scanout is on nothing is copied: the frame stays in its scanout target and
        /// the framebuffer to flip is left for `take_flip` (so every caller works unchanged).
        pub fn present(&mut self, fence: FrameFence, dst: &mut [u32], dst_stride_px: usize, damage: &[Rect]) -> usize {
            if self.scan.is_some() {
                self.flip_fb = self.present_scanout(fence, damage);
                return 0;
            }
            fence.wait(FENCE_TIMEOUT_MS);
            let areas = damage::merge(damage, self.w, self.h);
            let Some(t) = &self.target else { return 0 };
            // render-to-texture target: read the damage back
            if areas.is_empty() { 0 } else { self.read_back(t.fbo, &areas, dst, dst_stride_px) }
        }

        /// Desktop-only copy path without a dma-buf: `glReadPixels` straight into `dst` (row 0 of the
        /// FBO is row 0 of the frame, as in the dma-buf targets). Returns the pixels copied.
        fn read_back(&self, fbo: u32, areas: &[crate::gfx::Area], dst: &mut [u32], dst_stride_px: usize) -> usize {
            let Some(read) = self.read_px else { return 0 };
            let gl = &self.gl;
            gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
            gl!(gl, pixel_storei(desktop::GL_PACK_ALIGNMENT, 4));
            gl!(gl, pixel_storei(desktop::GL_PACK_ROW_LENGTH, dst_stride_px as i32));
            let mut n = 0;
            for a in areas {
                if a.x1 <= a.x0 || a.y1 <= a.y0 || a.x1 > dst_stride_px || (a.y1 - 1) * dst_stride_px + a.x1 > dst.len() {
                    continue;
                }
                let (w, h) = (a.x1 - a.x0, a.y1 - a.y0);
                // SAFETY: the bounds check above keeps the write inside dst; with PACK_ROW_LENGTH set to
                // the destination stride, GL writes h rows of w pixels starting at (x0, y0) of dst.
                unsafe {
                    read(a.x0 as i32, a.y0 as i32, w as i32, h as i32, GL_RGBA, GL_UNSIGNED_BYTE, dst[a.y0 * dst_stride_px + a.x0..].as_mut_ptr() as *mut c_void);
                }
                if self.swap_rb {
                    for y in a.y0..a.y1 {
                        for p in &mut dst[y * dst_stride_px + a.x0..][..w] {
                            *p = damage::swap_rb(*p);
                        }
                    }
                }
                n += w * h;
            }
            gl!(gl, pixel_storei(desktop::GL_PACK_ROW_LENGTH, 0));
            n
        }

        fn room(&mut self) {
            if self.batch.is_full() {
                self.flush();
            }
        }

        fn flush(&mut self) {
            if self.batch.is_empty() {
                return;
            }
            let gl = &self.gl;
            let v = self.batch.vertices();
            gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
            gl!(gl, buffer_data(GL_ARRAY_BUFFER, (v.len() * VERTEX_BYTES) as isize, v.as_ptr() as *const c_void, GL_STREAM_DRAW));
            for run in self.batch.runs() {
                if let Some(t) = run.tex {
                    match self.textures.get(t) {
                        Some(e) => gl!(gl, bind_texture(GL_TEXTURE_2D, e.gl)),
                        None => continue,
                    }
                }
                gl!(gl, draw_arrays(GL_TRIANGLES, run.start as i32, run.count as i32));
            }
            self.batch.clear();
        }
    }

    impl Drop for Gpu {
        fn drop(&mut self) {
            let gl = &self.gl;
            for e in self.textures.drain() {
                gl!(gl, delete_textures(1, &e.gl));
            }
            if self.scan.is_some() {
                drop(self.disable_scanout());
            }
            let gl = &self.gl;
            if let Some(t) = self.target.take() {
                t.destroy(&gl, &self.ext, self.dpy);
            }
            gl!(gl, delete_buffers(1, &self.vbo));
            gl!(gl, delete_vertex_arrays(1, &self.vao));
            gl!(gl, delete_program(self.program));
            // SAFETY: handles were created in `new`; releasing the context first, then its objects.
            unsafe {
                (self.egl.make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                if !self.surf.is_null() {
                    (self.egl.destroy_surface)(self.dpy, self.surf);
                }
                (self.egl.destroy_context)(self.dpy, self.ctx);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_signaled_fence_waits_for_nothing() {
            let f = FrameFence::signaled();
            assert!(f.is_signaled());
            assert!(f.wait(0));
        }
    }
}
