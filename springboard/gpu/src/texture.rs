//! Textures: a Vulkan image with its memory and view, made new, imported from a dma-buf or exported to one.
//!
//! Owner: wave G1 agent `gpu-texture`.
//!
//! Layout tracking: `Inner::layout` is the layout the last recorded use left the image in. The one-shot copies here
//! read it and write it back. The command module reads the same field when it records its own barriers.

use crate::device::Device;
use crate::dmabuf::{self, DmaBuf, ModifierInfo, MOD_LINEAR};
use crate::format::PixelFormat;
use crate::memory::memory_type_index;
use crate::{Error, Result};
use ash::vk;
use std::sync::{Arc, Mutex, MutexGuard};

/// What a texture is used for (combine with `|`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextureUsage(pub u32);

impl TextureUsage {
    /// Sampled in a shader.
    pub const SAMPLED: TextureUsage = TextureUsage(1);
    /// A render pass colour attachment.
    pub const RENDER_TARGET: TextureUsage = TextureUsage(2);
    /// A compute shader storage image.
    pub const STORAGE: TextureUsage = TextureUsage(4);
    /// The destination of uploads and copies.
    pub const COPY_DST: TextureUsage = TextureUsage(8);
    /// The source of copies and readback.
    pub const COPY_SRC: TextureUsage = TextureUsage(16);

    pub fn contains(self, other: TextureUsage) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for TextureUsage {
    type Output = TextureUsage;
    fn bitor(self, rhs: TextureUsage) -> TextureUsage {
        TextureUsage(self.0 | rhs.0)
    }
}

/// How to make a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub usage: TextureUsage,
    /// Allocate it exportable as a dma-buf (backing stores, scanout buffers).
    pub exportable: bool,
    /// The DRM modifiers the consumer accepts, most preferred first; empty means linear.
    pub modifiers: Vec<u64>,
}

/// A rectangle of texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    /// Null until the image is created; `Drop` destroys whatever was created, so a failed constructor cleans up.
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    /// One memory object (the planes of a modifier image share it). Freed by `Drop`.
    pub(crate) memory: Vec<vk::DeviceMemory>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: PixelFormat,
    pub(crate) usage: TextureUsage,
    pub(crate) modifier: Option<u64>,
    /// The layout the last recorded use left the image in (tracked by the command module).
    pub(crate) layout: Mutex<vk::ImageLayout>,
    /// Whether the image belongs to another process or the display (`VK_QUEUE_FAMILY_FOREIGN_EXT` transfers).
    pub(crate) foreign: bool,
}

impl Inner {
    /// An `Inner` with no Vulkan objects yet.
    pub(crate) fn shell(device: &Device, format: PixelFormat, width: u32, height: u32, usage: TextureUsage, foreign: bool, layout: vk::ImageLayout) -> Inner {
        Inner {
            device: device.clone(),
            image: vk::Image::null(),
            view: vk::ImageView::null(),
            memory: Vec::new(),
            width,
            height,
            format,
            usage,
            modifier: None,
            layout: Mutex::new(layout),
            foreign,
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let dev = &self.device.inner.device;
        // SAFETY: the texture is the last owner, so no command buffer still refers to these objects once the
        // submissions that use it have completed (submissions hold a clone of the texture until then).
        unsafe {
            if self.view != vk::ImageView::null() {
                dev.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                dev.destroy_image(self.image, None);
            }
            for memory in self.memory.drain(..) {
                if memory != vk::DeviceMemory::null() {
                    dev.free_memory(memory, None);
                }
            }
        }
    }
}

/// A GPU image. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct Texture {
    pub(crate) inner: Arc<Inner>,
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    pub fn height(&self) -> u32 {
        self.inner.height
    }

    pub fn format(&self) -> PixelFormat {
        self.inner.format
    }

    /// The DRM modifier the image was created or imported with, when it is shareable.
    pub fn modifier(&self) -> Option<u64> {
        self.inner.modifier
    }

