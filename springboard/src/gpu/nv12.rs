//! Importing a decoder picture (NV12 in one dma-buf) as an EGL image: the attribute list for
//! `eglCreateImageKHR(EGL_LINUX_DMA_BUF_EXT)` from `EGL_EXT_image_dma_buf_import`, the size the
//! image is imported at and the crop as texture coordinates. No GPU needed.

use crate::layout::Rect;
use crate::media::DmaPicture;

/// DRM_FORMAT_NV12 ('NV12'): a Y plane and an interleaved CbCr plane at half resolution.
pub const FOURCC_NV12: u32 = 0x3231_564E;

// EGL_EXT_image_dma_buf_import (eglext.h)
pub const EGL_WIDTH: i32 = 0x3057;
pub const EGL_HEIGHT: i32 = 0x3056;
pub const EGL_NONE: i32 = 0x3038;
pub const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
pub const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
pub const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
pub const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
pub const EGL_DMA_BUF_PLANE1_FD_EXT: i32 = 0x3275;
pub const EGL_DMA_BUF_PLANE1_OFFSET_EXT: i32 = 0x3276;
pub const EGL_DMA_BUF_PLANE1_PITCH_EXT: i32 = 0x3277;
pub const EGL_YUV_COLOR_SPACE_HINT_EXT: i32 = 0x327B;
pub const EGL_SAMPLE_RANGE_HINT_EXT: i32 = 0x327C;
pub const EGL_YUV_CHROMA_HORIZONTAL_SITING_HINT_EXT: i32 = 0x327D;
pub const EGL_YUV_CHROMA_VERTICAL_SITING_HINT_EXT: i32 = 0x327E;
pub const EGL_ITU_REC601_EXT: i32 = 0x327F;
pub const EGL_ITU_REC709_EXT: i32 = 0x3280;
pub const EGL_YUV_NARROW_RANGE_EXT: i32 = 0x3283;
pub const EGL_YUV_CHROMA_SITING_0_EXT: i32 = 0x3284;
pub const EGL_YUV_CHROMA_SITING_0_5_EXT: i32 = 0x3285;

/// HD material is BT.709, SD BT.601: the same split as the CPU path (`convert::matrix_for`).
pub fn color_space(height: u32) -> i32 {
    if height >= 720 {
        EGL_ITU_REC709_EXT
    } else {
        EGL_ITU_REC601_EXT
    }
}

/// The size the picture is imported at: up to the crop's far edges (so clamping at the image
/// border never samples the decoder's padding), the height even for the half-height chroma,
/// within the allocated rows. None when the picture does not describe a usable image.
pub fn image_size(p: &DmaPicture) -> Option<(u32, u32)> {
    let w = p.crop_left.checked_add(p.width)?;
    let h = (p.crop_top.checked_add(p.height)? + 1) & !1;
    if p.width == 0 || p.height == 0 || w > p.stride || h > p.rows {
        return None;
    }
    Some((w, h))
}

/// The visible picture as `[u0, v0, u1, v1]` inside the imported image.
pub fn crop_uv(p: &DmaPicture) -> Option<[f32; 4]> {
    let (w, h) = image_size(p)?;
    let crop = Rect { x: p.crop_left as i32, y: p.crop_top as i32, w: p.width as i32, h: p.height as i32 };
    Some(super::texture::uv(crop, w as usize, h as usize))
}

