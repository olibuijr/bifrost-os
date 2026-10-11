//! The layer compositor: draws a frame into the GPU's render target from up to two layers,
//! turned onto the portrait panel by the vertex positions (`quad`):
//!
//! * a video picture: the decoder's NV12 dma-buf imported as an external EGL image (cached per
//!   decoder buffer, which the decoder reuses from a small pool), or, when the driver refuses
//!   the import, the picture converted on the CPU and uploaded;
//! * the CPU frame: uploaded where it changed since the last upload (`overlay::diff_update`)
//!   and drawn opaque (GPU rotation) or blended over the picture (premultiplied alpha, the
//!   app's transparent pixels show the video).
//!
//! Every call sets the GL state it uses and puts back what `Gpu::begin_frame` relies on, so the
//! zoom compositor and this one can share the context. Only the main loop's thread calls in
//! (`Gpu` is !Send); decoder threads only hand over `DmaPicture`s.

use super::egl::*;
use super::error::{egl_error, gl_error};
use super::plan::{self, Import, Source};
use super::quad::{self, LayerVertex};
use super::{nv12, overlay, shaders, Gpu};
use crate::bare::sys;
use crate::gfx::rotate::Rotation;
use crate::gfx::Area;
use crate::layout::Rect;
use crate::media::{convert, DmaBuf, DmaPicture};
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::io;
use std::ptr;
use std::sync::{Arc, Weak};

/// Calls a GL function from the table.
// SAFETY (all uses): the Gpu's context is current on this thread (Gpu is !Send and made it
// current when created); each call passes pointers to live data of the size the call reads.
macro_rules! gl {
    ($t:expr, $f:ident($($a:expr),* $(,)?)) => {
        unsafe { ($t.$f)($($a),*) }
    };
}

/// Imported decoder buffers kept at most (a pool is ~10-25 buffers).
const MAX_IMAGES: usize = 40;
/// Threads for the CPU conversion of the upload fallback.
const CONVERT_THREADS: usize = 3;

/// A video picture and where it goes, in logical-frame pixels.
pub struct VideoInput<'a> {
    pub picture: &'a DmaPicture,
    pub dst: Rect,
}

/// What a composed frame did, for the presenter and the frame trace.
#[derive(Debug, Default)]
pub struct Composed {
    /// panel rectangles now holding new pixels in the presenter's buffer (empty: no change)
    pub damage: Vec<Rect>,
    /// diffing and uploading the CPU frame (and converting the picture in the fallback)
    pub upload_ms: f64,
    /// issuing the draws
    pub gpu_ms: f64,
    /// waiting for the GPU and copying the target out
    pub copy_ms: f64,
    pub uploaded_px: usize,
    /// how the picture was sampled (None without a video layer or when it did not change)
    pub source: Option<Source>,
}

struct Program {
    id: u32,
    /// the `u_alpha` uniform (-1 where the program has none)
    alpha: i32,
}

/// The rotation turn's two pictures on the GPU, uploaded once per turn.
struct TurnTextures {
    id: u64,
    old: u32,
    new: u32,
    old_size: (usize, usize),
    new_size: (usize, usize),
}

struct Imported {
    buf: Weak<dyn DmaBuf>,
    image: P,
    tex: u32,
    used: u64,
}

pub struct Layers {
    plain: Program,
    external: Option<Program>,
    vao: u32,
    vbo: u32,
    frame_tex: u32,
    frame_size: (usize, usize),
    /// the CPU frame as last uploaded
    prev: Vec<u32>,
    /// the upload fallback's picture texture and its size
    video_tex: u32,
    video_size: (usize, usize),
    images: HashMap<u64, Imported>,
    import: Import,
    /// (buffer id, picture seq, destination) of the picture drawn last
    last_video: Option<(u64, u64, Rect)>,
    /// (rotation, frame size) the target was last fully drawn for
    primed: Option<(Rotation, usize, usize)>,
    frame_no: u64,
    /// the cross-fade program of the rotation turn, built on its first use
    fade: Option<Program>,
    turn: Option<TurnTextures>,
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
        return Err(io::Error::other(format!("layer shader compile: {}", String::from_utf8_lossy(&buf))));
    }
    Ok(sh)
}

