//! Scanout targets and presentation: for each output, a small ring (3) of GBM buffer objects allocated with a
//! modifier the plane accepts, imported into Vulkan as render targets (`Device::import_dmabuf` of the BO's dma-buf),
//! presented with an atomic KMS commit through Smithay's `DrmSurface`, the render submission's sync_file as the
//! in-fence. Direct scanout: when the top full-screen surface is a dma-buf the primary plane accepts, it is put on the
//! plane and composition is skipped. The nested backend presents into the host window instead.
//!
//! Ring bookkeeping: a buffer is `Free`, `Acquired` (rendered into, or submitted and waiting for its page flip), or
//! `OnScreen` (the last flip showed it). `flipped(i)` moves `i` to `OnScreen` and frees the previous `OnScreen`
//! buffer; `abandon(i)` frees an acquired buffer whose frame was never presented. The bookkeeping functions are pure
//! and host-tested; only the constructor that imports GBM buffers needs the `drm` feature.
//!
//! Owner: wave G1 agent `wm-output`.

use bifrost_gpu::{DmaBufFormat, Error, PixelFormat, Result, Texture};
#[cfg(feature = "drm")]
use bifrost_gpu::Device;

/// The number of scanout buffers per output.
pub const RING: usize = 3;

/// The state of one scanout buffer in the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Not in use: the next frame may render into it.
    Free,
    /// Acquired for a frame: rendering, or submitted and waiting for its page flip.
    Acquired,
    /// The buffer the last page flip showed.
    OnScreen,
}

/// One output's swap state.
pub struct OutputTargets {
    pub width: u32,
    pub height: u32,
    /// Render targets, one per scanout buffer.
    pub targets: Vec<Texture>,
    /// Index of the buffer to render next.
    pub next: usize,
    /// The state of each buffer, parallel to `targets`.
    slots: Vec<Slot>,
}

impl OutputTargets {
    /// Targets over textures the caller already has (the nested backend, tests of the caller).
    pub fn from_textures(width: u32, height: u32, targets: Vec<Texture>) -> OutputTargets {
        let slots = vec![Slot::Free; targets.len()];
        OutputTargets { width, height, targets, next: 0, slots }
    }

    /// Imports GBM buffer objects as render targets. Each buffer is exported as a dma-buf (keeping the GBM buffer
    /// alive is the caller's job, since the KMS framebuffers come from it), converted to `bifrost_gpu::DmaBuf` and
    /// imported with `Device::import_dmabuf`. Exactly `RING` buffers of `width` x `height` are required, in the order
    /// the KMS side uses for its framebuffers.
    #[cfg(feature = "drm")]
    pub fn from_gbm(device: &Device, width: u32, height: u32, buffers: &[smithay::backend::allocator::gbm::GbmBuffer]) -> Result<OutputTargets> {
        use smithay::backend::allocator::dmabuf::AsDmabuf;
        use smithay::backend::allocator::Buffer;
        if buffers.len() != RING {
            return Err(Error::Invalid(format!("an output needs {RING} scanout buffers, got {}", buffers.len())));
        }
        let mut targets = Vec::with_capacity(RING);
        for buffer in buffers {
            if buffer.width() != width || buffer.height() != height {
                return Err(Error::Invalid(format!("scanout buffer is {}x{}, output is {width}x{height}", buffer.width(), buffer.height())));
            }
            let exported = buffer.export().map_err(|e| Error::DmaBuf(e.to_string()))?;
            let gpu = super::import::dmabuf_to_gpu(&exported)?;
            targets.push(device.import_dmabuf(&gpu)?);
        }
        Ok(OutputTargets::from_textures(width, height, targets))
    }

    /// The target to render the next frame into, or `None` when every buffer is still queued or on screen.
    pub fn acquire(&mut self) -> Option<(usize, Texture)> {
        let index = ring_acquire(&mut self.slots, self.next)?;
        self.next = (index + 1) % self.slots.len();
        let texture = self.targets.get(index)?.clone();
        Some((index, texture))
    }

