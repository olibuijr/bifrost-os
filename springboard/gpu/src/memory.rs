//! Device memory and buffers: dedicated allocations for big buffers, and a block sub-allocator for buffers under 256 KiB.
//!
//! A small buffer is a range of a shared block: one `VkBuffer` and one `VkDeviceMemory` of [`BLOCK_SIZE`] bytes that
//! serve every sub-allocation of their pool. Blocks are kept per (device, host-visible) pool and live while any buffer
//! of the pool does. Host-visible buffers (`STAGING`, `MAPPED`) are coherent and mapped for their whole lifetime, so a
//! write is a memcpy; device-local buffers have no CPU address.
//!
//! Owner: wave G1 agent `gpu-memory`.

use crate::{Error, Result};
use ash::vk;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

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

/// Buffers smaller than this are sub-allocated from a shared block; larger ones get their own memory.
pub(crate) const SUBALLOC_LIMIT: usize = 256 * 1024;
/// Size of one shared block (a pool grows by whole blocks).
pub(crate) const BLOCK_SIZE: u64 = 4 * 1024 * 1024;
/// Minimum alignment of every sub-allocated offset: the largest `minUniformBufferOffsetAlignment` and
/// `minStorageBufferOffsetAlignment` the Vulkan limits allow, so any buffer binding at a sub-allocated offset is valid.
const SUBALLOC_ALIGN: u64 = 256;

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    pub(crate) buffer: vk::Buffer,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) offset: u64,
    pub(crate) len: usize,
    /// Host pointer when the buffer is mapped, as an address (keeps `Buffer` `Send`).
    pub(crate) mapped: Option<usize>,
    /// For sub-allocated buffers: the pool, the block index and the reserved size, returned on drop.
    /// `None` for dedicated buffers, which free their own memory.
    pooled: Option<(Arc<Pool>, usize, u64)>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        match self.pooled.take() {
            Some((pool, block, size)) => pool.release(block, self.offset, size),
            None => {
                // SAFETY: a dedicated buffer and its memory are owned by this handle only. The GPU must not
                // still be using them: the command layer keeps every resource of an in-flight submission alive
                // (`CommandBuffer::keep`), so this runs only after the last use has completed.
                unsafe {
                    self.device.inner.device.destroy_buffer(self.buffer, None);
                }
                release_memory(&self.device, self.memory, self.mapped.is_some());
            }
        }
    }
}

/// A GPU buffer. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct Buffer {
    pub(crate) inner: Arc<Inner>,
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
        let base = self.host_base()?;
        check_range(self.inner.len, offset, bytes.len())?;
        // SAFETY: `base` is the start of this buffer's persistent host mapping, which the `Inner` keeps mapped for as
        // long as the buffer lives. The destination range `offset..offset + len` lies inside the buffer (checked just
        // above), and `bytes` is a live slice that cannot overlap memory the mapping hands out to the caller.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (base + offset) as *mut u8, bytes.len());
        }
        Ok(())
    }

    /// Reads `len` bytes from `offset` of a host-visible buffer.
    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let base = self.host_base()?;
        check_range(self.inner.len, offset, len)?;
        let mut out = vec![0u8; len];
        // SAFETY: the source range lies inside this buffer's persistent host mapping (checked above and kept mapped
        // by the `Inner`), and `out` is a fresh allocation of exactly `len` bytes, so the ranges do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping((base + offset) as *const u8, out.as_mut_ptr(), len);
        }
        Ok(out)
    }

    /// The host address of the buffer's first byte, for host-visible buffers only.
    fn host_base(&self) -> Result<usize> {
        self.inner.mapped.ok_or_else(|| Error::Invalid("buffer is device-local; CPU access needs a STAGING or MAPPED buffer".into()))
    }
}

/// Picks the memory type index for `type_bits` with `flags` (pure, unit-tested against fake tables).
pub fn memory_type_index(props: &ash::vk::PhysicalDeviceMemoryProperties, type_bits: u32, flags: ash::vk::MemoryPropertyFlags) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| type_bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(flags))
}

