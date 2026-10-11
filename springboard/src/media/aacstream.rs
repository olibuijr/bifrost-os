//! Self-framed AAC: ADTS (the `audio/aac` radio streams and `.aac` files)
//! and LATM in a LOAS sync stream (`audio/aacp`, `audio/mp4a-latm`, the
//! DAB+/HE-AAC stations). Both carry the same raw access units the MP4 path
//! feeds to `aac::Decoder`; this module finds the frames in a byte stream,
//! resyncing after garbage or a reconnect, and turns each frame's header
//! into a plain AAC-LC AudioSpecificConfig.
//!
//! HE-AAC: symphonia decodes AAC-LC only. For an HE-AAC stream (SBR, SBR+PS)
//! the LC core is still a complete signal at half the output rate, so the
//! config is rewritten to the core's rate and symphonia decodes the core.
//! The SBR payload of each frame (explicit or implicit signalling alike) is
//! then handed to `sbr::Sbr`, which rebuilds the high band and doubles the
//! output rate. Until a frame has shown valid SBR data, and for streams that
//! never do, the core plays as is; a lost SBR payload falls back to the core
//! upsampled. Parametric stereo is not decoded (the mono core stays mono).

use super::aac::Decoder;
use super::audiofile::AudioDecoder;
use super::sbr::Sbr;
use super::mp4::AAC_RATES;
use super::Error;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use symphonia_core::io::MediaSource;

/// Bytes scanned for a first frame before the stream is called not AAC.
const PROBE_LIMIT: usize = 64 * 1024;
const SAMPLES_PER_FRAME: u64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    Adts,
    Loas,
}

/// One access unit found in the byte stream.
#[derive(Debug, PartialEq)]
pub struct Frame {
    /// bytes of the input this frame (header included) occupied
    pub consumed: usize,
    /// AAC-LC AudioSpecificConfig for the frame's core
    pub asc: [u8; 2],
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Frame(Frame),
    /// drop this many bytes (junk, or a frame that carries no audio)
    Skip(usize),
    NeedMore,
}

/// The AudioSpecificConfig of AAC-LC at sample-rate index `rate_idx`.
fn lc_asc(rate_idx: u16, channels: u16) -> [u8; 2] {
    ((2u16 << 11) | (rate_idx << 7) | (channels << 3)).to_be_bytes()
}

/// Is a frame of `len` bytes at the start of `buf` followed by another
/// sync word (so the sync was no coincidence)? None: not enough data yet.
fn chained(buf: &[u8], len: usize, eof: bool, sync: impl Fn(&[u8]) -> bool) -> Option<bool> {
    if buf.len() >= len + 2 {
        Some(sync(&buf[len..]))
    } else if eof {
        Some(buf.len() >= len)
    } else {
        None
    }
}

fn adts_sync(b: &[u8]) -> bool {
    b[0] == 0xFF && b[1] & 0xF6 == 0xF0
}

/// (header length, frame length, config) of an ADTS header, if it is a
/// plain AAC-LC mono/stereo one.
fn adts_header(b: &[u8]) -> Option<(usize, usize, [u8; 2])> {
    if b.len() < 7 || !adts_sync(b) {
        return None;
    }
    let header = if b[1] & 1 == 1 { 7 } else { 9 };
    let (profile, rate_idx) = (b[2] >> 6, (b[2] >> 2) & 0xF);
    let channels = ((b[2] & 1) << 2 | b[3] >> 6) as u16;
    let len = ((b[3] & 3) as usize) << 11 | (b[4] as usize) << 3 | (b[5] >> 5) as usize;
    (profile == 1 && rate_idx < 13 && (1..=2).contains(&channels) && len >= header).then(|| (header, len, lc_asc(rate_idx as u16, channels)))
}

