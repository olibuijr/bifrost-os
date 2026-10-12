//! Command queues, command buffers and encoders (dynamic rendering, synchronization2). Image layouts and the
//! foreign-queue ownership transfers for dma-buf images are tracked here so callers never write barriers.
//!
//! Owner: wave G1 agent `gpu-command`.
//!
//! How it fits together:
//! - A `CommandQueue` owns a command pool (created on first use, `RESET_COMMAND_BUFFER`). Each `command_buffer()`
//!   allocates one primary buffer from it; the buffer goes back to the pool when its `Submission` completes (or when an
//!   uncommitted buffer is dropped), through a `Retire` guard kept in `keep`.
//! - Every use of a texture goes through `CommandBuffer::track`, which emits one `vkCmdPipelineBarrier2` when the image
//!   needs a layout change, a hazard or a queue ownership transfer. Layouts seen while recording are kept per buffer and
//!   written to `texture::Inner::layout` only at `commit`, so a buffer that is dropped leaves the textures untouched.
//! - Textures with `foreign` set belong to another process or the display: the first use in a buffer acquires them from
//!   `VK_QUEUE_FAMILY_FOREIGN_EXT`, and a buffer that writes them releases them back, ending in
//!   `SHADER_READ_ONLY_OPTIMAL` so the other side can sample them.
//! - Bindings (`set_texture`, `set_sampler`) are flushed at each draw or dispatch: pushed with
//!   `VK_KHR_push_descriptor` when the device has it, else written into a descriptor set from a per-buffer pool.
//! - Submission uses `vkQueueSubmit2`. Timeline waits and signals use the events' semaphores; a sync_file wait is
//!   imported into a temporary binary semaphore that is destroyed once the submission completes.
//!
//! Recording and commit order: layouts are tracked in recording order, so command buffers must be committed in the
//! order they were recorded (the normal case for one render thread per queue).

use crate::device::Device;
use crate::texture::Region;
use crate::{Buffer, ComputePipeline, Error, RenderPipeline, Result, Sampler, SharedEvent, Texture};
use ash::vk;
use std::collections::BTreeMap;
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

/// Serialises every `vkQueueSubmit2`: a `VkQueue` needs external synchronisation.

/// The most push constant bytes a pipeline may use (the Vulkan minimum limit).
pub const MAX_PUSH_BYTES: usize = 128;

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
    pub(crate) device: Device,
    pub(crate) pool: Arc<PoolInner>,
}

impl CommandQueue {
    /// A queue on `device` with its own (lazily created) command pool.
    pub(crate) fn new(device: Device) -> CommandQueue {
        let pool = Arc::new(PoolInner { device: device.clone(), pool: Mutex::new(None) });
        CommandQueue { device, pool }
    }

    /// A new command buffer, recording.
    pub fn command_buffer(&self) -> Result<CommandBuffer> {
        let inner = self.device.inner.clone();
        let raw = self.pool.with_pool(|pool| {
            let info = vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
            // SAFETY: `pool` is a live pool of this device; the buffer is freed through a `Retire` guard.
            let cbs = unsafe { inner.device.allocate_command_buffers(&info) }?;
            Ok(cbs[0])
        })?;
        // The guard is the first `keep` entry, so dropping an uncommitted buffer frees it.
        let guard = Keep::Retire(Arc::new(Retire::CommandBuffer { pool: self.pool.clone(), cb: raw }));
        let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `raw` was just allocated, is not recording and is only used by this thread.
        unsafe { self.device.inner.device.begin_command_buffer(raw, &begin) }?;
        Ok(CommandBuffer {
            device: self.device.clone(),
            raw,
            keep: vec![guard],
            waits: Vec::new(),
            signals: Vec::new(),
            wait_fds: Vec::new(),
            uses: Vec::new(),
            desc_pool: None,
        })
    }
}

/// Recorded GPU work. Encoders borrow it mutably; `commit` submits it. Dropping it without committing frees it.
pub struct CommandBuffer {
    pub(crate) device: Device,
    pub(crate) raw: vk::CommandBuffer,
    /// Resources the GPU uses until the submission completes (the buffer's own `Retire` guard is first).
    pub(crate) keep: Vec<Keep>,
    pub(crate) waits: Vec<(SharedEvent, u64)>,
    pub(crate) signals: Vec<(SharedEvent, u64)>,
    pub(crate) wait_fds: Vec<OwnedFd>,
    /// Per-texture state of this buffer: the layout after its last recorded use, and its ownership.
    pub(crate) uses: Vec<ImageUse>,
    /// The descriptor pool that new per-draw sets are allocated from (`None` until the first draw needs one).
    pub(crate) desc_pool: Option<vk::DescriptorPool>,
}

/// A resource held alive by a command buffer. The variants are read only by their drops (RAII keep-alive).
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) enum Keep {
    Texture(Texture),
    Buffer(Buffer),
    Render(RenderPipeline),
    Compute(ComputePipeline),
    Sampler(Sampler),
    /// A Vulkan object released when the last holder goes away (after the GPU finished with it).
    Retire(Arc<Retire>),
    /// A timeline event this buffer waits on or signals: it must outlive the pending submission.
    Event(SharedEvent),
}

/// The command pool of one queue, created on first use. Shared by the queue and its buffers.
pub(crate) struct PoolInner {
    device: Device,
    pool: Mutex<Option<vk::CommandPool>>,
}

impl PoolInner {
    /// Runs `f` with the pool, creating it on first use. The lock is held for the call.
    fn with_pool<T>(&self, f: impl FnOnce(vk::CommandPool) -> Result<T>) -> Result<T> {
        let mut guard = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        let pool = match *guard {
            Some(p) => p,
            None => {
                let info = vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(self.device.inner.queue_family);
                // SAFETY: a valid create info for the live device; destroyed in `Drop`.
                let p = unsafe { self.device.inner.device.create_command_pool(&info, None) }?;
                *guard = Some(p);
                p
            }
        };
        f(pool)
    }
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        let pool = self.pool.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(p) = pool {
            // SAFETY: every buffer allocated from the pool holds an `Arc<PoolInner>` through `Retire`, so none is in use.
            unsafe { self.device.inner.device.destroy_command_pool(p, None) };
        }
    }
}

/// Vulkan objects released when the last `Keep` holding them is dropped (after the GPU finished with them).
pub(crate) enum Retire {
    CommandBuffer { pool: Arc<PoolInner>, cb: vk::CommandBuffer },
    Semaphore { device: Device, sem: vk::Semaphore },
    DescriptorPool { device: Device, pool: vk::DescriptorPool },
}