/// Rejects a `len`-byte range at `offset` that does not fit a buffer of `buffer_len` bytes (overflow included).
pub(crate) fn check_range(buffer_len: usize, offset: usize, len: usize) -> Result<()> {
    match offset.checked_add(len) {
        Some(end) if end <= buffer_len => Ok(()),
        _ => Err(Error::Invalid(format!("range {offset}+{len} is outside a buffer of {buffer_len} bytes"))),
    }
}

/// Whether a buffer with `usage` lives in host-visible memory.
pub(crate) fn is_host(usage: BufferUsage) -> bool {
    usage.0 & (BufferUsage::STAGING.0 | BufferUsage::MAPPED.0) != 0
}

/// The `VkBufferUsageFlags` for `usage`. Every buffer may be a copy source and destination.
pub(crate) fn vk_usage(usage: BufferUsage) -> vk::BufferUsageFlags {
    let mut flags = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST;
    if usage.contains(BufferUsage::VERTEX) {
        flags |= vk::BufferUsageFlags::VERTEX_BUFFER;
    }
    if usage.contains(BufferUsage::INDEX) {
        flags |= vk::BufferUsageFlags::INDEX_BUFFER;
    }
    if usage.contains(BufferUsage::UNIFORM) {
        flags |= vk::BufferUsageFlags::UNIFORM_BUFFER;
    }
    if usage.contains(BufferUsage::STORAGE) {
        flags |= vk::BufferUsageFlags::STORAGE_BUFFER;
    }
    flags
}

/// The usage of a shared block: every buffer kind a sub-allocation may be bound as.
fn block_usage() -> vk::BufferUsageFlags {
    vk_usage(BufferUsage::VERTEX | BufferUsage::INDEX | BufferUsage::UNIFORM | BufferUsage::STORAGE)
}

/// First-fit free list over one block's byte range. Pure bookkeeping, no Vulkan calls: a free range is
/// `start -> length`, ranges never overlap, and adjacent free ranges are always merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubAllocator {
    capacity: u64,
    free: BTreeMap<u64, u64>,
}

impl SubAllocator {
    pub(crate) fn new(capacity: u64) -> SubAllocator {
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        SubAllocator { capacity, free }
    }

    /// Total free bytes, not counting fragmentation.
    pub(crate) fn free_bytes(&self) -> u64 {
        self.free.values().sum()
    }

    /// Whether the whole block is free (a block that is empty can be released).
    pub(crate) fn is_unused(&self) -> bool {
        self.free_bytes() == self.capacity
    }

    /// Reserves `size` bytes at an offset that is a multiple of `align` (a power of two), first fit.
    /// `None` when `size` or `align` is zero or `align` is not a power of two, or when no free range fits.
    pub(crate) fn alloc(&mut self, size: u64, align: u64) -> Option<u64> {
        if size == 0 || align == 0 || !align.is_power_of_two() {
            return None;
        }
        let (start, len, aligned) = self.free.iter().find_map(|(&start, &len)| {
            let aligned = start.checked_add(align - 1)? & !(align - 1);
            let need_end = aligned.checked_add(size)?;
            (need_end <= start.checked_add(len)?).then_some((start, len, aligned))
        })?;
        self.free.remove(&start);
        if aligned > start {
            self.free.insert(start, aligned - start);
        }
        let tail = aligned + size;
        let end = start + len;
        if tail < end {
            self.free.insert(tail, end - tail);
        }
        Some(aligned)
    }

    /// Returns `size` bytes at `offset` (as reserved by `alloc`), merging with free neighbours.
    pub(crate) fn free(&mut self, offset: u64, size: u64) {
        debug_assert!(size > 0 && offset.checked_add(size).is_some_and(|end| end <= self.capacity), "free outside the block");
        let mut start = offset;
        let mut len = size;
        if let Some((&prev, &prev_len)) = self.free.range(..offset).next_back() {
            debug_assert!(prev + prev_len <= offset, "double free before {offset}");
            if prev + prev_len == offset {
                self.free.remove(&prev);
                start = prev;
                len += prev_len;
            }
        }
        if let Some((&next, &next_len)) = self.free.range(offset..).next() {
            debug_assert!(offset + size <= next, "double free at {offset}");
            if offset + size == next {
                self.free.remove(&next);
                len += next_len;
            }
        }
        self.free.insert(start, len);
    }
}

