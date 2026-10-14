//! End to end: a client of the compositor's own user can list the globals and sees the privileged input globals.
//!
//! The filter of those globals runs while the Wayland backend holds its state lock, so it must not call back into
//! the backend (`Client::get_credentials` does, and never returns). The listing therefore runs on its own thread and
//! the test fails on a timeout instead of hanging with the compositor.

use std::{os::unix::net::UnixStream, path::PathBuf, sync::mpsc, thread, time::Duration};

use bifrost_wm::headless::Compositor;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};

struct Client;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

#[test]
fn a_same_user_client_lists_the_input_method_globals_without_wedging_the_compositor() {
    let wm = Compositor::start("wayland-bfwm-input-globals-test", 800, 600).expect("headless compositor");
    let socket = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).expect("XDG_RUNTIME_DIR").join(&wm.socket);

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let conn = Connection::from_socket(UnixStream::connect(socket).expect("connect")).expect("connection");
        let (globals, _queue) = registry_queue_init::<Client>(&conn).expect("registry");
        let names: Vec<String> = globals.contents().with_list(|list| list.iter().map(|g| g.interface.clone()).collect());
        tx.send(names).ok();
    });

    let names = rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| {
        // dropping the compositor joins its thread, which is the one that is stuck
        std::mem::forget(wm);
        panic!("listing the globals timed out: the compositor deadlocked in a global filter");
    });
    for interface in ["zwp_input_method_manager_v2", "zwp_virtual_keyboard_manager_v1", "zwp_text_input_manager_v3"] {
        assert!(names.iter().any(|n| n == interface), "{interface} is offered to a client of the same user, got {names:?}");
    }
}