impl Drop for Retire {
    fn drop(&mut self) {
        match self {
            Retire::CommandBuffer { pool, cb } => {
                let guard = pool.pool.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(p) = *guard {
                    // SAFETY: the GPU finished `cb` (its submission completed) and nothing else refers to it.
                    unsafe { pool.device.inner.device.free_command_buffers(p, &[*cb]) };
                }
            }
            Retire::Semaphore { device, sem } => {
                // SAFETY: the submission that waited on or signalled `sem` completed before this guard drops.
                unsafe { device.inner.device.destroy_semaphore(*sem, None) };
            }
            Retire::DescriptorPool { device, pool } => {
                // SAFETY: the sets allocated from `pool` are only used by completed submissions.
                unsafe { device.inner.device.destroy_descriptor_pool(*pool, None) };
            }
        }
    }
}

/// How one texture is used by a command buffer, as far as synchronisation goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Usage {
    /// A render pass colour attachment.
    ColorTarget,
    /// Sampled by a render pipeline (vertex or fragment stage).
    SampledGraphics,
    /// Sampled by a compute pipeline.
    SampledCompute,
    /// Read and written by a compute pipeline as a storage image.
    Storage,
    CopySource,
    CopyDestination,
}

/// The layout, stage, access and write flag a use needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsageInfo {
    pub(crate) layout: vk::ImageLayout,
    pub(crate) stage: vk::PipelineStageFlags2,
    pub(crate) access: vk::AccessFlags2,
    pub(crate) write: bool,
}

/// The layout, stage, access and write flag for a use of a texture.
pub(crate) fn usage_info(usage: Usage) -> UsageInfo {
    match usage {
        Usage::ColorTarget => UsageInfo {
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            stage: vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            access: vk::AccessFlags2::COLOR_ATTACHMENT_WRITE | vk::AccessFlags2::COLOR_ATTACHMENT_READ,
            write: true,
        },
        Usage::SampledGraphics => UsageInfo {
            layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            stage: vk::PipelineStageFlags2::ALL_GRAPHICS,
            access: vk::AccessFlags2::SHADER_SAMPLED_READ,
            write: false,
        },
        Usage::SampledCompute => UsageInfo {
            layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            stage: vk::PipelineStageFlags2::COMPUTE_SHADER,
            access: vk::AccessFlags2::SHADER_SAMPLED_READ,
            write: false,
        },
        Usage::Storage => UsageInfo {
            layout: vk::ImageLayout::GENERAL,
            stage: vk::PipelineStageFlags2::COMPUTE_SHADER,
            access: vk::AccessFlags2::SHADER_STORAGE_READ | vk::AccessFlags2::SHADER_STORAGE_WRITE,
            write: true,
        },
        Usage::CopySource => UsageInfo {
            layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            stage: vk::PipelineStageFlags2::TRANSFER,
            access: vk::AccessFlags2::TRANSFER_READ,
            write: false,
        },
        Usage::CopyDestination => UsageInfo {
            layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            stage: vk::PipelineStageFlags2::TRANSFER,
            access: vk::AccessFlags2::TRANSFER_WRITE,
            write: true,
        },
    }
}

/// The layout a foreign-owned texture is left in when a buffer releases it to the foreign queue.
pub(crate) const RELEASE_LAYOUT: vk::ImageLayout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;

/// Whether a use needs a barrier, given what this buffer did to the texture before.
///
/// The first use in a buffer always gets one (work from earlier submissions is not ordered against it otherwise).
/// After that a barrier is needed for a layout change, an ownership transfer, a write after anything, or any use after
/// a write. Read after read in the same layout needs none.
pub(crate) fn needs_barrier(first_in_buffer: bool, layout_changes: bool, acquires: bool, prev_write: bool, write: bool) -> bool {
    first_in_buffer || layout_changes || acquires || prev_write || write
}

/// The Vulkan row length in texels for a row `stride` bytes long, or `None` when the stride is not a whole number of
/// pixels.
pub(crate) fn row_length_texels(stride: usize, bytes_per_pixel: usize) -> Option<u32> {
    if stride == 0 || bytes_per_pixel == 0 || stride % bytes_per_pixel != 0 {
        return None;
    }
    u32::try_from(stride / bytes_per_pixel).ok()
}

/// Whether `r` lies inside a `w` by `h` texture and is not empty.
pub(crate) fn region_fits(w: u32, h: u32, r: Region) -> bool {
    r.width > 0 && r.height > 0 && r.x.checked_add(r.width).is_some_and(|v| v <= w) && r.y.checked_add(r.height).is_some_and(|v| v <= h)
}

/// The bytes of push constants, zero padded to a multiple of 4 and capped at `MAX_PUSH_BYTES`, with the length to push.
pub(crate) fn push_bytes(bytes: &[u8]) -> ([u8; MAX_PUSH_BYTES], usize) {
    let n = bytes.len().min(MAX_PUSH_BYTES);
    let mut out = [0u8; MAX_PUSH_BYTES];
    out[..n].copy_from_slice(&bytes[..n]);
    (out, (n + 3) & !3)
}

/// The colour subresource of a whole 2D image.
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).base_mip_level(0).level_count(1).base_array_layer(0).layer_count(1)
}

fn color_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1)
}

/// Per-texture state of one command buffer.
pub(crate) struct ImageUse {
    pub(crate) texture: Texture,
    /// The layout after the last recorded use.
    pub(crate) layout: vk::ImageLayout,
    /// For a foreign texture: whether this buffer already acquired it.
    pub(crate) acquired: bool,
    /// Whether any recorded use wrote it.
    pub(crate) written: bool,
    /// Stage and access of the last recorded use (the source of the next barrier).
    pub(crate) last_stage: vk::PipelineStageFlags2,
    pub(crate) last_access: vk::AccessFlags2,
    pub(crate) last_write: bool,
}

impl CommandBuffer {
    /// Begins a render pass.
    pub fn render_encoder(&mut self, pass: &RenderPassDescriptor) -> Result<RenderEncoder<'_>> {
        let tex = &pass.target;
        let full = Region { x: 0, y: 0, width: tex.width(), height: tex.height() };
        let area = pass.render_area.unwrap_or(full);
        if !region_fits(tex.width(), tex.height(), area) {
            return Err(Error::Invalid(format!("render area {area:?} outside {}x{} target", tex.width(), tex.height())));
        }
        self.track(tex, usage_info(Usage::ColorTarget));
        self.keep.push(Keep::Texture(tex.clone()));

        let (load_op, clear) = match pass.load {
            LoadAction::Load => (vk::AttachmentLoadOp::LOAD, [0.0; 4]),
            LoadAction::Clear(c) => (vk::AttachmentLoadOp::CLEAR, c),
            LoadAction::DontCare => (vk::AttachmentLoadOp::DONT_CARE, [0.0; 4]),
        };
        let store_op = match pass.store {
            StoreAction::Store => vk::AttachmentStoreOp::STORE,
            StoreAction::DontCare => vk::AttachmentStoreOp::DONT_CARE,
        };
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(tex.inner.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(load_op)
            .store_op(store_op)
            .clear_value(vk::ClearValue { color: vk::ClearColorValue { float32: clear } })];
        let rect = vk::Rect2D { offset: vk::Offset2D { x: area.x as i32, y: area.y as i32 }, extent: vk::Extent2D { width: area.width, height: area.height } };
        let info = vk::RenderingInfo::default().render_area(rect).layer_count(1).color_attachments(&attachments);
        let dev = &self.device.inner.device;
        // SAFETY: the target is in COLOR_ATTACHMENT_OPTIMAL (tracked above) and the area is inside it.
        unsafe { dev.cmd_begin_rendering(self.raw, &info) };

