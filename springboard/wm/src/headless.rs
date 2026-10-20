//! A headless backend: renders the output offscreen on a GPU render node, with no window on any screen. Tests and
//! screenshots use it; `Compositor` runs it on its own thread and takes commands.

use std::{
    error::Error,
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use smithay::{
    backend::{
        allocator::Fourcc,
        egl::{EGLContext, EGLDevice, EGLDisplay},
        renderer::{
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexture},
            Bind, ExportMem, Offscreen,
        },
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            channel::{self, Event as ChannelEvent},
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        wayland_server::Display,
    },
    utils::{Rectangle, Transform},
};

use bifrost_gpu::{Device, LoadAction, PixelFormat, Region, Texture, TextureDescriptor, TextureUsage};

use crate::{
    render_server::{
        compose::{DamageRects, SceneNode},
        server::Server,
    },
    session_protocols::CaptureSpec,
    state::WmState,
    window_capture::WindowCaptureSpec,
};

/// The desktop colour where nothing is drawn (the wallpaper is a background layer surface).
const CLEAR: [f32; 4] = [0.06, 0.06, 0.08, 1.0];
const FRAME: Duration = Duration::from_millis(16);

pub struct Headless {
    renderer: GlesRenderer,
    glass: crate::glass::Glass,
    target: GlesTexture,
    output: Output,
    damage: OutputDamageTracker,
    size: (i32, i32),
}

impl Headless {
    /// A `w` x `h` output on the first hardware render node (a software device if there is none).
    pub fn new(state: &mut WmState, w: i32, h: i32) -> Result<Self, Box<dyn Error>> {
        let devices: Vec<EGLDevice> = EGLDevice::enumerate()?.collect();
        let device = devices.iter().find(|d| !d.is_software()).or(devices.first()).cloned().ok_or("no EGL device")?;
        // SAFETY: the display and context live as long as the renderer that owns them.
        let display = unsafe { EGLDisplay::new(device)? };
        let context = EGLContext::new(&display)?;
        let mut renderer = unsafe { GlesRenderer::new(context)? };
        let target: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (w, h).into())?;

        let output = Output::new(
            "HEADLESS-1".into(),
            PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "BifrOSt".into(), model: "Headless".into() },
        );
        let _global = output.create_global::<WmState>(&state.dh);
        let mode = Mode { size: (w, h).into(), refresh: 60_000 };
        output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
        output.set_preferred(mode);
        state.add_output(&output);
        let damage = OutputDamageTracker::from_output(&output);
        let glass = crate::glass::Glass::new(&mut renderer)?;
        Ok(Headless { renderer, glass, target, output, damage, size: (w, h) })
    }

    /// Composites one frame and sends the clients their frame callbacks.
    pub fn render(&mut self, state: &mut WmState) -> Result<(), Box<dyn Error>> {
        let elements = crate::scene::build(&mut self.renderer, state, &self.output, &mut self.glass, Vec::new());
        {
            let mut fb = self.renderer.bind(&mut self.target)?;
            self.damage.render_output(&mut self.renderer, &mut fb, 0, &elements, CLEAR)?;
        }
        state.send_frames(&self.output);
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.dh.flush_clients();
        Ok(())
    }

    /// The last composited frame as RGBA rows, top first.
    pub fn pixels(&mut self) -> Result<Vec<u8>, Box<dyn Error>> {
        let (w, h) = self.size;
        let fb = self.renderer.bind(&mut self.target)?;
        let mapping = self.renderer.copy_framebuffer(&fb, Rectangle::from_size((w, h).into()), Fourcc::Abgr8888)?;
        drop(fb);
        Ok(self.renderer.map_texture(&mapping)?.to_vec())
    }

    pub fn size(&self) -> (i32, i32) { self.size }

    /// A capture (`SessionHooks::capture`): the region as RGBA rows over `clear`, `None` for another output or a failure.
    pub fn capture(&mut self, state: &WmState, spec: &CaptureSpec, clear: [f32; 4]) -> Option<Vec<u8>> {
        if spec.output != self.output.name() {
            return None;
        }
        // the headless output has no pointer image, so `spec.cursor` paints nothing
        match crate::capture::capture_scene(&mut self.renderer, &mut self.glass, state, &self.output, Vec::new(), clear, spec) {
            Ok(px) => Some(px),
            Err(e) => {
                eprintln!("bifrost-wm: capture: {e}");
                None
            }
        }
    }

    /// A window capture (`SessionHooks::capture_window`): the window alone at `spec.size` as RGBA rows over a
    /// transparent ground, `None` for an unknown window or a failure.
    pub fn capture_window(&mut self, state: &WmState, spec: &WindowCaptureSpec) -> Option<Vec<u8>> {
        let window = state.window_any(spec.id)?;
        match crate::capture::capture_window(&mut self.renderer, &self.glass, state, &window, spec.size) {
            Ok(px) => Some(px),
            Err(e) => {
                eprintln!("bifrost-wm: window capture: {e}");
                None
            }
        }
    }

    pub fn screenshot(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let (w, h) = self.size;
        let px = self.pixels()?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&px)?;
        Ok(())
    }
}

