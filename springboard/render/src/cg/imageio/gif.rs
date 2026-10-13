//! GIF: animated frames with delays, disposal and transparency on read; palette quantising LZW writer.

use super::exif::{Dict, PropValue, Properties};
use super::{Decoded, Frame, MAX_SIDE};

fn le16(d: &[u8], o: usize) -> Option<usize> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?) as usize)
}

/// Logical screen size.
pub fn probe(data: &[u8]) -> Option<Properties> {
    if !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        return None;
    }
    let (w, h) = (le16(data, 6)?, le16(data, 8)?);
    if w == 0 || h == 0 {
        return None;
    }
    let mut p = Properties::new(w, h);
    p.is_indexed = true;
    p.has_alpha = true;
    p.depth = 8;
    p.gif.insert("Version".into(), PropValue::Text(String::from_utf8_lossy(&data[3..6]).into_owned()));
    Some(p)
}

/// Skips data sub-blocks starting at `p`; returns the offset after the terminator.
fn skip_blocks(d: &[u8], mut p: usize) -> Option<usize> {
    loop {
        let n = *d.get(p)? as usize;
        p += 1 + n;
        if n == 0 {
            return Some(p);
        }
    }
}

fn gather_blocks(d: &[u8], mut p: usize) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    loop {
        let n = *d.get(p)? as usize;
        if n == 0 {
            return Some((out, p + 1));
        }
        out.extend_from_slice(d.get(p + 1..p + 1 + n)?);
        p += 1 + n;
    }
}

/// Number of image frames (walks the block structure only).
pub fn count(data: &[u8]) -> Option<usize> {
    probe(data)?;
    let flags = *data.get(10)?;
    let mut p = 13 + if flags & 0x80 != 0 { 3 << ((flags & 7) + 1) } else { 0 };
    let mut n = 0;
    while let Some(&b) = data.get(p) {
        match b {
            0x21 => p = match skip_blocks(data, p + 2) {
                Some(q) => q,
                None => break,
            },
            0x2C => {
                let Some(&lf) = data.get(p + 9) else { break };
                let table = if lf & 0x80 != 0 { 3usize << ((lf & 7) + 1) } else { 0 };
                p += 10 + table + 1;
                match skip_blocks(data, p) {
                    Some(q) => p = q,
                    None => {
                        n += 1;
                        break;
                    }
                }
                n += 1;
            }
            _ => break,
        }
    }
    Some(n.max(1))
}