/// The next ADTS frame at the start of `buf`.
pub fn adts_frame(buf: &[u8], eof: bool) -> Parsed {
    if buf.is_empty() {
        return Parsed::NeedMore;
    }
    if buf[0] != 0xFF {
        return Parsed::Skip(buf.iter().position(|&b| b == 0xFF).unwrap_or(buf.len()));
    }
    if buf.len() < 7 {
        return if eof { Parsed::Skip(buf.len()) } else { Parsed::NeedMore };
    }
    let Some((header, len, asc)) = adts_header(buf) else { return Parsed::Skip(1) };
    match chained(buf, len, eof, adts_sync) {
        None => Parsed::NeedMore,
        Some(false) => Parsed::Skip(1),
        Some(true) => Parsed::Frame(Frame { consumed: len, asc, payload: buf[header..len].to_vec() }),
    }
}

fn loas_sync(b: &[u8]) -> bool {
    b[0] == 0x56 && b[1] & 0xE0 == 0xE0
}

/// The stream config a LOAS stream last announced.
#[derive(Default)]
pub struct LatmState {
    asc: Option<[u8; 2]>,
}

/// MSB-first bit reader.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }

    fn left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    fn peek(&self, n: usize) -> Option<u32> {
        if n > self.left() {
            return None;
        }
        let mut v = 0;
        for p in self.pos..self.pos + n {
            v = v << 1 | (self.data[p / 8] >> (7 - p % 8) & 1) as u32;
        }
        Some(v)
    }

    fn bits(&mut self, n: usize) -> Option<u32> {
        let v = self.peek(n)?;
        self.pos += n;
        Some(v)
    }

    fn bit(&mut self) -> Option<bool> {
        self.bits(1).map(|b| b == 1)
    }

    fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        (0..n).map(|_| self.bits(8).map(|b| b as u8)).collect()
    }
}

/// LatmGetValue: 2 bits of byte count, then that many bytes + 1.
fn latm_value(r: &mut Bits<'_>) -> Option<usize> {
    let n = r.bits(2)? as usize;
    let mut v = 0;
    for _ in 0..=n {
        v = v << 8 | r.bits(8)? as usize;
    }
    Some(v)
}

fn object_type(r: &mut Bits<'_>) -> Option<u32> {
    match r.bits(5)? {
        31 => Some(32 + r.bits(6)?),
        t => Some(t),
    }
}

/// A sampling frequency as (index, Hz); an explicit rate must be a
/// standard one.
fn rate_index(r: &mut Bits<'_>) -> Option<usize> {
    match r.bits(4)? {
        15 => {
            let hz = r.bits(24)?;
            AAC_RATES.iter().position(|&x| x == hz)
        }
        i => (i < 13).then_some(i as usize),
    }
}

/// Reads an AudioSpecificConfig and returns the AAC-LC config of its core:
/// SBR / PS extension signalling (explicit, or the trailing sync extension)
/// is parsed past and dropped.
fn read_asc(r: &mut Bits<'_>) -> Option<[u8; 2]> {
    let mut aot = object_type(r)?;
    let core_idx = rate_index(r)?;
    let mut channels = r.bits(4)? as u16;
    let explicit = aot == 5 || aot == 29;
    if explicit {
        if aot == 29 {
            channels = 1; // parametric stereo: the core is mono
        }
        rate_index(r)?;
        aot = object_type(r)?;
    }
    if aot != 2 || channels == 0 || channels > 2 {
        return None; // not LC, or the layout sits in a program config element
    }
    if r.bit()? {
        return None; // 960-sample frames
    }
    if r.bit()? {
        r.bits(14)?;
    }
    if r.bit()? {
        r.bit()?;
    }
    if !explicit && r.peek(11) == Some(0x2b7) {
        r.bits(11)?;
        if object_type(r)? == 5 && r.bit()? {
            rate_index(r)?;
            if r.peek(11) == Some(0x548) {
                r.bits(12)?;
            }
        }
    }
    Some(lc_asc(core_idx as u16, channels))
}

