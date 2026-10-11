//! Tables mapped onto grid: the table is a grid container with one auto
//! track per column, every cell a grid item placed by row, column and span.
//! Row elements get no box; their backgrounds are painted from the union of
//! their cells (see `paint.rs`).

use super::super::dom::{Dom, NodeId};
use super::super::style::{self as cs, StyleTree};
use super::tree::{Builder, FChild, Kind, F_CELL, NONE};
use super::Ctx;
use taffy as tf;

fn span_attr(dom: &Dom, n: NodeId, name: &str) -> u16 {
    match dom.attr(n, name).and_then(|v| v.trim().parse::<u32>().ok()) {
        Some(0) | None => 1,
        Some(v) => v.min(1000) as u16,
    }
}

fn is_display(styles: &StyleTree, n: NodeId, d: cs::Display) -> bool {
    styles.get(n).map(|s| s.display == d).unwrap_or(false)
}

fn cells_of(dom: &Dom, styles: &StyleTree, row: NodeId) -> Vec<NodeId> {
    dom.children(row)
        .filter(|c| is_display(styles, *c, cs::Display::TableCell))
        .collect()
}

/// Builds the cell boxes of a table and records its column count in `aux`.
pub fn build_table(b: &mut Builder, idx: u32, n: NodeId) {
    let dom = b.dom;
    let styles = b.styles;
    let mut head: Vec<NodeId> = Vec::new();
    let mut body: Vec<NodeId> = Vec::new();
    let mut foot: Vec<NodeId> = Vec::new();
    let mut caption: Option<NodeId> = None;
    for c in dom.children(n) {
        let d = match styles.get(c) {
            Some(s) => s.display,
            None => continue,
        };
        match d {
            cs::Display::TableRow => body.push(c),
            cs::Display::TableRowGroup
            | cs::Display::TableHeaderGroup
            | cs::Display::TableFooterGroup => {
                let target = match d {
                    cs::Display::TableHeaderGroup => &mut head,
                    cs::Display::TableFooterGroup => &mut foot,
                    _ => &mut body,
                };
                for r in dom.children(c) {
                    if is_display(styles, r, cs::Display::TableRow) {
                        target.push(r);
                    }
                }
            }
            cs::Display::TableCaption => caption = Some(c),
            _ => {}
        }
    }
    let rows: Vec<NodeId> = head.into_iter().chain(body).chain(foot).collect();
    let base: usize = if caption.is_some() { 1 } else { 0 };
    let mut flow: Vec<u32> = Vec::new();
    let cap_box = caption.map(|c| {
        let k = b.make_box(FChild::Elem(c), idx, NONE);
        flow.push(k);
        k
    });
    let mut occ: Vec<Vec<bool>> = vec![Vec::new(); rows.len()];
    let mut ncols: usize = 0;
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0usize;
        for cell in cells_of(dom, styles, *row) {
            while occ[r].get(col).copied().unwrap_or(false) {
                col += 1;
            }
            let cs_ = span_attr(dom, cell, "colspan") as usize;
            let rs = (span_attr(dom, cell, "rowspan") as usize)
                .min(rows.len() - r)
                .max(1);
            for rr in r..r + rs {
                if occ[rr].len() < col + cs_ {
                    occ[rr].resize(col + cs_, false);
                }
                for cc in col..col + cs_ {
                    occ[rr][cc] = true;
                }
            }
            let prev = flow.last().copied().unwrap_or(NONE);
            let k = b.make_box(FChild::Elem(cell), idx, prev);
            {
                let bx = &mut b.t.boxes[k as usize];
                bx.flags |= F_CELL;
                bx.grid = [(r + base) as u16, col as u16, rs as u16, cs_ as u16];
                bx.aux = row.0;
            }
            flow.push(k);
            ncols = ncols.max(col + cs_);
            col += cs_;
        }
    }
    if let Some(k) = cap_box {
        let bx = &mut b.t.boxes[k as usize];
        bx.flags |= F_CELL;
        bx.grid = [0, 0, 1, ncols.max(1) as u16];
    }
    b.t.boxes[idx as usize].aux = ncols as u32;
    b.t.boxes[idx as usize].kind = Kind::Grid;
    b.set_kids_pub(idx, &flow, &[]);
}

/// Table-specific style: one auto track per column, shrink-to-fit width.
pub(crate) fn style_table(cx: &Ctx, b: u32, s: &mut tf::Style) {
    let ncols = cx.t.boxes[b as usize].aux as usize;
    let mut cols = Vec::new();
    for _ in 0..ncols.min(1000) {
        cols.push(tf::GridTemplateComponent::Single(tf::MinMax {
            min: tf::MinTrackSizingFunction::auto(),
            max: tf::MaxTrackSizingFunction::auto(),
        }));
    }
    s.grid_template_columns = cols;
    s.grid_template_rows = Vec::new();
    if let Some(cp) = cx.comp_of(b) {
        if matches!(cp.width, cs::Length::Auto) {
            s.size.width = tf::Dimension::fit_content();
        }
    }
}

/// Cell-specific style: explicit grid placement and vertical alignment.
pub(crate) fn style_cell(cx: &Ctx, b: u32, s: &mut tf::Style) {
    let g = cx.t.boxes[b as usize].grid;
    let line = |i: u16| tf::style_helpers::line::<tf::GridPlacement>((i + 1) as i16);
    s.grid_row = tf::Line {
        start: line(g[0]),
        end: tf::GridPlacement::Span(g[2]),
    };
    s.grid_column = tf::Line {
        start: line(g[1]),
        end: tf::GridPlacement::Span(g[3]),
    };
    // A table cell's height is a minimum (CSS 2.1 17.5.3): rowspan cells must still stretch.
    let h = s.size.height.into_raw();
    let min = match h.tag() {
        tf::CompactLength::LENGTH_TAG => Some(tf::LengthPercentageAuto::length(h.value())),
        tf::CompactLength::PERCENT_TAG => Some(tf::LengthPercentageAuto::percent(h.value())),
        _ => None,
    };
    if let Some(min) = min {
        s.min_size.height = min;
        s.size.height = tf::Dimension::auto();
    }
    if let Some(cp) = cx.comp_of(b) {
        s.align_self = match &cp.vertical_align {
            cs::VAlign::Keyword(cs::VAlignKw::Middle) => Some(tf::AlignItems::CENTER),
            cs::VAlign::Keyword(cs::VAlignKw::Bottom) => Some(tf::AlignItems::END),
            _ => None,
        };
    }
}
