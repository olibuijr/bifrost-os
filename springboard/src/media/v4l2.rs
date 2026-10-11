//! V4L2 capture without libv4l: probing /dev/video*, mmap streaming and
//! conversion of the camera's pixel formats (YUYV, NV12, NV21, MJPEG) to
//! the shell's RGBA pixels. Single-planar capture only; a node that offers
//! just the multi-planar API is reported as unusable.
//!
//! Nothing here opens a device until `probe` / `Device::open` is called, and
//! those are only called on an explicit user action in the Camera app. The
//! node and formats are not assumed: everything is discovered with
//! VIDIOC_QUERYCAP / ENUM_FMT / S_FMT and failures are plain `io::Error`s.

use crate::bare::sys::{close, ioc, ioctl_ptr, iow, iowr, mmap, munmap, open_dev, MAP_SHARED, O_NONBLOCK, O_RDWR, PROT_RW};
use crate::gfx::pack;
use std::ffi::{c_int, c_void};
use std::io;

pub const CAP_VIDEO_CAPTURE: u32 = 0x1;
pub const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x1000;
pub const CAP_META_CAPTURE: u32 = 0x0080_0000;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

const BUF_TYPE_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FIELD_ANY: u32 = 0;
const EAGAIN: i32 = 11;
const BUFFERS: u32 = 4;
/// The frame size asked for first; the driver rounds it to what it has.
const WANT: (u32, u32) = (1280, 720);

pub const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}
pub const YUYV: u32 = fourcc(b"YUYV");
pub const NV12: u32 = fourcc(b"NV12");
pub const NV21: u32 = fourcc(b"NV21");
pub const MJPG: u32 = fourcc(b"MJPG");
/// Formats we can convert, best first (raw YUV is cheapest on the CPU).
const PREFERRED: [u32; 4] = [YUYV, NV12, NV21, MJPG];

/// struct v4l2_capability
#[repr(C)]
#[derive(Default)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

/// struct v4l2_fmtdesc
#[repr(C)]
#[derive(Default)]
struct FmtDesc {
    index: u32,
    kind: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

/// struct v4l2_format with the `pix` member of its union (the union is 200
/// bytes and pointer-aligned, so `pix` starts at offset 8 on 64-bit).
#[repr(C)]
struct Format {
    kind: u32,
    _pad: u32,
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
    rest: [u8; 200 - 48],
}

/// struct v4l2_requestbuffers
#[repr(C)]
#[derive(Default)]
struct RequestBuffers {
    count: u32,
    kind: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

/// struct v4l2_buffer (64-bit layout; `m` is the offset/userptr union)
#[repr(C)]
struct Buffer {
    index: u32,
    kind: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: [i64; 2],
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    m: u64,
    length: u32,
    reserved2: u32,
    request_fd: u32,
}

fn ior<T>(nr: u8) -> u32 {
    ioc(2, b'V', nr, std::mem::size_of::<T>())
}
fn vidioc_querycap() -> u32 {
    ior::<Capability>(0)
}
fn vidioc_enum_fmt() -> u32 {
    iowr::<FmtDesc>(b'V', 2)
}
fn vidioc_s_fmt() -> u32 {
    iowr::<Format>(b'V', 5)
}
fn vidioc_reqbufs() -> u32 {
    iowr::<RequestBuffers>(b'V', 8)
}
fn vidioc_querybuf() -> u32 {
    iowr::<Buffer>(b'V', 9)
}
fn vidioc_qbuf() -> u32 {
    iowr::<Buffer>(b'V', 15)
}
fn vidioc_dqbuf() -> u32 {
    iowr::<Buffer>(b'V', 17)
}
fn vidioc_streamon() -> u32 {
    iow::<c_int>(b'V', 18)
}
fn vidioc_streamoff() -> u32 {
    iow::<c_int>(b'V', 19)
}

fn cstr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|&b| b == 0).next().unwrap_or(&[])).into_owned()
}

