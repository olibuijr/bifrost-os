//! Threads: messages that belong to one conversation. Messages are joined through their Message-ID, References
//! and In-Reply-To. A message with no threading headers joins the thread of an earlier message with the same
//! normalised subject that also has no threading headers, the usual fallback for mail that lost its References.

use std::collections::{HashMap, HashSet};

/// The threading data of one message. `index` is the caller's position for the message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub subject: String,
}

/// Removes reply and forward prefixes (`Re:`, `RE:`, `Fwd:`, `Fw:`), repeated, and lowercases the rest.
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let cut = ["re:", "fwd:", "fw:"].iter().find(|p| lower.starts_with(*p)).map(|p| p.len());
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    s.to_lowercase()
}

struct Forest {
    parent: Vec<usize>,
}

impl Forest {
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // the earlier index stays the root, so groups keep the caller's order
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi] = lo;
        }
    }
}

/// Groups messages into threads. Each group lists indices into `nodes` in ascending order, and groups are ordered by
/// their first message. Every index appears in exactly one group.
pub fn group(nodes: &[Node]) -> Vec<Vec<usize>> {
    let mut forest = Forest { parent: (0..nodes.len()).collect() };
    // every id seen so far (a message's own id or any id it references) -> a message that carries it
    let mut owner: HashMap<&str, usize> = HashMap::new();
    let mut by_subject: HashMap<String, usize> = HashMap::new();

    for (i, node) in nodes.iter().enumerate() {
        let mut ids: Vec<&str> = node.references.iter().chain(node.in_reply_to.iter()).map(String::as_str).collect();
        if let Some(own) = node.message_id.as_deref() {
            ids.push(own);
        }
        for id in ids {
            match owner.get(id) {
                Some(&other) => forest.union(i, other),
                None => {
                    owner.insert(id, i);
                }
            }
        }
        // headers-less messages: a shared normalised subject is enough
        if node.references.is_empty() && node.in_reply_to.is_empty() {
            let key = normalize_subject(&node.subject);
            if !key.is_empty() {
                match by_subject.get(&key) {
                    Some(&other) => forest.union(i, other),
                    None => {
                        by_subject.insert(key, i);
                    }
                }
            }
        }
    }

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for i in 0..nodes.len() {
        let root = forest.find(i);
        let g = *slot.entry(root).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(i);
    }
    groups
}

/// The members of one group in tree order, each with its depth (0 for a conversation's first message). A member's parent
/// is the nearest message it replies to that is in the group (In-Reply-To first, then References from the end). Roots
/// keep the group's order (the newest first in the list); the replies under a parent come oldest first, so a
/// conversation reads from its start down. A member caught in a reference cycle is placed as a root.
pub fn arrange(nodes: &[Node], group: &[usize]) -> Vec<(usize, usize)> {
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    for &i in group {
        if let Some(id) = nodes[i].message_id.as_deref() {
            by_id.entry(id).or_insert(i);
        }
    }
    let parent_of = |i: usize| -> Option<usize> {
        let node = &nodes[i];
        node.in_reply_to
            .iter()
            .chain(node.references.iter().rev())
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .find(|&p| p != i)
    };
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for &i in group {
        match parent_of(i) {
            Some(p) => children.entry(p).or_default().push(i),
            None => roots.push(i),
        }
    }
    let mut seen: HashSet<usize> = HashSet::new();
    let mut out = Vec::with_capacity(group.len());
    for &root in &roots {
        visit(root, 0, &children, &mut seen, &mut out);
    }
    for &i in group {
        visit(i, 0, &children, &mut seen, &mut out);
    }
    out
}

fn visit(i: usize, depth: usize, children: &HashMap<usize, Vec<usize>>, seen: &mut HashSet<usize>, out: &mut Vec<(usize, usize)>) {
    if !seen.insert(i) {
        return;
    }
    out.push((i, depth));
    if let Some(kids) = children.get(&i) {
        // the group lists the newest first, so the reverse puts the oldest reply first
        for &kid in kids.iter().rev() {
            visit(kid, depth + 1, children, seen, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_sit_under_their_parent_with_a_depth() {
        let nodes = vec![
            node("<a>", None, &[], "Plan"),
            node("<b>", Some("<a>"), &["<a>"], "Re: Plan"),
            node("<e>", Some("<b>"), &["<a>", "<b>"], "Re: Plan"),
            node("<f>", Some("<a>"), &["<a>"], "Re: Plan"),
        ];
        assert_eq!(arrange(&nodes, &[0, 1, 2, 3]), vec![(0, 0), (3, 1), (1, 1), (2, 2)]);
    }

    #[test]
    fn a_reply_whose_parent_is_missing_is_a_root() {
        let nodes = vec![node("<b>", Some("<gone>"), &["<gone>"], "Re: Plan"), node("<c>", None, &[], "Other")];
        assert_eq!(arrange(&nodes, &[0, 1]), vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn a_reference_cycle_still_lists_every_member_once() {
        let nodes = vec![node("<a>", Some("<b>"), &["<b>"], "x"), node("<b>", Some("<a>"), &["<a>"], "x")];
        let mut seen: Vec<usize> = arrange(&nodes, &[0, 1]).into_iter().map(|(i, _)| i).collect();
        seen.sort();
        assert_eq!(seen, vec![0, 1]);
    }

    fn node(id: &str, parent: Option<&str>, refs: &[&str], subject: &str) -> Node {
        Node {
            message_id: Some(id.to_string()),
            in_reply_to: parent.map(|p| vec![p.to_string()]).unwrap_or_default(),
            references: refs.iter().map(|r| r.to_string()).collect(),
            subject: subject.to_string(),
        }
    }

    #[test]
    fn subject_prefixes_are_stripped() {
        assert_eq!(normalize_subject("Re: RE: Fwd:  Fundur"), "fundur");
        assert_eq!(normalize_subject("Fw: Re: Plan"), "plan");
        assert_eq!(normalize_subject("  "), "");
    }

    #[test]
    fn references_join_a_conversation_and_missing_roots_still_group() {
        let nodes = vec![
            node("<a>", None, &[], "Plan"),
            node("<b>", Some("<a>"), &["<a>"], "Re: Plan"),
            node("<c>", Some("<z>"), &["<z>"], "Other"),
            node("<d>", Some("<z>"), &["<z>"], "Other again"),
            node("<e>", Some("<b>"), &["<a>", "<b>"], "Re: Plan"),
        ];
        assert_eq!(group(&nodes), vec![vec![0, 1, 4], vec![2, 3]]);
    }

    #[test]
    fn headerless_messages_group_by_subject_only_with_each_other() {
        let mut lone = node("<x>", None, &[], "Lunch");
        lone.message_id = None;
        let nodes = vec![
            node("<a>", None, &[], "Lunch"),
            lone.clone(),
            Node { subject: "Re: Lunch".into(), ..lone.clone() },
            node("<r>", Some("<a>"), &["<a>"], "Lunch"),
        ];
        assert_eq!(group(&nodes), vec![vec![0, 1, 2, 3]]);
        let apart = vec![node("<a>", Some("<q>"), &["<q>"], "Lunch"), lone];
        assert_eq!(group(&apart), vec![vec![0], vec![1]]);
    }

    #[test]
    fn every_index_is_in_one_group() {
        let nodes: Vec<Node> = (0..20).map(|i| node(&format!("<{i}>"), None, &[], &format!("s{}", i % 3))).collect();
        let groups = group(&nodes);
        let mut all: Vec<usize> = groups.concat();
        all.sort();
        assert_eq!(all, (0..20).collect::<Vec<_>>());
    }
}
