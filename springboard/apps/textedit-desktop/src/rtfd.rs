//! Rich text packages (RTFD): a folder holding the document as `TXT.rtf` and the files its attachments name (images
//! and other files embedded in the text).
//!
//! An attachment is a NeXTGraphic group in the RTF: `{\NeXTGraphic name.png \width W \height H}`, its file name and its
//! size in twips, standing for one `OBJECT` character in the text (see `rtf::write_with_attachments`). The package
//! carries the file of each attachment beside `TXT.rtf`, under the name the group gives. This module is pure: the folder
//! itself is read and written by the SDK (`bifrost_app::document_package`).

use crate::properties::Properties;
use crate::rtf::{self, PictureRef};
use crate::text::{Attachment, Cell, OBJECT};

/// The main file of a package.
pub const MAIN_FILE: &str = "TXT.rtf";
/// The largest width or height an attachment is given from its own pixel size, in points; a larger picture is scaled down.
pub const MAX_IMAGE_POINTS: f32 = 320.0;
/// The size of an attachment whose file gives no size and whose data is not an image we can measure, in points.
pub const FALLBACK_POINTS: (f32, f32) = (160.0, 120.0);

/// A package read back: the cells, the tab distance, the document properties and the attachments the cells number.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub cells: Vec<Cell>,
    pub tab: f32,
    pub properties: Properties,
    pub attachments: Vec<Attachment>,
}

/// Reads a package from its files (name and bytes). The main file is `TXT.rtf` (any other `.rtf` file if there is none
/// of that name). Returns `None` when there is no RTF main file. An attachment whose file is missing keeps its size
/// and has no data.
pub fn read(files: &[(String, Vec<u8>)]) -> Option<Package> {
    let main = files
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(MAIN_FILE))
        .or_else(|| files.iter().find(|(n, _)| n.to_ascii_lowercase().ends_with(".rtf")))?;
    let bytes = &main.1;
    if !rtf::is_rtf(bytes) {
        return None;
    }
    let (cells, tab, pictures) = rtf::read_doc_pictures(bytes);
    let attachments = pictures
        .into_iter()
        .map(|(file, width, height)| {
            let data = files.iter().find(|(n, _)| *n == file).map(|(_, d)| d.clone()).unwrap_or_default();
            attachment(file, width, height, data)
        })
        .collect();
    Some(Package { cells, tab, properties: rtf::read_info(bytes), attachments })
}

/// An attachment from what its RTF group and its file give: the size in points when the group gives one, else the
/// picture's own pixel size (a PNG header is read; a larger picture is scaled to `MAX_IMAGE_POINTS`), else the fallback.
pub fn attachment(file: String, width: f32, height: f32, data: Vec<u8>) -> Attachment {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        match png_size(&data).or_else(|| bifrost_app::graphics_imageio::pixel_size(&data).map(|(w, h)| (w as f32, h as f32))) {
            Some(size) => fit(size),
            None => FALLBACK_POINTS,
        }
    };
    Attachment { file, width, height, data }
}

/// Scales a size down, keeping its shape, so neither side is larger than `MAX_IMAGE_POINTS`.
fn fit((w, h): (f32, f32)) -> (f32, f32) {
    let scale = (MAX_IMAGE_POINTS / w.max(h)).min(1.0);
    (w * scale, h * scale)
}

