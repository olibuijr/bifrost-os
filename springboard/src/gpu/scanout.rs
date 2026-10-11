//! Runtime probe for zero-copy scanout: runs the kernel step (`bare::scanout`)
//! and the EGL import for every format, then applies the pure
//! decision table. Logs one line per decision; never leaves buffers behind.

use super::Gpu;
use crate::bare::scanout::{decide, fourcc_name, probe_drm, Decision, Method, Outcome};
use std::ffi::c_int;

/// Probes the dumb-dmabuf method on the DRM device `drm` for a `w` x `h` panel. The EGL
/// context of `gpu` must be current. Returns the decision and a log line per
/// (method, format).
pub fn probe(gpu: &Gpu, drm: c_int, w: usize, h: usize) -> (Decision, Vec<String>) {
    let mut log = Vec::new();
    let table: Vec<(u32, Outcome)>;
    {
        let method = Method::DumbDmabuf;
        let mut outcomes = Vec::new();
        for (fourcc, res) in probe_drm(drm, method, w, h).tries {
            let name = fourcc_name(fourcc);
            match res {
                Ok(buf) => {
                    let egl = gpu.can_import(buf.w, buf.h, buf.dmabuf_fd, buf.pitch, buf.fourcc);
                    log.push(format!(
                        "scanout probe {} {name}: kms framebuffer ok (pitch {}), egl import {}",
                        method.name(),
                        buf.pitch,
                        egl.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| format!("failed: {e}"))
                    ));
                    outcomes.push((fourcc, Outcome { drm: true, egl: egl.is_ok() }));
                    buf.release(drm);
                }
                Err(e) => {
                    log.push(format!("scanout probe {} {name}: kms failed: {e}", method.name()));
                    outcomes.push((fourcc, Outcome::default()));
                }
            }
        }
        table = outcomes;
    }
    let d = decide(&table);
    log.push(format!("scanout decision: {} {}", d.method.name(), if d.fourcc == 0 { String::new() } else { fourcc_name(d.fourcc) }));
    (d, log)
}
