//! `tiff2icns`: converts an image into an icon file (ICNS) with one PNG per icon size (`tiff2icns INPUT [OUTPUT.icns]`).
//!
//! The image is decoded and scaled by ImageMagick (`magick`, from the imagemagick package), which reads TIFF and most
//! other raster formats. The first frame is used, scaled to each size and centred on a transparent square. This tool
//! writes the ICNS container itself: the header and one element per size, each element a PNG with its type code.
//! The sizes are the PNG-only types: `ic07` (128), `ic08` (256), `ic09` (512), `ic10` (1024) and the 2x types `ic11`
//! (32), `ic12` (64), `ic13` (256), `ic14` (512).
//! Gap: the older 16 and 32 pixel types (`icp4`, `icp5`) and the 48 pixel type are not written, and the options of the
//! reference tool (`-noLarge` and the rest) are refused.

use std::process::Command;

pub const USAGE: &str = "usage: tiff2icns INPUT [OUTPUT.icns]";
/// The ICNS file header: the four bytes `icns`.
pub const MAGIC: &[u8; 4] = b"icns";
/// The eight-byte signature every PNG file starts with.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// Each icon element: its type code and the pixel size of its PNG.
pub const ICON_TYPES: &[(&str, u32)] = &[
    ("ic11", 32),
    ("ic12", 64),
    ("ic07", 128),
    ("ic13", 256),
    ("ic08", 256),
    ("ic14", 512),
    ("ic09", 512),
    ("ic10", 1024),
];

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run { input: String, output: String },
}

/// Reads the arguments (without the program name). The output defaults to the input name with the `.icns` extension.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut files: Vec<&String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-h" | "-help" | "--help" => return Ok(Parsed::Help),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unsupported option '{other}'\n{USAGE}"));
            }
            _ => files.push(arg),
        }
    }
    match files.as_slice() {
        [input] => Ok(Parsed::Run { input: (*input).clone(), output: default_output(input) }),
        [input, output] => Ok(Parsed::Run { input: (*input).clone(), output: (*output).clone() }),
        [] => Err(format!("no input file\n{USAGE}")),
        _ => Err(format!("too many files\n{USAGE}")),
    }
}

/// The output name for `input`: the same path with the `.icns` extension.
pub fn default_output(input: &str) -> String {
    std::path::Path::new(input).with_extension("icns").to_string_lossy().into_owned()
}

/// Whether the bytes start with the PNG signature.
pub fn is_png(data: &[u8]) -> bool {
    data.starts_with(&PNG_SIGNATURE)
}

/// Builds the ICNS file from `(type code, PNG bytes)` elements: the header `icns` and the total length, then each
/// element as its four-byte type, its own length (type and length included) and its data.
pub fn build_icns(elements: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let body: usize = elements.iter().map(|(_, data)| 8 + data.len()).sum();
    let total = u32::try_from(8 + body).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(8 + body);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&total.to_be_bytes());
    for (code, data) in elements {
        let length = u32::try_from(8 + data.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(code.as_bytes());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// The ImageMagick arguments that scale the first frame of `input` to a `size` square with transparent padding and
/// write it as PNG to standard output.
pub fn magick_args(input: &str, size: u32) -> Vec<String> {
    let square = format!("{size}x{size}");
    vec![
        format!("{input}[0]"),
        "-background".to_string(),
        "none".to_string(),
        "-resize".to_string(),
        square.clone(),
        "-gravity".to_string(),
        "center".to_string(),
        "-extent".to_string(),
        square,
        "png:-".to_string(),
    ]
}

/// Converts `input` to an ICNS file at `output`.
pub fn run(input: &str, output: &str) -> Result<(), String> {
    let mut elements: Vec<(&str, Vec<u8>)> = Vec::with_capacity(ICON_TYPES.len());
    for (code, size) in ICON_TYPES {
        let result = Command::new("magick").args(magick_args(input, *size)).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "magick is not installed (the imagemagick package)".to_string()
            } else {
                format!("magick: {e}")
            }
        })?;
        if !result.status.success() {
            let message = String::from_utf8_lossy(&result.stderr).trim().to_string();
            return Err(format!("{input}: cannot read the image: {message}"));
        }
        if !is_png(&result.stdout) {
            return Err(format!("{input}: magick did not produce a PNG for size {size}"));
        }
        elements.push((*code, result.stdout));
    }
    std::fs::write(output, build_icns(&elements)).map_err(|e| format!("{output}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_defaults_the_output_to_icns() {
        assert_eq!(
            parse(&args(&["art/logo.tiff"])).unwrap(),
            Parsed::Run { input: "art/logo.tiff".to_string(), output: "art/logo.icns".to_string() }
        );
        assert_eq!(
            parse(&args(&["logo.tiff", "app.icns"])).unwrap(),
            Parsed::Run { input: "logo.tiff".to_string(), output: "app.icns".to_string() }
        );
    }

    #[test]
    fn parse_refuses_options_and_extra_files() {
        assert!(parse(&args(&["-noLarge", "logo.tiff"])).is_err());
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["a.tiff", "b.icns", "c"])).is_err());
        assert_eq!(parse(&args(&["-h"])).unwrap(), Parsed::Help);
    }

    #[test]
    fn build_icns_writes_header_and_elements() {
        let png = PNG_SIGNATURE.to_vec();
        let icns = build_icns(&[("ic07", png.clone()), ("ic10", vec![1, 2, 3])]);
        assert_eq!(&icns[0..4], b"icns");
        let total = u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize;
        assert_eq!(total, icns.len());
        assert_eq!(&icns[8..12], b"ic07");
        let first_len = u32::from_be_bytes([icns[12], icns[13], icns[14], icns[15]]) as usize;
        assert_eq!(first_len, 8 + png.len());
        assert_eq!(&icns[16..16 + png.len()], png.as_slice());
        let second = 16 + png.len();
        assert_eq!(&icns[second..second + 4], b"ic10");
        assert_eq!(&icns[second + 8..], &[1u8, 2, 3][..]);
    }

    #[test]
    fn build_icns_with_no_elements_is_just_the_header() {
        assert_eq!(build_icns(&[]), vec![b'i', b'c', b'n', b's', 0, 0, 0, 8]);
    }

    #[test]
    fn icon_types_use_the_expected_sizes() {
        let size_of = |code: &str| ICON_TYPES.iter().find(|(c, _)| *c == code).map(|(_, s)| *s);
        assert_eq!(size_of("ic07"), Some(128));
        assert_eq!(size_of("ic10"), Some(1024));
        assert_eq!(size_of("ic11"), Some(32));
        assert_eq!(size_of("ic14"), Some(512));
        assert!(ICON_TYPES.iter().all(|(code, _)| code.len() == 4));
    }

    #[test]
    fn png_check_reads_the_signature() {
        assert!(is_png(&PNG_SIGNATURE));
        assert!(!is_png(b"II*\0"));
        assert!(!is_png(&[]));
    }

    #[test]
    fn magick_scales_the_first_frame_into_a_square() {
        let a = magick_args("logo.tiff", 256);
        assert_eq!(a[0], "logo.tiff[0]");
        assert!(a.windows(2).any(|w| w[0] == "-resize" && w[1] == "256x256"));
        assert!(a.windows(2).any(|w| w[0] == "-extent" && w[1] == "256x256"));
        assert_eq!(a.last().map(String::as_str), Some("png:-"));
    }
}
