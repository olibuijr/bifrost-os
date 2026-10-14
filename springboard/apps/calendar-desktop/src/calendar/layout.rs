//! Side-by-side layout of overlapping events in a Day/Week column, plus the
//! pure geometry for the current-time line and drag-to-create/move.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// Column index inside the overlap cluster, from 0.
    pub col: usize,
    /// Number of columns in the cluster; the event width is 1 / cols.
    pub cols: usize,
}

/// Lays out `(start, end)` minute intervals. An event shorter than
/// `min_len` is treated as `min_len` long so tiny events stay tappable and do
/// not hide behind their neighbours. Intervals that merely touch do not
/// overlap. The result is parallel to the input.
pub fn layout(events: &[(u32, u32)], min_len: u32) -> Vec<Slot> {
    let span = |i: usize| {
        let (s, e) = events[i];
        (s, e.max(s + min_len))
    };
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| {
        let (s, e) = span(i);
        (s, std::cmp::Reverse(e), i)
    });
    let mut out = vec![Slot { col: 0, cols: 1 }; events.len()];
    let mut cluster: Vec<usize> = vec![];
    let mut cluster_end = 0;
    // end of the last event placed in each column of the current cluster
    let mut col_ends: Vec<u32> = vec![];
    let flush = |cluster: &mut Vec<usize>, col_ends: &mut Vec<u32>, out: &mut Vec<Slot>| {
        for &i in cluster.iter() {
            out[i].cols = col_ends.len().max(1);
        }
        cluster.clear();
        col_ends.clear();
    };
    for i in order {
        let (s, e) = span(i);
        if !cluster.is_empty() && s >= cluster_end {
            flush(&mut cluster, &mut col_ends, &mut out);
        }
        let col = match col_ends.iter().position(|&end| end <= s) {
            Some(c) => c,
            None => {
                col_ends.push(0);
                col_ends.len() - 1
            }
        };
        col_ends[col] = e;
        out[i].col = col;
        cluster_end = if cluster.is_empty() { e } else { cluster_end.max(e) };
        cluster.push(i);
    }
    flush(&mut cluster, &mut col_ends, &mut out);
    out
}

/// The minutes a timed event fills in a Day or Week column: its travel time before it starts, then its length. The
/// block starts no earlier than midnight and ends no later than midnight.
pub fn travel_span(start: u32, duration: u32, travel: u32) -> (u32, u32) {
    (start.saturating_sub(travel), (start + duration).min(1440))
}

/// Pixel box `(x, width)` of a slot inside a column starting at `x0`.
pub fn slot_x(slot: Slot, x0: f32, width: f32, gap: f32) -> (f32, f32) {
    let w = width / slot.cols as f32;
    (x0 + w * slot.col as f32, (w - gap).max(1.0))
}

/// Vertical position of minute `minute` in an hour grid.
pub fn minute_y(minute: f32, top: f32, hour_h: f32) -> f32 {
    top + minute / 60.0 * hour_h
}

/// Minute of day under pixel `y`, snapped down to `snap` minutes and clamped
/// to the day.
pub fn y_minute(y: f32, top: f32, hour_h: f32, snap: u32) -> u32 {
    let m = ((y - top) / hour_h * 60.0).floor().max(0.0) as u32;
    (m / snap.max(1) * snap.max(1)).min(1440 - snap.max(1))
}

/// Y of the current-time indicator, or None when `minute` is off-screen.
pub fn now_line_y(minute: u32, top: f32, hour_h: f32) -> Option<f32> {
    (minute < 1440).then(|| minute_y(minute as f32, top, hour_h))
}

/// Result of dragging in a Day/Week column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragSpan {
    pub start: u32,
    pub end: u32,
}

/// Dragging on empty space creates an event from the press point to the
/// current point (either direction), at least `snap` minutes long.
pub fn drag_create(press_min: u32, now_min: u32, snap: u32) -> DragSpan {
    let snap = snap.max(1);
    let a = press_min / snap * snap;
    let b = now_min / snap * snap;
    let (start, end) = if a <= b { (a, b + snap) } else { (b, a + snap) };
    DragSpan { start, end: end.min(1440) }
}

/// Moving keeps the duration: `grab_offset` is the minute offset between the
/// event start and the point that was grabbed.
pub fn drag_move(duration: u32, grab_offset: u32, now_min: u32, snap: u32) -> DragSpan {
    let snap = snap.max(1);
    let duration = duration.min(1440);
    let raw = now_min.saturating_sub(grab_offset);
    let start = (raw / snap * snap).min(1440 - duration);
    DragSpan { start, end: start + duration }
}

