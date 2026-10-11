//! A headless backend: renders the output offscreen on a GPU render node, with no window on any screen. Tests and
//! screenshots use it; `Compositor` runs it on its own thread and takes commands.

use std::{
    error::Error,
    path::Path,
    sync::mpsc,
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
    desktop::layer_map_for_output,
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

use crate::state::WmState;

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
        let elapsed = state.start.elapsed();
        let output = self.output.clone();
        for window in state.space.elements() {
            window.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        for layer in layer_map_for_output(&output).layers() {
            layer.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
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
}

impl Compositor {
    /// Starts listening on `socket` in `runtime_dir` with a `w` x `h` output.
    pub fn start(socket: &str, w: i32, h: i32) -> Result<Self, Box<dyn Error>> {
        let (tx, rx) = channel::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
        let name = socket.to_string();
        let thread = thread::spawn(move || {
            let run = || -> Result<(), Box<dyn Error>> {
                let mut event_loop: EventLoop<'static, WmState> = EventLoop::try_new()?;
                let display: Display<WmState> = Display::new()?;
                let mut state = WmState::new(&mut event_loop, display, Some(&name));
                let backend = std::rc::Rc::new(std::cell::RefCell::new(Headless::new(&mut state, w, h)?));
                let b = backend.clone();
                event_loop.handle().insert_source(rx, move |event, _, state| {
                    if let ChannelEvent::Msg(cmd) = event {
                        cmd(state, &mut b.borrow_mut());
                    }
                })?;
                let b = backend.clone();
                event_loop.handle().insert_source(Timer::from_duration(FRAME), move |_, _, state| {
                    if let Err(e) = b.borrow_mut().render(state) {
                        eprintln!("bifrost-wm: render: {e}");
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
        });
        let socket = ready_rx.recv_timeout(Duration::from_secs(20))??;
        Ok(Compositor { commands: tx, socket, thread: Some(thread) })
    }

    pub fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut WmState, &mut Headless) -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        self.commands.send(Box::new(move |s, b| { tx.send(f(s, b)).ok(); })).expect("compositor thread");
        rx.recv_timeout(Duration::from_secs(20)).expect("compositor answered")
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.commands.send(Box::new(|s, _| s.loop_signal.stop()));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
