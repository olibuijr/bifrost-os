//! Backing stores for the direct path (docs/graphics-architecture.md, "SDK"): each layer that draws owns a small pool of
//! exportable `bifrost_gpu` textures. Each texture is exported as a dma-buf and wrapped as a `wl_buffer` with
//! linux-dmabuf `create_immed`; the layer binds a buffer to its contents id and commits it. When the compositor
//! releases a buffer its slot returns to the pool.
//!
//! The pool bookkeeping (`Pool`, slot states, the release tags) is pure and tested without a GPU. The GPU-backed
//! `Backing` wraps it.
//!
//! Owner: `sdk-wl-backing`.

use super::connection::{Connection, Error};
use bifrost_gpu::{Device, DmaBufFormat, PixelFormat, Texture, TextureDescriptor, TextureUsage};
use wayland_client::protocol::wl_buffer::WlBuffer;

/// Slots per layer: two are enough to draw one frame while the compositor reads the other. Three absorb a late
/// release without stalling the frame.
pub const POOL_SIZE: usize = 3;

/// Where a slot is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Free to draw into.
    Free,
    /// Handed to the app to draw into; not yet committed.
    Acquired,
    /// Committed: bound to a contents id and held by the compositor until its release event.
    Committed,
}

/// A misuse of the pool: a slot index outside the pool, or a transition from the wrong state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    BadSlot(usize),
    WrongState { slot: usize, state: SlotState },
}

/// The slot bookkeeping of one pool, with no GPU behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    slots: Vec<SlotState>,
}

impl Pool {
    /// A pool of `count` free slots.
    pub fn new(count: usize) -> Pool {
        Pool { slots: vec![SlotState::Free; count] }
    }

    /// Takes the lowest free slot for drawing. `None` when every slot is acquired or held by the compositor.
    pub fn acquire(&mut self) -> Option<usize> {
        let slot = self.slots.iter().position(|s| *s == SlotState::Free)?;
        self.slots[slot] = SlotState::Acquired;
        Some(slot)
    }

    /// Marks an acquired slot committed (bound and sent to the compositor).
    pub fn commit(&mut self, slot: usize) -> Result<(), PoolError> {
        self.transition(slot, SlotState::Acquired, SlotState::Committed)
    }

    /// Gives back an acquired slot that was not drawn or committed.
    pub fn cancel(&mut self, slot: usize) -> Result<(), PoolError> {
        self.transition(slot, SlotState::Acquired, SlotState::Free)
    }

    /// The compositor released a committed slot; it is free again.
    pub fn release(&mut self, slot: usize) -> Result<(), PoolError> {
        self.transition(slot, SlotState::Committed, SlotState::Free)
    }

    /// The state of a slot.
    pub fn state(&self, slot: usize) -> Option<SlotState> {
        self.slots.get(slot).copied()
    }

    /// How many slots can be acquired now.
    pub fn free_count(&self) -> usize {
        self.slots.iter().filter(|s| **s == SlotState::Free).count()
    }

    /// How many slots the pool has.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the pool has no slots.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn transition(&mut self, slot: usize, from: SlotState, to: SlotState) -> Result<(), PoolError> {
        let cur = *self.slots.get(slot).ok_or(PoolError::BadSlot(slot))?;
        if cur != from {
            return Err(PoolError::WrongState { slot, state: cur });
        }
        self.slots[slot] = to;
        Ok(())
    }
}

/// Encodes a store id and a slot into the `wl_buffer` user data, so a release event can find its slot.
pub fn encode_tag(store: u32, slot: u32) -> u64 {
    (u64::from(store) << 32) | u64::from(slot)
}

/// The store id and slot of a tag made by `encode_tag`.
pub fn decode_tag(tag: u64) -> (u32, u32) {
    ((tag >> 32) as u32, tag as u32)
}

/// The modifiers the feedback offers for one DRM fourcc, in preference order. Empty means the device chooses linear.
pub fn modifiers_for(formats: &[DmaBufFormat], fourcc: u32) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for f in formats.iter().filter(|f| f.fourcc == fourcc) {
        if !out.contains(&f.modifier) {
            out.push(f.modifier);
        }
    }
    out
}

/// One pool slot on the GPU: the texture the app draws into and its `wl_buffer` for the compositor.
struct Slot {
    texture: Texture,
    buffer: WlBuffer,
}

/// The backing store of one layer: `POOL_SIZE` exported textures with their `wl_buffer`s.
pub struct Backing {
    store: u32,
    width: u32,
    height: u32,
    format: PixelFormat,
    pool: Pool,
    slots: Vec<Slot>,
}

impl Backing {
    /// Makes the store: `count` textures of `width` x `height` in `format`, exportable with the modifiers `formats`
    /// offers for that format, each wrapped as a `wl_buffer` tagged with `store` and its slot index.
    pub fn new(
        conn: &Connection,
        device: &Device,
        store: u32,
        width: u32,
        height: u32,
        format: PixelFormat,
        formats: &[DmaBufFormat],
        count: usize,
    ) -> Result<Backing, Error> {
        let fourcc = format.fourcc().ok_or_else(|| Error::Protocol("format is not shareable".into()))?;
        let modifiers = modifiers_for(formats, fourcc);
        let mut slots = Vec::with_capacity(count);
        for slot in 0..count {
            let texture = device
                .new_texture(&TextureDescriptor {
                    width,
                    height,
                    format,
                    usage: TextureUsage::SAMPLED | TextureUsage::RENDER_TARGET | TextureUsage::COPY_DST,
                    exportable: true,
                    modifiers: modifiers.clone(),
                })
                .map_err(Error::Gpu)?;
            let dmabuf = texture.export_dmabuf().map_err(Error::Gpu)?;
            let buffer = conn.create_dmabuf_buffer(&dmabuf, encode_tag(store, slot as u32))?;
            slots.push(Slot { texture, buffer });
        }
        Ok(Backing { store, width, height, format, pool: Pool::new(count), slots })
    }

