//! The desktop `VideoDecoder`: libavcodec through `dlopen`, no link-time dependency and no
//! headers. Hardware decode goes through libavcodec's VA-API hwaccel on the DRM render node
//! (H.264 and HEVC, see `vaapi` for the libva probe that decides it); without VA-API, or for a
//! stream the GPU rejects, the same decoder runs in software. Either way pictures leave as
//! `Pixels::Nv12` CPU copies, so the Player and its NV12 path work unchanged.
//!
//! The C structs are opaque except for the few fields read or written at the offsets below
//! (x86_64 and aarch64 LP64, `AVPacket` / `AVFrame` stable across majors 58..63). The
//! `AVCodecContext` offsets (`get_format`, `hw_device_ctx`) are only valid for libavcodec 63;
//! on any other major the decoder stays software, which needs no `AVCodecContext` field at all.
//! Parameter sets arrive in-band (the Player prepends them as Annex B), so no extradata is set.

use super::vaapi;
use crate::media::dl::Lib;
use crate::media::{Codec, Frame, Pixels, VideoDecoder};
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::io;

type P = *mut c_void;

const CODEC_ID_H264: c_int = 27;
const CODEC_ID_HEVC: c_int = 172;
const PIX_FMT_YUV420P: i32 = 0;
const PIX_FMT_YUVJ420P: i32 = 12;
const PIX_FMT_NV12: i32 = 23;
const PIX_FMT_VAAPI: i32 = 44;
const HWDEVICE_TYPE_VAAPI: c_int = 3;
const NOPTS: i64 = i64::MIN;
const EAGAIN: c_int = -11;
/// `AVERROR_EOF`: `FFERRTAG('E','O','F',' ')` negated.
const EOF: c_int = -541_478_725;

const PKT_PTS: usize = 8;
const PKT_DTS: usize = 16;
const PKT_DATA: usize = 24;
const FRAME_DATA: usize = 0;
const FRAME_LINESIZE: usize = 64;
const FRAME_WIDTH: usize = 104;
const FRAME_HEIGHT: usize = 108;
const FRAME_FORMAT: usize = 116;
const FRAME_PTS: usize = 136;
/// libavcodec 63 `AVCodecContext`.
const CTX_MAJOR: c_uint = 63;
const CTX_GET_FORMAT: usize = 192;
const CTX_HW_DEVICE_CTX: usize = 560;

/// # Safety
/// `base + off` must lie inside the C object and hold a `T`.
unsafe fn rd<T: Copy>(base: *const c_void, off: usize) -> T {
    base.cast::<u8>().add(off).cast::<T>().read_unaligned()
}

/// # Safety
/// `base + off` must lie inside the C object and be a `T` field.
unsafe fn wr<T>(base: *mut c_void, off: usize, v: T) {
    base.cast::<u8>().add(off).cast::<T>().write_unaligned(v)
}

#[derive(Clone, Copy)]
struct Api {
    version: unsafe extern "C" fn() -> c_uint,
    find_decoder: unsafe extern "C" fn(c_int) -> *const c_void,
    alloc_context: unsafe extern "C" fn(*const c_void) -> P,
    open2: unsafe extern "C" fn(P, *const c_void, *mut c_void) -> c_int,
    free_context: unsafe extern "C" fn(*mut P),
    send_packet: unsafe extern "C" fn(P, P) -> c_int,
    receive_frame: unsafe extern "C" fn(P, P) -> c_int,
    flush_buffers: unsafe extern "C" fn(P),
    packet_alloc: unsafe extern "C" fn() -> P,
    packet_free: unsafe extern "C" fn(*mut P),
    packet_unref: unsafe extern "C" fn(P),
    new_packet: unsafe extern "C" fn(P, c_int) -> c_int,
    frame_alloc: unsafe extern "C" fn() -> P,
    frame_free: unsafe extern "C" fn(*mut P),
    frame_unref: unsafe extern "C" fn(P),
    hwdevice_ctx_create: unsafe extern "C" fn(*mut P, c_int, *const c_char, *mut c_void, c_int) -> c_int,
    buffer_ref: unsafe extern "C" fn(P) -> P,
    buffer_unref: unsafe extern "C" fn(*mut P),
    hwframe_transfer_data: unsafe extern "C" fn(P, P, c_int) -> c_int,
    opt_set_int: unsafe extern "C" fn(P, *const c_char, i64, c_int) -> c_int,
    strerror: unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int,
}

