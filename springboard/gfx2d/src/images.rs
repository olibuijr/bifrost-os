//! Image texture cache keyed by `ImageRef::id`, with byte-budget LRU eviction (default 64 MiB).
//!
//! Owner: wave G1 agent `gfx2d-images`.

use bifrost_gpu::{Device, Result};

pub struct ImageCache {
    pub(crate) device: Device,
    pub(crate) budget: usize,
}

impl ImageCache {
    pub fn new(device: &Device, budget_bytes: usize) -> Result<ImageCache> {
        let _ = (device, budget_bytes);
        todo!("gfx2d-images")
    }
}
