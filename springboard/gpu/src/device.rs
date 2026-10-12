//! The device: Vulkan instance, the physical device matching the compositor's DRM render node, one graphics+compute
//! queue, and the factory for every resource (the reference API's device object).
//!
//! Owner: wave G1 agent `gpu-device`. Contract: docs/graphics-architecture.md, "bifrost-gpu".

use crate::{Buffer, BufferUsage, CommandQueue, ComputePipeline, ComputePipelineDescriptor, DmaBuf, DmaBufFormat, Error, RenderPipeline, RenderPipelineDescriptor, Result, Sampler, SamplerDescriptor, SharedEvent, Texture, TextureDescriptor};
use ash::vk;
use std::ffi::{c_char, CStr};
use std::path::Path;
use std::sync::Arc;

/// The validation layer, loaded only when `BIFROST_GPU_VALIDATE=1`.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";
const APP_NAME: &CStr = c"bifrost-gpu";

/// Device extensions every device must have (docs/graphics-architecture.md, "bifrost-gpu").
const REQUIRED_EXTENSIONS: [&CStr; 6] = [
    vk::KHR_EXTERNAL_MEMORY_FD_NAME,
    vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME,
    vk::EXT_IMAGE_DRM_FORMAT_MODIFIER_NAME,
    vk::KHR_EXTERNAL_SEMAPHORE_FD_NAME,
    vk::EXT_PHYSICAL_DEVICE_DRM_NAME,
    vk::EXT_QUEUE_FAMILY_FOREIGN_NAME,
];

/// Enabled when the driver has it (optional).
const OPTIONAL_PUSH_DESCRIPTOR: &CStr = vk::KHR_PUSH_DESCRIPTOR_NAME;

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

impl Drop for Inner {
    fn drop(&mut self) {
        // SAFETY: every resource holds an `Arc<Inner>`, so none is alive when this runs. Waiting first makes the
        // destruction below legal even if work was submitted and never waited on. The device is destroyed before
        // the instance it was created from, as the specification requires.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
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
        let want = match render_node {
            Some(path) => Some(render_node_numbers(path)?),
            None => None,
        };
        // SAFETY: loads the system Vulkan loader library; the returned `Entry` owns it.
        let entry = unsafe { ash::Entry::load() }.map_err(|e| Error::Unsupported(format!("Vulkan loader: {e}")))?;
        // SAFETY: no pointers are passed; the loader is valid for the duration of this call.
        let loader_version = unsafe { entry.try_enumerate_instance_version() }.map_err(Error::from)?;
        let loader = match loader_version {
            Some(v) => split_api_version(v),
            None => (1, 0, 0),
        };
        if loader < (1, 3, 0) {
            return Err(Error::Unsupported(format!("Vulkan 1.3 required, the loader provides {}.{}", loader.0, loader.1)));
        }
        let validate = validation_requested(std::env::var("BIFROST_GPU_VALIDATE").ok().as_deref());
        if validate {
            // SAFETY: no pointers are passed; the returned list is owned by Rust.
            let layers = unsafe { entry.enumerate_instance_layer_properties() }.map_err(Error::from)?;
            let found = layers.iter().any(|l| name_of(&l.layer_name).as_bytes() == VALIDATION_LAYER.to_bytes());
            if !found {
                return Err(Error::Unsupported("BIFROST_GPU_VALIDATE=1 but layer VK_LAYER_KHRONOS_validation is not installed".into()));
            }
        }

        let app = vk::ApplicationInfo::default().application_name(APP_NAME).application_version(1).engine_name(APP_NAME).api_version(vk::API_VERSION_1_3);
        let layer_ptrs: Vec<*const c_char> = if validate { vec![VALIDATION_LAYER.as_ptr()] } else { Vec::new() };
        let create = vk::InstanceCreateInfo::default().application_info(&app).enabled_layer_names(&layer_ptrs);
        // SAFETY: `create` points at `app`, `layer_ptrs` and the static names above, all alive for this call.
        let instance = unsafe { entry.create_instance(&create, None) }.map_err(Error::from)?;
        let guard = InstanceGuard(Some(instance));

        let opened = open_on(guard.get(), want)?;
        let instance = guard.into_inner();
        Ok(Device {
            inner: Arc::new(Inner {
                entry,
                instance,
                physical: opened.physical,
                device: opened.device,
                queue: opened.queue,
                queue_family: opened.family,
                info: opened.info,
                memory_properties: opened.memory,
            }),
        })
    }