impl Api {
    fn load() -> io::Result<Api> {
        let names: Vec<String> = (58..=63).rev().map(|m| format!("libavcodec.so.{m}")).collect();
        let avcodec = Lib::open(&names.iter().map(String::as_str).collect::<Vec<_>>())?;
        // SAFETY: the type is the C prototype of avcodec_version.
        let version: unsafe extern "C" fn() -> c_uint = unsafe { avcodec.sym("avcodec_version")? };
        // libavutil's soname major trails libavcodec's by two (58 -> 56 ... 63 -> 61)
        let major = unsafe { version() } >> 16;
        let util = format!("libavutil.so.{}", major.saturating_sub(2));
        let avutil = Lib::open(&[util.as_str()])?;
        // SAFETY: every type below is the C prototype from the libavcodec / libavutil headers.
        unsafe {
            Ok(Api {
                version,
                find_decoder: avcodec.sym("avcodec_find_decoder")?,
                alloc_context: avcodec.sym("avcodec_alloc_context3")?,
                open2: avcodec.sym("avcodec_open2")?,
                free_context: avcodec.sym("avcodec_free_context")?,
                send_packet: avcodec.sym("avcodec_send_packet")?,
                receive_frame: avcodec.sym("avcodec_receive_frame")?,
                flush_buffers: avcodec.sym("avcodec_flush_buffers")?,
                packet_alloc: avcodec.sym("av_packet_alloc")?,
                packet_free: avcodec.sym("av_packet_free")?,
                packet_unref: avcodec.sym("av_packet_unref")?,
                new_packet: avcodec.sym("av_new_packet")?,
                frame_alloc: avutil.sym("av_frame_alloc")?,
                frame_free: avutil.sym("av_frame_free")?,
                frame_unref: avutil.sym("av_frame_unref")?,
                hwdevice_ctx_create: avutil.sym("av_hwdevice_ctx_create")?,
                buffer_ref: avutil.sym("av_buffer_ref")?,
                buffer_unref: avutil.sym("av_buffer_unref")?,
                hwframe_transfer_data: avutil.sym("av_hwframe_transfer_data")?,
                opt_set_int: avutil.sym("av_opt_set_int")?,
                strerror: avutil.sym("av_strerror")?,
            })
        }
    }

    fn error(&self, code: c_int) -> io::Error {
        let mut buf = [0 as c_char; 128];
        // SAFETY: the buffer is 128 bytes and av_strerror always NUL-terminates within the size given.
        let msg = unsafe {
            (self.strerror)(code, buf.as_mut_ptr(), buf.len());
            std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        };
        io::Error::other(format!("libavcodec: {msg} ({code})"))
    }

    fn major(&self) -> c_uint {
        // SAFETY: no arguments, no state.
        unsafe { (self.version)() >> 16 }
    }
}

/// libavcodec's `get_format`: the VA-API surface format when offered, else the last (software) entry.
unsafe extern "C" fn pick_format(_ctx: *mut c_void, formats: *const i32) -> i32 {
    let (mut p, mut last) = (formats, -1);
    while *p != -1 {
        if *p == PIX_FMT_VAAPI {
            return PIX_FMT_VAAPI;
        }
        last = *p;
        p = p.add(1);
    }
    last
}

/// Copies a decoded software frame (planar or semi-planar 4:2:0, 8 bit) into tight NV12 planes
/// with an even stride.
///
/// # Safety
/// `f` must be a valid `AVFrame` of `w` x `h` whose plane pointers cover `linesize * rows`.
unsafe fn planes(f: *const c_void, w: usize, h: usize) -> io::Result<Pixels> {
    let fmt: i32 = rd(f, FRAME_FORMAT);
    let data: [*const u8; 3] = rd(f, FRAME_DATA);
    let line: [i32; 3] = rd(f, FRAME_LINESIZE);
    if w == 0 || h == 0 || data[0].is_null() || data[1].is_null() {
        return Err(io::Error::other("libavcodec: empty frame"));
    }
    let stride = (w + 1) & !1;
    let (cw, ch) = (stride / 2, h.div_ceil(2));
    let mut y = vec![0u8; stride * h];
    for r in 0..h {
        std::ptr::copy_nonoverlapping(data[0].offset(r as isize * line[0] as isize), y[r * stride..].as_mut_ptr(), w);
    }
    let mut uv = vec![0u8; stride * ch];
    match fmt {
        PIX_FMT_NV12 => {
            for r in 0..ch {
                std::ptr::copy_nonoverlapping(data[1].offset(r as isize * line[1] as isize), uv[r * stride..].as_mut_ptr(), stride);
            }
        }
        PIX_FMT_YUV420P | PIX_FMT_YUVJ420P if !data[2].is_null() => {
            for r in 0..ch {
                let u = data[1].offset(r as isize * line[1] as isize);
                let v = data[2].offset(r as isize * line[2] as isize);
                for x in 0..cw {
                    uv[r * stride + 2 * x] = *u.add(x);
                    uv[r * stride + 2 * x + 1] = *v.add(x);
                }
            }
        }
        other => return Err(io::Error::new(io::ErrorKind::Unsupported, format!("libavcodec pixel format {other} (8 bit 4:2:0 only)"))),
    }
    Ok(Pixels::Nv12 { y, uv, stride: stride as u32 })
}

