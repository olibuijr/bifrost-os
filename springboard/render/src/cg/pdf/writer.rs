//! PDF writing: a document of pages, each with a content stream, the standard fourteen faces and raster images.
//!
//! The print pipeline builds its pages here (text from an app's layout, or raster slices of a captured frame) and the
//! same bytes go to a print queue or to a Save as PDF file. Content streams are written as plain text so a reader and
//! the tests can see the operators; images are RGB and Flate-compressed. Fonts are named by their standard names and
//! text is WinAnsi, see [`text_literal`].

use super::obj::{fmt_num, write_name};

/// A raster image drawn on a page: `width * height` pixels, three bytes each (red, green, blue), rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// One page: its size in points, its content stream and the images it draws. The content refers to the document's fonts
/// as `/F<n>` (the index into the font list given to [`write`]) and to this page's images as `/Im<n>` (the index into
/// `images`).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub content: Vec<u8>,
    pub images: Vec<PageImage>,
}

/// The WinAnsi byte for a character, `?` when the standard faces have no glyph for it.
pub fn winansi(ch: char) -> u8 {
    match ch {
        ' '..='~' => ch as u8,
        '\u{a0}'..='\u{ff}' => ch as u32 as u8,
        '\u{20ac}' => 0x80,
        '\u{2026}' => 0x85,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        _ => b'?',
    }
}

/// Appends `text` as a PDF string literal (`(...)`), escaping parentheses and backslashes.
pub fn text_literal(text: &str, out: &mut Vec<u8>) {
    out.push(b'(');
    for ch in text.chars() {
        match winansi(ch) {
            b @ (b'(' | b')' | b'\\') => out.extend([b'\\', b]),
            b => out.push(b),
        }
    }
    out.push(b')');
}

/// A stream object: its dictionary entries, then the bytes between `stream` and `endstream`.
fn stream(entries: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {entries} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// The image's stream: raw RGB compressed with zlib (`FlateDecode`).
fn image_object(img: &PageImage) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&img.rgb, 6);
    stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
            img.width, img.height
        ),
        &packed,
    )
}

