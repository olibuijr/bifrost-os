//! Video encoding for apps (Video Toolbox: `VTCompressionSession`, `VTFrameSilo`, `VTMultiPassStorage`,
//! `VTFrameProcessor`).
//!
//! The only encoder in this build is a lossless intra-frame one: every frame becomes a PNG
//! (`springboard::pngenc`, stored deflate blocks), so every frame is a key frame and the output is large.
//! [`Codec::H264`] and [`Codec::Hevc`] are named so app code keeps the reference shape, and creating a
//! session with them fails with [`Status::CouldNotFindVideoEncoder`]. Output order is submission order
//! (no frame reordering), so presentation and decode timestamps are equal.
//!
//! A session is driven as in the reference: `create`, `set_property`, `prepare_to_encode_frames`,
//! `encode_frame` (the output callback fires per frame), `complete_frames` (flush up to a time) and
//! `invalidate`. Multi-pass encoding records per-frame sizes in a [`MultiPassStorage`] and keeps
//! finished frames in a [`FrameSilo`] until the last pass.

use springboard::pngenc;
use std::collections::BTreeMap;

/// Video Toolbox status codes this module reports (`kVTParameterErr` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    ParameterErr,
    InvalidSessionErr,
    CouldNotFindVideoEncoder,
    PropertyNotSupported,
    PropertyReadOnly,
    FrameSiloInvalidTimeStamp,
    MultiPassStorageIdentifierMismatch,
}

/// Property keys (`kVTCompressionPropertyKey_*`).
pub mod key {
    pub const AVERAGE_BIT_RATE: &str = "AverageBitRate";
    pub const EXPECTED_FRAME_RATE: &str = "ExpectedFrameRate";
    pub const MAX_KEY_FRAME_INTERVAL: &str = "MaxKeyFrameInterval";
    pub const REAL_TIME: &str = "RealTime";
    pub const ALLOW_FRAME_REORDERING: &str = "AllowFrameReordering";
    pub const QUALITY: &str = "Quality";
    pub const PROFILE_LEVEL: &str = "ProfileLevel";
    /// Read-only: frames the encoder holds back (always 0 here).
    pub const NUMBER_OF_PENDING_FRAMES: &str = "NumberOfPendingFrames";
}

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

/// The codec types (`CMVideoCodecType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// Lossless, intra only: one PNG per frame.
    Png,
    H264,
    Hevc,
}

impl Codec {
    pub fn is_supported(self) -> bool {
        self == Codec::Png
    }
}

/// An uncompressed frame (the image buffer handed to `encode_frame`): `u32` pixels as
/// `pngenc::encode_rgb` reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Frame {
    pub fn new(width: usize, height: usize, pixels: Vec<u32>) -> Option<Frame> {
        (pixels.len() >= width * height && width > 0 && height > 0).then_some(Frame { width, height, pixels })
    }
}

/// An encoded sample (the sample buffer given to the output callback).
#[derive(Clone, Debug, PartialEq)]
pub struct EncodedFrame {
    /// Presentation time stamp, in the caller's time scale (ticks).
    pub pts: i64,
    pub duration: i64,
    pub data: Vec<u8>,
    /// Always true for the intra-only encoder.
    pub is_key_frame: bool,
}

/// Output callback (`VTCompressionOutputCallback`).
pub type OutputCallback = Box<dyn FnMut(Result<EncodedFrame, Status>)>;

/// A hook run on every frame before encoding (the `VTFrameProcessor` slot).
pub trait FrameProcessor {
    fn process(&mut self, frame: Frame) -> Frame;
}

/// `VTSuperResolutionScalerConfiguration` shape: integer nearest-neighbour upscaling. A model backend
/// would implement [`FrameProcessor`] the same way.
pub struct SuperResolutionScaler {
    factor: usize,
}

impl SuperResolutionScaler {
    /// `factor` must be 1 or more.
    pub fn new(factor: usize) -> Option<Self> {
        (factor >= 1).then_some(SuperResolutionScaler { factor })
    }
}

impl FrameProcessor for SuperResolutionScaler {
    fn process(&mut self, f: Frame) -> Frame {
        let k = self.factor;
        let (w, h) = (f.width * k, f.height * k);
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                px.push(f.pixels[(y / k) * f.width + x / k]);
            }
        }
        Frame { width: w, height: h, pixels: px }
    }
}

/// A frame processor session (`VTFrameProcessor`): start, process, end.
pub struct FrameProcessorSession {
    processor: Box<dyn FrameProcessor>,
    started: bool,
}

