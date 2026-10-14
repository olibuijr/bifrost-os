//! `iconutil`: converts between an iconset folder and an ICNS file. An iconset holds PNG files named by size (`icon_16x16.png`,
//! `icon_16x16@2x.png` and so on); an ICNS file holds the same PNG images as typed elements (`icp4`, `ic07`, ...). Both
//! directions copy the PNG bytes unchanged, so nothing is decoded or re-encoded.
//! Gap: legacy ICNS elements that hold raw RGB or mask data (`il32`, `ih32`, `it32`, `t8mk` and the like) are skipped when
//! an ICNS file is unpacked, and an iconset cannot yet hold them.

use std::fs;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage:
  iconutil -c icns <folder.iconset> [-o <file.icns>]
  iconutil -c iconset <file.icns> [-o <folder.iconset>]
  iconutil -h";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The iconset file names and the ICNS element type each one is stored as, in the order elements are written.
pub const ELEMENTS: &[(&str, &str)] = &[
    ("icon_16x16.png", "icp4"),
    ("icon_16x16@2x.png", "ic11"),
    ("icon_32x32.png", "icp5"),
    ("icon_32x32@2x.png", "ic12"),
    ("icon_128x128.png", "ic07"),
    ("icon_128x128@2x.png", "ic13"),
    ("icon_256x256.png", "ic08"),
    ("icon_256x256@2x.png", "ic14"),
    ("icon_512x512.png", "ic09"),
    ("icon_512x512@2x.png", "ic10"),
];

/// The direction of a conversion: the output type named by `-c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Icns,
    Iconset,
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Convert { target: Target, input: String, output: Option<String> },
}

/// Reads the arguments (without the program name). `-h` wins over the other options.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut target = None;
    let mut input = None;
    let mut output = None;
    let mut help = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "-help" | "--help" => help = true,
            "-c" => {
                let kind = it.next().ok_or("-c needs icns or iconset")?;
                target = Some(match kind.as_str() {
                    "icns" => Target::Icns,
                    "iconset" => Target::Iconset,
                    other => return Err(format!("unknown output type {other}")),
                });
            }
            "-o" => output = Some(it.next().ok_or("-o needs a path")?.clone()),
            opt if opt.starts_with('-') => return Err(format!("unknown option {opt}")),
            path => {
                if input.is_some() {
                    return Err("only one input can be converted at a time".to_string());
                }
                input = Some(path.to_string());
            }
        }
    }
    if help {
        return Ok(Command::Help);
    }
    let target = target.ok_or("-c icns or -c iconset is required")?;
    let input = input.ok_or("no input given")?;
    Ok(Command::Convert { target, input, output })
}

/// The output path when `-o` is absent: the input with the other extension, next to it.
pub fn default_output(input: &Path, target: Target) -> PathBuf {
    match target {
        Target::Icns => input.with_extension("icns"),
        Target::Iconset => input.with_extension("iconset"),
    }
}

/// True when the bytes start with the PNG signature.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIGNATURE)
}

/// The ICNS element type an iconset file name is stored as, or `None` for names that are not iconset images.
pub fn element_for_file(name: &str) -> Option<&'static str> {
    ELEMENTS.iter().find(|(file, _)| *file == name).map(|(_, kind)| *kind)
}

/// The iconset file name for an ICNS element type, or `None` for types this tool does not copy.
pub fn file_for_element(kind: &str) -> Option<&'static str> {
    ELEMENTS.iter().find(|(_, k)| *k == kind).map(|(file, _)| *file)
}

fn read_u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    let field: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(field))
}

