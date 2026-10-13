//! Running without Android: KMS display, evdev input, sysfs power.
pub mod chord_recovery;
pub mod bluetooth;
pub mod boost;
pub mod control;
pub mod cpuprofile;
pub mod crashlog;
pub mod gpu_comp;
pub mod heartbeat;
pub mod jank;
pub mod input;
pub mod pad_rumble;
pub mod pmqos;
pub mod pointer_finger;
pub mod pointer_prefs;
pub mod kms;
pub mod nl80211;
pub mod platform;
pub mod power;
pub mod scanout;
pub mod sleep;
pub mod sys;
pub mod sys_watch;
pub mod takeover;
pub mod timezone;
pub mod vibrator;
#[cfg(feature = "desktop-wayland")]
pub mod wayland;
#[cfg(feature = "desktop-wayland")]
pub mod material_client;

#[cfg(feature="desktop-wayland")]
pub mod desktop_input;

pub mod account;

#[cfg(feature="desktop-wayland")]
pub mod app_policy;

#[cfg(feature="desktop-wayland")]
pub mod docbroker;

#[cfg(feature="desktop-wayland")]
pub mod app_manager;
