//! `bootweave-fb`: paints the Bootweave boot frame (frame 0) into a Linux framebuffer, for an initramfs that has no
//! UEFI loader in front of it (the tablet: the boot loader leaves a lit buffer and simpledrm exposes it as
//! `/dev/fb0`). The pixels come from `bootweave_frame::render`, the same call the UEFI loader, `bifrost-bootweave`
//! and `bifrost-wm` make, so every later stage keeps this frame and `bifrost-wm` fades it into the desktop as on the PC
//! (docs/boot-architecture.md, "One frame").
//!
//! The logic here is plain functions over byte slices so it is tested on the host; `main.rs` holds the device I/O.

use bootweave_frame::{render, Appearance, Frame, PixelOrder, Screen};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;

/// `FBIOGET_VSCREENINFO` and `FBIOGET_FSCREENINFO` (linux/fb.h).
const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOGET_FSCREENINFO: u64 = 0x4602;

/// `struct fb_var_screeninfo` is 40 `u32` words. The ones used here (linux/fb.h order): `xres`, `yres`,
/// `xres_virtual`, `yres_virtual`, `xoffset`, `yoffset`, `bits_per_pixel`, `grayscale`, then `red`, `green`, `blue`
/// and `transp` as three words each (offset, length, msb_right).
pub type VarScreenInfo = [u32; 40];
const VAR_XRES: usize = 0;
const VAR_YRES: usize = 1;
const VAR_YOFFSET: usize = 5;
const VAR_BPP: usize = 6;
const VAR_RED_OFFSET: usize = 8;
const VAR_BLUE_OFFSET: usize = 14;

/// `struct fb_fix_screeninfo` (linux/fb.h), laid out as C does; only `line_length` is read.
#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct FixScreenInfo {
    id: [u8; 16],
    smem_start: usize,
    smem_len: u32,
    kind: u32,
    kind_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: usize,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

/// What the painter needs to know about a 32-bit framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbInfo {
    /// Visible size in pixels.
    pub width: u32,
    pub height: u32,
    /// Bytes per row in the buffer (at least `width * 4`).
    pub line_bytes: u32,
    /// Byte offset of the visible area (`yoffset` rows down; 0 unless the buffer was panned).
    pub offset: u64,
    pub order: PixelOrder,
}

impl FbInfo {
    /// From the two screen-info structures' relevant fields. Only 32 bits per pixel are drawn.
    pub fn from_screeninfo(var: &VarScreenInfo, line_length: u32) -> Result<FbInfo, String> {
        let (width, height) = (var[VAR_XRES], var[VAR_YRES]);
        if width == 0 || height == 0 {
            return Err(format!("framebuffer has no size ({width}x{height})"));
        }
        let bpp = var[VAR_BPP];
        if bpp != 32 {
            return Err(format!("framebuffer is {bpp} bits per pixel, only 32 is drawn"));
        }
        let line_bytes = line_length.max(width * 4);
        Ok(FbInfo {
            width,
            height,
            line_bytes,
            offset: var[VAR_YOFFSET] as u64 * line_bytes as u64,
            order: PixelOrder::from_channel_offsets(var[VAR_RED_OFFSET], var[VAR_BLUE_OFFSET]),
        })
    }

    /// A plain pixel buffer without padding, for painting into a regular file.
    pub fn raw(width: u32, height: u32, order: PixelOrder) -> FbInfo {
        FbInfo { width, height, line_bytes: width * 4, offset: 0, order }
    }
}

