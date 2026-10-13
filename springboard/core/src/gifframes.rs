//! Animated GIF frames: a GIF 89a reader that returns every image of the file with its frame
//! rectangle, palette, delay and disposal, plus a compositor that draws the frames onto a
//! full RGBA canvas (std only).
//!
//! The reader follows the GIF 89a block structure: the logical screen descriptor, the global
//! colour table, then a sequence of extension blocks (Graphic Control, NETSCAPE loop count) and
//! image descriptors until the trailer byte. Image data is LZW coded with a variable code size
//! of 3 to 12 bits. Interlaced frames are put back into row order. The module reads no files
//! and opens no socket: the caller hands it the bytes it already has.

/// What to do with a frame's rectangle before the next frame is drawn (GIF disposal method).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposal {
    /// No method given, or method 0.
    Unspecified,
    /// Leave the pixels in place (method 1).
    KeepInPlace,
    /// Clear the rectangle to transparent (method 2).
    RestoreBackground,
    /// Restore the canvas as it was before this frame was drawn (method 3).
    RestorePrevious,
}

/// One image of an animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifFrame {
    /// Position of the frame rectangle on the logical screen.
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    /// Time this frame stays on screen, in milliseconds (the file stores hundredths of a second).
    pub delay_ms: u32,
    pub disposal: Disposal,
    /// Palette index that is see-through in this frame, when the Graphic Control block sets it.
    pub transparent: Option<u8>,
    /// The local colour table, or the global one when the frame has no local table.
    pub palette: Vec<[u8; 3]>,
    /// One palette index per pixel, row by row, `width * height` entries.
    pub indices: Vec<u8>,
}

/// A decoded GIF animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifAnimation {
    /// Logical screen size.
    pub width: u16,
    pub height: u16,
    /// NETSCAPE loop count: `Some(0)` loops forever, `Some(n)` plays n extra times, `None`
    /// when the file has no loop extension (play once).
    pub loop_count: Option<u16>,
    pub frames: Vec<GifFrame>,
}

/// Why a GIF could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GifError {
    /// The bytes do not start with a GIF signature.
    Signature,
    /// The data ends before the trailer or inside a block.
    Truncated,
    /// A block is malformed: unknown introducer, missing palette, or bad LZW data.
    Corrupt,
}

/// Largest LZW code table a GIF can declare (12-bit codes).
const MAX_CODES: usize = 4096;

