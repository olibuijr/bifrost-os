//! Hardware video decoding.
//!
//! libavcodec is `dlopen`ed (`ffmpeg.rs`): VA-API hardware decode on the DRM render node,
//! software when that is missing (`vaapi.rs` probes libva). The pure parts (NAL plumbing in
//! `nal.rs`, NV12 to RGBA here) are tested on the host.

pub mod nal;

#[cfg(target_os = "linux")]
pub mod ffmpeg;
#[cfg(target_os = "linux")]
pub mod vaapi;

use crate::gfx::{pack, Image};
use std::io;

pub use crate::media::{Codec, Frame, Pixels, VideoDecoder};

/// Opens the hardware decoder, or says why there is none.
pub fn open(codec: Codec) -> io::Result<Box<dyn VideoDecoder>> {
    // VA-API through libavcodec, software when the GPU cannot
    #[cfg(target_os = "linux")]
    {
        ffmpeg::Decoder::open(codec).map(|d| Box::new(d) as Box<dyn VideoDecoder>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = codec;
        Err(io::Error::new(io::ErrorKind::Unsupported, "video decoding needs Linux"))
    }
}

impl Frame {
    /// The frame as premultiplied RGBA (BT.601 limited range for NV12).
    pub fn to_rgba(&self) -> Image {
        match &self.pixels {
            Pixels::Rgba(img) => img.clone(),
            Pixels::Nv12 { y, uv, stride } => nv12_to_rgba(y, uv, *stride as usize, self.width as usize, self.height as usize),
            Pixels::Dma(p) => p.with_nv12(|y, uv, stride| nv12_to_rgba(y, uv, stride, p.width as usize, p.height as usize)).unwrap_or_else(|| Image::new(1, 1)),
        }
    }
}

pub fn nv12_to_rgba(y: &[u8], uv: &[u8], stride: usize, w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for row in 0..h {
        let yr = &y[row * stride..row * stride + w];
        let ur = &uv[(row / 2) * stride..(row / 2) * stride + ((w + 1) & !1)];
        for (x, px) in img.px[row * w..(row + 1) * w].iter_mut().enumerate() {
            *px = yuv_to_rgba(yr[x], ur[x & !1], ur[(x & !1) + 1]);
        }
    }
    img
}

#[inline]
fn yuv_to_rgba(y: u8, u: u8, v: u8) -> u32 {
    let c = 298 * (y as i32 - 16);
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    let clamp = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
    pack(clamp(c + 409 * e), clamp(c - 100 * d - 208 * e), clamp(c + 516 * d), 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuv_limited_range_maps_black_white_and_red() {
        assert_eq!(yuv_to_rgba(16, 128, 128), pack(0, 0, 0, 255));
        assert_eq!(yuv_to_rgba(235, 128, 128), pack(255, 255, 255, 255));
        let red = yuv_to_rgba(81, 90, 240);
        assert_eq!(red & 0xff, 255);
        assert!((red >> 8) & 0xff < 10 && (red >> 16) & 0xff < 10, "{red:#x}");
    }

    #[test]
    fn nv12_conversion_honours_stride_and_odd_width() {
        let (stride, w, h) = (8usize, 3usize, 3usize);
        let mut y = vec![16u8; stride * h];
        y[stride * 2 + 2] = 235; // bottom-right pixel white
        let uv = vec![128u8; stride * 2];
        let img = nv12_to_rgba(&y, &uv, stride, w, h);
        assert_eq!((img.w, img.h), (w, h));
        assert_eq!(img.px[0], pack(0, 0, 0, 255));
        assert_eq!(img.px[2 * w + 2], pack(255, 255, 255, 255));
    }
}