/// A usable capture node found by `probe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraInfo {
    pub path: String,
    pub card: String,
}

/// Whether the capability words describe a single-planar streaming capture node.
pub fn usable(capabilities: u32, device_caps: u32) -> bool {
    let caps = if capabilities & CAP_DEVICE_CAPS != 0 { device_caps } else { capabilities };
    caps & CAP_VIDEO_CAPTURE != 0 && caps & CAP_META_CAPTURE == 0 && caps & CAP_STREAMING != 0
}

fn query_caps(fd: c_int) -> io::Result<Capability> {
    let mut cap = Capability::default();
    unsafe { ioctl_ptr(fd, vidioc_querycap(), &mut cap) }?;
    Ok(cap)
}

/// The pixel formats a node offers (FOURCC codes).
fn enum_formats(fd: c_int) -> Vec<u32> {
    let mut out = Vec::new();
    for index in 0..64 {
        let mut d = FmtDesc { index, kind: BUF_TYPE_CAPTURE, ..Default::default() };
        if unsafe { ioctl_ptr(fd, vidioc_enum_fmt(), &mut d) }.is_err() {
            break;
        }
        out.push(d.pixelformat);
    }
    out
}

/// Lists the /dev/video* nodes that can stream single-planar video capture
/// in a format we convert. Opens and closes each node; call it only on a
/// user action.
pub fn probe() -> Vec<CameraInfo> {
    let mut nodes: Vec<(u32, String)> = std::fs::read_dir("/dev")
        .map(|d| d.flatten().filter_map(|e| Some((e.file_name().to_string_lossy().strip_prefix("video")?.parse().ok()?, e.path().to_string_lossy().into_owned()))).collect())
        .unwrap_or_default();
    nodes.sort();
    let mut out = Vec::new();
    for (_, path) in nodes {
        let Ok(fd) = open_dev(&path, O_RDWR | O_NONBLOCK) else { continue };
        let found = query_caps(fd).ok().filter(|c| usable(c.capabilities, c.device_caps)).and_then(|c| {
            let formats = enum_formats(fd);
            PREFERRED.iter().any(|f| formats.contains(f)).then(|| CameraInfo { path: path.clone(), card: cstr(&c.card) })
        });
        // SAFETY: fd came from open_dev and is closed exactly once.
        unsafe { close(fd) };
        out.extend(found);
    }
    out
}

struct Mapping {
    ptr: *mut c_void,
    len: usize,
}

/// One owned frame as the driver delivered it.
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub format: u32,
}

/// An open, streaming capture node.
pub struct Device {
    fd: c_int,
    maps: Vec<Mapping>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub format: u32,
    streaming: bool,
}

impl Device {
    /// Opens `path`, picks a format and starts streaming.
    pub fn open(path: &str) -> io::Result<Device> {
        let fd = open_dev(path, O_RDWR | O_NONBLOCK)?;
        let mut dev = Device { fd, maps: Vec::new(), width: 0, height: 0, stride: 0, format: 0, streaming: false };
        dev.start()?; // on error Drop releases everything
        Ok(dev)
    }

