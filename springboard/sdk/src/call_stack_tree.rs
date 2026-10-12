//! Call stacks captured with a diagnostic report, as a pure model. A [`CallStackTree`] holds one or more threads, each
//! with a forest of [`CallStackFrame`]s. Samples that share a prefix of frames share those nodes, and every node counts
//! the samples that passed through it. The binary images the frames point into are kept once in a table keyed by
//! UUID, so a frame stores only the UUID.
//!
//! The model reads no stack and no clock. The caller supplies each sampled stack, root first. Nothing here is
//! symbolicated: names come from the binary table only.

use std::collections::BTreeMap;

/// One binary image referenced by the frames, keyed by its UUID in [`CallStackTree::binary_info`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryInfo {
    pub name: String,
    pub load_address: u64,
}

/// One frame of a sampled stack, root first in the slice given to [`CallStackTree::record_sample`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampledFrame {
    pub binary_uuid: String,
    pub address: u64,
    pub offset_into_binary_text_segment: u64,
}

/// A node of the call tree. A frame matches an earlier one when both the binary UUID and the offset into its text
/// segment are equal; the first address seen is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallStackFrame {
    pub binary_uuid: String,
    pub address: u64,
    pub offset_into_binary_text_segment: u64,
    /// Samples whose stack passed through this frame.
    pub sample_count: u32,
    pub sub_frames: Vec<CallStackFrame>,
}

/// The root frames of one thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallStackThread {
    pub root_frames: Vec<CallStackFrame>,
}

/// A set of sampled call stacks, one tree per thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallStackTree {
    /// True when the stacks are grouped by thread, false when all threads share one list.
    pub call_stack_per_thread: bool,
    pub threads: Vec<CallStackThread>,
    pub binary_info: BTreeMap<String, BinaryInfo>,
}

fn add_path(level: &mut Vec<CallStackFrame>, path: &[SampledFrame], count: u32) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    let index = match level.iter().position(|f| {
        f.binary_uuid == first.binary_uuid && f.offset_into_binary_text_segment == first.offset_into_binary_text_segment
    }) {
        Some(i) => i,
        None => {
            level.push(CallStackFrame {
                binary_uuid: first.binary_uuid.clone(),
                address: first.address,
                offset_into_binary_text_segment: first.offset_into_binary_text_segment,
                sample_count: 0,
                sub_frames: Vec::new(),
            });
            level.len() - 1
        }
    };
    let node = &mut level[index];
    node.sample_count = node.sample_count.saturating_add(count);
    add_path(&mut node.sub_frames, rest, count);
}

fn walk<F: FnMut(&CallStackFrame)>(frames: &[CallStackFrame], visit: &mut F) {
    for frame in frames {
        visit(frame);
        walk(&frame.sub_frames, visit);
    }
}

fn frame_json(frame: &CallStackFrame, out: &mut String) {
    out.push_str("{\"binaryUUID\":");
    out.push_str(&bifrost_core::json::quote(&frame.binary_uuid));
    out.push_str(&format!(
        ",\"address\":{},\"offsetIntoBinaryTextSegment\":{},\"sampleCount\":{},\"subFrames\":[",
        frame.address, frame.offset_into_binary_text_segment, frame.sample_count
    ));
    for (i, sub) in frame.sub_frames.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        frame_json(sub, out);
    }
    out.push_str("]}");
}

impl CallStackTree {
    pub fn new(call_stack_per_thread: bool) -> CallStackTree {
        CallStackTree { call_stack_per_thread, threads: Vec::new(), binary_info: BTreeMap::new() }
    }

    /// Adds a binary image. The first entry for a UUID wins, so a repeated image does not replace its name.
    pub fn add_binary(&mut self, uuid: &str, name: &str, load_address: u64) {
        self.binary_info
            .entry(uuid.to_string())
            .or_insert_with(|| BinaryInfo { name: name.to_string(), load_address });
    }

    /// Adds `count` samples of `stack` (root first) to thread `thread`, creating empty threads up to that index. An
    /// empty stack changes nothing. Counts saturate at `u32::MAX`.
    pub fn record_sample(&mut self, thread: usize, stack: &[SampledFrame], count: u32) {
        if stack.is_empty() {
            return;
        }
        while self.threads.len() <= thread {
            self.threads.push(CallStackThread::default());
        }
        add_path(&mut self.threads[thread].root_frames, stack, count);
    }

    /// The number of samples recorded, counted at the root frames of every thread.
    pub fn total_samples(&self) -> u64 {
        self.threads
            .iter()
            .flat_map(|t| t.root_frames.iter())
            .map(|f| f.sample_count as u64)
            .sum()
    }

    /// Calls `visit` on every frame, depth first, parents before children, threads in order.
    pub fn for_each_frame<F: FnMut(&CallStackFrame)>(&self, mut visit: F) {
        for thread in &self.threads {
            walk(&thread.root_frames, &mut visit);
        }
    }

    /// The name of the binary a frame points into, or `None` when the table has no entry for its UUID.
    pub fn binary_name(&self, frame: &CallStackFrame) -> Option<&str> {
        self.binary_info.get(&frame.binary_uuid).map(|b| b.name.as_str())
    }

