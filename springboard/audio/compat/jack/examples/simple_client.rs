//! A JACK client in the shape of the reference `simple_client`: a 440 Hz sine on `system:playback_1` and `_2`
//! for five seconds, written against the library's own C ABI. Run on Midget (or the host with the dummy driver):
//!
//! ```text
//! BIFROST_JACK_DUMMY=1 cargo run --example simple_client     # no server needed
//! cargo run --example simple_client                          # through bifrost-audiod
//! ```
//!
//! The C twin, `simple_client.c`, builds against the installed `libjack.so` with `cc simple_client.c -ljack -lm`.

use jack::ffi::*;
use std::ffi::{c_char, c_int, c_void, CString};

struct State {
    out: [*mut c_void; 2],
    phase: f32,
}

extern "C" fn process(nframes: u32, arg: *mut c_void) -> c_int {
    // SAFETY: `arg` is the State below, alive until the client is closed; the port buffers hold `nframes` floats.
    unsafe {
        let st = &mut *(arg as *mut State);
        let l = jack_port_get_buffer(st.out[0], nframes) as *mut f32;
        let r = jack_port_get_buffer(st.out[1], nframes) as *mut f32;
        for i in 0..nframes as usize {
            let s = (st.phase * std::f32::consts::TAU).sin() * 0.2;
            st.phase = (st.phase + 440.0 / 48000.0).fract();
            *l.add(i) = s;
            *r.add(i) = s;
        }
    }
    0
}

fn main() {
    // SAFETY: plain calls into the C ABI with valid NUL-terminated strings.
    unsafe {
        let name = CString::new("simple_client").unwrap();
        let mut status = 0;
        let client = jack_client_open(name.as_ptr(), 0, &mut status);
        if client.is_null() {
            eprintln!("cannot open a client, status {status:#x}");
            std::process::exit(1);
        }
        let audio = CString::new("32 bit float mono audio").unwrap();
        let mut st = Box::new(State { out: [std::ptr::null_mut(); 2], phase: 0.0 });
        for (i, n) in ["out_1", "out_2"].iter().enumerate() {
            let n = CString::new(*n).unwrap();
            st.out[i] = jack_port_register(client, n.as_ptr(), audio.as_ptr(), 2, 0);
        }
        jack_set_process_callback(client, Some(process), &mut *st as *mut State as *mut c_void);
        jack_activate(client);
        for (i, dst) in ["system:playback_1", "system:playback_2"].iter().enumerate() {
            let src = CString::new(format!("simple_client:out_{}", i + 1)).unwrap();
            let dst = CString::new(*dst).unwrap();
            if jack_connect(client, src.as_ptr() as *const c_char, dst.as_ptr()) != 0 {
                eprintln!("cannot connect {src:?} to {dst:?}");
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
        jack_client_close(client);
    }
}