        // Default dynamic state: the whole target as viewport, the render area as scissor.
        let viewport = vk::Viewport { x: 0.0, y: 0.0, width: tex.width() as f32, height: tex.height() as f32, min_depth: 0.0, max_depth: 1.0 };
        // SAFETY: inside the rendering scope begun above, on this command buffer.
        unsafe {
            dev.cmd_set_viewport(self.raw, 0, &[viewport]);
            dev.cmd_set_scissor(self.raw, 0, &[rect]);
        }
        Ok(RenderEncoder { cb: self, bind: Bindings::default() })
    }

    /// Begins a compute pass.
    pub fn compute_encoder(&mut self) -> Result<ComputeEncoder<'_>> {
        Ok(ComputeEncoder { cb: self, bind: Bindings::default() })
    }

    /// Copies `region` of `src` to (`dst_x`, `dst_y`) of `dst` (same format).
    pub fn copy_texture(&mut self, src: &Texture, region: Region, dst: &Texture, dst_x: u32, dst_y: u32) -> Result<()> {
        if src.format() != dst.format() {
            return Err(Error::Invalid(format!("copy between {:?} and {:?}", src.format(), dst.format())));
        }
        if !region_fits(src.width(), src.height(), region) {
            return Err(Error::Invalid(format!("copy region {region:?} outside the source")));
        }
        let to = Region { x: dst_x, y: dst_y, width: region.width, height: region.height };
        if !region_fits(dst.width(), dst.height(), to) {
            return Err(Error::Invalid(format!("copy to ({dst_x},{dst_y}) overflows the destination")));
        }
        self.track(src, usage_info(Usage::CopySource));
        self.track(dst, usage_info(Usage::CopyDestination));
        self.keep.push(Keep::Texture(src.clone()));
        self.keep.push(Keep::Texture(dst.clone()));
        let regions = [vk::ImageCopy2::default()
            .src_subresource(color_layers())
            .src_offset(vk::Offset3D { x: region.x as i32, y: region.y as i32, z: 0 })
            .dst_subresource(color_layers())
            .dst_offset(vk::Offset3D { x: dst_x as i32, y: dst_y as i32, z: 0 })
            .extent(vk::Extent3D { width: region.width, height: region.height, depth: 1 })];
        let info = vk::CopyImageInfo2::default()
            .src_image(src.inner.image)
            .src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .dst_image(dst.inner.image)
            .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .regions(&regions);
        // SAFETY: both images are in the transfer layouts tracked above, and the regions were checked.
        unsafe { self.device.inner.device.cmd_copy_image2(self.raw, &info) };
        Ok(())
    }

    /// Copies a buffer into a texture region (uploads recorded in a frame's command buffer).
    pub fn copy_buffer_to_texture(&mut self, src: &Buffer, offset: usize, stride: usize, dst: &Texture, region: Region) -> Result<()> {
        let bpp = dst.format().bytes_per_pixel();
        let row = row_length_texels(stride, bpp).ok_or_else(|| Error::Invalid(format!("row stride {stride} is not a whole number of {bpp}-byte pixels")))?;
        if !region_fits(dst.width(), dst.height(), region) {
            return Err(Error::Invalid(format!("upload region {region:?} outside the texture")));
        }
        let needed = stride
            .checked_mul((region.height as usize).saturating_sub(1))
            .and_then(|n| n.checked_add(offset))
            .and_then(|n| n.checked_add(region.width as usize * bpp))
            .ok_or_else(|| Error::Invalid("upload range overflows".into()))?;
        if needed > src.len() {
            return Err(Error::Invalid(format!("upload reads {needed} bytes from a {}-byte buffer", src.len())));
        }
        self.track(dst, usage_info(Usage::CopyDestination));
        self.keep.push(Keep::Buffer(src.clone()));
        self.keep.push(Keep::Texture(dst.clone()));
        let regions = [vk::BufferImageCopy2::default()
            .buffer_offset(src.inner.offset + offset as u64)
            .buffer_row_length(row)
            .buffer_image_height(0)
            .image_subresource(color_layers())
            .image_offset(vk::Offset3D { x: region.x as i32, y: region.y as i32, z: 0 })
            .image_extent(vk::Extent3D { width: region.width, height: region.height, depth: 1 })];
        let info = vk::CopyBufferToImageInfo2::default()
            .src_buffer(src.inner.buffer)
            .dst_image(dst.inner.image)
            .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .regions(&regions);
        // SAFETY: the destination is in TRANSFER_DST_OPTIMAL (tracked), the source range was checked above.
        unsafe { self.device.inner.device.cmd_copy_buffer_to_image2(self.raw, &info) };
        Ok(())
    }

    /// Moves `tex` into the sampled layout now, outside any render pass. Call it for every texture a pass samples
    /// after recording uploads into it: a layout change recorded inside a pass would be invalid.
    pub fn prepare_sampled(&mut self, tex: &Texture) {
        self.track(tex, usage_info(Usage::SampledGraphics));
        self.keep.push(Keep::Texture(tex.clone()));
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
    pub fn commit(mut self) -> Result<Submission> {
        // Release whatever earlier dropped submissions the GPU has finished since the last commit.
        self.device.reap();
        let inner = self.device.inner.clone();
        let dev = &inner.device;
        let family = inner.queue_family;

        // 1. Release the foreign images this buffer wrote, then close the command buffer.
        let releases: Vec<vk::ImageMemoryBarrier2> = self
            .uses
            .iter()
            .filter(|u| u.texture.inner.foreign && u.written)
            .map(|u| {
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(u.last_stage)
                    .src_access_mask(u.last_access)
                    .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                    .dst_access_mask(vk::AccessFlags2::NONE)
                    .old_layout(u.layout)
                    .new_layout(RELEASE_LAYOUT)
                    .src_queue_family_index(family)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .image(u.texture.inner.image)
                    .subresource_range(color_range())
            })
            .collect();
        // SAFETY: recording is in progress on `self.raw` and the barriers name live images.
        unsafe {
            if !releases.is_empty() {
                dev.cmd_pipeline_barrier2(self.raw, &vk::DependencyInfo::default().image_memory_barriers(&releases));
            }
            dev.end_command_buffer(self.raw)?;
        }

        // 2. Import the sync_file waits as temporary binary semaphores. Vulkan owns each fd once the import succeeds.
        let mut temp: Vec<Arc<Retire>> = Vec::new();
        let mut wait_infos = Vec::new();
        let ext = ash::khr::external_semaphore_fd::Device::new(&inner.instance, dev);
        for fd in std::mem::take(&mut self.wait_fds) {
            // SAFETY: a plain semaphore create on the live device.
            let sem = unsafe { dev.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?;
            temp.push(Arc::new(Retire::Semaphore { device: self.device.clone(), sem }));
            let import = vk::ImportSemaphoreFdInfoKHR::default()
                .semaphore(sem)
                .flags(vk::SemaphoreImportFlags::TEMPORARY)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
                .fd(fd.as_raw_fd());
            // SAFETY: `fd` is a sync_file and `sem` a binary semaphore that nothing else uses.
            unsafe { ext.import_semaphore_fd(&import) }?;
            // The import took the fd: forget our ownership so it is not closed twice.
            let _ = fd.into_raw_fd();
            wait_infos.push(vk::SemaphoreSubmitInfo::default().semaphore(sem).value(0).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS));
        }
        for (ev, value) in &self.waits {
            wait_infos.push(vk::SemaphoreSubmitInfo::default().semaphore(ev.inner.semaphore).value(*value).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS));
        }
        let mut signal_infos: Vec<vk::SemaphoreSubmitInfo> = self
            .signals
            .iter()
            .map(|(ev, value)| vk::SemaphoreSubmitInfo::default().semaphore(ev.inner.semaphore).value(*value).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS))
            .collect();
        // Every submission also signals an exportable binary semaphore, so `Submission::sync_file` can hand the
        // display or a client a sync_file for this work without stalling.
        let mut export = vk::ExportSemaphoreCreateInfo::default().handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        // SAFETY: a binary semaphore create on the live device; destroyed by its `Retire` guard after completion.
        let done_sem = unsafe { dev.create_semaphore(&vk::SemaphoreCreateInfo::default().push_next(&mut export), None) }?;
        temp.push(Arc::new(Retire::Semaphore { device: self.device.clone(), sem: done_sem }));
        signal_infos.push(vk::SemaphoreSubmitInfo::default().semaphore(done_sem).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS));

        // 3. Submit with a fence.
        // SAFETY: a plain fence create on the live device; destroyed by `Submission`'s drop, or below on failure.
        let fence = unsafe { dev.create_fence(&vk::FenceCreateInfo::default(), None) }?;
        let cb_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(self.raw)];
        let submit = vk::SubmitInfo2::default().wait_semaphore_infos(&wait_infos).command_buffer_infos(&cb_infos).signal_semaphore_infos(&signal_infos);
        if let Err(e) = self.device.submit(&[submit], fence) {
            // SAFETY: the submission failed, so nothing refers to the fence.
            unsafe { dev.destroy_fence(fence, None) };
            return Err(e);
        }

        // 4. The GPU now owns the work: publish the layouts and keep everything alive until the fence signals.
        for u in &self.uses {
            let layout = if u.texture.inner.foreign && u.written { RELEASE_LAYOUT } else { u.layout };
            *u.texture.inner.layout.lock().unwrap_or_else(|e| e.into_inner()) = layout;
        }
        let mut keep = std::mem::take(&mut self.keep);
        keep.extend(temp.into_iter().map(Keep::Retire));
        // The submission waits on and signals these timeline semaphores: they must not be destroyed while it is pending.
        keep.extend(std::mem::take(&mut self.waits).into_iter().map(|(ev, _)| Keep::Event(ev)));
        keep.extend(std::mem::take(&mut self.signals).into_iter().map(|(ev, _)| Keep::Event(ev)));
        Ok(Submission { device: self.device.clone(), fence, keep, done_sem, sync_file: Mutex::new(None) })
    }

    /// Records the state of one use of `tex`: emits a barrier when one is needed and updates the buffer's view of the
    /// texture.
    pub(crate) fn track(&mut self, tex: &Texture, u: UsageInfo) {
        let (idx, first) = match self.uses.iter().position(|x| Arc::ptr_eq(&x.texture.inner, &tex.inner)) {
            Some(i) => (i, false),
            None => {
                let layout = *tex.inner.layout.lock().unwrap_or_else(|e| e.into_inner());
                self.uses.push(ImageUse {
                    texture: tex.clone(),
                    layout,
                    acquired: !tex.inner.foreign,
                    written: false,
                    last_stage: vk::PipelineStageFlags2::TOP_OF_PIPE,
                    last_access: vk::AccessFlags2::NONE,
                    last_write: false,
                });
                (self.uses.len() - 1, true)
            }
        };
        let family = self.device.inner.queue_family;
        let st = &self.uses[idx];
        let acquires = tex.inner.foreign && !st.acquired;
        let old = st.layout;
        let layout_changes = old != u.layout;
        if !needs_barrier(first, layout_changes, acquires, st.last_write, u.write) {
            let st = &mut self.uses[idx];
            st.last_stage = u.stage;
            st.last_access = u.access;
            st.last_write = u.write;
            return;
        }
        let (src_stage, src_access, src_family, dst_family) = if acquires {
            (vk::PipelineStageFlags2::TOP_OF_PIPE, vk::AccessFlags2::NONE, vk::QUEUE_FAMILY_FOREIGN_EXT, family)
        } else if first {
            (vk::PipelineStageFlags2::ALL_COMMANDS, vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE, vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED)
        } else {
            (st.last_stage, st.last_access, vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED)
        };
        let barrier = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(src_stage)
            .src_access_mask(src_access)
            .dst_stage_mask(u.stage)
            .dst_access_mask(u.access)
            .old_layout(old)
            .new_layout(u.layout)
            .src_queue_family_index(src_family)
            .dst_queue_family_index(dst_family)
            .image(tex.inner.image)
            .subresource_range(color_range());
        // SAFETY: recording is in progress on `self.raw`; the image is kept alive by the `keep` entry the caller adds.
        unsafe {
            self.device.inner.device.cmd_pipeline_barrier2(self.raw, &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&barrier)));
        }
        let st = &mut self.uses[idx];
        st.layout = u.layout;
        if acquires {
            st.acquired = true;
        }
        st.written |= u.write;
        st.last_stage = u.stage;
        st.last_access = u.access;
        st.last_write = u.write;
    }

    /// Allocates one descriptor set for `layout` from this buffer's pool, starting a new pool when the current one is
    /// full.
    fn alloc_set(&mut self, layout: vk::DescriptorSetLayout) -> Result<vk::DescriptorSet> {
        for attempt in 0..2 {
            if self.desc_pool.is_none() || attempt == 1 {
                self.new_desc_pool()?;
            }
            let Some(pool) = self.desc_pool else { break };
            let info = vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(std::slice::from_ref(&layout));
            // SAFETY: the pool and layout belong to this device.
            match unsafe { self.device.inner.device.allocate_descriptor_sets(&info) } {
                Ok(sets) => return Ok(sets[0]),
                Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY) | Err(vk::Result::ERROR_FRAGMENTED_POOL) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(Error::OutOfMemory)
    }

    fn new_desc_pool(&mut self) -> Result<()> {
        const SETS: u32 = 1024;
        let sizes = [
            vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLED_IMAGE, descriptor_count: SETS * 8 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLER, descriptor_count: SETS * 8 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_IMAGE, descriptor_count: SETS * 8 },
        ];
        let info = vk::DescriptorPoolCreateInfo::default().max_sets(SETS).pool_sizes(&sizes);
        // SAFETY: a valid create info on the live device; destroyed by the `Retire` guard kept below.
        let pool = unsafe { self.device.inner.device.create_descriptor_pool(&info, None) }?;
        self.keep.push(Keep::Retire(Arc::new(Retire::DescriptorPool { device: self.device.clone(), pool })));
        self.desc_pool = Some(pool);
        Ok(())
    }
}

