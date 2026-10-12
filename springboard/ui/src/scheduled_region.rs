//! Scheduled playback on a sample timeline. A span of an open file, or a block of samples held in memory, is queued to
//! start at a sample time. For each output block the schedule answers which source frames play where, and it reports
//! each item once, when its last frame has been rendered. A file region plays its span once, then repeats it
//! `loop_count` extra times. Slices hold one channel of samples. The schedule reads no file and opens no device: the
//! caller reads the frames that each piece names.

use std::fmt;

/// Why a region or a slice was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// The file has no frames, or the region plays none.
    EmptyRegion,
    /// The region starts before the file or runs past its end.
    OutOfFile,
    /// The slice has no samples.
    EmptySlice,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::EmptyRegion => write!(f, "the region plays no frames"),
            ScheduleError::OutOfFile => write!(f, "the region is outside the file"),
            ScheduleError::EmptySlice => write!(f, "the slice has no samples"),
        }
    }
}

/// A span of an open file, played from `start_time` on the sample timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRegion {
    /// The caller's handle for the open file.
    pub file: u32,
    /// The first frame of the file that plays.
    pub start_frame: i64,
    /// How many frames of the span play before it repeats.
    pub frames_to_play: u32,
    /// How many extra times the span repeats. 0 plays it once.
    pub loop_count: u32,
    /// The sample time at which the span starts.
    pub start_time: i64,
}

impl FileRegion {
    /// A region of `frames_to_play` frames from `start_frame`, in a file of `file_frames` frames. The whole span must lie
    /// inside the file.
    pub fn new(
        file: u32,
        file_frames: i64,
        start_frame: i64,
        frames_to_play: u32,
        loop_count: u32,
        start_time: i64,
    ) -> Result<Self, ScheduleError> {
        if file_frames <= 0 || frames_to_play == 0 {
            return Err(ScheduleError::EmptyRegion);
        }
        if start_frame < 0 || start_frame > file_frames - frames_to_play as i64 {
            return Err(ScheduleError::OutOfFile);
        }
        Ok(FileRegion { file, start_frame, frames_to_play, loop_count, start_time })
    }

    /// The sample time just after the region's last frame.
    fn end(&self) -> i64 {
        let span = (self.frames_to_play as i64).saturating_mul(self.loop_count as i64 + 1);
        self.start_time.saturating_add(span)
    }
}

/// A block of one channel of samples held in memory, played from `start_time`.
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub start_time: i64,
    pub samples: Vec<f32>,
}

impl Slice {
    /// A slice of `samples` that starts at `start_time`. It must hold at least one sample.
    pub fn new(start_time: i64, samples: Vec<f32>) -> Result<Self, ScheduleError> {
        if samples.is_empty() {
            return Err(ScheduleError::EmptySlice);
        }
        Ok(Slice { start_time, samples })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Item {
    Region(FileRegion),
    Slice(Slice),
}

impl Item {
    fn start(&self) -> i64 {
        match self {
            Item::Region(r) => r.start_time,
            Item::Slice(s) => s.start_time,
        }
    }

    fn end(&self) -> i64 {
        match self {
            Item::Region(r) => r.end(),
            Item::Slice(s) => s.start_time.saturating_add(s.samples.len() as i64),
        }
    }
}

/// Where the frames of a piece come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Frames of an open file, starting at `frame`.
    File { file: u32, frame: i64 },
    /// Samples of a slice, starting at index `index`.
    Slice { index: usize },
}

/// A run of output frames that one item supplies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    /// The id the item was scheduled under.
    pub id: u32,
    pub source: Source,
    /// Output frames from the start of the rendered block.
    pub offset: usize,
    /// How many output frames the piece covers.
    pub frames: usize,
}

/// Queued items on a sample timeline.
#[derive(Debug, Default)]
pub struct Schedule {
    items: Vec<(u32, Item)>,
    last_id: u32,
    completed: Vec<u32>,
}

impl Schedule {
    pub fn new() -> Self {
        Schedule::default()
    }

    /// Queues a file region and returns its id.
    pub fn schedule_region(&mut self, region: FileRegion) -> u32 {
        self.push(Item::Region(region))
    }

    /// Queues a slice and returns its id.
    pub fn schedule_slice(&mut self, slice: Slice) -> u32 {
        self.push(Item::Slice(slice))
    }

    fn push(&mut self, item: Item) -> u32 {
        self.last_id += 1;
        self.items.push((self.last_id, item));
        self.last_id
    }

    /// Removes an item that has not finished. Returns false when the id is unknown or the item has finished.
    pub fn cancel(&mut self, id: u32) -> bool {
        let before = self.items.len();
        self.items.retain(|(i, _)| *i != id);
        self.items.len() != before
    }

    /// The number of items still queued.
    pub fn pending(&self) -> usize {
        self.items.len()
    }

