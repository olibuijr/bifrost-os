//! A snapshot of a page's document tree: the nodes an in-page script reported, kept as a tree the app can walk
//! (the reference SDK's DOM node snapshot). Pure model: the snapshot is built from flat node records (id, parent,
//! kind, name, text, attributes) that the web engine's script collector produces. Nothing here reads a live page, so a
//! snapshot does not change when the page does.

use std::collections::{BTreeMap, BTreeSet};

/// What a node is in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// The document itself. Exactly one node of a snapshot has this kind at the top.
    Document,
    /// An element such as a paragraph or a link. `name` holds the tag name.
    Element,
    /// A run of text. `text` holds the characters.
    Text,
}

/// One node as the collector reported it. `parent` is `None` only for the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeRecord {
    pub id: u32,
    pub parent: Option<u32>,
    pub kind: NodeKind,
    pub name: String,
    pub text: String,
    pub attributes: Vec<(String, String)>,
}

/// Why a set of records does not form a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// No record lacks a parent.
    NoRoot,
    /// More than one record lacks a parent.
    MultipleRoots,
    /// Two records share an id.
    DuplicateId(u32),
    /// A record names a parent id that is not in the set. The field is the child's id.
    UnknownParent(u32),
    /// A record cannot be reached from the root, which means a cycle or a detached branch. The field is its id.
    Unreachable(u32),
}

/// A validated, immutable tree of nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomSnapshot {
    nodes: BTreeMap<u32, NodeRecord>,
    children: BTreeMap<u32, Vec<u32>>,
    root: u32,
    order: Vec<u32>,
}

