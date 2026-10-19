//! Select Servo's public hardware or software context and own its native lifetimes.
use super::*;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop as NativeEventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowId};

/// Field order matters: release the GL context before its native window/display.
pub(super) struct Graphics {
    context: Rc<dyn RenderingContext>,
    native: Option<NativeGraphics>,
    mode: &'static str,
    driver: String,
}

struct NativeGraphics {
    app: HiddenWindow,
    event_loop: NativeEventLoop<()>,
}

#[derive(Default)]
struct HiddenWindow {
    window: Option<Window>,
    error: Option<String>,
}

impl ApplicationHandler for HiddenWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || self.error.is_some() { return; }
        let attributes = Window::default_attributes()
            .with_title("Bifrost Browser GPU context")
            .with_visible(false)
            .with_decorations(false)
            .with_inner_size(PhysicalSize::new(1, 1));
        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(error) => self.error = Some(format!("GPU context window: {error}")),
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window: WindowId, _event: WindowEvent) {
        // This window only owns the native GL surface. Servo paints to its offscreen FBO;
        // no native redraw, resize, input or buffer presentation is used.
    }
}

impl Graphics {
    pub(super) fn new(size: PhysicalSize<u32>) -> Result<Self, String> {
        match std::env::var("BIFROST_BROWSER_RENDERER").as_deref() {
            // Default: the GPU, and the CPU when the GPU cannot start (no `gpu` permission, no device). Naming the
            // renderer explicitly never falls back: `gpu` fails loudly.
            Err(std::env::VarError::NotPresent) => Self::gpu(size).or_else(|error| {
                eprintln!("browser: GPU renderer unavailable ({error}); falling back to the CPU renderer");
                Self::cpu(size)
            }),
            Ok("gpu") => Self::gpu(size),
            Ok("cpu") => Self::cpu(size),
            Ok(value) => Err(format!("invalid BIFROST_BROWSER_RENDERER={value:?}; expected gpu or cpu")),
            Err(error) => Err(format!("BIFROST_BROWSER_RENDERER: {error}")),
        }
    }

    fn cpu(size: PhysicalSize<u32>) -> Result<Self, String> {
        let context: Rc<dyn RenderingContext> = Rc::new(SoftwareRenderingContext::new(size)
            .map_err(|error| format!("CPU rendering context: {error:?}"))?);
        Self::finish(context, None, "cpu")
    }

    fn gpu(size: PhysicalSize<u32>) -> Result<Self, String> {
        let event_loop = NativeEventLoop::new().map_err(|error| format!("GPU display event loop: {error}"))?;
        let mut native = NativeGraphics { app: HiddenWindow::default(), event_loop };
        native.event_loop.pump_app_events(Some(Duration::ZERO), &mut native.app);
        if let Some(error) = native.app.error.take() { return Err(error); }
        let window = native.app.window.as_ref().ok_or("GPU context window was not created")?;
        let display = window.display_handle().map_err(|error| format!("GPU display handle: {error}"))?;
        let handle = window.window_handle().map_err(|error| format!("GPU window handle: {error}"))?;
        let parent = Rc::new(servo::WindowRenderingContext::new(display, handle, PhysicalSize::new(1, 1))
            .map_err(|error| format!("GPU rendering context: {error:?}"))?);
        // OffscreenRenderingContext retains the parent hardware context. Never swap its native
        // window buffers: readback from the FBO supplies the existing SDK shared-memory frames.
        let context: Rc<dyn RenderingContext> = Rc::new(parent.offscreen_context(size));
        drop(parent);
        Self::finish(context, Some(native), "gpu")
    }

    fn finish(context: Rc<dyn RenderingContext>, native: Option<NativeGraphics>, mode: &'static str) -> Result<Self, String> {
        if let Err(error) = context.make_current() {
            drop(context);
            drop(native);
            return Err(format!("{mode} make current: {error:?}"));
        }
        let driver = context.gleam_gl_api().get_string(gleam::gl::RENDERER);
        if mode == "gpu" && (driver.is_empty() || is_software_driver(&driver)) {
            drop(context);
            drop(native);
            return Err(format!("GPU requested but OpenGL selected unavailable/software renderer {driver:?}; select CPU fallback explicitly"));
        }
        eprintln!("bifrost-webview renderer={mode} driver={driver:?}");
        Ok(Self { context, native, mode, driver })
    }

    pub(super) fn context(&self) -> Rc<dyn RenderingContext> { self.context.clone() }
    pub(super) fn mode(&self) -> &'static str { self.mode }
    pub(super) fn driver(&self) -> &str { &self.driver }

    pub(super) fn pump(&mut self) -> Result<(), String> {
        let Some(native) = &mut self.native else { return Ok(()) };
        match native.event_loop.pump_app_events(Some(Duration::ZERO), &mut native.app) {
            PumpStatus::Continue => Ok(()),
            PumpStatus::Exit(code) => Err(format!("GPU display event loop exited ({code})")),
        }
    }
}

fn is_software_driver(driver: &str) -> bool {
    let driver = driver.to_ascii_lowercase();
    ["llvmpipe", "softpipe", "swrast", "swiftshader", "software rasterizer", "software renderer"]
        .iter().any(|name| driver.contains(name))
}