    /// What the device is.
    pub fn info(&self) -> &DeviceInfo {
        &self.inner.info
    }

    /// A command queue on the device's graphics+compute queue.
    pub fn new_command_queue(&self) -> CommandQueue {
        CommandQueue::new(self.clone())
    }

    /// A texture with its own memory.
    pub fn new_texture(&self, desc: &TextureDescriptor) -> Result<Texture> {
        crate::texture::create(self, desc)
    }

    /// A texture that samples (or renders into) another process's dma-buf, without a copy.
    pub fn import_dmabuf(&self, dmabuf: &DmaBuf) -> Result<Texture> {
        crate::dmabuf::import(self, dmabuf)
    }

    /// The fourcc and modifier pairs this device can import for sampling, for linux-dmabuf v4 feedback.
    pub fn supported_dmabuf_formats(&self) -> Vec<DmaBufFormat> {
        crate::dmabuf::supported_formats(self)
    }

    /// A buffer of `len` bytes.
    pub fn new_buffer(&self, len: usize, usage: BufferUsage) -> Result<Buffer> {
        crate::memory::new_buffer(self, len, usage)
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
        crate::sync::create(self)
    }

    /// Waits until the device has finished all submitted work (teardown and tests only).
    pub fn wait_idle(&self) -> Result<()> {
        // SAFETY: the device handle is valid while `self.inner` is alive.
        unsafe { self.inner.device.device_wait_idle() }.map_err(Error::from)
    }
}

/// Owns the instance until `open` succeeds, so an error on the way destroys it.
struct InstanceGuard(Option<ash::Instance>);

impl InstanceGuard {
    fn get(&self) -> &ash::Instance {
        self.0.as_ref().expect("instance guard is only emptied by into_inner")
    }

    fn into_inner(mut self) -> ash::Instance {
        self.0.take().expect("instance guard is only emptied by into_inner")
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(instance) = self.0.take() {
            // SAFETY: no device was created from this instance (the guard is dropped only on failure before that).
            unsafe { instance.destroy_instance(None) };
        }
    }
}

/// What `open_on` found and created on the chosen physical device.
struct Opened {
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    info: DeviceInfo,
    memory: vk::PhysicalDeviceMemoryProperties,
}

/// Probes every physical device, picks one, and creates the logical device with one graphics+compute queue.
fn open_on(instance: &ash::Instance, want: Option<(u32, u32)>) -> Result<Opened> {
    // SAFETY: the instance is valid; the returned handles belong to it.
    let pdevs = unsafe { instance.enumerate_physical_devices() }.map_err(Error::from)?;
    let mut probes = Vec::with_capacity(pdevs.len());
    for &pdev in &pdevs {
        probes.push(probe(instance, pdev)?);
    }
    let candidates: Vec<Candidate> = probes.iter().map(|p| p.candidate.clone()).collect();
    let index = choose_device(&candidates, want)?;
    let probe = &probes[index];
    let family = probe.candidate.queue_family.expect("choose_device only picks devices with a queue family");

    let priorities = [1.0f32];
    let queue_info = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
    let mut ext: Vec<*const c_char> = REQUIRED_EXTENSIONS.iter().map(|e| e.as_ptr()).collect();
    if probe.push_descriptors {
        ext.push(OPTIONAL_PUSH_DESCRIPTOR.as_ptr());
    }
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true).synchronization2(true);
    let mut f12 = vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
    let create = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_info)
        .enabled_extension_names(&ext)
        .push_next(&mut f13)
        .push_next(&mut f12);
    // SAFETY: `create` and everything it points at (`queue_info`, `priorities`, `ext`, the feature structs) are alive
    // for the call; the physical device came from this instance and passed the checks in `probe` and `choose_device`.
    let device = unsafe { instance.create_device(probe.pdev, &create, None) }.map_err(Error::from)?;
    // SAFETY: the family and index were requested in `queue_info` above.
    let queue = unsafe { device.get_device_queue(family, 0) };
    // SAFETY: the physical device belongs to this instance.
    let memory = unsafe { instance.get_physical_device_memory_properties(probe.pdev) };

    let info = DeviceInfo {
        name: probe.name.clone(),
        driver: probe.driver.clone(),
        api: probe.api,
        render_node: probe.candidate.render,
        push_descriptors: probe.push_descriptors,
    };
    Ok(Opened { physical: probe.pdev, device, queue, family, info, memory })
}

