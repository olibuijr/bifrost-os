//! Reading the structure of an MP4/QuickTime file: walks the box tree for the movie header (duration and time
//! scale), each track's kind, dimensions and sample description, and the file brand. Gives video media info for a
//! library and the basis for trimming and joining movies. Pure std over the bytes of the file (or its head).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxHeader {
    pub kind: [u8; 4],
    /// Offset of the box in the file.
    pub offset: usize,
    /// Size of the whole box in bytes (header included).
    pub size: usize,
    /// Size of the header (8, or 16 for a 64-bit size).
    pub header: usize,
}

impl BoxHeader {
    pub fn name(&self) -> String {
        self.kind.iter().map(|b| if b.is_ascii_graphic() { *b as char } else { '?' }).collect()
    }

    pub fn payload(&self) -> std::ops::Range<usize> {
        self.offset + self.header..self.offset + self.size
    }
}

/// The boxes laid end to end in `data[range]`. Stops at the first box that does not fit.
pub fn boxes(data: &[u8], range: std::ops::Range<usize>) -> Vec<BoxHeader> {
    let mut out = vec![];
    let mut at = range.start;
    let end = range.end.min(data.len());
    while at + 8 <= end {
        let size32 = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let kind = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        let (size, header) = match size32 {
            0 => (end - at, 8),
            1 => {
                if at + 16 > end {
                    break;
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&data[at + 8..at + 16]);
                (u64::from_be_bytes(b) as usize, 16)
            }
            n => (n, 8),
        };
        if size < header || at + size > end {
            break;
        }
        out.push(BoxHeader { kind, offset: at, size, header });
        at += size;
    }
    out
}

fn child(data: &[u8], parent: &BoxHeader, kind: &[u8; 4]) -> Option<BoxHeader> {
    boxes(data, parent.payload()).into_iter().find(|b| &b.kind == kind)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be64(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        u64::from_be_bytes(a)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub kind: TrackKind,
    /// Display width and height in pixels (video tracks).
    pub width: u32,
    pub height: u32,
    /// Four-character code of the first sample description (`avc1`, `hvc1`, `mp4a`...).
    pub codec: String,
    pub duration_units: u64,
    pub timescale: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub brand: String,
    pub timescale: u32,
    pub duration_units: u64,
    pub tracks: Vec<Track>,
}

impl Info {
    pub fn duration_secs(&self) -> f64 {
        if self.timescale == 0 { 0.0 } else { self.duration_units as f64 / f64::from(self.timescale) }
    }

    pub fn video(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Video)
    }
}

/// Parses a movie. None when there is no `moov` box.
pub fn parse(data: &[u8]) -> Option<Info> {
    let top = boxes(data, 0..data.len());
    let brand = top
        .iter()
        .find(|b| &b.kind == b"ftyp")
        .and_then(|b| data.get(b.offset + b.header..b.offset + b.header + 4))
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default();
    let moov = top.iter().find(|b| &b.kind == b"moov")?;
    let mvhd = child(data, moov, b"mvhd")?;
    let p = mvhd.offset + mvhd.header;
    let (timescale, duration) = if *data.get(p)? == 1 {
        (be32(data, p + 20)?, be64(data, p + 24)?)
    } else {
        (be32(data, p + 12)?, u64::from(be32(data, p + 16)?))
    };
    let mut tracks = vec![];
    for trak in boxes(data, moov.payload()).iter().filter(|b| &b.kind == b"trak") {
        tracks.push(parse_track(data, trak));
    }
    Some(Info { brand, timescale, duration_units: duration, tracks })
}

