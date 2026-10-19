//! Loading third-party audio units from shared libraries (the in-process plug-in host).
//!
//! A plug-in is a `.so` that exports one symbol, `bifrost_unit_entry`, returning a pointer to a `UnitVTable` that
//! stays valid while the library is loaded. The table is a plain C ABI (all `#[repr(C)]` types below, version
//! `PLUGIN_ABI_VERSION`), so a plug-in can be written in C, C++, Rust or anything that exports C functions:
//!
//! ```c
//! const UnitVTable *bifrost_unit_entry(void);  /* abi_version = 1 */
//! ```
//!
//! The host owns the parameters. It keeps the `ParameterTree` (ramps, clamping, read-only flags, presets) and hands
//! the plug-in two arrays per block: each parameter's value at the first frame (`param_start`) and the value one
//! frame after the last (`param_end`). A plug-in interpolates linearly between them, so ramps started by the
//! control side are heard at the block rate and sample-accurately when the plug-in interpolates per frame.
//!
//! `PluginUnit` is an `AudioUnit`, so it sits in a `UnitHost`, in an `Engine` as an effect or instrument node, or in
//! the out-of-process `Worker`, exactly like a built-in unit. `render` makes no allocation and takes no lock: the
//! channel pointers go on the stack and the MIDI and parameter arrays are allocated in `initialize`. Output that is
//! not finite is replaced by silence, so one bad plug-in cannot poison the graph.
//!
//! This module is the only unsafe code of the crate: `dlopen`/`dlsym` and the calls through the table. Each call is
//! made on a pointer the table's owner promised, with arguments that live across the call.

#![allow(unsafe_code)]

use crate::param::{ParamSpec, ParamUnit, ParameterTree};
use crate::registry::{four_cc, ComponentDescription, ComponentRegistry};
use crate::unit::{check_config, AudioUnit, Block, RenderConfig, UnitError, UnitInfo, UnitKind};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The ABI version a table must carry.
pub const PLUGIN_ABI_VERSION: u32 = 1;
/// The symbol a plug-in library exports.
pub const PLUGIN_ENTRY: &str = "bifrost_unit_entry";
/// The most channels a plug-in sees on one side.
pub const MAX_CHANNELS: usize = 32;
/// The most parameters a plug-in may declare.
pub const MAX_PARAMS: u32 = 4096;
/// The system directory scanned for plug-in libraries.
pub const SYSTEM_DIR: &str = "/usr/lib/bifrost/units";

/// Unit kinds in `CInfo::kind`.
pub const KIND_EFFECT: u32 = 0;
pub const KIND_INSTRUMENT: u32 = 1;
pub const KIND_GENERATOR: u32 = 2;

/// Flags in `CParam::flags`.
pub const PARAM_READ_ONLY: u32 = 1;

/// Static facts a plug-in reports. Text is UTF-8, NUL-terminated inside its field.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CInfo {
    pub kind: u32,
    pub accepts_midi: u32,
    pub max_channels: u32,
    pub param_count: u32,
    pub name: [u8; 64],
    /// The four-character codes of the component description (for example `aufx`, `gain`, `acme`).
    pub kind_code: [u8; 4],
    pub sub_kind: [u8; 4],
    pub manufacturer: [u8; 4],
}

impl CInfo {
    pub const EMPTY: CInfo =
        CInfo { kind: 0, accepts_midi: 0, max_channels: 0, param_count: 0, name: [0; 64], kind_code: [0; 4], sub_kind: [0; 4], manufacturer: [0; 4] };
}

/// One parameter. `id` must equal the index asked for. `unit` is 0 generic, 1 decibels, 2 hertz, 3 percent,
/// 4 seconds, 5 ratio, 6 boolean, 7 indexed; `value_strings` is a `|`-separated list for an indexed parameter.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CParam {
    pub id: u32,
    pub unit: u32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub flags: u32,
    /// The ramp length (frames) a plain `set` uses; 0 jumps.
    pub ramp_frames: u32,
    pub name: [u8; 64],
    /// A slash-separated group path; empty for the root.
    pub group: [u8; 64],
    pub value_strings: [u8; 192],
}

impl CParam {
    pub const EMPTY: CParam = CParam {
        id: 0,
        unit: 0,
        min: 0.0,
        max: 0.0,
        default: 0.0,
        flags: 0,
        ramp_frames: 0,
        name: [0; 64],
        group: [0; 64],
        value_strings: [0; 192],
    };
}

/// What `initialize` tells the plug-in.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CRenderConfig {
    pub sample_rate: f64,
    pub max_frames: u32,
    pub input_channels: u32,
    pub output_channels: u32,
}

/// A channel message at a frame offset inside the block.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CMidi {
    pub offset: u32,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
    pub reserved: u8,
}

