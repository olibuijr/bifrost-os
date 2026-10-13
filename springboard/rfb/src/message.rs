//! Messages after the handshake. Client messages are encoded into byte vectors for the caller to write; server
//! messages are read from any `Read`. Only the encodings the viewer asks for are decoded: Raw, CopyRect and the
//! DesktopSize pseudo-encoding.

use crate::{Error, PixelFormat};
use std::io::Read;

/// Raw rectangle: one pixel per wire pixel, row by row.
pub const ENC_RAW: i32 = 0;
/// CopyRect: the rectangle is copied from another place in the framebuffer.
pub const ENC_COPYRECT: i32 = 1;
/// DesktopSize pseudo-encoding: the framebuffer changed size.
pub const ENC_DESKTOP_SIZE: i32 = -223;
/// The encodings the viewer offers, in order of preference.
pub const ENCODINGS: [i32; 3] = [ENC_COPYRECT, ENC_RAW, ENC_DESKTOP_SIZE];

/// Largest raw rectangle payload accepted (64 MiB); larger rectangles are a protocol error.
const MAX_RECT_BYTES: usize = 64 << 20;
/// Largest server cut-text message accepted (1 MiB).
const MAX_CUT_TEXT: usize = 1 << 20;

// ---- client messages ----

/// SetPixelFormat.
pub fn set_pixel_format(f: &PixelFormat) -> Vec<u8> {
    let mut v = vec![0u8, 0, 0, 0];
    v.extend_from_slice(&f.encode());
    v
}

/// SetEncodings.
pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut v = vec![2u8, 0];
    v.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for e in encodings {
        v.extend_from_slice(&e.to_be_bytes());
    }
    v
}

/// FramebufferUpdateRequest: `incremental` asks only for what changed since the last update.
pub fn update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> Vec<u8> {
    let mut v = vec![3u8, incremental as u8];
    for n in [x, y, w, h] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    v
}

/// KeyEvent: a keysym pressed (`down`) or released.
pub fn key_event(down: bool, keysym: u32) -> Vec<u8> {
    let mut v = vec![4u8, down as u8, 0, 0];
    v.extend_from_slice(&keysym.to_be_bytes());
    v
}

/// PointerEvent: `buttons` is a mask (bit 0 left, bit 1 middle, bit 2 right); x and y are in remote pixels.
pub fn pointer_event(buttons: u8, x: u16, y: u16) -> Vec<u8> {
    let mut v = vec![5u8, buttons];
    v.extend_from_slice(&x.to_be_bytes());
    v.extend_from_slice(&y.to_be_bytes());
    v
}

/// ClientCutText: the viewer's clipboard text, in Latin-1 (characters beyond it become `?`).
pub fn client_cut_text(text: &str) -> Vec<u8> {
    let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
    let mut v = vec![6u8, 0, 0, 0];
    v.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    v.extend_from_slice(&bytes);
    v
}

// ---- server messages ----

/// What a rectangle carries once read off the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Pixels in the connection's pixel format, row by row.
    Raw(Vec<u8>),
    /// Copy the rectangle from (src_x, src_y) in the framebuffer.
    CopyRect { src_x: usize, src_y: usize },
    /// The new framebuffer size is the rectangle's w and h.
    DesktopSize,
}

/// One rectangle of a FramebufferUpdate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub payload: Payload,
}

/// A message from the server, after the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// A FramebufferUpdate: rectangles to apply in order.
    Update(Vec<Rect>),
    /// SetColourMapEntries was read and ignored (only true-colour formats are used).
    ColourMapIgnored,
    /// The server rang the bell.
    Bell,
    /// The server's clipboard text (Latin-1 decoded).
    CutText(String),
}

