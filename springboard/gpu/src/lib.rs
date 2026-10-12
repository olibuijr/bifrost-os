//! `bifrost-gpu`: a small, safe, Metal-shaped GPU API over Vulkan 1.3 (docs/graphics-architecture.md, "bifrost-gpu").
//!
//! The window server (`bifrost-wm`) and the SDK (`bifrost-2d`) draw through this crate only; it is the one place with
//! `unsafe` Vulkan calls. The shape follows the reference platform's GPU API: a `Device` makes resources and command
//! queues, a `CommandBuffer` records render and compute passes through encoders and is committed to its queue, and a
//! `SharedEvent` (a timeline semaphore) synchronises work with other processes and the display through sync_files.
//!
//! Module owners (wave G1): `device` (instance, physical device, queue), `texture` and `dmabuf` (images, dma-buf
//! import/export, modifiers), `memory` (allocation), `command` (queues, command buffers, encoders), `sync` (fences,
//! timeline semaphores, sync_file), `pipeline` and `shader` (SPIR-V, pipelines, samplers).

pub mod command;
pub mod device;
pub mod dmabuf;
pub mod format;
pub mod memory;
pub mod pipeline;
pub mod shader;
pub mod sync;
pub mod texture;

pub use command::{CommandBuffer, CommandQueue, ComputeEncoder, LoadAction, RenderEncoder, RenderPassDescriptor, StoreAction, Submission};
pub use device::{Device, DeviceInfo};
pub use dmabuf::{DmaBuf, DmaBufFormat, DmaBufPlane};
pub use format::PixelFormat;
pub use memory::{Buffer, BufferUsage};
pub use pipeline::{BlendMode, ComputePipeline, ComputePipelineDescriptor, RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerDescriptor, VertexLayout};
pub use sync::SharedEvent;
pub use texture::{Region, Texture, TextureDescriptor, TextureUsage};

/// Everything that can fail in this crate. Vulkan result codes are kept for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The Vulkan loader or a required extension or feature is missing; the caller falls back to the CPU path.
    Unsupported(String),
    /// A Vulkan call failed with this result code.
    Vulkan(i32),
    /// The device was lost (GPU reset or hang); every resource of the device is now invalid.
    DeviceLost,
    /// Out of device or host memory.
    OutOfMemory,
    /// A dma-buf could not be imported or exported (format, modifier or plane layout not accepted).
    DmaBuf(String),
    /// An argument broke the contract (size zero, region outside the texture, wrong usage).
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(s) => write!(f, "GPU unsupported: {s}"),
            Error::Vulkan(c) => write!(f, "Vulkan error {c}"),
            Error::DeviceLost => write!(f, "GPU device lost"),
            Error::OutOfMemory => write!(f, "GPU out of memory"),
            Error::DmaBuf(s) => write!(f, "dma-buf: {s}"),
            Error::Invalid(s) => write!(f, "invalid GPU call: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ash::vk::Result> for Error {
    fn from(r: ash::vk::Result) -> Self {
        match r {
            ash::vk::Result::ERROR_DEVICE_LOST => Error::DeviceLost,
            ash::vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | ash::vk::Result::ERROR_OUT_OF_HOST_MEMORY => Error::OutOfMemory,
            other => Error::Vulkan(other.as_raw()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
