//! The C ABI of libjack: the functions of `jack/jack.h`, `jack/transport.h` and `jack/midiport.h` that clients
//! use. Every function maps onto the process-local engine (`engine.rs`). Variadic `jack_client_open` is declared with
//! its three fixed parameters; the extra variadic argument (the server name) is ignored, which is ABI-compatible on
//! the System V calling convention.
//!
//! Not provided (see docs/audio/18-jack.md): internal clients, `jack_set_process_thread` (returns -1),
//! `jack_cycle_wait/signal`, session and metadata calls.

use crate::driver::{self, now_usecs, run_calls, Runtime};
use crate::engine::*;
use crate::midi;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_float, c_int, c_ulong, c_void};
use std::sync::atomic::Ordering::*;
use std::sync::{Arc, Mutex};

pub struct ClientObj {
    id: u32,
    name: CString,
    rt: Arc<Runtime>,
}

/// A `jack_port_t`: a handle that stays valid until the port is unregistered.
pub struct PortObj {
    pub id: u32,
    buf: *mut f32,
    meta: Arc<PortMeta>,
}

// SAFETY: the buffer pointer is only handed to the client, which uses it on the process thread.
unsafe impl Send for PortObj {}

static RUNTIME: Mutex<Option<Arc<Runtime>>> = Mutex::new(None);

fn runtime() -> Option<Arc<Runtime>> {
    RUNTIME.lock().ok().and_then(|r| r.clone())
}

fn lock(rt: &Runtime) -> std::sync::MutexGuard<'_, Engine> {
    rt.engine.lock().unwrap_or_else(|p| p.into_inner())
}

/// SAFETY helper: `c` must be a pointer returned by `jack_client_open`.
unsafe fn cl<'a>(c: *mut ClientObj) -> Option<&'a ClientObj> {
    c.as_ref()
}

/// Runs `f` on the engine with the client id; the notifications it returns run after the lock is released.
fn ctl<T>(c: &ClientObj, f: impl FnOnce(&mut Engine, u32) -> (T, Vec<Call>)) -> T {
    let (v, calls) = f(&mut lock(&c.rt), c.id);
    run_calls(calls);
    v
}

fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the caller passes a NUL-terminated string.
        unsafe { CStr::from_ptr(p) }.to_str().ok()
    }
}

/// One malloc block: `n + 1` string pointers (the last NULL) followed by the strings; `jack_free` releases it.
fn string_array(names: &[CString]) -> *mut *const c_char {
    if names.is_empty() {
        return std::ptr::null_mut();
    }
    let ptrs = (names.len() + 1) * std::mem::size_of::<*const c_char>();
    let total = ptrs + names.iter().map(|n| n.as_bytes_with_nul().len()).sum::<usize>();
    // SAFETY: the block is `total` bytes; every write below stays inside it.
    unsafe {
        let base = libc::malloc(total) as *mut u8;
        if base.is_null() {
            return std::ptr::null_mut();
        }
        let arr = base as *mut *const c_char;
        let mut at = base.add(ptrs);
        for (i, n) in names.iter().enumerate() {
            let b = n.as_bytes_with_nul();
            std::ptr::copy_nonoverlapping(b.as_ptr(), at, b.len());
            *arr.add(i) = at as *const c_char;
            at = at.add(b.len());
        }
        *arr.add(names.len()) = std::ptr::null();
        arr
    }
}

fn port_handle(rt: &Runtime, e: &Engine, id: u32) -> *mut c_void {
    let Some(p) = e.port(id) else { return std::ptr::null_mut() };
    let mut h = rt.handles.lock().unwrap_or_else(|p| p.into_inner());
    let b = h.entry(id).or_insert_with(|| Box::new(PortObj { id, buf: p.buf_ptr(), meta: p.meta.clone() }));
    &mut **b as *mut PortObj as *mut c_void
}

unsafe fn pid(p: *mut c_void) -> Option<u32> {
    (p as *const PortObj).as_ref().map(|p| p.id)
}

// ----- client ---------------------------------------------------------------------------------------------------

const OPT_USE_EXACT_NAME: c_int = 0x02;

