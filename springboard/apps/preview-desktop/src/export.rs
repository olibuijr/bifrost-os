//! Export of a page as a PNG or JPEG picture: the page is scaled, adjusted, turned and has its markup burned in, then
//! written beside the document as `<name>-markup.<ext>`. A PDF page is drawn by the worker, so its export runs there
//! too; a picture is written at once.

use crate::document::adjusted;
use crate::model::{self, Adjust, Mark};
use crate::pdfjob::{self, Job, Pdf, Reply, DOCUMENT_RENDER_BOUNDS};
use crate::pixels;
use bifrost_app::kit::Image;
use springboard::cg::imageio::encode::{crc32, encode_png, PngOptions};
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

/// Encodes `img` as `raster`. A PNG keeps transparency; a JPEG has none, so a transparent picture is laid over white
/// first.
pub fn encode(img: &Image, raster: Raster) -> Result<Vec<u8>, String> {
    let unencoded = || "The picture could not be encoded.".to_string();
    match raster {
        Raster::Png => png_bytes(img).ok_or_else(unencoded),
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
            Ok(encoded)
        }
    }
}

/// Encodes `img` as `raster` and writes it to `target`.
pub fn write_image(img: &Image, raster: Raster, target: &Path) -> Result<(), String> {
    let bytes = encode(img, raster)?;
    std::fs::write(target, bytes).map_err(|e| e.to_string())
}

/// What File > Save writes for a picture: `img` encoded as `raster` into `target`. A PNG keeps what `original` (the
/// bytes of the file the picture was opened from) says about the picture besides its pixels (`kept_png_chunks`);
/// `quarter_turn` says that the picture was turned by an odd number of quarter turns since. A JPEG is written without
/// any of it.
pub fn write_saved(img: &Image, raster: Raster, original: &[u8], quarter_turn: bool, target: &Path) -> Result<(), String> {
    let bytes = match raster {
        Raster::Png => with_png_chunks(encode(img, raster)?, &kept_png_chunks(original, quarter_turn)),
        Raster::Jpeg(_) => encode(img, raster)?,
    };
    std::fs::write(target, bytes).map_err(|e| e.to_string())
}

/// The first eight bytes of every PNG file.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// The length of the signature and the header chunk that follows it: 4 bytes of length, the type, 13 bytes and the CRC.
const PNG_HEAD: usize = 8 + 4 + 4 + 13 + 4;

/// The chunks of a PNG before its end chunk, as (type, body). Bytes that are no PNG give none, and a chunk that runs
/// past the end of the data ends the list, so no cut chunk is ever copied.
fn png_chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    if data.len() < PNG_SIGNATURE.len() || data[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return out;
    }
    let mut at = PNG_SIGNATURE.len();
    while at + 12 <= data.len() {
        let len = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let kind = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        let end = match at.checked_add(12).and_then(|e| e.checked_add(len)) {
            Some(end) if end <= data.len() => end,
            _ => break,
        };
        if &kind == b"IEND" {
            break;
        }
        out.push((kind, &data[at + 8..end - 4]));
        at = end;
    }
    out
}