    /// Exports the texture as a dma-buf (it must have been made `exportable`).
    pub fn export_dmabuf(&self) -> Result<DmaBuf> {
        dmabuf::export(self)
    }

    /// Uploads tightly packed (or `stride`-spaced) CPU pixels into `region`, through a staging buffer, and waits for
    /// the copy. For shm clients, the glyph atlas and decoded images.
    pub fn replace_region(&self, region: Region, bytes: &[u8], stride: usize) -> Result<()> {
        let inner = &*self.inner;
        if !inner.usage.contains(TextureUsage::COPY_DST) {
            return Err(Error::Invalid("replace_region needs COPY_DST usage".into()));
        }
        validate_region(region, inner.width, inner.height)?;
        let tight = pack_rows(bytes, stride, region.width, region.height, inner.format.bytes_per_pixel())?;
        let staging = Staging::new(&inner.device, tight.len(), vk::BufferUsageFlags::TRANSFER_SRC)?;
        staging.upload(&tight)?;
        transfer(inner, region, staging.buffer, true)
    }

    /// Reads `region` back to tightly packed CPU pixels (tests, screenshots, the bench). Waits for the GPU.
    pub fn read_region(&self, region: Region) -> Result<Vec<u8>> {
        let inner = &*self.inner;
        if !inner.usage.contains(TextureUsage::COPY_SRC) {
            return Err(Error::Invalid("read_region needs COPY_SRC usage".into()));
        }
        validate_region(region, inner.width, inner.height)?;
        let len = region.width as usize * region.height as usize * inner.format.bytes_per_pixel();
        let staging = Staging::new(&inner.device, len, vk::BufferUsageFlags::TRANSFER_DST)?;
        transfer(inner, region, staging.buffer, false)?;
        staging.download(len)
    }
}

/// Maps `TextureUsage` to Vulkan image usage flags.
pub(crate) fn usage_flags(usage: TextureUsage) -> vk::ImageUsageFlags {
    let mut flags = vk::ImageUsageFlags::empty();
    if usage.contains(TextureUsage::SAMPLED) {
        flags |= vk::ImageUsageFlags::SAMPLED;
    }
    if usage.contains(TextureUsage::RENDER_TARGET) {
        flags |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
    }
    if usage.contains(TextureUsage::STORAGE) {
        flags |= vk::ImageUsageFlags::STORAGE;
    }
    if usage.contains(TextureUsage::COPY_DST) {
        flags |= vk::ImageUsageFlags::TRANSFER_DST;
    }
    if usage.contains(TextureUsage::COPY_SRC) {
        flags |= vk::ImageUsageFlags::TRANSFER_SRC;
    }
    flags
}

/// The format features a modifier must report for `usage` to be created with it.
pub(crate) fn required_features(usage: TextureUsage) -> vk::FormatFeatureFlags {
    let mut flags = vk::FormatFeatureFlags::empty();
    if usage.contains(TextureUsage::SAMPLED) {
        flags |= vk::FormatFeatureFlags::SAMPLED_IMAGE;
    }
    if usage.contains(TextureUsage::RENDER_TARGET) {
        flags |= vk::FormatFeatureFlags::COLOR_ATTACHMENT;
    }
    if usage.contains(TextureUsage::STORAGE) {
        flags |= vk::FormatFeatureFlags::STORAGE_IMAGE;
    }
    if usage.contains(TextureUsage::COPY_DST) {
        flags |= vk::FormatFeatureFlags::TRANSFER_DST;
    }
    if usage.contains(TextureUsage::COPY_SRC) {
        flags |= vk::FormatFeatureFlags::TRANSFER_SRC;
    }
    flags
}