/// Day offset (-n, 0, +n) for a horizontal drag across week columns.
pub fn column_at(x: f32, left: f32, col_w: f32, cols: usize) -> usize {
    (((x - left) / col_w).floor().max(0.0) as usize).min(cols.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(col: usize, cols: usize) -> Slot {
        Slot { col, cols }
    }

    #[test]
    fn disjoint_events_use_full_width() {
        let l = layout(&[(60, 120), (120, 180), (300, 360)], 15);
        assert_eq!(l, vec![s(0, 1), s(0, 1), s(0, 1)]);
    }

    #[test]
    fn two_overlapping_split_the_width() {
        let l = layout(&[(60, 120), (90, 150)], 15);
        assert_eq!(l, vec![s(0, 2), s(1, 2)]);
    }

    #[test]
    fn column_is_reused_when_free_but_cluster_width_stays() {
        // A 0-180 overlaps B 30-60 and C 90-120; B and C share column 1.
        let l = layout(&[(0, 180), (30, 60), (90, 120)], 15);
        assert_eq!(l, vec![s(0, 2), s(1, 2), s(1, 2)]);
    }

    #[test]
    fn three_way_overlap() {
        let l = layout(&[(0, 100), (10, 100), (20, 100)], 15);
        assert_eq!(l, vec![s(0, 3), s(1, 3), s(2, 3)]);
    }

    #[test]
    fn separate_clusters_are_independent() {
        let l = layout(&[(0, 60), (30, 90), (200, 260)], 15);
        assert_eq!(l, vec![s(0, 2), s(1, 2), s(0, 1)]);
    }

    #[test]
    fn short_events_get_minimum_length() {
        // 60-65 is padded to 60-75, so it collides with 70-100.
        let l = layout(&[(60, 65), (70, 100)], 15);
        assert_eq!(l, vec![s(0, 2), s(1, 2)]);
        let l = layout(&[(60, 65), (75, 100)], 15);
        assert_eq!(l, vec![s(0, 1), s(0, 1)]);
    }

    #[test]
    fn input_order_does_not_matter() {
        let l = layout(&[(90, 150), (60, 120)], 15);
        assert_eq!(l, vec![s(1, 2), s(0, 2)]);
        assert!(layout(&[], 15).is_empty());
    }

    #[test]
    fn slot_geometry() {
        assert_eq!(slot_x(s(1, 2), 100.0, 200.0, 2.0), (200.0, 98.0));
        assert_eq!(slot_x(s(0, 1), 10.0, 50.0, 0.0), (10.0, 50.0));
    }

    #[test]
    fn minute_and_y_round_trip() {
        assert_eq!(minute_y(90.0, 100.0, 60.0), 190.0);
        assert_eq!(y_minute(190.0, 100.0, 60.0, 15), 90);
        assert_eq!(y_minute(193.0, 100.0, 60.0, 15), 90);
        assert_eq!(y_minute(-50.0, 100.0, 60.0, 15), 0);
        assert_eq!(y_minute(99999.0, 100.0, 60.0, 15), 1425);
    }

    #[test]
    fn now_line() {
        assert_eq!(now_line_y(720, 0.0, 50.0), Some(600.0));
        assert_eq!(now_line_y(1440, 0.0, 50.0), None);
    }

    #[test]
    fn drag_create_both_directions() {
        assert_eq!(drag_create(540, 600, 15), DragSpan { start: 540, end: 615 });
        assert_eq!(drag_create(600, 540, 15), DragSpan { start: 540, end: 615 });
        assert_eq!(drag_create(545, 545, 15), DragSpan { start: 540, end: 555 });
        assert_eq!(drag_create(1430, 1439, 15), DragSpan { start: 1425, end: 1440 });
    }

    #[test]
    fn drag_move_keeps_duration_and_clamps() {
        assert_eq!(drag_move(60, 20, 600, 15), DragSpan { start: 580 / 15 * 15, end: 580 / 15 * 15 + 60 });
        assert_eq!(drag_move(60, 0, 1439, 15), DragSpan { start: 1380, end: 1440 });
        assert_eq!(drag_move(60, 30, 10, 15), DragSpan { start: 0, end: 60 });
    }

    #[test]
    fn column_hit() {
        assert_eq!(column_at(5.0, 10.0, 100.0, 7), 0);
        assert_eq!(column_at(215.0, 10.0, 100.0, 7), 2);
        assert_eq!(column_at(9999.0, 10.0, 100.0, 7), 6);
    }
}