/// The attribute list for one NV12 picture (both planes in the same dma-buf `fd`), EGL_NONE
/// terminated. `siting` adds the MPEG-2 chroma siting hints (co-sited across, between rows
/// down), which some drivers refuse; the caller retries without them.
pub fn image_attribs(p: &DmaPicture, fd: i32, siting: bool) -> Option<Vec<i32>> {
    let (w, h) = image_size(p)?;
    let pitch = p.stride as i32;
    let mut a = vec![
        EGL_WIDTH,
        w as i32,
        EGL_HEIGHT,
        h as i32,
        EGL_LINUX_DRM_FOURCC_EXT,
        FOURCC_NV12 as i32,
        EGL_DMA_BUF_PLANE0_FD_EXT,
        fd,
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        0,
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        pitch,
        EGL_DMA_BUF_PLANE1_FD_EXT,
        fd,
        EGL_DMA_BUF_PLANE1_OFFSET_EXT,
        p.uv_offset as i32,
        EGL_DMA_BUF_PLANE1_PITCH_EXT,
        pitch,
        EGL_YUV_COLOR_SPACE_HINT_EXT,
        color_space(p.height),
        EGL_SAMPLE_RANGE_HINT_EXT,
        EGL_YUV_NARROW_RANGE_EXT,
    ];
    if siting {
        a.extend([EGL_YUV_CHROMA_HORIZONTAL_SITING_HINT_EXT, EGL_YUV_CHROMA_SITING_0_EXT, EGL_YUV_CHROMA_VERTICAL_SITING_HINT_EXT, EGL_YUV_CHROMA_SITING_0_5_EXT]);
    }
    a.push(EGL_NONE);
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::DmaBuf;
    use std::sync::Arc;

    struct Fake(usize);
    impl DmaBuf for Fake {
        fn fd(&self) -> i32 {
            42
        }
        fn ptr(&self) -> *const u8 {
            std::ptr::null()
        }
        fn len(&self) -> usize {
            self.0
        }
        fn sync_cpu_read(&self, _end: bool) {}
    }

    fn pic(stride: u32, rows: u32, crop: (u32, u32, u32, u32)) -> DmaPicture {
        let len = (stride * rows * 3 / 2) as usize;
        DmaPicture { buf: Arc::new(Fake(len)), id: 3, seq: 1, stride, rows, uv_offset: stride * rows, crop_left: crop.0, crop_top: crop.1, width: crop.2, height: crop.3 }
    }

    /// The value after `key` in an attribute list.
    fn get(a: &[i32], key: i32) -> Option<i32> {
        a[..a.len() - 1].chunks(2).find(|kv| kv[0] == key).map(|kv| kv[1])
    }

    #[test]
    fn nv12_is_the_drm_fourcc() {
        assert_eq!(&FOURCC_NV12.to_le_bytes(), b"NV12");
    }

    #[test]
    fn attribs_describe_both_planes_of_one_buffer() {
        let p = pic(1280, 736, (0, 0, 1280, 720));
        let a = image_attribs(&p, 9, false).unwrap();
        assert_eq!(a.len() % 2, 1);
        assert_eq!(*a.last().unwrap(), EGL_NONE);
        assert_eq!(get(&a, EGL_WIDTH), Some(1280));
        assert_eq!(get(&a, EGL_HEIGHT), Some(720));
        assert_eq!(get(&a, EGL_LINUX_DRM_FOURCC_EXT), Some(0x3231_564E));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE0_FD_EXT), Some(9));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE0_OFFSET_EXT), Some(0));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE0_PITCH_EXT), Some(1280));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE1_FD_EXT), Some(9));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE1_OFFSET_EXT), Some(1280 * 736));
        assert_eq!(get(&a, EGL_DMA_BUF_PLANE1_PITCH_EXT), Some(1280));
        assert_eq!(get(&a, EGL_YUV_COLOR_SPACE_HINT_EXT), Some(EGL_ITU_REC709_EXT));
        assert_eq!(get(&a, EGL_SAMPLE_RANGE_HINT_EXT), Some(EGL_YUV_NARROW_RANGE_EXT));
        assert_eq!(get(&a, EGL_YUV_CHROMA_VERTICAL_SITING_HINT_EXT), None);
    }

    #[test]
    fn siting_hints_are_optional() {
        let p = pic(1280, 736, (0, 0, 1280, 720));
        let a = image_attribs(&p, 9, true).unwrap();
        assert_eq!(get(&a, EGL_YUV_CHROMA_HORIZONTAL_SITING_HINT_EXT), Some(EGL_YUV_CHROMA_SITING_0_EXT));
        assert_eq!(get(&a, EGL_YUV_CHROMA_VERTICAL_SITING_HINT_EXT), Some(EGL_YUV_CHROMA_SITING_0_5_EXT));
        assert_eq!(image_attribs(&p, 9, false).unwrap().len() + 4, a.len());
    }

    #[test]
    fn the_extension_constants_match_eglext_h() {
        assert_eq!([EGL_DMA_BUF_PLANE1_FD_EXT, EGL_DMA_BUF_PLANE1_OFFSET_EXT, EGL_DMA_BUF_PLANE1_PITCH_EXT], [0x3275, 0x3276, 0x3277]);
        assert_eq!([EGL_YUV_COLOR_SPACE_HINT_EXT, EGL_SAMPLE_RANGE_HINT_EXT], [0x327B, 0x327C]);
        assert_eq!([EGL_ITU_REC601_EXT, EGL_ITU_REC709_EXT, EGL_YUV_NARROW_RANGE_EXT], [0x327F, 0x3280, 0x3283]);
        assert_eq!(EGL_LINUX_DRM_FOURCC_EXT, super::super::egl::EGL_LINUX_DRM_FOURCC_EXT);
        assert_eq!(EGL_DMA_BUF_PLANE0_PITCH_EXT, super::super::egl::EGL_DMA_BUF_PLANE0_PITCH_EXT);
    }

    #[test]
    fn colour_space_by_height() {
        assert_eq!(color_space(480), EGL_ITU_REC601_EXT);
        assert_eq!(color_space(576), EGL_ITU_REC601_EXT);
        assert_eq!(color_space(720), EGL_ITU_REC709_EXT);
        assert_eq!(color_space(1080), EGL_ITU_REC709_EXT);
        assert_eq!(get(&image_attribs(&pic(640, 480, (0, 0, 640, 480)), 1, false).unwrap(), EGL_YUV_COLOR_SPACE_HINT_EXT), Some(EGL_ITU_REC601_EXT));
    }

    #[test]
    fn the_image_reaches_the_crop_edges_and_the_uvs_cut_the_rest() {
        let p = pic(1920, 1088, (0, 0, 1920, 1080));
        assert_eq!(image_size(&p), Some((1920, 1080)));
        assert_eq!(crop_uv(&p), Some([0.0, 0.0, 1.0, 1.0]));
        let p = pic(1920, 1088, (8, 4, 1904, 1075));
        assert_eq!(image_size(&p), Some((1912, 1080)), "odd heights round up to whole chroma rows");
        let uv = crop_uv(&p).unwrap();
        assert!((uv[0] - 8.0 / 1912.0).abs() < 1e-6 && (uv[1] - 4.0 / 1080.0).abs() < 1e-6);
        assert!((uv[2] - 1.0).abs() < 1e-6 && (uv[3] - 1079.0 / 1080.0).abs() < 1e-6);
    }

    #[test]
    fn impossible_pictures_are_refused() {
        assert_eq!(image_size(&pic(1280, 720, (0, 0, 1290, 720))), None, "wider than the pitch");
        assert_eq!(image_size(&pic(1280, 720, (0, 0, 1280, 722))), None, "taller than the rows");
        assert_eq!(image_size(&pic(1280, 720, (0, 0, 0, 720))), None);
        assert!(image_attribs(&pic(1280, 720, (0, 0, 0, 0)), 1, true).is_none());
    }
}