/// The pixel size of a PNG file, from its IHDR chunk; `None` for anything else.
pub fn png_size(data: &[u8]) -> Option<(f32, f32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 24 || !data.starts_with(&SIGNATURE) || &data[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (w > 0 && h > 0).then_some((w as f32, h as f32))
}

/// The files of a package for the cells: `TXT.rtf`, then the data of each attachment the cells use. An attachment
/// the text no longer holds is not saved. Each file gets a name no other file of the package has (`my photo.png`
/// is written as `my_photo.png`; a clash becomes `name-2.png`).
pub fn write(cells: &[Cell], tab: f32, props: &Properties, attachments: &[Attachment]) -> Vec<(String, Vec<u8>)> {
    let mut used: Vec<String> = vec![MAIN_FILE.to_string()];
    let mut table = attachments.to_vec();
    let mut files = Vec::new();
    for (i, att) in attachments.iter().enumerate() {
        let number = i + 1;
        if !cells.iter().any(|c| c.ch == OBJECT && c.style.attachment as usize == number) {
            continue;
        }
        let mut base = rtf::safe_name(&att.file).replace('/', "_");
        if base.is_empty() {
            base = "attachment".into();
        }
        let name = unique(&base, &used);
        used.push(name.clone());
        table[i].file = name.clone();
        files.push((name, att.data.clone()));
    }
    let mut out = vec![(MAIN_FILE.to_string(), rtf::write_with_attachments(cells, tab, props, &table))];
    out.extend(files);
    out
}

/// `base`, or `stem-N.ext` with the first N from 2 that no used name has.
fn unique(base: &str, used: &[String]) -> String {
    if !used.iter().any(|u| u == base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => (&base[..i], &base[i..]),
        _ => (base, ""),
    };
    (2..).map(|n| format!("{stem}-{n}{ext}")).find(|c| !used.iter().any(|u| u == c)).unwrap_or_default()
}

/// The attachments of a single RTF file (no package): their sizes, with no data. Used when an RTF file that names
/// pictures is opened on its own.
pub fn attachments_without_data(pictures: Vec<PictureRef>) -> Vec<Attachment> {
    pictures.into_iter().map(|(file, width, height)| attachment(file, width, height, Vec::new())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Style;

    /// A PNG file's signature and IHDR chunk of the given size. Only the header is real: the checksums are not, which
    /// is fine because the size is all that is read.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    fn text_cells(s: &str) -> Vec<Cell> {
        s.chars().map(|ch| Cell { ch, style: Style::default() }).collect()
    }

    fn object(number: u16) -> Cell {
        Cell { ch: OBJECT, style: Style { attachment: number, ..Style::default() } }
    }

    #[test]
    fn a_picture_group_reads_as_one_attachment_character_with_its_size() {
        let (cells, _, pictures) = rtf::read_doc_pictures(b"{\\rtf1 a{\\NeXTGraphic pic.png \\width200\\height100}b}");
        assert_eq!(cells.iter().map(|c| c.ch).collect::<String>(), format!("a{OBJECT}b"));
        assert_eq!(cells[1].style.attachment, 1);
        assert_eq!(pictures, vec![("pic.png".to_string(), 10.0, 5.0)]);
    }

    #[test]
    fn a_picture_without_a_name_is_nothing_and_a_picture_group_is_not_text() {
        let (cells, _, pictures) = rtf::read_doc_pictures(b"{\\rtf1 x{\\NeXTGraphic \\width20}y}");
        assert_eq!(cells.iter().map(|c| c.ch).collect::<String>(), "xy");
        assert!(pictures.is_empty());
    }

    #[test]
    fn the_size_is_taken_from_the_picture_when_the_group_gives_none() {
        let big = attachment("a.png".into(), 0.0, 0.0, png(640, 480));
        assert_eq!((big.width, big.height), (320.0, 240.0), "scaled to fit the largest side");
        let small = attachment("b.png".into(), 0.0, 0.0, png(64, 32));
        assert_eq!((small.width, small.height), (64.0, 32.0));
        let unknown = attachment("c.jpg".into(), 0.0, 0.0, vec![0xff, 0xd8]);
        assert_eq!((unknown.width, unknown.height), FALLBACK_POINTS);
        let given = attachment("d.png".into(), 30.0, 40.0, png(640, 480));
        assert_eq!((given.width, given.height), (30.0, 40.0), "the group's size wins");
    }

    #[test]
    fn a_package_round_trips_through_its_files_with_the_image_bytes() {
        let photo = png(200, 100);
        let attachments = vec![Attachment { file: "my photo.png".into(), width: 100.0, height: 50.0, data: photo.clone() }];
        let mut cells = text_cells("a");
        cells.push(object(1));
        cells.extend(text_cells("b\n"));
        let props = Properties { title: "Letter".into(), ..Properties::default() };
        let files = write(&cells, 36.0, &props, &attachments);
        assert_eq!(files[0].0, MAIN_FILE);
        assert_eq!(files[1], ("my_photo.png".to_string(), photo.clone()));

        let back = read(&files).expect("a package with an RTF main file");
        assert_eq!(back.cells.iter().map(|c| (c.ch, c.style.attachment)).collect::<Vec<_>>(), cells.iter().map(|c| (c.ch, c.style.attachment)).collect::<Vec<_>>());
        assert_eq!(back.attachments, vec![Attachment { file: "my_photo.png".into(), width: 100.0, height: 50.0, data: photo }]);
        assert_eq!(back.properties.title, "Letter");
        assert_eq!(back.tab, 36.0);
    }

    #[test]
    fn an_attachment_the_text_no_longer_holds_is_not_saved() {
        let attachments = vec![Attachment { file: "gone.png".into(), width: 1.0, height: 1.0, data: png(1, 1) }];
        let files = write(&text_cells("only text"), 36.0, &Properties::default(), &attachments);
        assert_eq!(files.len(), 1, "just TXT.rtf");
        assert!(!String::from_utf8_lossy(&files[0].1).contains("NeXTGraphic"));
    }

    #[test]
    fn two_attachments_with_one_name_get_two_names() {
        let a = Attachment { file: "pic.png".into(), width: 10.0, height: 10.0, data: vec![1] };
        let b = Attachment { file: "pic.png".into(), width: 10.0, height: 10.0, data: vec![2] };
        let mut cells = vec![object(1)];
        cells.push(object(2));
        let files = write(&cells, 36.0, &Properties::default(), &[a, b]);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["TXT.rtf", "pic.png", "pic-2.png"]);
        assert_eq!(files[2].1, vec![2]);
    }

    #[test]
    fn a_missing_attachment_file_keeps_its_size_with_no_data() {
        let files = vec![("TXT.rtf".to_string(), b"{\\rtf1 {\\NeXTGraphic lost.png \\width40\\height20}}".to_vec())];
        let back = read(&files).unwrap();
        assert_eq!(back.attachments, vec![Attachment { file: "lost.png".into(), width: 2.0, height: 1.0, data: Vec::new() }]);
    }

    #[test]
    fn a_package_without_an_rtf_main_file_is_refused() {
        assert!(read(&[("notes.txt".to_string(), b"plain".to_vec())]).is_none());
        assert!(read(&[("TXT.rtf".to_string(), b"plain".to_vec())]).is_none(), "not RTF");
    }
}
