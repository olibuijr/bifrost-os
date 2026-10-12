//! Compute dispatch geometry and a CPU reference run: how a grid of threads splits into threadgroups, and a serial
//! walk of the threads in the order a GPU starts them. An encoder sizes its dispatches with the grid math; the walk is
//! the reference that a test or a software fallback checks results against. Nothing here touches a GPU.
//!
//! Rules: each axis of a threadgroup is at least 1, and the threads in one threadgroup never exceed the device limit
//! `max_threads`. A grid of threads is rounded up to whole threadgroups on each axis. Threads past the grid edge are
//! not run. Threads run x fastest, then y, then z, and threadgroups run in the same order.

/// Three counts: threads, threadgroups or pixels along x, y and z.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub width: usize,
    pub height: usize,
    pub depth: usize,
}

impl Size {
    pub const fn new(width: usize, height: usize, depth: usize) -> Self {
        Size { width, height, depth }
    }

    /// Product of the three axes. Saturates instead of overflowing.
    pub fn count(&self) -> usize {
        self.width.saturating_mul(self.height).saturating_mul(self.depth)
    }
}

/// Why a dispatch cannot run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchError {
    /// The grid or the threadgroup has a zero axis.
    ZeroAxis,
    /// The threads in one threadgroup are above the device limit.
    TooManyThreads { threads: usize, max: usize },
}

/// Checks a threadgroup shape against the device limit.
pub fn validate_threadgroup(per_group: Size, max_threads: usize) -> Result<(), DispatchError> {
    if per_group.width == 0 || per_group.height == 0 || per_group.depth == 0 {
        return Err(DispatchError::ZeroAxis);
    }
    let threads = per_group.count();
    if threads > max_threads {
        return Err(DispatchError::TooManyThreads { threads, max: max_threads });
    }
    Ok(())
}

/// Threadgroups needed to cover `grid` threads with groups of `per_group`: each axis rounded up.
pub fn threadgroups_for(grid: Size, per_group: Size) -> Result<Size, DispatchError> {
    if grid.width == 0 || grid.height == 0 || grid.depth == 0 {
        return Err(DispatchError::ZeroAxis);
    }
    if per_group.width == 0 || per_group.height == 0 || per_group.depth == 0 {
        return Err(DispatchError::ZeroAxis);
    }
    Ok(Size::new(
        ceil_div(grid.width, per_group.width),
        ceil_div(grid.height, per_group.height),
        ceil_div(grid.depth, per_group.depth),
    ))
}

/// Ceiling division of a count by a non-zero divisor.
fn ceil_div(count: usize, divisor: usize) -> usize {
    (count + divisor - 1) / divisor
}

/// Runs `kernel` once for every thread position in `grid`, group by group, and returns how many threads ran. The
/// threads past the grid edge in a partial group are skipped, so the count equals `grid.count()`.
pub fn run_threads<F: FnMut(Size)>(
    grid: Size,
    per_group: Size,
    max_threads: usize,
    mut kernel: F,
) -> Result<usize, DispatchError> {
    validate_threadgroup(per_group, max_threads)?;
    let groups = threadgroups_for(grid, per_group)?;
    let mut ran = 0;
    for gz in 0..groups.depth {
        for gy in 0..groups.height {
            for gx in 0..groups.width {
                for lz in 0..per_group.depth {
                    for ly in 0..per_group.height {
                        for lx in 0..per_group.width {
                            let x = gx * per_group.width + lx;
                            let y = gy * per_group.height + ly;
                            let z = gz * per_group.depth + lz;
                            if x < grid.width && y < grid.height && z < grid.depth {
                                kernel(Size::new(x, y, z));
                                ran += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(ran)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_round_each_axis_up() {
        let g = threadgroups_for(Size::new(10, 5, 1), Size::new(4, 4, 1)).unwrap();
        assert_eq!(g, Size::new(3, 2, 1));
        let exact = threadgroups_for(Size::new(8, 8, 2), Size::new(4, 4, 2)).unwrap();
        assert_eq!(exact, Size::new(2, 2, 1));
    }

    #[test]
    fn threadgroup_is_limited_by_the_device_thread_count() {
        assert_eq!(validate_threadgroup(Size::new(32, 32, 1), 1024), Ok(()));
        assert_eq!(
            validate_threadgroup(Size::new(33, 32, 1), 1024),
            Err(DispatchError::TooManyThreads { threads: 1056, max: 1024 })
        );
        assert_eq!(validate_threadgroup(Size::new(0, 1, 1), 1024), Err(DispatchError::ZeroAxis));
    }

    #[test]
    fn one_dimensional_walk_covers_the_grid_in_order() {
        let mut seen = Vec::new();
        let ran = run_threads(Size::new(10, 1, 1), Size::new(4, 1, 1), 1024, |p| seen.push(p.width)).unwrap();
        assert_eq!(ran, 10);
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn partial_groups_skip_threads_past_the_edge() {
        // Grid 3x2 in groups of 2x2: two groups along x. The second group keeps only x = 2.
        let mut seen = Vec::new();
        let ran = run_threads(Size::new(3, 2, 1), Size::new(2, 2, 1), 1024, |p| seen.push((p.width, p.height)))
            .unwrap();
        assert_eq!(ran, 6);
        assert_eq!(seen, vec![(0, 0), (1, 0), (0, 1), (1, 1), (2, 0), (2, 1)]);
    }

    #[test]
    fn walk_runs_exactly_the_grid_count() {
        let mut calls = 0usize;
        let ran = run_threads(Size::new(7, 3, 2), Size::new(4, 2, 1), 1024, |_| calls += 1).unwrap();
        assert_eq!(ran, 42);
        assert_eq!(calls, 42);
    }

    #[test]
    fn zero_grid_is_refused_before_any_thread_runs() {
        let mut calls = 0usize;
        let r = run_threads(Size::new(0, 4, 1), Size::new(4, 4, 1), 1024, |_| calls += 1);
        assert_eq!(r, Err(DispatchError::ZeroAxis));
        assert_eq!(calls, 0);
    }
}