/// One render call. Planar f32 channels, `frames` samples each; `midi` is sorted by offset.
#[repr(C)]
pub struct CBlock {
    pub frames: u32,
    pub input_count: u32,
    pub output_count: u32,
    pub midi_count: u32,
    /// Sample time of the first frame.
    pub sample_time: f64,
    pub inputs: *const *const f32,
    pub outputs: *const *mut f32,
    pub midi: *const CMidi,
    pub param_count: u32,
    pub reserved: u32,
    /// Each parameter's value at the first frame, and one frame after the last.
    pub param_start: *const f32,
    pub param_end: *const f32,
}

/// The table a plug-in exports. Every function but `describe` and `param` gets the instance `create` returned.
/// `render` runs on the real-time thread: it must not allocate, lock or make system calls.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UnitVTable {
    pub abi_version: u32,
    pub describe: unsafe extern "C" fn(*mut CInfo),
    /// Fills parameter `index`; returns 0 on success.
    pub param: unsafe extern "C" fn(u32, *mut CParam) -> i32,
    /// A new instance, or null.
    pub create: unsafe extern "C" fn() -> *mut c_void,
    pub destroy: unsafe extern "C" fn(*mut c_void),
    /// Allocates for the configuration; returns 0 on success.
    pub initialize: unsafe extern "C" fn(*mut c_void, *const CRenderConfig) -> i32,
    pub uninitialize: unsafe extern "C" fn(*mut c_void),
    /// Clears delay lines, filter memory and voices; parameters keep their values.
    pub reset: unsafe extern "C" fn(*mut c_void),
    pub render: unsafe extern "C" fn(*mut c_void, *const CBlock),
    pub latency: unsafe extern "C" fn(*mut c_void) -> u32,
    pub tail: unsafe extern "C" fn(*mut c_void) -> u32,
}

/// Why a plug-in could not be loaded or instantiated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    /// `dlopen` failed (missing file, bad ELF, unresolved symbol); the loader's message.
    Load(String),
    /// The library has no `bifrost_unit_entry`, or it returned null.
    NoEntry,
    /// The table's version is not `PLUGIN_ABI_VERSION`.
    BadAbi(u32),
    /// `describe` reported something out of range (named).
    BadInfo(&'static str),
    /// Parameter `n` is malformed (wrong id, bad range, call failed).
    BadParam(u32),
    /// `create` returned null.
    Instance,
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::Load(m) => write!(f, "cannot load the plug-in: {m}"),
            PluginError::NoEntry => write!(f, "the library exports no {PLUGIN_ENTRY}"),
            PluginError::BadAbi(v) => write!(f, "plug-in ABI version {v} is not supported (need {PLUGIN_ABI_VERSION})"),
            PluginError::BadInfo(what) => write!(f, "the plug-in reports an invalid {what}"),
            PluginError::BadParam(n) => write!(f, "plug-in parameter {n} is malformed"),
            PluginError::Instance => write!(f, "the plug-in could not make an instance"),
        }
    }
}

impl std::error::Error for PluginError {}

// ------------------------------------------------------------------------------------------------ dynamic loading

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *mut c_char;
}

/// An open library; closed when the last instance and the loader let go. A null handle is a table supplied in
/// process (`PluginLibrary::from_vtable`), which has nothing to close.
struct Library {
    handle: *mut c_void,
}

// SAFETY: the handle is an opaque token for `dlsym`/`dlclose`, which the C library allows from any thread.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Drop for Library {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `handle` came from a successful `dlopen` and is closed exactly once, here.
            unsafe { dlclose(self.handle) };
        }
    }
}

fn last_error() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated string valid until the next dl call on this thread.
    unsafe {
        let e = dlerror();
        if e.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|c| *c == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn code(field: [u8; 4]) -> Result<u32, PluginError> {
    let s = std::str::from_utf8(&field).map_err(|_| PluginError::BadInfo("component code"))?;
    four_cc(s).map_err(|_| PluginError::BadInfo("component code"))
}

fn param_unit(v: u32) -> ParamUnit {
    match v {
        1 => ParamUnit::Decibels,
        2 => ParamUnit::Hertz,
        3 => ParamUnit::Percent,
        4 => ParamUnit::Seconds,
        5 => ParamUnit::Ratio,
        6 => ParamUnit::Boolean,
        7 => ParamUnit::Indexed,
        _ => ParamUnit::Generic,
    }
}

// ------------------------------------------------------------------------------------------------------ the library

/// What a loaded plug-in says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub name: String,
    pub kind: UnitKind,
    pub accepts_midi: bool,
    pub max_channels: u32,
    pub description: ComponentDescription,
    pub param_count: usize,
}

struct Inner {
    vt: UnitVTable,
    info: PluginInfo,
    specs: Vec<ParamSpec>,
    /// Dropped last, so the table's functions stay mapped while an instance is destroyed.
    _lib: Library,
}

/// A loaded plug-in library: describes its component, makes instances. Cheap to clone; the library stays loaded
/// until the last clone and the last instance are gone.
#[derive(Clone)]
pub struct PluginLibrary {
    inner: Arc<Inner>,
}

