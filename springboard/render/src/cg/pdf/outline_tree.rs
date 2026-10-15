//! The outline (bookmark) tree of a PDF as nested nodes. [`page::outline`](super::page::outline) returns the entries
//! flattened in reading order with their levels; this module rebuilds the nesting, so an app can show the table of
//! contents as a tree and walk it depth first.
//!
//! An entry whose level is more than one step below the entry before it is placed directly under the nearest open
//! entry, so no entry is lost to a malformed level. The nodes own copies of the entries, so the tree does not borrow
//! the document.

use super::page::OutlineItem;

/// One outline entry and the entries nested under it, in reading order.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineNode {
    pub item: OutlineItem,
    pub children: Vec<OutlineNode>,
}

/// Rebuilds the nesting of flattened outline entries, as [`page::outline`](super::page::outline) returns them.
pub fn outline_tree(items: &[OutlineItem]) -> Vec<OutlineNode> {
    let mut pos = 0;
    take_level(items, &mut pos, 0)
}

/// Takes the entries from `*pos` that sit at `level` or deeper, nesting each one under the entries that follow it.
/// An entry above `level` ends this run and belongs to an ancestor.
fn take_level(items: &[OutlineItem], pos: &mut usize, level: u8) -> Vec<OutlineNode> {
    let mut out = Vec::new();
    while let Some(item) = items.get(*pos) {
        if item.level < level {
            break;
        }
        *pos += 1;
        let children = take_level(items, pos, item.level.saturating_add(1));
        out.push(OutlineNode { item: item.clone(), children });
    }
    out
}

/// The number of entries in the tree, the nested ones included.
pub fn entry_count(nodes: &[OutlineNode]) -> usize {
    nodes.iter().map(|n| 1 + entry_count(&n.children)).sum()
}

/// The titles of the tree's entries in reading order: each entry, then its children.
pub fn titles(nodes: &[OutlineNode]) -> Vec<&str> {
    let mut out = Vec::new();
    for node in nodes {
        out.push(node.item.title.as_str());
        out.extend(titles(&node.children));
    }
    out
}