type Command = Box<dyn FnOnce(&mut WmState, &mut Headless) + Send>;

/// A headless compositor on its own thread. `run` executes a closure inside its event loop and returns the result.
pub struct Compositor {
    commands: channel::Sender<Command>,
    pub socket: String,
    thread: Option<thread::JoinHandle<()>>,
    stopped: Receiver<()>,
}

impl Compositor {
    /// Starts listening on `socket` in `runtime_dir` with a `w` x `h` output.
    pub fn start(socket: &str, w: i32, h: i32) -> Result<Self, Box<dyn Error>> {
        let (tx, rx) = channel::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let name = socket.to_string();
        let thread = thread::spawn(move || {
            let run = || -> Result<(), Box<dyn Error>> {
                let mut event_loop: EventLoop<'static, WmState> = EventLoop::try_new()?;
                let display: Display<WmState> = Display::new()?;
                let mut state = WmState::new(&mut event_loop, display, Some(&name));
                let backend = std::rc::Rc::new(std::cell::RefCell::new(Headless::new(&mut state, w, h)?));
                // Unlike the DRM backend, the offscreen backend has no initial scanout to produce. Start idle;
                // accepted client requests mark the scene dirty and drive the first frame.
                state.dirty = false;
                let b = backend.clone();
                state.session.hooks.capture = Some(Box::new(move |state, spec| b.borrow_mut().capture(state, spec, CLEAR)));
                let b = backend.clone();
                state.session.hooks.capture_window = Some(Box::new(move |state, spec| b.borrow_mut().capture_window(state, spec)));
                let b = backend.clone();
                event_loop.handle().insert_source(rx, move |event, _, state| {
                    if let ChannelEvent::Msg(cmd) = event {
                        cmd(state, &mut b.borrow_mut());
                    }
                })?;
                let b = backend.clone();
                event_loop.handle().insert_source(Timer::from_duration(FRAME), move |_, _, state| {
                    if state.dirty {
                        state.dirty = false;
                        if let Err(e) = b.borrow_mut().render(state) {
                            state.dirty = true;
                            eprintln!("bifrost-wm: render: {e}");
                        }
                    }
                    TimeoutAction::ToDuration(FRAME)
                })?;
                ready_tx.send(Ok(state.socket_name.to_string_lossy().into_owned())).ok();
                event_loop.run(None, &mut state, |_| {})?;
                Ok(())
            };
            if let Err(e) = run() {
                ready_tx.send(Err(e.to_string())).ok();
            }
            let _ = stopped_tx.send(());
        });
        // the first EGL start on a machine compiles its shader cache: allow it a minute
        let socket = match ready_rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Ok(socket)) => socket,
            Ok(Err(error)) => return Err(error.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "headless compositor did not become ready within 60 seconds").into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "headless compositor exited before signaling readiness").into());
            }
        };
        Ok(Compositor { commands: tx, socket, thread: Some(thread), stopped: stopped_rx })
    }

    pub fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut WmState, &mut Headless) -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        if self.commands.send(Box::new(move |s, b| { tx.send(f(s, b)).ok(); })).is_err() {
            panic!("headless compositor event loop exited before accepting command");
        }
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("headless compositor did not answer command within 20 seconds"),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("headless compositor event loop exited before answering command"),
        }
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.commands.send(Box::new(|s, _| s.loop_signal.stop()));
        match self.stopped.recv_timeout(Duration::from_secs(5)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // A wedged renderer must not turn a bounded compositor test into a process-wide hang. Dropping
                // the JoinHandle detaches only this compositor worker; its socket is unique to its test/run.
                eprintln!("bifrost-wm: headless worker did not stop within 5 seconds; detaching it");
                drop(self.thread.take());
            }
        }
    }
}

/// The premultiplied clear colour of the Vulkan path (the GLES path's `CLEAR` in 0..1 RGBA).
const CLEAR_VK: [f32; 4] = [0.06, 0.06, 0.08, 1.0];