    /// The tree as compact JSON. Key names follow the property names of the official reference: `callStackPerThread`,
    /// `callStackThreads`, `binaryUUID`, `offsetIntoBinaryTextSegment`, `sampleCount`, `subFrames` and `address`. The
    /// per-thread root list is written as `callStackRootFrames`, and the binary table as `binaryInfo`.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{{\"callStackPerThread\":{},\"callStackThreads\":[", self.call_stack_per_thread));
        for (i, thread) in self.threads.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"callStackRootFrames\":[");
            for (j, frame) in thread.root_frames.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                frame_json(frame, &mut out);
            }
            out.push_str("]}");
        }
        out.push_str("],\"binaryInfo\":{");
        for (i, (uuid, info)) in self.binary_info.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&bifrost_core::json::quote(uuid));
            out.push_str(":{\"name\":");
            out.push_str(&bifrost_core::json::quote(&info.name));
            out.push_str(&format!(",\"loadAddress\":{}}}", info.load_address));
        }
        out.push_str("}}");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(uuid: &str, offset: u64) -> SampledFrame {
        SampledFrame { binary_uuid: uuid.to_string(), address: 0x1000 + offset, offset_into_binary_text_segment: offset }
    }

    #[test]
    fn samples_that_share_a_prefix_share_nodes() {
        let mut t = CallStackTree::new(false);
        let a = [frame("u", 1), frame("u", 2), frame("u", 3)];
        let b = [frame("u", 1), frame("u", 2), frame("u", 4)];
        t.record_sample(0, &a, 2);
        t.record_sample(0, &b, 3);
        let root = &t.threads[0].root_frames;
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].sample_count, 5);
        let mid = &root[0].sub_frames[0];
        assert_eq!(mid.offset_into_binary_text_segment, 2);
        assert_eq!(mid.sample_count, 5);
        let leaves: Vec<(u64, u32)> =
            mid.sub_frames.iter().map(|f| (f.offset_into_binary_text_segment, f.sample_count)).collect();
        assert_eq!(leaves, vec![(3, 2), (4, 3)]);
        assert_eq!(t.total_samples(), 5);
    }

    #[test]
    fn threads_are_created_up_to_the_index_and_empty_stacks_change_nothing() {
        let mut t = CallStackTree::new(true);
        t.record_sample(2, &[frame("u", 9)], 1);
        assert_eq!(t.threads.len(), 3);
        assert!(t.threads[0].root_frames.is_empty());
        assert!(t.threads[1].root_frames.is_empty());
        assert_eq!(t.threads[2].root_frames[0].sample_count, 1);
        t.record_sample(0, &[], 7);
        assert!(t.threads[0].root_frames.is_empty());
        assert_eq!(t.total_samples(), 1);
    }

    #[test]
    fn depth_first_walk_visits_parents_before_children() {
        let mut t = CallStackTree::new(false);
        t.record_sample(0, &[frame("u", 1), frame("u", 2)], 1);
        t.record_sample(0, &[frame("u", 1), frame("u", 3)], 1);
        t.record_sample(0, &[frame("u", 7)], 1);
        let mut seen = Vec::new();
        t.for_each_frame(|f| seen.push(f.offset_into_binary_text_segment));
        assert_eq!(seen, vec![1, 2, 3, 7]);
    }

    #[test]
    fn the_first_binary_entry_wins_and_names_resolve() {
        let mut t = CallStackTree::new(false);
        t.add_binary("AB", "server", 0x4000);
        t.add_binary("AB", "other", 0x9000);
        t.record_sample(0, &[frame("AB", 5)], 1);
        let f = &t.threads[0].root_frames[0];
        assert_eq!(t.binary_name(f), Some("server"));
        assert_eq!(t.binary_info["AB"].load_address, 0x4000);
        let unknown = SampledFrame { binary_uuid: "ZZ".into(), address: 0, offset_into_binary_text_segment: 0 };
        t.record_sample(0, &[unknown], 1);
        assert_eq!(t.binary_name(&t.threads[0].root_frames[1]), None);
    }

    #[test]
    fn counts_saturate_instead_of_wrapping() {
        let mut t = CallStackTree::new(false);
        t.record_sample(0, &[frame("u", 1)], u32::MAX);
        t.record_sample(0, &[frame("u", 1)], 5);
        assert_eq!(t.threads[0].root_frames[0].sample_count, u32::MAX);
    }

    #[test]
    fn json_lists_threads_frames_and_binaries() {
        let mut t = CallStackTree::new(false);
        t.add_binary("AB", "srv", 16);
        t.record_sample(0, &[frame("AB", 1), frame("AB", 2)], 4);
        let want = concat!(
            "{\"callStackPerThread\":false,\"callStackThreads\":[{\"callStackRootFrames\":[",
            "{\"binaryUUID\":\"AB\",\"address\":4097,\"offsetIntoBinaryTextSegment\":1,\"sampleCount\":4,",
            "\"subFrames\":[{\"binaryUUID\":\"AB\",\"address\":4098,\"offsetIntoBinaryTextSegment\":2,",
            "\"sampleCount\":4,\"subFrames\":[]}]}]}],\"binaryInfo\":{\"AB\":{\"name\":\"srv\",\"loadAddress\":16}}}"
        );
        assert_eq!(t.to_json(), want);
    }
}
