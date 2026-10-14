//! Models of system services that apps reach through the SDK. Each module is pure: it validates, decodes or
//! describes, and the system service applies the result. Nothing here opens a socket, a device or a file it does not
//! read from its arguments.

pub mod authorization_lock;
pub mod dma_buf;
pub mod effect_nodes;
pub mod ipv4_config;
pub mod sprite_shader;
pub mod update_integrity;
#[cfg(target_os = "linux")]
pub mod app_sandbox;
#[cfg(target_os = "linux")]
pub mod execution_policy;
