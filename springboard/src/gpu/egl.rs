//! The EGL / GLES libraries (Mesa), loaded with `dlopen`.
//! Function tables are plain structs of C function pointers.

use super::error::sym_error;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io;

pub type P = *mut c_void;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
/// Mesa / NVIDIA: EGL and GLES are separate libraries.
pub const DESKTOP_LIBS: (&str, &str) = ("libEGL.so.1", "libGLESv2.so.2");

/// The EGL and GLES library handles.
#[derive(Clone, Copy)]
pub struct Libs {
    pub egl: P,
    pub gles: P,
}

pub fn open_libs() -> io::Result<Libs> {
    let open = |name: &str| {
        let c = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: plain dlopen of a NUL-terminated name; the result is checked.
        let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
        if h.is_null() {
            Err(io::Error::new(io::ErrorKind::NotFound, format!("dlopen {name} failed")))
        } else {
            Ok(h)
        }
    };
    Ok(Libs { egl: open(DESKTOP_LIBS.0)?, gles: open(DESKTOP_LIBS.1)? })
}

// EGL constants
pub const EGL_EXTENSIONS: i32 = 0x3055;
pub const EGL_NONE: i32 = 0x3038;
pub const EGL_WIDTH: i32 = 0x3057;
pub const EGL_HEIGHT: i32 = 0x3056;
pub const EGL_OPENGL_ES_API: u32 = 0x30A0;
pub const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
pub const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
pub const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
pub const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
pub const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
pub const GL_RENDERER: u32 = 0x1F01;
pub const EGL_SYNC_NATIVE_FENCE_ANDROID: u32 = 0x3144;
pub const EGL_NO_NATIVE_FENCE_FD_ANDROID: c_int = -1;

pub const EGL_SYNC_FENCE_KHR: u32 = 0x30F9;
pub const EGL_SYNC_FLUSH_COMMANDS_BIT_KHR: i32 = 1;
pub const EGL_CONDITION_SATISFIED_KHR: i32 = 0x30F6;
pub const EXT_KHR_FENCE: &str = "EGL_KHR_fence_sync";
pub const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
pub const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;

pub const EXT_DMA_BUF: &str = "EGL_EXT_image_dma_buf_import";
pub const EXT_NATIVE_FENCE: &str = "EGL_ANDROID_native_fence_sync";

// GL constants
pub const GL_TEXTURE_2D: u32 = 0xDE1;
pub const GL_FRAMEBUFFER: u32 = 0x8D40;
pub const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
pub const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
pub const GL_BLEND: u32 = 0xBE2;
pub const GL_SCISSOR_TEST: u32 = 0xC11;
pub const GL_ONE: u32 = 1;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x303;
pub const GL_ARRAY_BUFFER: u32 = 0x8892;
pub const GL_STREAM_DRAW: u32 = 0x88E0;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_TRIANGLES: u32 = 4;
pub const GL_TEXTURE0: u32 = 0x84C0;
pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
pub const GL_LINEAR: i32 = 0x2601;
pub const GL_CLAMP_TO_EDGE: i32 = 0x812F;
pub const GL_RGBA: u32 = 0x1908;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_VERTEX_SHADER: u32 = 0x8B31;
pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_COMPILE_STATUS: u32 = 0x8B81;
pub const GL_LINK_STATUS: u32 = 0x8B82;
pub const GL_INFO_LOG_LENGTH: u32 = 0x8B84;
pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
pub const GL_UNPACK_ALIGNMENT: u32 = 0xCF5;
/// GLES 3.0: pixels per row of client memory for uploads (0 = the upload's width)
pub const GL_UNPACK_ROW_LENGTH: u32 = 0xCF2;
pub const GL_NEAREST: i32 = 0x2600;
/// OES_EGL_image_external: the texture target of YCbCr EGL images
pub const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;

/// A function pointer of type `T` from the library; T must be a C fn pointer.
fn sym<T: Copy>(lib: P, name: &str) -> io::Result<T> {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<P>());
    let c = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: dlsym on a live handle with a NUL-terminated name.
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return Err(sym_error(name));
    }
    // SAFETY: T is a pointer-sized extern "C" fn type chosen by the table macro to match the symbol.
    Ok(unsafe { std::mem::transmute_copy(&p) })
}

