//! `Server`: the render server's one entry point, used by every backend (DRM, nested, headless). The backend owns
//! the protocol state and the outputs; per output frame it builds the scene (`compose::SceneNode`s, back to front,
//! from the window stack and the layer trees), asks `Server::frame` to composite it into the output's next target,
//! and presents the returned submission (its sync_file is the KMS in-fence).
//!
//! Wave G2 owners: `wm-drm-vulkan` (DRM backend), `wm-nested-headless` (nested and headless backends and the GPU test
//! harness), `wm-scene` (building `SceneNode`s from Smithay's surface trees and the layer contexts). Change this
//! file's public API only with the lead.

use crate::render_server::compose::{self, ComposePipelines, DamageRects, SceneNode};
use crate::render_server::import::Importer;
use crate::render_server::layers::ContextStore;
use crate::render_server::material::Blur;
use crate::render_server::stats::{FrameRecord, Stats};
use bifrost_gpu::{CommandQueue, Device, LoadAction, PixelFormat, Result, Submission, Texture};
use std::path::Path;
use std::time::Instant;

/// The Vulkan render server: device, importer, pipelines, blur, layer contexts and frame statistics.
pub struct Server {
    pub device: Device,
    pub queue: CommandQueue,
    pub importer: Importer,
    pub compose: ComposePipelines,
    pub blur: Blur,
    pub layers: ContextStore,
    pub stats: Stats,
    seq: u64,
}

impl Server {
    /// Opens the GPU behind `render_node` (the scanout device's render node) and makes the pipelines for targets of
    /// `target_format` (`Bgra8Unorm` for XRGB/ARGB8888 scanout).
    pub fn new(render_node: Option<&Path>, target_format: PixelFormat) -> Result<Server> {
        let device = Device::open(render_node)?;
        Ok(Server {
            queue: device.new_command_queue(),
            importer: Importer::new(&device),
            compose: ComposePipelines::new(&device, target_format)?,
            blur: Blur::new(&device)?,
            layers: ContextStore::default(),
            stats: Stats::new(600),
            device,
            seq: 0,
        })
    }

    /// Composites `nodes` (back to front) into `target`, redrawing `damage` only. `load` is what the target holds:
    /// `Load` when it still has the frame of its buffer age (only the damage changes), `Clear` otherwise. Returns the
    /// submission; the caller presents the target once its `sync_file` signals (or passes it as the KMS in-fence).
    pub fn frame(&mut self, nodes: &[SceneNode], target: &Texture, damage: &DamageRects, load: LoadAction) -> Result<Submission> {
        let started = Instant::now();
        let importer = &self.importer;
        let items = compose::build_items(nodes, damage, &|key| importer.texture(key));
        let mut cb = self.queue.command_buffer()?;
        compose::record(&mut cb, &self.compose, target, load, damage, &items, &mut self.blur)?;
        let submission = cb.commit()?;
        self.seq += 1;
        let damaged_px = damage.iter().map(|r| r[2].max(0) as u64 * r[3].max(0) as u64).sum();
        self.stats.push(FrameRecord { seq: self.seq, cpu: started.elapsed(), damaged_px, ..FrameRecord::default() });
        Ok(submission)
    }
}
