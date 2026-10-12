//! Representative frame selection for `KeyFrameAnalysisRequest`: `representative` picks the frame whose luma
//! histogram is closest to every other frame (the medoid), and `histogram` builds those histograms from raw luma.
//! Pure data, no I/O, no model. The caller decodes the frames and supplies one histogram per frame.
//!
//! * A histogram is normalised to parts per million, so frames with different pixel counts compare fairly.
//! * The distance between two frames is half the sum of the absolute bin differences: 0 for equal frames and
//!   1_000_000 for frames with no bin in common.
//! * The chosen frame has the smallest sum of distances to all frames. Ties go to the earlier frame.
//! * Frames must all have the same number of bins. Otherwise there is no answer.

/// One decoded frame: its presentation time and its luma histogram.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub time_ms: f64,
    pub histogram: Vec<u32>,
}

/// The chosen representative frame: its index in the input and its time, for seeking or a thumbnail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyFrame {
    pub index: usize,
    pub time_ms: f64,
}

const PPM: u64 = 1_000_000;

/// The luma histogram of `luma` (0..=255) with `bins` equal-width bins. Zero bins gives an empty histogram.
pub fn histogram(luma: &[u8], bins: usize) -> Vec<u32> {
    if bins == 0 {
        return Vec::new();
    }
    let mut out = vec![0u32; bins];
    for &v in luma {
        // v <= 255, so v * bins / 256 is always below bins
        let b = v as usize * bins / 256;
        out[b] += 1;
    }
    out
}

/// The representative frame of `frames`, or `None` when there are no frames or the bin counts differ.
pub fn representative(frames: &[Frame]) -> Option<KeyFrame> {
    let first = frames.first()?;
    let bins = first.histogram.len();
    if frames.iter().any(|f| f.histogram.len() != bins) {
        return None;
    }
    let props: Vec<Vec<u64>> = frames.iter().map(|f| proportions(&f.histogram)).collect();
    let mut best: Option<(u64, usize)> = None;
    for i in 0..frames.len() {
        let total: u64 = (0..frames.len()).map(|j| distance(&props[i], &props[j])).sum();
        best = match best {
            Some((b, _)) if total >= b => best,
            _ => Some((total, i)),
        };
    }
    best.map(|(_, i)| KeyFrame { index: i, time_ms: frames[i].time_ms })
}

/// The bins of `h` as parts per million of its total. An empty frame has all bins at 0.
fn proportions(h: &[u32]) -> Vec<u64> {
    let total: u64 = h.iter().map(|&c| c as u64).sum();
    if total == 0 {
        return vec![0; h.len()];
    }
    h.iter().map(|&c| c as u64 * PPM / total).collect()
}

/// Half the sum of the absolute bin differences. Both inputs have the same length.
fn distance(a: &[u64], b: &[u64]) -> u64 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).sum::<u64>() / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(time_ms: f64, histogram: &[u32]) -> Frame {
        Frame { time_ms, histogram: histogram.to_vec() }
    }

    #[test]
    fn histogram_counts_luma_into_equal_bins() {
        // two bins: 0 and 127 fall in bin 0, 128 and 255 in bin 1
        assert_eq!(histogram(&[0, 0, 255, 255], 2), vec![2, 2]);
        assert_eq!(histogram(&[0, 127, 128, 255], 2), vec![2, 2]);
        // four bins of 64: 0 -> 0, 64 -> 1, 128 -> 2, 255 -> 3
        assert_eq!(histogram(&[0, 64, 128, 255], 4), vec![1, 1, 1, 1]);
        assert_eq!(histogram(&[1, 2, 3], 0), Vec::<u32>::new());
    }

    #[test]
    fn representative_is_the_medoid_of_the_frames() {
        // A = [10, 0], B = [9, 1], C = [0, 10]. Proportions in ppm: A [1e6, 0], B [9e5, 1e5], C [0, 1e6].
        // Distances: AB = 1e5, AC = 1e6, BC = 9e5. Sums: A 1.1e6, B 1.0e6, C 1.9e6. B wins.
        let frames = [frame(0.0, &[10, 0]), frame(500.0, &[9, 1]), frame(1000.0, &[0, 10])];
        assert_eq!(representative(&frames), Some(KeyFrame { index: 1, time_ms: 500.0 }));
    }

    #[test]
    fn identical_frames_tie_to_the_earlier_one() {
        // [1, 1] and [2, 2] have the same proportions, so every distance is 0 and index 0 wins
        let frames = [frame(0.0, &[1, 1]), frame(10.0, &[2, 2])];
        assert_eq!(representative(&frames), Some(KeyFrame { index: 0, time_ms: 0.0 }));
    }

    #[test]
    fn a_single_frame_is_its_own_representative() {
        let frames = [frame(42.0, &[5, 5, 5])];
        assert_eq!(representative(&frames), Some(KeyFrame { index: 0, time_ms: 42.0 }));
    }

    #[test]
    fn no_frames_or_mixed_bin_counts_give_none() {
        assert_eq!(representative(&[]), None);
        let mixed = [frame(0.0, &[1, 2]), frame(1.0, &[1, 2, 3])];
        assert_eq!(representative(&mixed), None);
    }

    #[test]
    fn frames_with_no_pixels_do_not_panic() {
        // an empty frame has all bins at 0, so it is 500_000 away from a [1, 1] frame
        let frames = [frame(0.0, &[0, 0]), frame(1.0, &[1, 1]), frame(2.0, &[1, 1])];
        // distances: empty-to-full = 500_000 each, full-to-full = 0.
        // sums: index 0 = 1_000_000, index 1 = 500_000, index 2 = 500_000. Index 1 wins as the earlier tie.
        assert_eq!(representative(&frames), Some(KeyFrame { index: 1, time_ms: 1.0 }));
    }
}