/// Reads every frame of a GIF 87a or 89a file.
pub fn decode(buf: &[u8]) -> Result<GifAnimation, GifError> {
    if !(buf.starts_with(b"GIF87a") || buf.starts_with(b"GIF89a")) {
        return Err(GifError::Signature);
    }
    if buf.len() < 13 {
        return Err(GifError::Truncated);
    }
    let width = le16(buf, 6);
    let height = le16(buf, 8);
    let packed = buf[10];
    let mut pos = 13;
    let global = if packed & 0x80 != 0 {
        let (table, next) = read_table(buf, pos, packed)?;
        pos = next;
        Some(table)
    } else {
        None
    };

    let mut anim = GifAnimation { width, height, loop_count: None, frames: Vec::new() };
    // Graphic Control state, valid for the next image only.
    let mut disposal = Disposal::Unspecified;
    let mut delay_ms = 0u32;
    let mut transparent = None;

    loop {
        let intro = *buf.get(pos).ok_or(GifError::Truncated)?;
        pos += 1;
        match intro {
            0x3B => break,
            0x21 => {
                let label = *buf.get(pos).ok_or(GifError::Truncated)?;
                pos += 1;
                let (data, next) = read_sub_blocks(buf, pos)?;
                pos = next;
                match label {
                    0xF9 if data.len() >= 4 => {
                        let flags = data[0];
                        disposal = match (flags >> 2) & 7 {
                            1 => Disposal::KeepInPlace,
                            2 => Disposal::RestoreBackground,
                            3 => Disposal::RestorePrevious,
                            _ => Disposal::Unspecified,
                        };
                        delay_ms = u32::from(le16(&data, 1)) * 10;
                        transparent = if flags & 1 != 0 { Some(data[3]) } else { None };
                    }
                    0xFF if data.len() >= 14 && data.starts_with(b"NETSCAPE2.0") && data[11] == 1 => {
                        anim.loop_count = Some(le16(&data, 12));
                    }
                    _ => {}
                }
            }
            0x2C => {
                let d = buf.get(pos..pos + 9).ok_or(GifError::Truncated)?;
                let (left, top, w, h, flags) = (le16(d, 0), le16(d, 2), le16(d, 4), le16(d, 6), d[8]);
                pos += 9;
                let local = if flags & 0x80 != 0 {
                    let (table, next) = read_table(buf, pos, flags)?;
                    pos = next;
                    Some(table)
                } else {
                    None
                };
                let palette = local.or_else(|| global.clone()).ok_or(GifError::Corrupt)?;
                let min_code = *buf.get(pos).ok_or(GifError::Truncated)?;
                pos += 1;
                let (data, next) = read_sub_blocks(buf, pos)?;
                pos = next;
                let total = usize::from(w) * usize::from(h);
                let mut indices = lzw_decode(&data, min_code, total).ok_or(GifError::Corrupt)?;
                indices.resize(total, 0);
                if flags & 0x40 != 0 {
                    indices = deinterlace(&indices, usize::from(w), usize::from(h));
                }
                anim.frames.push(GifFrame {
                    left,
                    top,
                    width: w,
                    height: h,
                    delay_ms,
                    disposal,
                    transparent,
                    palette,
                    indices,
                });
                disposal = Disposal::Unspecified;
                delay_ms = 0;
                transparent = None;
            }
            _ => return Err(GifError::Corrupt),
        }
    }
    Ok(anim)
}

/// Draws every frame onto a `width * height` RGBA canvas that starts fully transparent and
/// applies each frame's disposal before the next one. Returns one canvas per frame, each
/// `width * height * 4` bytes long, so a player can show frame `i` as `out[i]`.
pub fn composite(anim: &GifAnimation) -> Vec<Vec<u8>> {
    let w = usize::from(anim.width);
    let h = usize::from(anim.height);
    let mut canvas = vec![0u8; w * h * 4];
    let mut saved: Option<Vec<u8>> = None;
    let mut prev: Option<&GifFrame> = None;
    let mut out = Vec::with_capacity(anim.frames.len());
    for frame in &anim.frames {
        if let Some(p) = prev {
            match p.disposal {
                Disposal::RestoreBackground => clear_rect(&mut canvas, w, h, p),
                Disposal::RestorePrevious => {
                    if let Some(s) = &saved {
                        canvas.copy_from_slice(s);
                    }
                }
                Disposal::KeepInPlace | Disposal::Unspecified => {}
            }
        }
        saved = Some(canvas.clone());
        draw_frame(&mut canvas, w, h, frame);
        out.push(canvas.clone());
        prev = Some(frame);
    }
    out
}

/// Reads a colour table of `2^(n+1)` entries, where `n` is the low three bits of `flags`.
fn read_table(buf: &[u8], pos: usize, flags: u8) -> Result<(Vec<[u8; 3]>, usize), GifError> {
    let bytes = 3usize << ((flags & 7) + 1);
    let raw = buf.get(pos..pos + bytes).ok_or(GifError::Truncated)?;
    let table = raw.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2]]).collect();
    Ok((table, pos + bytes))
}

