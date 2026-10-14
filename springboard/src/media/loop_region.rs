//! Loop regions for sampled playback: the loop modes an app asks for and the read head of a player that runs through
//! them. Positions are frame indices. A region is `start..end` (end exclusive) inside the sample; the head moves by a
//! frame count per render block and reports when a one-shot sound has played out. Pure arithmetic, so the same head
//! moves the same way on every machine.

/// How playback treats the end of a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    /// No loop: the head plays to the end of the sample and stops.
    Off,
    /// The head jumps from the end of the region back to its start.
    Forward,
    /// The head runs to the end of the region, then runs back to its start, and so on. The end frames are repeated
    /// once at each turn, so the bounce does not lose a frame.
    PingPong,
}

/// The read head of a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    /// Reading frame `position`; `backward` is true while a ping-pong loop runs in reverse.
    Playing { position: u64, backward: bool },
    /// The sound has played out.
    Done,
}

/// A loop region with its mode. The end is exclusive and always after the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRegion {
    start: u64,
    end: u64,
    mode: LoopMode,
}

impl LoopRegion {
    /// A region from `start` up to, not including, `end`. Returns `None` for an empty or inverted region.
    pub fn new(start: u64, end: u64, mode: LoopMode) -> Option<Self> {
        (end > start).then_some(LoopRegion { start, end, mode })
    }

    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn end(&self) -> u64 {
        self.end
    }

    pub fn mode(&self) -> LoopMode {
        self.mode
    }

    /// The head at the first frame of the sample.
    pub fn head_at_start(&self) -> Head {
        Head::Playing { position: 0, backward: false }
    }

    /// Moves `head` forward by `frames` in a sample of `total` frames. A region that runs past the sample is cut at
    /// `total`. With `LoopMode::Off` the region is ignored and the head stops at `total`.
    pub fn advance(&self, head: Head, frames: u64, total: u64) -> Head {
        let Head::Playing { position, backward } = head else {
            return Head::Done;
        };
        match self.mode {
            LoopMode::Off => {
                let next = position.saturating_add(frames);
                if next >= total {
                    Head::Done
                } else {
                    Head::Playing { position: next, backward: false }
                }
            }
            LoopMode::Forward => {
                let (start, end) = self.clamped(total);
                let len = end - start;
                if position < start || len == 0 {
                    return Head::Playing { position: position.min(total.saturating_sub(1)), backward: false };
                }
                let offset = (position - start + frames % len) % len;
                Head::Playing { position: start + offset, backward: false }
            }
            LoopMode::PingPong => {
                let (start, end) = self.clamped(total);
                let len = end - start;
                if position < start || position >= end || len == 0 {
                    return Head::Playing { position: position.min(total.saturating_sub(1)), backward };
                }
                let period = 2 * len;
                let phase = if backward { period - 1 - (position - start) } else { position - start };
                let next = (phase + frames % period) % period;
                if next < len {
                    Head::Playing { position: start + next, backward: false }
                } else {
                    Head::Playing { position: start + (period - 1 - next), backward: true }
                }
            }
        }
    }

    /// The region cut to the sample length, so a loop never reads past the last frame.
    fn clamped(&self, total: u64) -> (u64, u64) {
        (self.start.min(total), self.end.min(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_inverted_region_is_refused() {
        assert!(LoopRegion::new(10, 10, LoopMode::Forward).is_none());
        assert!(LoopRegion::new(10, 4, LoopMode::Forward).is_none());
        assert!(LoopRegion::new(4, 10, LoopMode::Forward).is_some());
    }

    #[test]
    fn a_one_shot_plays_to_the_end_and_then_reports_done() {
        let r = LoopRegion::new(2, 6, LoopMode::Off).unwrap();
        let head = r.advance(r.head_at_start(), 5, 8);
        assert_eq!(head, Head::Playing { position: 5, backward: false });
        assert_eq!(r.advance(head, 3, 8), Head::Done);
        assert_eq!(r.advance(Head::Done, 1, 8), Head::Done, "a finished head stays finished");
    }

    #[test]
    fn a_forward_loop_wraps_from_the_region_end_to_its_start() {
        let r = LoopRegion::new(2, 6, LoopMode::Forward).unwrap();
        let head = Head::Playing { position: 4, backward: false };
        assert_eq!(r.advance(head, 3, 100), Head::Playing { position: 3, backward: false }, "4+3 = 7, wrapped by 4");
    }

    #[test]
    fn a_forward_loop_handles_a_jump_longer_than_the_region() {
        let r = LoopRegion::new(10, 14, LoopMode::Forward).unwrap();
        let head = Head::Playing { position: 10, backward: false };
        assert_eq!(r.advance(head, 4 * 1000 + 2, 100), Head::Playing { position: 12, backward: false });
    }

    #[test]
    fn a_ping_pong_loop_turns_at_both_ends_and_repeats_the_end_frames_once() {
        let r = LoopRegion::new(0, 4, LoopMode::PingPong).unwrap();
        let mut head = r.head_at_start();
        let mut seen = Vec::new();
        for _ in 0..8 {
            head = r.advance(head, 1, 100);
            if let Head::Playing { position, backward } = head {
                seen.push((position, backward));
            }
        }
        assert_eq!(
            seen,
            vec![(1, false), (2, false), (3, false), (3, true), (2, true), (1, true), (0, true), (0, false)]
        );
    }

    #[test]
    fn a_ping_pong_loop_keeps_its_direction_across_blocks() {
        let r = LoopRegion::new(0, 10, LoopMode::PingPong).unwrap();
        let head = Head::Playing { position: 9, backward: false };
        assert_eq!(r.advance(head, 3, 100), Head::Playing { position: 7, backward: true }, "three frames past the end, the head runs back from 7");
        assert_eq!(
            r.advance(Head::Playing { position: 9, backward: true }, 2, 100),
            Head::Playing { position: 7, backward: true }
        );
    }

    #[test]
    fn a_region_past_the_sample_end_is_cut_at_the_sample_length() {
        let r = LoopRegion::new(2, 50, LoopMode::Forward).unwrap();
        let head = Head::Playing { position: 4, backward: false };
        assert_eq!(r.advance(head, 10, 8), Head::Playing { position: 2 + (4 - 2 + 10) % 6, backward: false });
    }
}