    fn start(&mut self) -> io::Result<()> {
        let cap = query_caps(self.fd)?;
        if !usable(cap.capabilities, cap.device_caps) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "not a single-planar streaming capture node"));
        }
        let offered = enum_formats(self.fd);
        let mut last = io::Error::new(io::ErrorKind::Unsupported, "no supported pixel format");
        for &f in PREFERRED.iter().filter(|f| offered.contains(f)) {
            // SAFETY: Format is plain data.
            let mut fmt: Format = unsafe { std::mem::zeroed() };
            fmt.kind = BUF_TYPE_CAPTURE;
            fmt.width = WANT.0;
            fmt.height = WANT.1;
            fmt.pixelformat = f;
            fmt.field = FIELD_ANY;
            match unsafe { ioctl_ptr(self.fd, vidioc_s_fmt(), &mut fmt) } {
                // the driver may substitute another format: accept it only when we can convert it
                Ok(()) if PREFERRED.contains(&fmt.pixelformat) => {
                    self.width = fmt.width as usize;
                    self.height = fmt.height as usize;
                    self.stride = if fmt.bytesperline > 0 { fmt.bytesperline as usize } else { self.width };
                    self.format = fmt.pixelformat;
                    break;
                }
                Ok(()) => {}
                Err(e) => last = e,
            }
        }
        if self.format == 0 {
            return Err(last);
        }
        let mut req = RequestBuffers { count: BUFFERS, kind: BUF_TYPE_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
        unsafe { ioctl_ptr(self.fd, vidioc_reqbufs(), &mut req) }?;
        for index in 0..req.count {
            let mut b = Self::buffer(index);
            unsafe { ioctl_ptr(self.fd, vidioc_querybuf(), &mut b) }?;
            // SAFETY: mapping the buffer the driver just described; unmapped in Drop.
            let ptr = unsafe { mmap(std::ptr::null_mut(), b.length as usize, PROT_RW, MAP_SHARED, self.fd, b.m as i64) };
            if ptr as isize == -1 {
                return Err(io::Error::last_os_error());
            }
            self.maps.push(Mapping { ptr, len: b.length as usize });
            unsafe { ioctl_ptr(self.fd, vidioc_qbuf(), &mut b) }?;
        }
        let mut kind = BUF_TYPE_CAPTURE as c_int;
        unsafe { ioctl_ptr(self.fd, vidioc_streamon(), &mut kind) }?;
        self.streaming = true;
        Ok(())
    }

    fn buffer(index: u32) -> Buffer {
        // SAFETY: Buffer is plain data.
        let mut b: Buffer = unsafe { std::mem::zeroed() };
        b.index = index;
        b.kind = BUF_TYPE_CAPTURE;
        b.memory = MEMORY_MMAP;
        b
    }

    /// The newest frame the driver has ready, or None when there is none yet;
    /// older queued frames are skipped. Never blocks.
    pub fn latest(&mut self) -> io::Result<Option<Frame>> {
        let mut held: Option<(u32, usize)> = None;
        loop {
            let mut b = Self::buffer(0);
            match unsafe { ioctl_ptr(self.fd, vidioc_dqbuf(), &mut b) } {
                Ok(()) => {
                    if let Some((old, _)) = held.replace((b.index, b.bytesused as usize)) {
                        let mut q = Self::buffer(old);
                        unsafe { ioctl_ptr(self.fd, vidioc_qbuf(), &mut q) }?;
                    }
                }
                Err(e) if e.raw_os_error() == Some(EAGAIN) => break,
                Err(e) => return Err(e),
            }
        }
        let Some((index, used)) = held else { return Ok(None) };
        let m = &self.maps[index as usize];
        let n = if used == 0 { m.len } else { used.min(m.len) };
        // SAFETY: the mapping is m.len bytes and stays valid until Drop.
        let data = unsafe { std::slice::from_raw_parts(m.ptr as *const u8, n) }.to_vec();
        let mut q = Self::buffer(index);
        unsafe { ioctl_ptr(self.fd, vidioc_qbuf(), &mut q) }?;
        Ok(Some(Frame { data, width: self.width, height: self.height, stride: self.stride, format: self.format }))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if self.streaming {
            let mut kind = BUF_TYPE_CAPTURE as c_int;
            let _ = unsafe { ioctl_ptr(self.fd, vidioc_streamoff(), &mut kind) };
        }
        for m in &self.maps {
            // SAFETY: each mapping came from mmap in `start` and is released once.
            unsafe { munmap(m.ptr, m.len) };
        }
        // SAFETY: fd came from open_dev and is closed exactly once.
        unsafe { close(self.fd) };
    }
}

