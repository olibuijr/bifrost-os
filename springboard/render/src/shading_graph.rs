//! A small shading-graph evaluator: a straight-line program of scalar nodes, the smallest useful
//! piece of a MaterialX-style node graph. Every node reads only earlier nodes, so one pass in order
//! evaluates the graph, and the result is the value of the last node.

/// One node. Operands are indices of earlier nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Node {
    Constant(f32),
    /// An entry of the input slice; a missing input reads 0.
    Input(usize),
    Add(usize, usize),
    Multiply(usize, usize),
    /// `a + (b - a) * t`
    Mix {
        a: usize,
        b: usize,
        t: usize,
    },
    /// `value` clamped to `lo..=hi`.
    Clamp {
        value: usize,
        lo: f32,
        hi: f32,
    },
}

/// A validated graph: non-empty, every operand refers to an earlier node, clamp bounds ordered.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    nodes: Vec<Node>,
}

impl Graph {
    /// The graph for `nodes`, or `None` when it is empty or breaks one of the rules above.
    pub fn new(nodes: Vec<Node>) -> Option<Self> {
        if nodes.is_empty() {
            return None;
        }
        for (i, node) in nodes.iter().enumerate() {
            let ok = match *node {
                Node::Constant(_) | Node::Input(_) => true,
                Node::Add(a, b) | Node::Multiply(a, b) => a < i && b < i,
                Node::Mix { a, b, t } => a < i && b < i && t < i,
                Node::Clamp { value, lo, hi } => value < i && lo <= hi,
            };
            if !ok {
                return None;
            }
        }
        Some(Graph { nodes })
    }

    /// Evaluates every node against `inputs` and returns the value of the last one.
    pub fn eval(&self, inputs: &[f32]) -> f32 {
        let mut v: Vec<f32> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let x = match *node {
                Node::Constant(c) => c,
                Node::Input(i) => inputs.get(i).copied().unwrap_or(0.0),
                Node::Add(a, b) => v[a] + v[b],
                Node::Multiply(a, b) => v[a] * v[b],
                Node::Mix { a, b, t } => v[a] + (v[b] - v[a]) * v[t],
                Node::Clamp { value, lo, hi } => v[value].clamp(lo, hi),
            };
            v.push(x);
        }
        v[v.len() - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_a_linear_expression() {
        // input0 * 2 + 1
        let g = Graph::new(vec![
            Node::Input(0),
            Node::Constant(2.0),
            Node::Multiply(0, 1),
            Node::Constant(1.0),
            Node::Add(2, 3),
        ])
        .unwrap();
        assert_eq!(g.eval(&[3.0]), 7.0);
    }

    #[test]
    fn mix_interpolates() {
        let g = Graph::new(vec![
            Node::Constant(0.0),
            Node::Constant(10.0),
            Node::Constant(0.25),
            Node::Mix { a: 0, b: 1, t: 2 },
        ])
        .unwrap();
        assert_eq!(g.eval(&[]), 2.5);
    }

    #[test]
    fn clamp_limits_the_value() {
        let g = Graph::new(vec![
            Node::Input(0),
            Node::Clamp {
                value: 0,
                lo: 0.0,
                hi: 1.0,
            },
        ])
        .unwrap();
        assert_eq!(g.eval(&[5.0]), 1.0);
        assert_eq!(g.eval(&[-2.0]), 0.0);
    }

    #[test]
    fn missing_input_reads_zero() {
        let g = Graph::new(vec![Node::Input(3)]).unwrap();
        assert_eq!(g.eval(&[]), 0.0);
    }

    #[test]
    fn invalid_graphs_are_refused() {
        assert!(Graph::new(Vec::new()).is_none());
        assert!(Graph::new(vec![Node::Add(0, 1)]).is_none());
        assert!(Graph::new(vec![
            Node::Constant(0.0),
            Node::Clamp {
                value: 0,
                lo: 1.0,
                hi: 0.0
            }
        ])
        .is_none());
    }
}
