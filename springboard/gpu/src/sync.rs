//! Explicit synchronisation: timeline semaphores (the reference API's shared event) with sync_file import and export,
//! for client acquire/release fences (`wp_linux_drm_syncobj`) and KMS in/out fences.
//!
//! Owner: wave G1 agent `gpu-sync`.

use crate::Result;
use std::os::fd::OwnedFd;

pub(crate) struct Inner {
    pub(crate) device: crate::Device,
    pub(crate) semaphore: ash::vk::Semaphore,
}

/// A timeline semaphore. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct SharedEvent {
    pub(crate) inner: std::sync::Arc<Inner>,
}

impl SharedEvent {
    /// The value the GPU has signalled so far.
    pub fn signaled_value(&self) -> Result<u64> {
        todo!("gpu-sync")
    }

    /// Blocks until the value reaches `value` or `timeout_ns` passes; returns whether it was reached.
    pub fn wait(&self, value: u64, timeout_ns: u64) -> Result<bool> {
        let _ = (value, timeout_ns);
        todo!("gpu-sync")
    }

    /// Sets the value from the CPU.
    pub fn signal(&self, value: u64) -> Result<()> {
        let _ = value;
        todo!("gpu-sync")
    }

    /// A sync_file that signals when the value reaches `value` (exported through a binary semaphore).
    pub fn export_sync_file(&self, value: u64) -> Result<OwnedFd> {
        let _ = value;
        todo!("gpu-sync")
    }
}
