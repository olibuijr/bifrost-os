//! A composed timeline: media from source tracks spliced into one timeline, with gaps, removals and time scaling.
//! This is the pure model behind composite assets. Times are integer ticks of the track's time scale, so every
//! edit is exact; nothing here decodes media.

/// A span of ticks on a timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    pub start: i64,
    pub duration: i64,
}

impl TimeRange {
    pub fn new(start: i64, duration: i64) -> Self {
        Self { start, duration }
    }
    pub fn end(self) -> i64 {
        self.start + self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositionError {
    /// A track with this id already exists.
    DuplicateTrack(u32),
    /// No track has this id.
    UnknownTrack(u32),
    /// The position or range lies outside the timeline, or a duration is not positive.
    OutOfRange,
}

/// Where a segment's media comes from. `start` and `duration` are ticks of the source track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub track: u32,
    pub start: i64,
    pub duration: i64,
}

/// One piece of a composed track: media from a source, or an empty gap when `source` is `None`.
/// `range` is on the composed timeline, so a scaled segment can be longer or shorter than its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub range: TimeRange,
    pub source: Option<Source>,
}

/// Splits `total` in proportion `part / whole`, rounding to the nearest tick.
fn share(total: i64, part: i64, whole: i64) -> i64 {
    ((total as i128 * part as i128 + whole as i128 / 2) / whole as i128) as i64
}

/// A track of a composition: segments that tile the timeline from 0 with no holes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionTrack {
    pub id: u32,
    segments: Vec<Segment>,
}

impl CompositionTrack {
    pub fn new(id: u32) -> Self {
        Self { id, segments: Vec::new() }
    }
    pub fn duration(&self) -> i64 {
        self.segments.last().map_or(0, |s| s.range.end())
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    /// The segment covering a timeline position (`start <= t < end`).
    pub fn segment_at(&self, t: i64) -> Option<&Segment> {
        self.segments.iter().find(|s| s.range.start <= t && t < s.range.end())
    }
    /// Splices `range` of `source_track` in at `at`. Later media moves back by the inserted duration.
    pub fn insert(&mut self, source_track: u32, range: TimeRange, at: i64) -> Result<(), CompositionError> {
        if range.duration <= 0 || at < 0 || at > self.duration() {
            return Err(CompositionError::OutOfRange);
        }
        let source = Source { track: source_track, start: range.start, duration: range.duration };
        self.insert_segment(at, range.duration, Some(source));
        Ok(())
    }
    /// Adds an empty gap of `duration` ticks at `at`. Later media moves back by the same amount.
    pub fn insert_empty(&mut self, at: i64, duration: i64) -> Result<(), CompositionError> {
        if duration <= 0 || at < 0 || at > self.duration() {
            return Err(CompositionError::OutOfRange);
        }
        self.insert_segment(at, duration, None);
        Ok(())
    }
    /// Removes a span and closes the gap: media after it moves back.
    pub fn remove(&mut self, range: TimeRange) -> Result<(), CompositionError> {
        self.check_range(range)?;
        self.split_at(range.start);
        self.split_at(range.end());
        self.segments.retain(|s| !(s.range.start >= range.start && s.range.end() <= range.end()));
        for s in &mut self.segments {
            if s.range.start >= range.end() {
                s.range.start -= range.duration;
            }
        }
        Ok(())
    }
    /// Stretches or squeezes a span to `to_duration` ticks. Later media moves by the difference. Source
    /// durations are kept, so a scaled segment plays its source faster or slower.
    pub fn scale(&mut self, range: TimeRange, to_duration: i64) -> Result<(), CompositionError> {
        self.check_range(range)?;
        if to_duration <= 0 {
            return Err(CompositionError::OutOfRange);
        }
        self.split_at(range.start);
        self.split_at(range.end());
        let mut out: Vec<Segment> = self.segments.iter().filter(|s| s.range.end() <= range.start).copied().collect();
        let mut prev = range.start;
        for s in self.segments.iter().filter(|s| s.range.start >= range.start && s.range.end() <= range.end()) {
            // Boundaries are scaled from the span start, so the pieces add up to `to_duration` exactly.
            let boundary = range.start + share(s.range.end() - range.start, to_duration, range.duration);
            if boundary > prev {
                out.push(Segment { range: TimeRange::new(prev, boundary - prev), source: s.source });
                prev = boundary;
            }
        }
        let delta = to_duration - range.duration;
        for s in self.segments.iter().filter(|s| s.range.start >= range.end()) {
            out.push(Segment { range: TimeRange::new(s.range.start + delta, s.range.duration), source: s.source });
        }
        self.segments = out;
        Ok(())
    }
    fn check_range(&self, range: TimeRange) -> Result<(), CompositionError> {
        if range.duration <= 0 || range.start < 0 || range.end() > self.duration() {
            return Err(CompositionError::OutOfRange);
        }
        Ok(())
    }
    /// Makes `t` a segment boundary. A source segment is cut in proportion to its own duration.
    fn split_at(&mut self, t: i64) {
        let Some(i) = self.segments.iter().position(|s| s.range.start < t && t < s.range.end()) else {
            return;
        };
        let s = self.segments[i];
        let left = t - s.range.start;
        let right = s.range.duration - left;
        let (left_src, right_src) = match s.source {
            Some(src) => {
                let cut = share(src.duration, left, s.range.duration);
                (
                    Some(Source { duration: cut, ..src }),
                    Some(Source { start: src.start + cut, duration: src.duration - cut, ..src }),
                )
            }
            None => (None, None),
        };
        self.segments[i] = Segment { range: TimeRange::new(s.range.start, left), source: left_src };
        self.segments.insert(i + 1, Segment { range: TimeRange::new(t, right), source: right_src });
    }
    /// Inserts a piece at `at` (already validated) and moves everything after it by its duration.
    fn insert_segment(&mut self, at: i64, duration: i64, source: Option<Source>) {
        self.split_at(at);
        let index = self.segments.iter().position(|s| s.range.start >= at).unwrap_or(self.segments.len());
        for s in &mut self.segments[index..] {
            s.range.start += duration;
        }
        self.segments.insert(index, Segment { range: TimeRange::new(at, duration), source });
    }
}

/// Several tracks edited together: a span removed or a gap inserted applies to every track, as one edit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    tracks: Vec<CompositionTrack>,
}