fn lzw_decode(data: &[u8], min: u8, want: usize) -> Vec<u8> {
    let min = min.clamp(1, 11) as u32;
    let (clear, eoi) = (1u32 << min, (1u32 << min) + 1);
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    let mut out = Vec::with_capacity(want);
    let (mut width, mut next, mut prev): (u32, u32, Option<u32>) = (min + 1, eoi + 1, None);
    let (mut acc, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let expand = |code: u32, prefix: &[u16], suffix: &[u8], buf: &mut Vec<u8>| {
        buf.clear();
        let mut c = code;
        while c >= clear + 2 {
            buf.push(suffix[c as usize]);
            c = prefix[c as usize] as u32;
        }
        buf.push(c as u8);
        buf.reverse();
    };
    let mut buf = Vec::new();
    while out.len() < want {
        while nbits < width {
            let Some(&b) = data.get(pos) else { return out };
            acc |= (b as u32) << nbits;
            nbits += 8;
            pos += 1;
        }
        let code = acc & ((1 << width) - 1);
        acc >>= width;
        nbits -= width;
        if code == clear {
            width = min + 1;
            next = eoi + 1;
            prev = None;
            continue;
        }
        if code == eoi {
            break;
        }
        match prev {
            None => {
                if code >= clear {
                    break;
                }
                out.push(code as u8);
            }
            Some(p) => {
                if code > next {
                    break;
                }
                let first;
                if code < next {
                    expand(code, &prefix, &suffix, &mut buf);
                    first = buf[0];
                    out.extend_from_slice(&buf);
                } else {
                    expand(p, &prefix, &suffix, &mut buf);
                    first = buf[0];
                    out.extend_from_slice(&buf);
                    out.push(first);
                }
                if next < 4096 {
                    prefix[next as usize] = p as u16;
                    suffix[next as usize] = first;
                    next += 1;
                    if next == (1 << width) && width < 12 {
                        width += 1;
                    }
                }
            }
        }
        prev = Some(code);
    }
    out
}

pub fn decode(data: &[u8]) -> Option<Decoded> {
    let mut props0 = probe(data)?;
    let (w, h) = (props0.pixel_width, props0.pixel_height);
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let flags = *data.get(10)?;
    let bg_index = *data.get(11)? as usize;
    let mut p = 13;
    let global: Vec<u8> = if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        let t = data.get(p..p + n)?.to_vec();
        p += n;
        t
    } else {
        Vec::new()
    };
    let _ = bg_index;
    let mut canvas = vec![0u8; w * h * 4];
    let mut frames = Vec::new();
    let mut loop_count = None;
    let (mut delay, mut transparent, mut disposal) = (0u32, None::<u8>, 0u8);
    let mut comment: Option<String> = None;
    while let Some(&b) = data.get(p) {
        match b {
            0x21 => {
                let label = *data.get(p + 1)?;
                let Some((body, next)) = gather_blocks(data, p + 2) else { break };
                match label {
                    0xF9 if body.len() >= 4 => {
                        disposal = (body[0] >> 2) & 7;
                        transparent = (body[0] & 1 == 1).then_some(body[3]);
                        delay = u16::from_le_bytes([body[1], body[2]]) as u32 * 10;
                    }
                    0xFF if body.starts_with(b"NETSCAPE2.0") && body.len() >= 14 => {
                        loop_count = Some(u16::from_le_bytes([body[12], body[13]]) as u32);
                    }
                    0xFE => comment = Some(String::from_utf8_lossy(&body).into_owned()),
                    _ => {}
                }
                p = next;
            }
            0x2C => {
                let (fx, fy, fw, fh) = (le16(data, p + 1)?, le16(data, p + 3)?, le16(data, p + 5)?, le16(data, p + 7)?);
                let lf = *data.get(p + 9)?;
                p += 10;
                let local: Vec<u8> = if lf & 0x80 != 0 {
                    let n = 3 << ((lf & 7) + 1);
                    let t = data.get(p..p + n)?.to_vec();
                    p += n;
                    t
                } else {
                    Vec::new()
                };
                let table = if local.is_empty() { &global } else { &local };
                let min = *data.get(p)?;
                let (body, next) = gather_blocks(data, p + 1)?;
                p = next;
                let idx = lzw_decode(&body, min, fw * fh);
                let saved = (disposal == 3).then(|| canvas.clone());
                let interlaced = lf & 0x40 != 0;
                let rows: Vec<usize> = if interlaced {
                    [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(s, st)| (s..fh).step_by(st)).collect()
                } else {
                    (0..fh).collect()
                };
                for (src_row, &y) in rows.iter().enumerate() {
                    for x in 0..fw {
                        let Some(&ci) = idx.get(src_row * fw + x) else { continue };
                        if Some(ci) == transparent {
                            continue;
                        }
                        let (cx, cy) = (fx + x, fy + y);
                        if cx >= w || cy >= h {
                            continue;
                        }
                        if let Some(c) = table.get(ci as usize * 3..ci as usize * 3 + 3) {
                            let o = (cy * w + cx) * 4;
                            canvas[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                        }
                    }
                }
                let mut pr = props0.clone();
                pr.has_alpha = transparent.is_some() || frames.is_empty() && canvas.chunks_exact(4).any(|q| q[3] == 0);
                let mut g: Dict = props0.gif.clone();
                g.insert("DelayTime".into(), PropValue::Float(delay as f64 / 1000.0));
                g.insert("HasGlobalColorMap".into(), PropValue::Int((!global.is_empty()) as i64));
                if let Some(c) = &comment {
                    g.insert("Comment".into(), PropValue::Text(c.clone()));
                }
                pr.gif = g;
                frames.push((Frame { w, h, rgba8: canvas.clone(), rgba16: None, delay_ms: delay }, pr));
                match disposal {
                    2 => {
                        for y in fy..(fy + fh).min(h) {
                            for x in fx..(fx + fw).min(w) {
                                canvas[(y * w + x) * 4..(y * w + x) * 4 + 4].fill(0);
                            }
                        }
                    }
                    3 => {
                        if let Some(s) = saved {
                            canvas = s;
                        }
                    }
                    _ => {}
                }
                (delay, transparent, disposal) = (0, None, 0);
                if frames.len() > 4096 {
                    break;
                }
            }
            _ => break,
        }
    }
    if frames.is_empty() {
        return None;
    }
    if let Some(l) = loop_count {
        props0.gif.insert("LoopCount".into(), PropValue::Int(l as i64));
    }
    let props = frames.iter().map(|f| {
        let mut p = f.1.clone();
        if let Some(l) = loop_count {
            p.gif.insert("LoopCount".into(), PropValue::Int(l as i64));
        }
        p
    }).collect();
    Some(Decoded { frames: frames.into_iter().map(|f| f.0).collect(), props, loop_count })
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, code: u32, width: u32) {
        self.acc |= code << self.n;
        self.n += width;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

fn lzw_encode(indices: &[u8], min: u32) -> Vec<u8> {
    use std::collections::HashMap;
    let (clear, eoi) = (1u32 << min, (1u32 << min) + 1);
    let mut w = BitWriter { out: Vec::new(), acc: 0, n: 0 };
    let mut dict: HashMap<(u32, u8), u32> = HashMap::new();
    let (mut width, mut next) = (min + 1, eoi + 1);
    w.put(clear, width);
    let mut it = indices.iter();
    let Some(&first) = it.next() else {
        w.put(eoi, width);
        return w.finish();
    };
    let mut prefix = first as u32;
    for &b in it {
        if let Some(&c) = dict.get(&(prefix, b)) {
            prefix = c;
            continue;
        }
        w.put(prefix, width);
        if next > (1 << width) - 1 && width < 12 {
            width += 1;
        }
        if next < 4096 {
            dict.insert((prefix, b), next);
            next += 1;
        } else {
            w.put(clear, width);
            dict.clear();
            width = min + 1;
            next = eoi + 1;
        }
        prefix = b as u32;
    }
    w.put(prefix, width);
    // The decoder may add one more entry (and widen) before reading the end code.
    if next > (1 << width) - 1 && width < 12 {
        width += 1;
    }
    w.put(eoi, width);
    w.finish()
}

/// One frame to write: straight RGBA pixels and a delay.
pub struct GifFrame<'a> {
    pub rgba: &'a [u8],
    pub delay_ms: u32,
}

fn quantise(frames: &[GifFrame]) -> (Vec<[u8; 3]>, bool, Box<dyn Fn(&[u8]) -> u8>) {
    use std::collections::HashMap;
    let transparent = frames.iter().any(|f| f.rgba.chunks_exact(4).any(|p| p[3] < 128));
    let limit = if transparent { 255 } else { 256 };
    let mut exact: HashMap<[u8; 3], u8> = HashMap::new();
    let mut pal: Vec<[u8; 3]> = Vec::new();
    let mut fits = true;
    'scan: for f in frames {
        for p in f.rgba.chunks_exact(4) {
            if p[3] < 128 {
                continue;
            }
            let c = [p[0], p[1], p[2]];
            if !exact.contains_key(&c) {
                if pal.len() == limit {
                    fits = false;
                    break 'scan;
                }
                exact.insert(c, pal.len() as u8);
                pal.push(c);
            }
        }
    }
    if fits {
        let tidx = pal.len() as u8;
        return (pal, transparent, Box::new(move |p: &[u8]| if p[3] < 128 { tidx } else { *exact.get(&[p[0], p[1], p[2]]).unwrap_or(&0) }));
    }
    // 6 x 7 x 6 colour cube
    let mut cube = Vec::with_capacity(252);
    for r in 0..6u32 {
        for g in 0..7u32 {
            for b in 0..6u32 {
                cube.push([(r * 255 / 5) as u8, (g * 255 / 6) as u8, (b * 255 / 5) as u8]);
            }
        }
    }
    (
        cube,
        transparent,
        Box::new(|p: &[u8]| {
            if p[3] < 128 {
                return 252;
            }
            let r = (p[0] as u32 * 5 + 127) / 255;
            let g = (p[1] as u32 * 6 + 127) / 255;
            let b = (p[2] as u32 * 5 + 127) / 255;
            (r * 42 + g * 6 + b) as u8
        }),
    )
}

