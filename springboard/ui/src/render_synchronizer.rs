//! Render synchronizer model: one timeline that several queued sample renderers follow. Mirrors
//! `AVSampleBufferRenderSynchronizer`: `setRate(_:time:atHostTime:)` ties a media time to a host time and sets the
//! rate, `currentTime()` reads the media time at a host time, `rate` reports the rate, and renderers are added and
//! removed by identity. Boundary times are reported as the timeline passes them, the way boundary time observers fire.
//!
//! Pure arithmetic. The host clock is passed in by the caller in nanoseconds. Media times are in microseconds.
//!
//! * The timeline is a line: at host time `h` the media time is `anchor_media + (h - anchor_host) * rate`, where the
//!   anchor is the last `set_rate`. Changing the rate re-anchors, so the media time does not jump.
//! * A rate of zero holds the media time. A negative rate runs the timeline backwards.
//! * A boundary counts when the timeline reaches it. The end point is included and the start point is not: forward
//!   from `a` to `b` a boundary `t` counts when `a < t <= b`, backward when `b <= t < a`.
//! * The `timebase` of the official API is modelled by the anchor pair, not as a separate object.
//! * Renderers are named by a caller-chosen id. Ids are unique inside one synchronizer.

/// The reasons a synchronizer call is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncError {
    /// The rate is not a finite number.
    InvalidRate,
    /// A renderer with this id is already attached.
    DuplicateRenderer,
    /// No renderer with this id is attached.
    UnknownRenderer,
}

/// A timeline that renderers are attached to. The default has rate 0 at media time 0 and no renderers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderSynchronizer {
    rate: f64,
    anchor_media_us: i64,
    anchor_host_ns: i64,
    renderers: Vec<String>,
}

impl RenderSynchronizer {
    /// The current playback rate. Zero when paused.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Sets the rate, and says that media time `time_us` is at host time `at_host_ns`. The new rate applies from that
    /// point on. A non-finite rate is refused.
    pub fn set_rate(&mut self, rate: f64, time_us: i64, at_host_ns: i64) -> Result<(), SyncError> {
        if !rate.is_finite() {
            return Err(SyncError::InvalidRate);
        }
        self.rate = rate;
        self.anchor_media_us = time_us;
        self.anchor_host_ns = at_host_ns;
        Ok(())
    }

    /// The media time at host time `host_ns`, in microseconds. Host times before the anchor give earlier media times.
    pub fn current_time_us(&self, host_ns: i64) -> i64 {
        let elapsed_ns = host_ns.saturating_sub(self.anchor_host_ns) as f64;
        self.anchor_media_us + (elapsed_ns * self.rate / 1000.0).round() as i64
    }

    /// Attaches a renderer. Its id must not be attached already.
    pub fn add_renderer(&mut self, id: &str) -> Result<(), SyncError> {
        if self.renderers.iter().any(|r| r.as_str() == id) {
            return Err(SyncError::DuplicateRenderer);
        }
        self.renderers.push(id.to_string());
        Ok(())
    }

    /// Detaches a renderer.
    pub fn remove_renderer(&mut self, id: &str) -> Result<(), SyncError> {
        let index = self
            .renderers
            .iter()
            .position(|r| r.as_str() == id)
            .ok_or(SyncError::UnknownRenderer)?;
        self.renderers.remove(index);
        Ok(())
    }

    /// The attached renderer ids, in the order they were added.
    pub fn renderers(&self) -> &[String] {
        &self.renderers
    }

