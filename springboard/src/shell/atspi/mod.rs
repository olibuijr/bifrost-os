//! Linux AT-SPI2 accessibility bridge (parity: `NativeAccessibility`). `model` is the pure mapping from app-control
//! nodes to AT-SPI roles, states and object paths and always builds; `bridge` (feature `atspi`) talks D-Bus.
pub mod model;
#[cfg(feature = "atspi")]
pub mod bridge;

#[cfg(feature = "atspi")]
mod glue {
    use super::bridge::Bridge;
    use super::model::{build, AppSnap};
    use crate::shell::apphost::{installed, AppHost};
    use bifrost_proto::ToApp;
    use std::cell::RefCell;
    use std::sync::mpsc::{channel, Receiver, TryRecvError};

    enum State {
        Idle,
        Starting(Receiver<Option<Bridge>>),
        Up(Bridge),
        Off,
    }

    thread_local! {
        static STATE: RefCell<(State, f64)> = const { RefCell::new((State::Idle, f64::NEG_INFINITY)) };
    }

    /// Publishes the running apps' semantic trees to the accessibility bus and forwards screen-reader requests to the
    /// apps as `ToApp::AxSet`. Throttled to four times a second; the bus connection is made on a helper thread so a slow
    /// `org.a11y.Bus` never stalls the shell, and a missing bus turns the bridge off for the session.
    pub fn sync(ext: &mut [Option<AppHost>], t_ms: f64) {
        STATE.with(|s| {
            let (state, last) = &mut *s.borrow_mut();
            if t_ms - *last < 250.0 {
                return;
            }
            *last = t_ms;
            match state {
                State::Idle => {
                    let (tx, rx) = channel();
                    crate::bare::sys::spawn_named("atspi-start", move || {
                        let _ = tx.send(Bridge::start());
                    });
                    *state = State::Starting(rx);
                }
                State::Starting(rx) => match rx.try_recv() {
                    Ok(Some(b)) => *state = State::Up(b),
                    Ok(None) | Err(TryRecvError::Disconnected) => *state = State::Off,
                    Err(TryRecvError::Empty) => {}
                },
                State::Up(bridge) => {
                    let names = installed();
                    let apps = ext
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, h)| {
                            let h = h.as_ref().filter(|h| h.running())?;
                            let name = names.get(slot).map(|a| a.manifest.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| h.id.clone());
                            Some(AppSnap { slot: slot as u8, app_id: h.id.clone(), name, nodes: build(&h.controls, &h.controls_tree) })
                        })
                        .collect();
                    bridge.publish(apps);
                    for r in bridge.drain() {
                        if let Some(Some(h)) = ext.get_mut(r.slot as usize) {
                            h.send(&ToApp::AxSet { id: r.id, action: r.action });
                        }
                    }
                }
                State::Off => {}
            }
        });
    }
}

#[cfg(feature = "atspi")]
pub use glue::sync;
