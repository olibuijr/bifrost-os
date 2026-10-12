//! Pipelines and samplers. Descriptor set 0 holds textures and samplers by binding number; push constants carry
//! per-draw bytes. Pipelines use dynamic rendering (no render pass objects) and dynamic viewport and scissor.
//!
//! Binding map (set 0), shared with the encoders in `command.rs`:
//! - render pipeline: sampled texture `i` at binding `i` (`texture_binding`), sampler `j` at binding
//!   `textures + j` (`sampler_binding`); both visible to the vertex and fragment stages;
//! - compute pipeline: sampled texture `i` at binding `i`, storage image `k` at `sampled + k`
//!   (`compute_storage_binding`), sampler `j` at `sampled + storage + j` (`compute_sampler_binding`).
//!
//! Every pipeline is built through the device's pipeline cache (one `VkPipelineCache` per device, see `cache_for`).
//!
//! Owner: wave G1 agent `gpu-pipeline`.

use crate::shader::{self, Stage};
use crate::{Device, Error, PixelFormat, Result};
use ash::vk;
use std::sync::{Arc, Mutex};

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

/// Push constants are at most 128 bytes (the minimum `maxPushConstantsSize` every Vulkan 1.3 device provides).
pub const MAX_PUSH_CONSTANT_BYTES: u32 = 128;
/// Upper bound on textures or samplers of one kind in a pipeline, so descriptor sets stay small.
pub const MAX_DESCRIPTORS_PER_KIND: u32 = 32;
/// Upper bound on the vertex stride of an instance layout.
pub const MAX_VERTEX_STRIDE: u32 = 2048;

const MAIN: &std::ffi::CStr = c"main";

/// The binding of sampled texture `index` in set 0 (both pipeline kinds).
pub fn texture_binding(index: u32) -> u32 {
    index
}

/// The binding of sampler `index` of a render pipeline with `textures` sampled textures.
pub fn sampler_binding(textures: u32, index: u32) -> u32 {
    textures + index
}

/// The binding of storage image `index` of a compute pipeline with `sampled` sampled textures.
pub fn compute_storage_binding(sampled: u32, index: u32) -> u32 {
    sampled + index
}

/// The binding of sampler `index` of a compute pipeline with `sampled` sampled textures and `storage` storage images.
pub fn compute_sampler_binding(sampled: u32, storage: u32, index: u32) -> u32 {
    sampled + storage + index
}

/// Blend factors of a mode as (source, destination), used for colour and alpha alike. `None` disables blending.
pub(crate) fn blend_factors(mode: BlendMode) -> Option<(vk::BlendFactor, vk::BlendFactor)> {
    match mode {
        BlendMode::Replace => None,
        // Premultiplied colour: result = src + dst * (1 - src.a).
        BlendMode::PremultipliedOver => Some((vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA)),
        // result = src + dst.
        BlendMode::Add => Some((vk::BlendFactor::ONE, vk::BlendFactor::ONE)),
    }
}

/// The colour attachment blend state for a mode; every channel is written.
pub(crate) fn attachment_state(mode: BlendMode) -> vk::PipelineColorBlendAttachmentState {
    let state = vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);
    match blend_factors(mode) {
        None => state,
        Some((src, dst)) => state
            .blend_enable(true)
            .src_color_blend_factor(src)
            .dst_color_blend_factor(dst)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(src)
            .dst_alpha_blend_factor(dst)
            .alpha_blend_op(vk::BlendOp::ADD),
    }
}

/// Triangle strip for instanced quads, otherwise a triangle list.
pub(crate) fn topology(strip: bool) -> vk::PrimitiveTopology {
    if strip {
        vk::PrimitiveTopology::TRIANGLE_STRIP
    } else {
        vk::PrimitiveTopology::TRIANGLE_LIST
    }
}

/// The vertex input bindings and attributes of a layout: one binding (0) with the layout's stride and rate.
/// No layout means no vertex input (the shader generates its vertices from `gl_VertexIndex`).
pub(crate) fn vertex_input(layout: Option<&VertexLayout>) -> (Vec<vk::VertexInputBindingDescription>, Vec<vk::VertexInputAttributeDescription>) {
    let Some(layout) = layout else { return (Vec::new(), Vec::new()) };
    let rate = if layout.per_instance { vk::VertexInputRate::INSTANCE } else { vk::VertexInputRate::VERTEX };
    let bindings = vec![vk::VertexInputBindingDescription { binding: 0, stride: layout.stride, input_rate: rate }];
    let attributes = layout
        .attributes
        .iter()
        .map(|&(location, format, offset)| vk::VertexInputAttributeDescription { location, binding: 0, format, offset })
        .collect();
    (bindings, attributes)
}