macro_rules! table {
    ($(#[$m:meta])* $name:ident { $($f:ident : $t:ty = $s:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy)]
        pub struct $name { $(pub $f: $t),* }
        impl $name {
            pub fn load(lib: P) -> io::Result<Self> {
                Ok($name { $($f: sym::<$t>(lib, $s)?),* })
            }
        }
    };
}

table!(Egl {
    get_display: unsafe extern "C" fn(P) -> P = "eglGetDisplay",
    initialize: unsafe extern "C" fn(P, *mut i32, *mut i32) -> u32 = "eglInitialize",
    query_string: unsafe extern "C" fn(P, i32) -> *const c_char = "eglQueryString",
    choose_config: unsafe extern "C" fn(P, *const i32, *mut P, i32, *mut i32) -> u32 = "eglChooseConfig",
    bind_api: unsafe extern "C" fn(u32) -> u32 = "eglBindAPI",
    create_context: unsafe extern "C" fn(P, P, P, *const i32) -> P = "eglCreateContext",
    create_pbuffer: unsafe extern "C" fn(P, P, *const i32) -> P = "eglCreatePbufferSurface",
    make_current: unsafe extern "C" fn(P, P, P, P) -> u32 = "eglMakeCurrent",
    get_error: unsafe extern "C" fn() -> i32 = "eglGetError",
    get_proc: unsafe extern "C" fn(*const c_char) -> P = "eglGetProcAddress",
    destroy_context: unsafe extern "C" fn(P, P) -> u32 = "eglDestroyContext",
    destroy_surface: unsafe extern "C" fn(P, P) -> u32 = "eglDestroySurface",
});

table!(Gles {
    get_error: unsafe extern "C" fn() -> u32 = "glGetError",
    viewport: unsafe extern "C" fn(i32, i32, i32, i32) = "glViewport",
    scissor: unsafe extern "C" fn(i32, i32, i32, i32) = "glScissor",
    clear_color: unsafe extern "C" fn(f32, f32, f32, f32) = "glClearColor",
    clear: unsafe extern "C" fn(u32) = "glClear",
    finish: unsafe extern "C" fn() = "glFinish",
    flush: unsafe extern "C" fn() = "glFlush",
    enable: unsafe extern "C" fn(u32) = "glEnable",
    disable: unsafe extern "C" fn(u32) = "glDisable",
    blend_func: unsafe extern "C" fn(u32, u32) = "glBlendFunc",
    gen_textures: unsafe extern "C" fn(i32, *mut u32) = "glGenTextures",
    delete_textures: unsafe extern "C" fn(i32, *const u32) = "glDeleteTextures",
    bind_texture: unsafe extern "C" fn(u32, u32) = "glBindTexture",
    active_texture: unsafe extern "C" fn(u32) = "glActiveTexture",
    tex_parameteri: unsafe extern "C" fn(u32, u32, i32) = "glTexParameteri",
    tex_image_2d: unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) = "glTexImage2D",
    // glTexSubImage2D(target, level, xoffset, yoffset, width, height, format, type, pixels)
    tex_sub_image_2d: unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) = "glTexSubImage2D",
    pixel_storei: unsafe extern "C" fn(u32, i32) = "glPixelStorei",
    gen_framebuffers: unsafe extern "C" fn(i32, *mut u32) = "glGenFramebuffers",
    delete_framebuffers: unsafe extern "C" fn(i32, *const u32) = "glDeleteFramebuffers",
    bind_framebuffer: unsafe extern "C" fn(u32, u32) = "glBindFramebuffer",
    framebuffer_texture_2d: unsafe extern "C" fn(u32, u32, u32, u32, i32) = "glFramebufferTexture2D",
    check_framebuffer_status: unsafe extern "C" fn(u32) -> u32 = "glCheckFramebufferStatus",
    create_shader: unsafe extern "C" fn(u32) -> u32 = "glCreateShader",
    shader_source: unsafe extern "C" fn(u32, i32, *const *const c_char, *const i32) = "glShaderSource",
    compile_shader: unsafe extern "C" fn(u32) = "glCompileShader",
    get_shaderiv: unsafe extern "C" fn(u32, u32, *mut i32) = "glGetShaderiv",
    get_shader_info_log: unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char) = "glGetShaderInfoLog",
    delete_shader: unsafe extern "C" fn(u32) = "glDeleteShader",
    create_program: unsafe extern "C" fn() -> u32 = "glCreateProgram",
    attach_shader: unsafe extern "C" fn(u32, u32) = "glAttachShader",
    link_program: unsafe extern "C" fn(u32) = "glLinkProgram",
    get_programiv: unsafe extern "C" fn(u32, u32, *mut i32) = "glGetProgramiv",
    get_program_info_log: unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char) = "glGetProgramInfoLog",
    use_program: unsafe extern "C" fn(u32) = "glUseProgram",
    delete_program: unsafe extern "C" fn(u32) = "glDeleteProgram",
    get_uniform_location: unsafe extern "C" fn(u32, *const c_char) -> i32 = "glGetUniformLocation",
    uniform1i: unsafe extern "C" fn(i32, i32) = "glUniform1i",
    uniform1f: unsafe extern "C" fn(i32, f32) = "glUniform1f",
    uniform2f: unsafe extern "C" fn(i32, f32, f32) = "glUniform2f",
    uniform4f: unsafe extern "C" fn(i32, f32, f32, f32, f32) = "glUniform4f",
    get_string: unsafe extern "C" fn(u32) -> *const c_char = "glGetString",
    gen_buffers: unsafe extern "C" fn(i32, *mut u32) = "glGenBuffers",
    delete_buffers: unsafe extern "C" fn(i32, *const u32) = "glDeleteBuffers",
    bind_buffer: unsafe extern "C" fn(u32, u32) = "glBindBuffer",
    buffer_data: unsafe extern "C" fn(u32, isize, *const c_void, u32) = "glBufferData",
    gen_vertex_arrays: unsafe extern "C" fn(i32, *mut u32) = "glGenVertexArrays",
    delete_vertex_arrays: unsafe extern "C" fn(i32, *const u32) = "glDeleteVertexArrays",
    bind_vertex_array: unsafe extern "C" fn(u32) = "glBindVertexArray",
    enable_vertex_attrib_array: unsafe extern "C" fn(u32) = "glEnableVertexAttribArray",
    vertex_attrib_pointer: unsafe extern "C" fn(u32, i32, u32, u8, i32, *const c_void) = "glVertexAttribPointer",
    draw_arrays: unsafe extern "C" fn(u32, i32, i32) = "glDrawArrays",
    blit_framebuffer: unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, i32) = "glBlitFramebuffer",
});

