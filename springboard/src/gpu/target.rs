//! The render target: a texture on an FBO, either plain RGBA8 (read back with `glReadPixels`)
//! or a scanout dma-buf imported as an EGL image.

use super::egl::*;
use std::io;
use std::ptr;

/// DRM_FORMAT_XBGR8888 ('XB24'): red in the low byte, the same byte order as the shell's
/// `frame`, so presenting needs no channel swap.
pub const FOURCC_XB24: u32 = 0x3432_4258;

/// Attribute list for `eglCreateImageKHR(EGL_LINUX_DMA_BUF_EXT)`.
pub fn image_attribs(w: usize, h: usize, fd: i32, pitch: usize) -> [i32; 13] {
    image_attribs_fourcc(w, h, fd, pitch, FOURCC_XB24)
}

/// `image_attribs` for another format (scanout targets may need XR24).
pub fn image_attribs_fourcc(w: usize, h: usize, fd: i32, pitch: usize, fourcc: u32) -> [i32; 13] {
    [
        EGL_WIDTH,
        w as i32,
        EGL_HEIGHT,
        h as i32,
        EGL_LINUX_DRM_FOURCC_EXT,
        fourcc as i32,
        EGL_DMA_BUF_PLANE0_FD_EXT,
        fd,
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        0,
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        pitch as i32,
        EGL_NONE,
    ]
}

pub struct Target {
    pub image: P,
    pub tex: u32,
    pub fbo: u32,
    pub w: usize,
    pub h: usize,
    pub pitch_px: usize,
}

impl Target {
    /// Desktop render-to-texture target: a plain RGBA8 texture on an FBO, no dma-buf and no
    /// CPU mapping (`present` reads the damage back with `glReadPixels`). The context must be current.
    pub fn new_texture(gl: &Gles, w: usize, h: usize) -> io::Result<Target> {
        let (mut tex, mut fbo) = (0u32, 0u32);
        // SAFETY: the context is current (caller contract); all pointers are to live locals or null (no initial data).
        let status = unsafe {
            (gl.gen_textures)(1, &mut tex);
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.tex_image_2d)(GL_TEXTURE_2D, 0, GL_RGBA as i32, w as i32, h as i32, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr::null());
            (gl.gen_framebuffers)(1, &mut fbo);
            (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            (gl.check_framebuffer_status)(GL_FRAMEBUFFER)
        };
        if status != GL_FRAMEBUFFER_COMPLETE {
            // SAFETY: the names were created above on the current context.
            unsafe {
                (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                (gl.delete_framebuffers)(1, &fbo);
                (gl.delete_textures)(1, &tex);
            }
            return Err(super::error::fbo_error(status));
        }
        Ok(Target { image: ptr::null_mut(), tex, fbo, w, h, pitch_px: w })
    }

    /// A target over an existing dma-buf .
    /// The EGL context must be current.
    // EGL handles are opaque: the driver validates them (EGL_BAD_DISPLAY etc.); no Rust memory is read through them.
    #[allow(clippy::too_many_arguments, clippy::not_unsafe_ptr_arg_deref)]
    pub fn from_dmabuf(gl: &Gles, egl: &Egl, ext: &EglExt, dpy: P, w: usize, h: usize, fd: i32, pitch: usize, fourcc: u32) -> io::Result<Target> {
        let attrs = image_attribs_fourcc(w, h, fd, pitch, fourcc);
        // SAFETY: attrs is an EGL_NONE terminated list; the dma-buf fd stays open for the call.
        let image = unsafe { (ext.create_image)(dpy, ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, ptr::null_mut(), attrs.as_ptr()) };
        if image.is_null() {
            // SAFETY: eglGetError has no preconditions.
            return Err(super::error::egl_error("eglCreateImageKHR", unsafe { (egl.get_error)() }));
        }
        let (mut tex, mut fbo) = (0u32, 0u32);
        // SAFETY: the context is current (caller contract); all pointers are to live locals.
        let status = unsafe {
            (gl.gen_textures)(1, &mut tex);
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
            (ext.image_target_texture)(GL_TEXTURE_2D, image);
            (gl.gen_framebuffers)(1, &mut fbo);
            (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            (gl.check_framebuffer_status)(GL_FRAMEBUFFER)
        };
        let t = Target { image, tex, fbo, w, h, pitch_px: pitch / 4 };
        if status != GL_FRAMEBUFFER_COMPLETE {
            t.destroy(gl, ext, dpy);
            return Err(super::error::fbo_error(status));
        }
        Ok(t)
    }

    /// Releases the GL objects and the EGL image (the dma-buf closes on drop).
    // EGL handles are opaque: the driver validates them (EGL_BAD_DISPLAY etc.); no Rust memory is read through them.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn destroy(self, gl: &Gles, ext: &EglExt, dpy: P) {
        // SAFETY: the context is current; the names were created in `new`.
        unsafe {
            (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
            (gl.delete_framebuffers)(1, &self.fbo);
            (gl.delete_textures)(1, &self.tex);
            if !self.image.is_null() {
                (ext.destroy_image)(dpy, self.image);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xb24_is_the_drm_fourcc() {
        assert_eq!(&FOURCC_XB24.to_le_bytes(), b"XB24");
    }

    #[test]
    fn image_attribs_list_the_dmabuf_plane() {
        let a = image_attribs(1200, 1920, 9, 4800);
        assert_eq!(a[..4], [0x3057, 1200, 0x3056, 1920]);
        assert_eq!(a[4..6], [0x3271, 0x3432_4258]);
        assert_eq!(a[6..8], [0x3272, 9]);
        assert_eq!(a[8..10], [0x3273, 0]);
        assert_eq!(a[10..12], [0x3274, 4800]);
        assert_eq!(a[12], 0x3038);
    }
}
