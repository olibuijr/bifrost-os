//! The device: Vulkan instance, the physical device matching the compositor's DRM render node, one graphics+compute
//! queue, and the factory for every resource (the reference API's device object).
//!
//! Owner: wave G1 agent `gpu-device`. Contract: docs/graphics-architecture.md, "bifrost-gpu".

use crate::{Buffer, BufferUsage, CommandQueue, ComputePipeline, ComputePipelineDescriptor, DmaBuf, DmaBufFormat, RenderPipeline, RenderPipelineDescriptor, Result, Sampler, SamplerDescriptor, SharedEvent, Texture, TextureDescriptor};
use std::path::Path;
use std::sync::Arc;

/// What the device is, for logs, About and the bench history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The GPU name the driver reports ("Intel(R) HD Graphics 530 (SKL GT2)").
    pub name: String,
    /// The driver name and version ("Intel open-source Mesa driver 26.2.4").
    pub driver: String,
    /// Vulkan API version as (major, minor, patch).
    pub api: (u32, u32, u32),
    /// The DRM render node major and minor (from `VK_EXT_physical_device_drm`), when known.
    pub render_node: Option<(u32, u32)>,
    /// Whether `VK_KHR_push_descriptor` is enabled.
    pub push_descriptors: bool,
}

/// The shared device state. Owned by `Device` through an `Arc`; resources keep the `Arc` alive.
pub(crate) struct Inner {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) physical: ash::vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    pub(crate) queue: ash::vk::Queue,
    pub(crate) queue_family: u32,
    pub(crate) info: DeviceInfo,
    pub(crate) memory_properties: ash::vk::PhysicalDeviceMemoryProperties,
}

/// A GPU. Cheap to clone; every resource made from it keeps it alive.
#[derive(Clone)]
pub struct Device {
    pub(crate) inner: Arc<Inner>,
}

impl Device {
    /// Opens the GPU behind `render_node` (`/dev/dri/renderD128`), or the first suitable GPU when `None`.
    /// Fails with `Error::Unsupported` when the loader, Vulkan 1.3 or a required extension is missing.
    pub fn open(render_node: Option<&Path>) -> Result<Device> {
        let _ = render_node;
        todo!("gpu-device")
    }

    /// What the device is.
    pub fn info(&self) -> &DeviceInfo {
        &self.inner.info
    }

    /// A command queue on the device's graphics+compute queue.
    pub fn new_command_queue(&self) -> CommandQueue {
        todo!("gpu-command")
    }

    /// A texture with its own memory.
    pub fn new_texture(&self, desc: &TextureDescriptor) -> Result<Texture> {
        let _ = desc;
        todo!("gpu-texture")
    }

    /// A texture that samples (or renders into) another process's dma-buf, without a copy.
    pub fn import_dmabuf(&self, dmabuf: &DmaBuf) -> Result<Texture> {
        let _ = dmabuf;
        todo!("gpu-texture")
    }

    /// The fourcc and modifier pairs this device can import for sampling, for linux-dmabuf v4 feedback.
    pub fn supported_dmabuf_formats(&self) -> Vec<DmaBufFormat> {
        todo!("gpu-texture")
    }

    /// A buffer of `len` bytes.
    pub fn new_buffer(&self, len: usize, usage: BufferUsage) -> Result<Buffer> {
        let _ = (len, usage);
        todo!("gpu-memory")
    }

    /// A render pipeline (vertex + fragment shader, blend, vertex layout).
    pub fn new_render_pipeline(&self, desc: &RenderPipelineDescriptor) -> Result<RenderPipeline> {
        let _ = desc;
        todo!("gpu-pipeline")
    }

    /// A compute pipeline.
    pub fn new_compute_pipeline(&self, desc: &ComputePipelineDescriptor) -> Result<ComputePipeline> {
        let _ = desc;
        todo!("gpu-pipeline")
    }

    /// A sampler.
    pub fn new_sampler(&self, desc: &SamplerDescriptor) -> Result<Sampler> {
        let _ = desc;
        todo!("gpu-pipeline")
    }

    /// A timeline semaphore starting at 0.
    pub fn new_shared_event(&self) -> Result<SharedEvent> {
        todo!("gpu-sync")
    }

    /// Waits until the device has finished all submitted work (teardown and tests only).
    pub fn wait_idle(&self) -> Result<()> {
        todo!("gpu-device")
    }
}
