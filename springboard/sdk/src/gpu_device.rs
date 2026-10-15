//! GPU device inventory: which render nodes this machine has and which one applications get by default.
//!
//! A render node is a kernel device named `renderD<N>`, with N from 128 up. The shell opens one of them for presentation.
//! This model lets an application ask for the same device without opening it: the caller lists the device directory and
//! passes the entry names in, so the model holds no file handles and reads no directory itself.

/// The lowest minor number the kernel gives a render node.
pub const FIRST_RENDER_MINOR: u32 = 128;

/// One render node, identified by its minor number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenderNode {
    pub minor: u32,
}

impl RenderNode {
    /// The device file of this node.
    pub fn path(&self) -> String {
        format!("/dev/dri/renderD{}", self.minor)
    }

    /// Reads a directory entry name such as `renderD128`. Any other name, including a short or non-decimal number,
    /// gives None.
    pub fn from_entry_name(name: &str) -> Option<RenderNode> {
        let digits = name.strip_prefix("renderD")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let minor: u32 = digits.parse().ok()?;
        (minor >= FIRST_RENDER_MINOR).then_some(RenderNode { minor })
    }
}

/// The GPU render nodes of this machine, in minor-number order. The first node is the default device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuInventory {
    nodes: Vec<RenderNode>,
}

impl GpuInventory {
    /// Builds the inventory from directory entry names. Entries that are not render nodes are skipped, and a node
    /// listed twice counts once.
    pub fn from_entry_names<'a>(names: impl IntoIterator<Item = &'a str>) -> GpuInventory {
        let mut nodes: Vec<RenderNode> = names.into_iter().filter_map(RenderNode::from_entry_name).collect();
        nodes.sort();
        nodes.dedup();
        GpuInventory { nodes }
    }

    /// Every render node, lowest minor number first.
    pub fn devices(&self) -> &[RenderNode] {
        &self.nodes
    }

    /// The system default device: the lowest render node. None when the machine has no render node.
    pub fn default_device(&self) -> Option<RenderNode> {
        self.nodes.first().copied()
    }
}
