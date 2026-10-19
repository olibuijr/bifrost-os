//! Frame file, ack and line-protocol helpers of the engine.
use std::io::Write;

/// The shared frame file (`$BIFROST_WEBVIEW_FD`, default 3), mapped as large as the app has grown it. The app never shrinks it.
pub struct Shm {
    pub fd: i32,
    pub ptr: *mut u8,
    pub len: usize,
}

impl Shm {
    pub fn ensure(&mut self, len: usize) -> bool {
        if len <= self.len {
            return true;
        }
        unsafe {
            if !self.ptr.is_null() {
                libc::munmap(self.ptr.cast(), self.len);
                self.ptr = std::ptr::null_mut();
                self.len = 0;
            }
            let mut st: libc::stat = std::mem::zeroed();
            if libc::fstat(self.fd, &mut st) != 0 || (st.st_size as usize) < len {
                return false;
            }
            let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, self.fd, 0);
            if p == libc::MAP_FAILED {
                return false;
            }
            self.ptr = p.cast();
            self.len = len;
        }
        true
    }
}

/// Escapes newlines and backslashes for the line protocol and caps the text at 4 KB (mirrors the SDK's `escape_line`).
pub fn escape_line(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let part = match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\\' => "\\\\".to_string(),
            c => c.to_string(),
        };
        if out.len() + part.len() > 4096 {
            break;
        }
        out.push_str(&part);
    }
    out
}

pub fn say(line: &str) {
    let clean: String = line.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{clean}");
    let _ = out.flush();
}


/// Reverses `escape_line` (`\\n`, `\\r`, `\\\\`); an unknown or trailing escape is kept literally.
pub fn unescape_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

impl Shm {
    /// Copies `px` (RGBA bytes, `n` pixels) into the mapping as SDK premultiplied RGBA (`0xAABBGGRR`). Does nothing when the mapping is too small.
    pub fn write_rgba(&mut self, px: &[u8], n: usize) {
        if self.ptr.is_null() || n > self.len / 4 {
            return;
        }
        let dst = unsafe { std::slice::from_raw_parts_mut(self.ptr.cast::<u32>(), n) };
        for (d, p) in dst.iter_mut().zip(px.chunks_exact(4)) {
            // The SDK uses RGBA bytes too. Preserve channel order; web content is opaque.
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            *d = 0xff00_0000 | (v & 0x00ff_ffff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        for s in ["", "a\nb", "a\\nb", "\\", "\r\n\\\\", "tab\tok", "\u{1F600}\n\u{e9}", "trailing\\"] {
            assert_eq!(unescape_line(&escape_line(s)), s);
            assert!(!escape_line(s).contains(['\n', '\r']));
        }
    }

    #[test]
    fn escape_caps_at_4k_on_boundary() {
        let e = escape_line(&"\u{e9}\n".repeat(5000));
        assert!(e.len() <= 4096);
    }

    #[test]
    fn unescape_garbage() {
        assert_eq!(unescape_line("\\"), "\\");
        assert_eq!(unescape_line("\\q"), "\\q");
        assert_eq!(unescape_line("\\\\n"), "\\n");
    }

    #[test]
    fn write_rgba_preserves_channels_and_checks_bounds() {
        let mut buf = [0u32; 2];
        let mut shm = Shm { fd: -1, ptr: buf.as_mut_ptr().cast(), len: 8 };
        shm.write_rgba(&[0x11, 0x22, 0x33, 0xff, 1, 2, 3, 4], 2);
        assert_eq!(buf[0], 0xff33_2211);
        let mut shm = Shm { fd: -1, ptr: buf.as_mut_ptr().cast(), len: 4 };
        shm.write_rgba(&[0; 8], 2);
        assert_eq!(buf[0], 0xff33_2211);
    }
}
