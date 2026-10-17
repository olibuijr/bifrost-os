//! `sips`: reads an image's properties (`sips -g KEY FILE...`). See `bifrost_cmd::sips`.
//!
//! Supported: `-g pixelWidth`, `-g pixelHeight` and `-g format` (also `--getProperty KEY`), one or more keys, and one or
//! more files. The size and format come from the file's header, so no pixel data is decoded. PNG, GIF and JPEG are
//! recognised.
//! Resampling: `-Z N` (or `--resampleHeightWidthMax N`) makes the largest side N, never enlarging; `--resampleWidth W`
//! and `--resampleHeight H` set one side and keep the aspect. The picture is decoded with the shell's image decoders
//! (`bifrost_ui::cg::imageio`), resized, and written as PNG: in place, or to `--out FILE.png` (one file only). A JPEG or
//! GIF is written back only with `--out` naming a `.png`, because resampling writes PNG or JPEG output only.
//! Conversion: `-s format png|jpeg|gif FILE --out FILE` decodes the picture and writes it in the new format (`--out` is
//! required, so the source is never overwritten). GIF output uses a fixed 3-3-2 palette (256 colours) and LZW.
//! Refused with a message: other properties to set, rotation, flips, DPI and colour profile properties,
//! and every other option. Those need an image pipeline; the message names `magick` from the imagemagick package for them.
//! Gap: TIFF, BMP, HEIC and the other formats are reported as unsupported.

pub const USAGE: &str = "usage: sips -g pixelWidth | pixelHeight | format [-g KEY ...] FILE ...\n       sips -Z N | --resampleWidth W | --resampleHeight H [--out FILE.png] FILE\n       sips -s format png|jpeg|gif --out OUTFILE FILE";

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

/// How a resample sizes the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `-Z N`: the largest side becomes N (a smaller picture is left as it is)
    Max(u32),
    /// `--resampleWidth W`: the width becomes W and the height follows the aspect
    Width(u32),
    /// `--resampleHeight H`: the height becomes H and the width follows the aspect
    Height(u32),
}

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Query { keys: Vec<Key>, files: Vec<String> },
    Resample { target: Target, out: Option<String>, files: Vec<String> },
    Convert { to: Format, out: String, file: String },
}

/// The format a `-s format VALUE` names. PNG, JPEG and GIF can be written.
fn output_format(value: &str) -> Result<Format, String> {
    match value.to_ascii_lowercase().as_str() {
        "png" => Ok(Format::Png),
        "jpeg" | "jpg" => Ok(Format::Jpeg),
        "gif" => Ok(Format::Gif),
        other => Err(format!("format {other} cannot be written; sips here writes png, jpeg and gif")),
    }
}

/// The positive number after option `arg` at `args[i]`.
fn number(args: &[String], i: usize, arg: &str) -> Result<u32, String> {
    let v = args.get(i + 1).ok_or_else(|| format!("{arg} needs a number"))?;
    match v.parse::<u32>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!("{arg} needs a positive number, not {v}")),
    }
}

/// Reads the arguments. Properties come from `-g KEY` (or `--getProperty KEY`); a resample from `-Z`, `--resampleWidth`
/// or `--resampleHeight`, with `--out` for the file to write. Files are the operands.
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut keys = Vec::new();
    let mut files = Vec::new();
    let mut target = None;
    let mut out = None;
    let mut convert: Option<Format> = None;
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
            "-Z" | "--resampleHeightWidthMax" => {
                target = Some(Target::Max(number(args, i, arg)?));
                i += 2;
                continue;
            }
            "--resampleWidth" => {
                target = Some(Target::Width(number(args, i, arg)?));
                i += 2;
                continue;
            }
            "--resampleHeight" => {
                target = Some(Target::Height(number(args, i, arg)?));
                i += 2;
                continue;
            }
            "-s" | "--setProperty" => {
                let name = args.get(i + 1).ok_or_else(|| format!("{arg} needs a property name and a value"))?;
                let value = args.get(i + 2).ok_or_else(|| format!("{arg} {name} needs a value"))?;
                if name != "format" {
                    return Err(format!("property {name} cannot be set yet; sips here sets format"));
                }
                convert = Some(output_format(value)?);
                i += 3;
                continue;
            }
            "--out" => {
                out = Some(args.get(i + 1).ok_or_else(|| "--out needs a file name".to_string())?.clone());
                i += 2;
                continue;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("option {arg} is not supported yet; sips here reads properties with -g and resamples with -Z"));
            }
            _ => files.push(arg.to_string()),
        }
        i += 1;
    }
    if let Some(to) = convert {
        if target.is_some() || !keys.is_empty() {
            return Err("a conversion is its own run: give -s alone".to_string());
        }
        if files.len() != 1 {
            return Err("a conversion takes one image file".to_string());
        }
        let out = out.ok_or_else(|| "a conversion needs --out FILE with the new extension".to_string())?;
        return Ok(Action::Convert { to, out, file: files.remove(0) });
    }
    if let Some(target) = target {
        if !keys.is_empty() {
            return Err("a resample and -g are separate runs: give one or the other".to_string());
        }
        if files.is_empty() {
            return Err("sips needs at least one image file".to_string());
        }
        if out.is_some() && files.len() != 1 {
            return Err("--out takes one image file".to_string());
        }
        return Ok(Action::Resample { target, out, files });
    }
    if out.is_some() {
        return Err("--out is only for a resample".to_string());
    }
    if keys.is_empty() {
        return Err("sips needs -g pixelWidth, -g pixelHeight or -g format".to_string());
    }
    if files.is_empty() {
        return Err("sips needs at least one image file".to_string());
    }
    Ok(Action::Query { keys, files })
}

