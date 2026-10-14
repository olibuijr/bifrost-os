//! `sips`: reads an image's properties (`sips -g KEY FILE...`). See `bifrost_cmd::sips`.
//!
//! Supported: `-g pixelWidth`, `-g pixelHeight` and `-g format` (also `--getProperty KEY`), one or more keys, and one or
//! more files. The size and format come from the file's header, so no pixel data is decoded. PNG, GIF and JPEG are
//! recognised.
//! Refused with a message: resampling (`-Z` and the `--resample*` options), conversion (`-s` and `--setProperty`),
//! rotation, flips, DPI and colour profile properties, and every other option. Those need an image pipeline; the
//! message names `magick` from the imagemagick package for them.
//! Gap: TIFF, BMP, HEIC and the other formats are reported as unsupported.

pub const USAGE: &str = "usage: sips -g pixelWidth | pixelHeight | format [-g KEY ...] FILE ...";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The image formats the header reader recognises. The names are the ones `sips` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Gif,
    Jpeg,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Gif => "gif",
            Format::Jpeg => "jpeg",
        }
    }
}

/// The facts read from an image header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub format: Format,
    pub width: u32,
    pub height: u32,
}

/// One property the command can print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    PixelWidth,
    PixelHeight,
    Format,
}

impl Key {
    /// The property named on the command line. Unknown names are refused with the list of supported ones.
    pub fn parse(name: &str) -> Result<Key, String> {
        match name {
            "pixelWidth" => Ok(Key::PixelWidth),
            "pixelHeight" => Ok(Key::PixelHeight),
            "format" => Ok(Key::Format),
            other => Err(format!("property {other} is not supported yet; sips here reads pixelWidth, pixelHeight and format")),
        }
    }

    /// The property name as printed.
    pub fn label(self) -> &'static str {
        match self {
            Key::PixelWidth => "pixelWidth",
            Key::PixelHeight => "pixelHeight",
            Key::Format => "format",
        }
    }

    /// The property's value for an image.
    pub fn value(self, info: &Info) -> String {
        match self {
            Key::PixelWidth => info.width.to_string(),
            Key::PixelHeight => info.height.to_string(),
            Key::Format => info.format.name().to_string(),
        }
    }
}

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Query { keys: Vec<Key>, files: Vec<String> },
}

/// Reads the arguments. Properties come from `-g KEY` (or `--getProperty KEY`), files are the operands.
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut keys = Vec::new();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "-help" | "--help" => return Ok(Action::Help),
            "-g" | "--getProperty" => {
                let name = args.get(i + 1).ok_or_else(|| format!("{arg} needs a property name"))?;
                keys.push(Key::parse(name)?);
                i += 2;
                continue;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("option {arg} is not supported yet; sips here only reads properties with -g"));
            }
            _ => files.push(arg.to_string()),
        }
        i += 1;
    }
    if keys.is_empty() {
        return Err("sips needs -g pixelWidth, -g pixelHeight or -g format".to_string());
    }
    if files.is_empty() {
        return Err("sips needs at least one image file".to_string());
    }
    Ok(Action::Query { keys, files })
}

/// Reads the format and size from the header of an image file's bytes.
pub fn info(data: &[u8]) -> Result<Info, String> {
    if data.starts_with(&PNG_SIGNATURE) {
        png_info(data)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        gif_info(data)
    } else if data.starts_with(&[0xFF, 0xD8]) {
        jpeg_info(data)
    } else {
        Err("unsupported file format".to_string())
    }
}

/// The text `sips -g` prints for one file: the file name, then one `  property: value` line per key.
pub fn render(file: &str, info: &Info, keys: &[Key]) -> String {
    let mut out = format!("{file}\n");
    for key in keys {
        out.push_str(&format!("  {}: {}\n", key.label(), key.value(info)));
    }
    out
}

fn be_u16(d: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]) as u32)
}

fn le_u16(d: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]) as u32)
}