    /// Marks buffer `index` as shown (page flip done); the previous one becomes free. Ignored when `index` was not
    /// acquired (a duplicate or stale flip event).
    pub fn flipped(&mut self, index: usize) {
        ring_flipped(&mut self.slots, index);
    }

    /// Frees buffer `index` without presenting it (the frame was dropped or the commit failed).
    pub fn abandon(&mut self, index: usize) {
        ring_abandon(&mut self.slots, index);
    }

    /// How many buffers are free right now.
    pub fn free_count(&self) -> usize {
        self.slots.iter().filter(|s| **s == Slot::Free).count()
    }
}

/// Finds the first free buffer scanning from `start` (wrapping), marks it acquired and returns its index.
pub(crate) fn ring_acquire(slots: &mut [Slot], start: usize) -> Option<usize> {
    let n = slots.len();
    if n == 0 {
        return None;
    }
    for k in 0..n {
        let i = (start + k) % n;
        if slots[i] == Slot::Free {
            slots[i] = Slot::Acquired;
            return Some(i);
        }
    }
    None
}

/// Moves acquired buffer `index` on screen, freeing the buffer that was on screen before. Returns whether anything
/// changed.
pub(crate) fn ring_flipped(slots: &mut [Slot], index: usize) -> bool {
    if slots.get(index) != Some(&Slot::Acquired) {
        return false;
    }
    for slot in slots.iter_mut() {
        if *slot == Slot::OnScreen {
            *slot = Slot::Free;
        }
    }
    slots[index] = Slot::OnScreen;
    true
}

/// Frees acquired buffer `index` without a flip. Returns whether anything changed.
pub(crate) fn ring_abandon(slots: &mut [Slot], index: usize) -> bool {
    if slots.get(index) != Some(&Slot::Acquired) {
        return false;
    }
    slots[index] = Slot::Free;
    true
}

/// A client's buffer as the direct-scanout check sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanoutCandidate {
    pub width: u32,
    pub height: u32,
    /// DRM fourcc of the client buffer.
    pub fourcc: u32,
    /// DRM modifier of the client buffer.
    pub modifier: u64,
    /// The surface declares its whole buffer opaque (an opaque region covering it).
    pub opaque_region: bool,
    /// Something is drawn under this surface (another window, the desktop, a layer surface).
    pub surface_below: bool,
}