/// The size a `width` x `height` picture takes for `target`. `Max` never enlarges; a width or height is asked for exactly.
pub fn target_size(width: usize, height: usize, target: Target) -> Result<(usize, usize), String> {
    if width == 0 || height == 0 {
        return Err("the picture has no pixels".to_string());
    }
    let scaled = |k: f64| (((width as f64 * k).round() as usize).max(1), ((height as f64 * k).round() as usize).max(1));
    Ok(match target {
        Target::Max(n) => {
            let n = n as usize;
            if width <= n && height <= n {
                (width, height)
            } else {
                scaled(n as f64 / width.max(height) as f64)
            }
        }
        Target::Width(w) => (w as usize, ((height as f64 * w as f64 / width as f64).round() as usize).max(1)),
        Target::Height(h) => (((width as f64 * h as f64 / height as f64).round() as usize).max(1), h as usize),
    })
}

/// Resamples the picture in `data` to `target` and returns it as PNG bytes. The picture is decoded with the shell's
/// image decoders. `out_is_png` says the result will be written as a `.png` file, which a JPEG or GIF may be.
pub fn resample(data: &[u8], target: Target, out_is_png: bool) -> Result<Vec<u8>, String> {
    let info = info(data)?;
    if info.format != Format::Png && !out_is_png {
        return Err(format!("{} pictures are written back only as PNG: give --out a .png file", info.format.name()));
    }
    let img = bifrost_ui::cg::imageio::decode(data).ok_or_else(|| "the picture could not be decoded".to_string())?;
    let (w, h) = target_size(img.w, img.h, target)?;
    let img = if (w, h) == (img.w, img.h) { img } else { img.resized(w, h) };
    Ok(springboard::pngenc::encode_rgb(img.w, img.h, &img.px))
}

/// The JPEG quality used by a conversion, 0.0 to 1.0.
const JPEG_QUALITY: f32 = 0.8;

/// Converts the picture in `data` to `to` (PNG or JPEG) and returns the new file's bytes. The picture is decoded with the
/// shell's image decoders; alpha is dropped, as a JPEG has none.
pub fn convert(data: &[u8], to: Format) -> Result<Vec<u8>, String> {
    info(data)?;
    let img = bifrost_ui::cg::imageio::decode(data).ok_or_else(|| "the picture could not be decoded".to_string())?;
    match to {
        Format::Png => Ok(springboard::pngenc::encode_rgb(img.w, img.h, &img.px)),
        Format::Jpeg => {
            let rgba: Vec<u8> = img.px.iter().flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8, 255]).collect();
            bifrost_app::graphics_imageio::encode_jpeg(img.w, img.h, &rgba, JPEG_QUALITY)
                .ok_or_else(|| "the picture could not be encoded as JPEG".to_string())
        }
        Format::Gif => Ok(gif_encode(img.w, img.h, &img.px)),
    }
}

/// The GIF palette: 3 bits of red, 3 of green and 2 of blue, so index `i` is `r<<5 | g<<2 | b`.
fn gif_palette() -> Vec<u8> {
    (0..256u32)
        .flat_map(|i| {
            let r = ((i >> 5) & 7) * 255 / 7;
            let g = ((i >> 2) & 7) * 255 / 7;
            let b = (i & 3) * 255 / 3;
            [r as u8, g as u8, b as u8]
        })
        .collect()
}

/// The palette index of a pixel in the 3-3-2 palette. Pixels are read the way the JPEG path reads them: low byte red.
fn gif_index(p: u32) -> u8 {
    let (r, g, b) = ((p & 0xff) as u8, ((p >> 8) & 0xff) as u8, ((p >> 16) & 0xff) as u8);
    ((r >> 5) << 5) | ((g >> 5) << 2) | (b >> 6)
}

