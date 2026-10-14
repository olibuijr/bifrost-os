//! Export of a page as a PNG or JPEG picture: the page is scaled, adjusted, turned and has its markup burned in, then
//! written beside the document as `<name>-markup.<ext>`. A PDF page is drawn by the worker, so its export runs there
//! too; a picture is written at once.

use crate::document::adjusted;
use crate::model::{self, Adjust, Mark};
use crate::pdfjob::{self, Job, Pdf, Reply, PAGE_BOX};
use bifrost_app::kit::Image;
use springboard::cg::pdf::draw;
use springboard::shell::camera::jpeg;
use std::path::{Path, PathBuf};

/// The picture formats an export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raster {
    Png,
    Jpeg,
}

impl Raster {
    pub fn ext(self) -> &'static str {
        match self {
            Raster::Png => "png",
            Raster::Jpeg => "jpg",
        }
    }
}

/// The format a file is written in when it is saved over `path`: JPEG for a `.jpg` or `.jpeg`, otherwise PNG.
pub fn raster_for(path: &Path) -> Raster {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg") | Some("jpeg") => Raster::Jpeg,
        _ => Raster::Png,
    }
}

/// Where the file as it was before a save is kept: `<data_dir>/backups/<name>.<stamp>.bak`.
pub fn backup_path(data_dir: &Path, path: &Path, stamp: u64) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("document");
    data_dir.join("backups").join(format!("{name}.{stamp}.bak"))
}

/// Copies `path` to its backup place before it is written over. The backups folder is made when needed.
pub fn back_up(data_dir: &Path, path: &Path) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let target = backup_path(data_dir, path, stamp);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::copy(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// The file an export or a PDF save of `path` is written to: `<name>-markup.<ext>` beside it.
pub fn target(path: &Path, ext: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    path.with_file_name(format!("{stem}-markup.{ext}"))
}

/// The picture an export writes: `img` scaled by `scale`, adjusted, oriented by `turns` (quarter turns plus mirror
/// bits) and with the markup of source page `src` burned in.
pub fn finish(img: &Image, marks: &[Mark], src: usize, turns: u8, adjust: &Adjust, scale: f32) -> Image {
    let scaled = if (scale - 1.0).abs() < 1e-3 {
        img.clone()
    } else {
        let w = ((img.w as f32 * scale).round() as usize).max(1);
        let h = ((img.h as f32 * scale).round() as usize).max(1);
        img.resized(w, h)
    };
    let mut out = model::orient(&adjusted(&scaled, adjust), turns);
    model::burn(&mut out, marks, src, turns);
    out
}

/// Encodes `img` and writes it to `target`. `quality` (1 to 100) is the JPEG quality and is ignored for PNG.
pub fn write_image(img: &Image, raster: Raster, target: &Path, quality: u8) -> Result<(), String> {
    let bytes = match raster {
        Raster::Png => springboard::pngenc::encode_rgb(img.w, img.h, &img.px),
        Raster::Jpeg => {
            let encoded = jpeg::encode_pixels(img.w, img.h, &img.px, quality.clamp(1, 100), None);
            if encoded.is_empty() {
                return Err("The picture could not be encoded.".into());
            }
            encoded
        }
    };
    std::fs::write(target, bytes).map_err(|e| e.to_string())
}

/// A job that exports page `src` of a PDF: the page is drawn at `scale` of the window size, then finished and written.
pub fn pdf_page_job(src: usize, turns: u8, marks: Vec<Mark>, adjust: Adjust, scale: f32, raster: Raster, quality: u8, target: PathBuf) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let result = (|| -> Result<PathBuf, String> {
            let bw = ((PAGE_BOX.0 as f32 * scale).round() as usize).max(1);
            let bh = ((PAGE_BOX.1 as f32 * scale).round() as usize).max(1);
            let page = pdf.pages.get(src).ok_or("This page is missing.")?;
            let img = draw::render_page_fit(&pdf.doc, page, bw, bh).ok_or("The page could not be drawn.")?;
            let out = finish(&img, &marks, src, turns, &adjust, 1.0);
            write_image(&out, raster, &target, quality)?;
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
        assert_eq!(target(Path::new("/a/b.png"), Raster::Jpeg.ext()), PathBuf::from("/a/b-markup.jpg"));
        assert_eq!(target(Path::new("/a/b.png"), Raster::Png.ext()), PathBuf::from("/a/b-markup.png"));
    }

    #[test]
    fn saving_over_a_file_uses_its_own_format_and_keeps_a_backup_name() {
        assert_eq!(raster_for(Path::new("/a/b.JPG")), Raster::Jpeg);
        assert_eq!(raster_for(Path::new("/a/b.jpeg")), Raster::Jpeg);
        assert_eq!(raster_for(Path::new("/a/b.png")), Raster::Png);
        assert_eq!(backup_path(Path::new("/data"), Path::new("/a/b.png"), 42), PathBuf::from("/data/backups/b.png.42.bak"));
    }

    #[test]
    fn back_up_copies_the_file() {
        let dir = std::env::temp_dir().join(format!("viewer-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("doc.png");
        std::fs::write(&file, b"pixels").expect("written");
        let kept = back_up(&dir, &file).expect("a backup");
        assert_eq!(std::fs::read(&kept).expect("read"), b"pixels");
        assert!(back_up(&dir, &dir.join("missing.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mirrored_export_has_the_picture_reversed() {
        let img = Image { w: 2, h: 1, px: vec![1, 2] };
        let out = finish(&img, &[], 0, model::FLIP_H, &Adjust::default(), 1.0);
        assert_eq!(out.px, vec![2, 1]);
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
    fn a_picture_at_full_scale_is_unchanged_without_markup() {
        let img = Image { w: 3, h: 2, px: (0..6).map(|i| 0xff00_0000 | i).collect() };
        assert_eq!(finish(&img, &[], 0, 0, &Adjust::default(), 1.0), img);
    }
}
