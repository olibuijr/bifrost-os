//! Hit testing: which element is at a page-space point, walking the boxes in
//! reverse paint order through scroll containers, sticky and fixed offsets
//! and 2D transforms (using the scroll offset of the last `display_list`).

use super::super::display::{xf_point, Rect};
use super::super::dom::{Dom, NodeId};
use super::inline::FragKind;
use super::paint::{box_matrix, child_state, layer_key, own_off, root_state, St};
use super::positioned::invert;
use super::tree::*;
use super::LayoutTree;

/// The deepest element at the page-space point `(x, y)`.
pub fn hit_test(t: &LayoutTree, x: f32, y: f32) -> Option<NodeId> {
    if t.boxes.is_empty() || t.paint.len() != t.boxes.len() {
        return None;
    }
    let scroll = t.last_scroll.get();
    let st = root_state(t.viewport, scroll);
    hit_box(t, t.root, (0.0, 0.0), st, (x, y), scroll)
}

/// The nearest `<a href>` at the point (the hit element or an ancestor).
pub fn link_at(t: &LayoutTree, dom: &Dom, x: f32, y: f32) -> Option<NodeId> {
    let mut cur = hit_test(t, x, y);
    while let Some(n) = cur {
        if dom.tag(n) == Some("a") && dom.attr(n, "href").is_some() {
            return Some(n);
        }
        cur = dom.parent(n);
    }
    None
}

/// The nearest scroll container (`overflow: auto|scroll`) at the point.
pub fn scroller_at(t: &LayoutTree, dom: &Dom, x: f32, y: f32) -> Option<NodeId> {
    let mut cur = hit_test(t, x, y);
    while let Some(n) = cur {
        if let Some(b) = box_for(&t.box_of, n) {
            if t.paint[b as usize].scroll {
                return Some(n);
            }
        }
        cur = dom.parent(n);
    }
    None
}

impl LayoutTree {
    /// See [`hit_test`].
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        hit_test(self, x, y)
    }
}

fn node_of(t: &LayoutTree, k: u32) -> Option<NodeId> {
    let d = t.boxes[k as usize].dom;
    if d == NONE {
        None
    } else {
        Some(NodeId(d))
    }
}

fn inside(r: [f32; 4], p: (f32, f32)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

fn hit_box(
    t: &LayoutTree,
    k: u32,
    off: (f32, f32),
    st: St,
    p: (f32, f32),
    scroll: (f32, f32),
) -> Option<NodeId> {
    let bx = &t.boxes[k as usize];
    let pp = &t.paint[k as usize];
    let own = own_off(t, k, off, scroll, &st);
    let mut p = p;
    if let Some(m) = box_matrix(t, k, own) {
        let inv = invert(&m)?;
        p = xf_point(&inv, p.0, p.1);
    }
    let rect = [bx.abs[0] + own.0, bx.abs[1] + own.1, bx.size[0], bx.size[1]];
    let (cst, coff) = child_state(t, k, own, &st);
    let in_pad = !pp.clip || inside(cst.port, p);
    if in_pad {
        let (k0, nk, na) = (bx.kids as usize, bx.nkids as usize, bx.nabs as usize);
        let is_ifc = bx.kind == Kind::Ifc;
        let mut cands: Vec<((i32, i32), u32)> = Vec::new();
        if !is_ifc {
            for i in 0..nk {
                let kid = t.kids[k0 + i];
                cands.push((layer_key(&t.paint[kid as usize]), kid));
            }
        }
        for j in 0..na {
            let kid = t.kids[k0 + nk + j];
            cands.push((layer_key(&t.paint[kid as usize]), kid));
        }
        cands.sort_by_key(|c| c.0);
        for c in cands.iter().rev().filter(|c| c.0 .0 >= 1) {
            if let Some(h) = hit_box(t, c.1, coff, cst, p, scroll) {
                return Some(h);
            }
        }
        if is_ifc && bx.ifc != NONE {
            let d = &t.ifcs[bx.ifc as usize];
            let ox = bx.abs[0] + coff.0 + bx.border[3] + bx.padding[3];
            let oy = bx.abs[1] + coff.1 + bx.border[0] + bx.padding[0];
            for f in d.frags.iter().rev() {
                match f.kind {
                    FragKind::Atomic => {
                        if let Some(h) = hit_box(t, f.node, coff, cst, p, scroll) {
                            return Some(h);
                        }
                    }
                    FragKind::Text => {
                        if inside([ox + f.x, oy + f.y, f.w, f.h], p) {
                            return Some(dec(f.node).0);
                        }
                    }
                    FragKind::InlineBox => {}
                }
            }
            for f in d.frags.iter().filter(|f| f.kind == FragKind::InlineBox) {
                if inside([ox + f.x, oy + f.y, f.w, f.h], p) {
                    return Some(dec(f.node).0);
                }
            }
        } else {
            for c in cands.iter().rev().filter(|c| c.0 .0 == 0) {
                if let Some(h) = hit_box(t, c.1, coff, cst, p, scroll) {
                    return Some(h);
                }
            }
        }
        for c in cands.iter().rev().filter(|c| c.0 .0 < 0) {
            if let Some(h) = hit_box(t, c.1, coff, cst, p, scroll) {
                return Some(h);
            }
        }
    }
    if pp.visible && inside(rect, p) {
        return node_of(t, k);
    }
    None
}

#[allow(dead_code)]
fn _unused(_: Rect) {}