/// One PNG chunk: the length, the type, the body and the CRC of type and body.
fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend((body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(body);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// The chunks a saved PNG keeps from the file it was opened from, as whole chunks ready to be written: what the file
/// says about the picture besides its pixels. Empty when `original` is no PNG or has none of them.
///
/// - `pHYs`, the physical size of a pixel. After an odd number of quarter turns (`quarter_turn`) its two axes are
///   swapped, as the pixels were.
/// - `tEXt`, `zTXt` and `iTXt`, the text (title, author, software, an XMP packet), as they are.
/// - `eXIf`, with the orientation set to upright and the link to the embedded preview taken out (`upright_exif`): the
///   picture was turned upright when it was opened (`document::decode_picture`) and every turn since is in the pixels,
///   so a reader must not turn it again, and the preview showed the picture before the edits.
/// - `gAMA`, `cHRM` and `sRGB`, which say how the pixel values are to be read, when the file has no colour profile:
///   the values are written as they were read, so what was said about them still holds.
///
/// A colour profile (`iCCP`) is not kept, nor are the three above beside one: a picture with a profile is converted to
/// the display's colours when it is opened, so its pixel values are no longer in that profile. The chunks that
/// describe how the pixels are stored (`PLTE`, `tRNS`, `sBIT`, `bKGD`, `hIST`, `sPLT`) and the modification time
/// (`tIME`) belong to the old file. Of a chunk that a file may hold once, the first is kept.
pub fn kept_png_chunks(original: &[u8], quarter_turn: bool) -> Vec<u8> {
    let chunks = png_chunks(original);
    let profiled = chunks.iter().any(|(kind, _)| kind == b"iCCP");
    let mut once: Vec<[u8; 4]> = Vec::new();
    let mut out = Vec::new();
    for (kind, body) in chunks {
        let single = matches!(&kind, b"pHYs" | b"eXIf" | b"gAMA" | b"cHRM" | b"sRGB");
        if single && once.contains(&kind) {
            continue;
        }
        match &kind {
            b"pHYs" if body.len() == 9 => {
                let mut size = body.to_vec();
                if quarter_turn {
                    size[..4].copy_from_slice(&body[4..8]);
                    size[4..8].copy_from_slice(&body[..4]);
                }
                png_chunk(&mut out, &kind, &size);
            }
            b"tEXt" | b"zTXt" | b"iTXt" => png_chunk(&mut out, &kind, body),
            b"eXIf" => match upright_exif(body) {
                Some(block) => png_chunk(&mut out, &kind, &block),
                None => continue,
            },
            b"gAMA" | b"cHRM" | b"sRGB" if !profiled => png_chunk(&mut out, &kind, body),
            _ => continue,
        }
        if single {
            once.push(kind);
        }
    }
    out
}

/// A TIFF-structured metadata block (the body of a PNG's `eXIf` chunk) for a picture whose pixels are upright: the
/// orientation of its first directory is set to 1, and that directory's link to the next one, which holds the
/// embedded preview, is cleared. Everything else stays byte for byte. None when the block cannot be read, so a block
/// whose orientation could not be set is not kept at all.
fn upright_exif(block: &[u8]) -> Option<Vec<u8>> {
    let little = if block.starts_with(b"II") {
        true
    } else if block.starts_with(b"MM") {
        false
    } else {
        return None;
    };
    let short = |at: usize| -> Option<u16> {
        let b = block.get(at..at.checked_add(2)?)?;
        Some(if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let long = |at: usize| -> Option<u32> {
        let b = block.get(at..at.checked_add(4)?)?;
        Some(if little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    if short(2)? != 42 {
        return None;
    }
    let directory = long(4)? as usize;
    let count = short(directory)? as usize;
    // the entries are 12 bytes each, and the offset of the next directory follows them
    let next = directory.checked_add(2)?.checked_add(count.checked_mul(12)?)?;
    if next.checked_add(4)? > block.len() {
        return None;
    }
    let mut out = block.to_vec();
    for k in 0..count {
        let entry = directory + 2 + k * 12;
        // the orientation: tag 0x0112, one 16-bit value, held in the entry itself
        if short(entry)? == 0x0112 && short(entry + 2)? == 3 && long(entry + 4)? == 1 {
            let upright = if little { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            out[entry + 8..entry + 10].copy_from_slice(&upright);
        }
    }
    out[next..next + 4].copy_from_slice(&[0; 4]);
    Some(out)
}

/// The encoded PNG `png` with `kept` (whole chunks, from `kept_png_chunks`) right after its header, before the image
/// data, where every one of them may stand. A `png` that does not start with a header is returned as it is.
fn with_png_chunks(png: Vec<u8>, kept: &[u8]) -> Vec<u8> {
    if kept.is_empty() || png.len() < PNG_HEAD || png[..PNG_SIGNATURE.len()] != PNG_SIGNATURE || &png[12..16] != b"IHDR" {
        return png;
    }
    let mut out = Vec::with_capacity(png.len() + kept.len());
    out.extend_from_slice(&png[..PNG_HEAD]);
    out.extend_from_slice(kept);
    out.extend_from_slice(&png[PNG_HEAD..]);
    out
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

    /// A TIFF block as a camera writes it, little-endian: one directory with the orientation, and a link to a second
    /// directory (the preview) at `next`.
    fn exif_block(orientation: u8, next: u8) -> Vec<u8> {
        vec![0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0, next, 0, 0, 0]
    }

    /// The same block with the bytes of every number the other way round.
    fn exif_block_big_endian(orientation: u8) -> Vec<u8> {
        vec![0x4d, 0x4d, 0, 0x2a, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation, 0, 0, 0, 0, 0, 26]
    }

    fn chunk_named<'a>(png: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
        png_chunks(png).into_iter().find(|(k, _)| k == kind).map(|(_, body)| body)
    }

    /// A PNG with a gamma, a physical pixel size that differs on its two axes, a text and an EXIF block that says the
    /// picture is to be turned.
    fn png_with_metadata(img: &Image, icc: bool) -> Vec<u8> {
        let options = PngOptions {
            text: vec![("Title".to_string(), "Gradient".to_string())],
            dpi: Some((72.0, 144.0)),
            gamma: Some(0.45455),
            exif: Some(exif_block(6, 26)),
            icc: icc.then(|| ("profile".to_string(), vec![1, 2, 3, 4])),
            ..Default::default()
        };
        encode_png(img.w, img.h, &pixels::to_straight_rgba(img), &options).expect("a png")
    }

    /// Save keeps what the PNG said about the picture besides its pixels: the gamma, the physical pixel size and the
    /// text as they were, and the EXIF block with the orientation set to upright (the pixels are upright already) and
    /// no link to the old preview. The chunks stand between the header and the image data, and the file still decodes
    /// to the same pixels.
    #[test]
    fn a_saved_png_keeps_its_gamma_physical_size_text_and_exif() {
        let dir = std::env::temp_dir().join(format!("viewer-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let img = Image { w: 3, h: 2, px: vec![0xff00_00ff, 0xff00_ff00, 0xffff_0000, 0xff10_2030, 0xff40_5060, 0xff70_8090] };
        let original = png_with_metadata(&img, false);
        for kind in [b"gAMA", b"pHYs", b"tEXt", b"eXIf"] {
            assert!(chunk_named(&original, kind).is_some(), "the test file has {}", String::from_utf8_lossy(kind));
        }
        let target = dir.join("saved.png");
        write_saved(&img, Raster::Png, &original, false, &target).expect("a png");
        let saved = std::fs::read(&target).expect("the file");
        for kind in [b"gAMA", b"pHYs", b"tEXt"] {
            assert_eq!(chunk_named(&saved, kind), chunk_named(&original, kind), "{} is kept as it was", String::from_utf8_lossy(kind));
        }
        assert_eq!(chunk_named(&saved, b"tEXt"), Some(&b"Title\0Gradient"[..]));
        let mut upright = exif_block(1, 0);
        assert_eq!(chunk_named(&saved, b"eXIf"), Some(&upright[..]), "upright, and without the link to the preview");
        upright[18] = 6;
        assert_ne!(chunk_named(&saved, b"eXIf"), Some(&upright[..]));
        // the order: the header, what was kept, the image data
        let order: Vec<[u8; 4]> = png_chunks(&saved).into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(order.first(), Some(b"IHDR"));
        let data = order.iter().position(|k| k == b"IDAT").expect("image data");
        assert!(order[data..].iter().all(|k| k == b"IDAT"), "nothing follows the image data but its end: {order:?}");
        assert_eq!(order.iter().filter(|k| *k == b"pHYs").count(), 1);
        // every chunk has the CRC of its type and body, and the end chunk closes the file
        let mut at = PNG_SIGNATURE.len();
        while at + 12 <= saved.len() {
            let len = u32::from_be_bytes([saved[at], saved[at + 1], saved[at + 2], saved[at + 3]]) as usize;
            let crc = u32::from_be_bytes([saved[at + 8 + len], saved[at + 9 + len], saved[at + 10 + len], saved[at + 11 + len]]);
            assert_eq!(crc, crc32(&saved[at + 4..at + 8 + len]), "the chunk at {at}");
            at += 12 + len;
        }
        assert_eq!(at, saved.len());
        assert_eq!(&saved[saved.len() - 8..saved.len() - 4], b"IEND");
        // the pixels are the same, bit for bit, in the same 8-bit RGB
        assert_eq!((saved[24], saved[25]), (8, 2), "8 bits a sample, RGB");
        assert_eq!(crate::document::decode_picture(&saved).expect("it decodes"), img);
        // a quarter turn swaps the two axes of the physical size
        let turned = crate::model::rotate(&img, 1);
        write_saved(&turned, Raster::Png, &original, true, &target).expect("a png");
        let saved = std::fs::read(&target).expect("the file");
        let (was, now) = (chunk_named(&original, b"pHYs").unwrap(), chunk_named(&saved, b"pHYs").unwrap());
        assert_ne!(was[..4], was[4..8], "the test file's axes differ");
        assert_eq!((&now[..4], &now[4..8], now[8]), (&was[4..8], &was[..4], was[8]));
        assert_eq!(crate::document::decode_picture(&saved).expect("it decodes"), turned);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What does not hold for the saved pixels is not kept: a colour profile and the gamma beside it (a picture with
    /// a profile is converted to the display's colours when it opens), a second chunk of a kind a file holds once, an
    /// EXIF block that cannot be read, and anything from a file that is no PNG. A JPEG gets none of it.
    #[test]
    fn a_saved_png_does_not_keep_what_no_longer_holds() {
        let img = Image { w: 2, h: 1, px: vec![0xff00_00ff, 0xff00_ff00] };
        let profiled = png_with_metadata(&img, true);
        assert!(chunk_named(&profiled, b"iCCP").is_some() && chunk_named(&profiled, b"gAMA").is_some());
        let kept = kept_png_chunks(&profiled, false);
        let saved = with_png_chunks(png_bytes(&img).unwrap(), &kept);
        assert!(chunk_named(&saved, b"iCCP").is_none() && chunk_named(&saved, b"gAMA").is_none());
        assert!(chunk_named(&saved, b"pHYs").is_some() && chunk_named(&saved, b"tEXt").is_some() && chunk_named(&saved, b"eXIf").is_some());
        // two physical sizes, a storage chunk and a time: the first size only
        let mut odd = PNG_SIGNATURE.to_vec();
        png_chunk(&mut odd, b"pHYs", &[0, 0, 0, 1, 0, 0, 0, 2, 1]);
        png_chunk(&mut odd, b"pHYs", &[0, 0, 0, 3, 0, 0, 0, 4, 1]);
        png_chunk(&mut odd, b"sBIT", &[8, 8, 8]);
        png_chunk(&mut odd, b"tIME", &[7, 230, 1, 1, 0, 0, 0]);
        png_chunk(&mut odd, b"eXIf", b"not a tiff block");
        let mut first = Vec::new();
        png_chunk(&mut first, b"pHYs", &[0, 0, 0, 1, 0, 0, 0, 2, 1]);
        assert_eq!(kept_png_chunks(&odd, false), first);
        // a chunk cut off by the end of the data is not copied
        let mut cut = odd.clone();
        png_chunk(&mut cut, b"tEXt", b"Title\0Cut");
        cut.truncate(cut.len() - 3);
        assert_eq!(kept_png_chunks(&cut, false), first);
        assert!(kept_png_chunks(b"\xff\xd8\xff\xe0 a jpeg", false).is_empty() && kept_png_chunks(&[], true).is_empty());
        // both byte orders of the EXIF block
        assert_eq!(upright_exif(&exif_block_big_endian(8)), Some(exif_block_big_endian(1)).map(|mut b| {
            b[22..26].copy_from_slice(&[0; 4]);
            b
        }));
        assert_eq!(upright_exif(&exif_block(3, 0)), Some(exif_block(1, 0)));
        assert_eq!(upright_exif(&exif_block(3, 0)[..20]), None, "a block that ends inside its directory");
        // a JPEG is written as before
        let dir = std::env::temp_dir().join(format!("viewer-kept-jpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let (plain, saved) = (dir.join("a.jpg"), dir.join("b.jpg"));
        write_image(&img, Raster::Jpeg(JPEG_QUALITY), &plain).expect("a jpeg");
        write_saved(&img, Raster::Jpeg(JPEG_QUALITY), &profiled, false, &saved).expect("a jpeg");
        assert_eq!(std::fs::read(&plain).unwrap(), std::fs::read(&saved).unwrap());
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