/// The candidate modifiers, in preference order, that the device reports with all `required` features.
pub(crate) fn pick_modifiers(candidates: &[u64], infos: &[ModifierInfo], required: vk::FormatFeatureFlags) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for &c in candidates {
        let ok = infos.iter().any(|i| i.modifier == c && i.features.contains(required));
        if ok && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// The layout a texture is left in after an upload or readback, for the usage it was made with.
pub(crate) fn natural_layout(usage: TextureUsage) -> vk::ImageLayout {
    let sampled = usage.contains(TextureUsage::SAMPLED);
    let render = usage.contains(TextureUsage::RENDER_TARGET);
    if usage.contains(TextureUsage::STORAGE) || (sampled && render) {
        vk::ImageLayout::GENERAL
    } else if sampled {
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    } else if render {
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    } else {
        vk::ImageLayout::GENERAL
    }
}

/// Checks that `region` is non-empty and lies inside a `width` x `height` image.
pub(crate) fn validate_region(region: Region, width: u32, height: u32) -> Result<()> {
    if region.width == 0 || region.height == 0 {
        return Err(Error::Invalid("empty region".into()));
    }
    let right = region.x as u64 + region.width as u64;
    let bottom = region.y as u64 + region.height as u64;
    if right > width as u64 || bottom > height as u64 {
        return Err(Error::Invalid(format!(
            "region {}x{} at {},{} outside {}x{}",
            region.width, region.height, region.x, region.y, width, height
        )));
    }
    Ok(())
}

/// Copies `height` rows of `width * bpp` bytes out of `bytes`, which are `stride` bytes apart, into a tight buffer.
pub(crate) fn pack_rows(bytes: &[u8], stride: usize, width: u32, height: u32, bpp: usize) -> Result<Vec<u8>> {
    let row = width as usize * bpp;
    if stride < row {
        return Err(Error::Invalid(format!("stride {stride} is less than a row of {row} bytes")));
    }
    if height == 0 {
        return Ok(Vec::new());
    }
    let needed = stride
        .checked_mul(height as usize - 1)
        .and_then(|n| n.checked_add(row))
        .ok_or_else(|| Error::Invalid("pixel data size overflows".into()))?;
    if bytes.len() < needed {
        return Err(Error::Invalid(format!("pixel data is {} bytes, the region needs {needed}", bytes.len())));
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for r in 0..height as usize {
        let start = r * stride;
        out.extend_from_slice(&bytes[start..start + row]);
    }
    Ok(out)
}

/// The memory type for an allocation: device-local when possible, any allowed type otherwise.
fn pick_memory_type(device: &Device, bits: u32) -> Result<u32> {
    let props = &device.inner.memory_properties;
    memory_type_index(props, bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
        .or_else(|| memory_type_index(props, bits, vk::MemoryPropertyFlags::empty()))
        .ok_or_else(|| Error::Unsupported("no memory type accepts the image".into()))
}

/// The colour view of an image.
pub(crate) fn create_view(dev: &ash::Device, image: vk::Image, format: PixelFormat) -> Result<vk::ImageView> {
    let info = vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(format.vk()).subresource_range(color_range());
    // SAFETY: the image is a live 2D colour image of `format`.
    Ok(unsafe { dev.create_image_view(&info, None) }?)
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 }
}

/// Makes a texture (`Device::new_texture`).
pub(crate) fn create(device: &Device, desc: &TextureDescriptor) -> Result<Texture> {
    if desc.width == 0 || desc.height == 0 {
        return Err(Error::Invalid(format!("texture size {}x{}", desc.width, desc.height)));
    }
    if desc.usage.0 == 0 {
        return Err(Error::Invalid("texture has no usage".into()));
    }
    if desc.exportable && desc.format.fourcc().is_none() {
        return Err(Error::Invalid(format!("{:?} has no DRM fourcc, so it cannot be exported", desc.format)));
    }
    let dev = &device.inner.device;
    let format = desc.format;
    let shared = desc.exportable || !desc.modifiers.is_empty();

    // Shared images use DRM modifier tiling: the candidates the device accepts for this usage, most preferred first.
    let modifiers: Vec<u64> = if shared {
        let infos = dmabuf::modifier_infos(device, format.vk())?;
        let candidates: Vec<u64> = if desc.modifiers.is_empty() { vec![MOD_LINEAR] } else { desc.modifiers.clone() };
        let picked = pick_modifiers(&candidates, &infos, required_features(desc.usage));
        if picked.is_empty() {
            return Err(Error::Unsupported(format!("no DRM modifier of {format:?} accepts usage {:#x}", desc.usage.0)));
        }
        picked
    } else {
        Vec::new()
    };

    let mut inner = Inner::shell(device, format, desc.width, desc.height, desc.usage, false, vk::ImageLayout::UNDEFINED);
    let mut ext = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut list = vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
    let mut info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format.vk())
        .extent(vk::Extent3D { width: desc.width, height: desc.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage_flags(desc.usage))
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    if shared {
        info = info.tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT).push_next(&mut list);
        if desc.exportable {
            info = info.push_next(&mut ext);
        }
    }
    // SAFETY: the create info is fully initialised and its chained structs outlive the call.
    inner.image = unsafe { dev.create_image(&info, None) }?;

    if shared {
        let drm = ash::ext::image_drm_format_modifier::Device::new(&device.inner.instance, dev);
        let mut props = vk::ImageDrmFormatModifierPropertiesEXT::default();
        // SAFETY: the image was created with DRM modifier tiling.
        unsafe { drm.get_image_drm_format_modifier_properties(inner.image, &mut props) }?;
        inner.modifier = Some(props.drm_format_modifier);
    }

    // SAFETY: the image handle is live.
    let req = unsafe { dev.get_image_memory_requirements(inner.image) };
    let index = pick_memory_type(device, req.memory_type_bits)?;
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(inner.image);
    let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(index).push_next(&mut dedicated);
    if desc.exportable {
        alloc = alloc.push_next(&mut export);
    }
    // SAFETY: the allocation info is fully initialised; the chained structs outlive the call.
    let memory = unsafe { dev.allocate_memory(&alloc, None) }?;
    inner.memory.push(memory);
    // SAFETY: the memory is at least `req.size` bytes, matches the image's requirements and is bound once.
    unsafe { dev.bind_image_memory(inner.image, memory, 0) }?;
    inner.view = create_view(dev, inner.image, format)?;
    Ok(Texture { inner: Arc::new(inner) })
}

/// Serialises queue submissions made from this module (a Vulkan queue is not thread-safe). The command module must
/// take the same lock when it submits to this device's queue.

fn lock_layout(inner: &Inner) -> MutexGuard<'_, vk::ImageLayout> {
    inner.layout.lock().unwrap_or_else(|e| e.into_inner())
}

