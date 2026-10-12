//! dma-bufs: the cross-process GPU buffer (the reference platform's shared surface object).
//!
//! Owner: wave G1 agent `gpu-texture` (with `texture.rs`).
//!
//! A shared texture is a single-memory-object image with a DRM format modifier. Export duplicates the memory's fd for
//! each memory plane and reports each plane's offset and pitch (`vkGetImageSubresourceLayout` on the `MEMORY_PLANE_i`
//! aspects). Import wraps the planes' fds in one `VkDeviceMemory` (`VK_EXT_external_memory_dma_buf`) and an image
//! whose layout is given explicitly (`VK_EXT_image_drm_format_modifier`). The imported image is owned by the foreign
//! queue family until this device acquires it (`VK_EXT_queue_family_foreign`).

use crate::device::Device;
use crate::format::PixelFormat;
use crate::memory::memory_type_index;
use crate::texture::{self, Inner, Texture, TextureUsage};
use crate::{Error, Result};
use ash::vk;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Arc;

/// One plane of a dma-buf.
#[derive(Debug)]
pub struct DmaBufPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// A dma-buf as Wayland's linux-dmabuf and KMS describe it.
#[derive(Debug)]
pub struct DmaBuf {
    pub width: u32,
    pub height: u32,
    /// DRM fourcc (`PixelFormat::fourcc`).
    pub fourcc: u32,
    /// DRM format modifier (`DRM_FORMAT_MOD_LINEAR` = 0, `DRM_FORMAT_MOD_INVALID` = 0x00ff_ffff_ffff_ffff).
    pub modifier: u64,
    pub planes: Vec<DmaBufPlane>,
}

/// The DRM linear modifier.
pub const MOD_LINEAR: u64 = 0;
/// The DRM "implicit modifier" value.
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// A fourcc and modifier pair a device supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaBufFormat {
    pub fourcc: u32,
    pub modifier: u64,
}

impl DmaBuf {
    /// Duplicates every plane fd, so the same buffer can be imported twice (the compositor keeps a copy while a
    /// client reuses its `wl_buffer`).
    pub fn try_clone(&self) -> std::io::Result<DmaBuf> {
        let planes = self
            .planes
            .iter()
            .map(|p| Ok(DmaBufPlane { fd: p.fd.try_clone()?, offset: p.offset, stride: p.stride }))
            .collect::<std::io::Result<Vec<_>>>()?;
        Ok(DmaBuf { width: self.width, height: self.height, fourcc: self.fourcc, modifier: self.modifier, planes })
    }
}

/// The pixel formats that can be shared, in the order `supported_formats` lists them.
const SHARED_FORMATS: [PixelFormat; 5] = [
    PixelFormat::Bgra8Unorm,
    PixelFormat::Bgrx8Unorm,
    PixelFormat::Rgba8Unorm,
    PixelFormat::R8Unorm,
    PixelFormat::Rgba16Float,
];

/// The memory aspects of a modifier's memory planes (`VK_IMAGE_ASPECT_MEMORY_PLANE_i_BIT_EXT`), at most four.
const PLANE_ASPECTS: [vk::ImageAspectFlags; 4] = [
    vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
];

/// One modifier a physical device reports for a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModifierInfo {
    pub(crate) modifier: u64,
    pub(crate) planes: u32,
    pub(crate) features: vk::FormatFeatureFlags,
}

/// The aspect of memory plane `i`, or `None` past the four a modifier can have.
pub(crate) fn plane_aspect(i: usize) -> Option<vk::ImageAspectFlags> {
    PLANE_ASPECTS.get(i).copied()
}

/// The memory plane count a modifier has, if the device reports that modifier.
pub(crate) fn plane_count_of(infos: &[ModifierInfo], modifier: u64) -> Option<u32> {
    infos.iter().find(|i| i.modifier == modifier).map(|i| i.planes)
}

/// The `VkSubresourceLayout`s of an explicit-modifier import, from (offset, stride) per plane.
pub(crate) fn plane_layouts(planes: &[(u32, u32)]) -> Vec<vk::SubresourceLayout> {
    planes
        .iter()
        .map(|&(offset, stride)| vk::SubresourceLayout { offset: offset as u64, size: 0, row_pitch: stride as u64, array_pitch: 0, depth_pitch: 0 })
        .collect()
}