/// One binding of set 0, as the encoder last set it.
#[derive(Clone, Copy)]
pub(crate) enum Bind {
    Image { view: vk::ImageView, layout: vk::ImageLayout, ty: vk::DescriptorType },
    Sampler(vk::Sampler),
}

/// The bindings and pipeline state an encoder carries, shared by render and compute encoders.
#[derive(Default)]
pub(crate) struct Bindings {
    /// Bind point, pipeline layout and set-0 layout of the current pipeline.
    pub(crate) point: Option<vk::PipelineBindPoint>,
    pub(crate) layout: Option<vk::PipelineLayout>,
    pub(crate) set_layout: Option<vk::DescriptorSetLayout>,
    pub(crate) binds: BTreeMap<u32, Bind>,
    /// Set when a binding or the pipeline changed since the last flush.
    pub(crate) dirty: bool,
    /// Bytes of push constants the bound render pipeline declares (`set_bytes` never pushes more).
    pub(crate) push_cap: usize,
}

impl Bindings {
    /// Pushes (or binds a pool set for) set 0 with the current bindings, when anything changed since the last flush.
    fn flush(&mut self, cb: &mut CommandBuffer) -> Result<()> {
        let (Some(point), Some(layout), Some(set_layout)) = (self.point, self.layout, self.set_layout) else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        // A pipeline without textures or samplers has no set to push or bind (an empty push is invalid).
        if self.binds.is_empty() {
            self.dirty = false;
            return Ok(());
        }
        let infos: Vec<vk::DescriptorImageInfo> = self
            .binds
            .values()
            .map(|b| match *b {
                Bind::Image { view, layout, .. } => vk::DescriptorImageInfo::default().image_view(view).image_layout(layout),
                Bind::Sampler(s) => vk::DescriptorImageInfo::default().sampler(s),
            })
            .collect();
        let writes_for = |dst: Option<vk::DescriptorSet>| -> Vec<vk::WriteDescriptorSet> {
            self.binds
                .iter()
                .zip(infos.iter())
                .map(|((&binding, b), info)| {
                    let ty = match b {
                        Bind::Image { ty, .. } => *ty,
                        Bind::Sampler(_) => vk::DescriptorType::SAMPLER,
                    };
                    let w = vk::WriteDescriptorSet::default().dst_binding(binding).descriptor_count(1).descriptor_type(ty).image_info(std::slice::from_ref(info));
                    match dst {
                        Some(set) => w.dst_set(set),
                        None => w,
                    }
                })
                .collect()
        };
        let inner = cb.device.inner.clone();
        let dev = &inner.device;
        if inner.info.push_descriptors {
            let writes = writes_for(None);
            let push = ash::khr::push_descriptor::Device::new(&inner.instance, dev);
            // SAFETY: the bound pipeline's set-0 layout was created with push descriptors on this device; the views,
            // layouts and samplers were tracked and are kept alive by `keep`.
            unsafe { push.cmd_push_descriptor_set(cb.raw, point, layout, 0, &writes) };
        } else {
            let set = cb.alloc_set(set_layout)?;
            let writes = writes_for(Some(set));
            // SAFETY: `set` was allocated from this device's pool with `set_layout`; the writes name live views.
            unsafe {
                dev.update_descriptor_sets(&writes, &[]);
                dev.cmd_bind_descriptor_sets(cb.raw, point, layout, 0, &[set], &[]);
            }
        }
        self.dirty = false;
        Ok(())
    }
}