/// Checks a push constant byte count: a multiple of four, at most 128.
pub(crate) fn validate_push_constants(bytes: u32) -> Result<()> {
    if bytes > MAX_PUSH_CONSTANT_BYTES || bytes % 4 != 0 {
        return Err(Error::Invalid(format!("push constant bytes {bytes}: must be a multiple of 4 and at most {MAX_PUSH_CONSTANT_BYTES}")));
    }
    Ok(())
}

fn validate_count(kind: &str, n: u32) -> Result<()> {
    if n > MAX_DESCRIPTORS_PER_KIND {
        return Err(Error::Invalid(format!("{n} {kind} bindings: at most {MAX_DESCRIPTORS_PER_KIND} per pipeline")));
    }
    Ok(())
}

fn validate_vertex_layout(layout: &VertexLayout) -> Result<()> {
    if layout.stride == 0 || layout.stride > MAX_VERTEX_STRIDE || layout.stride % 4 != 0 {
        return Err(Error::Invalid(format!("vertex stride {}: must be a multiple of 4 in 4..={MAX_VERTEX_STRIDE}", layout.stride)));
    }
    for &(location, _, offset) in &layout.attributes {
        if offset >= layout.stride {
            return Err(Error::Invalid(format!("vertex attribute {location} at offset {offset} lies outside the {} byte stride", layout.stride)));
        }
    }
    Ok(())
}

/// Every check a render pipeline descriptor must pass before any Vulkan object is made.
pub(crate) fn validate_render(desc: &RenderPipelineDescriptor) -> Result<()> {
    shader::expect_stage(desc.vertex, Stage::Vertex)?;
    shader::expect_stage(desc.fragment, Stage::Fragment)?;
    validate_count("texture", desc.textures)?;
    validate_count("sampler", desc.samplers)?;
    validate_push_constants(desc.push_constant_bytes)?;
    if let Some(layout) = &desc.vertex_layout {
        validate_vertex_layout(layout)?;
    }
    Ok(())
}

/// Every check a compute pipeline descriptor must pass before any Vulkan object is made.
pub(crate) fn validate_compute(desc: &ComputePipelineDescriptor) -> Result<()> {
    shader::expect_stage(desc.shader, Stage::Compute)?;
    validate_count("sampled texture", desc.sampled_textures)?;
    validate_count("storage texture", desc.storage_textures)?;
    validate_count("sampler", desc.samplers)?;
    validate_push_constants(desc.push_constant_bytes)
}

/// The set 0 layout of a render pipeline: sampled images then samplers, both for the vertex and fragment stages.
pub(crate) fn render_bindings(textures: u32, samplers: u32) -> Vec<vk::DescriptorSetLayoutBinding<'static>> {
    let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
    let mut out = Vec::with_capacity((textures + samplers) as usize);
    for i in 0..textures {
        out.push(binding(texture_binding(i), vk::DescriptorType::SAMPLED_IMAGE, stages));
    }
    for j in 0..samplers {
        out.push(binding(sampler_binding(textures, j), vk::DescriptorType::SAMPLER, stages));
    }
    out
}

/// The set 0 layout of a compute pipeline: sampled images, storage images, then samplers, all for the compute stage.
pub(crate) fn compute_bindings(sampled: u32, storage: u32, samplers: u32) -> Vec<vk::DescriptorSetLayoutBinding<'static>> {
    let stage = vk::ShaderStageFlags::COMPUTE;
    let mut out = Vec::new();
    for i in 0..sampled {
        out.push(binding(texture_binding(i), vk::DescriptorType::SAMPLED_IMAGE, stage));
    }
    for k in 0..storage {
        out.push(binding(compute_storage_binding(sampled, k), vk::DescriptorType::STORAGE_IMAGE, stage));
    }
    for j in 0..samplers {
        out.push(binding(compute_sampler_binding(sampled, storage, j), vk::DescriptorType::SAMPLER, stage));
    }
    out
}