/// How an imported image may be used, from the format features its modifier reports.
pub(crate) fn import_usage(features: vk::FormatFeatureFlags) -> TextureUsage {
    let mut usage = TextureUsage::SAMPLED;
    if features.contains(vk::FormatFeatureFlags::TRANSFER_SRC) {
        usage = usage | TextureUsage::COPY_SRC;
    }
    if features.contains(vk::FormatFeatureFlags::TRANSFER_DST) {
        usage = usage | TextureUsage::COPY_DST;
    }
    if features.contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT) {
        usage = usage | TextureUsage::RENDER_TARGET;
    }
    usage
}

/// Every modifier the device reports for `format` (two calls: the count, then the list).
pub(crate) fn modifier_infos(device: &Device, format: vk::Format) -> Result<Vec<ModifierInfo>> {
    let inner = &device.inner;
    let mut count = vk::DrmFormatModifierPropertiesListEXT::default();
    {
        let mut props = vk::FormatProperties2::default().push_next(&mut count);
        // SAFETY: the physical device belongs to this instance; a null list only reports the count.
        unsafe { inner.instance.get_physical_device_format_properties2(inner.physical, format, &mut props) };
    }
    let n = count.drm_format_modifier_count as usize;
    let mut entries = vec![vk::DrmFormatModifierPropertiesEXT::default(); n];
    let written = {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut entries);
        {
            let mut props = vk::FormatProperties2::default().push_next(&mut list);
            // SAFETY: `entries` has room for `n` modifiers, the count the first call reported.
            unsafe { inner.instance.get_physical_device_format_properties2(inner.physical, format, &mut props) };
        }
        list.drm_format_modifier_count as usize
    };
    Ok(entries
        .iter()
        .take(written.min(n))
        .map(|e| ModifierInfo { modifier: e.drm_format_modifier, planes: e.drm_format_modifier_plane_count, features: e.drm_format_modifier_tiling_features })
        .collect())
}

/// The fourcc and modifier pairs this device can import for sampling (`Device::supported_dmabuf_formats`).
pub(crate) fn supported_formats(device: &Device) -> Vec<DmaBufFormat> {
    let mut out = Vec::new();
    for format in SHARED_FORMATS {
        let Some(fourcc) = format.fourcc() else { continue };
        let Ok(infos) = modifier_infos(device, format.vk()) else { continue };
        for info in infos {
            if info.features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE) {
                out.push(DmaBufFormat { fourcc, modifier: info.modifier });
            }
        }
    }
    out
}

/// Exports a shared texture's memory as a dma-buf (`Texture::export_dmabuf`).
pub(crate) fn export(texture: &Texture) -> Result<DmaBuf> {
    let inner = &*texture.inner;
    if inner.modifier.is_none() || inner.memory.is_empty() {
        return Err(Error::DmaBuf("texture was not made exportable".into()));
    }
    let dev = &inner.device.inner;
    let format = inner.format;
    let fourcc = format.fourcc().ok_or_else(|| Error::DmaBuf("format has no DRM fourcc".into()))?;

    // The modifier the driver chose for the image (it may differ from the first preferred one).
    let drm = ash::ext::image_drm_format_modifier::Device::new(&dev.instance, &dev.device);
    let mut props = vk::ImageDrmFormatModifierPropertiesEXT::default();
    // SAFETY: the image was created with DRM modifier tiling, which this call requires.
    unsafe { drm.get_image_drm_format_modifier_properties(inner.image, &mut props) }?;
    let modifier = props.drm_format_modifier;
    let infos = modifier_infos(&inner.device, format.vk())?;
    let plane_count = plane_count_of(&infos, modifier).ok_or_else(|| Error::DmaBuf(format!("modifier {modifier:#x} not reported")))?;

    // SAFETY: the memory was allocated with DMA_BUF export; the call returns a new fd that nothing else owns.
    let fd = unsafe {
        let loader = ash::khr::external_memory_fd::Device::new(&dev.instance, &dev.device);
        let info = vk::MemoryGetFdInfoKHR::default().memory(inner.memory[0]).handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let raw = loader.get_memory_fd(&info)?;
        OwnedFd::from_raw_fd(raw)
    };

    let mut planes = Vec::with_capacity(plane_count as usize);
    for i in 0..plane_count as usize {
        let aspect = plane_aspect(i).ok_or_else(|| Error::DmaBuf(format!("{plane_count} memory planes")))?;
        let sub = vk::ImageSubresource { aspect_mask: aspect, mip_level: 0, array_layer: 0 };
        // SAFETY: the image is bound to memory and uses modifier tiling, so the memory-plane aspect is valid.
        let layout = unsafe { dev.device.get_image_subresource_layout(inner.image, sub) };
        planes.push(DmaBufPlane {
            fd: fd.try_clone().map_err(|e| Error::DmaBuf(e.to_string()))?,
            offset: u32::try_from(layout.offset).map_err(|_| Error::DmaBuf("plane offset exceeds u32".into()))?,
            stride: u32::try_from(layout.row_pitch).map_err(|_| Error::DmaBuf("plane pitch exceeds u32".into()))?,
        });
    }
    Ok(DmaBuf { width: inner.width, height: inner.height, fourcc, modifier, planes })
}