fn clamp8(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

/// BT.601 limited-range YCbCr to a packed opaque pixel.
pub fn yuv_pixel(y: u8, u: u8, v: u8) -> u32 {
    let c = (y as i32 - 16).max(0) * 298;
    let (d, e) = (u as i32 - 128, v as i32 - 128);
    pack(clamp8((c + 409 * e + 128) >> 8) as u8, clamp8((c - 100 * d - 208 * e + 128) >> 8) as u8, clamp8((c + 516 * d + 128) >> 8) as u8, 255)
}

/// Converts `frame` to `ow` x `oh` pixels by nearest-neighbour sampling, so a
/// small viewfinder costs a fraction of a full-size conversion. A frame whose
/// data is too short for its geometry yields None.
pub fn to_pixels(frame: &Frame, ow: usize, oh: usize) -> Option<Vec<u32>> {
    to_pixels_region(frame, (0, 0, frame.width, frame.height), ow, oh)
}

/// Like `to_pixels` for only the `(x, y, w, h)` region of the frame: a digital
/// zoom samples the sensor's pixels instead of enlarging an already shrunk picture.
pub fn to_pixels_region(frame: &Frame, (rx, ry, rw, rh): (usize, usize, usize, usize), ow: usize, oh: usize) -> Option<Vec<u32>> {
    let (w, h, stride) = (frame.width, frame.height, frame.stride.max(frame.width));
    if w == 0 || h == 0 || ow == 0 || oh == 0 || rw == 0 || rh == 0 || rx + rw > w || ry + rh > h {
        return None;
    }
    // source column and row of every output column and row, worked out once
    let cols: Vec<usize> = (0..ow).map(|x| rx + x * rw / ow).collect();
    let rows: Vec<usize> = (0..oh).map(|y| ry + y * rh / oh).collect();
    let mut out = Vec::with_capacity(ow * oh);
    match frame.format {
        YUYV => {
            if frame.data.len() < (h - 1) * frame.stride.max(w * 2) + w * 2 {
                return None;
            }
            let row = frame.stride.max(w * 2);
            for &sy in &rows {
                for &sx in &cols {
                    let at = sy * row + (sx & !1) * 2;
                    let luma = frame.data[sy * row + sx * 2];
                    out.push(yuv_pixel(luma, frame.data[at + 1], frame.data[at + 3]));
                }
            }
        }
        NV12 | NV21 => {
            let base = stride * h;
            if frame.data.len() < base + stride * h.div_ceil(2) {
                return None;
            }
            for &sy in &rows {
                for &sx in &cols {
                    let c = base + (sy / 2) * stride + (sx & !1);
                    let (a, b) = (frame.data[c], frame.data[c + 1]);
                    let (u, v) = if frame.format == NV12 { (a, b) } else { (b, a) };
                    out.push(yuv_pixel(frame.data[sy * stride + sx], u, v));
                }
            }
        }
        MJPG => {
            let img = crate::imgdec::decode(&frame.data)?;
            if img.w == 0 || img.h == 0 {
                return None;
            }
            for &sy in &rows {
                let line = &img.px[(sy * img.h / h).min(img.h - 1) * img.w..][..img.w];
                out.extend(cols.iter().map(|&sx| line[(sx * img.w / w).min(img.w - 1)]));
            }
        }
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn uapi_struct_sizes_match_the_kernel() {
        assert_eq!(std::mem::size_of::<Capability>(), 104);
        assert_eq!(std::mem::size_of::<FmtDesc>(), 64);
        assert_eq!(std::mem::size_of::<Format>(), 208);
        assert_eq!(std::mem::size_of::<RequestBuffers>(), 20);
        assert_eq!(std::mem::size_of::<Buffer>(), 88);
    }

    #[test]
    fn ioctl_numbers_match_videodev2() {
        assert_eq!(vidioc_querycap(), 0x8068_5600);
        assert_eq!(vidioc_s_fmt(), 0xC0D0_5605);
        assert_eq!(vidioc_reqbufs(), 0xC014_5608);
        assert_eq!(vidioc_qbuf(), 0xC058_560F);
        assert_eq!(vidioc_dqbuf(), 0xC058_5611);
        assert_eq!(vidioc_streamon(), 0x4004_5612);
    }

    #[test]
    fn fourcc_codes_are_little_endian() {
        assert_eq!(YUYV, 0x5659_5559);
        assert_eq!(MJPG, 0x4750_4A4D);
    }

    #[test]
    fn capability_filter_prefers_device_caps_and_rejects_metadata_nodes() {
        assert!(usable(CAP_VIDEO_CAPTURE | CAP_STREAMING, 0));
        assert!(usable(CAP_DEVICE_CAPS, CAP_VIDEO_CAPTURE | CAP_STREAMING));
        assert!(!usable(CAP_DEVICE_CAPS, CAP_META_CAPTURE | CAP_STREAMING));
        assert!(!usable(CAP_VIDEO_CAPTURE_MPLANE | CAP_STREAMING, 0));
        assert!(!usable(CAP_VIDEO_CAPTURE, 0));
    }

    #[test]
    fn yuv_extremes_convert_to_black_white_and_grey() {
        assert_eq!(yuv_pixel(16, 128, 128), pack(0, 0, 0, 255));
        assert_eq!(yuv_pixel(235, 128, 128), pack(255, 255, 255, 255));
        let g = yuv_pixel(126, 128, 128);
        assert!(((g & 255) as i32 - 128).abs() <= 1);
    }

    #[test]
    fn yuyv_frame_converts_and_scales() {
        // 2x2, all white luma with neutral chroma
        let frame = Frame { data: vec![235, 128, 235, 128, 235, 128, 235, 128], width: 2, height: 2, stride: 4, format: YUYV };
        let px = to_pixels(&frame, 4, 4).unwrap();
        assert_eq!(px.len(), 16);
        assert!(px.iter().all(|&p| p == pack(255, 255, 255, 255)));
    }

    #[test]
    fn nv12_and_nv21_swap_the_chroma_order() {
        // 2x2: luma 128, U=90, V=200 (NV12 order) / V,U (NV21 order)
        let nv12 = Frame { data: vec![128, 128, 128, 128, 90, 200], width: 2, height: 2, stride: 2, format: NV12 };
        let nv21 = Frame { data: vec![128, 128, 128, 128, 200, 90], width: 2, height: 2, stride: 2, format: NV21 };
        assert_eq!(to_pixels(&nv12, 2, 2), to_pixels(&nv21, 2, 2));
        assert_eq!(to_pixels(&nv12, 2, 2).unwrap()[0], yuv_pixel(128, 90, 200));
    }

    #[test]
    fn a_region_samples_only_its_own_pixels() {
        // 4x2 NV12: luma 16 (black) in the left half, 235 (white) in the right half; neutral chroma
        let frame = Frame { data: vec![16, 16, 235, 235, 16, 16, 235, 235, 128, 128, 128, 128], width: 4, height: 2, stride: 4, format: NV12 };
        let right = to_pixels_region(&frame, (2, 0, 2, 2), 2, 2).unwrap();
        assert!(right.iter().all(|&p| p == pack(255, 255, 255, 255)));
        let left = to_pixels_region(&frame, (0, 0, 2, 2), 2, 2).unwrap();
        assert!(left.iter().all(|&p| p == pack(0, 0, 0, 255)));
        assert_eq!(to_pixels_region(&frame, (0, 0, 4, 2), 4, 2), to_pixels(&frame, 4, 2));
        assert!(to_pixels_region(&frame, (3, 0, 2, 2), 2, 2).is_none(), "a region past the edge is refused");
    }

    #[test]
    fn short_or_unknown_frames_are_refused() {
        let short = Frame { data: vec![0; 3], width: 2, height: 2, stride: 4, format: YUYV };
        assert!(to_pixels(&short, 2, 2).is_none());
        let odd = Frame { data: vec![0; 64], width: 2, height: 2, stride: 2, format: fourcc(b"XXXX") };
        assert!(to_pixels(&odd, 2, 2).is_none());
    }
}
