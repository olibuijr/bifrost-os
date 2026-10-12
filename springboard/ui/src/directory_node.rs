//! Directory sessions and nodes: a session holds a tree of named nodes, and each node holds the records of one
//! directory service.
//!
//! A node has a path such as `/Local/Default`. The first component is a top-level node; every other node must sit
//! under a node that already exists, and [`Session::subnode_names`] lists the immediate children of a node in name
//! order. Each node keeps its records keyed by record type and record name, so a record name is unique within its
//! type. The model is pure: it opens no connection to a server. A node that stands for a remote service is only a
//! named container here, filled by the caller.

use std::collections::BTreeMap;

use crate::directory_query::DirRecord;

/// Longest node path, in bytes.
pub const MAX_PATH: usize = 256;
/// Most nodes kept in one session.
pub const MAX_NODES: usize = 1_000;
/// Most records kept in one node.
pub const MAX_RECORDS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The path is empty, too long, does not start with `/`, or has an empty, `.` or `..` component.
    BadPath,
    UnknownNode,
    NodeExists,
    /// The parent of a node is missing from the session.
    ParentMissing,
    /// The node cannot be removed because other nodes sit under it.
    NodeHasSubnodes,
    /// A record with the same type and name is already in the node.
    DuplicateRecord,
    TooManyNodes,
    TooManyRecords,
}

/// A node of a session: a path, and the records it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    path: String,
    records: BTreeMap<(String, String), DirRecord>,
}

impl Node {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The records in (type, name) order.
    pub fn records(&self) -> Vec<&DirRecord> {
        self.records.values().collect()
    }

    pub fn record(&self, record_type: &str, name: &str) -> Option<&DirRecord> {
        self.records.get(&(record_type.to_string(), name.to_string()))
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Adds a record. A record with the same type and name is refused; the caller removes it first to replace it.
    pub fn add_record(&mut self, record: DirRecord) -> Result<(), Error> {
        if self.records.len() >= MAX_RECORDS {
            return Err(Error::TooManyRecords);
        }
        let key = (record.record_type().to_string(), record.name().to_string());
        if self.records.contains_key(&key) {
            return Err(Error::DuplicateRecord);
        }
        self.records.insert(key, record);
        Ok(())
    }

    /// Removes a record and returns it, or `None` when no record has that type and name.
    pub fn remove_record(&mut self, record_type: &str, name: &str) -> Option<DirRecord> {
        self.records.remove(&(record_type.to_string(), name.to_string()))
    }
}

/// The nodes that one session has opened. Paths are unique and kept in path order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    nodes: BTreeMap<String, Node>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Adds an empty node. A top-level path (one component) needs no parent; any other path needs its parent node.
    pub fn add_node(&mut self, path: &str) -> Result<(), Error> {
        let parent = parent_path(path)?;
        if self.nodes.len() >= MAX_NODES {
            return Err(Error::TooManyNodes);
        }
        if self.nodes.contains_key(path) {
            return Err(Error::NodeExists);
        }
        if let Some(parent) = parent {
            if !self.nodes.contains_key(parent) {
                return Err(Error::ParentMissing);
            }
        }
        self.nodes.insert(path.to_string(), Node { path: path.to_string(), records: BTreeMap::new() });
        Ok(())
    }

    /// Removes a node and returns it. A node with subnodes is kept, so no subnode is left without a parent.
    pub fn remove_node(&mut self, path: &str) -> Result<Node, Error> {
        if !self.nodes.contains_key(path) {
            return Err(Error::UnknownNode);
        }
        if self.has_subnodes(path) {
            return Err(Error::NodeHasSubnodes);
        }
        self.nodes.remove(path).ok_or(Error::UnknownNode)
    }

    pub fn node(&self, path: &str) -> Result<&Node, Error> {
        self.nodes.get(path).ok_or(Error::UnknownNode)
    }

    pub fn node_mut(&mut self, path: &str) -> Result<&mut Node, Error> {
        self.nodes.get_mut(path).ok_or(Error::UnknownNode)
    }