/// An H.264 / HEVC decoder on libavcodec, VA-API accelerated when the render node can.
pub struct Decoder {
    api: Api,
    ctx: P,
    pkt: P,
    frame: P,
    /// the CPU frame a VA-API surface is downloaded into
    sw: P,
    /// the VA-API device (null in software); also referenced by `ctx`
    dev: P,
    codec: Codec,
    queue: VecDeque<Frame>,
    /// pictures out since the last open: a hardware failure before the first one falls back to software
    produced: u64,
}

// SAFETY: the raw libavcodec objects belong to this decoder alone and are only touched through &mut self.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn open(codec: Codec) -> io::Result<Decoder> {
        let api = Api::load()?;
        let wanted = api.major() == CTX_MAJOR && std::env::var_os("SPRINGBOARD_VIDEO_SW").is_none();
        let dev = if wanted { vaapi::probe().filter(|s| s.has(codec)).map_or(std::ptr::null_mut(), |s| api.hw_device(&s.node)) } else { std::ptr::null_mut() };
        let ctx = match api.open_context(codec, dev) {
            Ok(c) => c,
            Err(e) => {
                api.release_device(dev);
                return Err(e);
            }
        };
        // SAFETY: plain allocations, checked below.
        let (pkt, frame, sw) = unsafe { ((api.packet_alloc)(), (api.frame_alloc)(), (api.frame_alloc)()) };
        let d = Decoder { api, ctx, pkt, frame, sw, dev, codec, queue: VecDeque::new(), produced: 0 };
        if pkt.is_null() || frame.is_null() || sw.is_null() {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "libavcodec allocation failed"));
        }
        Ok(d)
    }

    fn hardware(&self) -> bool {
        !self.dev.is_null()
    }

    /// Reopens without VA-API (the GPU rejected the stream).
    fn reopen_software(&mut self) -> io::Result<()> {
        eprintln!("ffmpeg: hardware decode failed, falling back to software");
        // SAFETY: ctx and dev are ours; free_context also drops the context's device reference.
        unsafe { (self.api.free_context)(&mut self.ctx) };
        self.api.release_device(self.dev);
        self.dev = std::ptr::null_mut();
        self.ctx = self.api.open_context(self.codec, std::ptr::null_mut())?;
        self.queue.clear();
        Ok(())
    }

    /// Sends one access unit; the libavcodec return code (0, EAGAIN or an error).
    fn send(&mut self, data: &[u8], pts: i64) -> io::Result<c_int> {
        let len = c_int::try_from(data.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: new_packet allocates `len` padded bytes into the blank packet, which we fill before sending.
        unsafe {
            if (self.api.new_packet)(self.pkt, len) < 0 {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "av_new_packet"));
            }
            let dst: *mut u8 = rd(self.pkt, PKT_DATA);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            wr(self.pkt, PKT_PTS, pts);
            wr(self.pkt, PKT_DTS, NOPTS);
            let r = (self.api.send_packet)(self.ctx, self.pkt);
            (self.api.packet_unref)(self.pkt);
            Ok(r)
        }
    }

    /// Moves every finished picture into the queue.
    fn drain(&mut self) -> io::Result<()> {
        loop {
            // SAFETY: ctx and frame are valid; the frame is unreferenced after each use.
            let r = unsafe { (self.api.receive_frame)(self.ctx, self.frame) };
            if r == EAGAIN || r == EOF {
                return Ok(());
            }
            if r < 0 {
                return Err(self.api.error(r));
            }
            let f = self.convert();
            // SAFETY: frame holds the picture just converted.
            unsafe { (self.api.frame_unref)(self.frame) };
            self.queue.push_back(f?);
            self.produced += 1;
        }
    }

    /// The received frame as an NV12 `Frame`, downloading a VA-API surface first.
    fn convert(&mut self) -> io::Result<Frame> {
        // SAFETY: `frame` holds a decoded picture; `sw` is a blank frame the transfer fills.
        unsafe {
            let (w, h): (i32, i32) = (rd(self.frame, FRAME_WIDTH), rd(self.frame, FRAME_HEIGHT));
            let pts: i64 = rd(self.frame, FRAME_PTS);
            let on_gpu = rd::<i32>(self.frame, FRAME_FORMAT) == PIX_FMT_VAAPI;
            let src = if on_gpu {
                let r = (self.api.hwframe_transfer_data)(self.sw, self.frame, 0);
                if r < 0 {
                    return Err(self.api.error(r));
                }
                self.sw
            } else {
                self.frame
            };
            let pixels = planes(src, w.max(0) as usize, h.max(0) as usize);
            if on_gpu {
                (self.api.frame_unref)(self.sw);
            }
            Ok(Frame { width: w as u32, height: h as u32, pts_us: if pts == NOPTS { 0 } else { pts }, pixels: pixels? })
        }
    }

    /// Plays out the pictures still held back for reordering (end of stream) and readies the decoder for more input.
    pub fn finish(&mut self) -> io::Result<()> {
        // SAFETY: a null packet is libavcodec's end-of-stream signal.
        unsafe { (self.api.send_packet)(self.ctx, std::ptr::null_mut()) };
        self.drain()?;
        // SAFETY: ctx is open.
        unsafe { (self.api.flush_buffers)(self.ctx) };
        Ok(())
    }
}