impl PluginLibrary {
    /// Loads a shared library and reads its table.
    pub fn open(path: &Path) -> Result<PluginLibrary, PluginError> {
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| PluginError::Load("the path contains a NUL byte".into()))?;
        // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
        let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            return Err(PluginError::Load(last_error()));
        }
        // From here the library closes itself on every error path.
        let lib = Library { handle };
        let sym = CString::new(PLUGIN_ENTRY).expect("a constant without NUL");
        // SAFETY: `handle` is open and `sym` is a valid NUL-terminated string.
        let entry = unsafe { dlsym(handle, sym.as_ptr()) };
        if entry.is_null() {
            return Err(PluginError::NoEntry);
        }
        // SAFETY: the plug-in contract is that this symbol is `const UnitVTable *bifrost_unit_entry(void)`.
        let entry: unsafe extern "C" fn() -> *const UnitVTable = unsafe { std::mem::transmute(entry) };
        // SAFETY: calling the entry function of a loaded plug-in, as the contract allows.
        let table = unsafe { entry() };
        if table.is_null() {
            return Err(PluginError::NoEntry);
        }
        // SAFETY: the table is valid while the library is loaded, and `UnitVTable` is `Copy`: it is copied out now.
        let vt = unsafe { *table };
        PluginLibrary::build(vt, lib)
    }

    /// Uses a table that lives in this process (an application's own plug-in, or a test).
    pub fn from_vtable(vt: UnitVTable) -> Result<PluginLibrary, PluginError> {
        PluginLibrary::build(vt, Library { handle: std::ptr::null_mut() })
    }

    fn build(vt: UnitVTable, lib: Library) -> Result<PluginLibrary, PluginError> {
        if vt.abi_version != PLUGIN_ABI_VERSION {
            return Err(PluginError::BadAbi(vt.abi_version));
        }
        let mut ci = CInfo::EMPTY;
        // SAFETY: `describe` gets a valid pointer to a `CInfo`.
        unsafe { (vt.describe)(&mut ci) };
        let kind = match ci.kind {
            KIND_EFFECT => UnitKind::Effect,
            KIND_INSTRUMENT => UnitKind::Instrument,
            KIND_GENERATOR => UnitKind::Generator,
            _ => return Err(PluginError::BadInfo("kind")),
        };
        if ci.max_channels == 0 || ci.max_channels as usize > MAX_CHANNELS {
            return Err(PluginError::BadInfo("channel count"));
        }
        if ci.param_count > MAX_PARAMS {
            return Err(PluginError::BadInfo("parameter count"));
        }
        let name = text(&ci.name);
        if name.is_empty() {
            return Err(PluginError::BadInfo("name"));
        }
        let description = ComponentDescription { kind: code(ci.kind_code)?, sub_kind: code(ci.sub_kind)?, manufacturer: code(ci.manufacturer)? };
        let mut specs = Vec::with_capacity(ci.param_count as usize);
        for i in 0..ci.param_count {
            let mut p = CParam::EMPTY;
            // SAFETY: `param` gets a valid pointer to a `CParam`.
            let r = unsafe { (vt.param)(i, &mut p) };
            if r != 0 || p.id != i {
                return Err(PluginError::BadParam(i));
            }
            let mut spec = ParamSpec::new(i, &text(&p.name), param_unit(p.unit), p.min, p.max, p.default)
                .map_err(|_| PluginError::BadParam(i))?
                .in_group(&text(&p.group))
                .with_ramp(p.ramp_frames);
            if p.flags & PARAM_READ_ONLY != 0 {
                spec = spec.read_only();
            }
            let strings = text(&p.value_strings);
            if !strings.is_empty() {
                let parts: Vec<&str> = strings.split('|').collect();
                spec = spec.with_value_strings(&parts);
            }
            specs.push(spec);
        }
        let info = PluginInfo {
            name,
            kind,
            accepts_midi: ci.accepts_midi != 0,
            max_channels: ci.max_channels,
            description,
            param_count: specs.len(),
        };
        Ok(PluginLibrary { inner: Arc::new(Inner { vt, info, specs, _lib: lib }) })
    }

    pub fn info(&self) -> &PluginInfo {
        &self.inner.info
    }

    /// The parameter definitions, as an instance starts with them.
    pub fn param_specs(&self) -> &[ParamSpec] {
        &self.inner.specs
    }

    /// Makes an instance (not initialized yet).
    pub fn instantiate(&self) -> Result<PluginUnit, PluginError> {
        // SAFETY: `create` takes no arguments; a non-null result is an instance the other functions accept.
        let instance = unsafe { (self.inner.vt.create)() };
        if instance.is_null() {
            return Err(PluginError::Instance);
        }
        let n = self.inner.specs.len();
        let info = &self.inner.info;
        let tree = match ParameterTree::new(self.inner.specs.clone()) {
            Ok(t) => t,
            Err(_) => {
                // SAFETY: `instance` was just made by this table and is destroyed once.
                unsafe { (self.inner.vt.destroy)(instance) };
                return Err(PluginError::BadParam(0));
            }
        };
        Ok(PluginUnit {
            lib: self.inner.clone(),
            instance,
            info: UnitInfo { name: info.name.clone(), kind: info.kind, accepts_midi: info.accepts_midi, max_channels: info.max_channels },
            tree,
            ready: false,
            max_frames: 0,
            midi: Vec::new(),
            start: vec![0.0; n],
            end: vec![0.0; n],
        })
    }

    /// Adds the component to a registry, so hosts find it by its description and instantiate it by index.
    pub fn register_into(&self, registry: &mut ComponentRegistry) {
        let lib = self.clone();
        let info = &self.inner.info;
        registry.register_boxed(
            info.description,
            &info.name,
            Box::new(move || {
                lib.instantiate().map(|u| Box::new(u) as Box<dyn AudioUnit>).map_err(|_| UnitError::NoComponent)
            }),
        );
    }
}