/// Reads one server message. Pixel payloads are read in `fmt`, the format the viewer asked for.
pub fn read_server_message<R: Read>(r: &mut R, fmt: &PixelFormat) -> Result<ServerMessage, Error> {
    let mut t = [0u8; 1];
    r.read_exact(&mut t)?;
    match t[0] {
        0 => read_update(r, fmt).map(ServerMessage::Update),
        1 => {
            let mut h = [0u8; 5];
            r.read_exact(&mut h)?;
            let n = u16::from_be_bytes([h[3], h[4]]) as usize;
            let mut skip = vec![0u8; n * 6];
            r.read_exact(&mut skip)?;
            Ok(ServerMessage::ColourMapIgnored)
        }
        2 => Ok(ServerMessage::Bell),
        3 => {
            let mut h = [0u8; 7];
            r.read_exact(&mut h)?;
            let len = u32::from_be_bytes([h[3], h[4], h[5], h[6]]) as usize;
            if len > MAX_CUT_TEXT {
                return Err(Error::Protocol("clipboard text too long".into()));
            }
            let mut b = vec![0u8; len];
            r.read_exact(&mut b)?;
            Ok(ServerMessage::CutText(b.iter().map(|&c| c as char).collect()))
        }
        other => Err(Error::Protocol(format!("unknown server message {other}"))),
    }
}