fn binding(binding: u32, kind: vk::DescriptorType, stages: vk::ShaderStageFlags) -> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default().binding(binding).descriptor_type(kind).descriptor_count(1).stage_flags(stages)
}

/// The push constant range covering `bytes` for `stages`, or none when there are no push constants.
pub(crate) fn push_ranges(bytes: u32, stages: vk::ShaderStageFlags) -> Vec<vk::PushConstantRange> {
    if bytes == 0 {
        Vec::new()
    } else {
        vec![vk::PushConstantRange { stage_flags: stages, offset: 0, size: bytes }]
    }
}

/// The sampler state of a descriptor: nearest or linear, clamped to the edge on every axis, no mips, no anisotropy.
pub(crate) fn sampler_info(desc: &SamplerDescriptor) -> vk::SamplerCreateInfo<'static> {
    let filter = match desc.filter {
        Filter::Nearest => vk::Filter::NEAREST,
        Filter::Linear => vk::Filter::LINEAR,
    };
    vk::SamplerCreateInfo::default()
        .mag_filter(filter)
        .min_filter(filter)
        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .min_lod(0.0)
        .max_lod(0.0)
        .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK)
}

/// The per-device pipeline cache. It lives as long as the process: the registry holds the device, so the cache is
/// destroyed before the device it was made on.
struct CacheOwner {
    device: Device,
    raw: vk::PipelineCache,
}

impl Drop for CacheOwner {
    fn drop(&mut self) {
        // SAFETY: the cache was created on `self.device`, which this struct keeps alive, and no pipeline built from
        // it can still be in use: pipelines do not reference the cache after creation.
        unsafe { self.device.inner.device.destroy_pipeline_cache(self.raw, None) };
    }
}

static CACHES: Mutex<Vec<Arc<CacheOwner>>> = Mutex::new(Vec::new());

/// Runs `f` with the device's pipeline cache. Creation is serialised under the registry lock because a
/// `VkPipelineCache` needs external synchronisation.
fn with_cache<T>(device: &Device, f: impl FnOnce(vk::PipelineCache) -> Result<T>) -> Result<T> {
    let mut all = CACHES.lock().unwrap_or_else(|p| p.into_inner());
    let raw = match all.iter().find(|c| Arc::ptr_eq(&c.device.inner, &device.inner)) {
        Some(c) => c.raw,
        None => {
            let info = vk::PipelineCacheCreateInfo::default();
            // SAFETY: `info` is valid for the call and the device is alive (`device` is borrowed).
            let raw = unsafe { device.inner.device.create_pipeline_cache(&info, None) }?;
            all.push(Arc::new(CacheOwner { device: device.clone(), raw }));
            raw
        }
    };
    f(raw)
}

/// The set 0 layout. With push descriptors enabled on the device the encoders push bindings
/// (`vkCmdPushDescriptorSetKHR`), which needs the layout to be created for it.
fn make_set_layout(d: &ash::Device, bindings: &[vk::DescriptorSetLayoutBinding<'_>], push_descriptors: bool) -> Result<vk::DescriptorSetLayout> {
    let flags = if push_descriptors { vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR } else { vk::DescriptorSetLayoutCreateFlags::empty() };
    let info = vk::DescriptorSetLayoutCreateInfo::default().flags(flags).bindings(bindings);
    // SAFETY: `info` and the bindings it points to are valid for the call; the layout is destroyed by the owner.
    Ok(unsafe { d.create_descriptor_set_layout(&info, None) }?)
}

fn make_pipeline_layout(d: &ash::Device, set: vk::DescriptorSetLayout, push: &[vk::PushConstantRange]) -> Result<vk::PipelineLayout> {
    let sets = [set];
    let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&sets).push_constant_ranges(push);
    // SAFETY: `info` and its slices are valid for the call; the layout is destroyed by the owner.
    Ok(unsafe { d.create_pipeline_layout(&info, None) }?)
}

fn make_module(d: &ash::Device, name: &str) -> Result<vk::ShaderModule> {
    let words = shader::shader(name)
        .ok_or_else(|| Error::Unsupported(format!("shader {name} is not built (glslc missing or the compile failed; see the build warnings)")))?;
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    // SAFETY: `words` outlives the call; the module is destroyed by the caller after pipeline creation.
    Ok(unsafe { d.create_shader_module(&info, None) }?)
}

