//! The few alsa-lib C symbols the plug-in needs, declared by hand (no bindgen). Layouts follow
//! `/usr/include/alsa/pcm_ioplug.h` (ioplug protocol 1.0.2) and are pinned by the tests below.
//! libasound is only the host that loads this library; BifrOSt code never links it on its own.

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

pub type SndPcm = c_void;
pub type SndConfig = c_void;
pub type SndPcmHwParams = c_void;
pub type SndPcmSwParams = c_void;
pub type SndOutput = c_void;
pub type SndConfigIterator = *mut c_void;
pub type Sframes = c_long;
pub type Uframes = c_ulong;

pub const SND_PCM_STREAM_PLAYBACK: c_int = 0;
pub const SND_PCM_STREAM_CAPTURE: c_int = 1;

pub const SND_PCM_ACCESS_MMAP_INTERLEAVED: c_uint = 0;
pub const SND_PCM_ACCESS_RW_INTERLEAVED: c_uint = 3;

pub const SND_PCM_FORMAT_S16_LE: c_uint = 2;
pub const SND_PCM_FORMAT_S24_LE: c_uint = 6;
pub const SND_PCM_FORMAT_S32_LE: c_uint = 10;
pub const SND_PCM_FORMAT_FLOAT_LE: c_uint = 14;

pub const SND_PCM_IOPLUG_HW_ACCESS: c_int = 0;
pub const SND_PCM_IOPLUG_HW_FORMAT: c_int = 1;
pub const SND_PCM_IOPLUG_HW_CHANNELS: c_int = 2;
pub const SND_PCM_IOPLUG_HW_RATE: c_int = 3;
pub const SND_PCM_IOPLUG_HW_PERIOD_BYTES: c_int = 4;
pub const SND_PCM_IOPLUG_HW_BUFFER_BYTES: c_int = 5;
pub const SND_PCM_IOPLUG_HW_PERIODS: c_int = 6;

pub const SND_PCM_IOPLUG_VERSION: c_uint = (1 << 16) | 2;
pub const SND_PCM_IOPLUG_FLAG_LISTED: c_uint = 1;
pub const SND_PCM_IOPLUG_FLAG_MONOTONIC: c_uint = 2;

pub const SND_CONFIG_TYPE_STRING: c_int = 4;

#[repr(C)]
pub struct ChannelArea {
    pub addr: *mut c_void,
    /// Offset of the first sample in bits.
    pub first: c_uint,
    /// Distance between two samples of this channel in bits.
    pub step: c_uint,
}

#[repr(C)]
pub struct Ioplug {
    pub version: c_uint,
    pub name: *const c_char,
    pub flags: c_uint,
    pub poll_fd: c_int,
    pub poll_events: c_uint,
    pub mmap_rw: c_uint,
    pub callback: *const IoplugCallback,
    pub private_data: *mut c_void,
    pub pcm: *mut SndPcm,
    pub stream: c_int,
    pub state: c_int,
    pub appl_ptr: Uframes,
    pub hw_ptr: Uframes,
    pub nonblock: c_int,
    pub access: c_uint,
    pub format: c_int,
    pub channels: c_uint,
    pub rate: c_uint,
    pub period_size: Uframes,
    pub buffer_size: Uframes,
}

#[repr(C)]
pub struct Pollfd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

#[repr(C)]
pub struct IoplugCallback {
    pub start: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub stop: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub pointer: Option<unsafe extern "C" fn(*mut Ioplug) -> Sframes>,
    pub transfer: Option<unsafe extern "C" fn(*mut Ioplug, *const ChannelArea, Uframes, Uframes) -> Sframes>,
    pub close: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub hw_params: Option<unsafe extern "C" fn(*mut Ioplug, *mut SndPcmHwParams) -> c_int>,
    pub hw_free: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub sw_params: Option<unsafe extern "C" fn(*mut Ioplug, *mut SndPcmSwParams) -> c_int>,
    pub prepare: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub drain: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub pause: Option<unsafe extern "C" fn(*mut Ioplug, c_int) -> c_int>,
    pub resume: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub poll_descriptors_count: Option<unsafe extern "C" fn(*mut Ioplug) -> c_int>,
    pub poll_descriptors: Option<unsafe extern "C" fn(*mut Ioplug, *mut Pollfd, c_uint) -> c_int>,
    pub poll_revents: Option<unsafe extern "C" fn(*mut Ioplug, *mut Pollfd, c_uint, *mut u16) -> c_int>,
    pub dump: Option<unsafe extern "C" fn(*mut Ioplug, *mut SndOutput)>,
    pub delay: Option<unsafe extern "C" fn(*mut Ioplug, *mut Sframes) -> c_int>,
    pub query_chmaps: Option<unsafe extern "C" fn(*mut Ioplug) -> *mut *mut c_void>,
    pub get_chmap: Option<unsafe extern "C" fn(*mut Ioplug) -> *mut c_void>,
    pub set_chmap: Option<unsafe extern "C" fn(*mut Ioplug, *const c_void) -> c_int>,
}

