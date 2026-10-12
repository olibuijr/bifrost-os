//! A nested backend: the compositor in a window on another Wayland or X11 desktop, for development.

use std::error::Error;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{texture::TextureRenderElement, Id, Kind},
            gles::{GlesRenderer, GlesTexture},
            ImportMem, Renderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::layer_map_for_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::{
    headless::{full_damage, VulkanHeadless},
    render_server::compose::SceneNode,
    state::WmState,
};

const CLEAR: [f32; 4] = [0.06, 0.06, 0.08, 1.0];

/// The nested window's Vulkan path (wave G2 `wm-nested-headless`), used when `render_server::selected()`. The winit
/// window is GLES, so this is the shm fallback: the render server composites into an offscreen Vulkan target, the
/// BGRA pixels are read back, and the GLES renderer draws them as one texture in the window. The copy costs a
/// readback per frame; a Vulkan swapchain would remove it and is not built yet.
struct VulkanPresenter {
    vk: VulkanHeadless,
}

impl VulkanPresenter {
    /// The one element that shows the Vulkan frame. A fresh element id each frame makes the damage tracker redraw it.
    fn element(&mut self, renderer: &mut GlesRenderer, nodes: &[SceneNode], size: (i32, i32)) -> Result<Vec<TextureRenderElement<GlesTexture>>, Box<dyn Error>> {
        let (w, h) = (size.0.max(1) as u32, size.1.max(1) as u32);
        self.vk.resize(w, h)?;
        let pixels = self.vk.render(nodes, &full_damage(w, h))?;
        let texture = renderer.import_memory(&pixels, Fourcc::Argb8888, (w as i32, h as i32).into(), false)?;
        Ok(vec![TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            texture,
            1,
            Transform::Normal,
            Some(1.0),
            None,
            None,
            None,
            Kind::Unspecified,
        )])
    }
}

/// Starts the Vulkan path when it is selected and a device is there; otherwise the GLES path runs alone.
fn vulkan_presenter(size: (i32, i32)) -> Option<VulkanPresenter> {
    if !crate::render_server::selected() {
        return None;
    }
    match VulkanHeadless::new(size.0.max(1) as u32, size.1.max(1) as u32) {
        Ok(vk) => Some(VulkanPresenter { vk }),
        Err(e) => {
            eprintln!("bifrost-wm: vulkan render server unavailable, using GLES: {e}");
            None
        }
    }
}

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
    let start = backend.window_size();
    let mut vulkan = vulkan_presenter((start.w, start.h));

    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            layer_map_for_output(&output).arrange();
            state.refresh_area();
        }
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Redraw => {
            let size = backend.window_size();
            if let Some(vk) = vulkan.as_mut() {
                // the scene nodes come from wm-scene; until it lands the Vulkan frame is the clear colour
                match vk.element(backend.renderer(), &[], (size.w, size.h)) {
                    Ok(elements) => {
                        let Ok((renderer, mut fb)) = backend.bind() else { return };
                        if let Err(e) = damage.render_output(renderer, &mut fb, 0, &elements, CLEAR) {
                            eprintln!("bifrost-wm: render: {e}");
                        }
                    }
                    Err(e) => eprintln!("bifrost-wm: vulkan frame: {e}"),
                }
            } else {
                let elements = crate::scene::build(backend.renderer(), state, &output, &mut glass, Vec::new());
                let Ok((renderer, mut fb)) = backend.bind() else { return };
                if let Err(e) = damage.render_output(renderer, &mut fb, 0, &elements, CLEAR) {
                    eprintln!("bifrost-wm: render: {e}");
                }
            }
            let _ = backend.submit(Some(&[Rectangle::from_size(size)]));
            state.send_frames(&output);
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
