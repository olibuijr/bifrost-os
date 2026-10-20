//! Client side of `bifrost_window_capture_v1` (wm/protocol/bifrost-window-capture-v1.xml, docs/window-server.md "Window
//! capture"): the shell asks the window server for a scaled picture of one window and reads it from a shared-memory
//! buffer it sends along. The global is listed on the privileged socket only (`privileged.rs`); without it the shell has no
//! window pictures and every consumer keeps its fallback.
//!
//! The pure parts (the buffer geometry, the pixel read-back) are here; the requests and events are in `wayland.rs`.

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
pub mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols_wlr::foreign_toplevel::v1::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        // The interface statics name `wayland_backend::protocol::*`; wayland-client re-exports that as `backend`.
        use wayland_client::backend as wayland_backend;
        use wayland_protocols_wlr::foreign_toplevel::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("wm/protocol/bifrost-window-capture-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("wm/protocol/bifrost-window-capture-v1.xml");
}

/// The protocol version the shell binds.
pub const VERSION: u32 = 1;

/// The bytes of a `w` by `h` ARGB8888 buffer; `None` for an empty or absurd size.
pub fn buffer_len(w: u32, h: u32) -> Option<usize> {
    if w == 0 || h == 0 || w > crate::shell::window_shots::MAX_SIDE || h > crate::shell::window_shots::MAX_SIDE {
        return None;
    }
    (w as usize).checked_mul(h as usize)?.checked_mul(4)
}

/// The picture the window server wrote into the top-left corner of a buffer of rows `stride` bytes apart: `w` by `h`
/// pixels as the shell's packed words (the shm ARGB8888 word is the shell's pixel: alpha in the top byte, premultiplied).
/// `None` when the buffer is shorter than the picture says.
pub fn read_picture(bytes: &[u8], stride: usize, w: usize, h: usize) -> Option<Vec<u32>> {
    if w == 0 || h == 0 || stride < w.checked_mul(4)? || bytes.len() < stride.checked_mul(h - 1)?.checked_add(w * 4)? {
        return None;
    }
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &bytes[y * stride..y * stride + w * 4];
        out.extend(row.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]])));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_is_four_bytes_a_pixel_and_never_absurd() {
        assert_eq!(buffer_len(100, 50), Some(20_000));
        assert_eq!(buffer_len(0, 50), None);
        assert_eq!(buffer_len(50, 0), None);
        assert_eq!(buffer_len(crate::shell::window_shots::MAX_SIDE + 1, 1), None);
    }

    #[test]
    fn the_picture_is_read_from_the_corner_with_the_rows_the_buffer_has() {
        // a 4-pixel-wide buffer (stride 16) holding a 2 by 2 picture in its top-left corner
        let mut buf = vec![0u8; 16 * 3];
        for (y, x, v) in [(0usize, 0usize, 0xff11_2233u32), (0, 1, 0xff44_5566), (1, 0, 0x8000_0080), (1, 1, 0xffff_ffff)] {
            buf[y * 16 + x * 4..y * 16 + x * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        buf[8..12].copy_from_slice(&0xdead_beefu32.to_le_bytes()); // outside the picture
        assert_eq!(read_picture(&buf, 16, 2, 2), Some(vec![0xff11_2233, 0xff44_5566, 0x8000_0080, 0xffff_ffff]));
    }

    #[test]
    fn a_short_buffer_gives_no_picture() {
        assert_eq!(read_picture(&[0; 12], 8, 2, 2), None, "the second row is missing");
        assert_eq!(read_picture(&[0; 64], 4, 2, 1), None, "rows shorter than the picture");
        assert_eq!(read_picture(&[], 0, 0, 0), None);
    }

    #[test]
    fn the_protocol_file_has_what_the_client_sends_and_hears() {
        let xml = include_str!("../../wm/protocol/bifrost-window-capture-v1.xml");
        assert!(xml.contains(&format!("name=\"bifrost_window_capture_manager_v1\" version=\"{VERSION}\"")));
        for name in ["capture_surface", "capture_toplevel"] {
            assert!(xml.contains(&format!("<request name=\"{name}\">")), "{name}");
        }
        for name in ["ready", "unchanged", "failed"] {
            assert!(xml.contains(&format!("<event name=\"{name}\">")), "{name}");
        }
    }
}
