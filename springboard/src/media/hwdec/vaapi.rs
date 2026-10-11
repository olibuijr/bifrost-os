//! What the desktop's GPU can decode in hardware, asked of libva (`libva.so.2` + `libva-drm.so.2`,
//! `dlopen`ed) on the DRM render node. libva itself is a slice-level API (the caller parses every
//! SPS/PPS/slice header and manages the reference pictures), so the decoding is driven by
//! libavcodec's VA-API hwaccel (`ffmpeg.rs`); this module only decides whether to ask for it.

use crate::media::dl::Lib;
use crate::media::Codec;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io;
use std::sync::OnceLock;

/// `VAProfile` values (va.h) per codec; a codec is supported when any of its profiles has a VLD entrypoint.
const H264: [c_int; 3] = [13, 6, 7]; // constrained baseline, main, high
const HEVC: [c_int; 2] = [17, 18]; // main, main 10
const VP9: [c_int; 1] = [19];
const AV1: [c_int; 1] = [32];
const ENTRYPOINT_VLD: c_int = 1;

/// The codecs a render node decodes in hardware.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Support {
    pub node: String,
    pub vendor: String,
    pub h264: bool,
    pub hevc: bool,
    pub vp9: bool,
    pub av1: bool,
}

impl Support {
    pub fn has(&self, codec: Codec) -> bool {
        match codec {
            Codec::H264 => self.h264,
            Codec::Hevc => self.hevc,
        }
    }

    pub fn summary(&self) -> String {
        let list: Vec<&str> = [("h264", self.h264), ("hevc", self.hevc), ("vp9", self.vp9), ("av1", self.av1)].iter().filter(|(_, on)| *on).map(|(n, _)| *n).collect();
        format!("{} on {}: {}", self.vendor, self.node, if list.is_empty() { "no hardware decode".into() } else { list.join(" ") })
    }
}

/// True when any profile of the codec passes `vld` (which says whether a profile decodes in hardware).
fn supported(profiles: &[c_int], mut vld: impl FnMut(c_int) -> bool) -> bool {
    profiles.iter().any(|p| vld(*p))
}

/// The first DRM render node (`/dev/dri/renderD128`, ...), or `SPRINGBOARD_VAAPI_DEVICE`.
pub fn render_node() -> Option<String> {
    if let Ok(p) = std::env::var("SPRINGBOARD_VAAPI_DEVICE") {
        return Some(p);
    }
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri").ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("renderD")).collect();
    nodes.sort();
    nodes.first().map(|n| format!("/dev/dri/{n}"))
}

type Dpy = *mut c_void;

/// Queries libva on `node`. An error means no usable libva or driver there.
pub fn probe_node(node: &str) -> io::Result<Support> {
    use crate::bare::sys::{close, open_dev, O_CLOEXEC, O_RDWR};
    let va = Lib::open(&["libva.so.2"])?;
    let drm = Lib::open(&["libva-drm.so.2"])?;
    // SAFETY: the types are the C prototypes from va.h / va_drm.h.
    let (get_display, initialize, terminate, max_entry, query_entry, vendor): (
        unsafe extern "C" fn(c_int) -> Dpy,
        unsafe extern "C" fn(Dpy, *mut c_int, *mut c_int) -> c_int,
        unsafe extern "C" fn(Dpy) -> c_int,
        unsafe extern "C" fn(Dpy) -> c_int,
        unsafe extern "C" fn(Dpy, c_int, *mut c_int, *mut c_int) -> c_int,
        unsafe extern "C" fn(Dpy) -> *const c_char,
    ) = unsafe { (drm.sym("vaGetDisplayDRM")?, va.sym("vaInitialize")?, va.sym("vaTerminate")?, va.sym("vaMaxNumEntrypoints")?, va.sym("vaQueryConfigEntrypoints")?, va.sym("vaQueryVendorString")?) };
    let fd = open_dev(node, O_RDWR | O_CLOEXEC)?;
    // SAFETY: the display and fd stay valid until terminate/close below; buffers are sized by vaMaxNumEntrypoints.
    let result = unsafe {
        let dpy = get_display(fd);
        let (mut major, mut minor) = (0, 0);
        if dpy.is_null() || initialize(dpy, &mut major, &mut minor) != 0 {
            Err(io::Error::other(format!("vaInitialize failed on {node}")))
        } else {
            let mut entries = vec![0 as c_int; max_entry(dpy).max(1) as usize];
            let mut vld = |profile: c_int| {
                let mut n = 0;
                query_entry(dpy, profile, entries.as_mut_ptr(), &mut n) == 0 && entries.iter().take(n.max(0) as usize).any(|e| *e == ENTRYPOINT_VLD)
            };
            let s = Support {
                node: node.to_string(),
                vendor: CStr::from_ptr(vendor(dpy)).to_string_lossy().into_owned(),
                h264: supported(&H264, &mut vld),
                hevc: supported(&HEVC, &mut vld),
                vp9: supported(&VP9, &mut vld),
                av1: supported(&AV1, &mut vld),
            };
            terminate(dpy);
            Ok(s)
        }
    };
    // SAFETY: the fd is ours and not used after this.
    unsafe { close(fd) };
    result
}

/// The render node's hardware decode support, probed once; None when there is no node or no libva.
pub fn probe() -> Option<&'static Support> {
    static CACHE: OnceLock<Option<Support>> = OnceLock::new();
    CACHE.get_or_init(|| render_node().and_then(|n| probe_node(&n).map_err(|e| eprintln!("vaapi: {e}")).ok())).as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codec_is_supported_when_any_profile_decodes() {
        assert!(supported(&H264, |p| p == 7));
        assert!(!supported(&HEVC, |p| p == 7));
        assert!(!supported(&AV1, |_| false));
    }

    #[test]
    fn support_answers_per_codec_and_summarises() {
        let s = Support { node: "/dev/dri/renderD128".into(), vendor: "Mesa".into(), h264: true, hevc: false, vp9: true, av1: false };
        assert!(s.has(Codec::H264) && !s.has(Codec::Hevc));
        assert_eq!(s.summary(), "Mesa on /dev/dri/renderD128: h264 vp9");
    }
}
