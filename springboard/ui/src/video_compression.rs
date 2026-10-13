//! Compression sessions: the shape of a video encoder session, with frames in, output callbacks and frame flags. A
//! session is opened on an encoder in the `video_session` catalogue and takes its properties from that encoder. Each
//! encoded frame is handed to a codec backend, and each packet the backend returns goes to the session's output
//! handler, with the status, the info flags and the source reference of the frame that produced it. `prepare` and
//! `complete_frames` follow the reference framework's calls of the same names. The session encodes nothing itself: the
//! codec backend is the only place that produces packets, and no backend is registered on this platform yet, so
//! `create` refuses every encoder id with `EncoderNotAvailable` until one is.

use std::cmp::Ordering;
use std::fmt;

use crate::video_session::{Catalogue, PropertyError, PropertySet, Value};

/// A presentation or decode time: a count of ticks at a timescale (ticks per second). The timescale is never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Time {
    pub value: i64,
    pub timescale: u32,
}

impl Time {
    /// A time, or None when the timescale is zero.
    pub fn new(value: i64, timescale: u32) -> Option<Self> {
        if timescale == 0 {
            None
        } else {
            Some(Time { value, timescale })
        }
    }

    /// Compares two times by the instant they name, whatever their timescales.
    pub fn compare(self, other: Time) -> Ordering {
        let left = self.value as i128 * other.timescale as i128;
        let right = other.value as i128 * self.timescale as i128;
        left.cmp(&right)
    }

    /// True when this time is the same instant as `other` or earlier.
    pub fn at_or_before(self, other: Time) -> bool {
        self.compare(other) != Ordering::Greater
    }
}

/// Per-frame options. `force_key_frame` asks the backend for a sync frame at this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameOptions {
    pub force_key_frame: bool,
}

/// One frame to encode. `source_ref` is returned unchanged with every output the frame produces, as the reference
/// framework's source frame reference does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pts: Time,
    pub duration: Time,
    pub options: FrameOptions,
    pub source_ref: u64,
}

/// One encoded packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub pts: Time,
    pub dts: Time,
    pub key_frame: bool,
    pub data: Vec<u8>,
}

/// The info flags that accompany an output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InfoFlags {
    /// The backend dropped the frame. No packet follows.
    pub frame_dropped: bool,
    /// The output came from a deferred (asynchronous) encode rather than the encode call itself.
    pub asynchronous: bool,
}

/// What a backend did with one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The frame produced these packets. An empty list means the backend is still buffering it.
    Packets(Vec<Packet>),
    /// The backend dropped the frame.
    Dropped,
}

/// One callback from a session: the status of an encode, its info flags, the packet when there is one, and the source
/// reference of the frame it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub status: Result<(), SessionError>,
    pub flags: InfoFlags,
    pub packet: Option<Packet>,
    pub source_ref: u64,
}

/// Why a session call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// A width, height, timescale or frame size is not valid for this session.
    Parameter,
    /// The session has been invalidated.
    InvalidSession,
    /// No encoder with the requested identifier is registered.
    EncoderNotAvailable,
    /// The encoder does not support the property.
    PropertyNotSupported,
    /// The property is read-only.
    PropertyReadOnly,
    /// The property value has the wrong kind, or is out of its range or list.
    PropertyValue,
    /// The backend failed to encode.
    EncodeFailed,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Parameter => write!(f, "a parameter is not valid for this session"),
            SessionError::InvalidSession => write!(f, "the session has been invalidated"),
            SessionError::EncoderNotAvailable => write!(f, "no encoder with this identifier is registered"),
            SessionError::PropertyNotSupported => write!(f, "the encoder does not support the property"),
            SessionError::PropertyReadOnly => write!(f, "the property is read-only"),
            SessionError::PropertyValue => write!(f, "the property value is not valid for the property"),
            SessionError::EncodeFailed => write!(f, "the encoder failed to encode the frame"),
        }
    }
}

impl From<PropertyError> for SessionError {
    fn from(err: PropertyError) -> Self {
        match err {
            PropertyError::Unsupported => SessionError::PropertyNotSupported,
            PropertyError::ReadOnly => SessionError::PropertyReadOnly,
            PropertyError::WrongKind | PropertyError::OutOfRange => SessionError::PropertyValue,
        }
    }
}