/// Records draws into one render pass. Ends the pass when dropped or on `end`.
pub struct RenderEncoder<'a> {
    pub(crate) cb: &'a mut CommandBuffer,
    pub(crate) bind: Bindings,
}

impl RenderEncoder<'_> {
    pub fn set_pipeline(&mut self, pipeline: &RenderPipeline) {
        let p = &pipeline.inner;
        self.cb.keep.push(Keep::Render(pipeline.clone()));
        // SAFETY: recording on `cb.raw` inside the rendering scope; the pipeline is kept alive above.
        unsafe { self.cb.device.inner.device.cmd_bind_pipeline(self.cb.raw, vk::PipelineBindPoint::GRAPHICS, p.pipeline) };
        self.bind.point = Some(vk::PipelineBindPoint::GRAPHICS);
        self.bind.layout = Some(p.layout);
        self.bind.set_layout = Some(p.set_layout);
        self.bind.push_cap = p.desc.push_constant_bytes as usize;
        self.bind.dirty = true;
    }

    /// Viewport in pixels; the default is the whole target.
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let vp = vk::Viewport { x, y, width: w, height: h, min_depth: 0.0, max_depth: 1.0 };
        // SAFETY: recording inside the rendering scope.
        unsafe { self.cb.device.inner.device.cmd_set_viewport(self.cb.raw, 0, &[vp]) };
    }

    pub fn set_scissor(&mut self, region: Region) {
        let rect = vk::Rect2D { offset: vk::Offset2D { x: region.x as i32, y: region.y as i32 }, extent: vk::Extent2D { width: region.width, height: region.height } };
        // SAFETY: recording inside the rendering scope.
        unsafe { self.cb.device.inner.device.cmd_set_scissor(self.cb.raw, 0, &[rect]) };
    }

    /// Push constants (at most 128 bytes, shared by the vertex and fragment stages). Bytes past 128 are ignored.
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        debug_assert!(bytes.len() <= MAX_PUSH_BYTES, "push constants over {MAX_PUSH_BYTES} bytes");
        let Some(layout) = self.bind.layout else { return };
        let cap = self.bind.push_cap.min(MAX_PUSH_BYTES);
        let (data, len) = push_bytes(&bytes[..bytes.len().min(cap)]);
        let len = len.min(cap);
        if len == 0 {
            return;
        }
        // SAFETY: the range covers offset 0 of the bound pipeline layout, which declares a VERTEX | FRAGMENT range of
        // `push_cap` bytes; `len` is capped to it.
        unsafe { self.cb.device.inner.device.cmd_push_constants(self.cb.raw, layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT, 0, &data[..len]) };
    }

    /// Binds a vertex (instance) buffer at `binding`.
    pub fn set_vertex_buffer(&mut self, binding: u32, buffer: &Buffer, offset: usize) {
        // Small buffers are slices of one shared VkBuffer: an offset past this one would read a neighbour's bytes.
        if crate::memory::check_range(buffer.len(), offset, 0).is_err() {
            debug_assert!(false, "vertex buffer offset {offset} outside a {}-byte buffer", buffer.len());
            return;
        }
        self.cb.keep.push(Keep::Buffer(buffer.clone()));
        // SAFETY: the buffer is live (kept above) and the offset is within it for the draws that follow.
        unsafe { self.cb.device.inner.device.cmd_bind_vertex_buffers(self.cb.raw, binding, &[buffer.inner.buffer], &[buffer.inner.offset + offset as u64]) };
    }

    /// Binds a sampled texture at descriptor `binding` of set 0.
    pub fn set_texture(&mut self, binding: u32, texture: &Texture) {
        self.cb.track(texture, usage_info(Usage::SampledGraphics));
        self.cb.keep.push(Keep::Texture(texture.clone()));
        self.bind.binds.insert(binding, Bind::Image { view: texture.inner.view, layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, ty: vk::DescriptorType::SAMPLED_IMAGE });
        self.bind.dirty = true;
    }

    pub fn set_sampler(&mut self, binding: u32, sampler: &Sampler) {
        self.cb.keep.push(Keep::Sampler(sampler.clone()));
        self.bind.binds.insert(binding, Bind::Sampler(sampler.inner.sampler));
        self.bind.dirty = true;
    }

    /// Draws `vertex_count` vertices (triangle list or strip, per the pipeline) for `instance_count` instances.
    /// A draw with no pipeline bound, or whose bindings cannot be allocated, is skipped.
    pub fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        if self.bind.point.is_none() || self.bind.flush(self.cb).is_err() {
            return;
        }
        // SAFETY: a graphics pipeline and its bindings are bound (flush above) inside the rendering scope.
        unsafe { self.cb.device.inner.device.cmd_draw(self.cb.raw, vertex_count, instance_count, first_vertex, first_instance) };
    }

    /// Ends the pass.
    pub fn end(self) {}
}