impl DomSnapshot {
    /// Builds a snapshot. Children keep the order of the records given, so the tree reads as the page does.
    pub fn new(records: Vec<NodeRecord>) -> Result<DomSnapshot, SnapshotError> {
        let mut ids = BTreeSet::new();
        for r in &records {
            if !ids.insert(r.id) {
                return Err(SnapshotError::DuplicateId(r.id));
            }
        }
        let mut root = None;
        let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for r in &records {
            match r.parent {
                None => {
                    if root.is_some() {
                        return Err(SnapshotError::MultipleRoots);
                    }
                    root = Some(r.id);
                }
                Some(p) => {
                    if !ids.contains(&p) {
                        return Err(SnapshotError::UnknownParent(r.id));
                    }
                    children.entry(p).or_default().push(r.id);
                }
            }
        }
        let root = root.ok_or(SnapshotError::NoRoot)?;
        let mut order = Vec::with_capacity(ids.len());
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            order.push(id);
            if let Some(kids) = children.get(&id) {
                for k in kids.iter().rev() {
                    stack.push(*k);
                }
            }
        }
        if order.len() != ids.len() {
            let reached: BTreeSet<u32> = order.iter().copied().collect();
            let stray = ids.iter().find(|id| !reached.contains(id)).copied().unwrap_or(root);
            return Err(SnapshotError::Unreachable(stray));
        }
        let nodes = records.into_iter().map(|r| (r.id, r)).collect();
        Ok(DomSnapshot { nodes, children, root, order })
    }

    /// The id of the root node.
    pub fn root(&self) -> u32 {
        self.root
    }

    /// Number of nodes in the snapshot.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the snapshot holds no node. A built snapshot always has the root, so this is never true for one.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The record for `id`, if the snapshot has it.
    pub fn node(&self, id: u32) -> Option<&NodeRecord> {
        self.nodes.get(&id)
    }

    /// The child ids of `id`, in the order the records gave them. Empty for a leaf or an unknown id.
    pub fn children(&self, id: u32) -> &[u32] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every node id in document order (depth first, parents before children, earlier siblings first).
    pub fn document_order(&self) -> &[u32] {
        &self.order
    }

    /// The value of attribute `name` on node `id`, if both exist.
    pub fn attribute(&self, id: u32, name: &str) -> Option<&str> {
        self.nodes
            .get(&id)?
            .attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The text of every text node under `id` (inclusive), joined in document order.
    pub fn text_content(&self, id: u32) -> String {
        let mut out = String::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(rec) = self.nodes.get(&n) {
                if rec.kind == NodeKind::Text {
                    out.push_str(&rec.text);
                }
            }
            for k in self.children(n).iter().rev() {
                stack.push(*k);
            }
        }
        out
    }

    /// Ids of the nodes whose attribute `name` equals `value`, in document order.
    pub fn find_by_attribute(&self, name: &str, value: &str) -> Vec<u32> {
        self.order
            .iter()
            .copied()
            .filter(|id| self.attribute(*id, name) == Some(value))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: u32, parent: Option<u32>, kind: NodeKind, name: &str, text: &str) -> NodeRecord {
        NodeRecord { id, parent, kind, name: name.to_string(), text: text.to_string(), attributes: Vec::new() }
    }

    /// Tree: 0 document -> 1 html -> 2 body -> { 3 text "Hello, ", 4 p -> 6 text "world" }; 0 -> 5 footer.
    fn sample() -> DomSnapshot {
        let mut link = rec(4, Some(2), NodeKind::Element, "a", "");
        link.attributes.push(("href".to_string(), "https://a.example".to_string()));
        DomSnapshot::new(vec![
            rec(0, None, NodeKind::Document, "", ""),
            rec(1, Some(0), NodeKind::Element, "html", ""),
            rec(2, Some(1), NodeKind::Element, "body", ""),
            rec(3, Some(2), NodeKind::Text, "", "Hello, "),
            link,
            rec(5, Some(0), NodeKind::Element, "footer", ""),
            rec(6, Some(4), NodeKind::Text, "", "world"),
        ])
        .unwrap()
    }

    #[test]
    fn document_order_walks_depth_first() {
        let s = sample();
        assert_eq!(s.document_order(), &[0, 1, 2, 3, 4, 6, 5]);
        assert_eq!(s.root(), 0);
        assert_eq!(s.len(), 7);
        assert_eq!(s.children(2), &[3, 4]);
        assert_eq!(s.children(6), &[] as &[u32]);
    }

    #[test]
    fn text_content_joins_text_nodes_in_order() {
        let s = sample();
        assert_eq!(s.text_content(2), "Hello, world");
        assert_eq!(s.text_content(5), "");
        assert_eq!(s.text_content(4), "world");
    }

    #[test]
    fn attributes_and_attribute_search() {
        let s = sample();
        assert_eq!(s.attribute(4, "href"), Some("https://a.example"));
        assert_eq!(s.attribute(4, "id"), None);
        assert_eq!(s.find_by_attribute("href", "https://a.example"), vec![4]);
        assert!(s.find_by_attribute("href", "https://b.example").is_empty());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let err = DomSnapshot::new(vec![rec(0, None, NodeKind::Document, "", ""), rec(0, None, NodeKind::Element, "p", "")]);
        assert_eq!(err, Err(SnapshotError::DuplicateId(0)));
    }

    #[test]
    fn rejects_two_roots_and_unknown_parents() {
        let two = DomSnapshot::new(vec![rec(0, None, NodeKind::Document, "", ""), rec(1, None, NodeKind::Element, "p", "")]);
        assert_eq!(two, Err(SnapshotError::MultipleRoots));
        let unknown = DomSnapshot::new(vec![rec(0, None, NodeKind::Document, "", ""), rec(1, Some(9), NodeKind::Element, "p", "")]);
        assert_eq!(unknown, Err(SnapshotError::UnknownParent(1)));
    }

    #[test]
    fn rejects_a_cycle_and_a_missing_root() {
        let cycle = DomSnapshot::new(vec![
            rec(0, None, NodeKind::Document, "", ""),
            rec(1, Some(2), NodeKind::Element, "a", ""),
            rec(2, Some(1), NodeKind::Element, "b", ""),
        ]);
        assert_eq!(cycle, Err(SnapshotError::Unreachable(1)));
        let no_root = DomSnapshot::new(vec![rec(1, Some(2), NodeKind::Element, "a", ""), rec(2, Some(1), NodeKind::Element, "b", "")]);
        assert_eq!(no_root, Err(SnapshotError::NoRoot));
    }
}