/// Writes a GIF89a of `w` x `h` pixels, one image with the 3-3-2 palette. Alpha is dropped.
pub fn gif_encode(w: usize, h: usize, px: &[u32]) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.push(0xF7); // global colour table present, 8 bits per colour, 256 entries
    out.push(0); // background colour index
    out.push(0); // pixel aspect ratio
    out.extend_from_slice(&gif_palette());
    out.push(0x2C); // image descriptor
    out.extend_from_slice(&[0, 0, 0, 0]); // left and top
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.push(0); // no local colour table, not interlaced
    out.push(8); // minimum LZW code size
    let indices: Vec<u8> = px.iter().take(w * h).map(|&p| gif_index(p)).collect();
    for block in lzw_encode(&indices).chunks(255) {
        out.push(block.len() as u8);
        out.extend_from_slice(block);
    }
    out.push(0); // end of the image data
    out.push(0x3B); // trailer
    out
}

/// The width of a GIF code while the encoder's next free code is `next`: the decoder lags one entry behind, so it is
/// the bits needed for `next - 1`, never less than 9 and never more than 12.
fn code_width(next: u32) -> u32 {
    let mut w = 9;
    while w < 12 && next - 1 >= (1 << w) {
        w += 1;
    }
    w
}

/// Packs GIF LZW codes least significant bit first.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, code: u32, width: u32) {
        self.acc |= code << self.n;
        self.n += width;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
}