impl Drop for RenderEncoder<'_> {
    fn drop(&mut self) {
        // SAFETY: this encoder began the rendering scope in `render_encoder` and ends it exactly once.
        unsafe { self.cb.device.inner.device.cmd_end_rendering(self.cb.raw) };
    }
}

/// Records dispatches into one compute pass.
pub struct ComputeEncoder<'a> {
    pub(crate) cb: &'a mut CommandBuffer,
    pub(crate) bind: Bindings,
}

impl ComputeEncoder<'_> {
    pub fn set_pipeline(&mut self, pipeline: &ComputePipeline) {
        let p = &pipeline.inner;
        self.cb.keep.push(Keep::Compute(pipeline.clone()));
        // SAFETY: recording on `cb.raw`; the pipeline is kept alive above.
        unsafe { self.cb.device.inner.device.cmd_bind_pipeline(self.cb.raw, vk::PipelineBindPoint::COMPUTE, p.pipeline) };
        self.bind.point = Some(vk::PipelineBindPoint::COMPUTE);
        self.bind.layout = Some(p.layout);
        self.bind.set_layout = Some(p.set_layout);
        self.bind.dirty = true;
    }

    /// Push constants (at most 128 bytes). Bytes past 128 are ignored.
    pub fn set_bytes(&mut self, bytes: &[u8]) {
        debug_assert!(bytes.len() <= MAX_PUSH_BYTES, "push constants over {MAX_PUSH_BYTES} bytes");
        let Some(layout) = self.bind.layout else { return };
        let (data, len) = push_bytes(bytes);
        // SAFETY: the range covers offset 0 of the bound pipeline layout (pipelines declare COMPUTE ranges).
        unsafe { self.cb.device.inner.device.cmd_push_constants(self.cb.raw, layout, vk::ShaderStageFlags::COMPUTE, 0, &data[..len]) };
    }

    /// Binds a texture at `binding`: sampled (`write` false) or a storage image (`write` true).
    pub fn set_texture(&mut self, binding: u32, texture: &Texture, write: bool) {
        let (usage, ty) = if write { (Usage::Storage, vk::DescriptorType::STORAGE_IMAGE) } else { (Usage::SampledCompute, vk::DescriptorType::SAMPLED_IMAGE) };
        let info = usage_info(usage);
        self.cb.track(texture, info);
        self.cb.keep.push(Keep::Texture(texture.clone()));
        self.bind.binds.insert(binding, Bind::Image { view: texture.inner.view, layout: info.layout, ty });
        self.bind.dirty = true;
    }

    pub fn set_sampler(&mut self, binding: u32, sampler: &Sampler) {
        self.cb.keep.push(Keep::Sampler(sampler.clone()));
        self.bind.binds.insert(binding, Bind::Sampler(sampler.inner.sampler));
        self.bind.dirty = true;
    }

    /// Dispatches `groups_x` by `groups_y` by `groups_z` workgroups. Skipped when no pipeline is bound or the bindings
    /// cannot be allocated.
    pub fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32) {
        if self.bind.point.is_none() || self.bind.flush(self.cb).is_err() {
            return;
        }
        // SAFETY: a compute pipeline and its bindings are bound (flush above).
        unsafe { self.cb.device.inner.device.cmd_dispatch(self.cb.raw, groups_x, groups_y, groups_z) };
    }

    pub fn end(self) {}
}

/// A committed command buffer. Dropping it waits for the GPU, then releases what the work used.
pub struct Submission {
    pub(crate) device: Device,
    pub(crate) fence: vk::Fence,
    /// Held until the fence signals (RAII keep-alive; read by `Drop` through the guards).
    #[allow(dead_code)]
    pub(crate) keep: Vec<Keep>,
    /// The exportable binary semaphore the submission signals (destroyed by its guard in `keep`).
    pub(crate) done_sem: vk::Semaphore,
    /// The sync_file exported from `done_sem` (a SYNC_FD export can happen once; later calls duplicate it).
    pub(crate) sync_file: Mutex<Option<OwnedFd>>,
}

/// A dropped submission still running on the GPU: released by `Device::reap` once its fence signals.
pub(crate) struct Retired {
    fence: vk::Fence,
    #[allow(dead_code)]
    keep: Vec<Keep>,
    device: ash::Device,
}

