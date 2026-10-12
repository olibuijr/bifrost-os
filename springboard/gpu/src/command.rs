//! Command queues, command buffers and encoders (dynamic rendering, synchronization2). Image layouts and the
//! foreign-queue ownership transfers for dma-buf images are tracked here so callers never write barriers.
//!
//! Owner: wave G1 agent `gpu-command`.

use crate::{Buffer, ComputePipeline, RenderPipeline, Result, Sampler, SharedEvent, Texture};
use crate::texture::Region;
use std::os::fd::OwnedFd;

/// What happens to an attachment at the start of a pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadAction {
    /// Keep what is there.
    Load,
    /// Clear to premultiplied RGBA in 0..1.
    Clear([f32; 4]),
    /// Contents are undefined (every pixel is drawn).
    DontCare,
}

/// What happens to an attachment at the end of a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreAction {
    Store,
    DontCare,
}

/// One render pass: a single colour target (BifrOSt draws 2D, no depth).
#[derive(Clone)]
pub struct RenderPassDescriptor {
    pub target: Texture,
    pub load: LoadAction,
    pub store: StoreAction,
    /// Restricts rendering to this region (damage); `None` is the whole target.
    pub render_area: Option<Region>,
}

/// A queue that command buffers are committed to, in order.
#[derive(Clone)]
pub struct CommandQueue {
    pub(crate) device: crate::Device,
}

impl CommandQueue {
    /// A new command buffer, recording.
    pub fn command_buffer(&self) -> Result<CommandBuffer> {
        todo!("gpu-command")
    }
}

/// Recorded GPU work. Encoders borrow it mutably; `commit` submits it.
pub struct CommandBuffer {
    pub(crate) device: crate::Device,
    pub(crate) raw: ash::vk::CommandBuffer,
    pub(crate) pool: ash::vk::CommandPool,
    /// Resources the GPU uses until the submission completes.
    pub(crate) keep: Vec<Keep>,
    pub(crate) waits: Vec<(SharedEvent, u64)>,
    pub(crate) signals: Vec<(SharedEvent, u64)>,
    pub(crate) wait_fds: Vec<OwnedFd>,
}

/// A resource held alive by a command buffer.
#[derive(Clone)]
pub(crate) enum Keep {
    Texture(Texture),
    Buffer(Buffer),
    Render(RenderPipeline),
    Compute(ComputePipeline),
    Sampler(Sampler),
}

impl CommandBuffer {
    /// Begins a render pass.
    pub fn render_encoder(&mut self, pass: &RenderPassDescriptor) -> Result<RenderEncoder<'_>> {
        let _ = pass;
        todo!("gpu-command")
    }

    /// Begins a compute pass.
    pub fn compute_encoder(&mut self) -> Result<ComputeEncoder<'_>> {
        todo!("gpu-command")
    }

    /// Copies `region` of `src` to (`dst_x`, `dst_y`) of `dst` (same format).
    pub fn copy_texture(&mut self, src: &Texture, region: Region, dst: &Texture, dst_x: u32, dst_y: u32) -> Result<()> {
        let _ = (src, region, dst, dst_x, dst_y);
        todo!("gpu-command")
    }

    /// Copies a buffer into a texture region (uploads recorded in a frame's command buffer).
    pub fn copy_buffer_to_texture(&mut self, src: &Buffer, offset: usize, stride: usize, dst: &Texture, region: Region) -> Result<()> {
        let _ = (src, offset, stride, dst, region);
        todo!("gpu-command")
    }

    /// The GPU waits for `event` to reach `value` before running this buffer.
    pub fn wait(&mut self, event: &SharedEvent, value: u64) {
        self.waits.push((event.clone(), value));
    }

    /// The GPU waits for a sync_file (a client's acquire fence, a KMS out-fence) before running this buffer.
    pub fn wait_sync_file(&mut self, fd: OwnedFd) {
        self.wait_fds.push(fd);
    }

    /// `event` is set to `value` when this buffer completes.
    pub fn signal(&mut self, event: &SharedEvent, value: u64) {
        self.signals.push((event.clone(), value));
    }

    /// Submits the buffer. Images used as dma-buf targets are released to the foreign queue family at the end.
    pub fn commit(self) -> Result<Submission> {
        todo!("gpu-command")
    }
}

/// A committed command buffer.
pub struct Submission {
    pub(crate) device: crate::Device,
    pub(crate) fence: ash::vk::Fence,
    pub(crate) keep: Vec<Keep>,
}

impl Submission {
    /// Blocks until the GPU finished the work.
    pub fn wait(&self) -> Result<()> {
        todo!("gpu-command")
    }

    /// Whether the GPU finished the work.
    pub fn is_complete(&self) -> bool {
        todo!("gpu-command")
    }

    /// A sync_file that signals when the work completes (for KMS in-fences and client release fences).
    pub fn sync_file(&self) -> Result<OwnedFd> {
        todo!("gpu-sync")
    }
}

/// Records draws into one render pass. Ends the pass when dropped or on `end`.
pub struct RenderEncoder<'a> {
    pub(crate) cb: &'a mut CommandBuffer,
}

impl RenderEncoder<'_> {
    pub fn set_pipeline(&mut self, pipeline: &RenderPipeline) {
        let _ = pipeline;
        todo!("gpu-command")
    }

    /// Viewport in pixels; the default is the whole target.
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let _ = (x, y, w, h);
        todo!("gpu-command")
    }

    pub fn set_scissor(&mut self, region: Region) {
        let _ = region;
        todo!("gpu-command")
    }

    /// Push constants (at most 128 bytes, shared by the vertex and fragment stages).
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        let _ = bytes;
        todo!("gpu-command")
    }

    /// Binds a vertex (instance) buffer at `binding`.
    pub fn set_vertex_buffer(&mut self, binding: u32, buffer: &Buffer, offset: usize) {
        let _ = (binding, buffer, offset);
        todo!("gpu-command")
    }

    /// Binds a sampled texture at descriptor `binding` of set 0.
    pub fn set_texture(&mut self, binding: u32, texture: &Texture) {
        let _ = (binding, texture);
        todo!("gpu-command")
    }

    pub fn set_sampler(&mut self, binding: u32, sampler: &Sampler) {
        let _ = (binding, sampler);
        todo!("gpu-command")
    }

    /// Draws `vertex_count` vertices (triangle list or strip, per the pipeline) for `instance_count` instances.
    pub fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        let _ = (vertex_count, instance_count, first_vertex, first_instance);
        todo!("gpu-command")
    }

    /// Ends the pass.
    pub fn end(self) {}
}

/// Records dispatches into one compute pass.
pub struct ComputeEncoder<'a> {
    pub(crate) cb: &'a mut CommandBuffer,
}

impl ComputeEncoder<'_> {
    pub fn set_pipeline(&mut self, pipeline: &ComputePipeline) {
        let _ = pipeline;
        todo!("gpu-command")
    }

    pub fn set_bytes(&mut self, bytes: &[u8]) {
        let _ = bytes;
        todo!("gpu-command")
    }

    /// Binds a texture at `binding`: sampled (`write` false) or a storage image (`write` true).
    pub fn set_texture(&mut self, binding: u32, texture: &Texture, write: bool) {
        let _ = (binding, texture, write);
        todo!("gpu-command")
    }

    pub fn set_sampler(&mut self, binding: u32, sampler: &Sampler) {
        let _ = (binding, sampler);
        todo!("gpu-command")
    }

    pub fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32) {
        let _ = (groups_x, groups_y, groups_z);
        todo!("gpu-command")
    }

    pub fn end(self) {}
}