/// A host-visible staging buffer, freed on drop.
pub(crate) struct Staging {
    device: Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    len: usize,
}

impl Staging {
    pub(crate) fn new(device: &Device, len: usize, usage: vk::BufferUsageFlags) -> Result<Staging> {
        let dev = &device.inner.device;
        let mut st = Staging { device: device.clone(), buffer: vk::Buffer::null(), memory: vk::DeviceMemory::null(), len };
        let info = vk::BufferCreateInfo::default().size(len as u64).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: the create info is fully initialised.
        st.buffer = unsafe { dev.create_buffer(&info, None) }?;
        // SAFETY: the buffer handle is live.
        let req = unsafe { dev.get_buffer_memory_requirements(st.buffer) };
        let props = &device.inner.memory_properties;
        let coherent = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        // Coherent memory only: `upload`/`download` do not flush or invalidate (GFX-6), as `Buffer` refuses too.
        let index = memory_type_index(props, req.memory_type_bits, coherent)
            .ok_or_else(|| Error::Unsupported("no host-visible memory for staging".into()))?;
        let alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(index);
        // SAFETY: the allocation info is fully initialised.
        st.memory = unsafe { dev.allocate_memory(&alloc, None) }?;
        // SAFETY: the memory is at least the buffer's requirement and is bound once.
        unsafe { dev.bind_buffer_memory(st.buffer, st.memory, 0) }?;
        Ok(st)
    }

