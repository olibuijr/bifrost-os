//! `dlopen` of the desktop's system libraries (libasound, libva, libavcodec). The shell links
//! none of them; a missing library is an `io::Error` the caller turns into a fallback.

use std::ffi::{c_char, c_int, c_void, CString};
use std::io;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;

/// A loaded shared object. Never unloaded: the function pointers taken from it live as long as the process.
#[derive(Clone, Copy)]
pub struct Lib(*mut c_void);

// SAFETY: a dlopen handle is just a token for the loader, which is thread-safe.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Lib {
    /// The first of `names` (sonames, newest first) the loader finds.
    pub fn open(names: &[&str]) -> io::Result<Lib> {
        for n in names {
            let Ok(c) = CString::new(*n) else { continue };
            // SAFETY: plain dlopen of a NUL-terminated name; the result is checked.
            let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
            if !h.is_null() {
                return Ok(Lib(h));
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("dlopen {} failed", names.join(" / "))))
    }

    /// The symbol `name` as the function pointer type `T`.
    ///
    /// # Safety
    /// `T` must be an `extern "C"` function pointer type matching the C prototype.
    pub unsafe fn sym<T: Copy>(&self, name: &str) -> io::Result<T> {
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        let c = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
        let p = dlsym(self.0, c.as_ptr());
        if p.is_null() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("missing symbol {name}")));
        }
        Ok(std::mem::transmute_copy::<*mut c_void, T>(&p))
    }
}
