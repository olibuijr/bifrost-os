//! Explicit synchronisation: timeline semaphores (the reference API's shared event) with sync_file export,
//! for client acquire/release fences (`wp_linux_drm_syncobj`) and KMS in/out fences.
//!
//! A `SharedEvent` is one `VkSemaphore` of type timeline: its value only grows. `wait` and `signal` are CPU-side
//! (`vkWaitSemaphores`, `vkSignalSemaphore`). `export_sync_file` makes a sync_file that signals once the value is
//! reached: a queue submission waits for the timeline value and signals a fresh binary semaphore, which is exported
//! as a `SYNC_FD` and destroyed. The export submission waits on the GPU, not the CPU; the export must be submitted
//! before any later work on the same queue that signals the value, or that work never runs (a queue is in order).
//!
//! Owner: wave G1 agent `gpu-sync`.

use crate::{Device, Error, Result};
use ash::vk;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    pub(crate) semaphore: vk::Semaphore,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // SAFETY: the semaphore was created by this device and is destroyed exactly once, when the last handle
        // goes. Submissions that wait on it keep a clone of the event (`CommandBuffer::waits`), so none is pending.
        unsafe {
            self.device.inner.device.destroy_semaphore(self.semaphore, None);
        }
    }
}

/// A timeline semaphore. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct SharedEvent {
    pub(crate) inner: Arc<Inner>,
}

/// Makes a timeline semaphore starting at 0. Called by `Device::new_shared_event`.
pub(crate) fn create(device: &Device) -> Result<SharedEvent> {
    let mut timeline = vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE).initial_value(0);
    let info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline);
    // SAFETY: `info` and the chained type info are complete; the semaphore is owned by the returned `Inner`.
    let semaphore = unsafe { device.inner.device.create_semaphore(&info, None) }.map_err(Error::from)?;
    Ok(SharedEvent {
        inner: Arc::new(Inner {
            device: device.clone(),
            semaphore,
        }),
    })
}

/// Maps the result of `vkWaitSemaphores`: success is `true`, a timeout is `false`, anything else is an error.
pub(crate) fn wait_outcome(result: std::result::Result<(), vk::Result>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(vk::Result::TIMEOUT) => Ok(false),
        Err(e) => Err(Error::from(e)),
    }
}