/// StreamMuxConfig of a single-program, single-layer stream.
fn read_stream_mux_config(r: &mut Bits<'_>) -> Option<[u8; 2]> {
    let version = r.bits(1)?;
    if version == 1 && r.bit()? {
        return None; // audioMuxVersionA, reserved
    }
    if version == 1 {
        latm_value(r)?; // taraBufferFullness
    }
    r.bits(1 + 6)?; // allStreamsSameTimeFraming, numSubFrames
    if r.bits(4)? != 0 || r.bits(3)? != 0 {
        return None; // more than one program or layer
    }
    let asc = if version == 0 {
        read_asc(r)?
    } else {
        let len = latm_value(r)?;
        let start = r.pos;
        let asc = read_asc(r);
        r.pos = start + len;
        asc?
    };
    if r.bits(3)? != 0 {
        return None; // frameLengthType other than variable-length payload
    }
    r.bits(8)?; // latmBufferFullness
    if r.bit()? {
        // otherDataPresent
        if version == 1 {
            latm_value(r)?;
        } else {
            while {
                let more = r.bit()?;
                r.bits(8)?;
                more
            } {}
        }
    }
    if r.bit()? {
        r.bits(8)?; // crc
    }
    Some(asc)
}

/// The config and the payload of one AudioMuxElement.
fn read_mux_element(data: &[u8], st: &mut LatmState) -> Option<([u8; 2], Vec<u8>)> {
    let mut r = Bits::new(data);
    if !r.bit()? {
        st.asc = Some(read_stream_mux_config(&mut r)?);
    }
    let asc = st.asc?;
    let mut len = 0;
    loop {
        let t = r.bits(8)? as usize;
        len += t;
        if t != 255 {
            break;
        }
    }
    Some((asc, r.bytes(len)?))
}

/// The next LOAS frame at the start of `buf`. A frame without audio (no
/// config seen yet, or an unsupported layout) is skipped whole.
pub fn loas_frame(buf: &[u8], eof: bool, st: &mut LatmState) -> Parsed {
    if buf.is_empty() {
        return Parsed::NeedMore;
    }
    if buf[0] != 0x56 {
        return Parsed::Skip(buf.iter().position(|&b| b == 0x56).unwrap_or(buf.len()));
    }
    if buf.len() < 3 {
        return if eof { Parsed::Skip(buf.len()) } else { Parsed::NeedMore };
    }
    if !loas_sync(buf) {
        return Parsed::Skip(1);
    }
    let len = ((buf[1] & 0x1F) as usize) << 8 | buf[2] as usize;
    let total = 3 + len;
    if len == 0 {
        return Parsed::Skip(1);
    }
    match chained(buf, total, eof, loas_sync) {
        None => Parsed::NeedMore,
        Some(false) => Parsed::Skip(1),
        Some(true) => match read_mux_element(&buf[3..total], st) {
            Some((asc, payload)) => Parsed::Frame(Frame { consumed: total, asc, payload }),
            None => Parsed::Skip(total),
        },
    }
}

/// An ADTS or LOAS byte stream (file or live) as an `AudioDecoder`.
pub struct AacStream {
    src: Box<dyn MediaSource>,
    framing: Framing,
    latm: LatmState,
    buf: Vec<u8>,
    eof: bool,
    decoder: Option<(Decoder, [u8; 2])>,
    /// the frame the probe found, handed out first
    pending: Option<Frame>,
    /// output rate: the core's, or twice that once SBR is decoding
    rate: u32,
    /// sample rate of the AAC-LC core (length and seek maths run on it)
    core_rate: u32,
    channels: usize,
    duration_ms: Option<f64>,
    sbr: Sbr,
    /// the decoded core of the current frame, input of `sbr`
    core: Vec<i16>,
}

