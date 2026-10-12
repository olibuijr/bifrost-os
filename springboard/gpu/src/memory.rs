//! Device memory and buffers: dedicated allocations for textures, a block sub-allocator for small buffers.
//!
//! Owner: wave G1 agent `gpu-memory`.

use crate::Result;

/// What a buffer is used for (combine with `|`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BufferUsage(pub u32);

impl BufferUsage {
    pub const VERTEX: BufferUsage = BufferUsage(1);
    pub const INDEX: BufferUsage = BufferUsage(2);
    pub const UNIFORM: BufferUsage = BufferUsage(4);
    pub const STORAGE: BufferUsage = BufferUsage(8);
    /// Upload or readback staging; always host-visible.
    pub const STAGING: BufferUsage = BufferUsage(16);
    /// Host-visible and mapped for the buffer's lifetime (per-frame instance data).
    pub const MAPPED: BufferUsage = BufferUsage(32);

    pub fn contains(self, other: BufferUsage) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for BufferUsage {
    type Output = BufferUsage;
    fn bitor(self, rhs: BufferUsage) -> BufferUsage {
        BufferUsage(self.0 | rhs.0)
    }
}

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    pub(crate) buffer: ash::vk::Buffer,
    pub(crate) memory: ash::vk::DeviceMemory,
    pub(crate) offset: u64,
    pub(crate) len: usize,
    pub(crate) usage: BufferUsage,
    /// Host pointer when the buffer is mapped, as an address (keeps `Buffer` `Send`).
    pub(crate) mapped: Option<usize>,
}

/// A GPU buffer. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct Buffer {
    pub(crate) inner: std::sync::Arc<Inner>,
}

impl Buffer {
    pub fn len(&self) -> usize {
        self.inner.len
    }

    pub fn is_empty(&self) -> bool {
        self.inner.len == 0
    }

    /// Copies `bytes` to `offset` of a host-visible buffer. Fails on a device-local buffer or out of range.
    pub fn write(&self, offset: usize, bytes: &[u8]) -> Result<()> {
        let _ = (offset, bytes);
        todo!("gpu-memory")
    }

    /// Reads `len` bytes from `offset` of a host-visible buffer.
    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let _ = (offset, len);
        todo!("gpu-memory")
    }
}

/// Picks the memory type index for `type_bits` with `flags` (pure, unit-tested against fake tables).
pub fn memory_type_index(props: &ash::vk::PhysicalDeviceMemoryProperties, type_bits: u32, flags: ash::vk::MemoryPropertyFlags) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| type_bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(flags))
}