impl FrameProcessorSession {
    pub fn new(processor: Box<dyn FrameProcessor>) -> Self {
        FrameProcessorSession { processor, started: false }
    }
    pub fn start_session(&mut self) {
        self.started = true;
    }
    pub fn process_frame(&mut self, frame: Frame) -> Result<Frame, Status> {
        if !self.started {
            return Err(Status::InvalidSessionErr);
        }
        Ok(self.processor.process(frame))
    }
    pub fn end_session(&mut self) {
        self.started = false;
    }
}

/// `VTFrameSilo`: finished samples kept by time stamp until the last pass.
#[derive(Default)]
pub struct FrameSilo {
    samples: BTreeMap<i64, EncodedFrame>,
}

impl FrameSilo {
    pub fn new() -> Self {
        Self::default()
    }
    /// `VTFrameSiloAddSampleBuffer`. Time stamps must be unique.
    pub fn add_sample(&mut self, s: EncodedFrame) -> Result<(), Status> {
        if self.samples.contains_key(&s.pts) {
            return Err(Status::FrameSiloInvalidTimeStamp);
        }
        self.samples.insert(s.pts, s);
        Ok(())
    }
    /// `VTFrameSiloCallFunctionForEachSampleBuffer`: samples with `start <= pts < end`, in time order.
    pub fn for_each_sample(&self, start: i64, end: i64, mut f: impl FnMut(&EncodedFrame)) {
        if start < end {
            self.samples.range(start..end).for_each(|(_, s)| f(s));
        }
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    /// A later pass replaces the earlier sample of the same time.
    fn replace(&mut self, s: EncodedFrame) {
        self.samples.insert(s.pts, s);
    }
}

/// `VTMultiPassStorage`: per-frame analysis (here the encoded size) keyed by time stamp, with a byte
/// form for keeping it between runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiPassStorage {
    entries: BTreeMap<i64, u64>,
    closed: bool,
}