impl Api {
    /// A VA-API device on `node`, or null.
    fn hw_device(&self, node: &str) -> P {
        let Ok(c) = CString::new(node) else { return std::ptr::null_mut() };
        let mut dev: P = std::ptr::null_mut();
        // SAFETY: out-pointer and name valid; a failure leaves `dev` null.
        let r = unsafe { (self.hwdevice_ctx_create)(&mut dev, HWDEVICE_TYPE_VAAPI, c.as_ptr(), std::ptr::null_mut(), 0) };
        if r < 0 {
            std::ptr::null_mut()
        } else {
            dev
        }
    }

    fn release_device(&self, mut dev: P) {
        if !dev.is_null() {
            // SAFETY: dev is a buffer reference we own.
            unsafe { (self.buffer_unref)(&mut dev) };
        }
    }

    /// An opened decoder context for `codec`, on `dev` when it is not null.
    fn open_context(&self, codec: Codec, dev: P) -> io::Result<P> {
        let id = match codec {
            Codec::H264 => CODEC_ID_H264,
            Codec::Hevc => CODEC_ID_HEVC,
        };
        // SAFETY: the codec and context pointers come from libavcodec and are checked; a failed open frees the context.
        unsafe {
            let c = (self.find_decoder)(id);
            if c.is_null() {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("libavcodec has no {codec:?} decoder")));
            }
            let mut ctx = (self.alloc_context)(c);
            if ctx.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "avcodec_alloc_context3"));
            }
            if dev.is_null() {
                // software: automatic frame/slice threading (the AVOption "threads")
                (self.opt_set_int)(ctx, c"threads".as_ptr(), 0, 0);
            } else {
                wr::<unsafe extern "C" fn(*mut c_void, *const i32) -> i32>(ctx, CTX_GET_FORMAT, pick_format);
                wr(ctx, CTX_HW_DEVICE_CTX, (self.buffer_ref)(dev));
            }
            let r = (self.open2)(ctx, c, std::ptr::null_mut());
            if r < 0 {
                (self.free_context)(&mut ctx);
                return Err(self.error(r));
            }
            Ok(ctx)
        }
    }
}

impl VideoDecoder for Decoder {
    /// Parameter sets travel in-band (Annex B), so only a codec change matters.
    fn configure(&mut self, codec: Codec, _extradata: &[u8], _width: u32, _height: u32) -> io::Result<()> {
        if codec != self.codec {
            let fresh = Decoder::open(codec)?;
            *self = fresh;
        }
        Ok(())
    }

    fn push(&mut self, nal_units: &[u8], pts_us: i64) -> io::Result<()> {
        let mut r = self.send(nal_units, pts_us)?;
        if r == EAGAIN {
            self.drain()?;
            r = self.send(nal_units, pts_us)?;
        }
        if r < 0 && self.hardware() && self.produced == 0 {
            self.reopen_software()?;
            r = self.send(nal_units, pts_us)?;
        }
        if r < 0 {
            return Err(self.api.error(r));
        }
        match self.drain() {
            Err(_) if self.hardware() && self.produced == 0 => {
                self.reopen_software()?;
                self.send(nal_units, pts_us)?;
                self.drain()
            }
            other => other,
        }
    }

