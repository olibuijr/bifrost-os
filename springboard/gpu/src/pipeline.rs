//! Pipelines and samplers. Descriptor set 0 holds textures and samplers by binding number; push constants carry
//! per-draw bytes. Pipelines use dynamic rendering (no render pass objects) and dynamic viewport and scissor.
//!
//! Owner: wave G1 agent `gpu-pipeline`.

use crate::PixelFormat;

/// How a pipeline blends into the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// No blending (the source replaces the target).
    Replace,
    /// Premultiplied source-over (the default for everything BifrOSt draws).
    PremultipliedOver,
    /// Additive (glow, debug overlays).
    Add,
}

/// The per-instance vertex data layout: `stride` bytes per instance, attributes as (location, format, offset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexLayout {
    pub stride: u32,
    pub attributes: Vec<(u32, ash::vk::Format, u32)>,
    /// Attributes advance per instance (true) or per vertex (false).
    pub per_instance: bool,
}

/// How to make a render pipeline.
#[derive(Debug, Clone)]
pub struct RenderPipelineDescriptor {
    /// Shader names from `shader::shader` (SPIR-V built from gpu/shaders/<name>.vert and .frag).
    pub vertex: &'static str,
    pub fragment: &'static str,
    pub target_format: PixelFormat,
    pub blend: BlendMode,
    pub vertex_layout: Option<VertexLayout>,
    /// Triangle strip (true, for instanced quads) or triangle list.
    pub strip: bool,
    /// Number of texture bindings and sampler bindings in set 0.
    pub textures: u32,
    pub samplers: u32,
    /// Push constant bytes (at most 128).
    pub push_constant_bytes: u32,
}

/// How to make a compute pipeline.
#[derive(Debug, Clone)]
pub struct ComputePipelineDescriptor {
    pub shader: &'static str,
    /// Bindings in set 0: sampled textures, storage images, samplers.
    pub sampled_textures: u32,
    pub storage_textures: u32,
    pub samplers: u32,
    pub push_constant_bytes: u32,
}

/// Sampler filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

/// How to make a sampler (always clamp to edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerDescriptor {
    pub filter: Filter,
}

pub(crate) struct RenderInner {
    pub(crate) device: crate::Device,
    pub(crate) pipeline: ash::vk::Pipeline,
    pub(crate) layout: ash::vk::PipelineLayout,
    pub(crate) set_layout: ash::vk::DescriptorSetLayout,
    pub(crate) desc: RenderPipelineDescriptor,
}

pub(crate) struct ComputeInner {
    pub(crate) device: crate::Device,
    pub(crate) pipeline: ash::vk::Pipeline,
    pub(crate) layout: ash::vk::PipelineLayout,
    pub(crate) set_layout: ash::vk::DescriptorSetLayout,
    pub(crate) desc: ComputePipelineDescriptor,
}

#[derive(Clone)]
pub struct RenderPipeline {
    pub(crate) inner: std::sync::Arc<RenderInner>,
}

#[derive(Clone)]
pub struct ComputePipeline {
    pub(crate) inner: std::sync::Arc<ComputeInner>,
}

pub(crate) struct SamplerInner {
    pub(crate) device: crate::Device,
    pub(crate) sampler: ash::vk::Sampler,
}

#[derive(Clone)]
pub struct Sampler {
    pub(crate) inner: std::sync::Arc<SamplerInner>,
}