// --------------------------------------------------------------------------------------------------------- the unit

/// One instance of a loaded plug-in, as an `AudioUnit`.
pub struct PluginUnit {
    lib: Arc<Inner>,
    instance: *mut c_void,
    info: UnitInfo,
    tree: ParameterTree,
    ready: bool,
    max_frames: usize,
    midi: Vec<CMidi>,
    start: Vec<f32>,
    end: Vec<f32>,
}

// SAFETY: the instance pointer is owned by this value alone and only used through `&mut self` (or `&self` for the
// two query calls); the plug-in contract allows the render thread to be a different thread from the creator.
unsafe impl Send for PluginUnit {}

impl Drop for PluginUnit {
    fn drop(&mut self) {
        // SAFETY: `instance` came from this table's `create` and is destroyed exactly once, while the library is
        // still held by `self.lib`.
        unsafe {
            if self.ready {
                (self.lib.vt.uninitialize)(self.instance);
            }
            (self.lib.vt.destroy)(self.instance);
        }
    }
}

impl PluginUnit {
    fn silence(block: &mut Block<'_, '_>) {
        let n = block.frames;
        for o in block.outputs.iter_mut() {
            let n = n.min(o.len());
            o[..n].iter_mut().for_each(|s| *s = 0.0);
        }
    }
}

impl AudioUnit for PluginUnit {
    fn info(&self) -> &UnitInfo {
        &self.info
    }

    fn tree(&self) -> &ParameterTree {
        &self.tree
    }

    fn tree_mut(&mut self) -> &mut ParameterTree {
        &mut self.tree
    }

    fn initialize(&mut self, cfg: &RenderConfig) -> Result<(), UnitError> {
        check_config(&self.info, cfg)?;
        if self.ready {
            return Err(UnitError::AlreadyInitialized);
        }
        let c = CRenderConfig {
            sample_rate: cfg.sample_rate,
            max_frames: cfg.max_frames,
            input_channels: cfg.input_channels,
            output_channels: cfg.output_channels,
        };
        // SAFETY: `instance` is live and `c` outlives the call.
        let r = unsafe { (self.lib.vt.initialize)(self.instance, &c) };
        if r != 0 {
            return Err(UnitError::BadFormat);
        }
        self.max_frames = cfg.max_frames as usize;
        self.midi = vec![CMidi::default(); self.max_frames.max(256)];
        self.ready = true;
        Ok(())
    }

    fn uninitialize(&mut self) {
        if self.ready {
            // SAFETY: `instance` is live.
            unsafe { (self.lib.vt.uninitialize)(self.instance) };
            self.ready = false;
        }
    }

    fn reset(&mut self) {
        // SAFETY: `instance` is live.
        unsafe { (self.lib.vt.reset)(self.instance) };
    }

    fn latency_frames(&self) -> u32 {
        // SAFETY: `instance` is live; the query does not change it as far as the host is concerned.
        unsafe { (self.lib.vt.latency)(self.instance) }
    }

    fn tail_frames(&self) -> u32 {
        // SAFETY: as for `latency_frames`.
        unsafe { (self.lib.vt.tail)(self.instance) }
    }