fn stage_info(stage: vk::ShaderStageFlags, module: vk::ShaderModule) -> vk::PipelineShaderStageCreateInfo<'static> {
    vk::PipelineShaderStageCreateInfo::default().stage(stage).module(module).name(MAIN)
}

/// Creates a render pipeline. Every created handle is destroyed on the error paths.
pub(crate) fn new_render(device: &Device, desc: &RenderPipelineDescriptor) -> Result<RenderPipeline> {
    validate_render(desc)?;
    let d = &device.inner.device;
    let set_layout = make_set_layout(d, &render_bindings(desc.textures, desc.samplers), device.inner.info.push_descriptors)?;
    let push = push_ranges(desc.push_constant_bytes, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
    let layout = match make_pipeline_layout(d, set_layout, &push) {
        Ok(l) => l,
        Err(e) => {
            // SAFETY: the set layout was created above on this device and nothing uses it yet.
            unsafe { d.destroy_descriptor_set_layout(set_layout, None) };
            return Err(e);
        }
    };
    let built = with_cache(device, |cache| build_render(d, cache, desc, layout));
    match built {
        Ok(pipeline) => Ok(RenderPipeline {
            inner: Arc::new(RenderInner { device: device.clone(), pipeline, layout, set_layout, desc: desc.clone() }),
        }),
        Err(e) => {
            // SAFETY: the layout and set layout were created above and no pipeline refers to them.
            unsafe {
                d.destroy_pipeline_layout(layout, None);
                d.destroy_descriptor_set_layout(set_layout, None);
            }
            Err(e)
        }
    }
}

fn build_render(d: &ash::Device, cache: vk::PipelineCache, desc: &RenderPipelineDescriptor, layout: vk::PipelineLayout) -> Result<vk::Pipeline> {
    let vs = make_module(d, desc.vertex)?;
    let fs = match make_module(d, desc.fragment) {
        Ok(m) => m,
        Err(e) => {
            // SAFETY: `vs` was created above and is used by nothing.
            unsafe { d.destroy_shader_module(vs, None) };
            return Err(e);
        }
    };
    let result = create_render_pipeline(d, cache, desc, layout, vs, fs);
    // SAFETY: the modules are no longer needed once the pipeline is created (or creation failed).
    unsafe {
        d.destroy_shader_module(vs, None);
        d.destroy_shader_module(fs, None);
    }
    result
}

fn create_render_pipeline(
    d: &ash::Device,
    cache: vk::PipelineCache,
    desc: &RenderPipelineDescriptor,
    layout: vk::PipelineLayout,
    vs: vk::ShaderModule,
    fs: vk::ShaderModule,
) -> Result<vk::Pipeline> {
    let stages = [stage_info(vk::ShaderStageFlags::VERTEX, vs), stage_info(vk::ShaderStageFlags::FRAGMENT, fs)];
    let (bindings, attributes) = vertex_input(desc.vertex_layout.as_ref());
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default().vertex_binding_descriptions(&bindings).vertex_attribute_descriptions(&attributes);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(topology(desc.strip));
    let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let attachments = [attachment_state(desc.blend)];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    // Dynamic rendering: the colour target's format is named here instead of a render pass object.
    let formats = [desc.target_format.vk()];
    let mut rendering = vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .push_next(&mut rendering);
    // SAFETY: every struct and slice in `info` lives until this call returns; the modules and layout are valid.
    let created = unsafe { d.create_graphics_pipelines(cache, std::slice::from_ref(&info), None) };
    match created {
        Ok(v) => Ok(v[0]),
        Err((pipelines, e)) => {
            // SAFETY: pipelines returned on failure are valid handles that nothing else owns.
            for p in pipelines {
                unsafe { d.destroy_pipeline(p, None) };
            }
            Err(e.into())
        }
    }
}

/// Creates a compute pipeline.
pub(crate) fn new_compute(device: &Device, desc: &ComputePipelineDescriptor) -> Result<ComputePipeline> {
    validate_compute(desc)?;
    let d = &device.inner.device;
    let set_layout = make_set_layout(d, &compute_bindings(desc.sampled_textures, desc.storage_textures, desc.samplers), device.inner.info.push_descriptors)?;
    let push = push_ranges(desc.push_constant_bytes, vk::ShaderStageFlags::COMPUTE);
    let layout = match make_pipeline_layout(d, set_layout, &push) {
        Ok(l) => l,
        Err(e) => {
            // SAFETY: the set layout was created above and nothing uses it yet.
            unsafe { d.destroy_descriptor_set_layout(set_layout, None) };
            return Err(e);
        }
    };
    let built = with_cache(device, |cache| build_compute(d, cache, desc, layout));
    match built {
        Ok(pipeline) => Ok(ComputePipeline {
            inner: Arc::new(ComputeInner { device: device.clone(), pipeline, layout, set_layout, desc: desc.clone() }),
        }),
        Err(e) => {
            // SAFETY: the layout and set layout were created above and no pipeline refers to them.
            unsafe {
                d.destroy_pipeline_layout(layout, None);
                d.destroy_descriptor_set_layout(set_layout, None);
            }
            Err(e)
        }
    }
}

fn build_compute(d: &ash::Device, cache: vk::PipelineCache, desc: &ComputePipelineDescriptor, layout: vk::PipelineLayout) -> Result<vk::Pipeline> {
    let module = make_module(d, desc.shader)?;
    let info = vk::ComputePipelineCreateInfo::default().stage(stage_info(vk::ShaderStageFlags::COMPUTE, module)).layout(layout);
    // SAFETY: `info` and the module are valid for the call; the module is destroyed below either way.
    let created = unsafe { d.create_compute_pipelines(cache, std::slice::from_ref(&info), None) };
    // SAFETY: the module is no longer needed once creation has returned.
    unsafe { d.destroy_shader_module(module, None) };
    match created {
        Ok(v) => Ok(v[0]),
        Err((pipelines, e)) => {
            // SAFETY: pipelines returned on failure are valid handles that nothing else owns.
            for p in pipelines {
                unsafe { d.destroy_pipeline(p, None) };
            }
            Err(e.into())
        }
    }
}

/// Creates a sampler (clamp to edge on every axis).
pub(crate) fn new_sampler(device: &Device, desc: &SamplerDescriptor) -> Result<Sampler> {
    let info = sampler_info(desc);
    // SAFETY: `info` is valid for the call; the sampler is destroyed by `SamplerInner::drop`.
    let sampler = unsafe { device.inner.device.create_sampler(&info, None) }?;
    Ok(Sampler { inner: Arc::new(SamplerInner { device: device.clone(), sampler }) })
}

impl Drop for RenderInner {
    fn drop(&mut self) {
        let d = &self.device.inner.device;
        // SAFETY: the objects were created on this device, which `self.device` keeps alive. The command buffers that
        // used this pipeline hold an `Arc` (`Keep::Render`) until their submission completes, so nothing in flight
        // refers to these handles.
        unsafe {
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

impl Drop for ComputeInner {
    fn drop(&mut self) {
        let d = &self.device.inner.device;
        // SAFETY: as for `RenderInner`: created on this device, kept alive by `self.device` and by in-flight
        // `Keep::Compute` references until completion.
        unsafe {
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

impl Drop for SamplerInner {
    fn drop(&mut self) {
        // SAFETY: created on `self.device`, kept alive by it; in-flight command buffers hold an `Arc` (`Keep::Sampler`).
        unsafe { self.device.inner.device.destroy_sampler(self.sampler, None) };
    }
}

impl RenderPipeline {
    /// The descriptor this pipeline was made from.
    pub fn descriptor(&self) -> &RenderPipelineDescriptor {
        &self.inner.desc
    }
}

impl ComputePipeline {
    /// The descriptor this pipeline was made from.
    pub fn descriptor(&self) -> &ComputePipelineDescriptor {
        &self.inner.desc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_desc() -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            vertex: "quad.vert",
            fragment: "quad.frag",
            target_format: PixelFormat::Bgra8Unorm,
            blend: BlendMode::PremultipliedOver,
            vertex_layout: None,
            strip: true,
            textures: 2,
            samplers: 1,
            push_constant_bytes: 64,
        }
    }

    fn compute_desc() -> ComputePipelineDescriptor {
        ComputePipelineDescriptor { shader: "kawase_down.comp", sampled_textures: 1, storage_textures: 1, samplers: 1, push_constant_bytes: 16 }
    }

    #[test]
    fn premultiplied_over_is_source_one_destination_one_minus_source_alpha() {
        assert_eq!(blend_factors(BlendMode::PremultipliedOver), Some((vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA)));
    }

    #[test]
    fn add_blends_one_and_one_and_replace_disables_blending() {
        assert_eq!(blend_factors(BlendMode::Add), Some((vk::BlendFactor::ONE, vk::BlendFactor::ONE)));
        assert_eq!(blend_factors(BlendMode::Replace), None);
        assert_eq!(attachment_state(BlendMode::Replace).blend_enable, vk::FALSE);
    }

    #[test]
    fn attachment_state_writes_all_channels_and_sets_factors_for_both_colour_and_alpha() {
        let s = attachment_state(BlendMode::PremultipliedOver);
        assert_eq!(s.blend_enable, vk::TRUE);
        assert_eq!(s.color_write_mask, vk::ColorComponentFlags::RGBA);
        assert_eq!(s.src_alpha_blend_factor, vk::BlendFactor::ONE);
        assert_eq!(s.dst_alpha_blend_factor, vk::BlendFactor::ONE_MINUS_SRC_ALPHA);
        assert_eq!(s.color_blend_op, vk::BlendOp::ADD);
        assert_eq!(s.alpha_blend_op, vk::BlendOp::ADD);
    }

    #[test]
    fn topology_is_strip_or_list() {
        assert_eq!(topology(true), vk::PrimitiveTopology::TRIANGLE_STRIP);
        assert_eq!(topology(false), vk::PrimitiveTopology::TRIANGLE_LIST);
    }

    #[test]
    fn no_vertex_layout_means_no_vertex_input() {
        let (b, a) = vertex_input(None);
        assert!(b.is_empty() && a.is_empty());
    }

    #[test]
    fn instance_layout_maps_to_one_binding_with_instance_rate() {
        let layout = VertexLayout {
            stride: 32,
            attributes: vec![(0, vk::Format::R32G32_SFLOAT, 0), (1, vk::Format::R32G32B32A32_SFLOAT, 16)],
            per_instance: true,
        };
        let (bindings, attrs) = vertex_input(Some(&layout));
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].binding, 0);
        assert_eq!(bindings[0].stride, 32);
        assert_eq!(bindings[0].input_rate, vk::VertexInputRate::INSTANCE);
        assert_eq!(attrs.len(), 2);
        assert_eq!((attrs[1].location, attrs[1].binding, attrs[1].offset), (1, 0, 16));
        assert_eq!(attrs[1].format, vk::Format::R32G32B32A32_SFLOAT);
    }

    #[test]
    fn per_vertex_layout_uses_vertex_rate() {
        let layout = VertexLayout { stride: 8, attributes: vec![(0, vk::Format::R32G32_SFLOAT, 0)], per_instance: false };
        let (bindings, _) = vertex_input(Some(&layout));
        assert_eq!(bindings[0].input_rate, vk::VertexInputRate::VERTEX);
    }

    #[test]
    fn push_constants_must_be_a_multiple_of_four_up_to_128() {
        assert!(validate_push_constants(0).is_ok());
        assert!(validate_push_constants(128).is_ok());
        assert!(validate_push_constants(6).is_err());
        assert!(validate_push_constants(132).is_err());
    }

    #[test]
    fn push_range_covers_the_bytes_or_is_absent() {
        assert!(push_ranges(0, vk::ShaderStageFlags::COMPUTE).is_empty());
        let r = push_ranges(48, vk::ShaderStageFlags::COMPUTE);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].offset, r[0].size), (0, 48));
        assert_eq!(r[0].stage_flags, vk::ShaderStageFlags::COMPUTE);
    }

    #[test]
    fn render_descriptor_passes_when_stages_and_limits_match() {
        assert!(validate_render(&render_desc()).is_ok());
    }

    #[test]
    fn render_descriptor_rejects_a_fragment_shader_in_the_vertex_slot() {
        let mut d = render_desc();
        d.vertex = "quad.frag";
        assert!(matches!(validate_render(&d), Err(Error::Invalid(_))));
    }

    #[test]
    fn render_descriptor_rejects_too_many_bindings_and_bad_strides() {
        let mut d = render_desc();
        d.textures = MAX_DESCRIPTORS_PER_KIND + 1;
        assert!(validate_render(&d).is_err());
        let mut d = render_desc();
        d.vertex_layout = Some(VertexLayout { stride: 30, attributes: vec![], per_instance: true });
        assert!(validate_render(&d).is_err());
        let mut d = render_desc();
        d.vertex_layout = Some(VertexLayout { stride: 16, attributes: vec![(0, vk::Format::R32_SFLOAT, 16)], per_instance: true });
        assert!(validate_render(&d).is_err());
    }

    #[test]
    fn compute_descriptor_needs_a_compute_shader() {
        assert!(validate_compute(&compute_desc()).is_ok());
        let mut d = compute_desc();
        d.shader = "quad.vert";
        assert!(validate_compute(&d).is_err());
    }

    #[test]
    fn render_bindings_are_textures_then_samplers_from_zero() {
        let b = render_bindings(2, 1);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].binding, b[0].descriptor_type), (0, vk::DescriptorType::SAMPLED_IMAGE));
        assert_eq!((b[1].binding, b[1].descriptor_type), (1, vk::DescriptorType::SAMPLED_IMAGE));
        // The sampler follows the two textures.
        assert_eq!((b[2].binding, b[2].descriptor_type), (2, vk::DescriptorType::SAMPLER));
        assert!(b.iter().all(|x| x.descriptor_count == 1));
        assert_eq!(sampler_binding(2, 0), 2);
    }

    #[test]
    fn compute_bindings_are_sampled_then_storage_then_samplers() {
        let b = compute_bindings(1, 2, 1);
        let kinds: Vec<(u32, vk::DescriptorType)> = b.iter().map(|x| (x.binding, x.descriptor_type)).collect();
        assert_eq!(
            kinds,
            vec![
                (0, vk::DescriptorType::SAMPLED_IMAGE),
                (1, vk::DescriptorType::STORAGE_IMAGE),
                (2, vk::DescriptorType::STORAGE_IMAGE),
                (3, vk::DescriptorType::SAMPLER),
            ]
        );
        assert!(b.iter().all(|x| x.stage_flags == vk::ShaderStageFlags::COMPUTE));
        assert_eq!(compute_storage_binding(1, 1), 2);
        assert_eq!(compute_sampler_binding(1, 2, 0), 3);
    }

    #[test]
    fn binding_helpers_are_contiguous() {
        // Textures, then samplers, never overlap for any counts.
        for t in 0..5 {
            for s in 0..5 {
                let b = render_bindings(t, s);
                let mut nums: Vec<u32> = b.iter().map(|x| x.binding).collect();
                nums.sort();
                assert_eq!(nums, (0..t + s).collect::<Vec<u32>>());
            }
        }
    }

    #[test]
    fn sampler_is_clamped_to_edge_with_the_chosen_filter() {
        let near = sampler_info(&SamplerDescriptor { filter: Filter::Nearest });
        assert_eq!((near.mag_filter, near.min_filter), (vk::Filter::NEAREST, vk::Filter::NEAREST));
        let lin = sampler_info(&SamplerDescriptor { filter: Filter::Linear });
        assert_eq!((lin.mag_filter, lin.min_filter), (vk::Filter::LINEAR, vk::Filter::LINEAR));
        for s in [near, lin] {
            assert_eq!(s.address_mode_u, vk::SamplerAddressMode::CLAMP_TO_EDGE);
            assert_eq!(s.address_mode_v, vk::SamplerAddressMode::CLAMP_TO_EDGE);
            assert_eq!(s.address_mode_w, vk::SamplerAddressMode::CLAMP_TO_EDGE);
            assert_eq!(s.anisotropy_enable, vk::FALSE);
            assert_eq!(s.max_lod, 0.0);
        }
    }

    #[test]
    fn pixel_format_maps_to_its_vulkan_target_format() {
        assert_eq!(PixelFormat::Bgra8Unorm.vk(), vk::Format::B8G8R8A8_UNORM);
        assert_eq!(PixelFormat::Rgba16Float.vk(), vk::Format::R16G16B16A16_SFLOAT);
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn gpu_render_and_compute_pipelines_build_on_the_device() {
        let dev = Device::open(None).expect("a Vulkan device");
        let r = dev.new_render_pipeline(&render_desc());
        // Shaders are optional on machines without glslc; the pipeline then reports Unsupported, never a panic.
        match r {
            Ok(p) => assert_eq!(p.descriptor().textures, 2),
            Err(Error::Unsupported(_)) => {}
            Err(e) => panic!("render pipeline: {e}"),
        }
        let _ = dev.new_compute_pipeline(&compute_desc());
        assert!(dev.new_sampler(&SamplerDescriptor { filter: Filter::Linear }).is_ok());
    }
}