    fn pull(&mut self) -> io::Result<Option<Frame>> {
        Ok(self.queue.pop_front())
    }

    /// A seek: drops every queued and in-flight picture.
    fn flush(&mut self) -> io::Result<()> {
        // SAFETY: ctx is open.
        unsafe { (self.api.flush_buffers)(self.ctx) };
        self.queue.clear();
        Ok(())
    }

    fn describe(&self) -> String {
        let v = self.api.major();
        let how = if self.hardware() { "VA-API hardware" } else { "software" };
        format!("libavcodec {v} {:?} ({how}); {}", self.codec, vaapi::probe().map_or("no VA-API".to_string(), |s| s.summary()))
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: every pointer is ours; each free function nulls its argument.
        unsafe {
            (self.api.free_context)(&mut self.ctx);
            (self.api.packet_free)(&mut self.pkt);
            (self.api.frame_free)(&mut self.frame);
            (self.api.frame_free)(&mut self.sw);
        }
        self.api.release_device(self.dev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `AVFrame` header (plane pointers, line sizes, format) over caller-owned planes.
    fn fake(fmt: i32, planes: [&[u8]; 3], lines: [i32; 3]) -> Vec<u8> {
        let mut f = vec![0u8; 256];
        let ptrs = [planes[0].as_ptr(), planes[1].as_ptr(), planes[2].as_ptr()];
        // SAFETY: the offsets lie inside the 256-byte buffer.
        unsafe {
            wr(f.as_mut_ptr().cast(), FRAME_DATA, ptrs);
            wr(f.as_mut_ptr().cast(), FRAME_LINESIZE, lines);
            wr(f.as_mut_ptr().cast(), FRAME_FORMAT, fmt);
        }
        f
    }

    fn nv12(p: Pixels) -> (Vec<u8>, Vec<u8>, u32) {
        match p {
            Pixels::Nv12 { y, uv, stride } => (y, uv, stride),
            _ => panic!("not NV12"),
        }
    }

    #[test]
    fn i420_interleaves_into_nv12_and_drops_line_padding() {
        let y = [1, 2, 3, 99, 4, 5, 6, 99];
        let (u, v) = ([10, 11, 98], [20, 21, 98]);
        let f = fake(PIX_FMT_YUV420P, [&y[..], &u[..], &v[..]], [4, 3, 3]);
        // SAFETY: the fake covers 3x2 pixels with the planes above.
        let (py, puv, stride) = nv12(unsafe { planes(f.as_ptr().cast(), 3, 2) }.unwrap());
        assert_eq!(stride, 4);
        assert_eq!(py, [1, 2, 3, 0, 4, 5, 6, 0]);
        assert_eq!(puv, [10, 20, 11, 21]);
    }

    #[test]
    fn nv12_input_is_repacked_to_the_even_stride() {
        let y = [7u8; 8];
        let uv = [1, 2, 3, 4, 0, 0];
        let f = fake(PIX_FMT_NV12, [&y[..], &uv[..], &[][..]], [4, 6, 0]);
        // SAFETY: the fake covers 4x2 pixels.
        let (_, puv, stride) = nv12(unsafe { planes(f.as_ptr().cast(), 4, 2) }.unwrap());
        assert_eq!((stride, puv), (4, vec![1, 2, 3, 4]));
    }

    #[test]
    fn other_pixel_formats_are_refused() {
        let p = [0u8; 16];
        let f = fake(64, [&p[..], &p[..], &p[..]], [4, 4, 4]);
        // SAFETY: the planes cover 4x2.
        assert!(unsafe { planes(f.as_ptr().cast(), 4, 2) }.is_err());
    }

    #[test]
    fn get_format_prefers_the_vaapi_surface() {
        let both = [PIX_FMT_VAAPI, PIX_FMT_NV12, -1];
        let software = [PIX_FMT_YUV420P, -1];
        // SAFETY: both lists are -1 terminated.
        unsafe {
            assert_eq!(pick_format(std::ptr::null_mut(), both.as_ptr()), PIX_FMT_VAAPI);
            assert_eq!(pick_format(std::ptr::null_mut(), software.as_ptr()), PIX_FMT_YUV420P);
        }
    }
}