/// Hands a binary semaphore signalled by a submission with `fence` to the device's retired list.
pub(crate) fn retire_semaphore(device: &Device, fence: vk::Fence, sem: vk::Semaphore) {
    let keep = vec![Keep::Retire(Arc::new(Retire::Semaphore { device: device.clone(), sem }))];
    device.inner.retired.lock().unwrap_or_else(|e| e.into_inner()).push(Retired { fence, keep, device: device.inner.device.clone() });
}

impl Retired {
    pub(crate) fn signalled(&self, _device: &Device) -> bool {
        // SAFETY: the fence belongs to this device and is destroyed only by this `Retired`.
        unsafe { self.device.get_fence_status(self.fence) }.unwrap_or(true)
    }
}

impl Drop for Retired {
    fn drop(&mut self) {
        // SAFETY: reaped only after the fence signalled (or the device was lost), so the GPU no longer uses `keep`.
        unsafe { self.device.destroy_fence(self.fence, None) };
    }
}

impl Submission {
    /// Blocks until the GPU finished the work.
    pub fn wait(&self) -> Result<()> {
        // SAFETY: the fence belongs to this device and was submitted with the work.
        unsafe { self.device.inner.device.wait_for_fences(&[self.fence], true, u64::MAX) }?;
        Ok(())
    }

    /// Whether the GPU finished the work. A lost device reports not complete.
    pub fn is_complete(&self) -> bool {
        // SAFETY: the fence belongs to this device.
        unsafe { self.device.inner.device.get_fence_status(self.fence) }.unwrap_or(false)
    }

    /// A sync_file that signals when the work completes (for KMS in-fences and client release fences).
    pub fn sync_file(&self) -> Result<OwnedFd> {
        let mut cached = self.sync_file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(fd) = cached.as_ref() {
            return fd.try_clone().map_err(|e| Error::Invalid(format!("dup sync_file: {e}")));
        }
        let inner = &self.device.inner;
        let ext = ash::khr::external_semaphore_fd::Device::new(&inner.instance, &inner.device);
        let get = vk::SemaphoreGetFdInfoKHR::default().semaphore(self.done_sem).handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        // SAFETY: `done_sem` was created exportable as SYNC_FD and has a signal operation pending or completed (it was
        // submitted with this work); it is exported at most once, guarded by `cached`.
        let fd = unsafe { ext.get_semaphore_fd(&get) }?;
        let file = if fd < 0 {
            // Already signalled: the driver hands back no file. An empty, already signalled sync_file is not
            // representable portably, so report it; callers treat `Unsupported` here as "done".
            return Err(Error::Unsupported("the submission already completed; no sync_file needed".into()));
        } else {
            // SAFETY: a new descriptor returned by `vkGetSemaphoreFdKHR` for this call; nothing else owns it.
            unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) }
        };
        let out = file.try_clone().map_err(|e| Error::Invalid(format!("dup sync_file: {e}")))?;
        *cached = Some(file);
        Ok(out)
    }
}