// SAFETY: the callback table only holds function pointers and is never mutated.
unsafe impl Sync for IoplugCallback {}

// These resolve in the process that loaded us (libasound is already mapped there). Unit tests of this crate
// never call them.
// (Only the test build links libasound, so the test binary resolves them; the shipped cdylib leaves them
// undefined and the host's libasound provides them.)
#[cfg_attr(test, link(name = "asound"))]
extern "C" {
    pub fn snd_pcm_ioplug_create(io: *mut Ioplug, name: *const c_char, stream: c_int, mode: c_int) -> c_int;
    pub fn snd_pcm_ioplug_delete(io: *mut Ioplug) -> c_int;
    pub fn snd_pcm_ioplug_set_param_minmax(io: *mut Ioplug, typ: c_int, min: c_uint, max: c_uint) -> c_int;
    pub fn snd_pcm_ioplug_set_param_list(io: *mut Ioplug, typ: c_int, num: c_uint, list: *const c_uint) -> c_int;
    pub fn snd_pcm_ioplug_params_reset(io: *mut Ioplug);
    pub fn snd_config_get_id(config: *const SndConfig, value: *mut *const c_char) -> c_int;
    pub fn snd_config_get_type(config: *const SndConfig) -> c_int;
    pub fn snd_config_get_string(config: *const SndConfig, value: *mut *const c_char) -> c_int;
    pub fn snd_config_iterator_first(node: *const SndConfig) -> SndConfigIterator;
    pub fn snd_config_iterator_next(it: SndConfigIterator) -> SndConfigIterator;
    pub fn snd_config_iterator_end(node: *const SndConfig) -> SndConfigIterator;
    pub fn snd_config_iterator_entry(it: SndConfigIterator) -> *mut SndConfig;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    // x86_64 LP64 layout worked out by hand from pcm_ioplug.h.
    #[test]
    fn ioplug_layout() {
        assert_eq!(offset_of!(Ioplug, version), 0);
        assert_eq!(offset_of!(Ioplug, name), 8);
        assert_eq!(offset_of!(Ioplug, flags), 16);
        assert_eq!(offset_of!(Ioplug, poll_fd), 20);
        assert_eq!(offset_of!(Ioplug, poll_events), 24);
        assert_eq!(offset_of!(Ioplug, mmap_rw), 28);
        assert_eq!(offset_of!(Ioplug, callback), 32);
        assert_eq!(offset_of!(Ioplug, private_data), 40);
        assert_eq!(offset_of!(Ioplug, pcm), 48);
        assert_eq!(offset_of!(Ioplug, stream), 56);
        assert_eq!(offset_of!(Ioplug, state), 60);
        assert_eq!(offset_of!(Ioplug, appl_ptr), 64);
        assert_eq!(offset_of!(Ioplug, hw_ptr), 72);
        assert_eq!(offset_of!(Ioplug, nonblock), 80);
        assert_eq!(offset_of!(Ioplug, access), 84);
        assert_eq!(offset_of!(Ioplug, format), 88);
        assert_eq!(offset_of!(Ioplug, channels), 92);
        assert_eq!(offset_of!(Ioplug, rate), 96);
        assert_eq!(offset_of!(Ioplug, period_size), 104);
        assert_eq!(offset_of!(Ioplug, buffer_size), 112);
        assert_eq!(size_of::<Ioplug>(), 120);
    }

    #[test]
    fn callback_and_area_layout() {
        assert_eq!(size_of::<IoplugCallback>(), 20 * 8);
        assert_eq!(offset_of!(IoplugCallback, delay), 16 * 8);
        assert_eq!(size_of::<ChannelArea>(), 16);
        assert_eq!(offset_of!(ChannelArea, step), 12);
        assert_eq!(size_of::<Pollfd>(), 8);
    }

    #[test]
    fn version_is_1_0_2() {
        assert_eq!(SND_PCM_IOPLUG_VERSION, 0x10002);
    }
}