/// The Vulkan headless path (wave G2 `wm-nested-headless`): one `Server` renders into an offscreen BGRA target and the
/// frame is read back to CPU pixels, for the end-to-end tests (`tests/vulkan_*.rs`, behind `gpu-tests`) and for
/// screenshots. Scene nodes come from the caller; building them from the window stack is `wm-scene`'s work.
pub struct VulkanHeadless {
    pub server: Server,
    target: Texture,
    size: (u32, u32),
}

impl VulkanHeadless {
    /// A `w` x `h` target on the first render node the Vulkan loader offers.
    pub fn new(w: u32, h: u32) -> bifrost_gpu::Result<VulkanHeadless> {
        let server = Server::new(None, PixelFormat::Bgra8Unorm)?;
        let target = new_target(&server.device, w, h)?;
        Ok(VulkanHeadless { server, target, size: (w, h) })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Reallocates the target when the output size changes; a no-op for the same size.
    pub fn resize(&mut self, w: u32, h: u32) -> bifrost_gpu::Result<()> {
        if (w, h) != self.size {
            self.target = new_target(&self.server.device, w, h)?;
            self.size = (w, h);
        }
        Ok(())
    }

    /// Composites `nodes` (back to front) over the clear colour, redrawing `damage` only, waits for the GPU and returns
    /// the whole target as BGRA rows, top first.
    pub fn render(&mut self, nodes: &[SceneNode], damage: &DamageRects) -> bifrost_gpu::Result<Vec<u8>> {
        let (w, h) = self.size;
        self.server.frame(nodes, &self.target, damage, LoadAction::Clear(CLEAR_VK))?.wait()?;
        self.target.read_region(Region { x: 0, y: 0, width: w, height: h })
    }
}

fn new_target(device: &Device, w: u32, h: u32) -> bifrost_gpu::Result<Texture> {
    device.new_texture(&TextureDescriptor {
        width: w,
        height: h,
        format: PixelFormat::Bgra8Unorm,
        usage: TextureUsage::RENDER_TARGET | TextureUsage::SAMPLED | TextureUsage::COPY_SRC,
        exportable: false,
        modifiers: vec![],
    })
}

/// Converts BGRA rows (memory order B, G, R, A) to RGBA rows, one 4-byte pixel at a time.
pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

/// The RGBA value of pixel `(x, y)` in a `w` x `h` RGBA buffer, or `None` outside the buffer.
pub fn rgba_at(rgba: &[u8], w: u32, h: u32, x: u32, y: u32) -> Option<[u8; 4]> {
    if x >= w || y >= h || rgba.len() < w as usize * h as usize * 4 {
        return None;
    }
    let i = (y as usize * w as usize + x as usize) * 4;
    Some([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
}

/// Damage covering the whole `w` x `h` output, in the form `Server::frame` takes.
pub fn full_damage(w: u32, h: u32) -> DamageRects {
    vec![[0, 0, w as i32, h as i32]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_swaps_red_and_blue_and_keeps_alpha() {
        // a red pixel (B=0, G=0, R=255, A=255) and a half-transparent cyan (B=255, G=255, R=0, A=128)
        assert_eq!(bgra_to_rgba(&[0, 0, 255, 255, 255, 255, 0, 128]), vec![255, 0, 0, 255, 0, 255, 255, 128]);
    }

    #[test]
    fn bgra_conversion_of_an_empty_buffer_is_empty() {
        assert!(bgra_to_rgba(&[]).is_empty());
    }

    #[test]
    fn bgra_conversion_drops_a_trailing_partial_pixel() {
        assert_eq!(bgra_to_rgba(&[1, 2, 3, 4, 5, 6]), vec![3, 2, 1, 4]);
    }

    #[test]
    fn rgba_at_reads_the_row_major_offset() {
        // 3 x 2 image; pixel (2, 1) is the last one: index (1 * 3 + 2) * 4 = 20
        let mut img = vec![0u8; 3 * 2 * 4];
        img[20..24].copy_from_slice(&[9, 8, 7, 6]);
        assert_eq!(rgba_at(&img, 3, 2, 2, 1), Some([9, 8, 7, 6]));
        assert_eq!(rgba_at(&img, 3, 2, 0, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn rgba_at_refuses_coordinates_outside_the_image() {
        let img = vec![0u8; 3 * 2 * 4];
        assert_eq!(rgba_at(&img, 3, 2, 3, 0), None);
        assert_eq!(rgba_at(&img, 3, 2, 0, 2), None);
        assert_eq!(rgba_at(&img[..10], 3, 2, 0, 0), None, "a short buffer is refused");
    }

    #[test]
    fn full_damage_is_the_whole_output_as_one_rectangle() {
        assert_eq!(full_damage(1280, 800), vec![[0, 0, 1280, 800]]);
    }
}