/// A codec backend. The session calls it once per frame and once per flush. The backend owns the codec state.
pub trait Backend {
    /// Encodes one frame. `props` holds the session's property values.
    fn encode(&mut self, frame: &Frame, props: &PropertySet) -> Result<Step, SessionError>;
    /// Returns every packet still buffered by the backend, in decode order.
    fn flush(&mut self) -> Result<Vec<Packet>, SessionError>;
}

/// The callback a session delivers its outputs to.
pub type OutputHandler = Box<dyn FnMut(Output)>;

/// A compression session on one encoder.
pub struct CompressionSession {
    width: u32,
    height: u32,
    props: PropertySet,
    backend: Box<dyn Backend>,
    handler: OutputHandler,
    prepared: bool,
    valid: bool,
}

impl CompressionSession {
    /// Opens a session on encoder `encoder_id` for frames of `width` by `height`. The encoder must be registered in
    /// `catalogue`, and the session takes its supported properties with no values set. `backend` encodes the frames
    /// and `handler` receives the outputs.
    pub fn create(
        catalogue: &Catalogue,
        encoder_id: &str,
        width: u32,
        height: u32,
        backend: Box<dyn Backend>,
        handler: OutputHandler,
    ) -> Result<Self, SessionError> {
        if width == 0 || height == 0 {
            return Err(SessionError::Parameter);
        }
        let props = catalogue.open(encoder_id).ok_or(SessionError::EncoderNotAvailable)?;
        Ok(CompressionSession { width, height, props, backend, handler, prepared: false, valid: true })
    }

    /// The width and height the session encodes.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Whether `prepare` has been called.
    pub fn is_prepared(&self) -> bool {
        self.prepared
    }

    /// Sets a property. The encoder's rules for names, kinds, ranges and read-only properties apply.
    pub fn set_property(&mut self, name: &str, value: Value) -> Result<(), SessionError> {
        self.check_valid()?;
        self.props.set_property(name, value).map_err(SessionError::from)
    }

    /// The value of a property: the one set, or the encoder's default.
    pub fn copy_property(&self, name: &str) -> Result<Value, SessionError> {
        self.check_valid()?;
        self.props.copy_property(name).ok_or(SessionError::PropertyNotSupported)
    }

    /// Hints that encoding is about to start. The session does no work for it beyond recording the call.
    pub fn prepare(&mut self) -> Result<(), SessionError> {
        self.check_valid()?;
        self.prepared = true;
        Ok(())
    }

    /// Encodes one frame. The frame must have the session's size. The handler receives one output for each packet the
    /// backend returns, or one output with `frame_dropped` set when the backend drops the frame. A backend that is still
    /// buffering the frame sends no output now.
    pub fn encode_frame(&mut self, frame: Frame) -> Result<(), SessionError> {
        self.check_valid()?;
        if frame.width != self.width || frame.height != self.height {
            return Err(SessionError::Parameter);
        }
        let source_ref = frame.source_ref;
        match self.backend.encode(&frame, &self.props) {
            Ok(Step::Packets(packets)) => {
                for packet in packets {
                    self.emit(Ok(()), InfoFlags::default(), Some(packet), source_ref);
                }
                Ok(())
            }
            Ok(Step::Dropped) => {
                self.emit(Ok(()), InfoFlags { frame_dropped: true, asynchronous: false }, None, source_ref);
                Ok(())
            }
            Err(err) => {
                self.emit(Err(err), InfoFlags::default(), None, source_ref);
                Err(err)
            }
        }
    }

    /// Delivers every buffered output whose presentation time is at or before `until`. With `None`, delivers all of
    /// them. Outputs that are still later than `until` stay in the backend.
    pub fn complete_frames(&mut self, until: Option<Time>) -> Result<(), SessionError> {
        self.check_valid()?;
        let packets = self.backend.flush()?;
        for packet in packets {
            if until.map_or(true, |t| packet.pts.at_or_before(t)) {
                self.emit(Ok(()), InfoFlags { frame_dropped: false, asynchronous: true }, Some(packet), 0);
            }
        }
        Ok(())
    }