/// Asks an open `/dev/fbN` for its geometry and pixel layout.
pub fn query(fb: &File) -> io::Result<FbInfo> {
    let fd = fb.as_raw_fd();
    let mut var: VarScreenInfo = [0; 40];
    // SAFETY: `var` is a writable buffer of 160 bytes, the size of `struct fb_var_screeninfo`, which the ioctl fills.
    if unsafe { ioctl(fd, FBIOGET_VSCREENINFO, var.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fix = FixScreenInfo::default();
    // SAFETY: `fix` is a `repr(C)` mirror of `struct fb_fix_screeninfo`, which the ioctl fills.
    if unsafe { ioctl(fd, FBIOGET_FSCREENINFO, &mut fix as *mut FixScreenInfo) } != 0 {
        return Err(io::Error::last_os_error());
    }
    FbInfo::from_screeninfo(&var, fix.line_length).map_err(io::Error::other)
}

/// Renders the boot frame (the mark on the background, no text, no bar) and hands it to `put` one row at a time as
/// `(byte offset in the buffer, row bytes)`. Rows are written straight to their place, so a framebuffer with row
/// padding needs no second buffer.
pub fn paint_with(
    info: &FbInfo,
    appearance: Appearance,
    mut put: impl FnMut(u64, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let (w, h) = (info.width as usize, info.height as usize);
    let frame = Frame { width: info.width, height: info.height, appearance, progress: None, screen: Screen::Boot };
    let mut px = vec![0u32; w * h];
    render(&frame, &mut px, w, info.order);
    let mut row = Vec::with_capacity(w * 4);
    for (y, line) in px.chunks_exact(w).enumerate() {
        row.clear();
        for p in line {
            row.extend_from_slice(&p.to_le_bytes());
        }
        put(info.offset + y as u64 * info.line_bytes as u64, &row)?;
    }
    Ok(())
}

/// Paints the boot frame into an open framebuffer device or a regular file.
pub fn paint(file: &File, info: &FbInfo, appearance: Appearance) -> io::Result<()> {
    paint_with(info, appearance, |at, row| file.write_all_at(row, at))
}

/// `"dark"` or `"light"`.
pub fn parse_appearance(text: &str) -> Option<Appearance> {
    match text {
        "dark" => Some(Appearance::Dark),
        "light" => Some(Appearance::Light),
        _ => None,
    }
}

/// `"WxH"`, both non-zero.
pub fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// `"bgrx"` or `"rgbx"`.
pub fn parse_order(text: &str) -> Option<PixelOrder> {
    match text {
        "bgrx" => Some(PixelOrder::Bgrx),
        "rgbx" => Some(PixelOrder::Rgbx),
        _ => None,
    }
}

/// The command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The framebuffer device.
    pub dev: String,
    /// How long to wait for the device to appear (the display driver may probe after the initramfs starts).
    pub wait_ms: u64,
    pub appearance: Appearance,
    /// Paint a plain pixel file of `size` instead of a framebuffer (host checks and the emulator tests).
    pub raw: Option<String>,
    pub size: Option<(u32, u32)>,
    pub order: PixelOrder,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            dev: "/dev/fb0".into(),
            wait_ms: 2000,
            appearance: Appearance::Dark,
            raw: None,
            size: None,
            order: PixelOrder::Bgrx,
        }
    }
}