/// Builds an ICNS file from PNG images, each given with its element type. Every element is a four-character type, the
/// length (header included) and the image bytes; the file header carries the total length.
pub fn encode_icns(images: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    for (kind, png) in images {
        let kind = kind.as_bytes();
        if kind.len() != 4 {
            return Err(format!("element type {kind:?} is not four characters"));
        }
        let len = u32::try_from(png.len() + 8).map_err(|_| "an icon image is too large".to_string())?;
        body.extend_from_slice(kind);
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(png);
    }
    let total = u32::try_from(body.len() + 8).map_err(|_| "the icon set is too large".to_string())?;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// The elements of an ICNS file as (element type, bytes), in file order.
pub fn decode_icns(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    if bytes.get(..4) != Some(b"icns".as_slice()) {
        return Err("not an ICNS file".to_string());
    }
    let total = read_u32_be(bytes, 4).ok_or("ICNS file is truncated")? as usize;
    if total < 8 || total > bytes.len() {
        return Err("ICNS file is truncated".to_string());
    }
    let mut elements = Vec::new();
    let mut pos = 8;
    while pos + 8 <= total {
        let kind = String::from_utf8_lossy(&bytes[pos..pos + 4]).into_owned();
        let len = read_u32_be(bytes, pos + 4).ok_or("ICNS element is truncated")? as usize;
        if len < 8 || pos + len > total {
            return Err(format!("ICNS element {kind} is damaged"));
        }
        elements.push((kind, &bytes[pos + 8..pos + len]));
        pos += len;
    }
    Ok(elements)
}

/// The ICNS file for named iconset files (name, PNG bytes). Only the names in `ELEMENTS` are used, in table order, and
/// every one must be a PNG.
pub fn icns_from_files(files: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut images: Vec<(&str, &[u8])> = Vec::new();
    for (file, kind) in ELEMENTS {
        if let Some((_, png)) = files.iter().find(|(name, _)| name == file) {
            if !is_png(png) {
                return Err(format!("{file} is not a PNG file"));
            }
            images.push((*kind, png.as_slice()));
        }
    }
    if images.is_empty() {
        return Err("no icon images found".to_string());
    }
    encode_icns(&images)
}

/// The iconset files (name, PNG bytes) for an ICNS file. Elements that are not PNG images, or that no iconset name covers,
/// are left out.
pub fn iconset_files_from_icns(bytes: &[u8]) -> Result<Vec<(&'static str, &[u8])>, String> {
    let files: Vec<(&'static str, &[u8])> = decode_icns(bytes)?
        .into_iter()
        .filter_map(|(kind, data)| {
            let file = file_for_element(&kind)?;
            is_png(data).then_some((file, data))
        })
        .collect();
    if files.is_empty() {
        return Err("no PNG icon images in the ICNS file".to_string());
    }
    Ok(files)
}

/// Runs one conversion and writes its output. An iconset folder is read, and written when it is created.
pub fn run(command: &Command) -> Result<(), String> {
    let Command::Convert { target, input, output } = command else {
        return Ok(());
    };
    let out = output.as_deref().map(PathBuf::from).unwrap_or_else(|| default_output(Path::new(input), *target));
    match target {
        Target::Icns => {
            let dir = Path::new(input);
            let mut files = Vec::new();
            for (file, _) in ELEMENTS {
                let path = dir.join(file);
                if path.is_file() {
                    let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    files.push((*file, data));
                }
            }
            let icns = icns_from_files(&files)?;
            fs::write(&out, icns).map_err(|e| format!("{}: {e}", out.display()))
        }
        Target::Iconset => {
            let bytes = fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let files = iconset_files_from_icns(&bytes)?;
            fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            for (file, data) in files {
                let path = out.join(file);
                fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A PNG stand-in: the signature followed by a marker, enough for the byte-level checks.
    fn png(marker: u8) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&[marker; 4]);
        v
    }

    #[test]
    fn parses_both_directions_with_output() {
        assert_eq!(
            parse(&args(&["-c", "icns", "AppIcon.iconset", "-o", "AppIcon.icns"])),
            Ok(Command::Convert {
                target: Target::Icns,
                input: "AppIcon.iconset".into(),
                output: Some("AppIcon.icns".into()),
            })
        );
        assert_eq!(
            parse(&args(&["-c", "iconset", "AppIcon.icns"])),
            Ok(Command::Convert { target: Target::Iconset, input: "AppIcon.icns".into(), output: None })
        );
    }

    #[test]
    fn help_wins_and_bad_arguments_are_refused() {
        assert_eq!(parse(&args(&["-h"])), Ok(Command::Help));
        assert_eq!(parse(&args(&["-c", "icns", "-h"])), Ok(Command::Help));
        assert!(parse(&args(&["-c", "tiff", "x"])).is_err());
        assert!(parse(&args(&["-c", "icns"])).is_err());
        assert!(parse(&args(&["x"])).is_err());
        assert!(parse(&args(&["-c", "icns", "a", "b"])).is_err());
        assert!(parse(&args(&["-c", "icns", "a", "-x"])).is_err());
        assert!(parse(&args(&["-c"])).is_err());
    }

    #[test]
    fn default_output_swaps_the_extension() {
        assert_eq!(default_output(Path::new("docs/App.iconset"), Target::Icns), PathBuf::from("docs/App.icns"));
        assert_eq!(default_output(Path::new("App.icns"), Target::Iconset), PathBuf::from("App.iconset"));
    }

    #[test]
    fn element_names_map_both_ways() {
        assert_eq!(element_for_file("icon_512x512@2x.png"), Some("ic10"));
        assert_eq!(element_for_file("icon_17x17.png"), None);
        assert_eq!(file_for_element("ic07"), Some("icon_128x128.png"));
        assert_eq!(file_for_element("il32"), None);
        for (file, kind) in ELEMENTS {
            assert_eq!(element_for_file(file), Some(*kind));
            assert_eq!(file_for_element(kind), Some(*file));
        }
    }

    #[test]
    fn encodes_and_decodes_an_icns_file() {
        let a = png(1);
        let b = png(2);
        let bytes = encode_icns(&[("icp4", a.as_slice()), ("ic10", b.as_slice())]).unwrap();
        assert_eq!(&bytes[..4], b"icns");
        assert_eq!(read_u32_be(&bytes, 4), Some(bytes.len() as u32));
        let elements = decode_icns(&bytes).unwrap();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].0, "icp4");
        assert_eq!(elements[0].1, a.as_slice());
        assert_eq!(elements[1].0, "ic10");
        assert_eq!(elements[1].1, b.as_slice());
    }

    #[test]
    fn rejects_bad_element_types() {
        assert!(encode_icns(&[("icp", png(1).as_slice())]).is_err());
    }

    #[test]
    fn decode_refuses_damaged_files() {
        assert!(decode_icns(b"nope0000").is_err());
        assert!(decode_icns(b"icns").is_err());
        let good = encode_icns(&[("icp4", png(1).as_slice())]).unwrap();
        assert!(decode_icns(&good[..good.len() - 1]).is_err());
        let mut damaged = good.clone();
        damaged[12..16].copy_from_slice(&1000u32.to_be_bytes());
        assert!(decode_icns(&damaged).is_err());
    }

    #[test]
    fn iconset_images_are_packed_in_table_order() {
        let files = vec![
            ("icon_512x512@2x.png", png(9)),
            ("icon_16x16.png", png(1)),
            ("Contents.json", b"{}".to_vec()),
        ];
        let bytes = icns_from_files(&files).unwrap();
        let kinds: Vec<String> = decode_icns(&bytes).unwrap().into_iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, vec!["icp4", "ic10"]);
    }

    #[test]
    fn iconset_refuses_non_png_and_empty_sets() {
        assert!(icns_from_files(&[("icon_16x16.png", b"not a png".to_vec())]).is_err());
        assert!(icns_from_files(&[("Contents.json", b"{}".to_vec())]).is_err());
    }

    #[test]
    fn icns_unpacks_to_iconset_names_and_skips_legacy_data() {
        let legacy = {
            let mut v = b"il32".to_vec();
            v.extend_from_slice(&12u32.to_be_bytes());
            v.extend_from_slice(&[0; 4]);
            v
        };
        let mut bytes = encode_icns(&[("ic07", png(3).as_slice())]).unwrap();
        let total = bytes.len() + legacy.len();
        bytes[4..8].copy_from_slice(&(total as u32).to_be_bytes());
        bytes.extend_from_slice(&legacy);
        let files = iconset_files_from_icns(&bytes).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "icon_128x128.png");
        assert_eq!(files[0].1, png(3).as_slice());
    }
}