impl Composition {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn tracks(&self) -> &[CompositionTrack] {
        &self.tracks
    }
    /// The longest track.
    pub fn duration(&self) -> i64 {
        self.tracks.iter().map(CompositionTrack::duration).max().unwrap_or(0)
    }
    pub fn add_track(&mut self, id: u32) -> Result<&mut CompositionTrack, CompositionError> {
        if self.track(id).is_some() {
            return Err(CompositionError::DuplicateTrack(id));
        }
        self.tracks.push(CompositionTrack::new(id));
        Ok(self.tracks.last_mut().expect("a track was just pushed"))
    }
    pub fn track(&self, id: u32) -> Option<&CompositionTrack> {
        self.tracks.iter().find(|t| t.id == id)
    }
    pub fn track_mut(&mut self, id: u32) -> Option<&mut CompositionTrack> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
    /// Removes a span from every track. Nothing changes unless the span fits in all of them.
    pub fn remove_time_range(&mut self, range: TimeRange) -> Result<(), CompositionError> {
        if self.tracks.iter().any(|t| t.check_range(range).is_err()) {
            return Err(CompositionError::OutOfRange);
        }
        for t in &mut self.tracks {
            t.remove(range)?;
        }
        Ok(())
    }
    /// Adds an empty gap at `at` to every track. Nothing changes unless `at` lies within all of them.
    pub fn insert_empty_time_range(&mut self, at: i64, duration: i64) -> Result<(), CompositionError> {
        if self.tracks.iter().any(|t| at < 0 || at > t.duration()) || duration <= 0 {
            return Err(CompositionError::OutOfRange);
        }
        for t in &mut self.tracks {
            t.insert_empty(at, duration)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (start, duration, source track and start) for each segment, to compare a whole timeline at once.
    fn spans(t: &CompositionTrack) -> Vec<(i64, i64, Option<(u32, i64)>)> {
        t.segments()
            .iter()
            .map(|s| (s.range.start, s.range.duration, s.source.map(|x| (x.track, x.start))))
            .collect()
    }

    #[test]
    fn insert_splits_the_timeline_and_moves_later_media_back() {
        let mut t = CompositionTrack::new(1);
        t.insert(10, TimeRange::new(0, 100), 0).unwrap();
        t.insert(11, TimeRange::new(500, 50), 40).unwrap();
        assert_eq!(t.duration(), 150);
        assert_eq!(
            spans(&t),
            vec![(0, 40, Some((10, 0))), (40, 50, Some((11, 500))), (90, 60, Some((10, 40)))]
        );
    }

    #[test]
    fn remove_closes_the_gap_and_keeps_source_positions() {
        let mut t = CompositionTrack::new(1);
        t.insert(10, TimeRange::new(0, 100), 0).unwrap();
        t.insert(11, TimeRange::new(500, 50), 40).unwrap();
        t.remove(TimeRange::new(30, 80)).unwrap();
        assert_eq!(t.duration(), 70);
        assert_eq!(spans(&t), vec![(0, 30, Some((10, 0))), (30, 40, Some((10, 60)))]);
    }

    #[test]
    fn empty_gaps_are_segments_with_no_source() {
        let mut t = CompositionTrack::new(5);
        t.insert(7, TimeRange::new(0, 100), 0).unwrap();
        t.insert_empty(50, 20).unwrap();
        assert_eq!(t.duration(), 120);
        assert_eq!(t.segment_at(60).unwrap().source, None);
        assert_eq!(t.segment_at(70).unwrap().source.unwrap().start, 50);
        assert!(t.segment_at(120).is_none(), "the timeline ends at its duration");
    }

    #[test]
    fn scale_changes_composed_length_but_keeps_the_source_duration() {
        let mut t = CompositionTrack::new(1);
        t.insert(7, TimeRange::new(0, 100), 0).unwrap();
        t.insert(8, TimeRange::new(0, 100), 100).unwrap();
        t.scale(TimeRange::new(0, 100), 150).unwrap();
        assert_eq!(t.duration(), 250);
        let first = t.segment_at(0).unwrap();
        assert_eq!((first.range.duration, first.source.unwrap().duration), (150, 100));
        assert_eq!(t.segment_at(150).unwrap().source.unwrap().track, 8, "later media moved by the difference");
    }

    #[test]
    fn out_of_range_and_non_positive_edits_are_refused_and_change_nothing() {
        let mut t = CompositionTrack::new(1);
        t.insert(10, TimeRange::new(0, 100), 0).unwrap();
        let before = t.clone();
        assert_eq!(t.insert(10, TimeRange::new(0, 10), 101), Err(CompositionError::OutOfRange));
        assert_eq!(t.insert(10, TimeRange::new(0, 0), 0), Err(CompositionError::OutOfRange));
        assert_eq!(t.remove(TimeRange::new(50, 100)), Err(CompositionError::OutOfRange));
        assert_eq!(t.scale(TimeRange::new(0, 100), 0), Err(CompositionError::OutOfRange));
        assert_eq!(t, before);
    }

    #[test]
    fn composition_edits_apply_to_every_track_together() {
        let mut c = Composition::new();
        c.add_track(1).unwrap().insert(10, TimeRange::new(0, 100), 0).unwrap();
        c.add_track(2).unwrap().insert(20, TimeRange::new(0, 60), 0).unwrap();
        assert_eq!(c.add_track(1), Err(CompositionError::DuplicateTrack(1)));
        c.insert_empty_time_range(50, 10).unwrap();
        assert_eq!((c.track(1).unwrap().duration(), c.track(2).unwrap().duration()), (110, 70));
        c.remove_time_range(TimeRange::new(0, 10)).unwrap();
        assert_eq!((c.track(1).unwrap().duration(), c.track(2).unwrap().duration()), (100, 60));
        assert_eq!(c.remove_time_range(TimeRange::new(0, 80)), Err(CompositionError::OutOfRange), "track 2 is too short");
        assert_eq!(c.duration(), 100, "a refused edit leaves every track as it was");
    }
}