/// One shared block: a `VkBuffer` over the whole block, its memory and its free list.
struct Block {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    /// Base address of the block's persistent mapping (host pools only).
    mapped: Option<usize>,
    /// Alignment every sub-allocation in this block honours.
    align: u64,
    alloc: SubAllocator,
}

/// The blocks of one (device, host-visibility) pool.
pub(crate) struct Pool {
    device: crate::Device,
    device_id: usize,
    host: bool,
    blocks: Mutex<Vec<Block>>,
}

/// Where a sub-allocation landed.
struct Placed {
    block: usize,
    offset: u64,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: Option<usize>,
}

impl Pool {
    /// Reserves `size` bytes in the first block with room, adding a block when none has.
    fn place(&self, size: u64) -> Result<Placed> {
        let mut blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        let found = blocks.iter_mut().enumerate().find_map(|(i, b)| b.alloc.alloc(size, b.align).map(|offset| (i, offset)));
        let (index, offset) = match found {
            Some(found) => found,
            None => {
                let mut block = new_block(&self.device, self.host)?;
                let offset = block.alloc.alloc(size, block.align).ok_or_else(|| Error::Invalid(format!("{size} bytes do not fit a block")))?;
                blocks.push(block);
                (blocks.len() - 1, offset)
            }
        };
        let b = &blocks[index];
        Ok(Placed {
            block: index,
            offset,
            buffer: b.buffer,
            memory: b.memory,
            mapped: b.mapped.map(|base| base + offset as usize),
        })
    }