/// Everything learned about one physical device before the choice.
struct Probe {
    pdev: vk::PhysicalDevice,
    candidate: Candidate,
    name: String,
    driver: String,
    api: (u32, u32, u32),
    push_descriptors: bool,
}

/// Queries one physical device: name, driver, API, DRM node, extensions, features and queue families.
fn probe(instance: &ash::Instance, pdev: vk::PhysicalDevice) -> Result<Probe> {
    // SAFETY: `pdev` came from `enumerate_physical_devices` on this instance.
    let exts = unsafe { instance.enumerate_device_extension_properties(pdev) }.map_err(Error::from)?;
    let names: Vec<String> = exts.iter().map(|e| name_of(&e.extension_name)).collect();
    let has = |want: &CStr| names.iter().any(|n| n.as_bytes() == want.to_bytes());
    let has_drm = has(vk::EXT_PHYSICAL_DEVICE_DRM_NAME);

    // Properties chain: the DRM and driver structs are only chained when the device advertises them.
    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut driver = vk::PhysicalDeviceDriverProperties::default();
    let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut driver);
    if has_drm {
        props = props.push_next(&mut drm);
    }
    // SAFETY: the chain points at structs that live until the end of this function.
    unsafe { instance.get_physical_device_properties2(pdev, &mut props) };
    let api = split_api_version(props.properties.api_version);
    let name = name_of(&props.properties.device_name);
    let device_type = props.properties.device_type;
    let driver_text = join_nonempty(&name_of(&driver.driver_name), &name_of(&driver.driver_info));
    let render = if has_drm && drm.has_render != 0 { Some((drm.render_major as u32, drm.render_minor as u32)) } else { None };

    let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut f2 = vk::PhysicalDeviceFeatures2::default().push_next(&mut f13).push_next(&mut f12);
    // SAFETY: the chain points at feature structs that live until the end of this function.
    unsafe { instance.get_physical_device_features2(pdev, &mut f2) };
    let feature_flags = [
        (f13.dynamic_rendering == vk::TRUE, "dynamic rendering"),
        (f13.synchronization2 == vk::TRUE, "synchronization2"),
        (f12.timeline_semaphore == vk::TRUE, "timeline semaphores"),
    ];

    // SAFETY: `pdev` belongs to this instance.
    let families = unsafe { instance.get_physical_device_queue_family_properties(pdev) };
    let flags: Vec<(vk::QueueFlags, u32)> = families.iter().map(|f| (f.queue_flags, f.queue_count)).collect();
    let queue_family = graphics_compute_family(&flags);

    let mut missing = missing_extensions(&names, &REQUIRED_EXTENSIONS);
    missing.extend(missing_features(&feature_flags));

    let candidate = Candidate {
        name: name.clone(),
        rank: device_rank(device_type),
        api,
        render,
        missing,
        queue_family,
    };
    Ok(Probe {
        pdev,
        candidate,
        name,
        driver: driver_text,
        api,
        push_descriptors: has(OPTIONAL_PUSH_DESCRIPTOR),
    })
}

/// One physical device as the choice sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub name: String,
    /// 0 discrete, 1 integrated, 2 virtual, 3 CPU, 4 other: lower is preferred.
    pub rank: u32,
    pub api: (u32, u32, u32),
    /// The DRM render node (major, minor), when `VK_EXT_physical_device_drm` reports one.
    pub render: Option<(u32, u32)>,
    /// Required extensions and features this device lacks, by name.
    pub missing: Vec<String>,
    /// The graphics+compute queue family, when it has one.
    pub queue_family: Option<u32>,
}