    /// Copies `data` (at most the staging length) into the buffer.
    pub(crate) fn upload(&self, data: &[u8]) -> Result<()> {
        if data.len() > self.len {
            return Err(Error::Invalid("staging upload larger than the buffer".into()));
        }
        let dev = &self.device.inner.device;
        // SAFETY: the memory is host-visible and mapped for `len` bytes only while this function holds it; the copy
        // stays inside the mapping.
        unsafe {
            let ptr = dev.map_memory(self.memory, 0, self.len as u64, vk::MemoryMapFlags::empty())? as *mut u8;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            dev.unmap_memory(self.memory);
        }
        Ok(())
    }

    /// Copies the first `len` bytes of the buffer out.
    pub(crate) fn download(&self, len: usize) -> Result<Vec<u8>> {
        if len > self.len {
            return Err(Error::Invalid("staging download larger than the buffer".into()));
        }
        let dev = &self.device.inner.device;
        let mut out = vec![0u8; len];
        // SAFETY: as in `upload`; the read stays inside the mapping and `out` holds `len` bytes.
        unsafe {
            let ptr = dev.map_memory(self.memory, 0, self.len as u64, vk::MemoryMapFlags::empty())? as *const u8;
            std::ptr::copy_nonoverlapping(ptr, out.as_mut_ptr(), len);
            dev.unmap_memory(self.memory);
        }
        Ok(out)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let dev = &self.device.inner.device;
        // SAFETY: the staging buffer is only used by the one-shot submission that waited before this drop.
        unsafe {
            if self.buffer != vk::Buffer::null() {
                dev.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                dev.free_memory(self.memory, None);
            }
        }
    }
}

/// A transient command pool and fence for one submission, destroyed on drop.
struct OneShot {
    device: Device,
    pool: vk::CommandPool,
    fence: vk::Fence,
}

impl Drop for OneShot {
    fn drop(&mut self) {
        let dev = &self.device.inner.device;
        // SAFETY: the fence was waited on before this guard is dropped on the success path; on error paths nothing
        // was submitted that still runs.
        unsafe {
            if self.fence != vk::Fence::null() {
                dev.destroy_fence(self.fence, None);
            }
            if self.pool != vk::CommandPool::null() {
                dev.destroy_command_pool(self.pool, None);
            }
        }
    }
}

/// Records `record` into a one-shot command buffer on the device queue, submits it and waits for completion.
fn one_shot(device: &Device, record: impl FnOnce(&ash::Device, vk::CommandBuffer) -> Result<()>) -> Result<()> {
    let dev = &device.inner.device;
    let mut guard = OneShot { device: device.clone(), pool: vk::CommandPool::null(), fence: vk::Fence::null() };
    let pool_info = vk::CommandPoolCreateInfo::default().flags(vk::CommandPoolCreateFlags::TRANSIENT).queue_family_index(device.inner.queue_family);
    // SAFETY: the pool info is fully initialised; the pool is destroyed by `guard`.
    guard.pool = unsafe { dev.create_command_pool(&pool_info, None) }?;
    // SAFETY: the fence is created unsignalled and destroyed by `guard`.
    guard.fence = unsafe { dev.create_fence(&vk::FenceCreateInfo::default(), None) }?;
    let alloc = vk::CommandBufferAllocateInfo::default().command_pool(guard.pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
    // SAFETY: the pool is live and the allocation is one primary buffer.
    let cb = unsafe { dev.allocate_command_buffers(&alloc) }?[0];
    let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    // SAFETY: the buffer is freshly allocated and not recording.
    unsafe { dev.begin_command_buffer(cb, &begin) }?;
    record(dev, cb)?;
    // SAFETY: recording was begun above and every command was recorded by `record`.
    unsafe { dev.end_command_buffer(cb) }?;

    let infos = [vk::CommandBufferSubmitInfo::default().command_buffer(cb)];
    let submit = [vk::SubmitInfo2::default().command_buffer_infos(&infos)];
    device.submit(&submit, guard.fence)?;
    // SAFETY: the fence was submitted above; waiting makes the copy finished before the staging buffer is freed.
    unsafe { dev.wait_for_fences(&[guard.fence], true, u64::MAX) }?;
    Ok(())
}

/// One layout barrier for one image, with optional queue-family ownership transfer.
fn barrier(
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src: (vk::PipelineStageFlags2, vk::AccessFlags2),
    dst: (vk::PipelineStageFlags2, vk::AccessFlags2),
    queues: (u32, u32),
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src.0)
        .src_access_mask(src.1)
        .dst_stage_mask(dst.0)
        .dst_access_mask(dst.1)
        .old_layout(old)
        .new_layout(new)
        .src_queue_family_index(queues.0)
        .dst_queue_family_index(queues.1)
        .image(image)
        .subresource_range(color_range())
}

