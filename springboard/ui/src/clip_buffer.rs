//! Rolling clip buffer (`ClipBuffer`): keeps the newest encoded frames of a stream for a fixed span and exports the
//! last part of that span as a clip. A clip starts on a keyframe so it decodes without the frames before it, which
//! means it can begin a little earlier than the duration asked for. Times are seconds as `f64`. Nothing here encodes
//! or writes files: the owner pushes frames that are already encoded and writes the frames that `export` returns.
use std::collections::VecDeque;

/// One encoded frame. `keyframe` marks a frame that decodes on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame<T> {
    pub time: f64,
    pub keyframe: bool,
    pub data: T,
}

/// Why a clip could not be exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    /// The duration is not a positive, finite number of seconds.
    InvalidDuration,
    /// The buffer holds no frames.
    Empty,
    /// The buffer holds no keyframe, so no clip can start.
    NoKeyframe,
}

/// A buffer that keeps about `span` seconds of frames while it is started.
#[derive(Clone, Debug)]
pub struct ClipBuffer<T> {
    span: f64,
    buffering: bool,
    frames: VecDeque<Frame<T>>,
}

impl<T> ClipBuffer<T> {
    /// A stopped, empty buffer that keeps `span` seconds. `None` when the span is not a positive, finite number.
    pub fn new(span: f64) -> Option<Self> {
        if span.is_finite() && span > 0.0 {
            Some(ClipBuffer { span, buffering: false, frames: VecDeque::new() })
        } else {
            None
        }
    }

    /// The length of time the buffer tries to keep, in seconds.
    pub fn span(&self) -> f64 {
        self.span
    }