pub const USAGE: &str = "usage: bootweave-fb [--dev /dev/fb0] [--wait MS] [--appearance dark|light]\n       \
                         bootweave-fb --raw FILE --size WxH [--order bgrx|rgbx] [--appearance dark|light]";

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options::default();
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
            match flag.as_str() {
                "--dev" => o.dev = value("--dev")?,
                "--wait" => {
                    let v = value("--wait")?;
                    o.wait_ms = v.parse().map_err(|_| format!("--wait: not a number: {v}"))?;
                }
                "--appearance" => {
                    let v = value("--appearance")?;
                    o.appearance = parse_appearance(&v).ok_or_else(|| format!("--appearance: dark or light, not {v}"))?;
                }
                "--raw" => o.raw = Some(value("--raw")?),
                "--size" => {
                    let v = value("--size")?;
                    o.size = Some(parse_size(&v).ok_or_else(|| format!("--size: WxH, not {v}"))?);
                }
                "--order" => {
                    let v = value("--order")?;
                    o.order = parse_order(&v).ok_or_else(|| format!("--order: bgrx or rgbx, not {v}"))?;
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        if o.raw.is_some() && o.size.is_none() {
            return Err("--raw needs --size".into());
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(xres: u32, yres: u32, yoffset: u32, bpp: u32, red: u32, blue: u32) -> VarScreenInfo {
        let mut v = [0u32; 40];
        v[VAR_XRES] = xres;
        v[VAR_YRES] = yres;
        v[VAR_YOFFSET] = yoffset;
        v[VAR_BPP] = bpp;
        v[VAR_RED_OFFSET] = red;
        v[VAR_BLUE_OFFSET] = blue;
        v
    }

    fn collect(info: &FbInfo, total: usize) -> Vec<u8> {
        let mut out = vec![0xaa_u8; total];
        paint_with(info, Appearance::Dark, |at, row| {
            out[at as usize..at as usize + row.len()].copy_from_slice(row);
            Ok(())
        })
        .unwrap();
        out
    }

    #[test]
    fn the_tablet_framebuffer_is_read_from_the_screen_info() {
        // simple-framebuffer a8r8g8b8 at 1200x1920, stride 4800 (kernel/dts/ums512-gta8wifi.dts)
        let info = FbInfo::from_screeninfo(&var(1200, 1920, 0, 32, 16, 0), 4800).unwrap();
        assert_eq!(info, FbInfo { width: 1200, height: 1920, line_bytes: 4800, offset: 0, order: PixelOrder::Bgrx });
    }

    #[test]
    fn rgb_channel_offsets_select_the_other_pixel_order() {
        let info = FbInfo::from_screeninfo(&var(8, 4, 0, 32, 0, 16), 32).unwrap();
        assert_eq!(info.order, PixelOrder::Rgbx);
    }

    #[test]
    fn a_short_line_length_falls_back_to_the_packed_row() {
        let info = FbInfo::from_screeninfo(&var(100, 10, 0, 32, 16, 0), 0).unwrap();
        assert_eq!(info.line_bytes, 400);
    }

    #[test]
    fn a_panned_buffer_is_painted_at_its_visible_rows() {
        let info = FbInfo::from_screeninfo(&var(100, 10, 10, 32, 16, 0), 480).unwrap();
        assert_eq!(info.offset, 4800);
    }

    #[test]
    fn unusable_framebuffers_are_refused() {
        assert!(FbInfo::from_screeninfo(&var(0, 10, 0, 32, 16, 0), 40).is_err());
        assert!(FbInfo::from_screeninfo(&var(10, 0, 0, 32, 16, 0), 40).is_err());
        assert!(FbInfo::from_screeninfo(&var(10, 10, 0, 16, 11, 0), 20).is_err());
    }

    #[test]
    fn the_painted_rows_are_the_shared_renderer_output() {
        let info = FbInfo::raw(64, 48, PixelOrder::Bgrx);
        let bytes = collect(&info, 64 * 48 * 4);
        let mut px = vec![0u32; 64 * 48];
        let frame = Frame::boot(64, 48);
        render(&frame, &mut px, 64, PixelOrder::Bgrx);
        let want: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
        assert_eq!(bytes, want);
    }

    #[test]
    fn row_padding_is_skipped_not_overwritten() {
        let info = FbInfo { width: 16, height: 4, line_bytes: 80, offset: 0, order: PixelOrder::Bgrx };
        let bytes = collect(&info, 80 * 4);
        for y in 0..4 {
            assert!(bytes[y * 80 + 64..(y + 1) * 80].iter().all(|&b| b == 0xaa), "row {y} padding untouched");
            assert_eq!(bytes[y * 80 + 3], 0xff, "row {y} starts with an opaque pixel");
        }
    }

    #[test]
    fn the_dark_frame_is_black_with_the_mark_and_light_is_white() {
        let info = FbInfo::raw(64, 48, PixelOrder::Bgrx);
        let dark = collect(&info, 64 * 48 * 4);
        assert_eq!(&dark[0..4], &[0, 0, 0, 0xff], "dark corner is black");
        let mut light = vec![0u8; 64 * 48 * 4];
        paint_with(&info, Appearance::Light, |at, row| {
            light[at as usize..at as usize + row.len()].copy_from_slice(row);
            Ok(())
        })
        .unwrap();
        assert_eq!(&light[0..4], &[0xff, 0xff, 0xff, 0xff], "light corner is white");
    }

    #[test]
    fn options_parse_and_default() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Options::parse(&[]).unwrap(), Options::default());
        let o = Options::parse(&a(&["--dev", "/dev/fb1", "--wait", "500", "--appearance", "light"])).unwrap();
        assert_eq!((o.dev.as_str(), o.wait_ms, o.appearance), ("/dev/fb1", 500, Appearance::Light));
        let o = Options::parse(&a(&["--raw", "f.bin", "--size", "1200x1920", "--order", "rgbx"])).unwrap();
        assert_eq!((o.raw.as_deref(), o.size, o.order), (Some("f.bin"), Some((1200, 1920)), PixelOrder::Rgbx));
    }

    #[test]
    fn bad_options_are_named() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(Options::parse(&a(&["--wait"])).unwrap_err().contains("needs a value"));
        assert!(Options::parse(&a(&["--wait", "x"])).is_err());
        assert!(Options::parse(&a(&["--appearance", "blue"])).is_err());
        assert!(Options::parse(&a(&["--size", "0x10", "--raw", "f"])).is_err());
        assert!(Options::parse(&a(&["--raw", "f"])).unwrap_err().contains("--size"));
        assert!(Options::parse(&a(&["--bogus"])).unwrap_err().contains("--bogus"));
    }

    #[test]
    fn size_and_order_text() {
        assert_eq!(parse_size("1200x1920"), Some((1200, 1920)));
        assert_eq!(parse_size("1200,1920"), None);
        assert_eq!(parse_size("0x0"), None);
        assert_eq!(parse_order("rgbx"), Some(PixelOrder::Rgbx));
        assert_eq!(parse_order("abgr"), None);
    }
}