impl AacStream {
    /// Finds the first frame (within the probe limit) and starts decoding
    /// there. `with_length` estimates the length from the source size (for
    /// files; a live stream has none).
    pub fn open(src: Box<dyn MediaSource>, framing: Framing, with_length: bool) -> Result<Self, Error> {
        let mut s = AacStream { src, framing, latm: LatmState::default(), buf: Vec::new(), eof: false, decoder: None, pending: None, rate: 48_000, core_rate: 48_000, channels: 2, duration_ms: None, sbr: Sbr::new(), core: Vec::new() };
        let mut skipped = 0;
        let first = loop {
            match s.parse() {
                Parsed::Frame(f) => break f,
                Parsed::Skip(n) => {
                    skipped += n;
                    s.buf.drain(..n.max(1).min(s.buf.len()));
                }
                Parsed::NeedMore => {
                    if s.eof || !s.fill() {
                        return Err(Error::Invalid("AAC stream"));
                    }
                }
            }
            if skipped > PROBE_LIMIT {
                return Err(Error::Invalid("AAC stream"));
            }
        };
        s.buf.drain(..first.consumed);
        if !s.tune(first.asc) {
            return Err(Error::Invalid("AAC stream"));
        }
        if let (true, true, Some(bytes)) = (with_length, s.src.is_seekable(), s.src.byte_len()) {
            // mean size of the frames in view; one frame alone is a poor guess for VBR streams
            let (mut total, mut count, mut off) = (first.consumed, 1usize, 0usize);
            if matches!(framing, Framing::Adts) {
                while count < 24 {
                    match adts_frame(&s.buf[off..], true) {
                        Parsed::Frame(f) => {
                            off += f.consumed;
                            total += f.consumed;
                            count += 1;
                        }
                        _ => break,
                    }
                }
            }
            let mean = (total / count).max(1) as u64;
            s.duration_ms = Some((bytes / mean * SAMPLES_PER_FRAME) as f64 * 1000.0 / s.rate as f64);
        }
        s.pending = Some(first);
        Ok(s)
    }

    fn parse(&mut self) -> Parsed {
        match self.framing {
            Framing::Adts => adts_frame(&self.buf, self.eof),
            Framing::Loas => loas_frame(&self.buf, self.eof, &mut self.latm),
        }
    }

    /// Reads more input; false when the source has ended.
    fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 4096];
        loop {
            match self.src.read(&mut chunk) {
                Ok(0) => {
                    self.eof = true;
                    return false;
                }
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    return true;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    self.eof = true;
                    return false;
                }
            }
        }
    }

    /// The next frame, None at the end of the source.
    fn next_frame(&mut self) -> Option<Frame> {
        loop {
            if self.buf.is_empty() && self.eof {
                return None;
            }
            match self.parse() {
                Parsed::Frame(f) => {
                    self.buf.drain(..f.consumed);
                    return Some(f);
                }
                Parsed::Skip(n) => {
                    self.buf.drain(..n.max(1).min(self.buf.len()));
                }
                Parsed::NeedMore => {
                    if !self.fill() && self.buf.is_empty() {
                        return None;
                    }
                }
            }
        }
    }

    /// Makes the decoder match `asc` (a new one when the stream's config
    /// changed); false when it cannot be built.
    fn tune(&mut self, asc: [u8; 2]) -> bool {
        if self.decoder.as_ref().map(|d| d.1) != Some(asc) {
            match Decoder::new(&asc) {
                Ok(d) => {
                    self.rate = d.rate();
                    self.core_rate = d.rate();
                    self.channels = d.channels();
                    self.sbr.reset();
                    self.decoder = Some((d, asc));
                }
                Err(_) => {
                    self.decoder = None;
                    return false;
                }
            }
        }
        true
    }
}

impl AudioDecoder for AacStream {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn duration_ms(&self) -> Option<f64> {
        self.duration_ms
    }

    fn next(&mut self, out: &mut Vec<i16>) -> Result<bool, Error> {
        out.clear();
        let Some(frame) = self.pending.take().or_else(|| self.next_frame()) else { return Ok(false) };
        if self.tune(frame.asc) {
            if let Some((d, _)) = &mut self.decoder {
                d.decode(&frame.payload, &mut self.core)?;
                self.core_rate = d.rate();
                self.channels = d.channels();
                if self.sbr.process(&frame.payload, &self.core, self.channels, self.core_rate, out) {
                    self.rate = self.core_rate * 2;
                } else {
                    std::mem::swap(out, &mut self.core);
                    self.rate = self.core_rate;
                }
            }
        }
        Ok(true)
    }

