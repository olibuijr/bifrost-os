//! Display timing (Core Video's `CVDisplayLink`, `CVTimeStamp` and `CVDisplayLinkGetActualOutputVideoRefreshPeriod`).
//!
//! A [`DisplayLink`] turns the app's ticks into one output time per refresh of a display. It holds no clock and no
//! hardware: the app passes the host time in nanoseconds at each tick (from its `next_wake` loop), and the link
//! reports the refresh that has begun since the last report. Refresh periods are whole nanoseconds computed from an
//! integer rate in hertz, so 100 Hz is 10_000_000 ns and 60 Hz is 16_666_666 ns. [`TimeStamp`] carries the fields
//! of the Core Video time stamp; its video time is in nanoseconds ([`VIDEO_TIME_SCALE`]). Refreshes that passed with no
//! tick are counted by [`DisplayLink::missed_refreshes`].

/// Units per second of [`TimeStamp::video_time`].
pub const VIDEO_TIME_SCALE: i32 = 1_000_000_000;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// One output time of a display link (`CVTimeStamp`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeStamp {
    /// Index of the refresh, counted from the link's start.
    pub frame_number: u64,
    /// Time of the refresh in host nanoseconds.
    pub host_time: u64,
    /// Time of the refresh since the link started, in units of `video_time_scale`.
    pub video_time: i64,
    /// Units per second of `video_time`.
    pub video_time_scale: i32,
    /// Playback rate relative to nominal. The link always runs at nominal rate, so this is 1.0.
    pub rate_scalar: f64,
    /// The refresh period in units of `video_time_scale`.
    pub video_refresh_period: i64,
}

/// A display link for one display at a fixed refresh rate (`CVDisplayLink`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayLink {
    period_ns: u64,
    /// Host time of refresh 0 while running.
    started_at: Option<u64>,
    /// Index of the last refresh reported.
    last_frame: Option<u64>,
    missed: u64,
}

impl DisplayLink {
    /// A link for a display at `refresh_hz` hertz. None for a rate of zero.
    pub fn new(refresh_hz: u32) -> Option<Self> {
        if refresh_hz == 0 {
            return None;
        }
        Some(DisplayLink {
            period_ns: NANOS_PER_SECOND / u64::from(refresh_hz),
            started_at: None,
            last_frame: None,
            missed: 0,
        })
    }

    /// The refresh period in nanoseconds (`CVDisplayLinkGetActualOutputVideoRefreshPeriod`).
    pub fn refresh_period(&self) -> u64 {
        self.period_ns
    }

    /// True between `start` and `stop`.
    pub fn is_running(&self) -> bool {
        self.started_at.is_some()
    }

    /// Starts the link: refresh 0 is at `host_time`. Clears the reports and the missed count.
    pub fn start(&mut self, host_time: u64) {
        self.started_at = Some(host_time);
        self.last_frame = None;
        self.missed = 0;
    }

    /// Stops the link. `tick` reports nothing until the next `start`.
    pub fn stop(&mut self) {
        self.started_at = None;
        self.last_frame = None;
    }

    /// Refreshes that passed with no tick since `start`. A refresh is missed when no tick reported it.
    pub fn missed_refreshes(&self) -> u64 {
        self.missed
    }

    /// Reports the newest refresh at or before `host_time`, once per refresh. None when the link is stopped, or when
    /// no new refresh has begun since the last report. Times before the start count as refresh 0.
    pub fn tick(&mut self, host_time: u64) -> Option<TimeStamp> {
        let start = self.started_at?;
        let frame = host_time.saturating_sub(start) / self.period_ns;
        let next = self.last_frame.map_or(0, |p| p + 1);
        if frame < next {
            return None;
        }
        self.missed += frame - next;
        self.last_frame = Some(frame);
        let offset = frame.saturating_mul(self.period_ns);
        Some(TimeStamp {
            frame_number: frame,
            host_time: start.saturating_add(offset),
            video_time: i64::try_from(offset).unwrap_or(i64::MAX),
            video_time_scale: VIDEO_TIME_SCALE,
            rate_scalar: 1.0,
            video_refresh_period: i64::try_from(self.period_ns).unwrap_or(i64::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_period_is_whole_nanoseconds_from_the_rate() {
        assert_eq!(DisplayLink::new(0), None);
        assert_eq!(DisplayLink::new(100).unwrap().refresh_period(), 10_000_000);
        assert_eq!(DisplayLink::new(60).unwrap().refresh_period(), 16_666_666);
    }

    #[test]
    fn a_stopped_link_reports_nothing() {
        let mut link = DisplayLink::new(100).unwrap();
        assert!(!link.is_running());
        assert_eq!(link.tick(50_000_000), None);
    }

    #[test]
    fn one_report_per_refresh() {
        let mut link = DisplayLink::new(100).unwrap();
        link.start(0);
        let first = link.tick(0).unwrap();
        assert_eq!(first.frame_number, 0);
        assert_eq!(first.host_time, 0);
        assert_eq!(first.video_time, 0);
        // Still inside refresh 0: nothing new.
        assert_eq!(link.tick(5_000_000), None);
        let second = link.tick(10_000_000).unwrap();
        assert_eq!(second.frame_number, 1);
        assert_eq!(second.host_time, 10_000_000);
        assert_eq!(second.video_time, 10_000_000);
        assert_eq!(second.video_time_scale, VIDEO_TIME_SCALE);
        assert_eq!(second.video_refresh_period, 10_000_000);
        assert_eq!(second.rate_scalar, 1.0);
        assert_eq!(link.missed_refreshes(), 0);
    }

    #[test]
    fn skipped_refreshes_are_counted_as_missed() {
        let mut link = DisplayLink::new(100).unwrap();
        link.start(0);
        link.tick(0).unwrap();
        // 35 ms is refresh 3; refreshes 1 and 2 had no tick.
        let late = link.tick(35_000_000).unwrap();
        assert_eq!(late.frame_number, 3);
        assert_eq!(late.host_time, 30_000_000);
        assert_eq!(link.missed_refreshes(), 2);
    }

    #[test]
    fn start_offset_and_restart_reset_the_count() {
        let mut link = DisplayLink::new(100).unwrap();
        link.start(500);
        // Refresh 1 is at 500 + 10_000_000 ns.
        let ts = link.tick(10_000_500).unwrap();
        assert_eq!(ts.frame_number, 1);
        assert_eq!(ts.host_time, 10_000_500);
        assert_eq!(ts.video_time, 10_000_000);
        link.stop();
        assert_eq!(link.tick(20_000_000), None);
        link.start(1_000_000_000);
        let again = link.tick(1_000_000_000).unwrap();
        assert_eq!(again.frame_number, 0);
        assert_eq!(link.missed_refreshes(), 0);
    }

    #[test]
    fn a_tick_before_the_start_is_refresh_zero() {
        let mut link = DisplayLink::new(100).unwrap();
        link.start(1000);
        let ts = link.tick(0).unwrap();
        assert_eq!(ts.frame_number, 0);
        assert_eq!(ts.host_time, 1000);
    }
}