/// Concatenates the data sub-blocks that start at `pos`, returning the data and the position
/// after the zero-length terminator.
fn read_sub_blocks(buf: &[u8], mut pos: usize) -> Result<(Vec<u8>, usize), GifError> {
    let mut data = Vec::new();
    loop {
        let size = usize::from(*buf.get(pos).ok_or(GifError::Truncated)?);
        pos += 1;
        if size == 0 {
            return Ok((data, pos));
        }
        data.extend_from_slice(buf.get(pos..pos + size).ok_or(GifError::Truncated)?);
        pos += size;
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// GIF LZW decoder. `min_code` is the minimum code size from the image descriptor (1 to 8).
/// Returns at most `limit` indices, or `None` when the code size or a code is invalid. Running
/// out of input early returns the indices read so far.
pub fn lzw_decode(data: &[u8], min_code: u8, limit: usize) -> Option<Vec<u8>> {
    if !(1..=8).contains(&min_code) {
        return None;
    }
    let clear = 1usize << min_code;
    let eoi = clear + 1;
    let base = u32::from(min_code) + 1;
    let mut table: Vec<Vec<u8>> = (0..clear).map(|i| vec![i as u8]).collect();
    table.push(Vec::new());
    table.push(Vec::new());
    let mut code_size = base;
    let mut out = Vec::new();
    let mut prev: Option<Vec<u8>> = None;
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    let mut pos = 0usize;
    while out.len() < limit {
        while nbits < code_size {
            let Some(&byte) = data.get(pos) else {
                return Some(out);
            };
            bits |= u32::from(byte) << nbits;
            pos += 1;
            nbits += 8;
        }
        let code = (bits & ((1 << code_size) - 1)) as usize;
        bits >>= code_size;
        nbits -= code_size;
        if code == clear {
            table.truncate(eoi + 1);
            code_size = base;
            prev = None;
            continue;
        }
        if code == eoi {
            break;
        }
        let entry = if code < table.len() {
            table[code].clone()
        } else if code == table.len() {
            // The code being defined: previous string plus its own first byte.
            let p = prev.as_ref()?;
            let mut e = p.clone();
            e.push(p[0]);
            e
        } else {
            return None;
        };
        out.extend_from_slice(&entry);
        if let Some(p) = prev {
            if table.len() < MAX_CODES {
                let mut next = p;
                next.push(entry[0]);
                table.push(next);
            }
        }
        if table.len() == 1 << code_size && code_size < 12 {
            code_size += 1;
        }
        prev = Some(entry);
    }
    Some(out)
}

/// Maps the rows of an interlaced image, stored in four passes, back to top-down order.
fn deinterlace(raw: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; raw.len()];
    let mut src_row = 0usize;
    for (start, step) in [(0usize, 8usize), (4, 8), (2, 4), (1, 2)] {
        let mut row = start;
        while row < h {
            let s = src_row * w;
            let d = row * w;
            if s + w <= raw.len() && d + w <= out.len() {
                out[d..d + w].copy_from_slice(&raw[s..s + w]);
            }
            src_row += 1;
            row += step;
        }
    }
    out
}

fn clear_rect(canvas: &mut [u8], w: usize, h: usize, f: &GifFrame) {
    let x1 = (usize::from(f.left) + usize::from(f.width)).min(w);
    let y1 = (usize::from(f.top) + usize::from(f.height)).min(h);
    for y in usize::from(f.top).min(h)..y1 {
        for x in usize::from(f.left).min(w)..x1 {
            let at = (y * w + x) * 4;
            canvas[at..at + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
    }
}

fn draw_frame(canvas: &mut [u8], w: usize, h: usize, f: &GifFrame) {
    let fw = usize::from(f.width);
    for row in 0..usize::from(f.height) {
        let y = usize::from(f.top) + row;
        if y >= h {
            break;
        }
        for col in 0..fw {
            let x = usize::from(f.left) + col;
            if x >= w {
                break;
            }
            let Some(&idx) = f.indices.get(row * fw + col) else {
                continue;
            };
            if f.transparent == Some(idx) {
                continue;
            }
            let Some(c) = f.palette.get(usize::from(idx)) else {
                continue;
            };
            let at = (y * w + x) * 4;
            canvas[at..at + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x1 GIF with a two-colour global table (black, white), one frame of pixels [1, 0] and
    /// a Graphic Control block with the given flags and delay (hundredths of a second).
    fn two_pixel_gif(gce_flags: u8, delay_cs: u8) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[2, 0, 1, 0, 0x80, 0, 0]); // screen 2x1, global table of 2 entries
        g.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        // NETSCAPE loop extension, loop count 3.
        g.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        g.extend_from_slice(b"NETSCAPE2.0");
        g.extend_from_slice(&[0x03, 0x01, 3, 0, 0x00]);
        // Graphic Control: flags, delay (little endian), transparent index 0, terminator.
        g.extend_from_slice(&[0x21, 0xF9, 0x04, gce_flags, delay_cs, 0, 0, 0x00]);
        // Image descriptor at 0,0, size 2x1, no local table, not interlaced.
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 1, 0, 0x00]);
        // LZW minimum code size 2, then codes 4 (clear), 1, 0, 5 (end) packed as [12, 10].
        g.extend_from_slice(&[2, 2, 12, 10, 0]);
        g.push(0x3B);
        g
    }

    #[test]
    fn lzw_decodes_hand_packed_codes() {
        // Codes 4, 0, 6, 5 at 3 bits: 0 then the string "00" defined as code 6.
        assert_eq!(lzw_decode(&[132, 11], 2, 3), Some(vec![0, 0, 0]));
        // Codes 4, 3, 5: a single pixel.
        assert_eq!(lzw_decode(&[92, 1], 2, 1), Some(vec![3]));
    }

    #[test]
    fn lzw_clear_code_resets_the_table() {
        // Codes 4, 0, 4, 3, 5: pixel 0, clear, pixel 3.
        assert_eq!(lzw_decode(&[4, 87], 2, 2), Some(vec![0, 3]));
    }

    #[test]
    fn lzw_rejects_an_invalid_minimum_code_size() {
        assert_eq!(lzw_decode(&[0, 0], 0, 4), None);
        assert_eq!(lzw_decode(&[0, 0], 9, 4), None);
    }

    #[test]
    fn decodes_frame_pixels_delay_and_loop_count() {
        let anim = decode(&two_pixel_gif(0x08, 5)).expect("valid gif");
        assert_eq!((anim.width, anim.height), (2, 1));
        assert_eq!(anim.loop_count, Some(3));
        assert_eq!(anim.frames.len(), 1);
        let f = &anim.frames[0];
        assert_eq!(f.indices, vec![1, 0]);
        assert_eq!(f.delay_ms, 50);
        assert_eq!(f.disposal, Disposal::RestoreBackground);
        assert_eq!(f.transparent, None);
        assert_eq!(f.palette, vec![[0, 0, 0], [255, 255, 255]]);
    }

    #[test]
    fn composite_draws_opaque_pixels_and_keeps_transparent_ones_clear() {
        let plain = decode(&two_pixel_gif(0x00, 0)).unwrap();
        assert_eq!(composite(&plain)[0], vec![255, 255, 255, 255, 0, 0, 0, 255]);
        // Transparent flag set, index 0 is see-through: the black pixel becomes clear.
        let see_through = decode(&two_pixel_gif(0x01, 0)).unwrap();
        assert_eq!(composite(&see_through)[0], vec![255, 255, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_a_bad_signature_and_a_truncated_file() {
        assert_eq!(decode(b"PNG-not-a-gif-at-all"), Err(GifError::Signature));
        let full = two_pixel_gif(0, 0);
        assert_eq!(decode(&full[..full.len() - 3]), Err(GifError::Truncated));
    }

    #[test]
    fn deinterlace_puts_the_four_passes_in_row_order() {
        // Height 8 with one-pixel rows: storage order is rows 0, 4, 2, 6, 1, 3, 5, 7.
        let stored: Vec<u8> = vec![0, 4, 2, 6, 1, 3, 5, 7];
        assert_eq!(deinterlace(&stored, 1, 8), vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }
}
