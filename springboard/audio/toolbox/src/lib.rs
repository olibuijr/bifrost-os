//! `bifrost-audio-toolbox`: services one level above the client library (docs/audio-architecture.md, "Crates").
//! Module owners: `converter`, `resample`, `file`, `queue`, `system_sound` (task 13); `varispeed` (task 11, the
//! adaptive resampler that aggregates and drift compensation use). The `varispeed` API below is shared by tasks 11
//! and 13, so it is fixed here.

/// The adaptive resampler for drift compensation: cheap, low latency (a short cubic kernel), its ratio changed
/// every cycle from a device DLL rate without clicks (task 11).
pub mod varispeed;