/// What stops a candidate from being used, empty when it is usable.
fn problems(c: &Candidate) -> Vec<String> {
    let mut p = Vec::new();
    if c.api < (1, 3, 0) {
        p.push(format!("Vulkan 1.3 (device has {}.{}.{})", c.api.0, c.api.1, c.api.2));
    }
    p.extend(c.missing.iter().cloned());
    if c.queue_family.is_none() {
        p.push("a graphics+compute queue family".to_string());
    }
    p
}

/// Picks the candidate index: the one whose render node is `want`, or else the best usable one (discrete first).
/// Fails with `Unsupported` naming what is missing.
pub(crate) fn choose_device(cands: &[Candidate], want: Option<(u32, u32)>) -> Result<usize> {
    if cands.is_empty() {
        return Err(Error::Unsupported("no Vulkan physical device".into()));
    }
    match want {
        Some((major, minor)) => {
            let Some(i) = cands.iter().position(|c| c.render == Some((major, minor))) else {
                return Err(Error::Unsupported(format!("no Vulkan device for render node {major}:{minor}")));
            };
            let p = problems(&cands[i]);
            if p.is_empty() {
                Ok(i)
            } else {
                Err(Error::Unsupported(format!("GPU {} lacks {}", cands[i].name, p.join(", "))))
            }
        }
        None => {
            let best = cands
                .iter()
                .enumerate()
                .filter(|(_, c)| problems(c).is_empty())
                .min_by_key(|(_, c)| c.rank)
                .map(|(i, _)| i);
            match best {
                Some(i) => Ok(i),
                None => {
                    let why: Vec<String> = cands.iter().map(|c| format!("{} lacks {}", c.name, problems(c).join(", "))).collect();
                    Err(Error::Unsupported(format!("no usable Vulkan GPU: {}", why.join("; "))))
                }
            }
        }
    }
}

/// The device-type preference rank (lower is better).
fn device_rank(t: vk::PhysicalDeviceType) -> u32 {
    if t == vk::PhysicalDeviceType::DISCRETE_GPU {
        0
    } else if t == vk::PhysicalDeviceType::INTEGRATED_GPU {
        1
    } else if t == vk::PhysicalDeviceType::VIRTUAL_GPU {
        2
    } else if t == vk::PhysicalDeviceType::CPU {
        3
    } else {
        4
    }
}

/// The first queue family (index, count > 0) with both graphics and compute.
pub(crate) fn graphics_compute_family(families: &[(vk::QueueFlags, u32)]) -> Option<u32> {
    let want = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE;
    families.iter().position(|(flags, count)| flags.contains(want) && *count > 0).map(|i| i as u32)
}

/// Required names that are not in `available`, in the order given.
pub(crate) fn missing_extensions(available: &[String], required: &[&CStr]) -> Vec<String> {
    required
        .iter()
        .filter(|r| !available.iter().any(|a| a.as_bytes() == r.to_bytes()))
        .map(|r| r.to_string_lossy().into_owned())
        .collect()
}

/// The names of the features that are not granted.
pub(crate) fn missing_features(flags: &[(bool, &str)]) -> Vec<String> {
    flags.iter().filter(|(ok, _)| !ok).map(|(_, name)| name.to_string()).collect()
}

/// Whether the validation layer was requested: only the exact value `1` turns it on.
pub(crate) fn validation_requested(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Splits a Vulkan version word into (major, minor, patch).
pub(crate) fn split_api_version(v: u32) -> (u32, u32, u32) {
    ((v >> 22) & 0x7f, (v >> 12) & 0x3ff, v & 0xfff)
}

/// The major and minor device numbers of a device node (`st_rdev`), split the way glibc's `major()` and `minor()` do.
pub(crate) fn dev_numbers(rdev: u64) -> (u32, u32) {
    let major = ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfffu64);
    let minor = (rdev & 0xff) | ((rdev >> 12) & !0xffu64);
    (major as u32, minor as u32)
}

/// The DRM render node's major and minor, from the node's file.
fn render_node_numbers(path: &Path) -> Result<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).map_err(|e| Error::Unsupported(format!("render node {}: {e}", path.display())))?;
    Ok(dev_numbers(meta.rdev()))
}