    fn render(&mut self, block: &mut Block<'_, '_>) {
        self.tree.sync();
        let n = block.frames;
        if !self.ready || n > self.max_frames || n == 0 {
            PluginUnit::silence(block);
            return;
        }
        for id in 0..self.start.len() {
            let id32 = id as u32;
            self.start[id] = self.tree.advance(id32, n);
            self.end[id] = self.tree.value(id32);
        }
        let m = block.midi.len().min(self.midi.len());
        for (d, e) in self.midi[..m].iter_mut().zip(block.midi.iter()) {
            *d = CMidi { offset: e.offset, status: e.status, data1: e.data1, data2: e.data2, reserved: 0 };
        }
        let mut ins: [*const f32; MAX_CHANNELS] = [std::ptr::null(); MAX_CHANNELS];
        let mut outs: [*mut f32; MAX_CHANNELS] = [std::ptr::null_mut(); MAX_CHANNELS];
        let ni = block.inputs.len().min(MAX_CHANNELS);
        let no = block.outputs.len().min(MAX_CHANNELS);
        for (d, s) in ins.iter_mut().zip(block.inputs.iter()) {
            *d = s.as_ptr();
        }
        for (d, s) in outs.iter_mut().zip(block.outputs.iter_mut()) {
            *d = s.as_mut_ptr();
        }
        let cb = CBlock {
            frames: n as u32,
            input_count: ni as u32,
            output_count: no as u32,
            midi_count: m as u32,
            sample_time: block.sample_time,
            inputs: ins.as_ptr(),
            outputs: outs.as_ptr(),
            midi: self.midi.as_ptr(),
            param_count: self.start.len() as u32,
            reserved: 0,
            param_start: self.start.as_ptr(),
            param_end: self.end.as_ptr(),
        };
        // SAFETY: `instance` is live and initialized; every pointer in `cb` points into buffers that outlive the call
        // (the host checked that each channel slice holds at least `frames` samples).
        unsafe { (self.lib.vt.render)(self.instance, &cb) };
        for o in block.outputs.iter_mut() {
            for s in o[..n].iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------- scanning

/// The directories scanned by default: the system one and the user's.
pub fn default_directories() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(SYSTEM_DIR)];
    if let Some(home) = std::env::var_os("HOME") {
        v.push(Path::new(&home).join(".local/share/bifrost/units"));
    }
    v
}

/// The outcome for one library found by `scan_directory`.
#[derive(Debug)]
pub struct ScanReport {
    pub path: PathBuf,
    /// The plug-in's name when it loaded and was registered, or why it did not.
    pub result: Result<String, PluginError>,
}

/// Loads every `*.so` in `dir` (in name order) and registers the ones that load. A bad library does not stop the
/// scan: it gets a report with its error. A missing directory has no reports.
pub fn scan_directory(dir: &Path, registry: &mut ComponentRegistry) -> Vec<ScanReport> {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "so")).collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let result = PluginLibrary::open(&path).map(|lib| {
                lib.register_into(registry);
                lib.info().name.clone()
            });
            ScanReport { path, result }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::UnitHost;
    use crate::midi::MidiEvent;
    use crate::registry::ComponentRegistry;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn put(dst: &mut [u8], s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(dst.len() - 1);
        dst[..n].copy_from_slice(&b[..n]);
        dst[n] = 0;
    }

    // ---- a gain effect with one ramped parameter, written the way a plug-in author would

    unsafe extern "C" fn gain_describe(out: *mut CInfo) {
        let o = &mut *out;
        put(&mut o.name, "Test Gain");
        o.kind = KIND_EFFECT;
        o.accepts_midi = 0;
        o.max_channels = 2;
        o.param_count = 1;
        o.kind_code = *b"aufx";
        o.sub_kind = *b"tgan";
        o.manufacturer = *b"test";
    }

    unsafe extern "C" fn gain_param(i: u32, out: *mut CParam) -> i32 {
        if i != 0 {
            return 1;
        }
        let p = &mut *out;
        p.id = 0;
        p.unit = 5;
        p.min = 0.0;
        p.max = 4.0;
        p.default = 1.0;
        p.flags = 0;
        p.ramp_frames = 0;
        put(&mut p.name, "Gain");
        put(&mut p.group, "Main");
        0
    }

    unsafe extern "C" fn state_create() -> *mut c_void {
        Box::into_raw(Box::new(0u32)) as *mut c_void
    }

    unsafe extern "C" fn state_destroy(p: *mut c_void) {
        drop(Box::from_raw(p as *mut u32));
    }

    /// Refuses sample rates above 100 kHz, to show a plug-in's own refusal.
    unsafe extern "C" fn init_ok(_: *mut c_void, cfg: *const CRenderConfig) -> i32 {
        if (*cfg).sample_rate > 100_000.0 {
            1
        } else {
            0
        }
    }

    unsafe extern "C" fn nothing(_: *mut c_void) {}

    unsafe extern "C" fn zero(_: *mut c_void) -> u32 {
        0
    }

    unsafe extern "C" fn gain_render(_: *mut c_void, b: *const CBlock) {
        let b = &*b;
        let n = b.frames as usize;
        let (s, e) = (*b.param_start, *b.param_end);
        for c in 0..b.input_count.min(b.output_count) as usize {
            let inp = std::slice::from_raw_parts(*b.inputs.add(c), n);
            let out = std::slice::from_raw_parts_mut(*b.outputs.add(c), n);
            for k in 0..n {
                out[k] = inp[k] * (s + (e - s) * k as f32 / n as f32);
            }
        }
    }

    static GAIN: UnitVTable = UnitVTable {
        abi_version: PLUGIN_ABI_VERSION,
        describe: gain_describe,
        param: gain_param,
        create: state_create,
        destroy: state_destroy,
        initialize: init_ok,
        uninitialize: nothing,
        reset: nothing,
        render: gain_render,
        latency: zero,
        tail: zero,
    };

    fn cfg(inp: u32, out: u32, frames: u32) -> RenderConfig {
        RenderConfig { sample_rate: 48_000.0, max_frames: frames, input_channels: inp, output_channels: out }
    }

    fn host_for(lib: &PluginLibrary, c: RenderConfig) -> UnitHost {
        let mut h = UnitHost::new(Box::new(lib.instantiate().unwrap()));
        h.initialize(c).unwrap();
        h
    }

    #[test]
    fn the_table_describes_the_component_and_its_parameters() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let info = lib.info();
        assert_eq!((info.name.as_str(), info.kind, info.accepts_midi, info.max_channels), ("Test Gain", UnitKind::Effect, false, 2));
        assert_eq!(info.description, ComponentDescription::new("aufx", "tgan", "test").unwrap());
        let specs = lib.param_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!((specs[0].name.as_str(), specs[0].group.as_str(), specs[0].unit), ("Gain", "Main", ParamUnit::Ratio));
        assert_eq!((specs[0].min, specs[0].max, specs[0].default, specs[0].read_only), (0.0, 4.0, 1.0, false));
    }

    #[test]
    fn an_unramped_plugin_passes_audio_at_the_default_gain() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut h = host_for(&lib, cfg(1, 1, 8));
        let i = [0.5f32, -0.25, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5];
        let mut o = [9.0f32; 8];
        h.render(&[&i], &mut [&mut o], 8, 0.0, &[]).unwrap();
        assert_eq!(o, i);
    }

    #[test]
    fn a_parameter_ramp_over_one_block_is_exact_per_frame() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut h = host_for(&lib, cfg(1, 1, 8));
        h.unit().params().set_ramped(0, 0.0, 8).unwrap();
        let i = [1.0f32; 8];
        let mut o = [0.0f32; 8];
        h.render(&[&i], &mut [&mut o], 8, 0.0, &[]).unwrap();
        let want: Vec<f32> = (0..8).map(|k| 1.0 - k as f32 / 8.0).collect();
        assert_eq!(o.to_vec(), want);
    }

    #[test]
    fn a_ramp_continues_across_blocks_without_a_step() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut h = host_for(&lib, cfg(1, 1, 4));
        h.unit().params().set_ramped(0, 0.0, 8).unwrap();
        let i = [1.0f32; 4];
        let (mut a, mut b) = ([0.0f32; 4], [0.0f32; 4]);
        h.render(&[&i], &mut [&mut a], 4, 0.0, &[]).unwrap();
        h.render(&[&i], &mut [&mut b], 4, 4.0, &[]).unwrap();
        let got: Vec<f32> = a.iter().chain(b.iter()).copied().collect();
        let want: Vec<f32> = (0..8).map(|k| 1.0 - k as f32 / 8.0).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn presets_apply_to_a_plugin_through_its_parameter_handle() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut h = host_for(&lib, cfg(1, 1, 4));
        let handle = h.unit().params();
        let mut store = crate::preset::PresetStore::new(vec![crate::preset::Preset { number: 0, name: "Loud".into(), values: vec![(0, 2.0)] }]).unwrap();
        store.select(0, &handle).unwrap();
        let i = [0.25f32; 4];
        let mut o = [0.0f32; 4];
        h.render(&[&i], &mut [&mut o], 4, 0.0, &[]).unwrap();
        assert_eq!(o, [0.5; 4]);
        // an out-of-range value is clamped by the host, not passed through
        assert_eq!(handle.set(0, 100.0).unwrap(), 4.0);
    }

    // ---- an instrument: sample-accurate MIDI

    unsafe extern "C" fn voice_describe(out: *mut CInfo) {
        let o = &mut *out;
        put(&mut o.name, "Test Voice");
        o.kind = KIND_INSTRUMENT;
        o.accepts_midi = 1;
        o.max_channels = 1;
        o.param_count = 0;
        o.kind_code = *b"aumu";
        o.sub_kind = *b"tvox";
        o.manufacturer = *b"test";
    }

    unsafe extern "C" fn voice_param(_: u32, _: *mut CParam) -> i32 {
        1
    }

    unsafe extern "C" fn voice_render(inst: *mut c_void, b: *const CBlock) {
        let level = &mut *(inst as *mut u32);
        let b = &*b;
        let n = b.frames as usize;
        let events = std::slice::from_raw_parts(b.midi, b.midi_count as usize);
        let out = std::slice::from_raw_parts_mut(*b.outputs, n);
        let mut next = 0;
        for k in 0..n {
            while next < events.len() && events[next].offset as usize == k {
                let e = events[next];
                *level = match e.status & 0xF0 {
                    0x90 if e.data2 > 0 => e.data1 as u32,
                    _ => 0,
                };
                next += 1;
            }
            out[k] = *level as f32 / 127.0;
        }
    }

    unsafe extern "C" fn voice_reset(inst: *mut c_void) {
        *(inst as *mut u32) = 0;
    }

    static VOICE: UnitVTable = UnitVTable {
        abi_version: PLUGIN_ABI_VERSION,
        describe: voice_describe,
        param: voice_param,
        create: state_create,
        destroy: state_destroy,
        initialize: init_ok,
        uninitialize: nothing,
        reset: voice_reset,
        render: voice_render,
        latency: zero,
        tail: zero,
    };

    #[test]
    fn midi_events_reach_the_plugin_at_their_frame_offsets() {
        let lib = PluginLibrary::from_vtable(VOICE).unwrap();
        assert_eq!(lib.info().kind, UnitKind::Instrument);
        let mut h = host_for(&lib, cfg(0, 1, 8));
        let ev = [MidiEvent::note_on(3, 0, 127, 100), MidiEvent::note_off(6, 0, 127)];
        let mut o = [9.0f32; 8];
        h.render(&[], &mut [&mut o], 8, 0.0, &ev).unwrap();
        assert_eq!(o, [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_jump_in_sample_time_resets_the_plugin() {
        let lib = PluginLibrary::from_vtable(VOICE).unwrap();
        let mut h = host_for(&lib, cfg(0, 1, 4));
        let mut o = [0.0f32; 4];
        h.render(&[], &mut [&mut o], 4, 0.0, &[MidiEvent::note_on(0, 0, 127, 100)]).unwrap();
        assert_eq!(o, [1.0; 4]);
        // contiguous: the note keeps sounding
        h.render(&[], &mut [&mut o], 4, 4.0, &[]).unwrap();
        assert_eq!(o, [1.0; 4]);
        // a seek: the host calls reset, and the voice is silent
        h.render(&[], &mut [&mut o], 4, 1000.0, &[]).unwrap();
        assert_eq!(o, [0.0; 4]);
    }

    // ---- lifecycle, bad output and bad tables

    static CREATED: AtomicU32 = AtomicU32::new(0);
    static DESTROYED: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn counted_create() -> *mut c_void {
        CREATED.fetch_add(1, Ordering::SeqCst);
        state_create()
    }

    unsafe extern "C" fn counted_destroy(p: *mut c_void) {
        DESTROYED.fetch_add(1, Ordering::SeqCst);
        state_destroy(p)
    }

    unsafe extern "C" fn nan_render(_: *mut c_void, b: *const CBlock) {
        let b = &*b;
        let out = std::slice::from_raw_parts_mut(*b.outputs, b.frames as usize);
        for (k, s) in out.iter_mut().enumerate() {
            *s = match k % 3 {
                0 => f32::NAN,
                1 => f32::INFINITY,
                _ => 0.5,
            };
        }
    }

    unsafe extern "C" fn null_create() -> *mut c_void {
        std::ptr::null_mut()
    }

    #[test]
    fn instances_are_created_and_destroyed_once_each() {
        let table = UnitVTable { create: counted_create, destroy: counted_destroy, ..GAIN };
        let lib = PluginLibrary::from_vtable(table).unwrap();
        let a = lib.instantiate().unwrap();
        let b = lib.instantiate().unwrap();
        assert_eq!(CREATED.load(Ordering::SeqCst), 2);
        drop(a);
        assert_eq!(DESTROYED.load(Ordering::SeqCst), 1);
        // an initialized instance is also uninitialized on drop, and destroyed once
        let mut b = b;
        b.initialize(&cfg(1, 1, 8)).unwrap();
        drop(b);
        assert_eq!(DESTROYED.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn output_that_is_not_finite_becomes_silence() {
        let table = UnitVTable { render: nan_render, ..GAIN };
        let lib = PluginLibrary::from_vtable(table).unwrap();
        let mut h = host_for(&lib, cfg(1, 1, 6));
        let i = [0.0f32; 6];
        let mut o = [1.0f32; 6];
        h.render(&[&i], &mut [&mut o], 6, 0.0, &[]).unwrap();
        assert_eq!(o, [0.0, 0.0, 0.5, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn a_failing_create_is_an_error_not_a_crash() {
        let table = UnitVTable { create: null_create, ..GAIN };
        let lib = PluginLibrary::from_vtable(table).unwrap();
        assert!(matches!(lib.instantiate(), Err(PluginError::Instance)));
        let mut r = ComponentRegistry::new();
        lib.register_into(&mut r);
        assert!(r.instantiate(0).is_err());
    }

    #[test]
    fn the_plugin_can_refuse_a_configuration() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut u = lib.instantiate().unwrap();
        // channel counts beyond the unit's own maximum are refused by the host before the plug-in is asked
        assert_eq!(u.initialize(&cfg(3, 3, 8)), Err(UnitError::BadFormat));
        assert_eq!(u.initialize(&cfg(1, 2, 8)), Err(UnitError::BadFormat));
        // a configuration the host accepts but the plug-in refuses leaves the unit uninitialized
        let fast = RenderConfig { sample_rate: 192_000.0, max_frames: 8, input_channels: 2, output_channels: 2 };
        assert_eq!(u.initialize(&fast), Err(UnitError::BadFormat));
        assert!(u.initialize(&cfg(2, 2, 8)).is_ok());
        assert_eq!(u.initialize(&cfg(2, 2, 8)), Err(UnitError::AlreadyInitialized));
    }

    #[test]
    fn a_block_longer_than_the_configured_maximum_is_silent_through_the_unit_itself() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut u = lib.instantiate().unwrap();
        u.initialize(&cfg(1, 1, 4)).unwrap();
        let i = [1.0f32; 8];
        let mut o = [5.0f32; 8];
        u.render(&mut Block { frames: 8, sample_time: 0.0, inputs: &[&i], outputs: &mut [&mut o], midi: &[] });
        assert_eq!(o, [0.0; 8]);
    }

    #[test]
    fn malformed_tables_are_refused_with_a_reason() {
        let wrong_version = UnitVTable { abi_version: 7, ..GAIN };
        assert!(matches!(PluginLibrary::from_vtable(wrong_version), Err(PluginError::BadAbi(7))));

        unsafe extern "C" fn bad_kind(out: *mut CInfo) {
            gain_describe(out);
            (*out).kind = 9;
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { describe: bad_kind, ..GAIN }), Err(PluginError::BadInfo("kind"))));

        unsafe extern "C" fn bad_code(out: *mut CInfo) {
            gain_describe(out);
            (*out).sub_kind = [b'a', b'b', 0, b'c'];
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { describe: bad_code, ..GAIN }), Err(PluginError::BadInfo("component code"))));

        unsafe extern "C" fn too_wide(out: *mut CInfo) {
            gain_describe(out);
            (*out).max_channels = 64;
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { describe: too_wide, ..GAIN }), Err(PluginError::BadInfo("channel count"))));

        unsafe extern "C" fn wrong_id(i: u32, out: *mut CParam) -> i32 {
            let r = gain_param(i, out);
            (*out).id = 5;
            r
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { param: wrong_id, ..GAIN }), Err(PluginError::BadParam(0))));

        unsafe extern "C" fn bad_range(i: u32, out: *mut CParam) -> i32 {
            let r = gain_param(i, out);
            (*out).min = 5.0;
            r
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { param: bad_range, ..GAIN }), Err(PluginError::BadParam(0))));

        unsafe extern "C" fn nameless(out: *mut CInfo) {
            gain_describe(out);
            (*out).name = [0; 64];
        }
        assert!(matches!(PluginLibrary::from_vtable(UnitVTable { describe: nameless, ..GAIN }), Err(PluginError::BadInfo("name"))));
    }

    // ---- the registry and real shared libraries

    #[test]
    fn a_plugin_registers_and_is_found_by_its_description() {
        let lib = PluginLibrary::from_vtable(GAIN).unwrap();
        let mut r = ComponentRegistry::with_builtins();
        let before = r.len();
        lib.register_into(&mut r);
        assert_eq!(r.len(), before + 1);
        let q = ComponentDescription::new("aufx", "tgan", "test").unwrap();
        let at = r.find_next(&q, None).unwrap();
        assert_eq!(r.name(at), Some("Test Gain"));
        let unit = r.instantiate(at).unwrap();
        assert_eq!(unit.info().name, "Test Gain");
        assert_eq!(unit.tree().specs().len(), 1);
    }

    #[test]
    fn opening_a_missing_file_or_a_library_without_the_entry_reports_the_reason() {
        match PluginLibrary::open(Path::new("/nonexistent/bifrost-plugin.so")) {
            Err(PluginError::Load(m)) => assert!(!m.is_empty()),
            other => panic!("{:?}", other.err()),
        }
        // the C math library loads fine and has no plug-in entry
        assert!(matches!(PluginLibrary::open(Path::new("libm.so.6")), Err(PluginError::NoEntry)));
    }

    #[test]
    fn scanning_a_directory_reports_each_library_and_ignores_other_files() {
        let dir = std::env::temp_dir().join(format!("bifrost-units-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.so"), b"not an elf").unwrap();
        std::fs::write(dir.join("readme.txt"), b"hello").unwrap();
        let mut r = ComponentRegistry::new();
        let reports = scan_directory(&dir, &mut r);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].path.ends_with("broken.so"));
        assert!(matches!(reports[0].result, Err(PluginError::Load(_))));
        assert!(r.is_empty());
        assert!(scan_directory(Path::new("/nonexistent/bifrost-units"), &mut r).is_empty());
    }

    #[test]
    fn default_directories_start_with_the_system_one() {
        let d = default_directories();
        assert_eq!(d[0], PathBuf::from(SYSTEM_DIR));
    }

    #[test]
    fn struct_layout_matches_the_documented_c_abi() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<CMidi>(), 8);
        assert_eq!(size_of::<CRenderConfig>(), 24);
        assert_eq!(offset_of!(CBlock, sample_time), 16);
        assert_eq!(offset_of!(CBlock, inputs), 24);
        assert_eq!(offset_of!(CBlock, param_start), 56);
        assert_eq!(size_of::<CBlock>(), 72);
        assert_eq!(offset_of!(CInfo, name), 16);
        assert_eq!(size_of::<CInfo>(), 92);
        assert_eq!(offset_of!(CParam, name), 28);
        assert_eq!(size_of::<CParam>(), 28 + 64 + 64 + 192);
    }
}