    pub fn is_buffering(&self) -> bool {
        self.buffering
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Times of the oldest and the newest frame kept, or `None` when the buffer is empty.
    pub fn bounds(&self) -> Option<(f64, f64)> {
        let first = self.frames.front()?.time;
        let last = self.frames.back()?.time;
        Some((first, last))
    }

    /// Seconds from the oldest kept frame to the newest one. Zero when there are fewer than two frames.
    pub fn kept_duration(&self) -> f64 {
        match self.bounds() {
            Some((first, last)) => last - first,
            None => 0.0,
        }
    }

    /// Starts taking frames. Frames kept from an earlier run stay in place.
    pub fn start(&mut self) {
        self.buffering = true;
    }

    /// Stops taking frames. The frames already kept can still be exported until `clear`.
    pub fn stop(&mut self) {
        self.buffering = false;
    }

    /// Drops every kept frame.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Adds a frame while buffering. A frame is refused (returns `false`) when buffering is off, when its time is not
    /// finite, or when its time is not after the newest kept frame. After each accepted frame, frames older than the
    /// span are dropped, but only up to the newest keyframe that is at or before the span's start, so the oldest kept
    /// frame is always a keyframe. With no such keyframe, nothing is dropped yet.
    pub fn push(&mut self, time: f64, keyframe: bool, data: T) -> bool {
        if !self.buffering || !time.is_finite() {
            return false;
        }
        if let Some(last) = self.frames.back() {
            if time <= last.time {
                return false;
            }
        }
        self.frames.push_back(Frame { time, keyframe, data });
        self.evict();
        true
    }

    fn evict(&mut self) {
        let newest = match self.frames.back() {
            Some(frame) => frame.time,
            None => return,
        };
        let cutoff = newest - self.span;
        let mut cut = None;
        for (i, frame) in self.frames.iter().enumerate() {
            if frame.time > cutoff {
                break;
            }
            if frame.keyframe {
                cut = Some(i);
            }
        }
        if let Some(i) = cut {
            for _ in 0..i {
                self.frames.pop_front();
            }
        }
    }

    /// The clip for the last `duration` seconds, oldest frame first. A duration longer than the span is cut to the
    /// span. The clip starts at the newest keyframe at or before the requested start; when there is none, it starts at
    /// the first keyframe kept. So the clip can be longer than asked for, never shorter.
    pub fn export(&self, duration: f64) -> Result<Vec<&Frame<T>>, ClipError> {
        if !(duration.is_finite() && duration > 0.0) {
            return Err(ClipError::InvalidDuration);
        }
        let newest = match self.frames.back() {
            Some(frame) => frame.time,
            None => return Err(ClipError::Empty),
        };
        let wanted = duration.min(self.span);
        let from = newest - wanted;
        let mut start = None;
        for (i, frame) in self.frames.iter().enumerate() {
            if frame.keyframe && frame.time <= from {
                start = Some(i);
            }
        }
        if start.is_none() {
            start = self.frames.iter().position(|frame| frame.keyframe);
        }
        let start = start.ok_or(ClipError::NoKeyframe)?;
        Ok(self.frames.iter().skip(start).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pushes frames every half second from 0.0 to `last`; every whole second is a keyframe.
    fn filled(span: f64, last_half_steps: usize) -> ClipBuffer<usize> {
        let mut buf = ClipBuffer::new(span).expect("valid span");
        buf.start();
        for i in 0..=last_half_steps {
            let t = i as f64 * 0.5;
            buf.push(t, i % 2 == 0, i);
        }
        buf
    }

    #[test]
    fn new_rejects_a_span_that_is_not_positive_and_finite() {
        assert!(ClipBuffer::<u8>::new(0.0).is_none());
        assert!(ClipBuffer::<u8>::new(-1.0).is_none());
        assert!(ClipBuffer::<u8>::new(f64::NAN).is_none());
        assert!(ClipBuffer::<u8>::new(f64::INFINITY).is_none());
        assert_eq!(ClipBuffer::<u8>::new(30.0).map(|b| b.span()), Some(30.0));
    }

    #[test]
    fn frames_are_refused_while_stopped_and_out_of_order() {
        let mut buf: ClipBuffer<u8> = ClipBuffer::new(10.0).unwrap();
        assert!(!buf.push(0.0, true, 0), "a stopped buffer takes no frames");
        buf.start();
        assert!(buf.push(0.0, true, 0));
        assert!(!buf.push(0.0, false, 1), "a repeated time is refused");
        assert!(!buf.push(-0.5, false, 2), "an earlier time is refused");
        assert!(!buf.push(f64::NAN, false, 3), "a time that is not finite is refused");
        assert_eq!(buf.len(), 1);
        buf.stop();
        assert!(!buf.push(1.0, true, 4));
    }

    #[test]
    fn eviction_drops_old_frames_but_keeps_the_oldest_keyframe_first() {
        // Span 2 s. Newest is 5.0, so the cutoff is 3.0 and the newest keyframe at or before it is 3.0.
        let buf = filled(2.0, 10);
        assert_eq!(buf.bounds(), Some((3.0, 5.0)));
        assert_eq!(buf.len(), 5);
        let first_is_key = buf.export(0.5).map(|clip| clip[0].keyframe);
        assert_eq!(first_is_key, Ok(true));
    }

    #[test]
    fn export_starts_at_the_keyframe_before_the_requested_start() {
        let buf = filled(2.0, 10);
        // One second back from 5.0 is 4.0, a keyframe, so the clip is 4.0 to 5.0.
        let clip = buf.export(1.0).unwrap();
        let times: Vec<f64> = clip.iter().map(|f| f.time).collect();
        assert_eq!(times, vec![4.0, 4.5, 5.0]);
        // 1.2 s back is 3.8, and the newest keyframe before it is 3.0, so the clip is longer than asked.
        let longer = buf.export(1.2).unwrap();
        assert_eq!(longer.first().map(|f| f.time), Some(3.0));
    }

    #[test]
    fn export_cuts_a_duration_longer_than_the_span_to_the_span() {
        let buf = filled(2.0, 10);
        let clip = buf.export(60.0).unwrap();
        assert_eq!(clip.first().map(|f| f.time), Some(3.0));
        assert_eq!(clip.len(), 5);
    }

    #[test]
    fn export_reports_bad_durations_empty_buffers_and_missing_keyframes() {
        let empty: ClipBuffer<u8> = ClipBuffer::new(5.0).unwrap();
        assert_eq!(empty.export(1.0).map(|c| c.len()), Err(ClipError::Empty));
        let buf = filled(5.0, 4);
        assert_eq!(buf.export(0.0).map(|c| c.len()), Err(ClipError::InvalidDuration));
        assert_eq!(buf.export(f64::NAN).map(|c| c.len()), Err(ClipError::InvalidDuration));

        let mut no_keys: ClipBuffer<u8> = ClipBuffer::new(5.0).unwrap();
        no_keys.start();
        no_keys.push(0.0, false, 0);
        no_keys.push(0.5, false, 1);
        assert_eq!(no_keys.export(1.0).map(|c| c.len()), Err(ClipError::NoKeyframe));
    }

    #[test]
    fn stop_keeps_frames_for_export_and_clear_drops_them() {
        let mut buf = filled(5.0, 4);
        buf.stop();
        assert!(!buf.is_buffering());
        assert!(buf.export(1.0).is_ok(), "frames stay exportable after stop");
        assert!((buf.kept_duration() - 2.0).abs() < 1e-9);
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.kept_duration(), 0.0);
    }
}
