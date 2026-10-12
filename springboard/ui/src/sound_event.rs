//! Sound event trees: a sound is a tree of nodes. A container plays all its children, a switch plays the one child whose
//! key matches a value, a random node picks one child by weight and a blend plays its children at gains read from
//! envelopes over a value. `play` walks the tree and returns the leaf sounds with their gains. Pure Rust: the caller
//! supplies the switch key, the random roll (0.0..1.0) and the blend value, so the result is deterministic.

use crate::sound_envelope::Envelope;

/// One sound that is audible, with its gain after every node on the way down.
#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    pub sound: String,
    pub gain: f64,
}

/// A node in a sound event tree.
#[derive(Clone, Debug, PartialEq)]
pub enum EventNode {
    /// A leaf: one sound, by identifier.
    Sound(String),
    /// Plays every child.
    Container(Vec<EventNode>),
    /// Plays the child whose key equals `value`; nothing plays when no key matches.
    Switch { value: String, cases: Vec<(String, EventNode)> },
    /// Plays one child chosen by weight. `roll` in 0.0..1.0 picks where the weights are cut; a roll of 1.0 or more
    /// picks the last child.
    Random { roll: f64, options: Vec<(f64, EventNode)> },
    /// Plays each child at the gain its envelope gives for `value`. A child whose gain is zero is not played.
    Blend { value: f64, children: Vec<(Envelope, EventNode)> },
}

impl EventNode {
    /// The audible sounds in this tree, in tree order, with their gains. A gain of 0.0 or less is not reported.
    pub fn play(&self) -> Vec<Playback> {
        let mut out = Vec::new();
        self.collect(1.0, &mut out);
        out
    }

    fn collect(&self, gain: f64, out: &mut Vec<Playback>) {
        if gain <= 0.0 {
            return;
        }
        match self {
            Self::Sound(name) => out.push(Playback { sound: name.clone(), gain }),
            Self::Container(children) => {
                for child in children {
                    child.collect(gain, out);
                }
            }
            Self::Switch { value, cases } => {
                if let Some((_, node)) = cases.iter().find(|(key, _)| key == value) {
                    node.collect(gain, out);
                }
            }
            Self::Random { roll, options } => {
                let total: f64 = options.iter().map(|(w, _)| w.max(0.0)).sum();
                if total <= 0.0 {
                    return;
                }
                let target = roll.clamp(0.0, 1.0) * total;
                let mut acc = 0.0;
                let mut chosen = None;
                for (w, node) in options {
                    acc += w.max(0.0);
                    if target < acc {
                        chosen = Some(node);
                        break;
                    }
                }
                // A roll of exactly 1.0 lands on the boundary after the last option; take the last one.
                let chosen = chosen.or_else(|| options.last().map(|(_, n)| n));
                if let Some(node) = chosen {
                    node.collect(gain, out);
                }
            }
            Self::Blend { value, children } => {
                for (envelope, node) in children {
                    let g = envelope.evaluate(*value).clamp(0.0, 1.0) * gain;
                    node.collect(g, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_envelope::{CurveType, EnvelopeSegment};

    fn leaf(name: &str) -> EventNode {
        EventNode::Sound(name.to_string())
    }

    fn ramp(from: f64, to: f64) -> Envelope {
        Envelope::new((0.0, from), vec![EnvelopeSegment { end_x: 1.0, end_y: to, curve: CurveType::Linear }]).unwrap()
    }

    fn names(list: &[Playback]) -> Vec<&str> {
        list.iter().map(|p| p.sound.as_str()).collect()
    }

    #[test]
    fn container_plays_every_child_in_order_at_full_gain() {
        let tree = EventNode::Container(vec![leaf("a"), leaf("b")]);
        let out = tree.play();
        assert_eq!(names(&out), vec!["a", "b"]);
        assert!(out.iter().all(|p| p.gain == 1.0));
        assert!(EventNode::Container(Vec::new()).play().is_empty());
    }

    #[test]
    fn switch_plays_only_the_matching_case() {
        let tree = EventNode::Switch {
            value: "hit".to_string(),
            cases: vec![("miss".to_string(), leaf("thud")), ("hit".to_string(), leaf("crack"))],
        };
        assert_eq!(names(&tree.play()), vec!["crack"]);
        let none = EventNode::Switch { value: "other".to_string(), cases: vec![("hit".to_string(), leaf("crack"))] };
        assert!(none.play().is_empty(), "no matching key plays nothing");
    }

    #[test]
    fn random_chooses_by_weight_from_the_roll() {
        // Weights 1 and 3 make a total of 4: the cut is at 1 for "a" and 1..4 for "b".
        let options = vec![(1.0, leaf("a")), (3.0, leaf("b"))];
        let low = EventNode::Random { roll: 0.2, options: options.clone() };
        let high = EventNode::Random { roll: 0.5, options: options.clone() };
        let last = EventNode::Random { roll: 1.0, options };
        assert_eq!(names(&low.play()), vec!["a"], "0.2 * 4 = 0.8 is below the first weight");
        assert_eq!(names(&high.play()), vec!["b"], "0.5 * 4 = 2.0 is past the first weight");
        assert_eq!(names(&last.play()), vec!["b"], "a roll of 1.0 falls back to the last option");
    }

    #[test]
    fn blend_plays_each_child_at_its_envelope_gain() {
        let tree = EventNode::Blend {
            value: 0.25,
            children: vec![(ramp(0.0, 1.0), leaf("a")), (ramp(1.0, 0.0), leaf("b"))],
        };
        let out = tree.play();
        assert_eq!(names(&out), vec!["a", "b"]);
        assert!((out[0].gain - 0.25).abs() < 1e-9, "a ramps up to 0.25 at 0.25");
        assert!((out[1].gain - 0.75).abs() < 1e-9, "b ramps down to 0.75 at 0.25");
    }

    #[test]
    fn gains_multiply_down_the_tree_and_silent_branches_are_dropped() {
        // A container at 0.5 (from a blend child whose envelope is 0.5 at this value) holds a child whose envelope is 0.
        let silent = ramp(0.0, 0.0);
        let tree = EventNode::Blend {
            value: 0.5,
            children: vec![
                (ramp(0.0, 1.0), EventNode::Container(vec![leaf("a"), leaf("quiet")])),
                (silent.clone(), leaf("never")),
            ],
        };
        let out = tree.play();
        assert_eq!(names(&out), vec!["a", "quiet"]);
        assert!(out.iter().all(|p| (p.gain - 0.5).abs() < 1e-9));
        // A zero envelope gain is not reported at all.
        let zero = EventNode::Blend { value: 0.5, children: vec![(silent, leaf("never"))] };
        assert!(zero.play().is_empty());
    }
}