/// `jack_client_open(name, options, status, ...)`. The variadic server name is ignored.
#[no_mangle]
pub unsafe extern "C" fn jack_client_open(name: *const c_char, options: c_int, status: *mut c_int) -> *mut ClientObj {
    let set = |s: u32| {
        if !status.is_null() {
            *status = s as c_int;
        }
    };
    let Some(name) = cstr(name) else {
        set(STATUS_FAILURE);
        return std::ptr::null_mut();
    };
    let rt = {
        let mut g = RUNTIME.lock().unwrap_or_else(|p| p.into_inner());
        match g.as_ref() {
            Some(rt) => rt.clone(),
            None => match driver::create() {
                Ok(rt) => {
                    *g = Some(rt.clone());
                    rt
                }
                Err(s) => {
                    set(s);
                    return std::ptr::null_mut();
                }
            },
        }
    };
    let opened = lock(&rt).open_client(name, options & OPT_USE_EXACT_NAME != 0);
    match opened {
        Ok((id, final_name, st, calls)) => {
            rt.clients.fetch_add(1, AcqRel);
            run_calls(calls);
            set(st);
            Box::into_raw(Box::new(ClientObj { id, name: CString::new(final_name).unwrap_or_default(), rt }))
        }
        Err(s) => {
            release(&rt);
            set(s);
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_new(name: *const c_char) -> *mut ClientObj {
    jack_client_open(name, 0, std::ptr::null_mut())
}

/// Stops the runtime when no client is left.
fn release(rt: &Arc<Runtime>) {
    if rt.clients.load(Acquire) == 0 {
        let mut g = RUNTIME.lock().unwrap_or_else(|p| p.into_inner());
        if g.as_ref().map_or(false, |r| Arc::ptr_eq(r, rt)) {
            *g = None;
            drop(g);
            rt.shutdown();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_close(client: *mut ClientObj) -> c_int {
    if client.is_null() {
        return -1;
    }
    let c = Box::from_raw(client);
    let calls = {
        let mut e = lock(&c.rt);
        let mut calls = e.deactivate(c.id);
        calls.extend(e.close_client(c.id));
        calls
    };
    run_calls(calls);
    c.rt.clients.fetch_sub(1, AcqRel);
    release(&c.rt);
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_name_size() -> c_int {
    CLIENT_NAME_SIZE as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_name_size() -> c_int {
    PORT_NAME_SIZE as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_type_size() -> c_int {
    32
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_type_get_buffer_size(client: *mut ClientObj, ty: *const c_char) -> usize {
    let Some(c) = cl(client) else { return 0 };
    match cstr(ty) {
        Some(AUDIO_TYPE) | Some(MIDI_TYPE) => c.rt.sh.buffer_size.load(Relaxed) as usize * 4,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_client_name(client: *mut ClientObj) -> *const c_char {
    cl(client).map_or(std::ptr::null(), |c| c.name.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_client_pid(name: *const c_char) -> c_int {
    let (Some(rt), Some(n)) = (runtime(), cstr(name)) else { return 0 };
    if lock(&rt).client_names().iter().any(|c| c == n) {
        libc::getpid()
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_activate(client: *mut ClientObj) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, id| (0, e.activate(id)))
}

#[no_mangle]
pub unsafe extern "C" fn jack_deactivate(client: *mut ClientObj) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, id| (0, e.deactivate(id)))
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_sample_rate(client: *mut ClientObj) -> u32 {
    cl(client).map_or(0, |c| c.rt.sh.sample_rate.load(Relaxed))
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_buffer_size(client: *mut ClientObj) -> u32 {
    cl(client).map_or(0, |c| c.rt.sh.buffer_size.load(Relaxed))
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_buffer_size(client: *mut ClientObj, nframes: u32) -> c_int {
    cl(client).map_or(-1, |c| c.rt.set_buffer_size(nframes))
}

#[no_mangle]
pub unsafe extern "C" fn jack_cpu_load(client: *mut ClientObj) -> c_float {
    cl(client).map_or(0.0, |c| c.rt.sh.load.load(Relaxed) as f32 / 100.0)
}

#[no_mangle]
pub unsafe extern "C" fn jack_is_realtime(_client: *mut ClientObj) -> c_int {
    1
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_thread_id(client: *mut ClientObj) -> c_ulong {
    cl(client).map_or(0, |c| c.rt.sh.rt_thread.load(Relaxed) as c_ulong)
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_version(major: *mut c_int, minor: *mut c_int, micro: *mut c_int, proto: *mut c_int) {
    for (p, v) in [(major, 1), (minor, 9), (micro, 22), (proto, 8)] {
        if !p.is_null() {
            *p = v;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_version_string() -> *const c_char {
    b"1.9.22\0".as_ptr() as *const c_char
}

#[no_mangle]
pub unsafe extern "C" fn jack_free(ptr: *mut c_void) {
    libc::free(ptr);
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_freewheel(client: *mut ClientObj, onoff: c_int) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    c.rt.sh.freewheel.store(onoff != 0, Release);
    ctl(c, |e, _| (0, e.freewheel_calls(onoff != 0)))
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_process_thread(_client: *mut ClientObj, _f: *mut c_void, _arg: *mut c_void) -> c_int {
    -1
}

// ----- diagnostics ----------------------------------------------------------------------------------------------

#[no_mangle]
pub static mut jack_error_callback: Option<unsafe extern "C" fn(*const c_char)> = None;
#[no_mangle]
pub static mut jack_info_callback: Option<unsafe extern "C" fn(*const c_char)> = None;

#[no_mangle]
pub unsafe extern "C" fn jack_set_error_function(f: Option<unsafe extern "C" fn(*const c_char)>) {
    jack_error_callback = f;
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_info_function(f: Option<unsafe extern "C" fn(*const c_char)>) {
    jack_info_callback = f;
}

// ----- callbacks ------------------------------------------------------------------------------------------------

macro_rules! setter {
    ($name:ident, $field:ident, $ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(client: *mut ClientObj, f: Option<$ty>, arg: *mut c_void) -> c_int {
            let Some(c) = cl(client) else { return -1 };
            ctl(c, |e, id| match e.client_mut(id) {
                Some(cc) if !cc.active => {
                    cc.cb.$field = f.map(|f| Cb(f, arg as usize));
                    (0, vec![])
                }
                _ => (-1, vec![]),
            })
        }
    };
}

setter!(jack_set_process_callback, process, ProcessFn);
setter!(jack_set_thread_init_callback, thread_init, ThreadInitFn);
setter!(jack_set_graph_order_callback, graph_order, GraphOrderFn);
setter!(jack_set_xrun_callback, xrun, XrunFn);
setter!(jack_set_buffer_size_callback, buffer_size, BufferSizeFn);
setter!(jack_set_sample_rate_callback, sample_rate, SampleRateFn);
setter!(jack_set_port_registration_callback, port_reg, PortRegFn);
setter!(jack_set_client_registration_callback, client_reg, ClientRegFn);
setter!(jack_set_port_connect_callback, port_connect, PortConnectFn);
setter!(jack_set_port_rename_callback, port_rename, PortRenameFn);
setter!(jack_set_freewheel_callback, freewheel, FreewheelFn);
setter!(jack_set_latency_callback, latency, LatencyFn);

#[no_mangle]
pub unsafe extern "C" fn jack_on_shutdown(client: *mut ClientObj, f: Option<ShutdownFn>, arg: *mut c_void) {
    if let Some(c) = cl(client) {
        ctl(c, |e, id| {
            if let Some(cc) = e.client_mut(id) {
                cc.cb.shutdown = f.map(|f| Cb(f, arg as usize));
            }
            ((), vec![])
        })
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_on_info_shutdown(client: *mut ClientObj, f: Option<InfoShutdownFn>, arg: *mut c_void) {
    if let Some(c) = cl(client) {
        ctl(c, |e, id| {
            if let Some(cc) = e.client_mut(id) {
                cc.cb.info_shutdown = f.map(|f| Cb(f, arg as usize));
            }
            ((), vec![])
        })
    }
}

// ----- ports ----------------------------------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn jack_port_register(client: *mut ClientObj, name: *const c_char, ty: *const c_char, flags: c_ulong, _buffer_size: c_ulong) -> *mut c_void {
    let (Some(c), Some(name), Some(ty)) = (cl(client), cstr(name), cstr(ty)) else { return std::ptr::null_mut() };
    let ptype = match ty {
        AUDIO_TYPE => PortType::Audio,
        MIDI_TYPE => PortType::Midi,
        _ => return std::ptr::null_mut(),
    };
    let (r, calls) = {
        let mut e = lock(&c.rt);
        match e.register_port(c.id, name, ptype, flags as u32) {
            Ok((id, calls)) => (port_handle(&c.rt, &e, id), calls),
            Err(_) => (std::ptr::null_mut(), vec![]),
        }
    };
    run_calls(calls);
    r
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_unregister(client: *mut ClientObj, port: *mut c_void) -> c_int {
    let (Some(c), Some(id)) = (cl(client), pid(port)) else { return -1 };
    ctl(c, |e, cid| ((), e.unregister_port(cid, id)));
    c.rt.handles.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_buffer(port: *mut c_void, _nframes: u32) -> *mut c_void {
    (port as *const PortObj).as_ref().map_or(std::ptr::null_mut(), |p| p.buf as *mut c_void)
}

/// Looks `port` up and applies `f`; `d` is the default when the port is gone.
/// Never takes the engine lock: these queries are called from the process callback, which runs under it.
unsafe fn with_port<T>(port: *mut c_void, d: T, f: impl FnOnce(&PortMeta) -> T) -> T {
    (port as *const PortObj).as_ref().map_or(d, |p| f(&p.meta))
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_name(port: *mut c_void) -> *const c_char {
    with_port(port, std::ptr::null(), |p| p.full_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_short_name(port: *mut c_void) -> *const c_char {
    // The short name is the part after the colon of the full name.
    with_port(port, std::ptr::null(), |p| p.short_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_flags(port: *mut c_void) -> c_int {
    with_port(port, 0, |p| p.flags as c_int)
}

static AUDIO_TYPE_C: &[u8] = b"32 bit float mono audio\0";
static MIDI_TYPE_C: &[u8] = b"8 bit raw midi\0";

#[no_mangle]
pub unsafe extern "C" fn jack_port_type(port: *mut c_void) -> *const c_char {
    with_port(port, std::ptr::null(), |p| match p.ptype {
        PortType::Audio => AUDIO_TYPE_C.as_ptr() as *const c_char,
        PortType::Midi => MIDI_TYPE_C.as_ptr() as *const c_char,
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_type_id(port: *mut c_void) -> u32 {
    with_port(port, 0, |p| p.ptype as u32)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_is_mine(client: *const ClientObj, port: *mut c_void) -> c_int {
    let Some(c) = (client as *mut ClientObj).as_ref() else { return 0 };
    with_port(port, false, |p| p.client == c.id) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_connected(port: *const c_void) -> c_int {
    with_port(port as *mut c_void, 0, |p| p.connection_count() as c_int)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_connected_to(port: *const c_void, name: *const c_char) -> c_int {
    let Some(n) = cstr(name) else { return 0 };
    with_port(port as *mut c_void, false, |p| p.connected_to(n)) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_connections(port: *const c_void) -> *mut *const c_char {
    jack_port_get_all_connections(std::ptr::null_mut(), port)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_all_connections(_client: *const ClientObj, port: *const c_void) -> *mut *const c_char {
    let names = with_port(port as *mut c_void, Vec::new(), |p| p.peer_names());
    string_array(&names)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_by_name(client: *mut ClientObj, name: *const c_char) -> *mut c_void {
    let (Some(c), Some(n)) = (cl(client), cstr(name)) else { return std::ptr::null_mut() };
    let e = lock(&c.rt);
    e.port_by_name(n).map_or(std::ptr::null_mut(), |id| port_handle(&c.rt, &e, id))
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_by_id(client: *mut ClientObj, id: u32) -> *mut c_void {
    let Some(c) = cl(client) else { return std::ptr::null_mut() };
    let e = lock(&c.rt);
    port_handle(&c.rt, &e, id)
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_ports(client: *mut ClientObj, name_pattern: *const c_char, type_pattern: *const c_char, flags: c_ulong) -> *mut *const c_char {
    let Some(c) = cl(client) else { return std::ptr::null_mut() };
    let names = lock(&c.rt).get_ports(cstr(name_pattern).unwrap_or(""), cstr(type_pattern).unwrap_or(""), flags as u32);
    string_array(&names)
}

#[no_mangle]
pub unsafe extern "C" fn jack_connect(client: *mut ClientObj, src: *const c_char, dst: *const c_char) -> c_int {
    let (Some(c), Some(s), Some(d)) = (cl(client), cstr(src), cstr(dst)) else { return -1 };
    ctl(c, |e, _| e.connect(s, d))
}

#[no_mangle]
pub unsafe extern "C" fn jack_disconnect(client: *mut ClientObj, src: *const c_char, dst: *const c_char) -> c_int {
    let (Some(c), Some(s), Some(d)) = (cl(client), cstr(src), cstr(dst)) else { return -1 };
    ctl(c, |e, _| e.disconnect(s, d))
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_disconnect(client: *mut ClientObj, port: *mut c_void) -> c_int {
    let (Some(c), Some(id)) = (cl(client), pid(port)) else { return -1 };
    ctl(c, |e, _| (0, e.disconnect_all(id)))
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_set_name(port: *mut c_void, name: *const c_char) -> c_int {
    let (Some(rt), Some(id), Some(n)) = (runtime(), pid(port), cstr(name)) else { return -1 };
    let r = lock(&rt).rename_port(id, n);
    match r {
        Ok(calls) => {
            run_calls(calls);
            0
        }
        Err(e) => e,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_rename(client: *mut ClientObj, port: *mut c_void, name: *const c_char) -> c_int {
    let _ = client;
    jack_port_set_name(port, name)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_set_alias(port: *mut c_void, alias: *const c_char) -> c_int {
    let (Some(rt), Some(id), Some(a)) = (runtime(), pid(port), cstr(alias)) else { return -1 };
    let mut e = lock(&rt);
    match e.port_aliases_mut(id) {
        Some(v) if v.len() < 2 => {
            v.push(CString::new(a).unwrap_or_default());
            0
        }
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_unset_alias(port: *mut c_void, alias: *const c_char) -> c_int {
    let (Some(rt), Some(id), Some(a)) = (runtime(), pid(port), cstr(alias)) else { return -1 };
    let mut e = lock(&rt);
    match e.port_aliases_mut(id) {
        Some(v) if v.iter().any(|x| x.to_str() == Ok(a)) => {
            v.retain(|x| x.to_str() != Ok(a));
            0
        }
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_aliases(port: *const c_void, aliases: *mut *mut c_char) -> c_int {
    let (Some(rt), Some(id)) = (runtime(), pid(port as *mut c_void)) else { return -1 };
    let e = lock(&rt);
    let Some(p) = e.port(id) else { return -1 };
    for (i, a) in p.aliases.iter().take(2).enumerate() {
        let b = a.as_bytes_with_nul();
        std::ptr::copy_nonoverlapping(b.as_ptr(), *aliases.add(i) as *mut u8, b.len().min(PORT_NAME_SIZE));
    }
    p.aliases.len().min(2) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_request_monitor(_port: *mut c_void, _onoff: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_request_monitor_by_name(_client: *mut ClientObj, _name: *const c_char, _onoff: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_ensure_monitor(_port: *mut c_void, _onoff: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_monitoring_input(_port: *mut c_void) -> c_int {
    0
}

// ----- latency --------------------------------------------------------------------------------------------------

#[repr(C)]
pub struct LatencyRange {
    min: u32,
    max: u32,
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_set_latency(port: *mut c_void, frames: u32) {
    if let (Some(rt), Some(id)) = (runtime(), pid(port)) {
        let mut e = lock(&rt);
        e.set_latency_range(id, false, (frames, frames));
        e.set_latency_range(id, true, (frames, frames));
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_latency(port: *mut c_void) -> u32 {
    let (Some(rt), Some(id)) = (runtime(), pid(port)) else { return 0 };
    let e = lock(&rt);
    e.port(id).map_or(0, |p| p.latency[1].1)
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_set_latency_range(port: *mut c_void, mode: c_int, range: *const LatencyRange) {
    if let (Some(rt), Some(id), Some(r)) = (runtime(), pid(port), range.as_ref()) {
        lock(&rt).set_latency_range(id, mode == 1, (r.min, r.max));
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_latency_range(port: *mut c_void, mode: c_int, range: *mut LatencyRange) {
    if let (Some(rt), Some(id), Some(r)) = (runtime(), pid(port), range.as_mut()) {
        let (min, max) = lock(&rt).latency_range(id, mode == 1);
        *r = LatencyRange { min, max };
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_total_latency(_client: *mut ClientObj, port: *mut c_void) -> c_int {
    let (Some(rt), Some(id)) = (runtime(), pid(port)) else { return 0 };
    let t = lock(&rt).total_latency(id);
    t as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_recompute_total_latencies(client: *mut ClientObj) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, _| (0, e.recompute_latencies()))
}

#[no_mangle]
pub unsafe extern "C" fn jack_recompute_total_latency(client: *mut ClientObj, _port: *mut c_void) -> c_int {
    jack_recompute_total_latencies(client)
}

// ----- time -----------------------------------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn jack_get_time() -> u64 {
    now_usecs()
}

fn frames_to_time(sh: &Shared, frames: u32) -> u64 {
    let rate = sh.sample_rate.load(Relaxed).max(1) as i64;
    let delta = frames.wrapping_sub(sh.last_frames.load(Acquire)) as i32 as i64;
    (sh.cycle_usecs.load(Acquire) as i64 + delta * 1_000_000 / rate).max(0) as u64
}

fn time_to_frames(sh: &Shared, usecs: u64) -> u32 {
    let rate = sh.sample_rate.load(Relaxed).max(1) as i64;
    let delta = (usecs as i64 - sh.cycle_usecs.load(Acquire) as i64) * rate / 1_000_000;
    sh.last_frames.load(Acquire).wrapping_add(delta as i32 as u32)
}

#[no_mangle]
pub unsafe extern "C" fn jack_frames_to_time(client: *const ClientObj, frames: u32) -> u64 {
    (client as *mut ClientObj).as_ref().map_or(0, |c| frames_to_time(&c.rt.sh, frames))
}

#[no_mangle]
pub unsafe extern "C" fn jack_time_to_frames(client: *const ClientObj, usecs: u64) -> u32 {
    (client as *mut ClientObj).as_ref().map_or(0, |c| time_to_frames(&c.rt.sh, usecs))
}

#[no_mangle]
pub unsafe extern "C" fn jack_frame_time(client: *const ClientObj) -> u32 {
    (client as *mut ClientObj).as_ref().map_or(0, |c| time_to_frames(&c.rt.sh, now_usecs()))
}

#[no_mangle]
pub unsafe extern "C" fn jack_last_frame_time(client: *const ClientObj) -> u32 {
    (client as *mut ClientObj).as_ref().map_or(0, |c| c.rt.sh.last_frames.load(Acquire))
}

#[no_mangle]
pub unsafe extern "C" fn jack_frames_since_cycle_start(client: *const ClientObj) -> u32 {
    (client as *mut ClientObj).as_ref().map_or(0, |c| {
        let now = time_to_frames(&c.rt.sh, now_usecs());
        (now.wrapping_sub(c.rt.sh.last_frames.load(Acquire)) as i32).max(0) as u32
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_cycle_times(client: *const ClientObj, current_frames: *mut u32, current_usecs: *mut u64, next_usecs: *mut u64, period_usecs: *mut c_float) -> c_int {
    let Some(c) = (client as *mut ClientObj).as_ref() else { return -1 };
    let sh = &c.rt.sh;
    if !current_frames.is_null() {
        *current_frames = sh.last_frames.load(Acquire);
    }
    if !current_usecs.is_null() {
        *current_usecs = sh.cycle_usecs.load(Acquire);
    }
    if !next_usecs.is_null() {
        *next_usecs = sh.next_usecs.load(Acquire);
    }
    if !period_usecs.is_null() {
        *period_usecs = sh.period_usecs.load(Acquire) as f32;
    }
    0
}

// ----- transport ------------------------------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn jack_transport_query(client: *const ClientObj, pos: *mut Position) -> c_int {
    let Some(c) = (client as *mut ClientObj).as_ref() else { return 0 };
    let sh = &c.rt.sh;
    if let Some(p) = pos.as_mut() {
        *p = *sh.pos.lock().unwrap_or_else(|p| p.into_inner());
        p.frame = sh.frame.load(Acquire);
        p.frame_rate = sh.sample_rate.load(Relaxed);
    }
    sh.state.load(Acquire) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_current_transport_frame(client: *const ClientObj) -> u32 {
    let Some(c) = (client as *mut ClientObj).as_ref() else { return 0 };
    let sh = &c.rt.sh;
    let base = sh.frame.load(Acquire);
    if sh.state.load(Acquire) == TRANSPORT_ROLLING {
        base.wrapping_add(jack_frames_since_cycle_start(client))
    } else {
        base
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_locate(client: *mut ClientObj, frame: u32) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, _| {
        e.transport_locate(frame);
        (0, vec![])
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_reposition(client: *mut ClientObj, pos: *const Position) -> c_int {
    let (Some(c), Some(p)) = (cl(client), pos.as_ref()) else { return -1 };
    jack_transport_locate(c as *const ClientObj as *mut ClientObj, p.frame)
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_start(client: *mut ClientObj) {
    if let Some(c) = cl(client) {
        ctl(c, |e, _| {
            e.transport_start();
            ((), vec![])
        })
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_stop(client: *mut ClientObj) {
    if let Some(c) = cl(client) {
        ctl(c, |e, _| {
            e.transport_stop();
            ((), vec![])
        })
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_sync_callback(client: *mut ClientObj, f: Option<SyncFn>, arg: *mut c_void) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, id| {
        if let Some(cc) = e.client_mut(id) {
            cc.cb.sync = f.map(|f| Cb(f, arg as usize));
        }
        e.set_sync_callback(id);
        (0, vec![])
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_sync_timeout(client: *mut ClientObj, timeout_usecs: u64) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, _| {
        e.set_sync_timeout(timeout_usecs);
        (0, vec![])
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_timebase_callback(client: *mut ClientObj, conditional: c_int, f: Option<TimebaseFn>, arg: *mut c_void) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, id| {
        let r = e.set_timebase(id, conditional != 0);
        if r == 0 {
            if let Some(cc) = e.client_mut(id) {
                cc.cb.timebase = f.map(|f| Cb(f, arg as usize));
            }
        }
        (r, vec![])
    })
}

#[no_mangle]
pub unsafe extern "C" fn jack_release_timebase(client: *mut ClientObj) -> c_int {
    let Some(c) = cl(client) else { return -1 };
    ctl(c, |e, id| (e.release_timebase(id), vec![]))
}

// ----- MIDI -----------------------------------------------------------------------------------------------------

#[repr(C)]
pub struct MidiEvent {
    time: u32,
    size: usize,
    buffer: *mut u8,
}

unsafe fn mbuf<'a>(p: *mut c_void) -> &'a mut [u8] {
    std::slice::from_raw_parts_mut(p as *mut u8, BUF_BYTES)
}

fn current_frames() -> u32 {
    driver::CURRENT_FRAMES.load(Relaxed)
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_get_event_count(port_buffer: *mut c_void) -> u32 {
    midi::count(mbuf(port_buffer))
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_get(event: *mut MidiEvent, port_buffer: *mut c_void, index: u32) -> c_int {
    let b = mbuf(port_buffer);
    match (midi::get(b, index), event.as_mut()) {
        (Some((time, data)), Some(ev)) => {
            *ev = MidiEvent { time, size: data.len(), buffer: data.as_ptr() as *mut u8 };
            0
        }
        _ => -libc::ENODATA,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_clear_buffer(port_buffer: *mut c_void) {
    midi::clear(mbuf(port_buffer));
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_reset_buffer(port_buffer: *mut c_void) {
    midi::clear(mbuf(port_buffer));
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_max_event_size(port_buffer: *mut c_void) -> usize {
    midi::max_event_size(mbuf(port_buffer))
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_reserve(port_buffer: *mut c_void, time: u32, size: usize) -> *mut u8 {
    let b = mbuf(port_buffer);
    match midi::reserve(b, time, size, current_frames()) {
        Some(at) => b.as_mut_ptr().add(at),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_write(port_buffer: *mut c_void, time: u32, data: *const u8, size: usize) -> c_int {
    let b = mbuf(port_buffer);
    if midi::write(b, time, std::slice::from_raw_parts(data, size), current_frames()) {
        0
    } else {
        -libc::ENOBUFS
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_get_lost_event_count(port_buffer: *mut c_void) -> u32 {
    midi::lost(mbuf(port_buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize};
    use std::time::Duration;

    // The FFI tests share the process-wide runtime, so they run inside one test function.
    static PROCESSED: AtomicU32 = AtomicU32::new(0);
    static SEEN: AtomicUsize = AtomicUsize::new(0);

    struct Ctx {
        input: *mut c_void,
        output: *mut c_void,
    }

    extern "C" fn process(nframes: u32, arg: *mut c_void) -> c_int {
        // SAFETY: arg is the Ctx below, alive until the client is closed.
        unsafe {
            let ctx = &*(arg as *const Ctx);
            // Port queries from the process callback must not take the engine lock (it is held by this very cycle).
            assert!(!jack_port_name(ctx.input).is_null());
            let _ = jack_port_connected(ctx.output) + jack_port_flags(ctx.output);
            let i = jack_port_get_buffer(ctx.input, nframes) as *const f32;
            let o = jack_port_get_buffer(ctx.output, nframes) as *mut f32;
            for k in 0..nframes as usize {
                *o.add(k) = *i.add(k) * 0.5 + 0.25;
            }
        }
        PROCESSED.fetch_add(1, Relaxed);
        0
    }

    extern "C" fn on_connect(_a: u32, _b: u32, connect: c_int, _arg: *mut c_void) {
        if connect != 0 {
            SEEN.fetch_add(1, Relaxed);
        }
    }

    #[test]
    fn a_client_opens_connects_processes_and_closes() {
        std::env::set_var("BIFROST_JACK_DUMMY", "1");
        std::env::set_var("BIFROST_JACK_FRAMES", "64");
        unsafe {
            let mut status = 0;
            let name = CString::new("simple").unwrap();
            let c = jack_client_open(name.as_ptr(), 0, &mut status);
            assert!(!c.is_null());
            assert_eq!(status, 0);
            assert_eq!(jack_get_buffer_size(c), 64);
            assert_eq!(jack_get_sample_rate(c), 48000);
            assert_eq!(CStr::from_ptr(jack_get_client_name(c)).to_str().unwrap(), "simple");

            // A second client of the same name gets a suffix and the "not unique" status.
            let mut st2 = 0;
            let c2 = jack_client_open(name.as_ptr(), 0, &mut st2);
            assert_eq!(st2 as u32, STATUS_NAME_NOT_UNIQUE);
            assert_eq!(CStr::from_ptr(jack_get_client_name(c2)).to_str().unwrap(), "simple-01");
            assert_eq!(jack_client_close(c2), 0);

            let audio = CString::new(AUDIO_TYPE).unwrap();
            let (pi, po) = (CString::new("in").unwrap(), CString::new("out").unwrap());
            let input = jack_port_register(c, pi.as_ptr(), audio.as_ptr(), PORT_IS_INPUT as c_ulong, 0);
            let output = jack_port_register(c, po.as_ptr(), audio.as_ptr(), PORT_IS_OUTPUT as c_ulong, 0);
            assert!(!input.is_null() && !output.is_null());
            assert_eq!(CStr::from_ptr(jack_port_name(output)).to_str().unwrap(), "simple:out");
            assert_eq!(CStr::from_ptr(jack_port_short_name(output)).to_str().unwrap(), "out");
            assert_eq!(jack_port_is_mine(c, output), 1);
            assert_eq!(CStr::from_ptr(jack_port_type(output)).to_str().unwrap(), AUDIO_TYPE);

            let ctx = Box::new(Ctx { input, output });
            assert_eq!(jack_set_process_callback(c, Some(process), &*ctx as *const Ctx as *mut c_void), 0);
            assert_eq!(jack_set_port_connect_callback(c, Some(on_connect), std::ptr::null_mut()), 0);
            assert_eq!(jack_activate(c), 0);
            // Callbacks cannot be changed while active.
            assert_eq!(jack_set_process_callback(c, Some(process), std::ptr::null_mut()), -1);

            let cap1 = CString::new("system:capture_1").unwrap();
            let play1 = CString::new("system:playback_1").unwrap();
            let sin = CString::new("simple:in").unwrap();
            let sout = CString::new("simple:out").unwrap();
            assert_eq!(jack_connect(c, cap1.as_ptr(), sin.as_ptr()), 0);
            assert_eq!(jack_connect(c, sout.as_ptr(), play1.as_ptr()), 0);
            assert_eq!(jack_connect(c, sout.as_ptr(), play1.as_ptr()), EEXIST);
            assert_eq!(SEEN.load(Relaxed), 2);
            assert_eq!(jack_port_connected(output), 1);

            // The physical ports are visible by pattern and flags.
            let pat = CString::new("^system:playback").unwrap();
            let list = jack_get_ports(c, pat.as_ptr(), std::ptr::null(), PORT_IS_PHYSICAL as c_ulong);
            assert!(!list.is_null());
            assert_eq!(CStr::from_ptr(*list).to_str().unwrap(), "system:playback_1");
            assert_eq!(CStr::from_ptr(*list.add(1)).to_str().unwrap(), "system:playback_2");
            assert!((*list.add(2)).is_null());
            jack_free(list as *mut c_void);

            let conns = jack_port_get_connections(output);
            assert_eq!(CStr::from_ptr(*conns).to_str().unwrap(), "system:playback_1");
            jack_free(conns as *mut c_void);

            // The dummy driver runs cycles: wait for a few.
            std::thread::sleep(Duration::from_millis(60));
            assert!(PROCESSED.load(Relaxed) > 5);
            // Capture is silence, so the output is 0 * 0.5 + 0.25.
            let o = jack_port_get_buffer(output, 64) as *const f32;
            assert_eq!(*o, 0.25);

            // Transport.
            let mut pos: Position = std::mem::zeroed();
            assert_eq!(jack_transport_query(c, &mut pos), TRANSPORT_STOPPED as c_int);
            jack_transport_start(c);
            std::thread::sleep(Duration::from_millis(30));
            assert_eq!(jack_transport_query(c, &mut pos), TRANSPORT_ROLLING as c_int);
            assert!(pos.frame > 0);
            let rate = pos.frame_rate;
            assert_eq!(rate, 48000);
            jack_transport_stop(c);
            assert_eq!(jack_transport_locate(c, 4800), 0);
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(jack_get_current_transport_frame(c), 4800);

            // Time conversion round trip within a period.
            let f = jack_last_frame_time(c);
            let t = jack_frames_to_time(c, f);
            assert_eq!(jack_time_to_frames(c, t), f);
            assert!(jack_cpu_load(c) >= 0.0);

            assert_eq!(jack_disconnect(c, sout.as_ptr(), play1.as_ptr()), 0);
            assert_eq!(jack_port_unregister(c, input), 0);
            assert_eq!(jack_deactivate(c), 0);
            assert_eq!(jack_client_close(c), 0);
            drop(ctx);
        }
        // The last client closing stops the runtime.
        assert!(runtime().is_none());
    }

    #[test]
    fn midi_functions_read_and_write_a_port_buffer() {
        let mut buf = vec![0u32; MAX_FRAMES];
        let p = buf.as_mut_ptr() as *mut c_void;
        unsafe {
            jack_midi_clear_buffer(p);
            assert_eq!(jack_midi_get_event_count(p), 0);
            assert_eq!(jack_midi_event_write(p, 4, [0x90u8, 64, 90].as_ptr(), 3), 0);
            let r = jack_midi_event_reserve(p, 8, 2);
            assert!(!r.is_null());
            *r = 0xC0;
            *r.add(1) = 5;
            assert_eq!(jack_midi_get_event_count(p), 2);
            let mut ev = MidiEvent { time: 0, size: 0, buffer: std::ptr::null_mut() };
            assert_eq!(jack_midi_event_get(&mut ev, p, 1), 0);
            assert_eq!((ev.time, ev.size, *ev.buffer, *ev.buffer.add(1)), (8, 2, 0xC0, 5));
            assert_eq!(jack_midi_event_get(&mut ev, p, 2), -libc::ENODATA);
            // Out of order: lost.
            assert_eq!(jack_midi_event_write(p, 1, [1u8].as_ptr(), 1), -libc::ENOBUFS);
            assert_eq!(jack_midi_get_lost_event_count(p), 1);
        }
    }

    #[test]
    fn string_array_is_one_block_with_a_null_end() {
        let names = vec![CString::new("a:b").unwrap(), CString::new("cc:d").unwrap()];
        let arr = string_array(&names);
        unsafe {
            assert_eq!(CStr::from_ptr(*arr).to_str().unwrap(), "a:b");
            assert_eq!(CStr::from_ptr(*arr.add(1)).to_str().unwrap(), "cc:d");
            assert!((*arr.add(2)).is_null());
            // The strings live in the same block: right behind the three pointers.
            assert_eq!(*arr as usize, arr as usize + 3 * 8);
            libc::free(arr as *mut c_void);
        }
        assert!(string_array(&[]).is_null());
    }
}