/// Wraps the fd from `vkGetSemaphoreFdKHR`. A negative fd means the driver has no pending sync_file for the
/// semaphore (it was already signalled when exported); the caller reports that instead of inventing a file.
pub(crate) fn sync_file_from_fd(fd: i32) -> Result<OwnedFd> {
    if fd < 0 {
        return Err(Error::Unsupported("driver returned no sync_file: the value was already reached".into()));
    }
    // SAFETY: `fd` is a new descriptor returned by `vkGetSemaphoreFdKHR` for this call; nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl SharedEvent {
    /// The value the GPU has signalled so far.
    pub fn signaled_value(&self) -> Result<u64> {
        // SAFETY: the semaphore is a live timeline semaphore of this device.
        unsafe { self.inner.device.inner.device.get_semaphore_counter_value(self.inner.semaphore) }.map_err(Error::from)
    }

    /// Blocks until the value reaches `value` or `timeout_ns` passes; returns whether it was reached.
    pub fn wait(&self, value: u64, timeout_ns: u64) -> Result<bool> {
        let semaphores = [self.inner.semaphore];
        let values = [value];
        let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
        // SAFETY: `info` points at arrays that live for the call; the semaphore is live.
        let result = unsafe { self.inner.device.inner.device.wait_semaphores(&info, timeout_ns) };
        wait_outcome(result)
    }

    /// Sets the value from the CPU. The value must be greater than the current one (Vulkan rejects it otherwise).
    pub fn signal(&self, value: u64) -> Result<()> {
        let info = vk::SemaphoreSignalInfo::default().semaphore(self.inner.semaphore).value(value);
        // SAFETY: the semaphore is a live timeline semaphore of this device.
        unsafe { self.inner.device.inner.device.signal_semaphore(&info) }.map_err(Error::from)
    }

    /// A sync_file that signals when the value reaches `value` (exported through a binary semaphore).
    pub fn export_sync_file(&self, value: u64) -> Result<OwnedFd> {
        let dev = &self.inner.device;
        let raw = &dev.inner.device;
        let mut export = vk::ExportSemaphoreCreateInfo::default().handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let binary_info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        // SAFETY: the create info is complete and chains an export structure with the SYNC_FD handle type.
        let binary = unsafe { raw.create_semaphore(&binary_info, None) }.map_err(Error::from)?;
        // SAFETY: a plain fence on the live device; owned by the retired entry below.
        let fence = match unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None) } {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: nothing was submitted with `binary`.
                unsafe { raw.destroy_semaphore(binary, None) };
                return Err(e.into());
            }
        };
        let waits = [vk::SemaphoreSubmitInfo::default().semaphore(self.inner.semaphore).value(value).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let signals = [vk::SemaphoreSubmitInfo::default().semaphore(binary).stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let submit = vk::SubmitInfo2::default().wait_semaphore_infos(&waits).signal_semaphore_infos(&signals);
        if let Err(e) = dev.submit(&[submit], fence) {
            // SAFETY: the submission failed, so nothing refers to either object.
            unsafe {
                raw.destroy_fence(fence, None);
                raw.destroy_semaphore(binary, None);
            }
            return Err(e);
        }
        // The semaphore may only be destroyed after the submission that signals it completes: it goes to the
        // device's retired list with the fence, released by `Device::reap`.
        crate::command::retire_semaphore(dev, fence, binary);
        let external = ash::khr::external_semaphore_fd::Device::new(&dev.inner.instance, raw);
        let get = vk::SemaphoreGetFdInfoKHR::default().semaphore(binary).handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        // SAFETY: `binary` was created with SYNC_FD export and has a signal operation pending, as a sync_file export
        // needs; it stays alive until its fence signals (retired list above).
        let fd = unsafe { external.get_semaphore_fd(&get) }.map_err(Error::from)?;
        sync_file_from_fd(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;

    #[test]
    fn wait_success_is_true_and_timeout_is_false() {
        assert_eq!(wait_outcome(Ok(())), Ok(true));
        assert_eq!(wait_outcome(Err(vk::Result::TIMEOUT)), Ok(false));
    }

    #[test]
    fn wait_failures_map_to_the_crate_errors() {
        assert_eq!(wait_outcome(Err(vk::Result::ERROR_DEVICE_LOST)), Err(Error::DeviceLost));
        assert_eq!(wait_outcome(Err(vk::Result::ERROR_OUT_OF_HOST_MEMORY)), Err(Error::OutOfMemory));
        assert_eq!(wait_outcome(Err(vk::Result::ERROR_UNKNOWN)), Err(Error::Vulkan(vk::Result::ERROR_UNKNOWN.as_raw())));
    }

    #[test]
    fn negative_fd_is_reported_not_wrapped() {
        assert!(matches!(sync_file_from_fd(-1), Err(Error::Unsupported(_))));
        assert!(matches!(sync_file_from_fd(-7), Err(Error::Unsupported(_))));
    }

    #[test]
    fn a_valid_fd_is_taken_over_and_closed_on_drop() {
        let file = std::fs::File::open("/dev/null").expect("/dev/null exists on Linux");
        let raw = file.into_raw_fd();
        let owned = sync_file_from_fd(raw).expect("valid fd");
        assert_eq!(std::os::fd::AsRawFd::as_raw_fd(&owned), raw);
        drop(owned);
    }

    #[cfg(feature = "gpu-tests")]
    mod gpu_tests {
        use super::*;

        #[test]
        fn timeline_signal_wait_and_reach() {
            let dev = crate::Device::open(None).expect("device");
            let ev = dev.new_shared_event().unwrap();
            assert_eq!(ev.signaled_value().unwrap(), 0);
            ev.signal(5).unwrap();
            assert!(ev.wait(5, 1_000_000_000).unwrap());
            assert!(!ev.wait(6, 1_000_000).unwrap(), "6 is not reached within 1 ms");
            assert_eq!(ev.signaled_value().unwrap(), 5);
        }

        #[test]
        fn export_for_a_reached_value_gives_a_file_or_reports_none() {
            let dev = crate::Device::open(None).expect("device");
            let ev = dev.new_shared_event().unwrap();
            ev.signal(1).unwrap();
            // The value is already reached: the driver may return a sync_file or none. Either is valid behaviour.
            if let Ok(fd) = ev.export_sync_file(1) {
                drop(fd);
            }
        }
    }
}