    fn seek_ms(&mut self, ms: f64) -> Result<f64, Error> {
        if !self.src.is_seekable() {
            return Err(Error::Invalid("a live stream cannot seek"));
        }
        self.src.seek(SeekFrom::Start(0))?;
        self.buf.clear();
        self.eof = false;
        self.pending = None;
        self.latm = LatmState::default();
        let want = (ms.max(0.0) * self.core_rate as f64 / 1000.0 / SAMPLES_PER_FRAME as f64) as u64;
        let mut at = 0;
        while at < want {
            if self.next_frame().is_none() {
                break;
            }
            at += 1;
        }
        if let Some((d, _)) = &mut self.decoder {
            d.reset();
        }
        self.sbr.flush();
        Ok((at * SAMPLES_PER_FRAME) as f64 * 1000.0 / self.core_rate as f64)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::audiofile::tests::{drain, power, M4A};
    use super::super::mp4::{Codec, Demuxer, Kind};
    use super::super::source::MemSource;
    use super::*;
    use std::io::Cursor;

    /// A seekable in-memory media source.
    pub struct Mem(pub Cursor<Vec<u8>>);

    impl std::io::Read for Mem {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(b)
        }
    }

    impl std::io::Seek for Mem {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(p)
        }
    }

    impl MediaSource for Mem {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            Some(self.0.get_ref().len() as u64)
        }
    }

    /// The fixture's raw AAC access units and its ASC (44.1 kHz stereo LC).
    pub fn raw_frames() -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut src = MemSource::new(M4A.to_vec());
        let mut d = Demuxer::open(&mut src).unwrap();
        let Codec::Aac { asc, .. } = d.audio().unwrap().codec.clone() else { panic!() };
        let mut frames = Vec::new();
        while let Some(s) = d.next(Kind::Audio, &mut src).unwrap() {
            frames.push(s.data.to_vec());
        }
        (asc, frames)
    }

    pub fn adts(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for f in frames {
            let len = f.len() + 7;
            // LC, 44.1 kHz (index 4), stereo, no CRC
            out.extend([0xFF, 0xF1, 0x50, 0x80 | (len >> 11) as u8 & 3, (len >> 3) as u8, (len << 5) as u8 | 0x1F, 0xFC]);
            out.extend(f);
        }
        out
    }

    #[derive(Default)]
    pub struct W {
        bytes: Vec<u8>,
        n: usize,
    }

    impl W {
        pub fn put(&mut self, v: u32, bits: usize) {
            for i in (0..bits).rev() {
                if self.n % 8 == 0 {
                    self.bytes.push(0);
                }
                *self.bytes.last_mut().unwrap() |= ((v >> i & 1) as u8) << (7 - self.n % 8);
                self.n += 1;
            }
        }
    }

    /// One LOAS frame; `config` writes the StreamMuxConfig for `asc_bits`.
    pub fn loas(payload: &[u8], asc_bits: Option<&dyn Fn(&mut W)>) -> Vec<u8> {
        let mut w = W::default();
        match asc_bits {
            Some(asc) => {
                w.put(0, 1); // useSameStreamMux = 0
                w.put(0, 1); // audioMuxVersion
                w.put(1, 1); // allStreamsSameTimeFraming
                w.put(0, 6);
                w.put(0, 4);
                w.put(0, 3);
                asc(&mut w);
                w.put(0, 3); // frameLengthType
                w.put(0xFF, 8);
                w.put(0, 1); // otherData
                w.put(0, 1); // crc
            }
            None => w.put(1, 1),
        }
        let mut left = payload.len();
        while left >= 255 {
            w.put(255, 8);
            left -= 255;
        }
        w.put(left as u32, 8);
        for &b in payload {
            w.put(b as u32, 8);
        }
        let len = w.bytes.len();
        let mut out = vec![0x56, 0xE0 | (len >> 8) as u8, len as u8];
        out.extend(w.bytes);
        out
    }

    /// Plain LC ASC bits for 44.1 kHz stereo.
    pub fn lc_bits(w: &mut W) {
        w.put(2, 5);
        w.put(4, 4);
        w.put(2, 4);
        w.put(0, 3);
    }

    /// Explicit HE-AAC signalling: SBR (aot 5), core 22.05 kHz, out 44.1.
    fn he_bits(w: &mut W) {
        w.put(5, 5);
        w.put(7, 4); // core 22050
        w.put(2, 4);
        w.put(4, 4); // extension rate 44100
        w.put(2, 5); // core object type
        w.put(0, 3);
    }

    fn loas_stream(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            out.extend(loas(f, (i % 20 == 0).then_some(&lc_bits as &dyn Fn(&mut W))));
        }
        out
    }

    fn assert_tone(pcm: &[i16]) {
        let tail = &pcm[pcm.len() / 2..];
        assert!(power(tail, 2, 44100, 440.0) > 20.0 * power(tail, 2, 44100, 1000.0).max(1e-9));
        assert!(tail.iter().any(|&s| s.abs() > 2000));
    }

    #[test]
    fn adts_frames_are_found_and_resync_after_junk() {
        let (_, frames) = raw_frames();
        let mut data = vec![1, 2, 3, 0xFF, 0x00];
        data.extend(adts(&frames[..4]));
        let mut off = 0;
        let mut got = Vec::new();
        loop {
            match adts_frame(&data[off..], true) {
                Parsed::Frame(f) => {
                    off += f.consumed;
                    got.push(f.payload);
                }
                Parsed::Skip(n) => off += n,
                Parsed::NeedMore => break,
            }
            if off >= data.len() {
                break;
            }
        }
        assert_eq!(got, frames[..4]);
        assert!(matches!(adts_frame(&data[5..12], false), Parsed::NeedMore), "needs the whole frame and the next sync");
    }

    #[test]
    fn adts_streams_decode_seek_and_report_a_length() {
        let (_, frames) = raw_frames();
        let mut d = AacStream::open(Box::new(Mem(Cursor::new(adts(&frames)))), Framing::Adts, true).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        let dur = d.duration_ms().unwrap();
        assert!((dur - 1200.0).abs() < 200.0, "estimated {dur}");
        let pcm = drain(&mut d);
        assert!(pcm.len() / 2 >= 44100, "{} frames", pcm.len() / 2);
        assert_tone(&pcm);
        let at = d.seek_ms(600.0).unwrap();
        assert!(at <= 600.0 && at > 500.0, "landed at {at}");
        let rest = drain(&mut d).len();
        assert!(rest > pcm.len() * 4 / 10 && rest < pcm.len() * 6 / 10, "{rest} of {}", pcm.len());
    }

    #[test]
    fn loas_streams_decode_with_the_config_sent_only_now_and_then() {
        let (_, frames) = raw_frames();
        let data = loas_stream(&frames);
        // a live stream joins mid-way: drop the first frames, so the first
        // audio waits for the next config
        let skip = loas(&frames[0], Some(&lc_bits)).len() + 5;
        let mut d = AacStream::open(Box::new(Mem(Cursor::new(data[skip..].to_vec()))), Framing::Loas, false).unwrap();
        assert_eq!((d.rate(), d.channels()), (44100, 2));
        assert_eq!(d.duration_ms(), None);
        let pcm = drain(&mut d);
        assert!(pcm.len() / 2 > 44100 / 2, "{} frames", pcm.len() / 2);
        assert_tone(&pcm);
    }

    #[test]
    fn he_aac_config_is_reduced_to_its_lc_core() {
        let mut st = LatmState::default();
        let f = loas(&[1, 2, 3], Some(&he_bits));
        let Parsed::Frame(fr) = loas_frame(&[f.clone(), vec![0x56, 0xE0, 0x01, 0]].concat(), false, &mut st) else { panic!() };
        assert_eq!(fr.asc, lc_asc(7, 2), "core rate 22.05 kHz, LC");
        assert_eq!(fr.payload, [1, 2, 3]);
        assert_eq!(super::super::mp4::asc_params(&fr.asc), Some((22050, 2)));
    }

    /// libfdk-aac HE-AAC: 44.1 kHz stereo, 22.05 kHz core, 1.16 s of the
    /// harmonics of 440 Hz (every 880 Hz) up to 16 kHz, amplitude modulated;
    /// the SBR header comes in frames 0, 10 and 20. The reference is the left
    /// channel samples 20000..20400 as FFmpeg and faad2 decode it (they agree
    /// to the sample).
    const HE_TONE: &[u8] = include_bytes!("testdata/he_aac_tone.adts");
    const HE_TONE_REF_L: &[u8] = include_bytes!("testdata/he_aac_tone_ref_l.raw");

    #[test]
    fn he_aac_streams_get_their_high_band_back() {
        let mut d = AacStream::open(Box::new(Mem(Cursor::new(HE_TONE.to_vec()))), Framing::Adts, false).unwrap();
        assert_eq!(d.rate(), 22050, "the core rate, until a frame has shown SBR data");
        let pcm = drain(&mut d);
        assert_eq!((d.rate(), d.channels()), (44100, 2), "twice the core rate once SBR decodes");
        assert_eq!(pcm.len(), 25 * 2048 * 2);

        let reference: Vec<i16> = HE_TONE_REF_L.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(reference.len(), 400);
        let worst = reference.iter().enumerate().map(|(i, &r)| (pcm[2 * (20000 + i)] as i32 - r as i32).abs()).max().unwrap();
        assert!(worst <= 32, "up to {worst} off the reference decoders");

        let tail = &pcm[pcm.len() / 2..];
        // the 31st harmonic (13.64 kHz) lies above the 11 kHz core; a core-only decode has none of it.
        // FFmpeg measures the same ~1e-4 here (the high band is quiet next to the 2.2 kHz tone).
        let hf = power(tail, 2, 44100, 13640.0);
        assert!(hf > 5e-5, "the harmonic above the core's 11 kHz is back: {hf}");
        assert!(hf > 20.0 * power(tail, 2, 44100, 18000.0).max(1e-9), "and nothing above the stream's own band");
    }

    #[test]
    fn he_aac_streams_seek_and_keep_decoding_the_high_band() {
        let mut d = AacStream::open(Box::new(Mem(Cursor::new(HE_TONE.to_vec()))), Framing::Adts, true).unwrap();
        let first = drain(&mut d);
        let at = d.seek_ms(500.0).unwrap();
        assert!(at <= 500.0 && at > 400.0, "landed at {at} ms (core frames of 46 ms)");
        let rest = drain(&mut d);
        assert_eq!(d.rate(), 44100);
        assert!(!rest.is_empty() && rest.len() < first.len());
        let tail = &rest[rest.len() - 5 * 2048 * 2..]; // the frames after the header of frame 20
        let hf = power(tail, 2, 44100, 13640.0);
        assert!(hf > 5e-5, "the high band survives the seek: {hf}");
    }

    #[test]
    fn a_frame_before_any_config_is_skipped_and_garbage_is_not_aac() {
        let mut st = LatmState::default();
        let f = loas(&[9; 10], None);
        assert_eq!(loas_frame(&[f.clone(), f.clone()].concat(), false, &mut st), Parsed::Skip(f.len()));
        assert!(AacStream::open(Box::new(Mem(Cursor::new(vec![0x41; 100_000]))), Framing::Adts, false).is_err());
        assert!(AacStream::open(Box::new(Mem(Cursor::new(vec![0x41; 100_000]))), Framing::Loas, false).is_err());
    }

    #[test]
    fn long_payload_lengths_use_the_escape_byte() {
        let mut st = LatmState::default();
        let payload = vec![7u8; 600];
        let f = loas(&payload, Some(&lc_bits));
        let Parsed::Frame(fr) = loas_frame(&[f.clone(), f.clone()].concat(), false, &mut st) else { panic!() };
        assert_eq!(fr.payload, payload);
    }
}