    /// Ends the session. Every later call fails with `InvalidSession`.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    fn check_valid(&self) -> Result<(), SessionError> {
        if self.valid {
            Ok(())
        } else {
            Err(SessionError::InvalidSession)
        }
    }

    fn emit(&mut self, status: Result<(), SessionError>, flags: InfoFlags, packet: Option<Packet>, source_ref: u64) {
        (self.handler)(Output { status, flags, packet, source_ref });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_session::{EncoderInfo, Kind, Spec};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A backend that returns a scripted step for each frame, and a fixed flush list.
    struct Scripted {
        steps: Vec<Result<Step, SessionError>>,
        flushed: Vec<Packet>,
        seen_force: Rc<RefCell<Vec<bool>>>,
    }

    impl Backend for Scripted {
        fn encode(&mut self, frame: &Frame, _props: &PropertySet) -> Result<Step, SessionError> {
            self.seen_force.borrow_mut().push(frame.options.force_key_frame);
            self.steps.remove(0)
        }
        fn flush(&mut self) -> Result<Vec<Packet>, SessionError> {
            Ok(std::mem::take(&mut self.flushed))
        }
    }

    fn catalogue() -> Catalogue {
        let specs = vec![
            Spec { name: "Quality", kind: Kind::Int { min: 0, max: 100 }, default: Value::Int(50), writable: true },
            Spec { name: "EncoderID", kind: Kind::Int { min: 0, max: i64::MAX }, default: Value::Int(0), writable: false },
        ];
        let mut cat = Catalogue::new();
        let info = EncoderInfo {
            id: "enc".to_string(),
            codec: "hvc1".to_string(),
            hardware: true,
            properties: PropertySet::new(specs),
        };
        assert!(cat.register(info).is_ok());
        cat
    }

    fn packet(pts: i64, key: bool) -> Packet {
        Packet { pts: Time::new(pts, 30).unwrap(), dts: Time::new(pts, 30).unwrap(), key_frame: key, data: vec![1, 2] }
    }

    fn frame(source_ref: u64, width: u32, force: bool) -> Frame {
        Frame {
            width,
            height: 2,
            pts: Time::new(0, 30).unwrap(),
            duration: Time::new(1, 30).unwrap(),
            options: FrameOptions { force_key_frame: force },
            source_ref,
        }
    }

    type Log = Rc<RefCell<Vec<Output>>>;

    fn session(steps: Vec<Result<Step, SessionError>>, flushed: Vec<Packet>) -> (CompressionSession, Log, Rc<RefCell<Vec<bool>>>) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&log);
        let backend = Scripted { steps, flushed, seen_force: Rc::clone(&seen) };
        let s = CompressionSession::create(
            &catalogue(),
            "enc",
            4,
            2,
            Box::new(backend),
            Box::new(move |out: Output| sink.borrow_mut().push(out)),
        )
        .unwrap();
        (s, log, seen)
    }

    #[test]
    fn an_unknown_encoder_or_zero_size_is_refused() {
        let cat = catalogue();
        let none = CompressionSession::create(
            &cat,
            "missing",
            4,
            2,
            Box::new(Scripted { steps: vec![], flushed: vec![], seen_force: Rc::new(RefCell::new(vec![])) }),
            Box::new(|_: Output| {}),
        );
        assert!(matches!(none, Err(SessionError::EncoderNotAvailable)));
        let zero = CompressionSession::create(
            &cat,
            "enc",
            0,
            2,
            Box::new(Scripted { steps: vec![], flushed: vec![], seen_force: Rc::new(RefCell::new(vec![])) }),
            Box::new(|_: Output| {}),
        );
        assert!(matches!(zero, Err(SessionError::Parameter)));
    }

    #[test]
    fn times_compare_across_timescales() {
        let half = Time::new(1, 2).unwrap();
        let third = Time::new(1, 3).unwrap();
        assert_eq!(half.compare(third), Ordering::Greater);
        assert!(Time::new(2, 4).unwrap().at_or_before(half));
        assert!(!half.at_or_before(third));
        assert_eq!(Time::new(1, 0), None);
    }

    #[test]
    fn properties_follow_the_encoder_rules() {
        let (mut s, _log, _seen) = session(vec![], vec![]);
        assert_eq!(s.copy_property("Quality"), Ok(Value::Int(50)));
        assert_eq!(s.set_property("Quality", Value::Int(70)), Ok(()));
        assert_eq!(s.copy_property("Quality"), Ok(Value::Int(70)));
        assert_eq!(s.set_property("Quality", Value::Int(101)), Err(SessionError::PropertyValue));
        assert_eq!(s.set_property("EncoderID", Value::Int(1)), Err(SessionError::PropertyReadOnly));
        assert_eq!(s.set_property("Bogus", Value::Int(1)), Err(SessionError::PropertyNotSupported));
        assert_eq!(s.copy_property("Bogus"), Err(SessionError::PropertyNotSupported));
    }

    #[test]
    fn packets_reach_the_handler_with_the_source_reference() {
        let (mut s, log, _seen) =
            session(vec![Ok(Step::Packets(vec![packet(0, true), packet(1, false)]))], vec![]);
        assert_eq!(s.encode_frame(frame(42, 4, false)), Ok(()));
        let outs = log.borrow();
        assert_eq!(outs.len(), 2);
        assert!(outs.iter().all(|o| o.source_ref == 42 && o.status == Ok(()) && !o.flags.frame_dropped));
        assert_eq!(outs[0].packet.as_ref().map(|p| p.key_frame), Some(true));
    }

    #[test]
    fn a_dropped_frame_sends_one_flagged_output_and_no_packet() {
        let (mut s, log, _seen) = session(vec![Ok(Step::Dropped)], vec![]);
        assert_eq!(s.encode_frame(frame(7, 4, false)), Ok(()));
        let outs = log.borrow();
        assert_eq!(outs.len(), 1);
        assert!(outs[0].flags.frame_dropped);
        assert!(outs[0].packet.is_none());
        assert_eq!(outs[0].source_ref, 7);
    }

    #[test]
    fn a_backend_error_reaches_the_handler_and_the_caller() {
        let (mut s, log, _seen) = session(vec![Err(SessionError::EncodeFailed)], vec![]);
        assert_eq!(s.encode_frame(frame(3, 4, false)), Err(SessionError::EncodeFailed));
        assert_eq!(log.borrow()[0].status, Err(SessionError::EncodeFailed));
    }

    #[test]
    fn a_frame_of_another_size_is_refused_before_the_backend_sees_it() {
        let (mut s, log, seen) = session(vec![], vec![]);
        assert_eq!(s.encode_frame(frame(1, 8, false)), Err(SessionError::Parameter));
        assert!(seen.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn the_force_key_frame_option_reaches_the_backend() {
        let (mut s, _log, seen) = session(vec![Ok(Step::Packets(vec![])), Ok(Step::Packets(vec![]))], vec![]);
        assert_eq!(s.encode_frame(frame(1, 4, true)), Ok(()));
        assert_eq!(s.encode_frame(frame(2, 4, false)), Ok(()));
        assert_eq!(*seen.borrow(), vec![true, false]);
    }

    #[test]
    fn prepare_is_recorded_and_complete_frames_honours_the_time_limit() {
        let (mut s, log, _seen) = session(vec![], vec![packet(1, true), packet(9, false)]);
        assert!(!s.is_prepared());
        assert_eq!(s.prepare(), Ok(()));
        assert!(s.is_prepared());
        assert_eq!(s.complete_frames(Time::new(5, 30)), Ok(()));
        let outs = log.borrow();
        assert_eq!(outs.len(), 1);
        assert!(outs[0].flags.asynchronous);
        assert_eq!(outs[0].packet.as_ref().map(|p| p.pts.value), Some(1));
    }

    #[test]
    fn complete_frames_without_a_limit_delivers_everything() {
        let (mut s, log, _seen) = session(vec![], vec![packet(1, true), packet(9, false)]);
        assert_eq!(s.complete_frames(None), Ok(()));
        assert_eq!(log.borrow().len(), 2);
    }

    #[test]
    fn an_invalidated_session_refuses_every_call() {
        let (mut s, _log, _seen) = session(vec![], vec![]);
        s.invalidate();
        assert_eq!(s.prepare(), Err(SessionError::InvalidSession));
        assert_eq!(s.set_property("Quality", Value::Int(1)), Err(SessionError::InvalidSession));
        assert_eq!(s.copy_property("Quality"), Err(SessionError::InvalidSession));
        assert_eq!(s.encode_frame(frame(1, 4, false)), Err(SessionError::InvalidSession));
        assert_eq!(s.complete_frames(None), Err(SessionError::InvalidSession));
    }
}