fn barrier_info<'a>(barriers: &'a [vk::ImageMemoryBarrier2<'a>]) -> vk::DependencyInfo<'a> {
    vk::DependencyInfo::default().image_memory_barriers(barriers)
}

/// Copies between `region` of the image and the staging buffer (`upload` is buffer to image, otherwise image to
/// buffer), with the layout transitions around it, and records the image's new layout.
fn transfer(inner: &Inner, region: Region, buffer: vk::Buffer, upload: bool) -> Result<()> {
    let image = inner.image;
    let mut layout = lock_layout(inner);
    let old = *layout;
    // An imported image is owned by the foreign queue family until it is acquired here.
    let (src_q, dst_q) = if inner.foreign {
        (vk::QUEUE_FAMILY_FOREIGN_EXT, inner.device.inner.queue_family)
    } else {
        (vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED)
    };
    let all = (vk::PipelineStageFlags2::ALL_COMMANDS, vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE);
    let copy = vk::BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 },
        image_offset: vk::Offset3D { x: region.x as i32, y: region.y as i32, z: 0 },
        image_extent: vk::Extent3D { width: region.width, height: region.height, depth: 1 },
    };
    let (work_layout, stage, access) = if upload {
        (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE)
    } else {
        (vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_READ)
    };
    let final_layout = if upload || old == vk::ImageLayout::UNDEFINED {
        natural_layout(inner.usage)
    } else {
        old
    };
    one_shot(&inner.device, |dev, cb| {
        let first = [barrier(image, old, work_layout, all, (stage, access), (src_q, dst_q))];
        // SAFETY: the command buffer is recording; the barrier names this image.
        unsafe { dev.cmd_pipeline_barrier2(cb, &barrier_info(&first)) };
        // SAFETY: the copy names the staging buffer and an image region inside the bounds `validate_region` checked.
        unsafe {
            if upload {
                dev.cmd_copy_buffer_to_image(cb, buffer, image, work_layout, &[copy]);
            } else {
                dev.cmd_copy_image_to_buffer(cb, image, work_layout, buffer, &[copy]);
            }
        }
        // An imported image goes back to the foreign family, so the next use acquires it exactly once (GFX-21).
        let back_queues = if inner.foreign { (inner.device.inner.queue_family, vk::QUEUE_FAMILY_FOREIGN_EXT) } else { (vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED) };
        let back = [barrier(image, work_layout, final_layout, (stage, access), all, back_queues)];
        // SAFETY: as above.
        unsafe { dev.cmd_pipeline_barrier2(cb, &barrier_info(&back)) };
        Ok(())
    })?;
    *layout = final_layout;
    Ok(())
}

#[cfg(all(test, feature = "gpu-tests"))]
mod gpu_tests {
    use super::*;

    fn device() -> Device {
        Device::open(None).expect("a Vulkan 1.3 device with the dma-buf extensions")
    }

    fn descriptor(usage: TextureUsage, exportable: bool) -> TextureDescriptor {
        TextureDescriptor { width: 4, height: 4, format: PixelFormat::Bgra8Unorm, usage, exportable, modifiers: Vec::new() }
    }

