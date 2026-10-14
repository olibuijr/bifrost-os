//! Lazy lists and grids (`LazyVStack`, `LazyVGrid`) as windowing arithmetic: given the row count, the row height, the spacing
//! and the scroll offset, which rows can be inside the viewport. The caller builds views only for those rows and places
//! them at their offsets, so a list of thousands of rows costs what its visible part costs.
//!
//! Rows are laid out top to bottom with `spacing` between neighbours. Row `k` starts at `k * (row_h + spacing)`. The
//! range returned may include one row past each visible edge, because a row that touches the edge can still be drawn into
//! it. `overscan` adds that many more rows on each side, for scrolling without a blank frame.

use std::ops::Range;

/// The height of `count` rows of `row_h` with `spacing` between them. No rows give no height.
pub fn content_height(count: usize, row_h: f32, spacing: f32) -> f32 {
    if count == 0 {
        0.0
    } else {
        count as f32 * row_h + (count - 1) as f32 * spacing.max(0.0)
    }
}

/// The rows to build for a viewport `viewport` points tall, scrolled `offset` points down. The range is clamped to
/// `0..count`. A row height that is not positive gives no rows.
pub fn visible_rows(count: usize, row_h: f32, spacing: f32, viewport: f32, offset: f32, overscan: usize) -> Range<usize> {
    if count == 0 || !(row_h > 0.0) {
        return 0..0;
    }
    let pitch = row_h + spacing.max(0.0);
    let top = offset.max(0.0);
    let bottom = top + viewport.max(0.0);
    let first = (top / pitch).floor() as usize;
    let last = (bottom / pitch).floor() as usize;
    let start = first.saturating_sub(overscan).min(count);
    let end = last.saturating_add(1).saturating_add(overscan).min(count);
    start..end.max(start)
}

/// The items to build for a grid with `columns` items per row. Items fill each row left to right, so item `i` is in row
/// `i / columns`. The range is over item indexes, and a grid with zero columns is treated as one column.
pub fn visible_items(
    count: usize,
    columns: usize,
    row_h: f32,
    spacing: f32,
    viewport: f32,
    offset: f32,
    overscan: usize,
) -> Range<usize> {
    let columns = columns.max(1);
    let rows = count.div_ceil(columns);
    let row_range = visible_rows(rows, row_h, spacing, viewport, offset, overscan);
    let start = (row_range.start * columns).min(count);
    let end = (row_range.end * columns).min(count);
    start..end.max(start)
}
