//! Picture thumbnails: images, the thumbnail an office document carries inside its zip, and the fonts' sample card.

use super::canvas::{Canvas, INK, MUTED};
use crate::files::preview::{decode_allowed, image_size, MAX_IMAGE_BYTES};
use crate::files::zip::Archive;
use crate::font::{Font, Style};
use bifrost_render::gfx::Image;
use bifrost_render::ui::Weight;
use bifrost_ui::thumb_service::{Pixels, SourceFile, ThumbArt, ThumbProvider};
use std::io::Read;

/// Largest embedded thumbnail read out of a document.
const EMBEDDED_MAX: u64 = 8 << 20;
/// Largest font file read for a sample.
const FONT_MAX: u64 = 32 << 20;

/// An image as thumbnail pixels.
pub fn pixels_of(img: &Image) -> Option<Pixels> {
    Pixels::new(img.w, img.h, img.px.clone())
}

/// Decodes a PNG or JPEG and scales it down to fit a square of `px` pixels (never enlarged). The header is read first,
/// so a picture too large to decode is refused before any pixel is allocated.
pub fn decode_fitted(bytes: &[u8], px: u32) -> Option<Pixels> {
    if let Some((w, h)) = image_size(bytes) {
        if !decode_allowed(w, h, bytes.len() as u64) {
            return None;
        }
    }
    let img = crate::cg::imageio::decode(bytes)?;
    let side = px as usize;
    pixels_of(&crate::imgdec::contain(&img, side, side))
}

/// Pictures the decoder reads (PNG and JPEG); other image types fall back to their icon.
pub struct ImageProvider;

impl ThumbProvider for ImageProvider {
    fn name(&self) -> &'static str {
        "image"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.image"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        if file.stamp.size > MAX_IMAGE_BYTES {
            return None;
        }
        let bytes = std::fs::read(&file.path).ok()?;
        decode_fitted(&bytes, px).map(ThumbArt::Pixels)
    }
}

/// The paths a document keeps its preview picture at: the XML formats and the OpenDocument formats.
const EMBEDDED: [&str; 5] = ["docProps/thumbnail.jpeg", "docProps/thumbnail.jpg", "docProps/thumbnail.png", "Thumbnails/thumbnail.png", "Thumbnails/thumbnail.jpeg"];

/// The embedded preview picture of an office document archive, if it has one.
pub fn embedded_preview<R: std::io::Read + std::io::Seek>(r: R) -> Option<Vec<u8>> {
    let mut zip = Archive::open(r).ok()?;
    let m = zip.members.iter().find(|m| !m.is_dir && EMBEDDED.contains(&m.name.as_str()))?.clone();
    if m.size > EMBEDDED_MAX {
        return None;
    }
    zip.read(&m).ok()
}

/// Office documents (word processing, spreadsheet, presentation; both XML and OpenDocument families): the picture the
/// document stores for its first page. Without one the type icon with the extension badge is used.
pub struct OfficeProvider;

impl ThumbProvider for OfficeProvider {
    fn name(&self) -> &'static str {
        "office"
    }
    fn types(&self) -> &'static [&'static str] {
        &[
            "org.openxmlformats.wordprocessingml.document",
            "org.openxmlformats.spreadsheetml.sheet",
            "org.openxmlformats.presentationml.presentation",
            "org.oasis-open.opendocument.text",
            "org.oasis-open.opendocument.spreadsheet",
            "org.oasis-open.opendocument.presentation",
        ]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let bytes = embedded_preview(std::fs::File::open(&file.path).ok()?)?;
        decode_fitted(&bytes, px).map(ThumbArt::Pixels)
    }
}

/// Fonts: a card with a large "Aa" and a line of sample glyphs, drawn with the font itself.
pub struct FontProvider;

impl ThumbProvider for FontProvider {
    fn name(&self) -> &'static str {
        "font"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.font"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let mut data = Vec::new();
        std::fs::File::open(&file.path).ok()?.take(FONT_MAX).read_to_end(&mut data).ok()?;
        font_card(&data, &file.name(), px)
    }
}