    /// The store id this store's buffers are tagged with.
    pub fn store_id(&self) -> u32 {
        self.store
    }

    /// The store's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The store's pixel format.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Takes a free slot to draw into and returns its index, or `None` when every slot is in flight.
    pub fn acquire(&mut self) -> Option<usize> {
        self.pool.acquire()
    }

    /// The texture of a slot (what the app draws into).
    pub fn texture(&self, slot: usize) -> Option<&Texture> {
        self.slots.get(slot).map(|s| &s.texture)
    }

    /// The `wl_buffer` of a slot, for binding to the layer's contents id.
    pub fn buffer(&self, slot: usize) -> Option<&WlBuffer> {
        self.slots.get(slot).map(|s| &s.buffer)
    }

    /// Marks a slot committed after the layer bound its buffer and committed the transaction.
    pub fn commit(&mut self, slot: usize) -> Result<(), PoolError> {
        self.pool.commit(slot)
    }

    /// Gives back an acquired slot that was not drawn.
    pub fn cancel(&mut self, slot: usize) -> Result<(), PoolError> {
        self.pool.cancel(slot)
    }

    /// Handles a `BufferReleased` event: frees the slot if the tag belongs to this store and the slot is committed.
    /// Returns whether the tag was this store's.
    pub fn release_tag(&mut self, tag: u64) -> bool {
        let (store, slot) = decode_tag(tag);
        if store != self.store {
            return false;
        }
        // A release for a slot that is not committed is a stale event (the store was resized or cancelled); ignore it.
        let _ = self.pool.release(slot as usize);
        true
    }

    /// The release tag of a slot, as `create_dmabuf_buffer` was given it.
    pub fn tag(&self, slot: usize) -> u64 {
        encode_tag(self.store, slot as u32)
    }

    /// How many slots can be acquired now.
    pub fn free_slots(&self) -> usize {
        self.pool.free_count()
    }
}

impl Drop for Backing {
    fn drop(&mut self) {
        for slot in &self.slots {
            slot.buffer.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_hands_out_the_lowest_free_slot() {
        let mut p = Pool::new(3);
        assert_eq!(p.acquire(), Some(0));
        assert_eq!(p.acquire(), Some(1));
        assert_eq!(p.free_count(), 1);
        assert_eq!(p.acquire(), Some(2));
        assert_eq!(p.acquire(), None, "every slot is in flight");
    }

    #[test]
    fn committed_slot_returns_only_on_release() {
        let mut p = Pool::new(2);
        let a = p.acquire().unwrap();
        p.commit(a).unwrap();
        assert_eq!(p.state(a), Some(SlotState::Committed));
        assert_eq!(p.acquire(), Some(1));
        assert_eq!(p.acquire(), None, "the compositor still holds slot 0");
        p.release(a).unwrap();
        assert_eq!(p.acquire(), Some(0));
    }

    #[test]
    fn cancel_frees_an_acquired_slot() {
        let mut p = Pool::new(1);
        let a = p.acquire().unwrap();
        p.cancel(a).unwrap();
        assert_eq!(p.state(a), Some(SlotState::Free));
        assert_eq!(p.acquire(), Some(0));
    }

    #[test]
    fn wrong_transitions_are_errors() {
        let mut p = Pool::new(2);
        assert_eq!(p.commit(0), Err(PoolError::WrongState { slot: 0, state: SlotState::Free }));
        assert_eq!(p.release(0), Err(PoolError::WrongState { slot: 0, state: SlotState::Free }));
        let a = p.acquire().unwrap();
        assert_eq!(p.release(a), Err(PoolError::WrongState { slot: a, state: SlotState::Acquired }));
        assert_eq!(p.commit(5), Err(PoolError::BadSlot(5)));
        assert_eq!(p.state(5), None);
    }

    #[test]
    fn released_slot_can_be_committed_again() {
        let mut p = Pool::new(1);
        for _ in 0..4 {
            let s = p.acquire().unwrap();
            p.commit(s).unwrap();
            p.release(s).unwrap();
        }
        assert_eq!(p.free_count(), 1);
    }

    #[test]
    fn empty_pool_has_nothing_to_acquire() {
        let mut p = Pool::new(0);
        assert!(p.is_empty());
        assert_eq!(p.acquire(), None);
    }

    #[test]
    fn tags_round_trip_store_and_slot() {
        let tag = encode_tag(0xdead, 2);
        assert_eq!(tag, 0x0000_dead_0000_0002);
        assert_eq!(decode_tag(tag), (0xdead, 2));
        assert_eq!(decode_tag(encode_tag(u32::MAX, u32::MAX)), (u32::MAX, u32::MAX));
    }

    #[test]
    fn modifiers_are_filtered_by_fourcc_without_duplicates() {
        let formats = [
            DmaBufFormat { fourcc: 1, modifier: 0 },
            DmaBufFormat { fourcc: 2, modifier: 7 },
            DmaBufFormat { fourcc: 1, modifier: 5 },
            DmaBufFormat { fourcc: 1, modifier: 0 },
        ];
        assert_eq!(modifiers_for(&formats, 1), vec![0, 5]);
        assert_eq!(modifiers_for(&formats, 2), vec![7]);
        assert!(modifiers_for(&formats, 3).is_empty());
    }

    #[test]
    fn three_slots_are_the_default_pool() {
        assert_eq!(POOL_SIZE, 3);
        assert_eq!(Pool::new(POOL_SIZE).free_count(), 3);
    }
}