    fn release(&self, block: usize, offset: u64, size: u64) {
        let mut blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = blocks.get_mut(block) {
            b.alloc.free(offset, size);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Every buffer of the pool is gone (the pool is held by each of them), so no block is in use by a handle.
        let blocks = self.blocks.get_mut().unwrap_or_else(|e| e.into_inner());
        for b in blocks.drain(..) {
            // SAFETY: the buffer and memory were made by this device, are bound to each other, and no live handle
            // refers to them. The GPU must have finished with them (see `Inner::drop`).
            unsafe {
                self.device.inner.device.destroy_buffer(b.buffer, None);
            }
            release_memory(&self.device, b.memory, b.mapped.is_some());
        }
    }
}

/// Live pools, looked up by (device, host). A pool lives while a buffer holds it, so the registry holds weak handles.
static POOLS: Mutex<Vec<Weak<Pool>>> = Mutex::new(Vec::new());

fn pool_for(device: &crate::Device, host: bool) -> Arc<Pool> {
    // The device identity is the address of its shared state; the pool keeps that state alive, so a live pool for
    // this address is for this device.
    let device_id = Arc::as_ptr(&device.inner) as usize;
    let mut pools = POOLS.lock().unwrap_or_else(|e| e.into_inner());
    pools.retain(|w| w.strong_count() > 0);
    if let Some(pool) = pools.iter().filter_map(Weak::upgrade).find(|p| p.device_id == device_id && p.host == host) {
        return pool;
    }
    let pool = Arc::new(Pool {
        device: device.clone(),
        device_id,
        host,
        blocks: Mutex::new(Vec::new()),
    });
    pools.push(Arc::downgrade(&pool));
    pool
}

/// Creates a `VkBuffer` of `size` bytes with `flags`, exclusive to the device's queue family.
fn create_raw_buffer(device: &crate::Device, size: u64, flags: vk::BufferUsageFlags) -> Result<vk::Buffer> {
    let info = vk::BufferCreateInfo::default().size(size).usage(flags).sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: `info` is a complete create info for this device; the caller owns the returned handle and destroys it.
    unsafe { device.inner.device.create_buffer(&info, None) }.map_err(Error::from)
}

/// Allocates memory for `req`: host-visible and coherent, mapped whole, when `host`; device-local otherwise.
/// Returns the memory and its mapping. Nothing is left allocated on error.
fn allocate_memory(device: &crate::Device, req: vk::MemoryRequirements, host: bool) -> Result<(vk::DeviceMemory, Option<usize>)> {
    let props = &device.inner.memory_properties;
    let index = if host {
        let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        memory_type_index(props, req.memory_type_bits, flags)
            .ok_or_else(|| Error::Unsupported("no host-visible coherent memory type for buffers".into()))?
    } else {
        memory_type_index(props, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
            .or_else(|| memory_type_index(props, req.memory_type_bits, vk::MemoryPropertyFlags::empty()))
            .ok_or_else(|| Error::Unsupported("no memory type for device-local buffers".into()))?
    };
    let info = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(index);
    // SAFETY: `info` names a memory type of this device that `req` allows; the memory is released by `release_memory`.
    let memory = unsafe { device.inner.device.allocate_memory(&info, None) }.map_err(Error::from)?;
    if !host {
        return Ok((memory, None));
    }
    // SAFETY: the memory is host-visible and was just allocated, so it is not mapped yet; it is mapped whole and stays
    // mapped until `release_memory` unmaps it. On failure the memory is freed before returning.
    match unsafe { device.inner.device.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()) } {
        Ok(ptr) => Ok((memory, Some(ptr as usize))),
        Err(e) => {
            release_memory(device, memory, false);
            Err(Error::from(e))
        }
    }
}

/// Allocates and binds memory for `buffer` at offset 0. Returns the memory, its mapping and the buffer's alignment.
/// On failure the memory is freed again; the buffer is still the caller's to destroy.
fn back_buffer(device: &crate::Device, buffer: vk::Buffer, host: bool) -> Result<(vk::DeviceMemory, Option<usize>, u64)> {
    // SAFETY: `buffer` is a live buffer of this device.
    let req = unsafe { device.inner.device.get_buffer_memory_requirements(buffer) };
    let (memory, mapped) = allocate_memory(device, req, host)?;
    // SAFETY: `buffer` is unbound and `memory` is at least `req.size` bytes from a type `req` allows, at offset 0.
    if let Err(e) = unsafe { device.inner.device.bind_buffer_memory(buffer, memory, 0) } {
        release_memory(device, memory, mapped.is_some());
        return Err(Error::from(e));
    }
    Ok((memory, mapped, req.alignment))
}

/// Unmaps (when mapped) and frees `memory`.
fn release_memory(device: &crate::Device, memory: vk::DeviceMemory, mapped: bool) {
    // SAFETY: `memory` was allocated by this device and is no longer bound to a live buffer (callers guarantee it);
    // it is unmapped before it is freed.
    unsafe {
        if mapped {
            device.inner.device.unmap_memory(memory);
        }
        device.inner.device.free_memory(memory, None);
    }
}

/// A new shared block for `host` or device-local sub-allocations.
fn new_block(device: &crate::Device, host: bool) -> Result<Block> {
    let buffer = create_raw_buffer(device, BLOCK_SIZE, block_usage())?;
    match back_buffer(device, buffer, host) {
        Ok((memory, mapped, alignment)) => Ok(Block {
            buffer,
            memory,
            mapped,
            align: alignment.max(SUBALLOC_ALIGN),
            alloc: SubAllocator::new(BLOCK_SIZE),
        }),
        Err(e) => {
            // SAFETY: the buffer was just created here and is not used anywhere.
            unsafe { device.inner.device.destroy_buffer(buffer, None) };
            Err(e)
        }
    }
}

/// A buffer of `len` bytes for `usage`: sub-allocated under [`SUBALLOC_LIMIT`], dedicated otherwise.
/// Called by `Device::new_buffer`.
pub(crate) fn new_buffer(device: &crate::Device, len: usize, usage: BufferUsage) -> Result<Buffer> {
    if len == 0 {
        return Err(Error::Invalid("buffer size is zero".into()));
    }
    let host = is_host(usage);
    let inner = if len < SUBALLOC_LIMIT {
        let pool = pool_for(device, host);
        let size = len as u64;
        let placed = pool.place(size)?;
        Inner {
            device: device.clone(),
            buffer: placed.buffer,
            memory: placed.memory,
            offset: placed.offset,
            len,
            mapped: placed.mapped,
            pooled: Some((pool, placed.block, size)),
        }
    } else {
        let buffer = create_raw_buffer(device, len as u64, vk_usage(usage))?;
        match back_buffer(device, buffer, host) {
            Ok((memory, mapped, _)) => Inner {
                device: device.clone(),
                buffer,
                memory,
                offset: 0,
                len,
                mapped,
                pooled: None,
            },
            Err(e) => {
                // SAFETY: the buffer was just created here and is not used anywhere.
                unsafe { device.inner.device.destroy_buffer(buffer, None) };
                return Err(e);
            }
        }
    };
    Ok(Buffer { inner: Arc::new(inner) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(types: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
        let mut p = vk::PhysicalDeviceMemoryProperties::default();
        p.memory_type_count = types.len() as u32;
        for (i, f) in types.iter().enumerate() {
            p.memory_types[i].property_flags = *f;
        }
        p
    }

    fn host_flags() -> vk::MemoryPropertyFlags {
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
    }

    #[test]
    fn memory_type_index_picks_the_first_allowed_type_with_all_flags() {
        let p = props(&[vk::MemoryPropertyFlags::DEVICE_LOCAL, host_flags(), vk::MemoryPropertyFlags::DEVICE_LOCAL | host_flags()]);
        assert_eq!(memory_type_index(&p, 0b110, host_flags()), Some(1));
        assert_eq!(memory_type_index(&p, 0b110, vk::MemoryPropertyFlags::DEVICE_LOCAL), Some(2));
        assert_eq!(memory_type_index(&p, 0b001, vk::MemoryPropertyFlags::DEVICE_LOCAL), Some(0));
        assert_eq!(memory_type_index(&p, 0b001, host_flags()), None);
        assert_eq!(memory_type_index(&p, 0b1000, vk::MemoryPropertyFlags::DEVICE_LOCAL), None);
    }

    #[test]
    fn sub_allocator_places_first_fit_and_aligns_offsets() {
        let mut sa = SubAllocator::new(1024);
        assert_eq!(sa.alloc(10, 1), Some(0));
        assert_eq!(sa.alloc(10, 256), Some(256));
        // The gap 10..256 is still free and the first fit at 10.
        assert_eq!(sa.alloc(10, 1), Some(10));
        assert_eq!(sa.free_bytes(), 1024 - 30);
    }

    #[test]
    fn sub_allocator_free_coalesces_both_neighbours_back_to_one_range() {
        let mut sa = SubAllocator::new(1024);
        assert_eq!(sa.alloc(100, 1), Some(0)); // free: {100: 924}
        assert_eq!(sa.alloc(100, 256), Some(256)); // free: {100: 156, 356: 668}
        assert_eq!(sa.free, BTreeMap::from([(100, 156), (356, 668)]));
        sa.free(256, 100); // merges left (100..256) and right (356..): {100: 924}
        assert_eq!(sa.free, BTreeMap::from([(100, 924)]));
        sa.free(0, 100); // merges right into the whole block: {0: 1024}
        assert_eq!(sa.free, BTreeMap::from([(0, 1024)]));
        assert!(sa.is_unused());
    }

    #[test]
    fn sub_allocator_rejects_bad_arguments_and_reports_exhaustion() {
        let mut sa = SubAllocator::new(512);
        assert_eq!(sa.alloc(0, 1), None);
        assert_eq!(sa.alloc(8, 0), None);
        assert_eq!(sa.alloc(8, 3), None, "alignment must be a power of two");
        assert_eq!(sa.alloc(513, 1), None, "larger than the block");
        assert_eq!(sa.alloc(256, 1), Some(0));
        assert_eq!(sa.alloc(256, 1), Some(256));
        assert_eq!(sa.alloc(1, 1), None, "block is full");
        assert_eq!(sa.free_bytes(), 0);
        sa.free(0, 256);
        assert_eq!(sa.alloc(256, 256), Some(0), "freed range is reused");
    }

    #[test]
    fn sub_allocator_leaves_the_alignment_gap_free_with_exact_arithmetic() {
        let mut sa = SubAllocator::new(1024);
        assert_eq!(sa.alloc(1, 1), Some(0)); // free: {1: 1023}
        assert_eq!(sa.alloc(8, 512), Some(512)); // gap 1..512 kept (511), tail 520..1024 kept (504)
        assert_eq!(sa.free, BTreeMap::from([(1, 511), (520, 504)]));
        assert_eq!(sa.free_bytes(), 1015);
        assert!(!sa.is_unused());
    }

    #[test]
    fn range_check_accepts_exact_fit_and_rejects_overflow() {
        assert!(check_range(16, 0, 16).is_ok());
        assert!(check_range(16, 8, 8).is_ok());
        assert!(check_range(16, 8, 9).is_err());
        assert!(check_range(16, usize::MAX, 2).is_err(), "offset + len must not overflow");
        assert!(check_range(0, 0, 1).is_err());
    }

    #[test]
    fn host_usage_is_staging_or_mapped() {
        assert!(is_host(BufferUsage::STAGING));
        assert!(is_host(BufferUsage::VERTEX | BufferUsage::MAPPED));
        assert!(!is_host(BufferUsage::VERTEX | BufferUsage::STORAGE));
    }

    #[test]
    fn vulkan_usage_maps_each_kind_and_always_allows_copies() {
        let u = vk_usage(BufferUsage::UNIFORM);
        assert!(u.contains(vk::BufferUsageFlags::UNIFORM_BUFFER));
        assert!(!u.contains(vk::BufferUsageFlags::STORAGE_BUFFER));
        assert!(u.contains(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST));
        let s = vk_usage(BufferUsage::STAGING);
        assert!(!s.contains(vk::BufferUsageFlags::VERTEX_BUFFER));
        assert!(block_usage().contains(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER));
    }

    #[test]
    fn sub_allocations_fit_inside_a_block_and_the_limit_is_below_it() {
        assert!(SUBALLOC_LIMIT as u64 * 4 <= BLOCK_SIZE);
        assert!(SUBALLOC_ALIGN <= 256);
        let mut sa = SubAllocator::new(BLOCK_SIZE);
        let n = BLOCK_SIZE / SUBALLOC_LIMIT as u64;
        for _ in 0..n {
            assert!(sa.alloc(SUBALLOC_LIMIT as u64, SUBALLOC_ALIGN).is_some());
        }
        assert!(!sa.is_unused());
        assert_eq!(sa.free_bytes(), 0);
    }

    #[cfg(feature = "gpu-tests")]
    mod gpu_tests {
        use super::*;

        #[test]
        fn small_host_buffers_round_trip_and_share_a_block() {
            let dev = crate::Device::open(None).expect("device");
            let a = dev.new_buffer(64, BufferUsage::MAPPED).unwrap();
            let b = dev.new_buffer(64, BufferUsage::MAPPED).unwrap();
            a.write(0, &[1, 2, 3]).unwrap();
            b.write(0, &[9]).unwrap();
            assert_eq!(a.read(0, 3).unwrap(), vec![1, 2, 3]);
            assert_eq!(b.read(0, 1).unwrap(), vec![9]);
            assert!(a.write(60, &[0; 8]).is_err(), "write past the end is refused");
            assert_eq!(a.inner.memory, b.inner.memory, "two small buffers share one block");
        }

        #[test]
        fn device_local_buffer_refuses_cpu_access() {
            let dev = crate::Device::open(None).expect("device");
            let v = dev.new_buffer(64, BufferUsage::VERTEX).unwrap();
            assert!(v.read(0, 1).is_err());
            assert!(v.write(0, &[1]).is_err());
        }

        #[test]
        fn large_buffer_is_dedicated_and_round_trips() {
            let dev = crate::Device::open(None).expect("device");
            let big = dev.new_buffer(512 * 1024, BufferUsage::STAGING).unwrap();
            let small = dev.new_buffer(64, BufferUsage::STAGING).unwrap();
            assert_ne!(big.inner.memory, small.inner.memory);
            big.write(512 * 1024 - 4, &[7, 7, 7, 7]).unwrap();
            assert_eq!(big.read(512 * 1024 - 4, 4).unwrap(), vec![7, 7, 7, 7]);
        }
    }
}
