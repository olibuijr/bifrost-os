//! Scanout targets and presentation: for each output, a small ring (3) of GBM buffer objects allocated with a
//! modifier the plane accepts, imported into Vulkan as render targets (`Device::import_dmabuf` of the BO's dma-buf),
//! presented with an atomic KMS commit through Smithay's `DrmSurface`, the render submission's sync_file as the
//! in-fence. Direct scanout: when the top full-screen surface is a dma-buf the primary plane accepts, it is put on the
//! plane and composition is skipped. The nested backend presents into the host window instead.
//!
//! Owner: wave G1 agent `wm-output`.

use bifrost_gpu::Texture;

/// One output's swap state.
pub struct OutputTargets {
    pub width: u32,
    pub height: u32,
    /// Render targets, one per scanout buffer.
    pub targets: Vec<Texture>,
    /// Index of the buffer to render next.
    pub next: usize,
}

impl OutputTargets {
    /// The target to render the next frame into, or `None` when every buffer is still queued or on screen.
    pub fn acquire(&mut self) -> Option<(usize, Texture)> {
        todo!("wm-output")
    }

    /// Marks buffer `index` as shown (page flip done); the previous one becomes free.
    pub fn flipped(&mut self, index: usize) {
        let _ = index;
        todo!("wm-output")
    }
}
