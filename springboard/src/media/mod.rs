//! Media playback: MP4 demux, AAC-LC decode, ALSA output, A/V sync and a
//! `Player` that drives it all for the Deildu app.
//!
//! # Codec decisions
//!
//! **Video is decoded by libavcodec**, `dlopen`ed: VA-API hardware decode on the DRM render node
//! when libva reports it, software otherwise (`hwdec::ffmpeg`, `hwdec::vaapi`). The player only
//! sees the [`VideoDecoder`] trait below; `default_decoder` is the seam where `hwdec::open` is
//! plugged in. Audio goes out through an IO procedure on the default device of
//! `bifrost-audiod` (`speaker`), which routes and mixes it. No H.264 decoder is linked into the binary.
//!
//! **Audio is pure Rust**: AAC-LC through `symphonia-codec-aac` (MPL-2.0,
//! no C), the only codec crate in the tree, resampled to the 48 kHz stereo
//! S16 the ALSA device is opened with. HE-AAC radio streams get their high
//! band back from `sbr` (spectral band replication, written in-tree).
//!
//! # Threads
//!
//! `Player::open` starts an audio thread (demux → AAC → ALSA; it owns the
//! clock) and a video thread (demux → `VideoDecoder` → NV12/RGBA → scaled
//! RGBA frame) that hands frames to the UI through a double buffer and a
//! pipe the UI thread can poll.

pub mod aac;
pub mod aacstream;
pub mod audiofile;
pub mod codec;
pub mod convert;
#[cfg(target_os = "linux")]
pub mod dl;
pub mod demux;
pub mod id3;
pub mod ilst;
pub mod mp3;
pub mod metadata;
pub mod mp4;
pub mod movie;
pub mod route;
pub mod sbr;
mod sbr_tables;
pub mod source;
pub mod stream;
pub mod pcm_channels;
pub mod time;
pub mod symfile;
pub mod system_sound;
pub mod volume;

#[cfg(target_os = "linux")]
pub mod audio_out;
#[cfg(target_os = "linux")]
pub mod capture;
#[cfg(target_os = "linux")]
pub mod v4l2;
#[cfg(target_os = "linux")]
pub mod speaker;
pub mod audioplayer;
pub mod queue;
#[cfg(target_os = "linux")]
pub mod player;
#[cfg(target_os = "linux")]
pub use player::{Player, State};
/// Subtitle cues (SubRip and WebVTT) for the video player, and the cue on screen at a time.
pub mod subtitles;
/// Picture in picture geometry: the size, corner and snap of a shrunken picture.
pub mod pip;

use std::io;