/// Writes `pages` as one PDF. `fonts` are standard font names (`Helvetica`, `Times-BoldItalic`, `Courier`, ...); every
/// page lists all of them as `/F0`, `/F1`, ... in that order. A page with no images has no XObject resources.
pub fn write(pages: &[Page], fonts: &[&str]) -> Vec<u8> {
    // object numbers: 1 catalog, 2 page tree, then the fonts, then each page, its content and its images
    let first_font = 3;
    let mut next = first_font + fonts.len();
    let mut page_ids = Vec::with_capacity(pages.len());
    let mut layout = Vec::with_capacity(pages.len());
    for page in pages {
        let page_id = next;
        let content_id = next + 1;
        let image_ids: Vec<usize> = (0..page.images.len()).map(|i| content_id + 1 + i).collect();
        next = content_id + 1 + page.images.len();
        page_ids.push(page_id);
        layout.push((page_id, content_id, image_ids));
    }
    let total = next - 1;
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(); total];

    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    let kids: Vec<String> = page_ids.iter().map(|id| format!("{id} 0 R")).collect();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    for (i, face) in fonts.iter().enumerate() {
        let mut font = b"<< /Type /Font /Subtype /Type1 /BaseFont ".to_vec();
        write_name(face, &mut font);
        font.extend(b" /Encoding /WinAnsiEncoding >>");
        objects[first_font + i - 1] = font;
    }
    let font_dict: String = (0..fonts.len()).map(|i| format!("/F{i} {} 0 R ", first_font + i)).collect();

    for (page, (page_id, content_id, image_ids)) in pages.iter().zip(&layout) {
        let mut resources = String::new();
        if !fonts.is_empty() {
            resources.push_str(&format!("/Font << {font_dict}>> "));
        }
        if !image_ids.is_empty() {
            let xobjects: String = image_ids.iter().enumerate().map(|(i, id)| format!("/Im{i} {id} 0 R ")).collect();
            resources.push_str(&format!("/XObject << {xobjects}>> "));
        }
        objects[page_id - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << {resources}>> /Contents {content_id} 0 R >>",
            fmt_num(page.width),
            fmt_num(page.height)
        )
        .into_bytes();
        objects[content_id - 1] = stream("", &page.content);
        for (img, id) in page.images.iter().zip(image_ids) {
            objects[id - 1] = image_object(img);
        }
    }

    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(total);
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", total + 1).into_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").into_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", total + 1).into_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(content: &str, images: Vec<PageImage>) -> Page {
        Page { width: 595.0, height: 842.0, content: content.as_bytes().to_vec(), images }
    }

    /// The byte offset that the xref table records for object `n`.
    fn xref_offset(pdf: &[u8], n: usize) -> usize {
        let text = String::from_utf8_lossy(pdf).into_owned();
        let at = text.rfind("startxref\n").unwrap() + "startxref\n".len();
        let xref: usize = text[at..].lines().next().unwrap().parse().unwrap();
        let entry = text[xref..].lines().nth(2 + n).unwrap();
        entry.split_whitespace().next().unwrap().parse().unwrap()
    }

    #[test]
    fn one_page_per_entry_with_a_valid_xref_and_trailer() {
        let pdf = write(&[page("BT ET", vec![]), page("BT ET", vec![])], &["Helvetica"]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Count 2"));
        assert!(s.trim_end().ends_with("%%EOF"));
        // 2 catalog and tree, 1 font, 2 per page
        assert!(s.contains("/Size 8"));
        assert_eq!(s.matches(" 0 obj").count(), 7);
        for n in 1..=7 {
            assert!(pdf[xref_offset(&pdf, n)..].starts_with(format!("{n} 0 obj").as_bytes()));
        }
    }

    #[test]
    fn fonts_are_named_in_resources_and_the_faces_are_winansi() {
        let pdf = write(&[page("BT ET", vec![])], &["Helvetica", "Courier-Bold"]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.contains("/Font << /F0 3 0 R /F1 4 0 R >>"));
        assert!(s.contains("/BaseFont /Courier-Bold /Encoding /WinAnsiEncoding"));
        assert!(!s.contains("/XObject"), "a page without images has no XObject resource");
    }

    #[test]
    fn images_are_flate_compressed_and_inflate_back_to_their_pixels() {
        let rgb: Vec<u8> = (0..2 * 2 * 3).map(|i| i as u8).collect();
        let img = PageImage { width: 2, height: 2, rgb: rgb.clone() };
        let pdf = write(&[page("q 10 0 0 10 0 0 cm /Im0 Do Q", vec![img])], &[]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.contains("/Im0 5 0 R"), "the image is object 5 after catalog, tree, page and content");
        assert!(s.contains("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"));
        let at = xref_offset(&pdf, 5);
        let body = &pdf[at..];
        let start = body.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = body.windows(10).position(|w| w == b"\nendstream").unwrap();
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&body[start..end]).unwrap();
        assert_eq!(inflated, rgb);
    }

    #[test]
    fn text_literals_escape_parentheses_and_map_unknown_characters() {
        let mut out = Vec::new();
        text_literal("hello (world) \\ \u{201c}q\u{201d} \u{65e5}", &mut out);
        assert_eq!(out, b"(hello \\(world\\) \\\\ \x93q\x94 ?)".to_vec());
        assert_eq!(winansi('\u{e9}'), 0xe9);
        assert_eq!(winansi('\u{20ac}'), 0x80);
        assert_eq!(winansi('\u{65e5}'), b'?');
    }

    #[test]
    fn page_size_is_written_in_points() {
        let p = Page { width: 612.0, height: 791.5, content: Vec::new(), images: Vec::new() };
        let s = String::from_utf8_lossy(&write(&[p], &[])).into_owned();
        assert!(s.contains("/MediaBox [0 0 612 791.5]"));
    }
}