impl Drop for Submission {
    fn drop(&mut self) {
        // Never block: an unfinished submission moves its fence and keep list to the device's retired list, which
        // `Device::reap` (run at every commit) releases once the fence signals.
        let retired = Retired { fence: self.fence, keep: std::mem::take(&mut self.keep), device: self.device.inner.device.clone() };
        if retired.signalled(&self.device) {
            drop(retired);
        } else {
            self.device.inner.retired.lock().unwrap_or_else(|e| e.into_inner()).push(retired);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_target_is_an_attachment_write() {
        let u = usage_info(Usage::ColorTarget);
        assert_eq!(u.layout, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        assert_eq!(u.stage, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
        assert!(u.write);
        assert!(u.access.contains(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE));
        assert!(u.access.contains(vk::AccessFlags2::COLOR_ATTACHMENT_READ));
    }

    #[test]
    fn sampled_uses_are_reads_in_shader_read_layout() {
        for u in [usage_info(Usage::SampledGraphics), usage_info(Usage::SampledCompute)] {
            assert_eq!(u.layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            assert!(!u.write);
            assert_eq!(u.access, vk::AccessFlags2::SHADER_SAMPLED_READ);
        }
        assert_eq!(usage_info(Usage::SampledCompute).stage, vk::PipelineStageFlags2::COMPUTE_SHADER);
        assert_eq!(usage_info(Usage::SampledGraphics).stage, vk::PipelineStageFlags2::ALL_GRAPHICS);
    }

    #[test]
    fn storage_uses_general_layout_and_write() {
        let u = usage_info(Usage::Storage);
        assert_eq!(u.layout, vk::ImageLayout::GENERAL);
        assert!(u.write);
        assert!(u.access.contains(vk::AccessFlags2::SHADER_STORAGE_WRITE));
    }

    #[test]
    fn copy_uses_map_to_transfer_layouts() {
        assert_eq!(usage_info(Usage::CopySource).layout, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        assert!(!usage_info(Usage::CopySource).write);
        assert_eq!(usage_info(Usage::CopyDestination).layout, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        assert!(usage_info(Usage::CopyDestination).write);
        assert_eq!(usage_info(Usage::CopyDestination).stage, vk::PipelineStageFlags2::TRANSFER);
    }

    #[test]
    fn barrier_rules() {
        // The first use in a buffer always synchronises, even for a read in the layout already there.
        assert!(needs_barrier(true, false, false, false, false));
        // Read after read in the same layout: no barrier.
        assert!(!needs_barrier(false, false, false, false, false));
        // A layout change, an acquire, a write after anything, or any use after a write all need one.
        assert!(needs_barrier(false, true, false, false, false));
        assert!(needs_barrier(false, false, true, false, false));
        assert!(needs_barrier(false, false, false, false, true));
        assert!(needs_barrier(false, false, false, true, false));
    }

    #[test]
    fn row_length_needs_whole_pixels() {
        // 1920 BGRA pixels at 4 bytes: a 7680-byte stride is 1920 texels.
        assert_eq!(row_length_texels(7680, 4), Some(1920));
        // A stride of 7682 bytes is not a whole number of 4-byte pixels.
        assert_eq!(row_length_texels(7682, 4), None);
        // R8 atlas rows: stride equals the width in bytes.
        assert_eq!(row_length_texels(33, 1), Some(33));
        assert_eq!(row_length_texels(0, 4), None);
        assert_eq!(row_length_texels(16, 0), None);
    }

    #[test]
    fn region_must_fit_and_be_non_empty() {
        let full = Region { x: 0, y: 0, width: 100, height: 50 };
        assert!(region_fits(100, 50, full));
        assert!(region_fits(100, 50, Region { x: 99, y: 49, width: 1, height: 1 }));
        assert!(!region_fits(100, 50, Region { x: 99, y: 0, width: 2, height: 1 }));
        assert!(!region_fits(100, 50, Region { x: 0, y: 0, width: 0, height: 10 }));
        // Overflowing x + width must not wrap around and pass.
        assert!(!region_fits(100, 50, Region { x: u32::MAX, y: 0, width: 2, height: 1 }));
    }

    #[test]
    fn push_constants_pad_to_four_and_cap_at_128() {
        let (data, len) = push_bytes(&[1, 2, 3, 4, 5]);
        assert_eq!(len, 8);
        assert_eq!(&data[..5], &[1, 2, 3, 4, 5]);
        assert_eq!(data[5..8], [0, 0, 0]);
        assert_eq!(push_bytes(&[9; 8]).1, 8);
        assert_eq!(push_bytes(&[0; 0]).1, 0);
        let (big, len) = push_bytes(&[7; 200]);
        assert_eq!(len, MAX_PUSH_BYTES);
        assert!(big.iter().all(|&b| b == 7));
    }

    #[test]
    fn release_leaves_images_readable_by_the_foreign_side() {
        assert_eq!(RELEASE_LAYOUT, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
    }

    #[test]
    fn submissions_and_buffers_cross_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<Submission>();
        assert_send::<CommandBuffer>();
        assert_send::<CommandQueue>();
    }
}

#[cfg(all(test, feature = "gpu-tests"))]
mod gpu_tests {
    use super::*;
    use crate::{Device, PixelFormat, TextureDescriptor, TextureUsage};

    /// Clears a texture to a colour in a render pass, reads it back and checks the pixels.
    #[test]
    fn clear_pass_reads_back() {
        let device = Device::open(None).expect("a Vulkan 1.3 device");
        let queue = device.new_command_queue();
        let tex = device
            .new_texture(&TextureDescriptor {
                width: 16,
                height: 8,
                format: PixelFormat::Rgba8Unorm,
                usage: TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC,
                exportable: false,
                modifiers: vec![],
            })
            .expect("texture");
        let mut cb = queue.command_buffer().expect("command buffer");
        let pass = RenderPassDescriptor { target: tex.clone(), load: LoadAction::Clear([1.0, 0.0, 0.0, 1.0]), store: StoreAction::Store, render_area: None };
        drop(cb.render_encoder(&pass).expect("pass"));
        cb.commit().expect("commit").wait().expect("wait");
        let px = tex.read_region(Region { x: 0, y: 0, width: 16, height: 8 }).expect("readback");
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
    }

    fn target(device: &Device) -> Texture {
        device
            .new_texture(&TextureDescriptor { width: 64, height: 64, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC, exportable: false, modifiers: vec![] })
            .expect("texture")
    }

    /// Dropping an unfinished submission must not wait for the GPU: it moves to the retired list, and the next
    /// commit or `wait_idle` releases it.
    #[test]
    fn dropping_a_submission_retires_it_without_waiting() {
        let device = Device::open(None).expect("a Vulkan 1.3 device");
        let queue = device.new_command_queue();
        let tex = target(&device);
        for _ in 0..8 {
            let mut cb = queue.command_buffer().expect("command buffer");
            let pass = RenderPassDescriptor { target: tex.clone(), load: LoadAction::Clear([0.0, 1.0, 0.0, 1.0]), store: StoreAction::Store, render_area: None };
            drop(cb.render_encoder(&pass).expect("pass"));
            drop(cb.commit().expect("commit"));
        }
        device.wait_idle().expect("idle");
        assert!(device.inner.retired.lock().unwrap().is_empty(), "wait_idle reaps every retired submission");
        let px = tex.read_region(Region { x: 0, y: 0, width: 1, height: 1 }).expect("readback");
        assert_eq!(px, vec![0, 255, 0, 255]);
    }

    /// A submission hands out a sync_file for the display's in-fence, and the same file again on a second call.
    #[test]
    fn a_submission_exports_a_sync_file_once_and_duplicates_it() {
        let device = Device::open(None).expect("a Vulkan 1.3 device");
        let queue = device.new_command_queue();
        let tex = target(&device);
        let mut cb = queue.command_buffer().expect("command buffer");
        let pass = RenderPassDescriptor { target: tex.clone(), load: LoadAction::Clear([0.0, 0.0, 1.0, 1.0]), store: StoreAction::Store, render_area: None };
        drop(cb.render_encoder(&pass).expect("pass"));
        let sub = cb.commit().expect("commit");
        match (sub.sync_file(), sub.sync_file()) {
            (Ok(a), Ok(b)) => assert_ne!(std::os::fd::AsRawFd::as_raw_fd(&a), std::os::fd::AsRawFd::as_raw_fd(&b), "the second call is a duplicate"),
            // A driver may already have finished and hand back no file; that is the documented `Unsupported`.
            (Err(Error::Unsupported(_)), _) => {}
            other => panic!("unexpected sync_file results: {:?}", (other.0.is_ok(), other.1.is_ok())),
        }
        sub.wait().expect("wait");
    }

    /// A timeline value exported as a sync_file keeps its binary semaphore alive until the export's fence signals.
    #[test]
    fn exporting_a_pending_timeline_value_retires_the_semaphore() {
        let device = Device::open(None).expect("a Vulkan 1.3 device");
        let ev = device.new_shared_event().expect("event");
        let file = ev.export_sync_file(1).expect("a pending value exports a sync_file");
        assert!(!device.inner.retired.lock().unwrap().is_empty(), "the export's semaphore waits in the retired list");
        ev.signal(1).expect("signal");
        drop(file);
        device.wait_idle().expect("idle");
        assert!(device.inner.retired.lock().unwrap().is_empty());
    }

    /// Small buffers are slices of a shared block: a copy from the second one must read its own bytes, not the
    /// block's first bytes (the sub-allocation offset is added).
    #[test]
    fn copies_from_a_sub_allocated_buffer_use_its_offset() {
        use crate::BufferUsage;
        let device = Device::open(None).expect("a Vulkan 1.3 device");
        let queue = device.new_command_queue();
        let first = device.new_buffer(16, BufferUsage::STAGING).expect("first small buffer");
        let second = device.new_buffer(16, BufferUsage::STAGING).expect("second small buffer");
        assert!(second.inner.offset > 0 || first.inner.buffer != second.inner.buffer, "second buffer is a later slice");
        first.write(0, &[9u8; 16]).unwrap();
        let pixels: Vec<u8> = (1..=16).collect();
        second.write(0, &pixels).unwrap();
        let tex = device
            .new_texture(&TextureDescriptor { width: 2, height: 2, format: PixelFormat::Rgba8Unorm, usage: TextureUsage::COPY_DST | TextureUsage::COPY_SRC, exportable: false, modifiers: vec![] })
            .expect("texture");
        let mut cb = queue.command_buffer().expect("command buffer");
        cb.copy_buffer_to_texture(&second, 0, 8, &tex, Region { x: 0, y: 0, width: 2, height: 2 }).expect("copy");
        cb.commit().expect("commit").wait().expect("wait");
        assert_eq!(tex.read_region(Region { x: 0, y: 0, width: 2, height: 2 }).unwrap(), pixels);
    }
}