/// Encodes same-sized frames; `loop_count` 0 loops forever (written only for animations when `Some`).
pub fn encode(w: usize, h: usize, frames: &[GifFrame], loop_count: Option<u32>) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > 65535 || h > 65535 || frames.is_empty() || frames.iter().any(|f| f.rgba.len() < w * h * 4) {
        return None;
    }
    let (pal, transparent, index_of) = quantise(frames);
    let tidx = if transparent { Some(if pal.len() == 252 { 252u8 } else { pal.len() as u8 }) } else { None };
    let mut o = b"GIF89a".to_vec();
    o.extend((w as u16).to_le_bytes());
    o.extend((h as u16).to_le_bytes());
    o.extend([0xF7, 0, 0]);
    for i in 0..256 {
        o.extend(pal.get(i).copied().unwrap_or([0, 0, 0]));
    }
    if frames.len() > 1 {
        o.extend([0x21, 0xFF, 11]);
        o.extend(b"NETSCAPE2.0");
        o.extend([3, 1]);
        o.extend((loop_count.unwrap_or(0) as u16).to_le_bytes());
        o.push(0);
    }
    for f in frames {
        let disposal = if transparent { 2u8 } else { 1 };
        o.extend([0x21, 0xF9, 4, (disposal << 2) | transparent as u8]);
        o.extend(((f.delay_ms + 5) / 10).min(65535).to_le_bytes()[..2].iter());
        o.push(tidx.unwrap_or(0));
        o.push(0);
        o.extend([0x2C, 0, 0, 0, 0]);
        o.extend((w as u16).to_le_bytes());
        o.extend((h as u16).to_le_bytes());
        o.push(0);
        o.push(8);
        let idx: Vec<u8> = f.rgba.chunks_exact(4).take(w * h).map(|p| index_of(p)).collect();
        for chunk in lzw_encode(&idx, 8).chunks(255) {
            o.push(chunk.len() as u8);
            o.extend(chunk);
        }
        o.push(0);
    }
    o.push(0x3B);
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).flat_map({ let f = &f; move |x| f(x, y) })).collect()
    }

    #[test]
    fn lzw_round_trips_long_runs_and_noise() {
        let mut v: Vec<u8> = (0..20000u32).map(|i| (i * 7 % 251) as u8).collect();
        v.extend(vec![3u8; 9000]);
        for min in [8u32] {
            let enc = lzw_encode(&v, min);
            assert_eq!(lzw_decode(&enc, min as u8, v.len()), v);
        }
        let small: Vec<u8> = (0..500u32).map(|i| (i % 3) as u8).collect();
        assert_eq!(lzw_decode(&lzw_encode(&small, 2), 2, small.len()), small);
    }

    #[test]
    fn animated_gif_round_trips_with_delays_and_transparency() {
        let a = frame(4, 3, |x, _| if x < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        let b = frame(4, 3, |_, y| if y == 0 { [0, 0, 0, 0] } else { [0, 255, 0, 255] });
        let gif = encode(4, 3, &[GifFrame { rgba: &a, delay_ms: 100 }, GifFrame { rgba: &b, delay_ms: 250 }], Some(3)).unwrap();
        let p = probe(&gif).unwrap();
        assert_eq!((p.pixel_width, p.pixel_height), (4, 3));
        assert_eq!(count(&gif), Some(2));
        let d = decode(&gif).unwrap();
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.loop_count, Some(3));
        assert_eq!(d.frames[0].delay_ms, 100);
        assert_eq!(d.frames[1].delay_ms, 250);
        assert_eq!(d.frames[0].rgba8, a);
        assert_eq!(&d.frames[1].rgba8[..4], &[0, 0, 0, 0], "transparent pixel");
        assert_eq!(&d.frames[1].rgba8[16..20], &[0, 255, 0, 255]);
    }

    #[test]
    fn many_colours_quantise_without_failing() {
        let a = frame(32, 32, |x, y| [(x * 8) as u8, (y * 8) as u8, ((x + y) * 4) as u8, 255]);
        let gif = encode(32, 32, &[GifFrame { rgba: &a, delay_ms: 0 }], None).unwrap();
        let d = decode(&gif).unwrap();
        let (src, got) = (&a[(5 * 32 + 5) * 4..][..3], &d.frames[0].rgba8[(5 * 32 + 5) * 4..][..3]);
        for k in 0..3 {
            assert!((src[k] as i32 - got[k] as i32).abs() < 60);
        }
    }

    #[test]
    fn foreign_data_is_refused() {
        assert!(probe(b"GIF").is_none());
        assert!(decode(b"GIF89a\x01\x00\x01\x00\x00\x00\x00").is_none());
        assert!(encode(0, 1, &[], None).is_none());
    }
}