/// Builds a layer program and points its uniforms at texture unit 0 and the target size.
fn program(gl: &Gles, external: bool, w: usize, h: usize) -> io::Result<Program> {
    build(gl, &shaders::layer_fragment_source(external), w, h)
}

/// The layer vertex stage with the fragment stage `fragment`.
fn build(gl: &Gles, fragment: &str, w: usize, h: usize) -> io::Result<Program> {
    let vs = compile(gl, GL_VERTEX_SHADER, &shaders::layer_vertex_source())?;
    let fs = match compile(gl, GL_FRAGMENT_SHADER, fragment) {
        Ok(fs) => fs,
        Err(e) => {
            gl!(gl, delete_shader(vs));
            return Err(e);
        }
    };
    let id = gl!(gl, create_program());
    gl!(gl, attach_shader(id, vs));
    gl!(gl, attach_shader(id, fs));
    gl!(gl, link_program(id));
    gl!(gl, delete_shader(vs));
    gl!(gl, delete_shader(fs));
    let mut ok = 0;
    gl!(gl, get_programiv(id, GL_LINK_STATUS, &mut ok));
    if ok == 0 {
        let mut buf = vec![0u8; 2048];
        let mut n = 0;
        gl!(gl, get_program_info_log(id, buf.len() as i32, &mut n, buf.as_mut_ptr() as *mut _));
        buf.truncate(n.max(0) as usize);
        gl!(gl, delete_program(id));
        return Err(io::Error::other(format!("layer program link: {}", String::from_utf8_lossy(&buf))));
    }
    gl!(gl, use_program(id));
    let view = CString::new(shaders::UNIFORM_VIEW).unwrap_or_default();
    let tex = CString::new(shaders::UNIFORM_TEX).unwrap_or_default();
    let view_loc = gl!(gl, get_uniform_location(id, view.as_ptr()));
    let tex_loc = gl!(gl, get_uniform_location(id, tex.as_ptr()));
    gl!(gl, uniform2f(view_loc, w as f32, h as f32));
    gl!(gl, uniform1i(tex_loc, 0));
    let alpha = CString::new(shaders::UNIFORM_ALPHA).unwrap_or_default();
    let alpha = gl!(gl, get_uniform_location(id, alpha.as_ptr()));
    gl!(gl, use_program(0));
    Ok(Program { id, alpha })
}

/// A fresh RGBA texture of `w` x `h` (contents undefined) with the given filter.
fn rgba_texture(gl: &Gles, w: usize, h: usize, filter: i32) -> u32 {
    let mut tex = 0u32;
    gl!(gl, gen_textures(1, &mut tex));
    gl!(gl, bind_texture(GL_TEXTURE_2D, tex));
    gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter));
    gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter));
    gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE));
    gl!(gl, tex_parameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE));
    gl!(gl, tex_image_2d(GL_TEXTURE_2D, 0, GL_RGBA as i32, w as i32, h as i32, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr::null()));
    tex
}

/// Uploads `rects` of the `w`-pixels-per-row image `px` into the bound 2D texture.
fn upload_rects(gl: &Gles, px: &[u32], w: usize, rects: &[Rect]) -> usize {
    gl!(gl, pixel_storei(GL_UNPACK_ROW_LENGTH, w as i32));
    let mut n = 0;
    for r in rects {
        if r.w <= 0 || r.h <= 0 || r.x < 0 || r.y < 0 {
            continue;
        }
        let start = r.y as usize * w + r.x as usize;
        // the last row read ends at start + (h - 1) * w + r.w
        if start + (r.h as usize - 1) * w + r.w as usize > px.len() {
            continue;
        }
        // RGBA bytes in memory order: each u32 is R, G, B, A from the low byte up
        gl!(gl, tex_sub_image_2d(GL_TEXTURE_2D, 0, r.x, r.y, r.w, r.h, GL_RGBA, GL_UNSIGNED_BYTE, px[start..].as_ptr() as *const c_void));
        n += (r.w * r.h) as usize;
    }
    gl!(gl, pixel_storei(GL_UNPACK_ROW_LENGTH, 0));
    n
}

