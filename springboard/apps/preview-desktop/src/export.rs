//! Export of a page as a PNG or JPEG picture: the page is scaled, adjusted, turned and has its markup burned in, then
//! written beside the document as `<name>-markup.<ext>`. A PDF page is drawn by the worker, so its export runs there
//! too; a picture is written at once.

use crate::document::adjusted;
use crate::model::{self, Adjust, Mark};
use crate::pdfjob::{self, Job, Pdf, Reply, DOCUMENT_RENDER_BOUNDS};
use crate::pixels;
use bifrost_app::kit::Image;
use springboard::cg::imageio::encode::{encode_png, PngOptions};
use springboard::cg::pdf::draw;
use springboard::shell::camera::jpeg;
use std::path::{Path, PathBuf};

/// The quality of an exported JPEG unless File > JPEG Quality says otherwise.
pub const JPEG_QUALITY: u8 = 90;

/// The picture formats an export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raster {
    Png,
    /// A JPEG at this quality, 1 to 100.
    Jpeg(u8),
}

impl Raster {
    pub fn ext(self) -> &'static str {
        match self {
            Raster::Png => "png",
            Raster::Jpeg(_) => "jpg",
        }
    }
}

/// The file an export or a PDF save of `path` is written to: `<name>-markup.<ext>` beside it.
pub fn target(path: &Path, ext: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    path.with_file_name(format!("{stem}-markup.{ext}"))
}

/// The picture an export writes: `img` scaled by `scale`, adjusted, turned `turns` quarters and with the markup of
/// source page `src` burned in.
pub fn finish(img: &Image, marks: &[Mark], src: usize, turns: u8, adjust: &Adjust, scale: f32) -> Image {
    let scaled = if (scale - 1.0).abs() < 1e-3 {
        img.clone()
    } else {
        let w = ((img.w as f32 * scale).round() as usize).max(1);
        let h = ((img.h as f32 * scale).round() as usize).max(1);
        img.resized(w, h)
    };
    let mut out = model::rotate(&adjusted(&scaled, adjust), turns);
    model::burn(&mut out, marks, src, turns);
    out
}

/// The picture as PNG bytes, transparency kept. None when it cannot be encoded.
pub fn png_bytes(img: &Image) -> Option<Vec<u8>> {
    encode_png(img.w, img.h, &pixels::to_straight_rgba(img), &PngOptions::default())
}

/// Encodes `img` and writes it to `target`. A PNG keeps transparency; a JPEG has none, so a transparent picture is laid
/// over white first.
pub fn write_image(img: &Image, raster: Raster, target: &Path) -> Result<(), String> {
    let unencoded = || "The picture could not be encoded.".to_string();
    let bytes = match raster {
        Raster::Png => png_bytes(img).ok_or_else(unencoded)?,
        Raster::Jpeg(quality) => {
            let flat;
            let opaque = if pixels::has_alpha(img) {
                flat = pixels::flatten_on_white(img);
                &flat
            } else {
                img
            };
            let encoded = jpeg::encode_pixels(opaque.w, opaque.h, &opaque.px, quality.clamp(1, 100), None);
            if encoded.is_empty() {
                return Err(unencoded());
            }
            encoded
        }
    };
    std::fs::write(target, bytes).map_err(|e| e.to_string())
}

/// A job that exports page `src` of a PDF: the page is drawn at `scale` of the window size, then finished and written.
pub fn pdf_page_job(src: usize, turns: u8, marks: Vec<Mark>, adjust: Adjust, scale: f32, raster: Raster, target: PathBuf) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let result = (|| -> Result<PathBuf, String> {
            let bw = ((DOCUMENT_RENDER_BOUNDS.0 as f32 * scale).round() as usize).max(1);
            let bh = ((DOCUMENT_RENDER_BOUNDS.1 as f32 * scale).round() as usize).max(1);
            let page = pdf.pages.get(src).ok_or("This page is missing.")?;
            let img = draw::render_page_fit(&pdf.doc, page, bw, bh).ok_or("The page could not be drawn.")?;
            let out = finish(&img, &marks, src, turns, &adjust, 1.0);
            write_image(&out, raster, &target)?;
            Ok(target.clone())
        })();
        Reply::Written(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Norm, MarkKind, MARK_BLUE};

    #[test]
    fn exports_are_named_beside_the_document() {
        assert_eq!(target(Path::new("/a/b.pdf"), "pdf"), PathBuf::from("/a/b-markup.pdf"));
        assert_eq!(target(Path::new("/a/b.png"), Raster::Jpeg(JPEG_QUALITY).ext()), PathBuf::from("/a/b-markup.jpg"));
        assert_eq!(target(Path::new("/a/b.png"), Raster::Png.ext()), PathBuf::from("/a/b-markup.png"));
    }

    #[test]
    fn finish_scales_turns_and_burns_the_markup() {
        let img = Image { w: 4, h: 2, px: vec![0xff00_0000; 8] };
        let mark = Mark { kind: MarkKind::Rect, page: 0, rect: Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, color: MARK_BLUE, text: String::new(), ink: Vec::new() };
        let out = finish(&img, std::slice::from_ref(&mark), 0, 1, &Adjust::default(), 0.5);
        // half size is 2 by 1, and a quarter turn makes it 1 by 2
        assert_eq!((out.w, out.h), (1, 2));
        assert_ne!(out.px[0], 0xff00_0000);
    }

    #[test]
    fn a_png_keeps_transparency_and_a_jpeg_is_written_flattened() {
        let dir = std::env::temp_dir().join(format!("viewer-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let img = Image { w: 2, h: 1, px: vec![0, 0xff00_00ff] };
        let png = dir.join("a.png");
        write_image(&img, Raster::Png, &png).expect("a png");
        let bytes = std::fs::read(&png).expect("the file");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR colour type 6 is RGBA
        assert_eq!(bytes[25], 6);
        let jpg = dir.join("a.jpg");
        write_image(&img, Raster::Jpeg(JPEG_QUALITY), &jpg).expect("a jpeg");
        assert_eq!(&std::fs::read(&jpg).expect("the file")[..2], &[0xff, 0xd8]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lower_jpeg_quality_writes_a_smaller_file() {
        let dir = std::env::temp_dir().join(format!("viewer-quality-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        // a deterministic noisy picture, which a JPEG cannot squeeze without losing quality
        let mut seed = 12345u32;
        let px = (0..48 * 48)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                0xff00_0000 | (seed >> 8)
            })
            .collect();
        let img = Image { w: 48, h: 48, px };
        let size = |quality: u8| {
            let path = dir.join(format!("q{quality}.jpg"));
            write_image(&img, Raster::Jpeg(quality), &path).expect("a jpeg");
            std::fs::metadata(&path).expect("the file").len()
        };
        assert!(size(30) < size(95));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_at_full_scale_is_unchanged_without_markup() {
        let img = Image { w: 3, h: 2, px: (0..6).map(|i| 0xff00_0000 | i).collect() };
        assert_eq!(finish(&img, &[], 0, 0, &Adjust::default(), 1.0), img);
    }
}