fn parse_track(data: &[u8], trak: &BoxHeader) -> Track {
    let mut t = Track { kind: TrackKind::Other, width: 0, height: 0, codec: String::new(), duration_units: 0, timescale: 0 };
    if let Some(tkhd) = child(data, trak, b"tkhd") {
        let p = tkhd.offset + tkhd.header;
        let v1 = data.get(p) == Some(&1);
        // width and height are the last two 16.16 fixed-point fields of the 84- or 92-byte body
        let end = tkhd.offset + tkhd.size;
        if let (Some(w), Some(h)) = (be32(data, end.saturating_sub(8)), be32(data, end.saturating_sub(4))) {
            t.width = w >> 16;
            t.height = h >> 16;
        }
        t.duration_units = if v1 { be64(data, p + 28).unwrap_or(0) } else { u64::from(be32(data, p + 20).unwrap_or(0)) };
    }
    if let Some(mdia) = child(data, trak, b"mdia") {
        if let Some(mdhd) = child(data, &mdia, b"mdhd") {
            let p = mdhd.offset + mdhd.header;
            t.timescale = if data.get(p) == Some(&1) { be32(data, p + 20).unwrap_or(0) } else { be32(data, p + 12).unwrap_or(0) };
        }
        if let Some(hdlr) = child(data, &mdia, b"hdlr") {
            let p = hdlr.offset + hdlr.header;
            t.kind = match data.get(p + 8..p + 12) {
                Some(b"vide") => TrackKind::Video,
                Some(b"soun") => TrackKind::Audio,
                Some(b"text") | Some(b"sbtl") | Some(b"subt") => TrackKind::Text,
                _ => TrackKind::Other,
            };
        }
        let stsd = child(data, &mdia, b"minf")
            .and_then(|minf| child(data, &minf, b"stbl"))
            .and_then(|stbl| child(data, &stbl, b"stsd"));
        if let Some(stsd) = stsd {
            // version/flags (4), entry count (4), then the first entry: size (4), format (4)
            let p = stsd.offset + stsd.header + 8;
            if let Some(f) = data.get(p + 4..p + 8) {
                t.codec = String::from_utf8_lossy(f).into_owned();
            }
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    fn mvhd(scale: u32, duration: u32) -> Vec<u8> {
        let mut p = vec![0u8; 4]; // version 0, flags
        p.extend_from_slice(&[0; 8]); // creation, modification
        p.extend_from_slice(&scale.to_be_bytes());
        p.extend_from_slice(&duration.to_be_bytes());
        p.extend_from_slice(&[0; 80]);
        mk(b"mvhd", &p)
    }

    fn trak(handler: &[u8; 4], codec: &[u8; 4], w: u32, h: u32) -> Vec<u8> {
        let mut tk = vec![0u8; 4];
        tk.extend_from_slice(&[0; 8]);
        tk.extend_from_slice(&1u32.to_be_bytes()); // track id
        tk.extend_from_slice(&[0; 4]);
        tk.extend_from_slice(&90_000u32.to_be_bytes()); // duration
        tk.extend_from_slice(&[0; 52]);
        tk.extend_from_slice(&(w << 16).to_be_bytes());
        tk.extend_from_slice(&(h << 16).to_be_bytes());
        let tkhd = mk(b"tkhd", &tk);
        let mut md = vec![0u8; 4];
        md.extend_from_slice(&[0; 8]);
        md.extend_from_slice(&30_000u32.to_be_bytes());
        md.extend_from_slice(&[0; 8]);
        let mdhd = mk(b"mdhd", &md);
        let mut hd = vec![0u8; 8];
        hd.extend_from_slice(handler);
        hd.extend_from_slice(&[0; 12]);
        let hdlr = mk(b"hdlr", &hd);
        let mut entry = 16u32.to_be_bytes().to_vec();
        entry.extend_from_slice(codec);
        entry.extend_from_slice(&[0; 8]);
        let mut sd = vec![0u8; 4];
        sd.extend_from_slice(&1u32.to_be_bytes());
        sd.extend_from_slice(&entry);
        let stsd = mk(b"stsd", &sd);
        let stbl = mk(b"stbl", &stsd);
        let minf = mk(b"minf", &stbl);
        let mut mdia_payload = mdhd;
        mdia_payload.extend(hdlr);
        mdia_payload.extend(minf);
        let mdia = mk(b"mdia", &mdia_payload);
        let mut tp = tkhd;
        tp.extend(mdia);
        mk(b"trak", &tp)
    }

    fn movie() -> Vec<u8> {
        let mut file = mk(b"ftyp", b"isom\0\0\0\0isom");
        let mut moov = mvhd(1000, 12_500);
        moov.extend(trak(b"vide", b"avc1", 1920, 1080));
        moov.extend(trak(b"soun", b"mp4a", 0, 0));
        file.extend(mk(b"moov", &moov));
        file.extend(mk(b"mdat", &[1, 2, 3, 4]));
        file
    }

    #[test]
    fn walks_top_level_boxes() {
        let f = movie();
        let top = boxes(&f, 0..f.len());
        assert_eq!(top.iter().map(|b| b.name()).collect::<Vec<_>>(), vec!["ftyp", "moov", "mdat"]);
        assert_eq!(top[2].payload().len(), 4);
        assert!(boxes(&f[..f.len() - 2], 0..f.len() - 2).len() == 2); // a cut-off mdat is not listed
        assert!(boxes(&[0, 0, 0], 0..3).is_empty());
    }

    #[test]
    fn reads_the_movie_and_its_tracks() {
        let info = parse(&movie()).unwrap();
        assert_eq!(info.brand, "isom");
        assert_eq!(info.timescale, 1000);
        assert_eq!(info.duration_secs(), 12.5);
        assert_eq!(info.tracks.len(), 2);
        let v = info.video().unwrap();
        assert_eq!((v.width, v.height), (1920, 1080));
        assert_eq!(v.codec, "avc1");
        assert_eq!(v.timescale, 30_000);
        assert_eq!(v.duration_units, 90_000);
        assert_eq!(info.tracks[1].kind, TrackKind::Audio);
        assert_eq!(info.tracks[1].codec, "mp4a");
    }

    #[test]
    fn no_moov_is_none() {
        assert_eq!(parse(&mk(b"ftyp", b"isom\0\0\0\0")), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn sixty_four_bit_sizes() {
        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend_from_slice(b"free");
        v.extend_from_slice(&20u64.to_be_bytes());
        v.extend_from_slice(&[9, 9, 9, 9]);
        let b = boxes(&v, 0..v.len());
        assert_eq!((b[0].size, b[0].header), (20, 16));
    }
}