    /// The names of the immediate subnodes of `path`, in name order. Pass `""` to list the top-level nodes.
    pub fn subnode_names(&self, path: &str) -> Result<Vec<String>, Error> {
        if !path.is_empty() && !self.nodes.contains_key(path) {
            return Err(Error::UnknownNode);
        }
        let mut names: Vec<String> = Vec::new();
        for p in self.nodes.keys() {
            if let Ok(parent) = parent_path(p) {
                if parent.unwrap_or("") == path {
                    names.push(last_component(p).to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn has_subnodes(&self, path: &str) -> bool {
        self.nodes.keys().any(|p| parent_path(p).ok().flatten() == Some(path))
    }
}

/// The path of the parent of `path`, `None` for a top-level node. Checks the path as well.
fn parent_path(path: &str) -> Result<Option<&str>, Error> {
    check_path(path)?;
    let cut = path.rfind('/').ok_or(Error::BadPath)?;
    if cut == 0 {
        Ok(None)
    } else {
        Ok(Some(&path[..cut]))
    }
}

fn last_component(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

fn check_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.len() > MAX_PATH || !path.starts_with('/') || path == "/" {
        return Err(Error::BadPath);
    }
    for component in path[1..].split('/') {
        if component.is_empty() || component == "." || component == ".." || component.contains('\0') {
            return Err(Error::BadPath);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(record_type: &str, name: &str) -> DirRecord {
        DirRecord::new(record_type, name)
    }

    #[test]
    fn a_node_needs_its_parent_and_a_top_level_node_needs_none() {
        let mut s = Session::new();
        assert_eq!(s.add_node("/Local/Default"), Err(Error::ParentMissing), "the parent /Local is missing");
        assert_eq!(s.add_node("/Local"), Ok(()));
        assert_eq!(s.add_node("/Local/Default"), Ok(()));
        assert_eq!(s.add_node("/Local"), Err(Error::NodeExists));
        assert_eq!(s.node_count(), 2);
    }

    #[test]
    fn paths_must_be_absolute_and_free_of_empty_or_dot_components() {
        let mut s = Session::new();
        assert_eq!(s.add_node(""), Err(Error::BadPath));
        assert_eq!(s.add_node("Local"), Err(Error::BadPath), "no leading slash");
        assert_eq!(s.add_node("/"), Err(Error::BadPath));
        assert_eq!(s.add_node("/Local//Default"), Err(Error::BadPath), "empty component");
        assert_eq!(s.add_node("/Local/.."), Err(Error::BadPath), "parent reference");
        assert_eq!(s.add_node("/Local/"), Err(Error::BadPath), "trailing slash is an empty component");
    }

    #[test]
    fn subnode_names_list_the_immediate_children_in_name_order() {
        let mut s = Session::new();
        for p in ["/Local", "/Search", "/Local/Default", "/Local/Cache", "/Local/Cache/Shared"] {
            s.add_node(p).unwrap();
        }
        assert_eq!(s.subnode_names("").unwrap(), vec!["Local", "Search"]);
        assert_eq!(s.subnode_names("/Local").unwrap(), vec!["Cache", "Default"], "Shared is two levels down");
        assert_eq!(s.subnode_names("/Local/Cache").unwrap(), vec!["Shared"]);
        assert_eq!(s.subnode_names("/Search").unwrap(), Vec::<String>::new());
        assert_eq!(s.subnode_names("/Missing"), Err(Error::UnknownNode));
    }

    #[test]
    fn a_node_with_subnodes_cannot_be_removed_until_they_are() {
        let mut s = Session::new();
        s.add_node("/Local").unwrap();
        s.add_node("/Local/Default").unwrap();
        assert_eq!(s.remove_node("/Local"), Err(Error::NodeHasSubnodes));
        let removed = s.remove_node("/Local/Default").unwrap();
        assert_eq!(removed.path(), "/Local/Default");
        let parent = s.remove_node("/Local").unwrap();
        assert_eq!(parent.path(), "/Local", "once its subnodes are gone the parent can go");
        assert_eq!(s.node_count(), 0);
        assert_eq!(s.remove_node("/Local"), Err(Error::UnknownNode));
    }

    #[test]
    fn records_are_unique_by_type_and_name() {
        let mut n = Node::default();
        n.add_record(record("dsRecTypeStandard:Users", "anna")).unwrap();
        n.add_record(record("dsRecTypeStandard:Groups", "anna")).unwrap();
        assert_eq!(n.add_record(record("dsRecTypeStandard:Users", "anna")), Err(Error::DuplicateRecord));
        assert_eq!(n.record_count(), 2, "same name under another type is a different record");
        assert!(n.record("dsRecTypeStandard:Groups", "anna").is_some());
        assert!(n.remove_record("dsRecTypeStandard:Users", "anna").is_some());
        assert!(n.remove_record("dsRecTypeStandard:Users", "anna").is_none());
        assert_eq!(n.record_count(), 1);
    }

    #[test]
    fn records_list_in_type_then_name_order() {
        let mut n = Node::default();
        n.add_record(record("b-type", "x")).unwrap();
        n.add_record(record("a-type", "z")).unwrap();
        n.add_record(record("a-type", "m")).unwrap();
        let listed = n.records();
        let order: Vec<(&str, &str)> = listed.iter().map(|r| (r.record_type(), r.name())).collect();
        assert_eq!(order, vec![("a-type", "m"), ("a-type", "z"), ("b-type", "x")]);
    }
}