impl MultiPassStorage {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, pts: i64, size: u64) -> Result<(), Status> {
        if self.closed {
            return Err(Status::InvalidSessionErr);
        }
        self.entries.insert(pts, size);
        Ok(())
    }
    pub fn get(&self, pts: i64) -> Option<u64> {
        self.entries.get(&pts).copied()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `VTMultiPassStorageClose`: no more writes.
    pub fn close(&mut self) {
        self.closed = true;
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = b"VTMP".to_vec();
        out.extend((self.entries.len() as u64).to_le_bytes());
        for (k, v) in &self.entries {
            out.extend(k.to_le_bytes());
            out.extend(v.to_le_bytes());
        }
        out
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, Status> {
        if b.len() < 12 || &b[..4] != b"VTMP" {
            return Err(Status::MultiPassStorageIdentifierMismatch);
        }
        let n = u64::from_le_bytes(b[4..12].try_into().unwrap());
        let want = n.checked_mul(16).and_then(|x| x.checked_add(12));
        if want != Some(b.len() as u64) {
            return Err(Status::MultiPassStorageIdentifierMismatch);
        }
        let mut entries = BTreeMap::new();
        for rec in b[12..].chunks_exact(16) {
            entries.insert(
                i64::from_le_bytes(rec[..8].try_into().unwrap()),
                u64::from_le_bytes(rec[8..].try_into().unwrap()),
            );
        }
        Ok(MultiPassStorage { entries, closed: false })
    }
}

/// `VTCompressionSession`.
pub struct CompressionSession {
    width: usize,
    height: usize,
    codec: Codec,
    props: BTreeMap<&'static str, Value>,
    callback: OutputCallback,
    processor: Option<FrameProcessorSession>,
    storage: Option<MultiPassStorage>,
    silo: Option<FrameSilo>,
    pass: Option<u32>,
    valid: bool,
    last_pts: Option<i64>,
}

impl CompressionSession {
    /// `VTCompressionSessionCreate`.
    pub fn create(width: usize, height: usize, codec: Codec, callback: OutputCallback) -> Result<Self, Status> {
        if width == 0 || height == 0 {
            return Err(Status::ParameterErr);
        }
        if !codec.is_supported() {
            return Err(Status::CouldNotFindVideoEncoder);
        }
        let mut props = BTreeMap::new();
        props.insert(key::REAL_TIME, Value::Bool(false));
        props.insert(key::ALLOW_FRAME_REORDERING, Value::Bool(true));
        props.insert(key::QUALITY, Value::Float(1.0));
        props.insert(key::MAX_KEY_FRAME_INTERVAL, Value::Int(0));
        Ok(CompressionSession {
            width,
            height,
            codec,
            props,
            callback,
            processor: None,
            storage: None,
            silo: None,
            pass: None,
            valid: true,
            last_pts: None,
        })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// The canonical key and whether it is writable.
    fn spec(name: &str) -> Option<(&'static str, bool)> {
        [
            (key::AVERAGE_BIT_RATE, true),
            (key::EXPECTED_FRAME_RATE, true),
            (key::MAX_KEY_FRAME_INTERVAL, true),
            (key::REAL_TIME, true),
            (key::ALLOW_FRAME_REORDERING, true),
            (key::QUALITY, true),
            (key::PROFILE_LEVEL, true),
            (key::NUMBER_OF_PENDING_FRAMES, false),
        ]
        .into_iter()
        .find(|(k, _)| *k == name)
    }

    /// `VTSessionSetProperty`. Checks the key, the writability and the value type and range.
    pub fn set_property(&mut self, name: &str, value: Value) -> Result<(), Status> {
        let (k, writable) = Self::spec(name).ok_or(Status::PropertyNotSupported)?;
        if !writable {
            return Err(Status::PropertyReadOnly);
        }
        let ok = match (k, &value) {
            (key::AVERAGE_BIT_RATE, Value::Int(v)) => *v > 0,
            (key::EXPECTED_FRAME_RATE, Value::Float(v)) => *v > 0.0,
            (key::EXPECTED_FRAME_RATE, Value::Int(v)) => *v > 0,
            (key::MAX_KEY_FRAME_INTERVAL, Value::Int(v)) => *v >= 0,
            (key::QUALITY, Value::Float(v)) => (0.0..=1.0).contains(v),
            (key::PROFILE_LEVEL, Value::Text(_)) => true,
            (key::REAL_TIME, Value::Bool(_)) | (key::ALLOW_FRAME_REORDERING, Value::Bool(_)) => true,
            _ => false,
        };
        if !ok {
            return Err(Status::ParameterErr);
        }
        self.props.insert(k, value);
        Ok(())
    }

    /// `VTSessionCopyProperty`: `Ok(None)` for a supported key that has no value yet.
    pub fn copy_property(&self, name: &str) -> Result<Option<Value>, Status> {
        let (k, _) = Self::spec(name).ok_or(Status::PropertyNotSupported)?;
        if k == key::NUMBER_OF_PENDING_FRAMES {
            return Ok(Some(Value::Int(0)));
        }
        Ok(self.props.get(k).cloned())
    }

    /// Installs a frame processing hook that runs before encoding.
    pub fn set_frame_processor(&mut self, mut p: FrameProcessorSession) {
        p.start_session();
        self.processor = Some(p);
    }

    /// `VTCompressionSessionPrepareToEncodeFrames`.
    pub fn prepare_to_encode_frames(&mut self) -> Result<(), Status> {
        self.check_valid()
    }

    /// Multi-pass: the storage and a fresh silo used by `begin_pass` / `end_pass`.
    pub fn set_multi_pass(&mut self, storage: MultiPassStorage) {
        self.storage = Some(storage);
        self.silo = Some(FrameSilo::new());
    }

    /// `VTCompressionSessionBeginPass`.
    pub fn begin_pass(&mut self, pass_id: u32) -> Result<(), Status> {
        self.check_valid()?;
        if self.storage.is_none() || self.pass.is_some() {
            return Err(Status::ParameterErr);
        }
        self.pass = Some(pass_id);
        self.last_pts = None;
        Ok(())
    }

    /// `VTCompressionSessionEndPass`: reports whether another pass is wanted (the first pass asks for
    /// one more; the second does not).
    pub fn end_pass(&mut self) -> Result<bool, Status> {
        self.check_valid()?;
        let id = self.pass.take().ok_or(Status::ParameterErr)?;
        Ok(id == 0)
    }

    /// The silo with the samples of the passes so far.
    pub fn silo(&self) -> Option<&FrameSilo> {
        self.silo.as_ref()
    }

    /// The multi-pass storage, with the sizes recorded so far.
    pub fn storage(&self) -> Option<&MultiPassStorage> {
        self.storage.as_ref()
    }

    /// `VTCompressionSessionEncodeFrame`. Time stamps must increase. The callback fires before this returns.
    pub fn encode_frame(&mut self, frame: Frame, pts: i64, duration: i64) -> Result<(), Status> {
        self.check_valid()?;
        if self.last_pts.map_or(false, |l| pts <= l) {
            return Err(Status::ParameterErr);
        }
        let frame = match self.processor.as_mut() {
            Some(p) => p.process_frame(frame)?,
            None => frame,
        };
        if frame.width != self.width || frame.height != self.height {
            return Err(Status::ParameterErr);
        }
        self.last_pts = Some(pts);
        let data = pngenc::encode_rgb(frame.width, frame.height, &frame.pixels);
        let out = EncodedFrame { pts, duration, data, is_key_frame: true };
        if self.pass.is_some() {
            if let Some(st) = self.storage.as_mut() {
                st.set(pts, out.data.len() as u64)?;
            }
            if let Some(silo) = self.silo.as_mut() {
                silo.replace(out.clone());
            }
        }
        (self.callback)(Ok(out));
        Ok(())
    }

    /// `VTCompressionSessionCompleteFrames`. Frames are emitted as they are submitted, so nothing is held;
    /// this only validates the session.
    pub fn complete_frames(&mut self, _until_pts: Option<i64>) -> Result<(), Status> {
        self.check_valid()
    }

    /// `VTCompressionSessionInvalidate`.
    pub fn invalidate(&mut self) {
        self.valid = false;
        if let Some(p) = self.processor.as_mut() {
            p.end_session();
        }
        if let Some(s) = self.storage.as_mut() {
            s.close();
        }
    }

    fn check_valid(&self) -> Result<(), Status> {
        if self.valid {
            Ok(())
        } else {
            Err(Status::InvalidSessionErr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Out = Rc<RefCell<Vec<Result<EncodedFrame, Status>>>>;

    fn session(w: usize, h: usize) -> (CompressionSession, Out) {
        let out: Out = Rc::default();
        let o = out.clone();
        let s = CompressionSession::create(w, h, Codec::Png, Box::new(move |r| o.borrow_mut().push(r))).unwrap();
        (s, out)
    }

    fn frame(w: usize, h: usize, c: u32) -> Frame {
        Frame::new(w, h, vec![c; w * h]).unwrap()
    }

    #[test]
    fn create_rejects_zero_size_and_missing_codecs() {
        let cb = || -> OutputCallback { Box::new(|_| {}) };
        assert_eq!(CompressionSession::create(0, 4, Codec::Png, cb()).err(), Some(Status::ParameterErr));
        assert_eq!(CompressionSession::create(4, 4, Codec::H264, cb()).err(), Some(Status::CouldNotFindVideoEncoder));
        assert_eq!(CompressionSession::create(4, 4, Codec::Hevc, cb()).err(), Some(Status::CouldNotFindVideoEncoder));
    }

    #[test]
    fn properties_validate_key_type_range_and_writability() {
        let (mut s, _) = session(2, 2);
        assert_eq!(s.set_property("Nope", Value::Int(1)), Err(Status::PropertyNotSupported));
        assert_eq!(s.set_property(key::NUMBER_OF_PENDING_FRAMES, Value::Int(1)), Err(Status::PropertyReadOnly));
        assert_eq!(s.set_property(key::AVERAGE_BIT_RATE, Value::Int(0)), Err(Status::ParameterErr));
        assert_eq!(s.set_property(key::AVERAGE_BIT_RATE, Value::Bool(true)), Err(Status::ParameterErr));
        assert_eq!(s.set_property(key::QUALITY, Value::Float(1.5)), Err(Status::ParameterErr));
        assert_eq!(s.set_property(key::AVERAGE_BIT_RATE, Value::Int(4_000_000)), Ok(()));
        assert_eq!(s.copy_property(key::AVERAGE_BIT_RATE), Ok(Some(Value::Int(4_000_000))));
        assert_eq!(s.copy_property(key::REAL_TIME), Ok(Some(Value::Bool(false))));
        assert_eq!(s.copy_property("Nope"), Err(Status::PropertyNotSupported));
    }

    #[test]
    fn frames_come_out_in_order_with_timestamps_as_png() {
        let (mut s, out) = session(3, 2);
        s.prepare_to_encode_frames().unwrap();
        s.encode_frame(frame(3, 2, 0xff0000), 0, 10).unwrap();
        s.encode_frame(frame(3, 2, 0x00ff00), 10, 10).unwrap();
        s.complete_frames(None).unwrap();
        let o = out.borrow();
        assert_eq!(o.len(), 2);
        let a = o[0].as_ref().unwrap();
        let b = o[1].as_ref().unwrap();
        assert_eq!((a.pts, a.duration, b.pts), (0, 10, 10));
        assert!(a.is_key_frame);
        assert_eq!(&a.data[..8], b"\x89PNG\r\n\x1a\n");
        assert_ne!(a.data, b.data);
    }

    #[test]
    fn out_of_order_or_wrong_size_frames_are_rejected() {
        let (mut s, out) = session(2, 2);
        s.encode_frame(frame(2, 2, 1), 5, 1).unwrap();
        assert_eq!(s.encode_frame(frame(2, 2, 1), 5, 1), Err(Status::ParameterErr));
        assert_eq!(s.encode_frame(frame(3, 3, 1), 6, 1), Err(Status::ParameterErr));
        assert_eq!(out.borrow().len(), 1);
    }

    #[test]
    fn invalidated_session_refuses_work() {
        let (mut s, _) = session(2, 2);
        s.invalidate();
        assert_eq!(s.encode_frame(frame(2, 2, 1), 0, 1), Err(Status::InvalidSessionErr));
        assert_eq!(s.complete_frames(None), Err(Status::InvalidSessionErr));
        assert_eq!(s.prepare_to_encode_frames(), Err(Status::InvalidSessionErr));
    }

    #[test]
    fn multi_pass_records_sizes_fills_the_silo_and_asks_for_a_second_pass() {
        let (mut s, _) = session(2, 2);
        assert_eq!(s.begin_pass(0), Err(Status::ParameterErr)); // no storage yet
        s.set_multi_pass(MultiPassStorage::new());
        s.begin_pass(0).unwrap();
        s.encode_frame(frame(2, 2, 7), 0, 1).unwrap();
        s.encode_frame(frame(2, 2, 8), 1, 1).unwrap();
        assert_eq!(s.end_pass(), Ok(true));
        assert_eq!(s.storage().unwrap().len(), 2);
        assert!(s.storage().unwrap().get(1).unwrap() > 0);
        s.begin_pass(1).unwrap();
        s.encode_frame(frame(2, 2, 9), 0, 1).unwrap();
        assert_eq!(s.end_pass(), Ok(false));
        let mut seen = vec![];
        s.silo().unwrap().for_each_sample(0, 2, |f| seen.push(f.pts));
        assert_eq!(seen, vec![0, 1]);
    }

    #[test]
    fn multi_pass_storage_round_trips_and_rejects_garbage() {
        let mut m = MultiPassStorage::new();
        m.set(3, 100).unwrap();
        m.set(-1, 7).unwrap();
        let back = MultiPassStorage::from_bytes(&m.to_bytes()).unwrap();
        assert_eq!(back, m);
        assert_eq!(MultiPassStorage::from_bytes(b"nope"), Err(Status::MultiPassStorageIdentifierMismatch));
        let mut cut = m.to_bytes();
        cut.pop();
        assert!(MultiPassStorage::from_bytes(&cut).is_err());
        m.close();
        assert_eq!(m.set(9, 1), Err(Status::InvalidSessionErr));
    }

    #[test]
    fn silo_rejects_duplicate_times_and_ranges() {
        let mut silo = FrameSilo::new();
        let s = |pts| EncodedFrame { pts, duration: 1, data: vec![], is_key_frame: true };
        silo.add_sample(s(2)).unwrap();
        silo.add_sample(s(1)).unwrap();
        assert_eq!(silo.add_sample(s(2)), Err(Status::FrameSiloInvalidTimeStamp));
        let mut seen = vec![];
        silo.for_each_sample(2, 3, |f| seen.push(f.pts));
        assert_eq!(seen, vec![2]);
    }

    #[test]
    fn frame_processor_runs_before_encoding() {
        let (mut s, out) = session(4, 4);
        let sr = SuperResolutionScaler::new(2).unwrap();
        s.set_frame_processor(FrameProcessorSession::new(Box::new(sr)));
        s.encode_frame(frame(2, 2, 5), 0, 1).unwrap(); // upscaled to 4x4
        assert_eq!(out.borrow().len(), 1);
        assert!(SuperResolutionScaler::new(0).is_none());
    }

    #[test]
    fn super_resolution_repeats_pixels_and_session_must_be_started() {
        let mut sr = SuperResolutionScaler::new(2).unwrap();
        let f = Frame::new(2, 1, vec![1, 2]).unwrap();
        assert_eq!(sr.process(f.clone()).pixels, vec![1, 1, 2, 2, 1, 1, 2, 2]);
        let mut fp = FrameProcessorSession::new(Box::new(SuperResolutionScaler::new(1).unwrap()));
        assert_eq!(fp.process_frame(f.clone()), Err(Status::InvalidSessionErr));
        fp.start_session();
        assert_eq!(fp.process_frame(f.clone()), Ok(f));
    }
}
