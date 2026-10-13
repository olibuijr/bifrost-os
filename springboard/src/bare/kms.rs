//! Legacy DRM/KMS modesetting with two dumb buffers and page flipping.
//! Frames are composed in RAM and copied out, since the scanout buffers are
//! uncached and must never be read back.

use super::sys::*;
use crate::gfx::{threads, Area, Damage};
use std::ffi::c_int;
use std::io;

#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_ptr: u64,
    crtc_ptr: u64,
    conn_ptr: u64,
    enc_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_conns: u32,
    count_encs: u32,
    min_w: u32,
    max_w: u32,
    min_h: u32,
    max_h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ModeInfo {
    clock: u32,
    pub hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    pub vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    pub vrefresh: u32,
    flags: u32,
    kind: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct GetConnector {
    enc_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encs: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_w: u32,
    mm_h: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct Crtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

#[repr(C)]
#[derive(Default)]
struct CursorCmd {
    flags: u32,
    crtc_id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    handle: u32,
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

#[repr(C)]
#[derive(Default)]
struct FbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
#[derive(Default)]
struct PageFlip {
    crtc_id: u32,
    fb_id: u32,
    flags: u32,
    reserved: u32,
    user_data: u64,
}

#[repr(C)]
#[derive(Default)]
struct GetProperty {
    values_ptr: u64,
    enum_blob_ptr: u64,
    prop_id: u32,
    flags: u32,
    name: [u8; 32],
    count_values: u32,
    count_enum_blobs: u32,
}

impl GetProperty {
    fn is_named(&self, name: &str) -> bool {
        self.name.split(|&b| b == 0).next() == Some(name.as_bytes())
    }
}

#[repr(C)]
#[derive(Default)]
struct SetConnectorProperty {
    value: u64,
    prop_id: u32,
    connector_id: u32,
}

const XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
const DPMS_ON: u64 = 0;
const DPMS_OFF: u64 = 3;
const PAGE_FLIP_EVENT: u32 = 1;

struct Buffer {
    fb: u32,
    pitch: usize,
    map: *mut u32,
}

/// What each of the two scanout buffers still owes the frame: every change
/// since that buffer was last written, so a buffer that sat out a frame
/// catches up on both frames' damage when its turn comes.
struct Owed([Damage; 2]);

impl Owed {
    fn new(w: usize, h: usize) -> Self {
        let mut all = Damage::new(w, h);
        all.all();
        Owed([all.clone(), all])
    }

    /// Something else drew on screen: both buffers owe the whole frame.
    fn stale_all(&mut self) {
        for d in &mut self.0 {
            d.all();
        }
    }

    /// Records a new frame's changes against both buffers.
    fn note(&mut self, damage: &Damage) {
        for d in &mut self.0 {
            d.union(damage);
        }
    }

    /// What `buf` must copy now; `paid` clears it once copied.
    fn due(&self, buf: usize) -> &Damage {
        &self.0[buf]
    }

    fn paid(&mut self, buf: usize) {
        self.0[buf].clear();
    }
}

/// A scanout mapping shared with the copy threads, which write disjoint rows.
#[derive(Clone, Copy)]
struct Map(*mut u32);
// SAFETY: the mapping lives as long as the Display and every thread writes
// only its own interleaved rows (see `copy_areas`), so there is no aliasing.
unsafe impl Send for Map {}
unsafe impl Sync for Map {}

impl Map {
    /// Pixel `i` of the mapping (a method, so closures capture the whole
    /// wrapper rather than the raw pointer inside).
    fn at(&self, i: usize) -> *mut u32 {
        // SAFETY: callers keep `i` inside the buffer.
        unsafe { self.0.add(i) }
    }
}

/// One pixel with red and blue swapped (RGBA in a u32 with R low -> BGRA).
#[inline(always)]
fn swap_px(s: u32) -> u32 {
    (s & 0xff00_ff00) | (s & 0xff) << 16 | (s >> 16) & 0xff
}

/// A row kernel: `n` pixels from `src` to `dst`, the last flag asking for streaming
/// (non-temporal) stores where the CPU has them.
type Kernel = unsafe fn(*mut u32, *const u32, usize, bool);

unsafe fn row_scalar_swap(dst: *mut u32, src: *const u32, n: usize, _stream: bool) {
    for i in 0..n {
        *dst.add(i) = swap_px(*src.add(i));
    }
}

unsafe fn row_copy(dst: *mut u32, src: *const u32, n: usize, _stream: bool) {
    std::ptr::copy_nonoverlapping(src, dst, n);
}

/// Tail of a SIMD row: the pixels the vector loop left over.
#[allow(dead_code)]
#[inline(always)]
unsafe fn tail<const SWAP: bool>(dst: *mut u32, src: *const u32, from: usize, n: usize) {
    for i in from..n {
        let s = *src.add(i);
        *dst.add(i) = if SWAP { swap_px(s) } else { s };
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::tail;
    use std::arch::x86_64::*;

    /// 8 pixels per step; the shuffle swaps bytes 0 and 2 of every pixel. With `stream` the
    /// destination is first aligned to 32 bytes and written with non-temporal stores, which
    /// skip the read-for-ownership of a full-frame copy and leave the cache to the renderer.
    #[target_feature(enable = "avx2")]
    pub unsafe fn avx2<const SWAP: bool>(dst: *mut u32, src: *const u32, n: usize, stream: bool) {
        let mask = _mm256_setr_epi8(2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15);
        let mut i = 0;
        if stream {
            while i < n && (dst.add(i) as usize) & 31 != 0 {
                let s = *src.add(i);
                *dst.add(i) = if SWAP { super::swap_px(s) } else { s };
                i += 1;
            }
        }
        while i + 8 <= n {
            let mut v = _mm256_loadu_si256(src.add(i) as *const __m256i);
            if SWAP {
                v = _mm256_shuffle_epi8(v, mask);
            }
            if stream {
                _mm256_stream_si256(dst.add(i) as *mut __m256i, v);
            } else {
                _mm256_storeu_si256(dst.add(i) as *mut __m256i, v);
            }
            i += 8;
        }
        if stream {
            _mm_sfence();
        }
        tail::<SWAP>(dst, src, i, n);
    }

    /// The same with 4 pixels per step for CPUs without AVX2 (SSSE3 has the byte shuffle).
    #[target_feature(enable = "ssse3")]
    pub unsafe fn ssse3_swap(dst: *mut u32, src: *const u32, n: usize, stream: bool) {
        let mask = _mm_setr_epi8(2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15);
        let mut i = 0;
        if stream {
            while i < n && (dst.add(i) as usize) & 15 != 0 {
                *dst.add(i) = super::swap_px(*src.add(i));
                i += 1;
            }
        }
        while i + 4 <= n {
            let v = _mm_shuffle_epi8(_mm_loadu_si128(src.add(i) as *const __m128i), mask);
            if stream {
                _mm_stream_si128(dst.add(i) as *mut __m128i, v);
            } else {
                _mm_storeu_si128(dst.add(i) as *mut __m128i, v);
            }
            i += 4;
        }
        if stream {
            _mm_sfence();
        }
        tail::<true>(dst, src, i, n);
    }
}

#[cfg(target_arch = "aarch64")]
mod arm {
    use super::tail;
    use std::arch::aarch64::*;

    /// 16 pixels per step: four loads, a table lookup that swaps bytes 0 and 2 of each pixel,
    /// four stores. (NEON is part of the aarch64 baseline.)
    pub unsafe fn neon_swap(dst: *mut u32, src: *const u32, n: usize, _stream: bool) {
        const IDX: [u8; 16] = [2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15];
        let idx = vld1q_u8(IDX.as_ptr());
        let (s, d) = (src as *const u8, dst as *mut u8);
        let mut i = 0;
        while i + 16 <= n {
            let p = i * 4;
            let (a, b, c, e) = (vld1q_u8(s.add(p)), vld1q_u8(s.add(p + 16)), vld1q_u8(s.add(p + 32)), vld1q_u8(s.add(p + 48)));
            vst1q_u8(d.add(p), vqtbl1q_u8(a, idx));
            vst1q_u8(d.add(p + 16), vqtbl1q_u8(b, idx));
            vst1q_u8(d.add(p + 32), vqtbl1q_u8(c, idx));
            vst1q_u8(d.add(p + 48), vqtbl1q_u8(e, idx));
            i += 16;
        }
        while i + 4 <= n {
            vst1q_u8(d.add(i * 4), vqtbl1q_u8(vld1q_u8(s.add(i * 4)), idx));
            i += 4;
        }
        tail::<true>(dst, src, i, n);
    }
}

/// The fastest row kernel this CPU has (decided once): AVX2 or SSSE3 on x86_64, NEON on
/// aarch64, else scalar code (a plain memcpy when no swap is wanted).
fn kernel(swap: bool) -> Kernel {
    static PICK: std::sync::OnceLock<[Kernel; 2]> = std::sync::OnceLock::new();
    PICK.get_or_init(|| -> [Kernel; 2] {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                return [x86::avx2::<false>, x86::avx2::<true>];
            }
            if is_x86_feature_detected!("ssse3") {
                return [row_copy, x86::ssse3_swap];
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            return [row_copy, arm::neon_swap];
        }
        #[allow(unreachable_code)]
        [row_copy, row_scalar_swap]
    })[swap as usize]
}

/// Below this many pixels a thread costs more in wake-up and join than the copy saves.
const MIN_PX_PER_THREAD: usize = 96 * 1024;
/// A copy at least this big (2 MB) goes around the cache on x86; smaller ones are about to be
/// read by the scanout engine from the cache or are too small to pollute it.
const STREAM_PX: usize = 1 << 19;

/// Copies the rows of `areas` from `frame` (stride `w`) into `map` (stride `pitch`), as RGBA
/// or swapped to BGRA for an XRGB scanout. The work is cut into row chunks (about two per
/// thread, dealt out dynamically by the pool, so the cores that are free take more) and tiny
/// damage stays on the calling thread. Chunks are whole rows, so two threads never write the
/// same cache line unless the pitch is not a multiple of 64 bytes.
fn copy_areas(frame: &[u32], w: usize, map: Map, pitch: usize, areas: &[Area], swap_rb: bool, n: usize) {
    let n = n.min(crate::gfx::affinity::pool_cpus());
    copy_chunks(frame, w, map, pitch, areas, swap_rb, n, MIN_PX_PER_THREAD);
}

#[allow(clippy::too_many_arguments)]
fn copy_chunks(frame: &[u32], w: usize, map: Map, pitch: usize, areas: &[Area], swap_rb: bool, n: usize, min_px: usize) {
    let total: usize = areas.iter().map(Area::pixels).sum();
    if total == 0 {
        return;
    }
    let row = kernel(swap_rb);
    let stream = cfg!(target_arch = "x86_64") && total >= STREAM_PX;
    let copy = move |a: &Area| {
        for y in a.y0..a.y1 {
            let src = &frame[y * w + a.x0..y * w + a.x1];
            // SAFETY: the area is inside the frame and the buffer has the same size; this call
            // is the only one writing these rows (chunks are disjoint).
            unsafe { row(map.at(y * pitch + a.x0), src.as_ptr(), src.len(), stream) };
        }
    };
    let n = n.min(total / min_px.max(1)).max(1);
    if n == 1 {
        return areas.iter().for_each(copy);
    }
    let per_chunk = total.div_ceil(n * 2).max(min_px / 2).max(1);
    let mut chunks = Vec::with_capacity(n * 3);
    for a in areas.iter().filter(|a| a.x0 < a.x1 && a.y0 < a.y1) {
        let rows = per_chunk.div_ceil(a.x1 - a.x0).max(1);
        let mut y = a.y0;
        while y < a.y1 {
            let y1 = (y + rows).min(a.y1);
            chunks.push(Area { y0: y, y1, ..*a });
            y = y1;
        }
    }
    crate::gfx::pool::run(chunks.len(), &|k| copy(&chunks[k]));
}

/// Safe entry to the copy-out path for host benchmarks: copies `areas` of
/// `frame` (stride `w`) into `out` (stride `pitch`) on `n` threads.
pub fn copy_into(frame: &[u32], w: usize, out: &mut [u32], pitch: usize, areas: &[Area], swap_rb: bool, n: usize) {
    assert!(areas.iter().all(|a| a.x1 <= w && a.x1 <= pitch && a.y1 * w <= frame.len() && a.y1 * pitch <= out.len()));
    copy_areas(frame, w, Map(out.as_mut_ptr()), pitch, areas, swap_rb, n.max(1));
}

/// `SPRINGBOARD_VIRTUAL=WxH` (or `WxH@HZ`) asks for an emulated panel instead of KMS.
pub fn virtual_size(v: Option<&str>) -> Option<(usize, usize, u32)> {
    let (dim, hz) = match v?.trim().split_once('@') {
        Some((d, hz)) => (d, hz.parse().ok().filter(|&hz| (1..=240).contains(&hz))?),
        None => (v?.trim(), 60),
    };
    let (w, h) = dim.split_once('x')?;
    let (w, h): (usize, usize) = (w.parse().ok()?, h.parse().ok()?);
    ((64..=8192).contains(&w) && (64..=8192).contains(&h)).then_some((w, h, hz))
}

/// The kernel driver behind `/dev/dri/card<n>` ("i915", "amdgpu", "simpledrm", ...), "" when unknown.
fn card_driver(n: u32) -> String {
    std::fs::read_link(format!("/sys/class/drm/card{n}/device/driver")).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned())).unwrap_or_default()
}

fn drm_drivers() -> Vec<String> {
    std::fs::read_dir("/sys/class/drm").map(|d| d.filter_map(Result::ok).filter_map(|e| e.file_name().to_str()?.strip_prefix("card")?.parse::<u32>().ok()).map(card_driver).collect()).unwrap_or_default()
}

/// Drivers that scan out the framebuffer the firmware set up rather than programming a GPU.
fn is_firmware_fb(driver: &str) -> bool {
    matches!(driver, "simpledrm" | "simple-framebuffer" | "efifb" | "vesafb" | "uvesafb" | "ofdrm")
}

/// A PCI display controller (class 0x03xxxx) with no driver bound yet.
fn pci_display_without_driver() -> bool {
    std::fs::read_dir("/sys/bus/pci/devices").map(|d| {
        d.filter_map(Result::ok).any(|e| {
            let p = e.path();
            std::fs::read_to_string(p.join("class")).is_ok_and(|c| c.trim().starts_with("0x03")) && !p.join("driver").exists()
        })
    }).unwrap_or(false)
}

/// Optional fixed mode for hardware-like emulators: do not let a frontend window's
/// dynamically preferred size alter the panel being tested.
fn select_mode(modes: &[ModeInfo], requested: Option<&str>, max_refresh: bool) -> io::Result<ModeInfo> {
    if let Some(requested) = requested {
        let (w, h, hz) = virtual_size(Some(requested)).ok_or_else(|| io::Error::other("invalid SPRINGBOARD_MODE (expected WxH@Hz)"))?;
        return modes.iter().find(|m| m.hdisplay as usize == w && m.vdisplay as usize == h && m.vrefresh == hz)
            .copied().ok_or_else(|| io::Error::other(format!("display mode {requested} unavailable")));
    }
    // The fastest refresh at the native size is the default (owner, round 75): a faster panel is never left at the
    // firmware's 60 Hz. `refresh=firmware` in /etc/bifrost/display.conf keeps the preferred mode (no modeset).
    let native = modes.iter().find(|m| m.kind & 8 != 0).or(modes.first()).ok_or_else(|| io::Error::other("no display modes"))?;
    if !max_refresh {
        return Ok(*native);
    }
    let best = modes.iter().filter(|m| m.hdisplay == native.hdisplay && m.vdisplay == native.vdisplay).max_by_key(|m| (m.vrefresh, m.kind & 8 != 0));
    Ok(*best.unwrap_or(native))
}

/// The fastest refresh at native size unless /etc/bifrost/display.conf says `refresh=firmware` (the same switch the
/// session's video-arg and display-refresh read) or Settings chose `native` (/var/lib/springboard/display-refresh).
fn refresh_max_requested() -> bool {
    let firmware = std::fs::read_to_string("/etc/bifrost/display.conf").is_ok_and(|c| c.split_whitespace().any(|t| t == "refresh=firmware"));
    let native = std::fs::read_to_string("/var/lib/springboard/display-refresh").is_ok_and(|c| c.trim() == "native");
    !firmware && !native
}

/// The refresh rates the panel offers at a size, fastest first, from the mode timings (164.85 Hz is not a whole
/// `vrefresh`). Interlaced modes are left out.
fn refresh_rates(modes: &[ModeInfo], w: u16, h: u16) -> Vec<f32> {
    const INTERLACE: u32 = 1 << 4;
    let mut rates: Vec<f32> = modes
        .iter()
        .filter(|m| m.hdisplay == w && m.vdisplay == h && m.clock != 0 && m.htotal != 0 && m.vtotal != 0 && m.flags & INTERLACE == 0)
        .map(|m| ((m.clock as f64 * 1000.0 / (m.htotal as f64 * m.vtotal as f64)) * 100.0).round() as f32 / 100.0)
        .collect();
    rates.sort_by(|a, b| b.total_cmp(a));
    rates.dedup();
    rates
}

/// The refresh rates `wlr-randr` lists at the size of the current mode, fastest first (`2560x1440 px, 164.847000 Hz
/// (current)` lines of the first enabled output).
pub fn wlr_randr_rates(text: &str) -> Vec<f32> {
    let parse = |l: &str| {
        let (size, rest) = l.trim().split_once(" px, ")?;
        let hz: f32 = rest.split_whitespace().next()?.parse().ok()?;
        Some((size.to_string(), hz, rest.contains("current")))
    };
    let modes: Vec<(String, f32, bool)> = text.lines().filter_map(parse).collect();
    let Some(size) = modes.iter().find(|m| m.2).map(|m| m.0.clone()) else { return Vec::new() };
    let mut rates: Vec<f32> = modes.iter().filter(|m| m.0 == size).map(|m| (m.1 * 100.0).round() / 100.0).collect();
    rates.sort_by(|a, b| b.total_cmp(a));
    rates.dedup();
    rates
}

#[cfg(test)]
mod wlr_rates_tests {
    #[test]
    fn rates_at_the_current_size_fastest_first() {
        let t = "DP-2 \"x\"\n  Modes:\n    2560x1440 px, 59.951000 Hz (preferred)\n    2560x1440 px, 164.847000 Hz (current)\n    2560x1440 px, 143.998993 Hz\n    1920x1080 px, 60.000000 Hz\n";
        assert_eq!(super::wlr_randr_rates(t), vec![164.85, 144.0, 59.95]);
        assert!(super::wlr_randr_rates("no modes").is_empty());
    }
}

/// The inherited mode stays only when it is the selected size at the selected refresh or faster.
/// Identical timings count as the same mode whatever the rounded `vrefresh` says (59.951 Hz reads as 60).
fn keeps_inherited(inherited: &ModeInfo, selected: &ModeInfo) -> bool {
    let same_timings = inherited.clock != 0 && (inherited.clock, inherited.htotal, inherited.vtotal, inherited.hdisplay, inherited.vdisplay)
        == (selected.clock, selected.htotal, selected.vtotal, selected.hdisplay, selected.vdisplay);
    same_timings || inherited.hdisplay == selected.hdisplay && inherited.vdisplay == selected.vdisplay && inherited.vrefresh >= selected.vrefresh
}

#[cfg(feature = "desktop-wayland")]
static WAYLAND_FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Display {
    #[cfg(feature = "desktop-wayland")]
    wayland: Option<super::wayland::Display>,
    /// first boot only: the loader's frame's hand-off (when the fade into the first screen started), a blend buffer, and
    /// the reveal gate that holds the frame until the desktop is ready (`bootreveal`)
    boot_fade: Option<(bootweave_frame::Handoff, Vec<u32>, crate::bootreveal::Reveal)>,
    /// no panel behind it (emulator, headless): frames are only read back through the control socket
    virt: bool,
    /// when the emulated panel last took a frame: the next present waits for the following vblank
    virt_last: Option<std::time::Instant>,
    /// the panel's physical size in mm from the connector (EDID); None when unknown
    mm: Option<(u32, u32)>,
    fd: c_int,
    crtc: u32,
    conn: u32,
    /// the connector's "DPMS" property id
    dpms: Option<u32>,
    pub mode: ModeInfo,
    /// the refresh rates the panel offers at the current size, fastest first (Settings > Displays > Refresh Rate)
    pub rates: Vec<f32>,
    bufs: [Buffer; 2],
    owed: Owed,
    back: usize,
    /// scanout is XRGB, so red and blue are swapped while copying
    swap_rb: bool,
    flip_pending: bool,
    /// when the pending flip was queued, to give up on an event that never comes
    flip_at: Option<std::time::Instant>,
    use_flip: bool,
    /// handle of the 64x64 hardware cursor buffer, once one is set
    cursor: Option<u32>,
    /// the hardware cursor is on the plane (shown, not hidden)
    cursor_on: bool,
}

fn arr<T: Default + Clone>(n: u32) -> Vec<T> {
    vec![T::default(); n as usize]
}

impl Display {
    /// Open the first usable DRM card on a Linux desktop. Firmware framebuffers can
    /// claim card0 before virtio-gpu or a discrete GPU, and some kernels omit the
    /// corresponding /dev node; probe every card instead of assuming card0.
    /// Like `open_first_available`, but for a machine whose real GPU driver may still be loading: while only
    /// firmware framebuffers (simpledrm) exist and a PCI display device has no driver yet, wait up to `wait_ms` for the
    /// real driver (after a kexec the firmware framebuffer's geometry can be wrong, the real driver never is).
    pub fn open_best(wait_ms: u64) -> io::Result<Self> {
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < wait_ms as u128 {
            let drivers = drm_drivers();
            if drivers.iter().any(|d| !is_firmware_fb(d)) || !pci_display_without_driver() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        Self::open_first_available()
    }

    pub fn open_first_available() -> io::Result<Self> {
        let mut cards = std::fs::read_dir("/dev/dri")?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                name.strip_prefix("card")?.parse::<u32>().ok().map(|n| (n, entry.path()))
            })
            .collect::<Vec<_>>();
        // real GPU drivers first: a firmware framebuffer (simpledrm) can carry the wrong geometry
        cards.sort_by_key(|(n, _)| (is_firmware_fb(&card_driver(*n)), *n));
        if cards.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no DRM card nodes in /dev/dri"));
        }
        let mut errors = Vec::new();
        for (_, path) in cards {
            match Self::open(path.to_str().unwrap_or_default()) {
                Ok(display) => {
                    eprintln!("springboard: KMS device {}", path.display());
                    return Ok(display);
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        Err(io::Error::other(format!("no usable DRM card ({})", errors.join("; "))))
    }

    pub fn open(path: &str) -> io::Result<Self> {
        let fd = open_dev(path, O_RDWR)?;
        // Become DRM master; fails if a compositor still holds the card.
        unsafe { ioctl_ptr::<()>(fd, io(b'd', 0x1e), std::ptr::null_mut()) }?;

        let mut res = CardRes::default();
        unsafe { ioctl_ptr(fd, iowr::<CardRes>(b'd', 0xa0), &mut res) }?;
        let (mut crtcs, mut conns): (Vec<u32>, Vec<u32>) = (arr(res.count_crtcs), arr(res.count_conns));
        let (mut fbs, mut encs): (Vec<u32>, Vec<u32>) = (arr(res.count_fbs), arr(res.count_encs));
        res.crtc_ptr = crtcs.as_mut_ptr() as u64;
        res.conn_ptr = conns.as_mut_ptr() as u64;
        res.fb_ptr = fbs.as_mut_ptr() as u64;
        res.enc_ptr = encs.as_mut_ptr() as u64;
        unsafe { ioctl_ptr(fd, iowr::<CardRes>(b'd', 0xa0), &mut res) }?;

        let mut found = None;
        for &id in &conns {
            let mut c = GetConnector { connector_id: id, ..Default::default() };
            unsafe { ioctl_ptr(fd, iowr::<GetConnector>(b'd', 0xa7), &mut c) }?;
            if c.connection != 1 || c.count_modes == 0 {
                continue;
            }
            let mut modes: Vec<ModeInfo> = arr(c.count_modes);
            let (ne, np) = (c.count_encs, c.count_props);
            let mut encs2: Vec<u32> = arr(ne);
            let mut props: Vec<u32> = arr(np);
            let mut vals: Vec<u64> = arr(np);
            c.modes_ptr = modes.as_mut_ptr() as u64;
            c.enc_ptr = encs2.as_mut_ptr() as u64;
            c.props_ptr = props.as_mut_ptr() as u64;
            c.prop_values_ptr = vals.as_mut_ptr() as u64;
            unsafe { ioctl_ptr(fd, iowr::<GetConnector>(b'd', 0xa7), &mut c) }?;
            let requested = std::env::var("SPRINGBOARD_MODE").ok();
            let mode = select_mode(&modes, requested.as_deref(), refresh_max_requested())?;
            let enc_id = if c.encoder_id != 0 { c.encoder_id } else { encs2.first().copied().unwrap_or(0) };
            let mut e = GetEncoder { encoder_id: enc_id, ..Default::default() };
            unsafe { ioctl_ptr(fd, iowr::<GetEncoder>(b'd', 0xa6), &mut e) }?;
            let crtc = if e.crtc_id != 0 { e.crtc_id } else { crtcs.iter().enumerate().find(|(i, _)| e.possible_crtcs & (1 << i) != 0).map(|(_, &c)| c).or(crtcs.first().copied()).ok_or_else(|| io::Error::other("no CRTC"))? };
            let dpms = props.iter().copied().find(|&prop_id| {
                let mut p = GetProperty { prop_id, ..Default::default() };
                unsafe { ioctl_ptr(fd, iowr::<GetProperty>(b'd', 0xaa), &mut p) }.is_ok() && p.is_named("DPMS")
            });
            // Keep the mode the loader set and the driver inherited (fastboot) when it is already the best one.
            // The loader sets the preferred mode and so does the selection (unless refresh=max): no modeset.
            let mut active = Crtc { crtc_id: crtc, ..Default::default() };
            let inherited = requested.is_none()
                && unsafe { ioctl_ptr(fd, iowr::<Crtc>(b'd', 0xa1), &mut active) }.is_ok()
                && active.mode_valid == 1
                && active.mode.hdisplay != 0
                && keeps_inherited(&active.mode, &mode);
            let mode = if inherited { active.mode } else { mode };
            let rates = refresh_rates(&modes, mode.hdisplay, mode.vdisplay);
            eprintln!("springboard: display mode {}x{}@{} ({})", mode.hdisplay, mode.vdisplay, mode.vrefresh, if inherited { "inherited" } else { "selected" });
            found = Some((id, crtc, mode, dpms, (c.mm_w, c.mm_h), rates));
            break;
        }
        let (conn, crtc, mode, dpms, mm, rates) = found.ok_or_else(|| io::Error::other("no connected display"))?;
        let (w, h) = (mode.hdisplay as u32, mode.vdisplay as u32);

        let mut swap_rb = false;
        let mut make = || -> io::Result<Buffer> {
            let mut d = CreateDumb { width: w, height: h, bpp: 32, ..Default::default() };
            unsafe { ioctl_ptr(fd, iowr::<CreateDumb>(b'd', 0xb2), &mut d) }?;
            let mut f2 = FbCmd2 { width: w, height: h, pixel_format: XBGR8888, ..Default::default() };
            f2.handles[0] = d.handle;
            f2.pitches[0] = d.pitch;
            let fb = if unsafe { ioctl_ptr(fd, iowr::<FbCmd2>(b'd', 0xb8), &mut f2) }.is_ok() {
                f2.fb_id
            } else {
                let mut f = FbCmd { width: w, height: h, pitch: d.pitch, bpp: 32, depth: 24, handle: d.handle, ..Default::default() };
                unsafe { ioctl_ptr(fd, iowr::<FbCmd>(b'd', 0xae), &mut f) }?;
                swap_rb = true;
                f.fb_id
            };
            let mut m = MapDumb { handle: d.handle, ..Default::default() };
            unsafe { ioctl_ptr(fd, iowr::<MapDumb>(b'd', 0xb3), &mut m) }?;
            let map = unsafe { mmap(std::ptr::null_mut(), d.size as usize, PROT_RW, MAP_SHARED, fd, m.offset as i64) };
            if map as isize == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(Buffer { fb, pitch: d.pitch as usize / 4, map: map as *mut u32 })
        };
        let bufs = [make()?, make()?];
        let owed = Owed::new(w as usize, h as usize);
        let mut disp = Display { #[cfg(feature = "desktop-wayland")] wayland: None, boot_fade: None, virt: false, virt_last: None, mm: Some(mm), fd, crtc, conn, dpms, mode, rates, bufs, owed, back: 1, swap_rb, flip_pending: false, flip_at: None, use_flip: true, cursor: None, cursor_on: false };
        disp.set_crtc(0)?;
        Ok(disp)
    }

    /// Hardware cursor plane (what cosmic-comp/smithay do): the pointer is a 64x64 ARGB sprite the display
    /// controller overlays and moves on its own, so it follows the mouse without recomposing a frame.
    /// `scale` is Cursor Size (1.0 is the sprite as drawn; see `scale_sprite` for what a larger one keeps).
    pub fn set_cursor(&mut self, argb: &[u32], scale: f32) -> bool {
        if self.virt || argb.len() != 64 * 64 {
            return false;
        }
        let side = cursor_side(scale);
        let sprite = scale_sprite(argb, side);
        let ok = (|| -> io::Result<u32> {
            let mut d = CreateDumb { width: side as u32, height: side as u32, bpp: 32, ..Default::default() };
            unsafe { ioctl_ptr(self.fd, iowr::<CreateDumb>(b'd', 0xb2), &mut d) }?;
            let mut m = MapDumb { handle: d.handle, ..Default::default() };
            unsafe { ioctl_ptr(self.fd, iowr::<MapDumb>(b'd', 0xb3), &mut m) }?;
            let map = unsafe { mmap(std::ptr::null_mut(), d.size as usize, PROT_RW, MAP_SHARED, self.fd, m.offset as i64) };
            if map as isize == -1 {
                return Err(io::Error::last_os_error());
            }
            unsafe { std::ptr::copy_nonoverlapping(sprite.as_ptr(), map as *mut u32, side * side) };
            let mut c = CursorCmd { flags: 1, crtc_id: self.crtc, width: side as u32, height: side as u32, handle: d.handle, ..Default::default() };
            unsafe { ioctl_ptr(self.fd, iowr::<CursorCmd>(b'd', 0xa3), &mut c) }?;
            Ok(d.handle)
        })();
        self.cursor = ok.ok();
        self.cursor.is_some()
    }

    /// Moves the hardware cursor; false when there is none (draw it in software instead).
    pub fn move_cursor(&mut self, x: i32, y: i32) -> bool {
        if self.cursor.is_none() {
            return false;
        }
        let mut c = CursorCmd { flags: 2, crtc_id: self.crtc, x, y, ..Default::default() };
        unsafe { ioctl_ptr(self.fd, iowr::<CursorCmd>(b'd', 0xa3), &mut c) }.is_ok()
    }

    /// Shows the hardware cursor with its top-left corner at (x, y); false when there is none.
    pub fn show_cursor(&mut self, x: i32, y: i32) -> bool {
        let Some(handle) = self.cursor else { return false };
        let mut c = CursorCmd { flags: 1 | 2, crtc_id: self.crtc, x, y, width: 64, height: 64, handle };
        self.cursor_on = unsafe { ioctl_ptr(self.fd, iowr::<CursorCmd>(b'd', 0xa3), &mut c) }.is_ok();
        self.cursor_on
    }

    /// Hides the hardware cursor; its buffer stays for the next `show_cursor`.
    pub fn hide_cursor(&mut self) {
        if !self.cursor_on {
            return;
        }
        let mut c = CursorCmd { flags: 1, crtc_id: self.crtc, ..Default::default() };
        let _ = unsafe { ioctl_ptr(self.fd, iowr::<CursorCmd>(b'd', 0xa3), &mut c) };
        self.cursor_on = false;
    }

    #[cfg(feature = "desktop-wayland")]
    pub fn wayland() -> io::Result<Self> {
        let backend = super::wayland::Display::open()?;
        let (w,h) = backend.size();
        let mut display = Self::virtual_panel(w,h,60);
        display.mode.vrefresh = backend.refresh();
        // the rates Settings > Displays > Refresh Rate offers: the window server's modes at the current size
        display.rates = std::process::Command::new("wlr-randr").output().ok()
            .map(|o| wlr_randr_rates(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
        display.wayland = Some(backend);
        // No shell copy of the boot frame under bifrost-wm: its surface can be scaled, so a frame drawn here would not
        // match the loader's mark box in panel pixels. Bootweave's frame stays up until the first present.
        Ok(display)
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_activity(&mut self) -> (bool,bool) {
        self.wayland.as_mut().map(|w|(w.active(),w.resumed())).unwrap_or((false,false))
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_lock(&mut self, wanted:bool) -> io::Result<bool> {
        if let Some(w)=self.wayland.as_mut(){w.lock(wanted)?;return Ok(w.needs_redraw());}
        Ok(false)
    }
    /// Sends the shell's pending output mode request to the window server (see `wayland::Display::output_modes`).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_output_modes(&mut self) -> io::Result<()> {
        if let Some(w)=&mut self.wayland { w.output_modes()?; }
        Ok(())
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_inputs(&mut self, out: &mut Vec<super::input::Event>) {
        if let Some(w) = &mut self.wayland { w.inputs(out); }
    }
    /// The window server's input method batches since the last call (a focused client's text field).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_input_method_batches(&mut self) -> Vec<crate::shell::textinput::ImBatch> {
        self.wayland.as_mut().map(|w| w.input_method_batches()).unwrap_or_default()
    }
    /// The keyboard's output for the focused client's field, through the input method (see `wayland::Display`).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_input_method_commit(&mut self, out: &[crate::shell::textinput::ImOut]) -> io::Result<()> {
        if let Some(w) = &mut self.wayland { w.input_method_commit(out)?; }
        Ok(())
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_chrome(&mut self,frames:Vec<super::wayland::ChromeFrame>)->io::Result<()> {if let Some(w)=&mut self.wayland {w.chrome(frames)?;}Ok(())}
    /// The CPU draw time of the frame just presented, for the GPU presenter's averages.
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_note_draw(&mut self, ms: f64) {if let Some(w)=&mut self.wayland {w.note_draw(ms);}}
    /// The wallpaper the GPU presenter draws under the desktop; a no-op on the SHM path.
    /// True when the GPU presenter is drawing (it keeps the wallpaper texture).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_trim_wake_ms(&self) -> Option<i32> {self.wayland.as_ref().and_then(|w| w.trim_wake_ms())}
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_gpu_active(&self) -> bool {self.wayland.as_ref().is_some_and(|w| w.gpu_active())}
    /// True once after the GPU presenter gave up for the SHM path: the shell draws a full frame.
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_take_gpu_fallback(&mut self) -> bool {self.wayland.as_mut().is_some_and(|w| w.take_gpu_fallback())}
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_wallpaper(&mut self, wp: crate::gfx::View, x: i32) {if let Some(w)=&mut self.wayland {w.wallpaper(wp,x);}}
    /// Screen Zoom state and pointer for the next present (see `wayland::Display::set_zoom`).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_set_zoom(&mut self, zoom: crate::shell::accessibility::zoom::ScreenZoom, pointer: (f32, f32)) {if let Some(w)=&mut self.wayland {w.set_zoom(zoom,pointer);}}
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_foreign_maximize(&mut self,id:u32,maximized:bool)->io::Result<()> {if let Some(w)=&mut self.wayland {w.foreign_maximize(id,maximized)?;}Ok(())}
    #[cfg(feature="desktop-wayland")]
    pub fn wayland_foreign_action(&mut self,id:u32,minimize:bool)->io::Result<()> {if let Some(w)=&mut self.wayland {w.foreign_action(id,minimize)?;}Ok(())}
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_windows(&mut self, frames: Vec<super::wayland::WindowFrame>) -> io::Result<()> {
        if let Some(w) = &mut self.wayland { w.windows(frames)?; }
        Ok(())
    }
    /// Each window's behind-window materials (window px); the GPU presenter blurs the desktop under them.
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_window_materials(&mut self, list: Vec<(u32, Vec<bifrost_render::glass::Rect>)>) {
        if let Some(w) = &mut self.wayland { for (id, rects) in list { w.window_materials(id, rects); } }
    }
    /// Points to px for the presenter's window metrics (shadow reach, radius).
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_set_scale(&mut self, s: f32) { if let Some(w) = &mut self.wayland { w.set_scale(s); } }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_window_events(&mut self) -> Vec<super::wayland::WindowEvent> {
        self.wayland.as_mut().map(|w| w.window_events()).unwrap_or_default()
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_foreign_windows(&mut self) -> Option<Vec<super::wayland::ForeignWindow>> {
        self.wayland.as_mut().and_then(|w| w.foreign_windows())
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_raise_front(&mut self) { if let Some(w)=&mut self.wayland { w.raise_front(); } }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_activate_foreign(&mut self,id:u32) -> io::Result<()> {
        if let Some(w)=&mut self.wayland { w.activate_foreign(id)?; } Ok(())
    }
    /// A display with nothing behind it: presents are dropped, the fd never becomes ready.
    pub fn virtual_panel(w: usize, h: usize, hz: u32) -> Self {
        let mode = ModeInfo { hdisplay: w as u16, vdisplay: h as u16, vrefresh: hz.max(1), ..Default::default() };
        let none = || Buffer { fb: 0, pitch: 0, map: std::ptr::null_mut() };
        Display { #[cfg(feature = "desktop-wayland")] wayland: None, boot_fade: None, virt: true, virt_last: None, mm: None, fd: -1, crtc: 0, conn: 0, dpms: None, cursor: None, cursor_on: false, mode, rates: Vec::new(), bufs: [none(), none()], owed: Owed::new(w, h), back: 0, swap_rb: false, flip_pending: false, flip_at: None, use_flip: false }
    }

    /// The panel's physical size in mm, when the connector reported one (never for a virtual panel).
    pub fn size_mm(&self) -> Option<(u32, u32)> {
        self.mm.filter(|&(w, h)| w > 0 && h > 0)
    }

    pub fn size(&self) -> (usize, usize) {
        (self.mode.hdisplay as usize, self.mode.vdisplay as usize)
    }

    pub fn fd(&self) -> c_int {
        #[cfg(feature = "desktop-wayland")]
        if let Some(w) = &self.wayland { return w.fd(); }
        self.fd
    }

    fn set_crtc(&mut self, buf: usize) -> io::Result<()> {
        let mut conn = self.conn;
        let mut c = Crtc {
            set_connectors_ptr: &mut conn as *mut u32 as u64,
            count_connectors: 1,
            crtc_id: self.crtc,
            fb_id: self.bufs[buf].fb,
            mode_valid: 1,
            mode: self.mode,
            ..Default::default()
        };
        unsafe { ioctl_ptr(self.fd, iowr::<Crtc>(b'd', 0xa2), &mut c) }
    }

    /// Powers the panel down or up through the connector's DPMS property.
    /// Nothing may be presented while it is off.
    pub fn set_dpms(&mut self, on: bool) -> io::Result<()> {
        #[cfg(feature = "desktop-wayland")]
        if let Some(w)=&mut self.wayland {return w.power(on);}
        if self.virt {
            return Ok(());
        }
        let prop_id = self.dpms.ok_or_else(|| io::Error::other("connector has no DPMS property"))?;
        let mut p = SetConnectorProperty { value: if on { DPMS_ON } else { DPMS_OFF }, prop_id, connector_id: self.conn };
        unsafe { ioctl_ptr(self.fd, iowr::<SetConnectorProperty>(b'd', 0xab), &mut p) }
    }

    /// Drains a completed page-flip event from the DRM fd.
    /// Drains DRM events; returns the scanout time (CLOCK_MONOTONIC ms) of
    /// the last completed page flip, if one arrived.
    pub fn handle_event(&mut self) -> Option<f64> {
        #[cfg(feature = "desktop-wayland")]
        if let Some(w) = &mut self.wayland {
            w.read().unwrap_or_else(|e| { eprintln!("springboard: Wayland: {e}"); std::process::exit(1) });
            return w.take_flip();
        }
        if self.virt {
            return None;
        }
        let mut buf = [0u8; 256];
        let n = unsafe { read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n <= 0 {
            return None;
        }
        self.flip_pending = false;
        flip_time(&buf[..n as usize])
    }

    /// Refresh period in ms, from the mode timings (more exact than vrefresh).
    pub fn period_ms(&self) -> f64 {
        let m = &self.mode;
        let total = m.htotal as f64 * m.vtotal as f64;
        if m.clock == 0 || total == 0.0 {
            return 1000.0 / m.vrefresh.max(1) as f64;
        }
        total / m.clock as f64
    }

    /// A flip whose event has not arrived within a few frame periods is presumed lost.
    fn flip_stale(&self) -> bool {
        let limit = (self.period_ms() * 4.0).max(40.0);
        self.flip_at.is_some_and(|t| t.elapsed().as_secs_f64() * 1e3 > limit)
    }

    /// Waits (briefly) for a queued flip to land, so the next frame may be queued.
    fn settle_flip(&mut self) {
        if self.flip_pending {
            let _ = wait(&[self.fd], 50);
            let _ = self.handle_event();
            if self.flip_pending && self.flip_stale() {
                self.flip_pending = false;
            }
        }
    }

    /// Allocates `n` zero-copy render buffers of the panel size (see `scanout`).
    /// Nothing is shown yet; on error nothing stays allocated.
    pub fn scanout_alloc(&mut self, method: super::scanout::Method, fourcc: u32, n: usize) -> io::Result<Vec<super::scanout::ScanoutBuf>> {
        if self.virt {
            return Err(io::Error::other("virtual display has no scanout buffers"));
        }
        let (w, h) = self.size();
        let mut out = Vec::new();
        for _ in 0..n {
            match super::scanout::alloc(method, self.fd, w, h, fourcc) {
                Ok(b) => out.push(b),
                Err(e) => {
                    self.scanout_free(out);
                    return Err(e);
                }
            }
        }
        Ok(out)
    }

    /// Releases buffers from `scanout_alloc`. The panel must not be showing one
    /// (present a CPU frame first); the EGL images must be gone.
    pub fn scanout_free(&mut self, bufs: Vec<super::scanout::ScanoutBuf>) {
        for b in bufs {
            b.release(self.fd);
        }
    }

    /// Page-flips to a GPU-rendered framebuffer (no copy). False when the flip
    /// was refused: the caller must go back to the copy path. Both CPU buffers
    /// are stale afterwards, so the next CPU frame is copied in full.
    pub fn flip_scanout(&mut self, fb: u32) -> bool {
        if self.virt {
            return false;
        }
        self.settle_flip();
        let mut f = PageFlip { crtc_id: self.crtc, fb_id: fb, flags: PAGE_FLIP_EVENT, ..Default::default() };
        if unsafe { ioctl_ptr(self.fd, iowr::<PageFlip>(b'd', 0xb0), &mut f) }.is_err() {
            return false;
        }
        self.flip_pending = true;
        self.flip_at = Some(std::time::Instant::now());
        self.owed.stale_all();
        true
    }

    /// Waits (briefly) until a queued flip has landed, so buffers the panel
    /// was showing may be released.
    pub fn wait_flip(&mut self) {
        self.settle_flip();
    }

    pub fn busy(&self) -> bool {
        #[cfg(feature = "desktop-wayland")]
        if let Some(w) = &self.wayland { return w.busy(); }
        self.flip_pending && !self.flip_stale()
    }

    /// Copies what changed in the frame (w*h, R in the low byte) to the back
    /// buffer and flips. `damage` is what this frame changed; each buffer
    /// also still owes the previous frame's changes, so it receives the union
    /// of the two. Nothing to copy means nothing to flip.
    /// True while the first screen is still fading in from the boot frame: the caller keeps drawing full frames.
    pub fn boot_fading(&self) -> bool {
        self.boot_fade.is_some()
    }

    pub fn present(&mut self, frame: &[u32], damage: &Damage) {
        if let Some((mut hand, mut mix, mut reveal)) = self.boot_fade.take() {
            let (w, h) = self.size();
            let mut all = Damage::new(w, h);
            all.all();
            // reveal once: the boot frame stays up until the wallpaper, menu bar and Dock have presented (or the ceiling)
            let now = crate::bootsplash::boottime_ms();
            let was_ready = reveal.ready_at().is_some();
            if reveal.poll(now, || std::fs::read_to_string(crate::bootsplash::MARKS).unwrap_or_default()).is_none() {
                hand.hold(&mut mix, now as f32);
                self.present(&mix, &all);
                self.boot_fade = Some((hand, mix, reveal));
                return;
            }
            if !was_ready {
                crate::bootsplash::mark("desktop-ready");
            }
            if !hand.started() {
                crate::bootsplash::mark("desktop-first-ui");
            }
            // the loader's bar fills to the end, then the boot frame fades into the first screen
            if !hand.compose(frame, &mut mix, super::sys::now_ms() as f32, now as f32) {
                crate::bootsplash::mark("desktop-faded");
                return self.present(frame, &all);
            }
            self.present(&mix, &all);
            self.boot_fade = Some((hand, mix, reveal));
            return;
        }
        #[cfg(feature = "desktop-wayland")]
        if let Some(w) = &mut self.wayland {
            if std::env::var_os("BIFROST_BOOT_FADE").is_some() && !WAYLAND_FIRST.swap(true, std::sync::atomic::Ordering::Relaxed) {
                // the boot milestones the shell's own fade used to write (bench and boot-time learning read them)
                crate::bootsplash::mark("desktop-ready");
                crate::bootsplash::mark("desktop-first-ui");
                crate::bootsplash::mark("desktop-faded");
            }
            w.present(frame, damage).unwrap_or_else(|e| { eprintln!("springboard: Wayland present: {e}"); std::process::exit(1) });
            return;
        }
        if self.virt {
            // an emulated panel still has a refresh rate: block until the vblank a real flip would wait for
            let period = std::time::Duration::from_secs_f64(self.period_ms() / 1e3);
            if let Some(due) = self.virt_last.map(|t| t + period) {
                std::thread::sleep(due.saturating_duration_since(std::time::Instant::now()));
            }
            self.virt_last = Some(std::time::Instant::now());
            return;
        }
        self.settle_flip();
        self.owed.note(damage);
        let due = self.owed.due(self.back);
        if due.is_empty() {
            return;
        }
        let (w, _) = self.size();
        let b = &self.bufs[self.back];
        copy_areas(frame, w, Map(b.map), b.pitch, due.areas(), self.swap_rb, threads());
        self.owed.paid(self.back);
        if self.use_flip {
            let mut f = PageFlip { crtc_id: self.crtc, fb_id: b.fb, flags: PAGE_FLIP_EVENT, ..Default::default() };
            if unsafe { ioctl_ptr(self.fd, iowr::<PageFlip>(b'd', 0xb0), &mut f) }.is_ok() {
                self.flip_pending = true;
                self.flip_at = Some(std::time::Instant::now());
            } else {
                self.use_flip = false;
            }
        }
        if !self.use_flip {
            let _ = self.set_crtc(self.back);
        }
        self.back ^= 1;
    }
}

const DRM_EVENT_FLIP_COMPLETE: u32 = 2;

/// Side of the cursor plane for a Cursor Size: 64 px at 1x, growing with the scale up to 128 px. A driver that
/// refuses the larger plane fails the ioctl and `set_cursor` returns false, so the pointer is drawn in software.
pub fn cursor_side(scale: f32) -> usize {
    let s = if scale.is_finite() { scale.clamp(1.0, 2.0) } else { 1.0 };
    ((64.0 * s).round() as usize).clamp(64, 128)
}

/// The 64x64 sprite resampled to `side` x `side` (nearest neighbour). The hotspot stays at the top left.
pub fn scale_sprite(src: &[u32], side: usize) -> Vec<u32> {
    let mut out = vec![0u32; side * side];
    for y in 0..side {
        for x in 0..side {
            out[y * side + x] = src[(y * 64 / side) * 64 + x * 64 / side];
        }
    }
    out
}

/// Timestamp of the last flip-complete event in a read() of DRM events
/// (struct drm_event header + drm_event_vblank: user_data, tv_sec, tv_usec, ...).
pub fn flip_time(buf: &[u8]) -> Option<f64> {
    let u32_at = |o: usize| buf.get(o..o + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()));
    let mut p = 0;
    let mut last = None;
    while let (Some(kind), Some(len)) = (u32_at(p), u32_at(p + 4)) {
        if len < 8 {
            break;
        }
        if kind == DRM_EVENT_FLIP_COMPLETE {
            if let (Some(sec), Some(usec)) = (u32_at(p + 16), u32_at(p + 20)) {
                last = Some(sec as f64 * 1e3 + usec as f64 / 1e3);
            }
        }
        p += len as usize;
    }
    last
}

/// The next vsync at or after `now`, given the last flip time and period.
pub fn next_vsync(last_flip: Option<f64>, period: f64, now: f64) -> f64 {
    match last_flip {
        Some(v) if now >= v && now - v < 4.0 * period => v + ((now - v) / period).ceil().max(1.0) * period,
        _ => now,
    }
}

/// Running estimate of what a frame costs to draw and copy out, in ms: it rises fast (one slow
/// frame must stop the late start at once) and falls slowly.
#[derive(Clone, Copy, Debug)]
pub struct FrameCost(f64);

impl FrameCost {
    /// Starts at half a refresh period, a guess that leaves room until the first real frames.
    pub fn new(period: f64) -> Self {
        FrameCost(period * 0.5)
    }

    pub fn note(&mut self, ms: f64) {
        let ms = ms.max(0.0);
        self.0 = if ms > self.0 { 0.5 * self.0 + 0.5 * ms } else { 0.9 * self.0 + 0.1 * ms };
    }

    pub fn ms(&self) -> f64 {
        self.0
    }
}

/// When to start drawing the frame that lands on the next vblank, so it is as fresh as
/// possible (input and animation time sampled late) yet done before the flip. `None` = draw
/// now: no recent flip to predict from, or frames cost too much of the period to wait. With
/// a flip still pending nothing is drawn before it lands.
pub fn draw_start(last_flip: Option<f64>, period: f64, now: f64, cost: f64, busy: bool) -> Option<f64> {
    let v = last_flip.filter(|&v| now >= v && now - v < 4.0 * period)?;
    if cost > 0.7 * period {
        return None;
    }
    let vsync = next_vsync(Some(v), period, now);
    Some(if busy { vsync } else { vsync - cost * 1.25 - 1.0 })
}

type NFds = std::ffi::c_ulong;

extern "C" {
    fn ppoll(fds: *mut pollfd, n: NFds, timeout: *const timespec, mask: *const u8) -> c_int;
}

/// `sys::wait_into` with a fractional-millisecond timeout (ppoll takes a timespec): a 120 Hz
/// frame is 8.33 ms and `poll`'s whole milliseconds would round it up to 9. Negative or NaN
/// waits poll once; a huge one blocks (an unreachable timeout is infinite).
pub fn wait_precise(fds: &[c_int], timeout_ms: f64, out: &mut Vec<bool>) {
    thread_local! {
        static SCRATCH: std::cell::RefCell<Vec<pollfd>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    SCRATCH.with(|s| {
        let mut p = s.borrow_mut();
        p.clear();
        p.extend(fds.iter().map(|&fd| pollfd { fd, events: POLLIN, revents: 0 }));
        let ms = if timeout_ms.is_nan() { 0.0 } else { timeout_ms.max(0.0) };
        let ts = timespec { tv_sec: (ms / 1e3).min(1e9) as i64, tv_nsec: ((ms % 1e3) * 1e6) as _ };
        let forever = ms >= 1e9;
        // SAFETY: `p` and `ts` live through the call; a null sigmask leaves the signal mask alone
        unsafe { ppoll(p.as_mut_ptr(), p.len() as NFds, if forever { std::ptr::null() } else { &ts }, std::ptr::null()) };
        out.clear();
        out.extend(p.iter().map(|f| f.revents & (POLLIN | POLLHUP | POLLERR) != 0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_side_grows_with_cursor_size_and_stays_in_range() {
        assert_eq!(cursor_side(1.0), 64);
        assert_eq!(cursor_side(1.5), 96);
        assert_eq!(cursor_side(2.0), 128);
        assert_eq!(cursor_side(9.0), 128);
        assert_eq!(cursor_side(0.2), 64);
        assert_eq!(cursor_side(f32::NAN), 64);
    }

    #[test]
    fn scale_sprite_keeps_the_sprite_at_1x_and_doubles_it_at_2x() {
        let src: Vec<u32> = (0..64 * 64).map(|i| i as u32).collect();
        assert_eq!(scale_sprite(&src, 64), src);
        let big = scale_sprite(&src, 128);
        assert_eq!(big.len(), 128 * 128);
        // the top-left source pixel fills the 2x2 block at the hotspot; the next source column starts at x = 2
        assert_eq!((big[0], big[1], big[128], big[129]), (src[0], src[0], src[0], src[0]));
        assert_eq!(big[2], src[1]);
        assert_eq!(big[2 * 128], src[64]);
        assert_eq!(big[127 * 128 + 127], src[63 * 64 + 63]);
    }

    #[test]
    fn virtual_size_parses_the_environment() {
        assert_eq!(virtual_size(Some("1200x1920")), Some((1200, 1920, 60)));
        assert_eq!(virtual_size(Some("1920x1080")), Some((1920, 1080, 60)));
        assert_eq!(virtual_size(Some("1280x800@30")), Some((1280, 800, 30)));
        assert_eq!(virtual_size(Some("3840x2160")), Some((3840, 2160, 60)));
        assert_eq!(virtual_size(Some(" 800x600@30\n")), Some((800, 600, 30)));
        assert_eq!(virtual_size(None), None);
        assert_eq!(virtual_size(Some("")), None);
        assert_eq!(virtual_size(Some("1200")), None);
        assert_eq!(virtual_size(Some("0x10")), None);
        assert_eq!(virtual_size(Some("100000x100")), None);
        assert_eq!(virtual_size(Some("800x600@0")), None);
    }

    #[test]
    fn firmware_framebuffers_rank_behind_real_gpu_drivers() {
        assert!(is_firmware_fb("simpledrm") && is_firmware_fb("efifb") && !is_firmware_fb("i915") && !is_firmware_fb("amdgpu") && !is_firmware_fb("virtio_gpu"));
        let mut cards = [(0u32, "simpledrm"), (1, "i915")];
        cards.sort_by_key(|(n, d)| (is_firmware_fb(d), *n));
        assert_eq!(cards[0].1, "i915", "the real driver is tried first even though simpledrm is card0");
    }

    #[test]
    fn refresh_rates_come_from_the_timings_at_the_native_size() {
        // htotal x vtotal = 10^6, so clock (kHz) / 1000 is the rate in hertz
        let mode = |hdisplay, vdisplay, clock, flags| ModeInfo { hdisplay, vdisplay, clock, htotal: 1000, vtotal: 1000, flags, ..Default::default() };
        let modes = [
            mode(2560, 1600, 164_850, 0),
            mode(2560, 1600, 144_000, 0),
            mode(2560, 1600, 144_000, 0),
            mode(2560, 1600, 200_000, 0x10),
            mode(1920, 1080, 60_000, 0),
            mode(2560, 1600, 0, 0),
        ];
        assert_eq!(refresh_rates(&modes, 2560, 1600), vec![164.85, 144.0], "fastest first, no repeats, interlaced and other sizes left out");
        assert!(refresh_rates(&modes, 800, 600).is_empty());
    }

    #[test]
    fn fixed_panel_mode_ignores_frontend_preference_and_rejects_missing_modes() {
        let modes = [
            ModeInfo { hdisplay: 2536, vdisplay: 1363, vrefresh: 60, kind: 8, ..Default::default() },
            ModeInfo { hdisplay: 1200, vdisplay: 1920, vrefresh: 60, ..Default::default() },
        ];
        assert_eq!(select_mode(&modes, None, false).unwrap().hdisplay, 2536);
        let hz = [
            ModeInfo { hdisplay: 1920, vdisplay: 1080, vrefresh: 60, kind: 8, ..Default::default() },
            ModeInfo { hdisplay: 1920, vdisplay: 1080, vrefresh: 144, ..Default::default() },
            ModeInfo { hdisplay: 1280, vdisplay: 720, vrefresh: 240, ..Default::default() },
        ];
        let pref = select_mode(&hz, None, false).unwrap();
        assert_eq!((pref.hdisplay, pref.vrefresh), (1920, 60), "no request: the preferred mode, so the firmware's mode is kept");
        let best = select_mode(&hz, None, true).unwrap();
        assert_eq!((best.hdisplay, best.vrefresh), (1920, 144), "refresh=max: max refresh at the native size, never a lower resolution");
        let fixed = select_mode(&modes, Some("1200x1920@60"), false).unwrap();
        assert_eq!((fixed.hdisplay, fixed.vdisplay, fixed.vrefresh), (1200, 1920, 60));
        assert!(select_mode(&modes, Some("1200x1920@75"), false).is_err());
        assert!(select_mode(&modes, Some("bad"), false).is_err());
        assert!(select_mode(&[], None, false).is_err());
    }

    #[test]
    fn inherited_loader_mode_gives_way_to_a_faster_refresh_at_the_same_size() {
        let loader = ModeInfo { hdisplay: 2560, vdisplay: 1440, vrefresh: 60, ..Default::default() };
        let fast = ModeInfo { hdisplay: 2560, vdisplay: 1440, vrefresh: 165, ..Default::default() };
        assert!(!keeps_inherited(&loader, &fast), "the 60 Hz boot mode must be replaced by 165 Hz");
        assert!(keeps_inherited(&fast, &fast));
        assert!(keeps_inherited(&fast, &loader), "already faster than the selection: no modeset");
        let a = ModeInfo { clock: 241_500, htotal: 2720, vtotal: 1481, vrefresh: 60, ..loader };
        assert!(keeps_inherited(&a, &ModeInfo { vrefresh: 59, ..a }), "same timings are the same mode");
        let other_size = ModeInfo { hdisplay: 1920, vdisplay: 1080, vrefresh: 165, ..Default::default() };
        assert!(!keeps_inherited(&loader, &other_size), "a different size is always a modeset");
    }

    #[test]
    fn virtual_display_paces_presents_like_vsync() {
        let mut d = Display::virtual_panel(64, 64, 200); // 5 ms period
        let dmg = Damage::new(64, 64);
        let t = std::time::Instant::now();
        for _ in 0..5 {
            d.present(&vec![0; 64 * 64], &dmg);
        }
        // the first present is immediate, each later one waits for the next vblank
        assert!(t.elapsed().as_secs_f64() * 1e3 >= 4.0 * 5.0 * 0.9, "{:?}", t.elapsed());
        assert!(t.elapsed().as_secs_f64() * 1e3 < 4.0 * 5.0 * 3.0);
    }

    #[test]
    fn virtual_display_has_no_hardware() {
        let mut d = Display::virtual_panel(1200, 1920, 60);
        assert_eq!(d.size(), (1200, 1920));
        assert_eq!(d.fd(), -1);
        assert!(!d.busy());
        assert_eq!(d.handle_event(), None);
        assert!((d.period_ms() - 1000.0 / 60.0).abs() < 0.01);
        assert!(d.set_dpms(false).is_ok() && d.set_dpms(true).is_ok());
        assert!(d.scanout_alloc(super::super::scanout::Method::DumbDmabuf, 0, 2).is_err());
        assert!(!d.flip_scanout(1));
        d.present(&vec![0; 1200 * 1920], &Damage::new(1200, 1920));
        d.wait_flip();
        assert!(!d.busy());
    }

    fn event(kind: u32, sec: u32, usec: u32) -> Vec<u8> {
        let mut e = Vec::new();
        for v in [kind, 32, 0, 0, sec, usec, 7, 1] {
            e.extend(v.to_ne_bytes());
        }
        e
    }

    #[test]
    fn flip_time_takes_the_last_flip_event() {
        let mut buf = event(DRM_EVENT_FLIP_COMPLETE, 10, 500_000);
        buf.extend(event(1, 99, 0));
        buf.extend(event(DRM_EVENT_FLIP_COMPLETE, 11, 0));
        assert_eq!(flip_time(&buf), Some(11_000.0));
        assert_eq!(flip_time(&event(1, 5, 0)), None);
        assert_eq!(flip_time(&[0; 4]), None);
    }

    #[test]
    fn vsync_prediction_snaps_to_the_refresh_grid() {
        let p = 1000.0 / 60.0;
        assert_eq!(next_vsync(Some(1000.0), p, 1000.0 + 0.2 * p), 1000.0 + p);
        assert_eq!(next_vsync(Some(1000.0), p, 1000.0 + 1.5 * p), 1000.0 + 2.0 * p);
        // stale or missing flips fall back to now
        assert_eq!(next_vsync(Some(1000.0), p, 5000.0), 5000.0);
        assert_eq!(next_vsync(None, p, 42.0), 42.0);
    }

    #[test]
    fn uapi_struct_sizes() {
        use std::mem::size_of;
        assert_eq!(size_of::<CardRes>(), 64);
        assert_eq!(size_of::<ModeInfo>(), 68);
        assert_eq!(size_of::<GetConnector>(), 80);
        assert_eq!(size_of::<GetEncoder>(), 20);
        assert_eq!(size_of::<Crtc>(), 104);
        assert_eq!(size_of::<CreateDumb>(), 32);
        assert_eq!(size_of::<MapDumb>(), 16);
        assert_eq!(size_of::<FbCmd2>(), 104);
        assert_eq!(size_of::<FbCmd>(), 28);
        assert_eq!(size_of::<PageFlip>(), 24);
        assert_eq!(size_of::<GetProperty>(), 64);
        assert_eq!(size_of::<SetConnectorProperty>(), 16);
    }

    #[test]
    fn each_buffer_owes_every_frame_since_it_was_last_written() {
        let mut owed = Owed::new(100, 100);
        assert!(owed.due(0).is_all() && owed.due(1).is_all(), "fresh buffers need everything");
        owed.paid(0);
        let mut a = Damage::new(100, 100);
        a.add(0, 0, 10, 10);
        owed.note(&a);
        assert_eq!(owed.due(0).areas(), a.areas());
        assert!(owed.due(1).is_all(), "the other buffer still owes the first full frame");
        owed.paid(1);
        let mut b = Damage::new(100, 100);
        b.add(50, 50, 10, 10);
        owed.note(&b);
        assert_eq!(owed.due(0).areas().len(), 2, "buffer 0 owes both frames");
        assert_eq!(owed.due(1).areas(), b.areas());
        owed.paid(0);
        assert!(owed.due(0).is_empty());
    }

    #[test]
    fn copy_areas_writes_only_the_damage_on_every_thread_count() {
        let (w, h, pitch) = (16usize, 12usize, 20usize);
        let frame: Vec<u32> = (0..(w * h) as u32).map(|i| 0x0100_0000 + i).collect();
        let areas = [Area { x0: 2, y0: 1, x1: 9, y1: 11 }, Area { x0: 10, y0: 3, x1: 16, y1: 5 }];
        for n in 1..=4 {
            for swap in [false, true] {
                let mut out = vec![u32::MAX; pitch * h];
                copy_chunks(&frame, w, Map(out.as_mut_ptr()), pitch, &areas, swap, n, 1);
                for y in 0..h {
                    for x in 0..pitch {
                        let inside = areas.iter().any(|a| x >= a.x0 && x < a.x1 && y >= a.y0 && y < a.y1);
                        let s = frame.get(y * w + x).copied().unwrap_or(0);
                        let want = match (inside, swap) {
                            (false, _) => u32::MAX,
                            (true, false) => s,
                            (true, true) => (s & 0xff00_ff00) | (s & 0xff) << 16 | (s >> 16) & 0xff,
                        };
                        assert_eq!(out[y * pitch + x], want, "n={n} swap={swap} at ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn row_kernels_match_the_scalar_swap_for_every_length_and_alignment() {
        let src: Vec<u32> = (0..300u32).map(|i| i.wrapping_mul(0x9e37_79b1) ^ 0x0102_0304).collect();
        for swap in [false, true] {
            for stream in [false, true] {
                for start in 0..9 {
                    for n in (0..70).chain([127, 128, 129, 250]) {
                        let mut out = vec![0xdead_beefu32; 300];
                        // SAFETY: start + n <= 259 stays inside both buffers
                        unsafe { kernel(swap)(out.as_mut_ptr().add(start), src.as_ptr().add(start), n, stream) };
                        for i in 0..300 {
                            let want = if i >= start && i < start + n { if swap { swap_px(src[i]) } else { src[i] } } else { 0xdead_beef };
                            assert_eq!(out[i], want, "swap={swap} stream={stream} start={start} n={n} i={i}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn threshold_keeps_tiny_damage_on_one_thread_and_still_copies_it() {
        let (w, h) = (64usize, 64usize);
        let frame: Vec<u32> = (0..(w * h) as u32).collect();
        let mut out = vec![0u32; w * h];
        copy_areas(&frame, w, Map(out.as_mut_ptr()), w, &[Area { x0: 0, y0: 0, x1: w, y1: h }], false, 4);
        assert_eq!(out, frame);
    }

    #[test]
    fn render_start_waits_for_the_vblank_minus_the_render_time() {
        let p = 1000.0 / 60.0;
        // the flip landed at 1000; 5 ms of drawing, so start about 7 ms before the next vblank
        let s = draw_start(Some(1000.0), p, 1002.0, 5.0, false).unwrap();
        assert!(s > 1002.0 && s < 1000.0 + p - 5.0, "{s}");
        // a slow renderer or no flip history draws at once
        assert_eq!(draw_start(Some(1000.0), p, 1002.0, 14.0, false), None);
        assert_eq!(draw_start(None, p, 1002.0, 5.0, false), None);
        assert_eq!(draw_start(Some(1000.0), p, 9000.0, 5.0, false), None);
        // a flip still pending: nothing before it lands
        assert_eq!(draw_start(Some(1000.0), p, 1010.0, 5.0, true), Some(1000.0 + p));
        // already past the start time: now
        assert!(draw_start(Some(1000.0), p, 1000.0 + p - 1.0, 5.0, false).unwrap() <= 1000.0 + p - 1.0);
    }

    #[test]
    fn frame_cost_attacks_fast_and_decays_slowly() {
        let mut c = FrameCost::new(16.0);
        assert_eq!(c.ms(), 8.0);
        c.note(14.0);
        assert!(c.ms() > 10.9);
        let hi = c.ms();
        c.note(2.0);
        assert!(c.ms() < hi && c.ms() > hi - 2.0);
    }

    #[test]
    fn precise_wait_returns_when_the_timeout_runs_out() {
        let mut ready = Vec::new();
        let t = std::time::Instant::now();
        wait_precise(&[-1], 2.5, &mut ready);
        assert_eq!(ready, vec![false]);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert!((2.0..30.0).contains(&ms), "{ms}");
    }

    #[test]
    fn property_names_match_without_nul_padding() {
        let mut p = GetProperty::default();
        p.name[..4].copy_from_slice(b"DPMS");
        assert!(p.is_named("DPMS"));
        assert!(!p.is_named("DPM"));
        assert!(!p.is_named("CRTC_ID"));
    }
}