fn be_u32(d: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = d.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// The IHDR chunk comes first in a PNG file, with the width and height as big-endian 32-bit numbers.
fn png_info(d: &[u8]) -> Result<Info, String> {
    if d.get(12..16) != Some(&b"IHDR"[..]) {
        return Err("corrupt PNG: no IHDR chunk".to_string());
    }
    let width = be_u32(d, 16).ok_or("truncated PNG header")?;
    let height = be_u32(d, 20).ok_or("truncated PNG header")?;
    Ok(Info { format: Format::Png, width, height })
}

/// The logical screen size comes right after the signature, as little-endian 16-bit numbers.
fn gif_info(d: &[u8]) -> Result<Info, String> {
    let width = le_u16(d, 6).ok_or("truncated GIF header")?;
    let height = le_u16(d, 8).ok_or("truncated GIF header")?;
    Ok(Info { format: Format::Gif, width, height })
}

/// Start-of-frame markers carry the sample precision, then the height and the width. The table and coding markers
/// that share the C0 to CF range (DHT, JPG, DAC) are not frame headers.
fn is_frame_header(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// Walks the JPEG segments until the first frame header. Segments carry a 16-bit length that counts itself.
fn jpeg_info(d: &[u8]) -> Result<Info, String> {
    let mut i = 2;
    while i + 1 < d.len() {
        if d[i] != 0xFF {
            return Err("corrupt JPEG: marker expected".to_string());
        }
        let marker = d[i + 1];
        match marker {
            0xFF => {
                i += 1;
                continue;
            }
            0x01 | 0xD0..=0xD7 => {
                i += 2;
                continue;
            }
            0xD9 | 0xDA => break,
            _ => {}
        }
        if is_frame_header(marker) {
            let height = be_u16(d, i + 5).ok_or("truncated JPEG frame header")?;
            let width = be_u16(d, i + 7).ok_or("truncated JPEG frame header")?;
            return Ok(Info { format: Format::Jpeg, width, height });
        }
        let len = be_u16(d, i + 2).ok_or("truncated JPEG segment")? as usize;
        if len < 2 {
            return Err("corrupt JPEG: bad segment length".to_string());
        }
        i += 2 + len;
    }
    Err("JPEG has no frame header".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut d = PNG_SIGNATURE.to_vec();
        d.extend_from_slice(&13u32.to_be_bytes());
        d.extend_from_slice(b"IHDR");
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.extend_from_slice(&[8, 2, 0, 0, 0]);
        d
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut d = b"GIF89a".to_vec();
        d.extend_from_slice(&width.to_le_bytes());
        d.extend_from_slice(&height.to_le_bytes());
        d
    }

    #[test]
    fn parses_properties_and_files() {
        let a = parse(&args(&["-g", "pixelWidth", "-g", "format", "a.png"])).unwrap();
        assert_eq!(a, Action::Query { keys: vec![Key::PixelWidth, Key::Format], files: vec!["a.png".to_string()] });
    }

    #[test]
    fn accepts_the_long_option_and_several_files() {
        let a = parse(&args(&["--getProperty", "pixelHeight", "x.jpg", "y.gif"])).unwrap();
        assert_eq!(a, Action::Query { keys: vec![Key::PixelHeight], files: vec!["x.jpg".to_string(), "y.gif".to_string()] });
    }

    #[test]
    fn help_is_recognised() {
        assert_eq!(parse(&args(&["-h"])).unwrap(), Action::Help);
        assert_eq!(parse(&args(&["--help"])).unwrap(), Action::Help);
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        assert!(parse(&args(&["-Z", "100", "a.png"])).unwrap_err().contains("-Z"));
        assert!(parse(&args(&["-s", "format", "png", "a.png"])).unwrap_err().contains("-s"));
        assert!(parse(&args(&["-g", "dpiWidth", "a.png"])).unwrap_err().contains("dpiWidth"));
        assert!(parse(&args(&["-g"])).unwrap_err().contains("needs a property name"));
        assert!(parse(&args(&["-g", "format"])).unwrap_err().contains("at least one image file"));
        assert!(parse(&args(&["a.png"])).unwrap_err().contains("-g pixelWidth"));
    }

    #[test]
    fn key_names_round_trip() {
        for key in [Key::PixelWidth, Key::PixelHeight, Key::Format] {
            assert_eq!(Key::parse(key.label()).unwrap(), key);
        }
    }

    #[test]
    fn reads_png_size() {
        assert_eq!(info(&png(640, 480)).unwrap(), Info { format: Format::Png, width: 640, height: 480 });
    }

    #[test]
    fn rejects_png_without_ihdr() {
        let mut d = png(1, 1);
        d[12..16].copy_from_slice(b"IDAT");
        assert!(info(&d).unwrap_err().contains("IHDR"));
        assert!(info(&PNG_SIGNATURE).unwrap_err().contains("IHDR"));
    }

    #[test]
    fn reads_gif_size() {
        assert_eq!(info(&gif(320, 200)).unwrap(), Info { format: Format::Gif, width: 320, height: 200 });
        assert!(info(b"GIF89a\x01").unwrap_err().contains("truncated"));
    }

    #[test]
    fn reads_jpeg_size_past_other_segments() {
        let mut d = vec![0xFF, 0xD8];
        d.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0xAA, 0xBB]); // APP0 with two payload bytes
        d.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x02]); // DHT: a table marker in the C range, not a frame
        d.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0x03]);
        assert_eq!(info(&d).unwrap(), Info { format: Format::Jpeg, width: 640, height: 480 });
    }

    #[test]
    fn jpeg_without_frame_header_is_refused() {
        let d = vec![0xFF, 0xD8, 0xFF, 0xD9];
        assert!(info(&d).unwrap_err().contains("no frame header"));
        let bad = vec![0xFF, 0xD8, 0x00, 0x00];
        assert!(info(&bad).unwrap_err().contains("marker expected"));
    }

    #[test]
    fn unknown_bytes_are_unsupported() {
        assert_eq!(info(b"hello world").unwrap_err(), "unsupported file format");
    }

    #[test]
    fn renders_one_line_per_key() {
        let info = Info { format: Format::Png, width: 2, height: 3 };
        assert_eq!(
            render("a.png", &info, &[Key::PixelWidth, Key::PixelHeight, Key::Format]),
            "a.png\n  pixelWidth: 2\n  pixelHeight: 3\n  format: png\n"
        );
    }
}