fn read_update<R: Read>(r: &mut R, fmt: &PixelFormat) -> Result<Vec<Rect>, Error> {
    let mut h = [0u8; 3];
    r.read_exact(&mut h)?;
    let n = u16::from_be_bytes([h[1], h[2]]) as usize;
    let mut rects = Vec::with_capacity(n);
    for _ in 0..n {
        let mut b = [0u8; 12];
        r.read_exact(&mut b)?;
        let x = u16::from_be_bytes([b[0], b[1]]) as usize;
        let y = u16::from_be_bytes([b[2], b[3]]) as usize;
        let w = u16::from_be_bytes([b[4], b[5]]) as usize;
        let rh = u16::from_be_bytes([b[6], b[7]]) as usize;
        let enc = i32::from_be_bytes([b[8], b[9], b[10], b[11]]);
        let payload = match enc {
            ENC_RAW => {
                let len = w * rh * fmt.bytes_per_pixel();
                if len > MAX_RECT_BYTES {
                    return Err(Error::Protocol("rectangle too large".into()));
                }
                let mut px = vec![0u8; len];
                r.read_exact(&mut px)?;
                Payload::Raw(px)
            }
            ENC_COPYRECT => {
                let mut s = [0u8; 4];
                r.read_exact(&mut s)?;
                Payload::CopyRect { src_x: u16::from_be_bytes([s[0], s[1]]) as usize, src_y: u16::from_be_bytes([s[2], s[3]]) as usize }
            }
            ENC_DESKTOP_SIZE => Payload::DesktopSize,
            other => return Err(Error::Unsupported(format!("encoding {other}"))),
        };
        rects.push(Rect { x, y, w, h: rh, payload });
    }
    Ok(rects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn client_messages_have_the_wire_layout() {
        assert_eq!(set_pixel_format(&PixelFormat::VIEWER)[..4], [0, 0, 0, 0]);
        assert_eq!(set_pixel_format(&PixelFormat::VIEWER).len(), 20);
        assert_eq!(set_encodings(&[0, -223]), vec![2, 0, 0, 2, 0, 0, 0, 0, 0xff, 0xff, 0x2d, 0x21]);
        assert_eq!(update_request(true, 1, 2, 3, 4), vec![3, 1, 0, 1, 0, 2, 0, 3, 0, 4]);
        assert_eq!(key_event(true, 0x61), vec![4, 1, 0, 0, 0, 0, 0, 0x61]);
        assert_eq!(pointer_event(4, 0x0100, 0x0002), vec![5, 4, 1, 0, 0, 2]);
    }

    #[test]
    fn cut_text_is_latin_one_with_a_length() {
        assert_eq!(client_cut_text("hi"), vec![6, 0, 0, 0, 0, 0, 0, 2, b'h', b'i']);
        assert_eq!(client_cut_text("é€")[8..], [0xe9, b'?']);
    }

    #[test]
    fn reads_a_raw_update_and_a_copy_rect() {
        // One update with two rectangles: a 2x1 raw strip of red and blue, then a copy of (0,0) to (5,5).
        // Header: type 0, padding, two rectangles.
        let mut m = vec![0u8, 0, 0, 2];
        // Rect 1: (0,0) 2x1 Raw, then its pixels in B,G,R,pad order.
        m.extend_from_slice(&[0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0]);
        m.extend_from_slice(&[0, 0, 0xff, 0, 0xff, 0, 0, 0]);
        // Rect 2: (5,5) 1x1 CopyRect from (0,0): the source position is 4 bytes.
        m.extend_from_slice(&[0, 5, 0, 5, 0, 1, 0, 1, 0, 0, 0, 1]);
        m.extend_from_slice(&[0, 0, 0, 0]);
        let msg = read_server_message(&mut Cursor::new(m), &PixelFormat::VIEWER).unwrap();
        let ServerMessage::Update(rects) = msg else { panic!("expected an update") };
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect { x: 0, y: 0, w: 2, h: 1, payload: Payload::Raw(vec![0, 0, 0xff, 0, 0xff, 0, 0, 0]) });
        assert_eq!(rects[1], Rect { x: 5, y: 5, w: 1, h: 1, payload: Payload::CopyRect { src_x: 0, src_y: 0 } });
    }

    #[test]
    fn desktop_size_and_bell_and_cut_text() {
        // One rectangle: DesktopSize 800x600 (0x0320 x 0x0258) with encoding -223.
        let m = vec![0u8, 0, 0, 1, 0, 0, 0, 0, 0x03, 0x20, 0x02, 0x58, 0xff, 0xff, 0xff, 0x21];
        let mut r = Cursor::new(m);
        let ServerMessage::Update(rects) = read_server_message(&mut r, &PixelFormat::VIEWER).unwrap() else { panic!() };
        assert_eq!(rects, vec![Rect { x: 0, y: 0, w: 800, h: 600, payload: Payload::DesktopSize }]);
        assert_eq!(read_server_message(&mut Cursor::new(vec![2u8]), &PixelFormat::VIEWER).unwrap(), ServerMessage::Bell);
        let cut = [3u8, 0, 0, 0, 0, 0, 0, 2, 0x41, 0xe9];
        assert_eq!(read_server_message(&mut Cursor::new(cut.to_vec()), &PixelFormat::VIEWER).unwrap(), ServerMessage::CutText("A\u{e9}".into()));
    }

    #[test]
    fn unknown_messages_and_encodings_fail() {
        assert!(matches!(read_server_message(&mut Cursor::new(vec![42u8]), &PixelFormat::VIEWER), Err(Error::Protocol(_))));
        // A FramebufferUpdate with one Hextile (5) rectangle, which this viewer never asks for.
        let m = vec![0u8, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 5];
        assert!(matches!(read_server_message(&mut Cursor::new(m), &PixelFormat::VIEWER), Err(Error::Unsupported(_))));
    }

    #[test]
    fn colour_map_is_skipped_whole() {
        // SetColourMapEntries: padding, first colour 0, two colours (6 bytes each), then a bell.
        // Type 1, padding, first colour (2 bytes), count 2 (2 bytes), then 2 x 6 bytes of colours.
        let mut m = vec![1u8, 0, 0, 0, 0, 2];
        m.extend_from_slice(&[0; 12]);
        m.push(2);
        let mut r = Cursor::new(m);
        assert_eq!(read_server_message(&mut r, &PixelFormat::VIEWER).unwrap(), ServerMessage::ColourMapIgnored);
        assert_eq!(read_server_message(&mut r, &PixelFormat::VIEWER).unwrap(), ServerMessage::Bell);
    }
}
