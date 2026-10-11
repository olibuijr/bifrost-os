//! A nested backend: the compositor in a window on another Wayland or X11 desktop, for development.

use std::{error::Error, time::Duration};

use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    desktop::layer_map_for_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::state::WmState;

const CLEAR: [f32; 4] = [0.06, 0.06, 0.08, 1.0];

pub fn init(event_loop: &mut EventLoop<'static, WmState>, state: &mut WmState) -> Result<(), Box<dyn Error>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "NESTED-1".into(),
        PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "BifrOSt".into(), model: "Nested".into() },
    );
    let _global = output.create_global::<WmState>(&state.dh);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.add_output(&output);
    let mut damage = OutputDamageTracker::from_output(&output);
    let mut glass = crate::glass::Glass::new(backend.renderer())?;

    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            layer_map_for_output(&output).arrange();
            state.refresh_area();
        }
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Redraw => {
            let size = backend.window_size();
            {
                let elements = crate::scene::build(backend.renderer(), state, &output, &mut glass, Vec::new());
                let Ok((renderer, mut fb)) = backend.bind() else { return };
                if let Err(e) = damage.render_output(renderer, &mut fb, 0, &elements, CLEAR) {
                    eprintln!("bifrost-wm: render: {e}");
                }
            }
            let _ = backend.submit(Some(&[Rectangle::from_size(size)]));
            let elapsed = state.start.elapsed();
            for window in state.space.elements() {
                window.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
            }
            for layer in layer_map_for_output(&output).layers() {
                layer.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
            }
            state.space.refresh();
            state.popups.cleanup();
            let _ = state.dh.flush_clients();
            backend.window().request_redraw();
        }
        WinitEvent::CloseRequested => state.loop_signal.stop(),
        _ => {}
    })?;
    Ok(())
}