/// Extension entry points, resolved through `eglGetProcAddress`.
#[derive(Clone, Copy)]
pub struct EglExt {
    pub create_image: unsafe extern "C" fn(P, P, u32, P, *const i32) -> P,
    pub destroy_image: unsafe extern "C" fn(P, P) -> u32,
    pub image_target_texture: unsafe extern "C" fn(u32, P),
    /// `EGL_ANDROID_native_fence_sync`, when the driver has it
    pub fence: Option<FenceFns>,
    /// `EGL_KHR_fence_sync`: a CPU-side wait with a timeout, used instead of `glFinish`
    pub khr: Option<KhrFenceFns>,
}

#[derive(Clone, Copy)]
pub struct KhrFenceFns {
    pub create_sync: unsafe extern "C" fn(P, u32, *const i32) -> P,
    pub destroy_sync: unsafe extern "C" fn(P, P) -> u32,
    /// (display, sync, flags, timeout in ns) -> EGL_CONDITION_SATISFIED_KHR / TIMEOUT_EXPIRED / 0
    pub client_wait: unsafe extern "C" fn(P, P, i32, u64) -> i32,
}

#[derive(Clone, Copy)]
pub struct FenceFns {
    pub create_sync: unsafe extern "C" fn(P, u32, *const i32) -> P,
    pub destroy_sync: unsafe extern "C" fn(P, P) -> u32,
    pub dup_fence_fd: unsafe extern "C" fn(P, P) -> c_int,
}

fn proc_addr<T: Copy>(egl: &Egl, name: &str) -> Option<T> {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<P>());
    let c = CString::new(name).ok()?;
    // SAFETY: eglGetProcAddress with a NUL-terminated name; null means unsupported.
    let p = unsafe { (egl.get_proc)(c.as_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: T is the extension's documented pointer-sized C signature.
    Some(unsafe { std::mem::transmute_copy(&p) })
}

impl EglExt {
    pub fn load(egl: &Egl, extensions: &str) -> io::Result<Self> {
        let need = |n: &str| sym_error(n);
        let create_image = proc_addr(egl, "eglCreateImageKHR").ok_or_else(|| need("eglCreateImageKHR"))?;
        let destroy_image = proc_addr(egl, "eglDestroyImageKHR").ok_or_else(|| need("eglDestroyImageKHR"))?;
        let image_target_texture =
            proc_addr(egl, "glEGLImageTargetTexture2DOES").ok_or_else(|| need("glEGLImageTargetTexture2DOES"))?;
        let fence = if super::error::has_extension(extensions, EXT_NATIVE_FENCE) {
            match (
                proc_addr(egl, "eglCreateSyncKHR"),
                proc_addr(egl, "eglDestroySyncKHR"),
                proc_addr(egl, "eglDupNativeFenceFDANDROID"),
            ) {
                (Some(create_sync), Some(destroy_sync), Some(dup_fence_fd)) => {
                    Some(FenceFns { create_sync, destroy_sync, dup_fence_fd })
                }
                _ => None,
            }
        } else {
            None
        };
        let khr = if super::error::has_extension(extensions, EXT_KHR_FENCE) {
            match (
                proc_addr(egl, "eglCreateSyncKHR"),
                proc_addr(egl, "eglDestroySyncKHR"),
                proc_addr(egl, "eglClientWaitSyncKHR"),
            ) {
                (Some(create_sync), Some(destroy_sync), Some(client_wait)) => Some(KhrFenceFns { create_sync, destroy_sync, client_wait }),
                _ => None,
            }
        } else {
            None
        };
        Ok(EglExt { create_image, destroy_image, image_target_texture, fence, khr })
    }
}

/// Copies a driver-owned C string.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string that stays valid for the call (what EGL and GL return).
pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: EGL/GL hand back NUL-terminated strings owned by the driver.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a desktop GL stack (Mesa or NVIDIA) on the host, like the dev machine and the Arch target.
    #[test]
    fn desktop_libraries_resolve_every_function_the_compositor_uses() {
        let libs = open_libs().expect("libEGL.so.1 and libGLESv2.so.2");
        assert_ne!(libs.egl, libs.gles);
        Egl::load(libs.egl).expect("EGL symbols");
        Gles::load(libs.gles).expect("GLES symbols");
    }
}