/// The LZW stream for palette indices, with clear and end codes, as a GIF image needs it.
fn lzw_encode(indices: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;
    const CLEAR: u32 = 256;
    const END: u32 = 257;
    let mut bits = Bits { out: Vec::new(), acc: 0, n: 0 };
    let mut dict: HashMap<(u32, u8), u32> = HashMap::new();
    let mut next = 258u32;
    bits.put(CLEAR, 9);
    let mut cur: Option<u32> = None;
    for &b in indices {
        let Some(p) = cur else {
            cur = Some(b as u32);
            continue;
        };
        if let Some(&code) = dict.get(&(p, b)) {
            cur = Some(code);
            continue;
        }
        bits.put(p, code_width(next));
        if next < 4096 {
            dict.insert((p, b), next);
            next += 1;
        }
        if next == 4096 {
            bits.put(CLEAR, 12);
            dict.clear();
            next = 258;
        }
        cur = Some(b as u32);
    }
    if let Some(p) = cur {
        bits.put(p, code_width(next));
    }
    // The decoder has one entry more than the encoder at the end, so the end code uses the width of `next + 1`.
    bits.put(END, code_width(next + 1));
    if bits.n > 0 {
        bits.out.push(bits.acc as u8);
    }
    bits.out
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
    fn resample_options_parse_and_size_pictures() {
        assert_eq!(parse(&args(&["-Z", "100", "a.png"])), Ok(Action::Resample { target: Target::Max(100), out: None, files: vec!["a.png".into()] }));
        assert_eq!(
            parse(&args(&["--resampleWidth", "64", "--out", "b.png", "a.jpg"])),
            Ok(Action::Resample { target: Target::Width(64), out: Some("b.png".into()), files: vec!["a.jpg".into()] })
        );
        assert!(parse(&args(&["-Z", "0", "a.png"])).unwrap_err().contains("positive"));
        assert!(parse(&args(&["-Z", "9", "-g", "format", "a.png"])).unwrap_err().contains("separate runs"));
        assert!(parse(&args(&["-Z", "9", "a.png", "b.png", "--out", "c.png"])).unwrap_err().contains("one image"));
        assert!(parse(&args(&["--out", "c.png", "a.png"])).unwrap_err().contains("only for a resample"));
        assert_eq!(target_size(400, 200, Target::Max(100)), Ok((100, 50)));
        assert_eq!(target_size(50, 40, Target::Max(100)), Ok((50, 40)), "never enlarged");
        assert_eq!(target_size(400, 300, Target::Width(200)), Ok((200, 150)));
        assert_eq!(target_size(400, 300, Target::Height(150)), Ok((200, 150)));
        assert!(target_size(0, 3, Target::Max(9)).is_err());
    }

    #[test]
    fn a_png_resamples_to_a_smaller_png() {
        let px = vec![0xff00_00ffu32; 8]; // 4 x 2
        let png = springboard::pngenc::encode_rgb(4, 2, &px);
        let small = resample(&png, Target::Max(2), false).unwrap();
        let back = info(&small).unwrap();
        assert_eq!((back.width, back.height), (2, 1));
        assert_eq!(back.format, Format::Png);
    }

    #[test]
    fn parses_a_conversion_to_png_or_jpeg() {
        assert_eq!(
            parse(&args(&["-s", "format", "jpeg", "a.png", "--out", "b.jpg"])).unwrap(),
            Action::Convert { to: Format::Jpeg, out: "b.jpg".to_string(), file: "a.png".to_string() }
        );
        assert_eq!(
            parse(&args(&["--setProperty", "format", "PNG", "a.gif", "--out", "b.png"])).unwrap(),
            Action::Convert { to: Format::Png, out: "b.png".to_string(), file: "a.gif".to_string() }
        );
        assert!(parse(&args(&["-s", "format", "jpeg", "a.png", "b.png", "--out", "c.jpg"])).unwrap_err().contains("one image"));
        assert!(parse(&args(&["-s", "format", "jpeg", "-Z", "9", "a.png", "--out", "b.jpg"])).unwrap_err().contains("own run"));
    }

    #[test]
    fn a_png_converts_to_a_jpeg_and_back() {
        let px = vec![0xff00_00ffu32; 8]; // 4 x 2
        let png = springboard::pngenc::encode_rgb(4, 2, &px);
        let jpeg = convert(&png, Format::Jpeg).unwrap();
        assert_eq!(info(&jpeg).unwrap(), Info { format: Format::Jpeg, width: 4, height: 2 });
        let back = convert(&jpeg, Format::Png).unwrap();
        assert_eq!(info(&back).unwrap(), Info { format: Format::Png, width: 4, height: 2 });
    }

    /// Decodes the LZW stream of a GIF written by `gif_encode` (one image, 8-bit minimum code size) into indices.
    fn gif_indices(gif: &[u8]) -> Vec<u8> {
        let start = 13 + 3 * (1 << ((gif[10] & 7) + 1)); // header, global colour table, then the image descriptor
        assert_eq!(gif[start], 0x2C);
        let mut data = Vec::new();
        let mut at = start + 11;
        loop {
            let n = gif[at] as usize;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&gif[at + 1..at + 1 + n]);
            at += 1 + n;
        }
        let mut table: Vec<Vec<u8>> = (0..256).map(|i| vec![i as u8]).collect();
        table.push(Vec::new());
        table.push(Vec::new());
        let (mut width, mut bit, mut out, mut prev) = (9usize, 0usize, Vec::new(), None::<Vec<u8>>);
        while bit + width <= data.len() * 8 {
            let mut code = 0usize;
            for i in 0..width {
                code |= (((data[(bit + i) / 8] >> ((bit + i) % 8)) & 1) as usize) << i;
            }
            bit += width;
            if code == 256 {
                table.truncate(258);
                width = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                break;
            }
            let entry = match (code < table.len(), &prev) {
                (true, _) => table[code].clone(),
                (false, Some(p)) => {
                    let mut e = p.clone();
                    e.push(p[0]);
                    e
                }
                (false, None) => panic!("first code after a clear is not in the table"),
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev {
                let mut ne = p;
                ne.push(entry[0]);
                table.push(ne);
            }
            prev = Some(entry);
            if table.len() >= 1 << width && width < 12 {
                width += 1;
            }
        }
        out
    }

    #[test]
    fn a_png_converts_to_a_gif_with_the_332_palette() {
        // 3 x 2: red, green, blue, white, black, red. Indices are r<<5 | g<<2 | b.
        let px = [0x0000_00ffu32, 0x0000_ff00, 0x00ff_0000, 0x00ff_ffff, 0x0000_0000, 0x0000_00ff];
        let png = springboard::pngenc::encode_rgb(3, 2, &px);
        let gif = convert(&png, Format::Gif).unwrap();
        assert_eq!(info(&gif).unwrap(), Info { format: Format::Gif, width: 3, height: 2 });
        assert_eq!(gif_indices(&gif), vec![224, 28, 3, 255, 0, 224]);
        // Palette entry 224 is red.
        let palette = &gif[13..13 + 768];
        assert_eq!(&palette[224 * 3..224 * 3 + 3], &[255, 0, 0]);
    }

    #[test]
    fn a_large_gif_resets_its_code_table_and_round_trips() {
        // Enough pixels to fill the 4096-entry LZW table several times over and grow the code width to 12 bits.
        let (w, h) = (120usize, 100usize);
        let px: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2_654_435_761)).collect();
        let expected: Vec<u8> = px.iter().map(|&p| gif_index(p)).collect();
        assert_eq!(gif_indices(&gif_encode(w, h, &px)), expected);
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        assert!(parse(&args(&["-s", "format", "bmp", "a.png", "--out", "b.bmp"])).unwrap_err().contains("bmp"));
        assert!(parse(&args(&["-s", "dpiWidth", "72", "a.png", "--out", "b.png"])).unwrap_err().contains("dpiWidth"));
        assert!(parse(&args(&["-s", "format", "jpeg", "a.png"])).unwrap_err().contains("--out"));
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