/// A NUL-terminated name from a fixed-size Vulkan char array, as text.
pub(crate) fn name_of(raw: &[c_char]) -> String {
    let bytes: Vec<u8> = raw.iter().map(|&c| c as u8).take_while(|&b| b != 0).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// "a b", or just the non-empty one of the two.
fn join_nonempty(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(name: &str, rank: u32, render: Option<(u32, u32)>, missing: &[&str], family: Option<u32>) -> Candidate {
        Candidate {
            name: name.into(),
            rank,
            api: (1, 3, 281),
            render,
            missing: missing.iter().map(|s| s.to_string()).collect(),
            queue_family: family,
        }
    }

    #[test]
    fn api_version_word_splits_into_major_minor_patch() {
        assert_eq!(vk::API_VERSION_1_3, 0x0040_3000);
        assert_eq!(split_api_version(vk::API_VERSION_1_3), (1, 3, 0));
        assert_eq!(split_api_version(0x0040_3000 + 281), (1, 3, 281));
        // Variant bits (29..31) are ignored: variant 0 with 1.4.2 encoded by hand.
        assert_eq!(split_api_version((1 << 22) | (4 << 12) | 2), (1, 4, 2));
        assert_eq!(split_api_version(0), (0, 0, 0));
    }

    #[test]
    fn device_numbers_match_glibc_major_minor() {
        // makedev(226, 128) = 226 << 8 | 128 = 57984: the first DRM render node.
        assert_eq!(dev_numbers(57984), (226, 128));
        assert_eq!(dev_numbers(0), (0, 0));
        // Minor 256 needs the high minor bits: (256 & !0xff) << 12 = 1 << 20.
        assert_eq!(dev_numbers(1 << 20), (0, 256));
        // Major 4096 needs the high major bits: (4096 & !0xfff) << 32 = 1 << 44.
        assert_eq!(dev_numbers(1 << 44), (4096, 0));
    }

    #[test]
    fn validation_layer_only_for_exact_one() {
        assert!(validation_requested(Some("1")));
        assert!(!validation_requested(Some("0")));
        assert!(!validation_requested(Some("yes")));
        assert!(!validation_requested(Some("")));
        assert!(!validation_requested(None));
    }

    #[test]
    fn missing_extensions_lists_only_the_absent_ones_in_order() {
        let available = vec!["VK_KHR_a".to_string(), "VK_EXT_b".to_string()];
        let required = [c"VK_EXT_b", c"VK_KHR_c", c"VK_KHR_a", c"VK_EXT_d"];
        assert_eq!(missing_extensions(&available, &required), vec!["VK_KHR_c".to_string(), "VK_EXT_d".to_string()]);
        assert!(missing_extensions(&available, &[c"VK_KHR_a"]).is_empty());
    }

    #[test]
    fn missing_features_names_each_ungranted_feature() {
        let flags = [(true, "dynamic rendering"), (false, "synchronization2"), (false, "timeline semaphores")];
        assert_eq!(missing_features(&flags), vec!["synchronization2".to_string(), "timeline semaphores".to_string()]);
        assert!(missing_features(&[(true, "x")]).is_empty());
    }

    #[test]
    fn queue_family_needs_graphics_and_compute_with_queues() {
        let g = vk::QueueFlags::GRAPHICS;
        let c = vk::QueueFlags::COMPUTE;
        let t = vk::QueueFlags::TRANSFER;
        // Index 0 has no compute, index 1 has no queues left, index 2 is the match.
        assert_eq!(graphics_compute_family(&[(g, 1), (g | c, 0), (g | c | t, 2)]), Some(2));
        assert_eq!(graphics_compute_family(&[(t, 4), (c, 2)]), None);
        assert_eq!(graphics_compute_family(&[]), None);
    }

    #[test]
    fn name_of_stops_at_the_nul() {
        let mut raw = [0 as c_char; 8];
        for (i, b) in b"Mesa\0xx".iter().enumerate().take(5) {
            raw[i] = *b as c_char;
        }
        assert_eq!(name_of(&raw), "Mesa");
        assert_eq!(name_of(&[0 as c_char; 4]), "");
    }

    #[test]
    fn join_skips_an_empty_half() {
        assert_eq!(join_nonempty("Intel open-source Mesa driver", "Mesa 26.2.4"), "Intel open-source Mesa driver Mesa 26.2.4");
        assert_eq!(join_nonempty("", "Mesa"), "Mesa");
        assert_eq!(join_nonempty("Driver", ""), "Driver");
    }

    #[test]
    fn device_rank_prefers_discrete_then_integrated() {
        assert!(device_rank(vk::PhysicalDeviceType::DISCRETE_GPU) < device_rank(vk::PhysicalDeviceType::INTEGRATED_GPU));
        assert!(device_rank(vk::PhysicalDeviceType::INTEGRATED_GPU) < device_rank(vk::PhysicalDeviceType::CPU));
        assert_eq!(device_rank(vk::PhysicalDeviceType::OTHER), 4);
    }

    #[test]
    fn choose_takes_the_usable_integrated_gpu_over_an_unusable_discrete_one() {
        let cands = [
            cand("discrete", 0, Some((226, 129)), &["synchronization2"], Some(0)),
            cand("hd530", 1, Some((226, 128)), &[], Some(0)),
        ];
        assert_eq!(choose_device(&cands, None).unwrap(), 1);
    }

    #[test]
    fn choose_prefers_the_discrete_gpu_when_both_work() {
        let cands = [cand("hd530", 1, None, &[], Some(0)), cand("rx", 0, None, &[], Some(0))];
        assert_eq!(choose_device(&cands, None).unwrap(), 1);
    }

    #[test]
    fn choose_follows_the_render_node_even_when_another_gpu_is_better() {
        let cands = [cand("rx", 0, Some((226, 129)), &[], Some(0)), cand("hd530", 1, Some((226, 128)), &[], Some(0))];
        assert_eq!(choose_device(&cands, Some((226, 128))).unwrap(), 1);
    }

    #[test]
    fn choose_names_the_missing_node_when_nothing_matches() {
        let cands = [cand("hd530", 1, Some((226, 128)), &[], Some(0))];
        match choose_device(&cands, Some((226, 129))) {
            Err(Error::Unsupported(msg)) => assert!(msg.contains("226:129"), "{msg}"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn choose_names_what_the_node_device_lacks() {
        let cands = [cand("hd530", 1, Some((226, 128)), &["timeline semaphores"], Some(0))];
        match choose_device(&cands, Some((226, 128))) {
            Err(Error::Unsupported(msg)) => {
                assert!(msg.contains("hd530"), "{msg}");
                assert!(msg.contains("timeline semaphores"), "{msg}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn choose_reports_every_gpu_when_none_is_usable() {
        let cands = [cand("a", 0, None, &["dynamic rendering"], Some(0)), cand("b", 1, None, &[], None)];
        match choose_device(&cands, None) {
            Err(Error::Unsupported(msg)) => {
                assert!(msg.contains("a lacks dynamic rendering"), "{msg}");
                assert!(msg.contains("b lacks a graphics+compute queue family"), "{msg}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn choose_rejects_a_vulkan_1_2_device() {
        let mut c = cand("old", 1, None, &[], Some(0));
        c.api = (1, 2, 200);
        let p = problems(&c);
        assert_eq!(p, vec!["Vulkan 1.3 (device has 1.2.200)".to_string()]);
        assert!(choose_device(&[c], None).is_err());
        assert!(choose_device(&[], None).is_err());
    }
}

#[cfg(all(test, feature = "gpu-tests"))]
mod gpu_tests {
    use super::*;

    #[test]
    fn opens_the_default_gpu_and_reports_it() {
        let dev = Device::open(None).expect("a Vulkan 1.3 GPU");
        let info = dev.info();
        assert!(!info.name.is_empty());
        assert!(info.api >= (1, 3, 0));
        dev.wait_idle().unwrap();
    }

    #[test]
    fn opens_the_render_node_when_present() {
        let path = Path::new("/dev/dri/renderD128");
        if !path.exists() {
            return;
        }
        let dev = Device::open(Some(path)).expect("the render node's GPU");
        use std::os::unix::fs::MetadataExt;
        let rdev = std::fs::metadata(path).unwrap().rdev();
        assert_eq!(dev.info().render_node, Some(dev_numbers(rdev)));
    }
}