    /// The pieces of the output span `[from, from + frames)`, in scheduling order. An item whose last frame falls at or
    /// before the end of the span finishes and is reported by [`Schedule::take_completed`].
    pub fn render(&mut self, from: i64, frames: usize) -> Vec<Piece> {
        let end = from.saturating_add(frames as i64);
        let mut pieces = Vec::new();
        for (id, item) in &self.items {
            let start = item.start();
            let lo = start.max(from);
            let hi = item.end().min(end);
            if hi <= lo {
                continue;
            }
            match item {
                Item::Region(r) => {
                    let period = r.frames_to_play as i64;
                    let mut t = lo;
                    while t < hi {
                        let within = (t - start) % period;
                        let run = (period - within).min(hi - t);
                        pieces.push(Piece {
                            id: *id,
                            source: Source::File { file: r.file, frame: r.start_frame + within },
                            offset: (t - from) as usize,
                            frames: run as usize,
                        });
                        t += run;
                    }
                }
                Item::Slice(_) => pieces.push(Piece {
                    id: *id,
                    source: Source::Slice { index: (lo - start) as usize },
                    offset: (lo - from) as usize,
                    frames: (hi - lo) as usize,
                }),
            }
        }
        let finished = &mut self.completed;
        self.items.retain(|(id, item)| {
            let done = item.end() <= end;
            if done {
                finished.push(*id);
            }
            !done
        });
        pieces
    }

    /// The ids of the items that finished since the last call, in the order they finished.
    pub fn take_completed(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(id: u32, source: Source, offset: usize, frames: usize) -> Piece {
        Piece { id, source, offset, frames }
    }

    #[test]
    fn a_region_must_be_non_empty_and_inside_its_file() {
        assert_eq!(FileRegion::new(1, 1_000, 0, 0, 0, 0), Err(ScheduleError::EmptyRegion));
        assert_eq!(FileRegion::new(1, 0, 0, 10, 0, 0), Err(ScheduleError::EmptyRegion));
        assert_eq!(FileRegion::new(1, 1_000, 900, 200, 0, 0), Err(ScheduleError::OutOfFile));
        assert_eq!(FileRegion::new(1, 1_000, -1, 10, 0, 0), Err(ScheduleError::OutOfFile));
        assert!(FileRegion::new(1, 1_000, 800, 200, 0, 0).is_ok());
    }

    #[test]
    fn a_region_plays_its_frames_at_its_start_time_and_then_finishes() {
        let mut s = Schedule::new();
        let region = FileRegion::new(1, 1_000, 10, 50, 0, 100).unwrap();
        let id = s.schedule_region(region);
        assert_eq!(id, 1);
        let pieces = s.render(0, 200);
        assert_eq!(pieces, vec![piece(1, Source::File { file: 1, frame: 10 }, 100, 50)]);
        assert_eq!(s.take_completed(), vec![1]);
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn a_looped_region_repeats_its_span_loop_count_extra_times() {
        let mut s = Schedule::new();
        s.schedule_region(FileRegion::new(2, 1_000, 10, 4, 2, 0).unwrap());
        let pieces = s.render(0, 12);
        let offsets: Vec<usize> = pieces.iter().map(|p| p.offset).collect();
        assert_eq!(offsets, vec![0, 4, 8]);
        for p in &pieces {
            assert_eq!(p.frames, 4);
            assert_eq!(p.source, Source::File { file: 2, frame: 10 });
        }
        assert_eq!(s.take_completed().len(), 1);
    }

    #[test]
    fn a_block_that_crosses_a_loop_edge_is_split_at_the_edge() {
        let mut s = Schedule::new();
        s.schedule_region(FileRegion::new(2, 1_000, 10, 4, 1, 0).unwrap());
        // The span is 8 frames long. Output frames 2 to 5 read frames 12, 13 of the file, then 10, 11.
        let pieces = s.render(2, 4);
        assert_eq!(
            pieces,
            vec![
                piece(1, Source::File { file: 2, frame: 12 }, 0, 2),
                piece(1, Source::File { file: 2, frame: 10 }, 2, 2),
            ]
        );
        assert!(s.take_completed().is_empty());
    }

    #[test]
    fn a_slice_supplies_its_samples_from_the_index_that_is_due() {
        let mut s = Schedule::new();
        assert_eq!(Slice::new(0, Vec::new()), Err(ScheduleError::EmptySlice));
        let slice = Slice::new(50, vec![0.1, 0.2, 0.3, 0.4]).unwrap();
        let id = s.schedule_slice(slice);
        // Output frames 48 to 57. The slice covers 50 to 53, so it starts at output offset 2.
        let pieces = s.render(48, 10);
        assert_eq!(pieces, vec![piece(id, Source::Slice { index: 0 }, 2, 4)]);
        assert_eq!(s.take_completed(), vec![id]);
    }

    #[test]
    fn a_region_renders_continuously_across_blocks() {
        let mut s = Schedule::new();
        s.schedule_region(FileRegion::new(3, 1_000, 0, 10, 0, 0).unwrap());
        assert_eq!(s.render(0, 4), vec![piece(1, Source::File { file: 3, frame: 0 }, 0, 4)]);
        assert!(s.take_completed().is_empty());
        assert_eq!(s.render(4, 10), vec![piece(1, Source::File { file: 3, frame: 4 }, 0, 6)]);
        assert_eq!(s.take_completed(), vec![1]);
    }

    #[test]
    fn a_future_item_is_not_rendered_and_can_be_cancelled_once() {
        let mut s = Schedule::new();
        let id = s.schedule_region(FileRegion::new(4, 1_000, 0, 10, 0, 500).unwrap());
        assert!(s.render(0, 100).is_empty());
        assert_eq!(s.pending(), 1);
        assert!(s.cancel(id));
        assert!(!s.cancel(id));
        assert_eq!(s.pending(), 0);
    }
}