/// Why a demux or decode step could not complete.
#[derive(Debug)]
pub enum Error {
    /// The bytes at `offset..offset + len` are not in the file yet.
    NeedData { offset: u64, len: u64 },
    /// The source is complete and the range lies past its end.
    Eof,
    /// Malformed input.
    Invalid(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NeedData { offset, len } => write!(f, "need {len} bytes at {offset}"),
            Error::Eof => write!(f, "end of file"),
            Error::Invalid(what) => write!(f, "invalid {what}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Where decoded frames go.
pub trait Sink {
    fn present(&mut self, frame: &crate::gfx::Image, pts_ms: f64);
}

/// A video stream codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

/// Pixels of a decoded picture.
pub enum Pixels {
    /// Planar luma plus interleaved chroma, both `stride` bytes per row.
    Nv12 { y: Vec<u8>, uv: Vec<u8>, stride: u32 },
    Rgba(crate::gfx::Image),
    /// Still in the decoder's output buffer (zero-copy, see `VideoDecoder::set_zero_copy`).
    Dma(DmaPicture),
}

/// A dma-buf the decoder wrote a picture into. Whoever holds an `Arc` of it keeps the
/// decoder from writing it again; the decoder reuses it once only its own reference is left.
pub trait DmaBuf: Send + Sync {
    /// The dma-buf fd (owned by the buffer; importers dup or reference it, never close it).
    fn fd(&self) -> i32;
    /// The CPU mapping, `len()` bytes.
    fn ptr(&self) -> *const u8;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Brackets CPU reads of a cached mapping (`end` = false before, true after).
    fn sync_cpu_read(&self, end: bool);
}

/// An NV12 picture inside a decoder buffer: the luma plane at offset 0, the interleaved chroma
/// at `uv_offset`, both `stride` bytes per row; `rows` luma rows are allocated. The visible
/// picture is the crop rectangle (`crop_left`/`crop_top` even).
#[derive(Clone)]
pub struct DmaPicture {
    pub buf: std::sync::Arc<dyn DmaBuf>,
    /// Identifies the buffer for the lifetime of the process (GPU import caches key on it).
    pub id: u64,
    /// Bumped for every picture the decoder emits: a new value means new pixels.
    pub seq: u64,
    pub stride: u32,
    pub rows: u32,
    pub uv_offset: u32,
    pub crop_left: u32,
    pub crop_top: u32,
    pub width: u32,
    pub height: u32,
}

/// A picture handed from a video thread to the UI: converted for the CPU, or left in the
/// decoder's buffer for the GPU (converted on the UI thread only if a CPU frame needs it).
pub enum Picture {
    Rgba(crate::gfx::Image),
    Gpu(DmaPicture),
}

impl Picture {
    /// The picture as RGBA fitted into `view` (`w`, `h`): as is for `Rgba`, converted for `Gpu`.
    pub fn into_rgba(self, view: (u32, u32)) -> crate::gfx::Image {
        match self {
            Picture::Rgba(img) => img,
            Picture::Gpu(p) => {
                let (ow, oh) = convert::fit(p.width, p.height, view.0, view.1);
                convert::dma_to_rgba(&p, ow, oh, convert::matrix_for(p.height), 3)
            }
        }
    }
}

/// A fullscreen app's video for the GPU: the decoder picture and where it goes, in pixels of
/// the shell's logical frame. An app that hands one out draws only its overlay for that frame
/// (transparent, premultiplied zero, where the picture shows); see `Shell::video_layer`.
#[derive(Clone)]
pub struct VideoLayer {
    pub frame: DmaPicture,
    pub dst: crate::layout::Rect,
}

impl DmaPicture {
    /// Byte ranges (start, len) of the cropped luma and chroma planes inside the buffer;
    /// None when they do not fit (a corrupt description).
    pub fn plane_ranges(&self) -> Option<((usize, usize), (usize, usize))> {
        let (stride, w, h) = (self.stride as usize, self.width as usize, self.height as usize);
        if w == 0 || h == 0 || stride < ((w + 1) & !1) + self.crop_left as usize {
            return None;
        }
        let y0 = self.crop_top as usize * stride + self.crop_left as usize;
        let y_len = stride * (h - 1) + w;
        let uv_rows = h.div_ceil(2);
        let uv0 = self.uv_offset as usize + (self.crop_top as usize / 2) * stride + self.crop_left as usize;
        let uv_len = stride * (uv_rows - 1) + ((w + 1) & !1);
        let end = (y0 + y_len).max(uv0 + uv_len);
        (end <= self.buf.len()).then_some(((y0, y_len), (uv0, uv_len)))
    }

    /// Runs `f` on the cropped planes (luma, chroma, stride) read through the CPU mapping;
    /// None when the description does not fit the buffer.
    pub fn with_nv12<R>(&self, f: impl FnOnce(&[u8], &[u8], usize) -> R) -> Option<R> {
        let ((y0, yl), (u0, ul)) = self.plane_ranges()?;
        self.buf.sync_cpu_read(false);
        // SAFETY: plane_ranges checked both ranges lie inside the mapping, which lives as long as
        // `buf`; the decoder does not write a buffer while this picture holds a reference to it.
        let r = unsafe {
            let base = self.buf.ptr();
            f(std::slice::from_raw_parts(base.add(y0), yl), std::slice::from_raw_parts(base.add(u0), ul), self.stride as usize)
        };
        self.buf.sync_cpu_read(true);
        Some(r)
    }
}

/// A decoded picture in display order.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pts_us: i64,
    pub pixels: Pixels,
}

/// The hardware decoder, implemented in `hwdec` over the vendor OpenMAX
/// libraries. The player calls it from its video thread.
///
/// `configure` receives the codec's parameter sets as the raw `avcC` /
/// `hvcC` box payload in `extradata`; `push` receives one access unit in
/// Annex B form (4-byte start codes), with the SPS/PPS prepended as Annex
/// B before the first push and after every seek.
pub trait VideoDecoder: Send {
    fn configure(&mut self, codec: Codec, extradata: &[u8], width: u32, height: u32) -> io::Result<()>;
    fn push(&mut self, nal_units: &[u8], pts_us: i64) -> io::Result<()>;
    fn pull(&mut self) -> io::Result<Option<Frame>>;
    fn flush(&mut self) -> io::Result<()>;
    /// Asks for `Pixels::Dma` frames that stay in the decoder's buffers (for the GPU) instead
    /// of CPU copies; returns whether the decoder does that. Frames already queued keep their form.
    fn set_zero_copy(&mut self, on: bool) -> bool {
        let _ = on;
        false
    }
    /// What is doing the decoding, for diagnostics.
    fn describe(&self) -> String {
        "video decoder".into()
    }
}

/// The decoder `Player::open` uses: libavcodec / VA-API through `hwdec`.
pub fn default_decoder(codec: Codec) -> io::Result<Box<dyn VideoDecoder>> {
    hwdec::open(codec)
}

pub mod hwdec;

#[cfg(test)]
mod dma_tests {
    use super::*;
    use std::sync::Arc;

    struct Mem(Vec<u8>);
    impl DmaBuf for Mem {
        fn fd(&self) -> i32 {
            -1
        }
        fn ptr(&self) -> *const u8 {
            self.0.as_ptr()
        }
        fn len(&self) -> usize {
            self.0.len()
        }
        fn sync_cpu_read(&self, _end: bool) {}
    }

    fn pic(stride: u32, rows: u32, crop: (u32, u32, u32, u32)) -> DmaPicture {
        let bytes = (stride * rows * 3 / 2) as usize;
        let mut v = vec![0u8; bytes];
        for (i, b) in v.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        DmaPicture { buf: Arc::new(Mem(v)), id: 1, seq: 1, stride, rows, uv_offset: stride * rows, crop_left: crop.0, crop_top: crop.1, width: crop.2, height: crop.3 }
    }

    #[test]
    fn plane_ranges_follow_the_crop() {
        let p = pic(1280, 736, (0, 0, 1280, 720));
        let ((y0, yl), (u0, ul)) = p.plane_ranges().unwrap();
        assert_eq!(y0, 0);
        assert_eq!(yl, 1280 * 719 + 1280);
        assert_eq!(u0, 1280 * 736);
        assert_eq!(ul, 1280 * 359 + 1280);
        let p = pic(1920, 1088, (2, 4, 1916, 1080));
        let ((y0, _), (u0, _)) = p.plane_ranges().unwrap();
        assert_eq!(y0, 4 * 1920 + 2);
        assert_eq!(u0, 1920 * 1088 + 2 * 1920 + 2);
    }

    #[test]
    fn plane_ranges_reject_what_does_not_fit() {
        assert!(pic(64, 32, (0, 0, 64, 40)).plane_ranges().is_none(), "taller than the buffer");
        assert!(pic(64, 32, (0, 0, 0, 16)).plane_ranges().is_none(), "empty");
        assert!(pic(64, 32, (2, 0, 64, 16)).plane_ranges().is_none(), "wider than the stride");
    }

    #[test]
    fn with_nv12_hands_out_the_cropped_planes() {
        let p = pic(16, 8, (2, 2, 8, 4));
        let (y, uv, stride) = p.with_nv12(|y, uv, s| (y[0], uv[0], s)).unwrap();
        assert_eq!(stride, 16);
        assert_eq!(y as usize, (2 * 16 + 2) % 251);
        assert_eq!(uv as usize, (16 * 8 + 16 + 2) % 251);
    }
}