    #[test]
    fn upload_then_read_round_trips_tight_pixels() {
        let dev = device();
        let tex = dev.new_texture(&descriptor(TextureUsage::SAMPLED | TextureUsage::COPY_DST | TextureUsage::COPY_SRC, false)).unwrap();
        let pixels: Vec<u8> = (0..64).collect();
        let full = Region { x: 0, y: 0, width: 4, height: 4 };
        tex.replace_region(full, &pixels, 16).unwrap();
        assert_eq!(tex.read_region(full).unwrap(), pixels);
    }

    #[test]
    fn a_sub_region_upload_leaves_the_rest_alone() {
        let dev = device();
        let tex = dev.new_texture(&descriptor(TextureUsage::COPY_DST | TextureUsage::COPY_SRC, false)).unwrap();
        let zeros = vec![0u8; 64];
        tex.replace_region(Region { x: 0, y: 0, width: 4, height: 4 }, &zeros, 16).unwrap();
        let ones = vec![0xffu8; 8];
        tex.replace_region(Region { x: 1, y: 1, width: 2, height: 1 }, &ones, 8).unwrap();
        let back = tex.read_region(Region { x: 1, y: 1, width: 2, height: 1 }).unwrap();
        assert_eq!(back, ones);
        assert_eq!(tex.read_region(Region { x: 0, y: 0, width: 1, height: 1 }).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn an_exported_texture_imports_with_the_same_pixels() {
        let dev = device();
        let usage = TextureUsage::SAMPLED | TextureUsage::COPY_DST | TextureUsage::COPY_SRC;
        let tex = dev.new_texture(&descriptor(usage, true)).unwrap();
        let full = Region { x: 0, y: 0, width: 4, height: 4 };
        let pixels: Vec<u8> = (0..64).map(|v| v as u8 ^ 0x5a).collect();
        tex.replace_region(full, &pixels, 16).unwrap();
        let buf = tex.export_dmabuf().unwrap();
        assert_eq!(buf.fourcc, PixelFormat::Bgra8Unorm.fourcc().unwrap());
        let imported = dev.import_dmabuf(&buf).unwrap();
        assert_eq!(imported.modifier(), Some(buf.modifier));
        assert_eq!(imported.read_region(full).unwrap(), pixels);
    }

    #[test]
    fn supported_formats_include_bgra_linear_for_sampling() {
        let dev = device();
        let formats = dev.supported_dmabuf_formats();
        let argb = PixelFormat::Bgra8Unorm.fourcc().unwrap();
        assert!(formats.iter().any(|f| f.fourcc == argb && f.modifier == crate::dmabuf::MOD_LINEAR), "{formats:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(modifier: u64, features: vk::FormatFeatureFlags) -> ModifierInfo {
        ModifierInfo { modifier, planes: 1, features }
    }

    #[test]
    fn usage_maps_to_image_usage_flags() {
        assert_eq!(usage_flags(TextureUsage::SAMPLED), vk::ImageUsageFlags::SAMPLED);
        assert_eq!(
            usage_flags(TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC | TextureUsage::COPY_DST),
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST
        );
        assert_eq!(usage_flags(TextureUsage::STORAGE), vk::ImageUsageFlags::STORAGE);
        assert_eq!(usage_flags(TextureUsage::default()), vk::ImageUsageFlags::empty());
    }

    #[test]
    fn usage_requires_the_matching_format_features() {
        let f = required_features(TextureUsage::SAMPLED | TextureUsage::COPY_DST);
        assert_eq!(f, vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::TRANSFER_DST);
        assert_eq!(required_features(TextureUsage::STORAGE), vk::FormatFeatureFlags::STORAGE_IMAGE);
    }

    #[test]
    fn modifiers_are_kept_in_preference_order_when_they_have_the_features() {
        let sampled = vk::FormatFeatureFlags::SAMPLED_IMAGE;
        let infos = [info(1, sampled), info(2, vk::FormatFeatureFlags::empty()), info(3, sampled)];
        assert_eq!(pick_modifiers(&[3, 2, 1], &infos, sampled), vec![3, 1]);
    }

    #[test]
    fn modifiers_missing_a_feature_or_the_device_are_dropped() {
        let sampled = vk::FormatFeatureFlags::SAMPLED_IMAGE;
        let infos = [info(1, sampled), info(2, vk::FormatFeatureFlags::TRANSFER_SRC)];
        assert!(pick_modifiers(&[2, 9], &infos, sampled).is_empty());
        assert_eq!(pick_modifiers(&[1, 1, 1], &infos, sampled), vec![1]);
    }

    #[test]
    fn natural_layouts_for_each_usage() {
        assert_eq!(natural_layout(TextureUsage::SAMPLED), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        assert_eq!(natural_layout(TextureUsage::RENDER_TARGET), vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        assert_eq!(natural_layout(TextureUsage::STORAGE | TextureUsage::SAMPLED), vk::ImageLayout::GENERAL);
        assert_eq!(natural_layout(TextureUsage::SAMPLED | TextureUsage::RENDER_TARGET), vk::ImageLayout::GENERAL);
        assert_eq!(natural_layout(TextureUsage::COPY_DST), vk::ImageLayout::GENERAL);
    }

    #[test]
    fn region_must_be_non_empty_and_inside() {
        let full = Region { x: 0, y: 0, width: 64, height: 32 };
        assert!(validate_region(full, 64, 32).is_ok());
        assert!(validate_region(Region { x: 60, y: 0, width: 4, height: 32 }, 64, 32).is_ok());
        assert!(validate_region(Region { x: 60, y: 0, width: 5, height: 32 }, 64, 32).is_err());
        assert!(validate_region(Region { x: 0, y: 0, width: 0, height: 4 }, 64, 32).is_err());
        assert!(validate_region(Region { x: u32::MAX, y: 0, width: u32::MAX, height: 1 }, 64, 32).is_err());
    }

    #[test]
    fn pack_rows_drops_the_padding_between_rows() {
        // 2 rows of 2 pixels x 2 bytes, stride 8: the last 4 bytes of each stride are padding.
        let src = [1, 2, 3, 4, 90, 91, 92, 93, 5, 6, 7, 8];
        let out = pack_rows(&src, 8, 2, 2, 2).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn pack_rows_accepts_a_tight_buffer_without_the_trailing_padding() {
        // The last row needs only width * bpp bytes, not a full stride.
        let src = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let out = pack_rows(&src[..12], 8, 2, 2, 2);
        assert_eq!(out.unwrap(), vec![1, 2, 3, 4, 9, 10, 11, 12]);
    }

    #[test]
    fn pack_rows_rejects_short_data_and_small_strides() {
        assert!(pack_rows(&[0; 7], 8, 2, 2, 2).is_err());
        assert!(pack_rows(&[0; 64], 3, 2, 2, 2).is_err());
        assert_eq!(pack_rows(&[], 16, 4, 0, 4).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn create_view_and_range_are_colour_mip_zero_layer_zero() {
        let r = color_range();
        assert_eq!((r.base_mip_level, r.level_count, r.base_array_layer, r.layer_count), (0, 1, 0, 1));
        assert_eq!(r.aspect_mask, vk::ImageAspectFlags::COLOR);
    }

    #[test]
    fn barrier_carries_the_queue_families() {
        let b = barrier(
            vk::Image::null(),
            vk::ImageLayout::GENERAL,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            (vk::PipelineStageFlags2::ALL_COMMANDS, vk::AccessFlags2::empty()),
            (vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE),
            (vk::QUEUE_FAMILY_FOREIGN_EXT, 3),
        );
        assert_eq!(b.src_queue_family_index, vk::QUEUE_FAMILY_FOREIGN_EXT);
        assert_eq!(b.dst_queue_family_index, 3);
        assert_eq!(b.new_layout, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
    }
}
