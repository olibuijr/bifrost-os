//! Error naming and mapping: turns raw EGL / GL codes into `io::Error`s with
//! readable messages, and small string helpers. No GPU needed.

use std::io;

pub fn egl_error_name(code: i32) -> &'static str {
    match code {
        0x3000 => "EGL_SUCCESS",
        0x3001 => "EGL_NOT_INITIALIZED",
        0x3002 => "EGL_BAD_ACCESS",
        0x3003 => "EGL_BAD_ALLOC",
        0x3004 => "EGL_BAD_ATTRIBUTE",
        0x3005 => "EGL_BAD_CONFIG",
        0x3006 => "EGL_BAD_CONTEXT",
        0x3007 => "EGL_BAD_CURRENT_SURFACE",
        0x3008 => "EGL_BAD_DISPLAY",
        0x3009 => "EGL_BAD_MATCH",
        0x300A => "EGL_BAD_NATIVE_PIXMAP",
        0x300B => "EGL_BAD_NATIVE_WINDOW",
        0x300C => "EGL_BAD_PARAMETER",
        0x300D => "EGL_BAD_SURFACE",
        0x300E => "EGL_CONTEXT_LOST",
        _ => "EGL_UNKNOWN",
    }
}

pub fn gl_error_name(code: u32) -> &'static str {
    match code {
        0 => "GL_NO_ERROR",
        0x500 => "GL_INVALID_ENUM",
        0x501 => "GL_INVALID_VALUE",
        0x502 => "GL_INVALID_OPERATION",
        0x505 => "GL_OUT_OF_MEMORY",
        0x506 => "GL_INVALID_FRAMEBUFFER_OPERATION",
        _ => "GL_UNKNOWN",
    }
}

pub fn fbo_status_name(code: u32) -> &'static str {
    match code {
        0x8CD5 => "GL_FRAMEBUFFER_COMPLETE",
        0x8CD6 => "GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT",
        0x8CD7 => "GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT",
        0x8CDD => "GL_FRAMEBUFFER_UNSUPPORTED",
        0x8D56 => "GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE",
        _ => "GL_FRAMEBUFFER_UNKNOWN",
    }
}

fn egl_kind(code: i32) -> io::ErrorKind {
    match code {
        0x3003 => io::ErrorKind::OutOfMemory,
        0x3001 | 0x3008 => io::ErrorKind::NotConnected,
        0x3002 => io::ErrorKind::PermissionDenied,
        0x3004 | 0x3005 | 0x3009 | 0x300C => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    }
}

pub fn egl_error(what: &str, code: i32) -> io::Error {
    io::Error::new(egl_kind(code), format!("{what}: {} (0x{code:x})", egl_error_name(code)))
}

pub fn gl_error(what: &str, code: u32) -> io::Error {
    let kind = if code == 0x505 { io::ErrorKind::OutOfMemory } else { io::ErrorKind::Other };
    io::Error::new(kind, format!("{what}: {} (0x{code:x})", gl_error_name(code)))
}

pub fn fbo_error(status: u32) -> io::Error {
    let kind = if status == 0x8CDD { io::ErrorKind::Unsupported } else { io::ErrorKind::Other };
    io::Error::new(kind, format!("render target: {} (0x{status:x})", fbo_status_name(status)))
}

pub fn sym_error(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, format!("GPU driver lacks {name}"))
}

/// Whole-word match in a space separated extension list (a substring test
/// would find `EGL_KHR_image` inside `EGL_KHR_image_base`).
pub fn has_extension(list: &str, name: &str) -> bool {
    list.split_ascii_whitespace().any(|e| e == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egl_names() {
        assert_eq!(egl_error_name(0x3003), "EGL_BAD_ALLOC");
        assert_eq!(egl_error_name(0x300E), "EGL_CONTEXT_LOST");
        assert_eq!(egl_error_name(0x1234), "EGL_UNKNOWN");
    }

    #[test]
    fn gl_and_fbo_names() {
        assert_eq!(gl_error_name(0x505), "GL_OUT_OF_MEMORY");
        assert_eq!(gl_error_name(0), "GL_NO_ERROR");
        assert_eq!(fbo_status_name(0x8CD5), "GL_FRAMEBUFFER_COMPLETE");
        assert_eq!(fbo_status_name(0x8CDD), "GL_FRAMEBUFFER_UNSUPPORTED");
        assert_eq!(fbo_status_name(1), "GL_FRAMEBUFFER_UNKNOWN");
    }

    #[test]
    fn egl_errors_map_to_kinds_and_messages() {
        let e = egl_error("eglCreateImage", 0x3003);
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert!(e.to_string().contains("eglCreateImage") && e.to_string().contains("EGL_BAD_ALLOC") && e.to_string().contains("0x3003"));
        assert_eq!(egl_error("x", 0x3009).kind(), io::ErrorKind::InvalidInput);
        assert_eq!(egl_error("x", 0x3001).kind(), io::ErrorKind::NotConnected);
        assert_eq!(egl_error("x", 0x300D).kind(), io::ErrorKind::Other);
    }

    #[test]
    fn gl_errors_map_to_kinds() {
        assert_eq!(gl_error("tex", 0x505).kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(gl_error("tex", 0x502).kind(), io::ErrorKind::Other);
    }

    #[test]
    fn fbo_errors_map_to_kinds() {
        assert_eq!(fbo_error(0x8CDD).kind(), io::ErrorKind::Unsupported);
        assert!(fbo_error(0x8CD6).to_string().contains("INCOMPLETE_ATTACHMENT"));
    }

    #[test]
    fn missing_symbol_names_the_symbol() {
        let e = sym_error("glFoo");
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        assert!(e.to_string().contains("glFoo"));
    }

    #[test]
    fn extension_match_is_by_word() {
        let l = "EGL_KHR_image_base EGL_EXT_image_dma_buf_import  EGL_ANDROID_native_fence_sync";
        assert!(has_extension(l, "EGL_EXT_image_dma_buf_import"));
        assert!(has_extension(l, "EGL_ANDROID_native_fence_sync"));
        assert!(!has_extension(l, "EGL_KHR_image"));
        assert!(!has_extension("", "EGL_KHR_image"));
    }
}