/// The sample card for font bytes. `None` when the font does not parse or has no glyph for "A".
pub fn font_card(data: &[u8], file_name: &str, px: u32) -> Option<ThumbArt> {
    let font = Font::parse(data)?;
    if font.glyph('A') == 0 {
        return None;
    }
    let h = px as usize;
    let w = ((px as f32 * 200.0 / 260.0).round() as usize).max(1);
    let mut canvas = Canvas::new(w, h, 200.0)?;
    canvas.page(6.0);
    let s = canvas.s;
    let white = 0xFFFF_FFFF;
    // a glyph the font lacks would draw as an empty box, so the line keeps only what it has
    let has = |c: char| font.glyph(c) != 0;
    let sample = |text: &str| -> String { text.chars().filter(|&c| c == ' ' || has(c)).collect() };

    let big = font.render("Aa", &Style { px: 78.0 * s, embolden: 0.0, color: white, shadow: false });
    let bx = ((w as i32 - big.w as i32) / 2).max(0);
    canvas.surface().blit_colored(&big, bx, (30.0 * s) as i32, INK, 255);
    for (i, (line, pt)) in [("ABCDEFGHIJKLM", 11.0), ("abcdefghijklm", 11.0), ("0123456789", 11.0)].into_iter().enumerate() {
        let t = sample(line);
        if t.trim().is_empty() {
            continue;
        }
        let img = font.render(&font.fit(&t, pt * s, 172.0 * s), &Style { px: pt * s, embolden: 0.0, color: white, shadow: false });
        let x = ((w as i32 - img.w as i32) / 2).max(0);
        canvas.surface().blit_colored(&img, x, ((150.0 + i as f32 * 18.0) * s) as i32, INK, 255);
    }
    let stem = file_name.rsplit_once('.').map_or(file_name, |(a, _)| a);
    canvas.text_centered(stem, 8.0, Weight::Regular, MUTED, 100.0, 236.0, 176.0);
    canvas.into_pixels().map(ThumbArt::Pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::zip::Writer;
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&vec![200u8; (w * h * 4) as usize]).unwrap();
        }
        out
    }

    #[test]
    fn a_picture_is_scaled_to_fit_the_square() {
        let p = decode_fitted(&png(64, 32), 16).unwrap();
        assert_eq!((p.w, p.h), (16, 8));
        // never enlarged
        let p = decode_fitted(&png(8, 8), 64).unwrap();
        assert_eq!((p.w, p.h), (8, 8));
        assert!(decode_fitted(b"GIF89a", 64).is_none());
    }

    #[test]
    fn the_image_provider_reads_the_file() {
        let dir = std::env::temp_dir().join(format!("bifrost-thumbs-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        std::fs::write(&path, png(20, 10)).unwrap();
        let f = SourceFile::new(path, Default::default());
        let Some(ThumbArt::Pixels(p)) = ImageProvider.render(&f, 10) else { panic!("pixels") };
        assert_eq!((p.w, p.h), (10, 5));
    }

    fn doc_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        for (n, d) in entries {
            w.add_bytes(n, d, 0).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn an_office_document_gives_its_embedded_picture() {
        let img = png(30, 40);
        let docx = doc_zip(&[("word/document.xml", b"<w/>"), ("docProps/thumbnail.png", &img)]);
        assert_eq!(embedded_preview(Cursor::new(docx)).as_deref(), Some(&img[..]));
        let odt = doc_zip(&[("content.xml", b"<c/>"), ("Thumbnails/thumbnail.png", &img)]);
        assert_eq!(embedded_preview(Cursor::new(odt)).as_deref(), Some(&img[..]));
    }

    #[test]
    fn a_document_without_a_picture_or_a_zip_has_none() {
        let plain = doc_zip(&[("word/document.xml", b"<w/>")]);
        assert!(embedded_preview(Cursor::new(plain)).is_none());
        assert!(embedded_preview(Cursor::new(b"not a zip at all, just text".to_vec())).is_none());
    }

    #[test]
    fn data_that_is_not_a_font_has_no_card() {
        assert!(font_card(b"definitely not a font", "x.ttf", 128).is_none());
    }
}
