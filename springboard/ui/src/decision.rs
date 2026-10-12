//! Decision tree: a tree of questions about named attributes, whose branches are picked by the answer, and whose leaves
//! name an action. The caller gives the answers it knows as a map from attribute to answer and reads back the action at
//! the leaf the answers lead to.
//!
//! A question node tests one attribute. Each branch carries a test (an exact answer, or a predicate over the answer);
//! the first branch whose test passes is followed, in the order the branches were added. The walk stops with None when
//! the attribute has no answer, or when no branch matches. Nothing is random and nothing is learned from examples: a
//! tree is written out by the caller.

use std::collections::BTreeMap;

/// How a branch decides whether it applies to an answer.
pub enum Test {
    /// The answer is exactly this string.
    Equals(String),
    /// The predicate returns true for the answer.
    Where(Box<dyn Fn(&str) -> bool>),
}

impl Test {
    pub fn matches(&self, answer: &str) -> bool {
        match self {
            Test::Equals(value) => value == answer,
            Test::Where(predicate) => predicate(answer),
        }
    }
}

/// A node of a decision tree: either a question with branches or an action.
pub struct DecisionNode {
    attribute: Option<String>,
    action: Option<String>,
    branches: Vec<(Test, DecisionNode)>,
}

impl DecisionNode {
    /// A question about `attribute`. Add branches with `branch`.
    pub fn question(attribute: &str) -> Self {
        DecisionNode { attribute: Some(attribute.to_string()), action: None, branches: Vec::new() }
    }

    /// A leaf that says what to do. It has no branches.
    pub fn action(name: &str) -> Self {
        DecisionNode { attribute: None, action: Some(name.to_string()), branches: Vec::new() }
    }

    /// Adds a branch to this question. A leaf given a branch keeps its action and never reaches the branch.
    pub fn branch(mut self, test: Test, child: DecisionNode) -> Self {
        self.branches.push((test, child));
        self
    }

    pub fn attribute(&self) -> Option<&str> {
        self.attribute.as_deref()
    }
}

/// A decision tree with a root node.
pub struct DecisionTree {
    root: DecisionNode,
}

impl DecisionTree {
    pub fn new(root: DecisionNode) -> Self {
        DecisionTree { root }
    }

    pub fn root(&self) -> &DecisionNode {
        &self.root
    }

    /// Follows the answers from the root and returns the action of the leaf reached. A leaf is reached as soon as a
    /// node has an action, so an action node is never asked a question.
    pub fn find_action(&self, answers: &BTreeMap<String, String>) -> Option<&str> {
        let mut node = &self.root;
        loop {
            if let Some(action) = &node.action {
                return Some(action.as_str());
            }
            let attribute = node.attribute.as_ref()?;
            let answer = answers.get(attribute)?;
            node = node.branches.iter().find(|(test, _)| test.matches(answer)).map(|(_, child)| child)?;
        }
    }

    /// Number of nodes in the tree, leaves included.
    pub fn node_count(&self) -> usize {
        fn count(node: &DecisionNode) -> usize {
            1 + node.branches.iter().map(|(_, child)| count(child)).sum::<usize>()
        }
        count(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// A small tree: is it raining? If yes, is the person carrying an umbrella? Otherwise go out.
    fn weather_tree() -> DecisionTree {
        let umbrella = DecisionNode::question("umbrella")
            .branch(Test::Equals("yes".to_string()), DecisionNode::action("walk under cover"))
            .branch(Test::Equals("no".to_string()), DecisionNode::action("wait indoors"));
        let root = DecisionNode::question("rain")
            .branch(Test::Equals("yes".to_string()), umbrella)
            .branch(Test::Equals("no".to_string()), DecisionNode::action("go out"));
        DecisionTree::new(root)
    }

    #[test]
    fn follows_the_answers_to_the_matching_leaf() {
        let t = weather_tree();
        assert_eq!(t.find_action(&answers(&[("rain", "no")])), Some("go out"));
        assert_eq!(t.find_action(&answers(&[("rain", "yes"), ("umbrella", "yes")])), Some("walk under cover"));
        assert_eq!(t.find_action(&answers(&[("rain", "yes"), ("umbrella", "no")])), Some("wait indoors"));
    }

    #[test]
    fn a_missing_answer_or_an_unmatched_branch_gives_none() {
        let t = weather_tree();
        assert_eq!(t.find_action(&answers(&[])), None, "no answer for the root attribute");
        assert_eq!(t.find_action(&answers(&[("rain", "yes")])), None, "no answer for umbrella");
        assert_eq!(t.find_action(&answers(&[("rain", "maybe")])), None, "no branch equals maybe");
    }

    #[test]
    fn the_first_matching_branch_wins() {
        let root = DecisionNode::question("temperature")
            .branch(Test::Where(Box::new(|t: &str| t.parse::<i32>().map(|v| v < 10).unwrap_or(false))), DecisionNode::action("cold"))
            .branch(Test::Where(Box::new(|_: &str| true)), DecisionNode::action("mild"));
        let t = DecisionTree::new(root);
        assert_eq!(t.find_action(&answers(&[("temperature", "4")])), Some("cold"));
        assert_eq!(t.find_action(&answers(&[("temperature", "25")])), Some("mild"));
        assert_eq!(t.find_action(&answers(&[("temperature", "x")])), Some("mild"), "parse failure is not a cold match");
    }

    #[test]
    fn an_action_node_is_a_leaf_even_with_branches() {
        let root = DecisionNode::action("stop").branch(Test::Equals("a".to_string()), DecisionNode::action("never"));
        let t = DecisionTree::new(root);
        assert_eq!(t.find_action(&answers(&[("a", "a")])), Some("stop"));
        assert_eq!(t.find_action(&answers(&[])), Some("stop"));
    }

    #[test]
    fn node_count_includes_questions_and_leaves() {
        // rain, umbrella, walk under cover, wait indoors, go out
        assert_eq!(weather_tree().node_count(), 5);
        assert_eq!(DecisionTree::new(DecisionNode::action("only")).node_count(), 1);
        assert_eq!(weather_tree().root().attribute(), Some("rain"));
    }
}
