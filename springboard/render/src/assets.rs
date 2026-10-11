//! Read-only process-lifetime asset mappings, shared through the OS page cache.
#[cfg(target_os = "linux")]
extern "C" {
    fn mmap(addr: *mut std::ffi::c_void, length: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut std::ffi::c_void;
}
#[cfg(target_os = "linux")]
pub fn map_file(path: &str) -> std::io::Result<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(path)?;
    let len = f.metadata()?.len() as usize;
    if len == 0 {
        return Ok(&[]);
    }
    let p = unsafe { mmap(std::ptr::null_mut(), len, 1, 2, f.as_raw_fd(), 0) };
    if p as isize == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // the mapping outlives the file descriptor and is never unmapped
    Ok(unsafe { std::slice::from_raw_parts(p as *const u8, len) })
}

#[cfg(not(target_os = "linux"))]
pub fn map_file(path: &str) -> std::io::Result<&'static [u8]> {
    Ok(Box::leak(std::fs::read(path)?.into_boxed_slice()))
}