/// Whether a full-screen client buffer can go straight on the primary plane instead of being composed:
/// its size is the output's, the plane accepts its exact fourcc and modifier pair, and nothing needs blending under it
/// (the buffer is opaque, or nothing is drawn below it). X formats (no alpha) count as opaque.
pub fn direct_scanout_eligible(candidate: &ScanoutCandidate, output_width: u32, output_height: u32, plane_formats: &[DmaBufFormat]) -> bool {
    if candidate.width != output_width || candidate.height != output_height {
        return false;
    }
    let format_is_opaque = PixelFormat::from_fourcc(candidate.fourcc).map(|f| f.is_opaque()).unwrap_or(false);
    if candidate.surface_below && !(candidate.opaque_region || format_is_opaque) {
        return false;
    }
    let wanted = DmaBufFormat { fourcc: candidate.fourcc, modifier: candidate.modifier };
    plane_formats.contains(&wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRGB8888: u32 = 0x3432_5258;
    const ARGB8888: u32 = 0x3432_5241;
    const LINEAR: u64 = 0;
    const X_TILED: u64 = 0x0100_0000_0000_0001;

    fn fmt(fourcc: u32, modifier: u64) -> DmaBufFormat {
        DmaBufFormat { fourcc, modifier }
    }

    #[test]
    fn acquire_hands_out_each_free_buffer_once() {
        let mut slots = vec![Slot::Free; 3];
        assert_eq!(ring_acquire(&mut slots, 0), Some(0));
        assert_eq!(ring_acquire(&mut slots, 1), Some(1));
        assert_eq!(ring_acquire(&mut slots, 2), Some(2));
        // All three are acquired: nothing is free until a flip.
        assert_eq!(ring_acquire(&mut slots, 0), None);
        assert_eq!(slots, vec![Slot::Acquired; 3]);
    }

    #[test]
    fn acquire_scans_from_the_start_index_and_wraps() {
        let mut slots = vec![Slot::Free, Slot::OnScreen, Slot::Acquired];
        // Starting at 1: 1 is on screen, 2 is acquired, the scan wraps to 0.
        assert_eq!(ring_acquire(&mut slots, 1), Some(0));
        let mut all_free = vec![Slot::Free; 3];
        assert_eq!(ring_acquire(&mut all_free, 2), Some(2));
    }

    #[test]
    fn flip_frees_the_previous_buffer_on_screen() {
        let mut slots = vec![Slot::Acquired; 3];
        assert!(ring_flipped(&mut slots, 0));
        assert_eq!(slots, vec![Slot::OnScreen, Slot::Acquired, Slot::Acquired]);
        assert!(ring_flipped(&mut slots, 1));
        assert_eq!(slots, vec![Slot::Free, Slot::OnScreen, Slot::Acquired]);
        // The freed buffer is the one acquire returns next.
        assert_eq!(ring_acquire(&mut slots, 2), Some(0));
    }

    #[test]
    fn flip_of_a_buffer_not_acquired_is_ignored() {
        let mut slots = vec![Slot::OnScreen, Slot::Free, Slot::Acquired];
        assert!(!ring_flipped(&mut slots, 1));
        assert!(!ring_flipped(&mut slots, 0));
        assert!(!ring_flipped(&mut slots, 9));
        assert_eq!(slots, vec![Slot::OnScreen, Slot::Free, Slot::Acquired]);
    }

    #[test]
    fn duplicate_flip_does_not_free_the_screen_buffer() {
        let mut slots = vec![Slot::Acquired, Slot::Acquired, Slot::Free];
        assert!(ring_flipped(&mut slots, 0));
        assert!(!ring_flipped(&mut slots, 0));
        assert_eq!(slots, vec![Slot::OnScreen, Slot::Acquired, Slot::Free]);
    }

    #[test]
    fn abandon_frees_only_an_acquired_buffer() {
        let mut slots = vec![Slot::OnScreen, Slot::Acquired, Slot::Free];
        assert!(ring_abandon(&mut slots, 1));
        assert!(!ring_abandon(&mut slots, 1));
        assert!(!ring_abandon(&mut slots, 0));
        assert!(!ring_abandon(&mut slots, 2));
        assert_eq!(slots, vec![Slot::OnScreen, Slot::Free, Slot::Free]);
    }

    #[test]
    fn steady_triple_buffering_never_starves() {
        // Render, flip, render, flip... each frame acquires one buffer and the flip of the frame before frees one,
        // so the ring always has a free buffer for the next frame.
        let mut slots = vec![Slot::Free; RING];
        let mut next = 0;
        let mut pending: Option<usize> = None;
        for _ in 0..30 {
            let i = ring_acquire(&mut slots, next).expect("a buffer is free");
            next = (i + 1) % RING;
            if let Some(prev) = pending {
                assert!(ring_flipped(&mut slots, prev));
            }
            pending = Some(i);
        }
        assert_eq!(slots.iter().filter(|s| **s == Slot::OnScreen).count(), 1);
        assert_eq!(slots.iter().filter(|s| **s == Slot::Acquired).count(), 1);
        assert_eq!(slots.iter().filter(|s| **s == Slot::Free).count(), 1);
    }

    #[test]
    fn empty_ring_has_nothing_to_acquire() {
        let mut slots: Vec<Slot> = Vec::new();
        assert_eq!(ring_acquire(&mut slots, 0), None);
        assert!(!ring_flipped(&mut slots, 0));
    }

    #[test]
    fn full_screen_opaque_buffer_on_a_matching_plane_scans_out() {
        let c = ScanoutCandidate { width: 2560, height: 1440, fourcc: XRGB8888, modifier: LINEAR, opaque_region: false, surface_below: true };
        assert!(direct_scanout_eligible(&c, 2560, 1440, &[fmt(XRGB8888, LINEAR)]));
    }

    #[test]
    fn size_must_match_the_output() {
        let c = ScanoutCandidate { width: 2560, height: 1400, fourcc: XRGB8888, modifier: LINEAR, opaque_region: true, surface_below: false };
        assert!(!direct_scanout_eligible(&c, 2560, 1440, &[fmt(XRGB8888, LINEAR)]));
        let narrow = ScanoutCandidate { width: 1280, ..c };
        assert!(!direct_scanout_eligible(&narrow, 2560, 1440, &[fmt(XRGB8888, LINEAR)]));
    }

    #[test]
    fn plane_must_accept_the_exact_format_and_modifier() {
        let c = ScanoutCandidate { width: 100, height: 100, fourcc: XRGB8888, modifier: X_TILED, opaque_region: true, surface_below: false };
        // Same fourcc, other modifier: no.
        assert!(!direct_scanout_eligible(&c, 100, 100, &[fmt(XRGB8888, LINEAR)]));
        // Same modifier, other fourcc: no.
        assert!(!direct_scanout_eligible(&c, 100, 100, &[fmt(ARGB8888, X_TILED)]));
        // Exact pair in a longer list: yes.
        assert!(direct_scanout_eligible(&c, 100, 100, &[fmt(ARGB8888, LINEAR), fmt(XRGB8888, X_TILED)]));
        // No planes at all: no.
        assert!(!direct_scanout_eligible(&c, 100, 100, &[]));
    }

    #[test]
    fn translucent_buffer_over_something_is_not_scanned_out() {
        let translucent = ScanoutCandidate { width: 10, height: 10, fourcc: ARGB8888, modifier: LINEAR, opaque_region: false, surface_below: true };
        assert!(!direct_scanout_eligible(&translucent, 10, 10, &[fmt(ARGB8888, LINEAR)]));
    }

    #[test]
    fn opaque_region_makes_an_alpha_buffer_eligible() {
        let declared = ScanoutCandidate { width: 10, height: 10, fourcc: ARGB8888, modifier: LINEAR, opaque_region: true, surface_below: true };
        assert!(direct_scanout_eligible(&declared, 10, 10, &[fmt(ARGB8888, LINEAR)]));
    }

    #[test]
    fn nothing_below_makes_an_alpha_buffer_eligible() {
        let alone = ScanoutCandidate { width: 10, height: 10, fourcc: ARGB8888, modifier: LINEAR, opaque_region: false, surface_below: false };
        assert!(direct_scanout_eligible(&alone, 10, 10, &[fmt(ARGB8888, LINEAR)]));
    }

    #[test]
    fn unknown_fourcc_is_not_treated_as_opaque() {
        // A fourcc BifrOSt does not handle is not X-format, so with something below it is refused even when the
        // plane lists it.
        let unknown = ScanoutCandidate { width: 10, height: 10, fourcc: 0x3631_564E, modifier: LINEAR, opaque_region: false, surface_below: true };
        assert!(!direct_scanout_eligible(&unknown, 10, 10, &[fmt(0x3631_564E, LINEAR)]));
    }

    #[test]
    fn fresh_ring_has_ring_free_buffers() {
        // Textures need a device, so this checks the state `from_textures` sets (every slot Free) through the pure
        // helpers: a fresh ring of RING slots hands out RING buffers, then nothing.
        let mut slots = vec![Slot::Free; RING];
        for expected in 0..RING {
            assert_eq!(ring_acquire(&mut slots, 0), Some(expected));
        }
        assert_eq!(ring_acquire(&mut slots, 0), None);
    }
}