/// Imports a dma-buf as a sampled texture without copying its pixels (`Device::import_dmabuf`).
pub(crate) fn import(device: &Device, dmabuf: &DmaBuf) -> Result<Texture> {
    if dmabuf.planes.is_empty() {
        return Err(Error::DmaBuf("no planes".into()));
    }
    if dmabuf.width == 0 || dmabuf.height == 0 {
        return Err(Error::DmaBuf(format!("size {}x{}", dmabuf.width, dmabuf.height)));
    }
    let format = PixelFormat::from_fourcc(dmabuf.fourcc).ok_or_else(|| Error::DmaBuf(format!("unsupported fourcc {:#010x}", dmabuf.fourcc)))?;
    let infos = modifier_infos(device, format.vk())?;
    let info = infos
        .iter()
        .copied()
        .find(|i| i.modifier == dmabuf.modifier && i.features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE))
        .ok_or_else(|| Error::DmaBuf(format!("format {:?} with modifier {:#x} is not sampleable", format, dmabuf.modifier)))?;
    if info.planes as usize != dmabuf.planes.len() {
        return Err(Error::DmaBuf(format!("modifier has {} planes, the buffer {}", info.planes, dmabuf.planes.len())));
    }

    let usage = import_usage(info.features);
    let layouts = plane_layouts(&dmabuf.planes.iter().map(|p| (p.offset, p.stride)).collect::<Vec<_>>());
    let dev = &device.inner.device;
    // The producer leaves a shared image in GENERAL; the first use acquires it from the foreign queue family.
    let mut inner = Inner::shell(device, format, dmabuf.width, dmabuf.height, usage, true, vk::ImageLayout::GENERAL);
    inner.modifier = Some(dmabuf.modifier);

    let mut ext = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut explicit = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default().drm_format_modifier(dmabuf.modifier).plane_layouts(&layouts);
    let create = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format.vk())
        .extent(vk::Extent3D { width: dmabuf.width, height: dmabuf.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(texture::usage_flags(usage))
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut ext)
        .push_next(&mut explicit);
    // SAFETY: the create info is fully initialised; the plane layouts outlive the call.
    inner.image = unsafe { dev.create_image(&create, None) }?;

    // The memory: one dma-buf fd for every plane (the planes are views into one buffer), memory types both the image
    // and the fd accept.
    let req = unsafe { dev.get_image_memory_requirements(inner.image) };
    let owned = dmabuf.planes[0].fd.try_clone().map_err(|e| Error::DmaBuf(e.to_string()))?;
    let fd_loader = ash::khr::external_memory_fd::Device::new(&device.inner.instance, dev);
    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    // SAFETY: the fd is a valid dma-buf owned by `owned` for the duration of the call.
    unsafe { fd_loader.get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT, owned.as_raw_fd(), &mut fd_props) }?;
    let bits = req.memory_type_bits & fd_props.memory_type_bits;
    let mem_props = &device.inner.memory_properties;
    let index = memory_type_index(mem_props, bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
        .or_else(|| memory_type_index(mem_props, bits, vk::MemoryPropertyFlags::empty()))
        .ok_or_else(|| Error::DmaBuf("no memory type accepts the buffer".into()))?;

    // Vulkan takes the fd when the allocation succeeds and leaves it open when it fails.
    let raw = owned.into_raw_fd();
    let mut import_info = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT).fd(raw);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(inner.image);
    let alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(index).push_next(&mut import_info).push_next(&mut dedicated);
    // SAFETY: the allocation takes the fd on success; on failure the fd is still ours and is closed below.
    let memory = match unsafe { dev.allocate_memory(&alloc, None) } {
        Ok(m) => m,
        Err(e) => {
            drop(unsafe { OwnedFd::from_raw_fd(raw) });
            return Err(e.into());
        }
    };
    inner.memory.push(memory);
    unsafe { dev.bind_image_memory(inner.image, memory, 0) }?;
    inner.view = texture::create_view(dev, inner.image, format)?;
    Ok(Texture { inner: Arc::new(inner) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(modifier: u64, planes: u32, features: vk::FormatFeatureFlags) -> ModifierInfo {
        ModifierInfo { modifier, planes, features }
    }

    #[test]
    fn plane_aspects_are_the_four_memory_planes() {
        assert_eq!(plane_aspect(0), Some(vk::ImageAspectFlags::MEMORY_PLANE_0_EXT));
        assert_eq!(plane_aspect(3), Some(vk::ImageAspectFlags::MEMORY_PLANE_3_EXT));
        assert_eq!(plane_aspect(4), None);
    }

    #[test]
    fn plane_count_finds_the_reported_modifier() {
        let infos = [info(MOD_LINEAR, 1, vk::FormatFeatureFlags::empty()), info(0x0100_0000_0000_0001, 2, vk::FormatFeatureFlags::empty())];
        assert_eq!(plane_count_of(&infos, 0x0100_0000_0000_0001), Some(2));
        assert_eq!(plane_count_of(&infos, 0x42), None);
    }

    #[test]
    fn plane_layouts_keep_offset_and_pitch() {
        let layouts = plane_layouts(&[(0, 4096), (8192, 2048)]);
        assert_eq!(layouts.len(), 2);
        assert_eq!((layouts[0].offset, layouts[0].row_pitch, layouts[0].size), (0, 4096, 0));
        assert_eq!((layouts[1].offset, layouts[1].row_pitch), (8192, 2048));
    }

    #[test]
    fn import_usage_follows_the_modifier_features() {
        let all = vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::TRANSFER_SRC | vk::FormatFeatureFlags::TRANSFER_DST | vk::FormatFeatureFlags::COLOR_ATTACHMENT;
        let u = import_usage(all);
        assert!(u.contains(TextureUsage::SAMPLED | TextureUsage::COPY_SRC | TextureUsage::COPY_DST | TextureUsage::RENDER_TARGET));
        let sample_only = import_usage(vk::FormatFeatureFlags::SAMPLED_IMAGE);
        assert_eq!(sample_only, TextureUsage::SAMPLED);
    }

    #[test]
    fn try_clone_duplicates_fds_and_keeps_layout() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let buf = DmaBuf {
            width: 4,
            height: 4,
            fourcc: 0x3432_5241,
            modifier: MOD_LINEAR,
            planes: vec![DmaBufPlane { fd: OwnedFd::from(file), offset: 16, stride: 16 }],
        };
        let copy = buf.try_clone().unwrap();
        assert_eq!((copy.width, copy.height, copy.fourcc, copy.modifier), (4, 4, 0x3432_5241, MOD_LINEAR));
        assert_eq!((copy.planes[0].offset, copy.planes[0].stride), (16, 16));
        assert_ne!(copy.planes[0].fd.as_raw_fd(), buf.planes[0].fd.as_raw_fd());
    }

    #[test]
    fn invalid_modifier_is_the_implicit_value() {
        assert_eq!(MOD_INVALID, 0x00ff_ffff_ffff_ffff);
        assert_eq!(MOD_LINEAR, 0);
    }

    #[test]
    fn shared_format_list_has_a_fourcc_for_each() {
        for f in SHARED_FORMATS {
            assert!(f.fourcc().is_some(), "{f:?}");
        }
    }
}