    /// The boundary times, from `boundaries_us`, that the timeline passes between host times `from_host_ns` and
    /// `to_host_ns`. Forward movement returns them ascending, backward movement descending. The rules for the end
    /// points are in the module notes.
    pub fn boundary_crossings(&self, from_host_ns: i64, to_host_ns: i64, boundaries_us: &[i64]) -> Vec<i64> {
        let start = self.current_time_us(from_host_ns);
        let end = self.current_time_us(to_host_ns);
        let mut crossed: Vec<i64> = if end > start {
            boundaries_us.iter().copied().filter(|&t| t > start && t <= end).collect()
        } else if end < start {
            boundaries_us.iter().copied().filter(|&t| t >= end && t < start).collect()
        } else {
            Vec::new()
        };
        if end >= start {
            crossed.sort_unstable();
        } else {
            crossed.sort_unstable_by(|a, b| b.cmp(a));
        }
        crossed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC_NS: i64 = 1_000_000_000;

    #[test]
    fn media_time_follows_the_rate_from_the_anchor() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(1.0, 0, 0).unwrap();
        assert_eq!(s.current_time_us(2 * SEC_NS), 2_000_000);
        s.set_rate(2.0, 0, 0).unwrap();
        assert_eq!(s.current_time_us(SEC_NS), 2_000_000);
    }

    #[test]
    fn a_zero_rate_holds_the_media_time() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(0.0, 5_000_000, SEC_NS).unwrap();
        assert_eq!(s.current_time_us(9 * SEC_NS), 5_000_000);
        assert_eq!(s.rate(), 0.0);
    }

    #[test]
    fn changing_the_rate_re_anchors_without_a_jump() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(1.0, 0, 0).unwrap();
        assert_eq!(s.current_time_us(3 * SEC_NS), 3_000_000);
        // At host 3 s the time is 3 s. From there, half speed for two seconds gives 4 s at host 5 s.
        s.set_rate(0.5, 3_000_000, 3 * SEC_NS).unwrap();
        assert_eq!(s.current_time_us(5 * SEC_NS), 4_000_000);
    }

    #[test]
    fn a_negative_rate_runs_backwards() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(-1.0, 10_000_000, 0).unwrap();
        assert_eq!(s.current_time_us(4 * SEC_NS), 6_000_000);
    }

    #[test]
    fn a_non_finite_rate_is_refused_and_leaves_the_timeline_alone() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(1.0, 0, 0).unwrap();
        assert_eq!(s.set_rate(f64::NAN, 0, 0), Err(SyncError::InvalidRate));
        assert_eq!(s.set_rate(f64::INFINITY, 0, 0), Err(SyncError::InvalidRate));
        assert_eq!(s.rate(), 1.0);
    }

    #[test]
    fn renderers_are_unique_by_id_and_removed_by_id() {
        let mut s = RenderSynchronizer::default();
        s.add_renderer("video").unwrap();
        s.add_renderer("audio").unwrap();
        assert_eq!(s.add_renderer("video"), Err(SyncError::DuplicateRenderer));
        assert_eq!(s.renderers(), &["video".to_string(), "audio".to_string()]);
        assert_eq!(s.remove_renderer("captions"), Err(SyncError::UnknownRenderer));
        s.remove_renderer("video").unwrap();
        assert_eq!(s.renderers(), &["audio".to_string()]);
    }

    #[test]
    fn forward_movement_reports_boundaries_in_the_half_open_range() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(1.0, 0, 0).unwrap();
        // Media 0 to 3 s. The boundary at 0 is excluded (start), the one at 3 s is included (end).
        let bounds = [4_000_000, 3_000_000, 2_500_000, 1_000_000, 0];
        assert_eq!(s.boundary_crossings(0, 3 * SEC_NS, &bounds), vec![1_000_000, 2_500_000, 3_000_000]);
    }

    #[test]
    fn backward_movement_reports_boundaries_in_descending_order() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(-1.0, 3_000_000, 0).unwrap();
        // Media 3 s down to 1 s. The boundary at 3 s is excluded (start), the one at 1 s is included (end).
        let bounds = [4_000_000, 3_000_000, 2_500_000, 1_000_000];
        assert_eq!(s.boundary_crossings(0, 2 * SEC_NS, &bounds), vec![2_500_000, 1_000_000]);
    }

    #[test]
    fn a_paused_timeline_crosses_nothing() {
        let mut s = RenderSynchronizer::default();
        s.set_rate(0.0, 2_000_000, 0).unwrap();
        assert!(s.boundary_crossings(0, 5 * SEC_NS, &[2_000_000, 3_000_000]).is_empty());
    }
}