impl Layers {
    /// Builds the programs and buffers on `gpu`'s context. Without the external-image
    /// program (no `GL_OES_EGL_image_external_essl3`) pictures take the upload path.
    pub fn new(gpu: &Gpu) -> io::Result<Layers> {
        let gl = &gpu.gl;
        let (w, h) = gpu.size();
        let plain = program(gl, false, w, h)?;
        let external = match program(gl, true, w, h) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("springboard: gpu video: no external sampler, pictures are uploaded: {e}");
                None
            }
        };
        let (mut vao, mut vbo) = (0u32, 0u32);
        gl!(gl, gen_vertex_arrays(1, &mut vao));
        gl!(gl, bind_vertex_array(vao));
        gl!(gl, gen_buffers(1, &mut vbo));
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, vbo));
        let stride = std::mem::size_of::<LayerVertex>() as i32;
        gl!(gl, enable_vertex_attrib_array(shaders::LAYER_LOC_POS));
        gl!(gl, vertex_attrib_pointer(shaders::LAYER_LOC_POS, 2, GL_FLOAT, 0, stride, ptr::null()));
        gl!(gl, enable_vertex_attrib_array(shaders::LAYER_LOC_UV));
        gl!(gl, vertex_attrib_pointer(shaders::LAYER_LOC_UV, 2, GL_FLOAT, 0, stride, (2 * std::mem::size_of::<f32>()) as *const c_void));
        gl!(gl, bind_vertex_array(0));
        let err = gl!(gl, get_error());
        let layers = Layers {
            plain,
            external,
            vao,
            vbo,
            frame_tex: 0,
            frame_size: (0, 0),
            prev: Vec::new(),
            video_tex: 0,
            video_size: (0, 0),
            images: HashMap::new(),
            import: Import::Untried,
            last_video: None,
            primed: None,
            frame_no: 0,
            fade: None,
            turn: None,
        };
        if err != 0 {
            layers.destroy(gpu);
            return Err(gl_error("layer setup", err));
        }
        Ok(layers)
    }

    /// Whether NV12 pictures are sampled straight from the decoder's buffers.
    pub fn source(&self) -> Source {
        plan::source(self.import, self.external.is_some())
    }

    /// The next `compose` redraws everything (another path drew into the target meanwhile).
    pub fn invalidate(&mut self) {
        self.primed = None;
    }

    /// Frees the imports of decoder buffers nobody holds any more, and (`all`) every import
    /// and the upload fallback's texture: the video layer has gone.
    pub fn trim(&mut self, gpu: &Gpu, all: bool) {
        let gl = &gpu.gl;
        let dead: Vec<u64> = self.images.iter().filter(|(_, i)| all || i.buf.strong_count() == 0).map(|(k, _)| *k).collect();
        for k in dead {
            if let Some(i) = self.images.remove(&k) {
                Self::free_image(gpu, i);
            }
        }
        if all && self.video_tex != 0 {
            gl!(gl, delete_textures(1, &self.video_tex));
            self.video_tex = 0;
            self.video_size = (0, 0);
        }
        if all {
            self.last_video = None;
        }
    }

    fn free_image(gpu: &Gpu, i: Imported) {
        gl!(gpu.gl, delete_textures(1, &i.tex));
        // SAFETY: the image was created on this display and is no longer bound to a texture.
        unsafe { (gpu.ext.destroy_image)(gpu.dpy, i.image) };
    }

    /// Releases every GL object (the context must still be current).
    pub fn destroy(mut self, gpu: &Gpu) {
        self.trim(gpu, true);
        let gl = &gpu.gl;
        if self.frame_tex != 0 {
            gl!(gl, delete_textures(1, &self.frame_tex));
        }
        gl!(gl, delete_buffers(1, &self.vbo));
        gl!(gl, delete_vertex_arrays(1, &self.vao));
        gl!(gl, delete_program(self.plain.id));
        if let Some(p) = &self.external {
            gl!(gl, delete_program(p.id));
        }
        if let Some(p) = &self.fade {
            gl!(gl, delete_program(p.id));
        }
        self.end_turn(gpu);
    }

    /// Frees the turn's textures (the animation is over, or failed).
    pub fn end_turn(&mut self, gpu: &Gpu) {
        if let Some(t) = self.turn.take() {
            gl!(gpu.gl, delete_textures(1, &t.old));
            gl!(gpu.gl, delete_textures(1, &t.new));
        }
        self.primed = None;
    }

    /// Whether the turn's textures are uploaded.
    pub fn turn_live(&self) -> bool {
        self.turn.is_some()
    }

    /// One frame of the rotation turn on the GPU: `old` and `new` (uploaded when `id` is new)
    /// drawn as one turned and scaled rectangle each at progress `k`, cross-faded, black
    /// around; the whole panel lands in `out`. `rot` is how the new frame is held on the panel.
    /// InvalidInput means the frames do not fit the target (draw it on the CPU).
    #[allow(clippy::too_many_arguments)]
    pub fn turn(&mut self, gpu: &mut Gpu, id: u64, old: &crate::gfx::Image, new: &crate::gfx::Image, from: Rotation, k: f32, rot: Rotation, out: &mut [u32]) -> io::Result<Composed> {
        let (pw, ph) = gpu.size();
        let (fw, fh) = (new.w, new.h);
        if rot.size(fw, fh) != (pw, ph) || out.len() < pw * ph || old.px.len() < old.w * old.h || new.px.len() < fw * fh || old.w == 0 || old.h == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "turn frames do not match the render target"));
        }
        let t0 = sys::now_ms();
        let mut res = Composed::default();
        if self.fade.is_none() {
            self.fade = Some(build(&gpu.gl, &shaders::fade_fragment_source(), pw, ph)?);
        }
        if self.turn.as_ref().map_or(true, |t| t.id != id) {
            self.end_turn(gpu);
            let gl = &gpu.gl;
            let (a, b) = (rgba_texture(gl, old.w, old.h, GL_LINEAR), rgba_texture(gl, fw, fh, GL_LINEAR));
            gl!(gl, bind_texture(GL_TEXTURE_2D, a));
            res.uploaded_px += upload_rects(gl, &old.px, old.w, &[Rect { x: 0, y: 0, w: old.w as i32, h: old.h as i32 }]);
            gl!(gl, bind_texture(GL_TEXTURE_2D, b));
            res.uploaded_px += upload_rects(gl, &new.px, fw, &[Rect { x: 0, y: 0, w: fw as i32, h: fh as i32 }]);
            let err = gl!(gl, get_error());
            if err != 0 {
                gl!(gl, delete_textures(1, &a));
                gl!(gl, delete_textures(1, &b));
                return Err(gl_error("turn upload", err));
            }
            self.turn = Some(TurnTextures { id, old: a, new: b, old_size: (old.w, old.h), new_size: (fw, fh) });
        }
        let (Some(t), Some(fade)) = (self.turn.as_ref(), self.fade.as_ref()) else { return Err(io::Error::other("no turn textures")) };
        let k = k.clamp(0.0, 1.0);
        let theta = from.angle() * (1.0 - k);
        let verts = quad::turn_quad(rot, theta, quad::turn_size(t.old_size, t.new_size, k), fw, fh);
        let fbo = gpu.target.as_ref().map(|t| t.fbo).ok_or_else(|| io::Error::other("no render target"))?;
        let t1 = sys::now_ms();
        res.upload_ms = t1 - t0;
        let gl = &gpu.gl;
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
        gl!(gl, viewport(0, 0, pw as i32, ph as i32));
        gl!(gl, disable(GL_SCISSOR_TEST));
        gl!(gl, disable(GL_BLEND));
        gl!(gl, clear_color(0.0, 0.0, 0.0, 1.0));
        gl!(gl, clear(GL_COLOR_BUFFER_BIT));
        gl!(gl, bind_vertex_array(self.vao));
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
        gl!(gl, buffer_data(GL_ARRAY_BUFFER, std::mem::size_of_val(&verts) as isize, verts.as_ptr() as *const c_void, GL_STREAM_DRAW));
        gl!(gl, active_texture(GL_TEXTURE0));
        gl!(gl, use_program(fade.id));
        if k < 1.0 {
            gl!(gl, uniform1f(fade.alpha, 1.0 - k));
            gl!(gl, bind_texture(GL_TEXTURE_2D, t.old));
            gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
        }
        if k > 0.0 {
            // premultiplied sum: old * (1 - k) + new * k, as the CPU path mixes them
            gl!(gl, enable(GL_BLEND));
            gl!(gl, blend_func(GL_ONE, GL_ONE));
            gl!(gl, uniform1f(fade.alpha, k));
            gl!(gl, bind_texture(GL_TEXTURE_2D, t.new));
            gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
        }
        // what Gpu::begin_frame and the layer compositor expect
        gl!(gl, enable(GL_BLEND));
        gl!(gl, blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA));
        gl!(gl, bind_texture(GL_TEXTURE_2D, 0));
        gl!(gl, bind_vertex_array(0));
        gl!(gl, use_program(0));
        // the target now holds the turn: the next layer frame redraws in full
        self.primed = None;
        let err = gl!(gl, get_error());
        if err != 0 {
            return Err(gl_error("turn draw", err));
        }
        let fence = gpu.end_frame();
        let t2 = sys::now_ms();
        res.gpu_ms = t2 - t1;
        let whole = Rect { x: 0, y: 0, w: pw as i32, h: ph as i32 };
        gpu.present(fence, out, pw, &[whole]);
        res.copy_ms = sys::now_ms() - t2;
        res.damage = vec![whole];
        Ok(res)
    }

    /// The picture as an external texture, importing its buffer on first sight; None when the
    /// driver refuses NV12 (then never asked again).
    fn external_texture(&mut self, gpu: &Gpu, pic: &DmaPicture) -> Option<u32> {
        let frame_no = self.frame_no;
        if let Some(i) = self.images.get_mut(&pic.id) {
            if std::ptr::addr_eq(i.buf.as_ptr(), Arc::as_ptr(&pic.buf)) {
                i.used = frame_no;
                return Some(i.tex);
            }
            // a recycled id would be a bug; drop the stale import rather than sample it
            if let Some(stale) = self.images.remove(&pic.id) {
                Self::free_image(gpu, stale);
            }
        }
        let fd = pic.buf.fd();
        let mut image: P = ptr::null_mut();
        let mut code = 0;
        for siting in [true, false] {
            let Some(attrs) = nv12::image_attribs(pic, fd, siting) else { break };
            // SAFETY: attrs is EGL_NONE terminated; the dma-buf fd stays open while `pic.buf` lives
            // and the driver takes its own reference to the buffer.
            image = unsafe { (gpu.ext.create_image)(gpu.dpy, ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, ptr::null_mut(), attrs.as_ptr()) };
            if !image.is_null() {
                break;
            }
            // SAFETY: eglGetError has no preconditions.
            code = unsafe { (gpu.egl.get_error)() };
        }
        if image.is_null() {
            eprintln!("springboard: gpu video: NV12 import refused ({}), uploading pictures instead", egl_error("eglCreateImageKHR", code));
            self.import = Import::Refused;
            return None;
        }
        let gl = &gpu.gl;
        let mut tex = 0u32;
        gl!(gl, gen_textures(1, &mut tex));
        gl!(gl, bind_texture(GL_TEXTURE_EXTERNAL_OES, tex));
        gl!(gl, tex_parameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR));
        gl!(gl, tex_parameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR));
        gl!(gl, tex_parameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE));
        gl!(gl, tex_parameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE));
        // SAFETY: the external texture is bound on the current context and the image is live.
        unsafe { (gpu.ext.image_target_texture)(GL_TEXTURE_EXTERNAL_OES, image) };
        let err = gl!(gl, get_error());
        gl!(gl, bind_texture(GL_TEXTURE_EXTERNAL_OES, 0));
        if err != 0 {
            gl!(gl, delete_textures(1, &tex));
            // SAFETY: created above on this display, bound to nothing now.
            unsafe { (gpu.ext.destroy_image)(gpu.dpy, image) };
            eprintln!("springboard: gpu video: NV12 image as texture failed ({}), uploading pictures instead", gl_error("glEGLImageTargetTexture2DOES", err));
            self.import = Import::Refused;
            return None;
        }
        if self.import == Import::Untried {
            eprintln!("springboard: gpu video: NV12 pictures sampled in place ({}x{} stride {})", pic.width, pic.height, pic.stride);
        }
        self.import = Import::Works;
        if self.images.len() >= MAX_IMAGES {
            if let Some(k) = self.images.iter().min_by_key(|(_, i)| i.used).map(|(k, _)| *k) {
                if let Some(old) = self.images.remove(&k) {
                    Self::free_image(gpu, old);
                }
            }
        }
        self.images.insert(pic.id, Imported { buf: Arc::downgrade(&pic.buf), image, tex, used: frame_no });
        Some(tex)
    }

    /// The upload fallback: the picture converted at its own size and put in a 2D texture
    /// (only when it changed).
    fn uploaded_texture(&mut self, gpu: &Gpu, pic: &DmaPicture, changed: bool) -> u32 {
        let gl = &gpu.gl;
        let size = (pic.width as usize, pic.height as usize);
        if self.video_tex == 0 || self.video_size != size {
            if self.video_tex != 0 {
                gl!(gl, delete_textures(1, &self.video_tex));
            }
            self.video_tex = rgba_texture(gl, size.0, size.1, GL_LINEAR);
            self.video_size = size;
        } else if !changed {
            return self.video_tex;
        }
        let img = convert::dma_to_rgba(pic, size.0, size.1, convert::matrix_for(pic.height), CONVERT_THREADS);
        gl!(gl, bind_texture(GL_TEXTURE_2D, self.video_tex));
        upload_rects(gl, &img.px, img.w, &[Rect { x: 0, y: 0, w: img.w as i32, h: img.h as i32 }]);
        self.video_tex
    }

    /// Composes one frame and copies what changed into `out` (the panel-sized presenter
    /// buffer). `frame` is the shell's CPU frame (`fw` x `fh`, logical); `areas` is what the
    /// shell redrew in it. `video` puts the picture under the frame (which is then an overlay,
    /// transparent where the picture shows); without it the frame is drawn opaque. `full`
    /// redraws everything (the target holds another path's pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn compose(&mut self, gpu: &mut Gpu, video: Option<VideoInput>, frame: &[u32], fw: usize, fh: usize, areas: &[Area], rot: Rotation, full: bool, out: &mut [u32]) -> io::Result<Composed> {
        let (pw, ph) = gpu.size();
        if rot.size(fw, fh) != (pw, ph) || frame.len() < fw * fh || out.len() < pw * ph {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame does not match the render target"));
        }
        self.frame_no += 1;
        let mut full = full || self.primed != Some((rot, fw, fh));
        let mut res = Composed::default();
        let t0 = sys::now_ms();

        // the CPU frame: diff against the last upload, upload the changed bands
        if self.frame_tex == 0 || self.frame_size != (fw, fh) {
            if self.frame_tex != 0 {
                gl!(gpu.gl, delete_textures(1, &self.frame_tex));
            }
            self.frame_tex = rgba_texture(&gpu.gl, fw, fh, GL_NEAREST);
            self.frame_size = (fw, fh);
            full = true;
        }
        let whole = Rect { x: 0, y: 0, w: fw as i32, h: fh as i32 };
        let rects = if full {
            overlay::sync_all(&mut self.prev, &frame[..fw * fh]);
            vec![whole]
        } else {
            overlay::diff_update(&mut self.prev, frame, fw, fh, areas)
        };
        if !rects.is_empty() {
            gl!(gpu.gl, bind_texture(GL_TEXTURE_2D, self.frame_tex));
            res.uploaded_px = upload_rects(&gpu.gl, frame, fw, &rects);
        }

        // the picture
        let mut picture: Option<(u32, u32, [f32; 4], Rect)> = None;
        let mut video_changed = false;
        match &video {
            Some(v) => {
                let key = (v.picture.id, v.picture.seq, v.dst);
                video_changed = full || self.last_video != Some(key);
                let mut drawn = None;
                if self.source() == Source::External {
                    if let (Some(tex), Some(uv)) = (self.external_texture(gpu, v.picture), nv12::crop_uv(v.picture)) {
                        drawn = Some((GL_TEXTURE_EXTERNAL_OES, tex, uv, Source::External));
                    }
                }
                if drawn.is_none() {
                    // first fallback frame: the upload texture is new, so draw it in full
                    video_changed |= self.video_tex == 0;
                    let tex = self.uploaded_texture(gpu, v.picture, video_changed);
                    drawn = Some((GL_TEXTURE_2D, tex, [0.0, 0.0, 1.0, 1.0], Source::Upload));
                }
                if let Some((target, tex, uv, src)) = drawn {
                    picture = Some((target, tex, uv, v.dst));
                    if video_changed {
                        res.source = Some(src);
                    }
                }
                self.last_video = Some(key);
            }
            None => {
                if self.last_video.is_some() || !self.images.is_empty() {
                    self.trim(gpu, true);
                }
            }
        }
        if self.frame_no % 64 == 0 {
            self.trim(gpu, false);
        }
        let t1 = sys::now_ms();
        res.upload_ms = t1 - t0;

        let logical = plan::logical_damage(full, &rects, picture.map(|p| p.3), video_changed, fw, fh);
        let panel = plan::panel_rects(rot, &logical, fw, fh);
        self.primed = Some((rot, fw, fh));
        if panel.is_empty() {
            return Ok(res);
        }

        let fbo = gpu.target.as_ref().map(|t| t.fbo).ok_or_else(|| io::Error::other("no render target"))?;
        let mut verts: Vec<LayerVertex> = Vec::with_capacity(12);
        if let Some((_, _, uv, dst)) = picture {
            verts.extend(quad::layer_quad(rot, dst, uv, fw, fh));
        }
        let frame_first = verts.len() as i32;
        verts.extend(quad::layer_quad(rot, whole, [0.0, 0.0, 1.0, 1.0], fw, fh));
        let ext_prog = self.external.as_ref().map_or(0, |p| p.id);
        let gl = &gpu.gl;
        gl!(gl, bind_framebuffer(GL_FRAMEBUFFER, fbo));
        gl!(gl, viewport(0, 0, pw as i32, ph as i32));
        gl!(gl, bind_vertex_array(self.vao));
        gl!(gl, bind_buffer(GL_ARRAY_BUFFER, self.vbo));
        gl!(gl, buffer_data(GL_ARRAY_BUFFER, std::mem::size_of_val(verts.as_slice()) as isize, verts.as_ptr() as *const c_void, GL_STREAM_DRAW));
        gl!(gl, active_texture(GL_TEXTURE0));
        gl!(gl, enable(GL_SCISSOR_TEST));
        gl!(gl, blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA));
        for r in &panel {
            gl!(gl, scissor(r.x, r.y, r.w, r.h));
            gl!(gl, disable(GL_BLEND));
            if let Some((target, tex, _, _)) = picture {
                // letterbox bars are black; the picture is opaque
                gl!(gl, clear_color(0.0, 0.0, 0.0, 1.0));
                gl!(gl, clear(GL_COLOR_BUFFER_BIT));
                gl!(gl, use_program(if target == GL_TEXTURE_EXTERNAL_OES { ext_prog } else { self.plain.id }));
                gl!(gl, bind_texture(target, tex));
                gl!(gl, draw_arrays(GL_TRIANGLES, 0, 6));
                gl!(gl, bind_texture(target, 0));
                // the frame over it: premultiplied, transparent where the picture shows
                gl!(gl, enable(GL_BLEND));
            }
            gl!(gl, use_program(self.plain.id));
            gl!(gl, bind_texture(GL_TEXTURE_2D, self.frame_tex));
            gl!(gl, draw_arrays(GL_TRIANGLES, frame_first, 6));
        }
        // what Gpu::begin_frame and the zoom compositor expect (they set the rest themselves)
        gl!(gl, disable(GL_SCISSOR_TEST));
        gl!(gl, enable(GL_BLEND));
        gl!(gl, bind_texture(GL_TEXTURE_2D, 0));
        gl!(gl, bind_vertex_array(0));
        gl!(gl, use_program(0));
        let err = gl!(gl, get_error());
        if err != 0 {
            self.primed = None;
            return Err(gl_error("layer draw", err));
        }
        let fence = gpu.end_frame();
        let t2 = sys::now_ms();
        res.gpu_ms = t2 - t1;
        gpu.present(fence, out, pw, &panel);
        res.copy_ms = sys::now_ms() - t2;
        res.damage = panel;
        Ok(res)
    }
}
