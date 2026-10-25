//! BifrOSt's native Wayland display/input backend. The compositor owns KMS and
//! evdev; external apps submit their GPU buffers directly to it. Only the shell
//! raster uses SHM, with damage tracking and released-buffer ownership.
use std::{io, os::fd::AsFd};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, Proxy};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_output, wl_region, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1 as layer, zwlr_layer_surface_v1 as surface};
use crate::gfx::Damage;
use wayland_protocols::xdg::shell::client::{xdg_wm_base, xdg_surface, xdg_toplevel};
use std::collections::{HashMap,HashSet};
use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notifier_v1 as idle_notifier, ext_idle_notification_v1 as idle_notification};
use wayland_protocols_wlr::output_power_management::v1::client::{zwlr_output_power_manager_v1 as power_manager,zwlr_output_power_v1 as output_power};
use wayland_protocols_wlr::output_management::v1::client::{zwlr_output_manager_v1 as om_manager, zwlr_output_head_v1 as om_head, zwlr_output_mode_v1 as om_mode, zwlr_output_configuration_v1 as om_config, zwlr_output_configuration_head_v1 as om_config_head};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_v1 as activation,xdg_activation_token_v1 as activation_token};
use wayland_protocols::ext::session_lock::v1::client::{ext_session_lock_manager_v1 as lock_manager,ext_session_lock_v1 as lock,ext_session_lock_surface_v1 as lock_surface};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{zwlr_foreign_toplevel_manager_v1 as foreign_manager, zwlr_foreign_toplevel_handle_v1 as foreign_handle};
use wayland_protocols::ext::background_effect::v1::client::{ext_background_effect_manager_v1 as bg_manager, ext_background_effect_surface_v1 as bg_surface};
#[derive(Clone, PartialEq, Eq)]
/// `minimized`: the window server reports the toplevel minimized (Show Desktop leaves such a window where it is).
pub struct ForeignWindow { pub id:u32, pub title:String, pub app_id:String, pub active:bool,pub maximized:bool,pub minimized:bool }
struct Foreign { handle:foreign_handle::ZwlrForeignToplevelHandleV1, info:ForeignWindow }
struct Lock { obj:lock::ExtSessionLockV1, surface:wl_surface::WlSurface, role:lock_surface::ExtSessionLockSurfaceV1, configured:bool, locked:bool }


/// A chrome surface's pixels at screen (x, y). `input` limits the pointer to a part of the surface (x, y, w, h in
/// surface px) when the surface is larger than what takes input: a popup is exported with room for its shadow, and a
/// click on the shadow must reach what is below. `None` takes the whole surface.
pub struct ChromeFrame {pub id:u32,pub x:i32,pub y:i32,pub width:usize,pub height:usize,pub pixels:Vec<u32>,pub input:Option<[i32;4]>,pub dock:Option<DockPlacement>}
/// The Dock's edge and reserved depth, independent of the transparent room for its icons, labels and shadow.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct DockPlacement {pub edge:DockEdge,pub reserve:i32}
struct ChromeWindow {surface:wl_surface::WlSurface,layer:surface::ZwlrLayerSurfaceV1,slots:Vec<Slot>,configured:bool,frame:ChromeFrame,pending:Damage}
#[derive(Clone)]
pub struct WindowFrame { pub id:u32, pub title:&'static str, pub x:i32, pub y:i32, pub width:usize, pub height:usize, pub pixels:Option<std::sync::Arc<[u32]>>, pub damage:Damage, pub maximized:bool, pub floating:bool, pub title_height:i32,
    /// The window is full screen: the window server puts it on the whole output (xdg_toplevel set_fullscreen).
    pub fullscreen:bool,
    /// Where a toplevel created for this frame maps (`bifrost_window_level_v1` `set_origin`): the position of a window the
    /// window server placed before and whose toplevel was destroyed while it was hidden. None lets the window server
    /// place it (cascade). Used only when the toplevel is created.
    pub origin:Option<(i32,i32)>,
    /// A press above `drag_height` (window px) moves the window, unless it lands in one of `no_drag` (x, y, w, h):
    /// the traffic lights and an app's toolbar controls in a full-size-content window.
    pub drag_height:i32, pub no_drag:Vec<[i32;4]>,
    /// Open, close, minimize and zoom transform of the window (identity at rest), applied by the GPU presenter.
    pub fx:bifrost_render::glass::Fx,
    /// The content is opaque inside its corner radius (a shell-drawn window without vibrancy). An opaque window
    /// that covers another one lets the presenter skip drawing the hidden one (`gpu::occlusion`).
    pub opaque:bool,
    /// The smallest outer size (window px, title bar included) the window may be resized to: the app's own minimum
    /// (`desk::size_range`, Settings' split width), advertised with xdg_toplevel `set_min_size` so the window server's
    /// interactive resize stops where the shell's does. (0, 0) falls back to `DEFAULT_MIN_SIZE`.
    pub min_size:(i32,i32),
    /// The parts of a shaped window that take the pointer (x, y, w, h in window px): a borderless window's drawn pixels,
    /// so a click on its transparent rest reaches the window below (`desk::alpha_shape`). A shaped window draws its own
    /// edges and gets no shell shadow. `None`: the whole window takes input.
    pub input:Option<std::sync::Arc<[[i32;4]]>> }
/// The minimum a toplevel advertises when its frame names none.
const DEFAULT_MIN_SIZE:(i32,i32)=(320,220);
/// The `set_min_size` a toplevel sends for `min`: the frame's own minimum, or the default when it has none.
fn min_size_of(min:(i32,i32)) -> (i32,i32) { if min.0>0 && min.1>0 { min } else { DEFAULT_MIN_SIZE } }
/// What a shell window's toplevel reported. `Frame` is the window server's own frame of the toplevel (x, y, width,
/// height in desktop px, `bifrost_window_level_v1` version 2): the window server places and moves toplevels, so this
/// is where the window really is.
pub enum WindowEvent { Focus(u32), Resize(u32,i32,i32), Close(u32), Maximized(u32,bool), Frame(u32,i32,i32,i32,i32),
    /// The window server's full-screen state of the toplevel, when it differs from what the shell last asked for (the
    /// window server's own keys, or a refusal).
    Fullscreen(u32,bool) }
struct Window { surface:wl_surface::WlSurface, level_sent:Option<bool>, xdg:xdg_surface::XdgSurface, top:xdg_toplevel::XdgToplevel, slots:Vec<Slot>, buffer_size:(usize,usize), configured:bool, frame:WindowFrame,
    /// The full-screen state asked of the window server and not yet answered by a configure: a configure sent before
    /// the request reached it carries the old state and must not undo the request.
    fs_pending:Option<bool>,
    /// Whether the last toplevel configure carried the Activated state, and whether the shell was told about it.
    act:bool, act_seen:bool,
    /// A toplevel that has not had the keyboard yet. The window server now maps the shell's windows behind the front one
    /// without the keyboard and activates them only when the shell asks (`wm` `map_takes_front`); this guard stays so a
    /// first focus of a window the shell did not put in front is still never taken as the person's choice (W4-1).
    fresh:bool,
    /// GPU path: content damage not presented yet, the shadow margin around the content, the drawable size
    pending:Damage, margin:i32, gpu_size:(usize,usize),
    /// window-server materials (ext-background-effect): the effect object and the blur rects last sent (surface px)
    bg:Option<bg_surface::ExtBackgroundEffectSurfaceV1>, bg_rects:Vec<[i32;4]>,
    /// bifrost_material_v1 (when the compositor offers it): the object and the margin and regions last sent
    mat:Option<mat_surface::BifrostMaterialSurfaceV1>, mat_sent:(i32,Vec<Glass>) }
/// Golden Gate window shadow reach, pt: the key window's shadow falls ~80 pt below and ~40 pt to the sides and
/// top (the reference's bottom shadow runs past 74 pt). The top/side reach is also the content's offset in the
/// drawable, so pointer coordinates subtract `Window::margin`.
const SHADOW_SIDE_PT: f32 = 40.0;
const SHADOW_BOTTOM_PT: f32 = 80.0;
/// Fallback points-per-px divisor when the shell has not called `set_scale`: desk.rs draws a 30 pt title bar.
const TITLE_PT: f32 = 30.0;
/// Presenter keys of built-in app windows (chrome ids are small, the background is u32::MAX).
const GPU_WINDOW: u32 = 0x1000_0000;
use super::input::Event;
use super::wl_touch::{self as touch_model, Placement};
use crate::shell::textinput::{ImBatch, ImOut};
use wayland_protocols_misc::zwp_input_method_v2::client::{zwp_input_method_manager_v2 as im_manager, zwp_input_method_v2 as im};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1 as vk_manager, zwp_virtual_keyboard_v1 as vk};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_manager_v1 as cursor_manager, wp_cursor_shape_device_v1 as cursor_device};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{zwp_linux_dmabuf_v1 as dmabuf, zwp_linux_buffer_params_v1 as dmabuf_params};
use crate::gpu::wl::{Backdrop, BufferInfo, Magnify, Presenter};
use crate::shell::accessibility::zoom::{self as zoom_model, Mode, ScreenZoom};
use bifrost_render::glass::Rect as Glass;
use super::material_client::{args as material_args, protocol::{bifrost_material_manager_v1 as mat_manager, bifrost_material_surface_v1 as mat_surface}};
use super::window_level_client::protocol::bifrost_window_level_manager_v1 as lvl_manager;
use super::window_capture_client::protocol::{bifrost_window_capture_manager_v1 as cap_manager, bifrost_window_capture_frame_v1 as cap_frame};
use crate::shell::window_shots::{Outcome as ShotOutcome, Request as ShotRequest, Source as ShotSource};
/// The presenter's key for the desktop background (and the lock surface, which shares its drawables).
const GPU_BG: u32 = u32::MAX;
/// A dma-buf `wl_buffer` around one presenter drawable.
struct GpuSlot { buffer: wl_buffer::WlBuffer, busy: bool }

fn err(e: impl std::fmt::Display) -> io::Error { io::Error::other(e.to_string()) }
/// The Wayland socket is non-blocking: `WouldBlock` means nothing to read yet, or a full send buffer whose queued
/// requests go out with the next flush. Only other errors are a broken connection (a busy compositor's momentarily
/// full socket ended the session under bifrost-wm).
fn transient<T>(r: Result<T, wayland_client::backend::WaylandError>) -> io::Result<()> {
    match r {
        Ok(_) => Ok(()),
        Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(e) => Err(err(e)),
    }
}
struct Slot { buffer: wl_buffer::WlBuffer, map: memmap2::MmapMut, busy: bool, damage: Damage }
/// A window capture in flight (`bifrost_window_capture_v1`): the buffer the window server fills and the frame object that
/// answers. The buffer is as large as the biggest picture wanted; the picture comes back in its top-left corner.
struct CapJob { buffer:wl_buffer::WlBuffer, frame:cap_frame::BifrostWindowCaptureFrameV1, map:memmap2::MmapMut, stride:usize }
/// At most this many captures are out at once.
const MAX_CAPTURES_IN_FLIGHT: usize = 64;
#[derive(Default)]
struct State {
    /// The window server's output management (mode requests from the shell), its first head, the last serial it sent,
    /// and whether a configuration is in flight.
    output_manager:Option<om_manager::ZwlrOutputManagerV1>, output_head:Option<om_head::ZwlrOutputHeadV1>, output_serial:u32, output_busy:bool,
    bg_manager:Option<bg_manager::ExtBackgroundEffectManagerV1>, bg_blur:bool, mat_manager:Option<mat_manager::BifrostMaterialManagerV1>, level_mgr:Option<lvl_manager::BifrostWindowLevelManagerV1>,
    /// the window server applies Night Shift to the whole output (`white_point_mode` not none), and the white point it was
    /// last sent (None: nothing sent yet)
    wm_white_point:bool, white_point_sent:Option<Option<[u32;3]>>,
    /// the Screen Zoom factor last sent to the window server (`set_zoom`, 256 = 1.0)
    zoom_sent:Option<u32>,
    foreign_manager:Option<foreign_manager::ZwlrForeignToplevelManagerV1>, foreign:Vec<Foreign>, foreign_dirty:bool,
    /// window capture (the global is listed on the privileged socket only): the manager, the requests in flight by token, the answers
    cap_mgr:Option<cap_manager::BifrostWindowCaptureManagerV1>, cap_jobs:HashMap<u64,CapJob>, cap_results:Vec<(u64,ShotOutcome)>,
    cursor_manager: Option<cursor_manager::WpCursorShapeManagerV1>, cursor_device: Option<cursor_device::WpCursorShapeDeviceV1>, pointer: Option<wl_pointer::WlPointer>, cursor_serial:u32,
    /// the seat's keyboard: one object only, or every capability change would add one and each key would arrive again
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// the seat's touch device (bound when the seat reports touch) and the fingers on it
    touch: Option<wl_touch::WlTouch>, fingers: touch_model::Translator,
    lock_manager:Option<lock_manager::ExtSessionLockManagerV1>, output:Option<wl_output::WlOutput>, lock:Option<Lock>, lock_wanted:bool, redraw:bool, refresh:u32,
    idle_notifier:Option<idle_notifier::ExtIdleNotifierV1>, idle:bool, resumed:bool, keys:HashSet<u16>, power_manager:Option<power_manager::ZwlrOutputPowerManagerV1>, output_power:Option<output_power::ZwlrOutputPowerV1>,
    activation:Option<activation::XdgActivationV1>, focused:Option<u32>, serial:u32, raise_front:bool, z:Vec<u32>,
    /// a fresh toplevel the window server gave the keyboard while another window is the shell's front one: (front,
    /// entered). The front window takes the keyboard back once the events read with it are handled, unless a press on
    /// the entered window came with them (the person clicked it: E2E N2)
    refocus:Option<(u32,u32)>,
    wm:Option<xdg_wm_base::XdgWmBase>, seat:Option<wl_seat::WlSeat>, windows:HashMap<u32,Window>, materials:HashMap<u32,Vec<Glass>>, scale:f32, window_events:Vec<WindowEvent>, target:Option<u32>,
    compositor: Option<wl_compositor::WlCompositor>, shm: Option<wl_shm::WlShm>,
    layer: Option<layer::ZwlrLayerShellV1>, surface: Option<wl_surface::WlSurface>,
    chrome:HashMap<u32,ChromeWindow>, chrome_target:Option<u32>, chrome_configured:bool,
    slots: Vec<Slot>, size: (u32,u32), configured: bool, closed: bool, starved: Starved,
    pending: bool, desktop_flip: Option<f64>, inputs: Vec<Event>, xy: (f32,f32), pressed: bool,
    dmabuf: Option<dmabuf::ZwpLinuxDmabufV1>, dmabuf_formats: Vec<(u32,u64)>, gpu_slots: HashMap<u32,Vec<GpuSlot>>,
    /// the input method (the keyboard types into the focused client's field) and the virtual keyboard (named keys);
    /// the batch being received until its `done`, the completed batches, and the serial the next commit carries
    im_manager: Option<im_manager::ZwpInputMethodManagerV2>, im: Option<im::ZwpInputMethodV2>, im_pending: ImBatch, im_batches: Vec<ImBatch>, im_serial: u32,
    vk_manager: Option<vk_manager::ZwpVirtualKeyboardManagerV1>, vk: Option<vk::ZwpVirtualKeyboardV1>,
}
/// The GPU presenter and what it needs between frames.
struct Gpu { p: Presenter, glass: Vec<Glass>, log: (std::time::Instant, usize, f64, f64, usize), draw: (usize, f64), last_frame: std::time::Instant, zoom: ScreenZoom, pointer: (f32, f32), zoomed: bool,
    /// when the Night Shift fade last stepped (`night`)
    night_at: std::time::Instant }

/// The Screen Zoom pass for a `w` x `h` screen with the pointer at `pointer`, in the presenter's terms: the Full
/// Screen source rect, or the Picture in Picture lens and the part of the frame it shows. None while zoom is off.
fn magnify_for(z: ScreenZoom, pointer: (f32, f32), (w, h): (usize, usize)) -> Option<Magnify> {
    let screen = (w as f32, h as f32);
    let source = z.source_rect(pointer, screen)?;
    let uv = zoom_model::uv(source, screen);
    Some(match z.mode {
        Mode::Full => Magnify { full: Some(uv), lens: None },
        Mode::Lens => Magnify { full: None, lens: z.lens_rect(pointer, screen).map(|rect| (rect, uv)) },
    })
}
pub struct Display { conn: Connection, queue: EventQueue<State>, state: State,
    stats: Option<(std::time::Instant,usize,usize)>, gpu: Option<Gpu>,
    /// the GPU presenter gave up and the SHM path took over: the shell must draw a full frame (`take_gpu_fallback`)
    fell_back: bool,
}
/// Uploads the named-key keymap to the virtual keyboard: an anonymous file holding the NUL-terminated xkb text.
fn send_keymap(keyboard: &vk::ZwpVirtualKeyboardV1) -> io::Result<()> {
    use std::io::Write;
    let map = crate::shell::textinput::virtual_keymap();
    let mut file = tempfile::tempfile()?;
    file.write_all(map.as_bytes())?;
    file.write_all(&[0])?;
    keyboard.keymap(1, file.as_fd(), (map.len() + 1) as u32);
    Ok(())
}
/// The event time the virtual keyboard's key events carry: milliseconds of the wall clock, truncated.
fn event_time() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u32)
}
/// Wraps presenter drawables in `wl_buffer`s; fails when the compositor does not list the format.
fn gpu_buffers(state:&State, infos:&[BufferInfo], qh:&QueueHandle<State>) -> io::Result<Vec<GpuSlot>> {
    let d = state.dmabuf.as_ref().ok_or_else(||err("linux-dmabuf unavailable"))?;
    let mut out = Vec::new();
    for i in infos {
        if !state.dmabuf_formats.iter().any(|&(f,m)| f==i.fourcc && m==i.modifier) {
            return Err(err(format!("compositor does not import format {:#x} modifier {:#x}", i.fourcc, i.modifier)));
        }
        let params = d.create_params(qh, ());
        // SAFETY: the presenter keeps the fd open for as long as the drawable (and this buffer) lives.
        params.add(unsafe { std::os::fd::BorrowedFd::borrow_raw(i.fd) }, 0, i.offset, i.stride, (i.modifier >> 32) as u32, i.modifier as u32);
        let buffer = params.create_immed(i.w as i32, i.h as i32, i.fourcc, dmabuf_params::Flags::empty(), qh, ());
        params.destroy();
        out.push(GpuSlot { buffer, busy: false });
    }
    Ok(out)
}
impl Display {
    pub fn open() -> io::Result<Self> {
        let conn = super::privileged::connect().map_err(err)?;
        let mut queue = conn.new_event_queue(); let qh = queue.handle();
        let mut state = State::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state).map_err(err)?;
        let seat=state.seat.as_ref().ok_or_else(||err("Wayland seat unavailable"))?;
        state.idle_notifier.as_ref().ok_or_else(||err("Wayland idle notifier unavailable"))?.get_idle_notification(1000,seat,&qh,());
        if let (Some(manager),Some(output))=(state.power_manager.as_ref(),state.output.as_ref()) {state.output_power=Some(manager.get_output_power(output,&qh,()));}
        // The shell is the window server's input method: a focused client's text field raises the on-screen keyboard,
        // and the keyboard types into it. The virtual keyboard sends the named keys. Without either protocol the
        // keyboard has nothing to type into, as before.
        if let Some(seat) = state.seat.as_ref() {
            if let Some(manager) = state.im_manager.as_ref() { state.im = Some(manager.get_input_method(seat, &qh, ())); }
            if let Some(manager) = state.vk_manager.as_ref() {
                let vk = manager.create_virtual_keyboard(seat, &qh, ());
                send_keymap(&vk)?;
                state.vk = Some(vk);
            }
        }
        let s = state.compositor.as_ref().ok_or_else(|| err("Wayland compositor unavailable"))?.create_surface(&qh, ());
        let ls = state.layer.as_ref().ok_or_else(|| err("wlr-layer-shell unavailable"))?.get_layer_surface(&s, None, layer::Layer::Background, "bifrost-desktop".into(), &qh, ());
        ls.set_size(0, 0);
        ls.set_anchor(surface::Anchor::Top | surface::Anchor::Bottom | surface::Anchor::Left | surface::Anchor::Right);
        ls.set_exclusive_zone(-1);
        ls.set_keyboard_interactivity(if std::env::var("BIFROST_SESSION_MODE").ok().as_deref()==Some("installer") {surface::KeyboardInteractivity::Exclusive} else {surface::KeyboardInteractivity::OnDemand});
        s.commit(); state.surface = Some(s);
        queue.roundtrip(&mut state).map_err(err)?;
        if !state.configured || state.size.0 == 0 || state.size.1 == 0 { return Err(err("Wayland output did not configure the desktop")); }
        let (w,h) = state.size;
        if w > 8192 || h > 8192 { return Err(err("Wayland output exceeds supported size")); }
        let len = w as usize * h as usize * 4;
        for _ in 0..2 {
            let file = tempfile::tempfile()?; file.set_len(len as u64)?;
            let map = unsafe { memmap2::MmapMut::map_mut(&file)? };
            let pool = state.shm.as_ref().ok_or_else(|| err("Wayland SHM unavailable"))?.create_pool(file.as_fd(), len as i32, &qh, ());
            let buffer = pool.create_buffer(0, w as i32, h as i32, w as i32 * 4, wl_shm::Format::Xrgb8888, &qh, ());
            pool.destroy();
            let mut damage = Damage::new(w as usize,h as usize); damage.all();
            state.slots.push(Slot { buffer, map, busy: false, damage });
        }
        transient(conn.flush())?;
        let stats = std::path::Path::new("/var/lib/springboard/render-stats").exists()
            .then(|| (std::time::Instant::now(),0,0));
        queue.roundtrip(&mut state).map_err(err)?;
        let gpu = if std::env::var("BIFROST_GPU").ok().as_deref() == Some("0") { None } else {
            match Presenter::open().and_then(|mut p| {
                let infos = p.create_surface(GPU_BG, w as usize, h as usize, true, GPU_POOL_MIN)?;
                let slots = gpu_buffers(&state, &infos, &qh)?;
                state.gpu_slots.insert(GPU_BG, slots);
                Ok(p)
            }) {
                Ok(p) => {
                    bifrost_core::log::shell_logger("presenter").notice(format!("Wayland dma-buf presenter on {} ({}x{}, {}..{} drawables)", p.renderer, w, h, GPU_POOL_MIN, GPU_POOL_MAX));
                    bifrost_render::glass::enable(true);
                    Some(Gpu { p, glass: Vec::new(), log: (std::time::Instant::now(), 0, 0., 0., 0), draw: (0, 0.), last_frame: std::time::Instant::now(), zoom: ScreenZoom::default(), pointer: (0.0, 0.0), zoomed: false, night_at: std::time::Instant::now() })
                }
                Err(e) => { bifrost_core::log::shell_logger("presenter").error(format!("presenter unavailable, SHM path: {e}")); state.gpu_slots.clear(); None }
            }
        };
        transient(conn.flush())?;
        Ok(Self { conn, queue, state, stats, gpu, fell_back: false })
    }
    pub fn active(&self) -> bool {!self.state.idle}
    pub fn resumed(&mut self) -> bool {std::mem::take(&mut self.state.resumed)}
    pub fn power(&mut self,on:bool) -> io::Result<()> {
        if let Some(power)=&self.state.output_power {power.set_mode(if on {output_power::Mode::On} else {output_power::Mode::Off});transient(self.conn.flush())?;}
        Ok(())
    }
    /// Sends the shell's pending mode request (`display_services::MODE_REQUEST`) to the window server: a configuration
    /// with the first head and the requested custom mode. Its `succeeded` / `failed` answer lands in `MODE_RESULT`. With
    /// no output management (the bare shell) the request is refused at once.
    pub fn output_modes(&mut self) -> io::Result<()> {
        use crate::shell::services::display_services::{MODE_REQUEST, MODE_RESULT};
        if self.state.output_busy {return Ok(());}
        let Some(request) = MODE_REQUEST.lock().unwrap_or_else(|p| p.into_inner()).take() else {return Ok(());};
        let (Some(manager), Some(head)) = (self.state.output_manager.clone(), self.state.output_head.clone()) else {
            *MODE_RESULT.lock().unwrap_or_else(|p| p.into_inner()) = Some(false);
            return Ok(());
        };
        let qh = self.queue.handle();
        let config = manager.create_configuration(self.state.output_serial, &qh, ());
        let head_config = config.enable_head(&head, &qh, ());
        head_config.set_custom_mode(request.width as i32, request.height as i32, request.refresh_mhz);
        config.apply();
        self.state.output_busy = true;
        transient(self.conn.flush())
    }
    pub fn refresh(&self) -> u32 { if self.state.refresh == 0 {60} else {self.state.refresh} }
    pub fn needs_redraw(&mut self) -> bool { std::mem::take(&mut self.state.redraw) }
    pub fn lock(&mut self, wanted:bool) -> io::Result<()> {
        if self.state.lock_wanted == wanted { return Ok(()); }
        self.state.lock_wanted = wanted;
        if wanted {
            let qh = self.queue.handle();
            let obj = self.state.lock_manager.as_ref().ok_or_else(||err("ext-session-lock unavailable"))?.lock(&qh,());
            let surface = self.state.compositor.as_ref().unwrap().create_surface(&qh,());
            let output = self.state.output.as_ref().ok_or_else(||err("Wayland output unavailable"))?;
            let role = obj.get_lock_surface(&surface,output,&qh,());
            // The desktop surface stays mapped under the lock: bifrost-wm draws only the lock surface while locked, and a
            // null-buffer commit would unmap the layer surface for good (smithay sends no new configure for the fresh
            // initial commit a remap needs, so the desktop never came back: flat compositor clear colour after unlock).
            self.state.lock = Some(Lock {obj,surface,role,configured:false,locked:false});
        } else if let Some(lock) = self.state.lock.take() {
            if !lock.locked { return Err(err("unlock attempted before compositor confirmed lock")); }
            lock.obj.unlock_and_destroy(); lock.role.destroy(); lock.surface.destroy();
            for slot in &mut self.state.slots { slot.damage.all(); }
        }
        if let Some(g) = &mut self.gpu { g.p.invalidate(GPU_BG); }
        self.state.pending=false; self.state.redraw=true;
        transient(self.conn.flush())
    }
    pub fn size(&self) -> (usize,usize) { (self.state.size.0 as usize,self.state.size.1 as usize) }
    pub fn fd(&self) -> i32 { use std::os::fd::AsRawFd; self.conn.as_fd().as_raw_fd() }
    pub fn busy(&self) -> bool { self.state.pending }
    /// The time of the desktop surface's last frame callback, once (PR-1).
    pub fn take_flip(&mut self) -> Option<f64> { self.state.desktop_flip.take() }
    pub fn read(&mut self) -> io::Result<()> {
        self.queue.dispatch_pending(&mut self.state).map_err(err)?;
        if let Some(read) = self.conn.prepare_read() { transient(read.read())?; }
        self.queue.dispatch_pending(&mut self.state).map_err(err)?;
        self.render_windows()?;
        self.render_chrome()?;
        transient(self.conn.flush())?;
        if self.state.closed { return Err(err("Wayland desktop closed or output resized; restart required")); }
        Ok(())
    }
    pub fn foreign_windows(&mut self) -> Option<Vec<ForeignWindow>> {
        if !std::mem::take(&mut self.state.foreign_dirty) { return None; }
        Some(self.state.foreign.iter().filter(|w| !w.info.app_id.is_empty()).map(|w| w.info.clone()).collect())
    }
    /// Whether the window server offers window capture to this connection (the privileged socket).
    pub fn window_capture_ready(&self) -> bool { self.state.cap_mgr.is_some() }
    /// Sends window captures. A request that cannot be sent (the window is gone, no shared memory) is answered `Failed` at
    /// once, so the consumer falls back; the answers come back through `capture_results`.
    pub fn capture_windows(&mut self,reqs:Vec<ShotRequest>) -> io::Result<()> {
        let qh=self.queue.handle();
        for r in reqs {
            if self.send_capture(&qh,&r).is_none() { self.state.cap_results.push((r.token,ShotOutcome::Failed)); }
        }
        transient(self.conn.flush())
    }
    fn send_capture(&mut self,qh:&QueueHandle<State>,r:&ShotRequest) -> Option<()> {
        if self.state.cap_jobs.len()>=MAX_CAPTURES_IN_FLIGHT { return None; }
        let mgr=self.state.cap_mgr.clone()?;
        let shm=self.state.shm.clone()?;
        // the window is named first: no buffer is made for one that is gone
        let surface=match r.source { ShotSource::Surface(id)=>Some(self.state.windows.get(&id)?.surface.clone()), ShotSource::Foreign(_)=>None };
        let handle=match r.source { ShotSource::Foreign(id)=>Some(self.state.foreign.iter().find(|f|f.info.id==id)?.handle.clone()), ShotSource::Surface(_)=>None };
        let (w,h)=r.max;
        let len=super::window_capture_client::buffer_len(w,h)?;
        let file=tempfile::tempfile().ok()?; file.set_len(len as u64).ok()?;
        let map=unsafe{memmap2::MmapMut::map_mut(&file)}.ok()?;
        let pool=shm.create_pool(file.as_fd(),len as i32,qh,());
        let buffer=pool.create_buffer(0,w as i32,h as i32,w as i32*4,wl_shm::Format::Argb8888,qh,());
        pool.destroy();
        let frame=match (surface,handle) {
            (Some(s),_)=>mgr.capture_surface(&s,&buffer,r.since,qh,r.token),
            (_,Some(t))=>mgr.capture_toplevel(&t,&buffer,r.since,qh,r.token),
            _=>{ buffer.destroy(); return None; }
        };
        self.state.cap_jobs.insert(r.token,CapJob{buffer,frame,map,stride:w as usize*4});
        Some(())
    }
    /// The answers to window captures since the last call: (token, outcome).
    pub fn capture_results(&mut self) -> Vec<(u64,ShotOutcome)> { std::mem::take(&mut self.state.cap_results) }
    pub fn activate_foreign(&mut self,id:u32) -> io::Result<()> {
        if let (Some(w),Some(seat))=(self.state.foreign.iter().find(|w|w.info.id==id),self.state.seat.as_ref()) { w.handle.unset_minimized(); w.handle.activate(seat); transient(self.conn.flush())?; }
        Ok(())
    }
    pub fn window_events(&mut self) -> Vec<WindowEvent> {
        // a fresh window took the keyboard from the front one and no press on it came with it: the front one gets it back
        if let Some((front,_))=self.state.refocus.take() {
            if self.state.focused==Some(front) && self.state.windows.contains_key(&front) {
                request_activation(&self.state,&self.queue.handle(),front);
                let _=self.conn.flush();
            }
        }
        std::mem::take(&mut self.state.window_events)
    }
    /// The input method batches received since the last call; each one ended with its `done`.
    pub fn input_method_batches(&mut self) -> Vec<ImBatch> { std::mem::take(&mut self.state.im_batches) }
    /// Sends the keyboard's output for the focused client's field: text, preedit and deletions go to the input method
    /// (ended by `commit` with the serial of the `done`s so far), named keys go through the virtual keyboard. Without
    /// the input method the output is dropped, as before the protocol was bound.
    pub fn input_method_commit(&mut self, out: &[ImOut]) -> io::Result<()> {
        let Some(im) = self.state.im.as_ref() else { return Ok(()) };
        let time = event_time();
        for o in out {
            match o {
                ImOut::Commit(text) => im.commit_string(text.clone()),
                ImOut::Preedit { text, begin, end } => im.set_preedit_string(text.clone(), *begin, *end),
                ImOut::Delete { before, after } => im.delete_surrounding_text(*before, *after),
                ImOut::Key(code) => {
                    if let Some(keyboard) = self.state.vk.as_ref() {
                        keyboard.key(time, *code, 1);
                        keyboard.key(time, *code, 0);
                    }
                }
            }
        }
        im.commit(self.state.im_serial);
        transient(self.conn.flush())
    }
    pub fn windows(&mut self, frames: Vec<WindowFrame>) -> io::Result<()> {
        let ids:Vec<u32> = frames.iter().map(|f|f.id).collect();
        let levels:Vec<(u32,bool)> = frames.iter().map(|f|(f.id,f.floating)).collect();
        let front=ids.last().copied();
        self.state.z=ids.clone();
        // an explicit open or focus (`raise_front`: the Dock, Overview, the Window menu) asks for the activation again
        // even when the shell already thinks the front window has the keyboard: the window server may have handed it
        // to another window since (a window it mapped), and the raise must not be lost (W4-1, W4-5)
        let raise=std::mem::take(&mut self.state.raise_front);
        let activate=front.filter(|id|raise||Some(*id)!=self.state.focused);
        self.state.focused=front;
        let removed:Vec<u32> = self.state.windows.keys().filter(|id|!ids.contains(id)).copied().collect();
        for id in removed {
            let w = self.state.windows.remove(&id).unwrap();
            self.state.materials.remove(&id);
            w.top.destroy(); w.xdg.destroy(); w.surface.destroy();
            for slot in w.slots { slot.buffer.destroy(); }
            if let Some(slots)=self.state.gpu_slots.remove(&(GPU_WINDOW+id)) {for s in slots {s.buffer.destroy();}}
            if let Some(g)=&mut self.gpu {g.p.destroy_surface(GPU_WINDOW+id);}
        }
        let qh = self.queue.handle();
        for mut frame in frames {
            if let Some(w) = self.state.windows.get_mut(&frame.id) {
                // full screen first: the window server keeps what a window held (a zoom, a tile) while it is full screen and
                // gives it back, so the zoom flag changing around full screen is not a maximize request
                let fs_changed = w.frame.fullscreen != frame.fullscreen;
                if fs_changed { if frame.fullscreen { w.top.set_fullscreen(None); } else { w.top.unset_fullscreen(); } w.fs_pending = Some(frame.fullscreen); }
                if !fs_changed && !frame.fullscreen && w.frame.maximized != frame.maximized { if frame.maximized { w.top.set_maximized(); } else { w.top.unset_maximized(); } }
                for slot in &mut w.slots { slot.damage.union(&frame.damage); }
                if w.pending.size()==frame.damage.size() { w.pending.union(&frame.damage); } else { w.pending=Damage::new(frame.width,frame.height); w.pending.all(); }
                // a moved window with vibrancy shows a different part of the wallpaper
                if (w.frame.x,w.frame.y)!=(frame.x,frame.y) && self.state.materials.contains_key(&frame.id) { w.pending=Damage::new(frame.width,frame.height); w.pending.all(); }
                // the app's minimum can change with the scale or the screen (a minimum never exceeds the screen)
                if min_size_of(w.frame.min_size)!=min_size_of(frame.min_size) { let (mw,mh)=min_size_of(frame.min_size); w.top.set_min_size(mw,mh); }
                let pixels = frame.pixels.take().or_else(||w.frame.pixels.take());
                let shape_changed = w.frame.input != frame.input;
                w.frame = frame;
                w.frame.pixels = pixels;
                if shape_changed { Self::set_window_input(self.state.compositor.as_ref(),&w.surface,w.margin,w.frame.width,w.frame.height,w.frame.input.as_deref(),&qh); }
            } else {
                let surface = self.state.compositor.as_ref().unwrap().create_surface(&qh,());
                let xdg = self.state.wm.as_ref().ok_or_else(||err("xdg-shell unavailable"))?.get_xdg_surface(&surface,&qh,frame.id);
                let top = xdg.get_toplevel(&qh,frame.id); top.set_title(frame.title.to_string()); top.set_app_id(format!("is.olibuijr.bifrost.{}",frame.id));
                let (mw,mh)=min_size_of(frame.min_size); top.set_min_size(mw,mh);
                // a window coming back from Overview or a minimize maps where it was, not at the cascade
                if let (Some((ox,oy)),Some(mgr))=(frame.origin,self.state.level_mgr.as_ref()) {
                    if mgr.version()>=super::window_level_client::ORIGIN_SINCE { mgr.set_origin(&surface,ox,oy); }
                }
                if frame.input.is_some() { Self::set_window_input(self.state.compositor.as_ref(),&surface,0,frame.width,frame.height,frame.input.as_deref(),&qh); }
                surface.commit();
                let mut pending=Damage::new(frame.width,frame.height); pending.all();
                self.state.windows.insert(frame.id,Window { surface,xdg,top,level_sent:None,slots:vec![],buffer_size:(0,0),configured:false,frame,fs_pending:None,act:false,act_seen:false,fresh:true,pending,margin:0,gpu_size:(0,0),bg:None,bg_rects:Vec::new(),mat:None,mat_sent:(0,Vec::new()) });
            }
        }
        // bifrost_window_level_v1: a window whose band changed is told the new level (the window server keeps it per surface)
        if let Some(mgr)=self.state.level_mgr.as_ref() {
            for (id,floating) in levels {
                if let Some(w)=self.state.windows.get_mut(&id) {
                    if w.level_sent!=Some(floating) {
                        mgr.set_level(&w.surface,super::window_level_client::wire(floating));
                        w.level_sent=Some(floating);
                    }
                }
            }
        }
        if let Some(id)=activate { request_activation(&self.state,&qh,id); }
        // an explicit open or focus: xdg-activation needs a fresh input serial that control-socket and dock opens
        // lack, and a native client may hold focus; wlroots compositors always obey a foreign-toplevel activate
        if raise {
            if let (Some(id),Some(seat))=(front,self.state.seat.as_ref()) {
                let app_id=format!("is.olibuijr.bifrost.{id}");
                if let Some(f)=self.state.foreign.iter().find(|f|f.info.app_id==app_id) {f.handle.unset_minimized();f.handle.activate(seat);}
            }
        }
        self.render_windows()?;
        self.render_chrome()?;
        transient(self.conn.flush())
    }
    /// Raises the front window over every client with the next `windows` call.
    pub fn raise_front(&mut self) {self.state.raise_front=true;}
    /// The UI scale (px per pt) for window shadows and corner radii; until set it is derived from the title height.
    pub fn set_scale(&mut self, s:f32) {self.state.scale=s;}
    /// Golden Gate vibrancy: the window-local rects (px, with corner radius) of a window's translucent sidebar
    /// and toolbar materials. The GPU presenter draws the blurred wallpaper behind them, offset by the window's
    /// origin; the window's pixels must be translucent there. An empty list removes the material.
    pub fn window_materials(&mut self, id:u32, rects:Vec<Glass>) {
        let w=self.state.windows.get_mut(&id);
        if self.state.materials.get(&id).map_or(rects.is_empty(),|old|*old==rects) {return;}
        if let Some(w)=w {w.pending.all();}
        if rects.is_empty() {self.state.materials.remove(&id);} else {self.state.materials.insert(id,rects);}
    }
    fn render_windows(&mut self) -> io::Result<()> {
        let qh = self.queue.handle();
        if let Some(g)=&mut self.gpu {
            // occlusion: a window that no visible pixel can show is not drawn; its pending damage waits for it
            let order:Vec<u32>=self.state.z.iter().copied().filter(|id|self.state.windows.get(id).is_some_and(|w|w.configured)).collect();
            let state_scale=self.state.scale;
            let layers:Vec<crate::gpu::occlusion::Layer>=order.iter().map(|id|{
                let f=&self.state.windows[id].frame;
                let scale=if state_scale>0.0 {state_scale} else {(f.title_height as f32/TITLE_PT).max(1.0)};
                let (m,b)=if f.maximized||f.fullscreen {(0,0)} else {((SHADOW_SIDE_PT*scale).round() as i32,(SHADOW_BOTTOM_PT*scale).round() as i32)};
                let (x,y,cw,ch)=(f.x,f.y,f.width as i32,f.height as i32);
                let radius=if f.maximized||f.fullscreen {0} else {(scale*bifrost_ui::prefs_layout::m::WINDOW_RADIUS).round() as i32};
                let glass=self.state.materials.get(id).is_some_and(|v|!v.is_empty());
                let still=f.fx.is_identity();
                crate::gpu::occlusion::Layer {
                    rect:[x-m,y-m,cw+2*m,ch+m+b],
                    // only a window that has shown content covers the ones below it
                    cover:(f.opaque&&f.pixels.is_some()&&!glass&&still).then(||crate::gpu::occlusion::opaque_inset([x,y,cw,ch],radius)).flatten(),
                    glass,
                    still,
                }
            }).collect();
            let hidden:HashSet<u32>=crate::gpu::occlusion::hidden(&layers).into_iter().zip(&order).filter(|(h,_)|*h).map(|(_,id)|*id).collect();
            let ids:Vec<u32>=self.state.windows.keys().copied().collect();
            for id in ids {
                let key=GPU_WINDOW+id;
                let focused=self.state.focused==Some(id);
                let w=self.state.windows.get_mut(&id).unwrap();
                if !w.configured || w.frame.pixels.is_none() { continue; }
                let (width,height)=(w.frame.width,w.frame.height);
                // a window with no area or an absurd size is skipped, not a reason to drop the shell's connection
                if width==0||height==0||width>8192||height>8192 { continue; }
                // the margins hold the GPU shadow (a dedicated reach, longer below); maximized windows have none
                let scale=if self.state.scale>0.0 {self.state.scale} else {(w.frame.title_height as f32/TITLE_PT).max(1.0)};
                let (margin,bottom)=if w.frame.maximized||w.frame.fullscreen||w.frame.input.is_some() {(0,0)} else {((SHADOW_SIDE_PT*scale).round() as i32,(SHADOW_BOTTOM_PT*scale).round() as i32)};
                let (bw,bh)=(width+2*margin as usize,height+(margin+bottom) as usize);
                if w.gpu_size!=(bw,bh) || !g.p.has_surface(key) {
                    if let Some(slots)=self.state.gpu_slots.remove(&key) {for s in slots {s.buffer.destroy();}}
                    let infos=g.p.create_framed(key,bw,bh,(margin as usize,margin as usize,width,height),false,2)?;
                    let slots=gpu_buffers(&self.state,&infos,&qh)?;
                    let w=self.state.windows.get_mut(&id).unwrap();
                    w.gpu_size=(bw,bh); w.margin=margin; w.pending=Damage::new(width,height); w.pending.all();
                    w.xdg.set_window_geometry(margin,margin,width as i32,height as i32);
                    // the shadow is not part of the window: clicks there reach what is below (and a shaped window takes
                    // input only where it drew)
                    Self::set_window_input(self.state.compositor.as_ref(),&w.surface,margin,width,height,w.frame.input.as_deref(),&qh);
                    self.state.gpu_slots.insert(key,slots);
                }
                let w=self.state.windows.get_mut(&id).unwrap();
                let margin=w.margin;
                g.p.set_shadow(key,(margin>0).then_some(crate::gpu::wl::Shadow{radius:scale*bifrost_ui::prefs_layout::m::WINDOW_RADIUS,focused}));
                g.p.set_fx(key,w.frame.fx);
                let Some(slot)=self.state.gpu_slots.get(&key).and_then(|v|v.iter().position(|s|!s.busy)) else {continue;};
                let pixels=w.frame.pixels.as_deref().unwrap();
                // vibrancy: the window's material rects in screen space, the blurred wallpaper behind them
                // under the window server (bifrost-wm) the materials are requested, not painted: the window stays
                // translucent and the compositor blurs the real scene below it (docs/window-server.md, "Live blur")
                let wm_mat=self.state.mat_manager.is_some();
                let wm_blur=wm_mat || (self.state.bg_blur && self.state.bg_manager.is_some());
                if wm_mat {
                    // bifrost_material_v1: kind, state, blending, emphasis and per-corner radii per region
                    let regions:Vec<Glass>=self.state.materials.get(&id).map(|m|m.iter().filter(|g|g.w>0.0&&g.h>0.0).copied().collect()).unwrap_or_default();
                    if (margin,&regions)!=(w.mat_sent.0,&w.mat_sent.1) {
                        if let (Some(mgr),Some(comp))=(self.state.mat_manager.as_ref(),self.state.compositor.as_ref()) {
                            let obj=w.mat.get_or_insert_with(||mgr.get_material(&w.surface,&qh,()));
                            obj.clear();
                            for (g,r) in regions.iter().zip(blur_rects(&regions,margin)) {
                                let a=material_args(g);
                                let region=comp.create_region(&qh,());
                                region.add(r[0],r[1],r[2],r[3]);
                                obj.add_region(&region,a.kind,a.state,a.blending,a.emphasized,a.radii[0],a.radii[1],a.radii[2],a.radii[3]);
                                region.destroy();
                            }
                            w.mat_sent=(margin,regions);
                        }
                    }
                } else if wm_blur {
                    let rects=blur_rects(self.state.materials.get(&id).map(|m|m.as_slice()).unwrap_or(&[]),margin);
                    if rects!=w.bg_rects {
                        if let (Some(mgr),Some(comp))=(self.state.bg_manager.as_ref(),self.state.compositor.as_ref()) {
                            let obj=w.bg.get_or_insert_with(||mgr.get_background_effect(&w.surface,&qh,()));
                            if rects.is_empty() {obj.set_blur_region(None);} else {
                                let region=comp.create_region(&qh,());
                                for r in &rects {region.add(r[0],r[1],r[2],r[3]);}
                                obj.set_blur_region(Some(&region)); region.destroy();
                            }
                            w.bg_rects=rects;
                        }
                    }
                }
                let (back,glass)=match self.state.materials.get(&id) {
                    Some(_) if wm_blur => (Backdrop::None,Vec::new()),
                    // safe mode: no compositor with the material protocol, so nothing can blur the scene behind the
                    // window; its materials are the inactive opaque plate (`kind.inactive()`), never a blur
                    Some(m) => (Backdrop::GlassOnly{x:w.frame.x-margin,y:w.frame.y-margin,active:false},
                        m.iter().map(|r|Glass{x:r.x+w.frame.x as f32,y:r.y+w.frame.y as f32,active:Some(false),..*r}).collect()),
                    None => (Backdrop::None,Vec::new()),
                };
                // a hidden window keeps its pending damage and is drawn when it shows again
                if hidden.contains(&id) {continue;}
                let areas=g.p.draw(key,slot,pixels,width,&w.pending,back,&glass);
                w.pending.clear();
                if areas.is_empty() {continue;}
                let s=&mut self.state.gpu_slots.get_mut(&key).unwrap()[slot];
                w.surface.attach(Some(&s.buffer),0,0);
                for a in &areas {w.surface.damage_buffer(a.x0 as i32,a.y0 as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
                w.surface.frame(&qh,false); w.surface.commit(); s.busy=true;
            }
            if let Some(why)=g.p.failure().map(str::to_owned) { self.fall_back(&why); return Ok(()); }
            return Ok(());
        }
        for w in self.state.windows.values_mut() {
            if !w.configured { continue; }
            let width = w.frame.width; let height = w.frame.height; let len = width*height*4;
            if width == 0 || height == 0 || width > 8192 || height > 8192 { continue; }
            if w.buffer_size != (width,height) {
                for slot in w.slots.drain(..) { slot.buffer.destroy(); }
            }
            let wanted = window_buffer_count(w.slots.len(), w.slots.iter().any(|s|!s.busy), w.slots.iter().any(|s|!s.damage.is_empty()));
            if w.slots.len() < wanted {
                w.buffer_size = (width,height);
                for _ in w.slots.len()..wanted {
                    let file = tempfile::tempfile()?; file.set_len(len as u64)?;
                    let map = unsafe {memmap2::MmapMut::map_mut(&file)?};
                    let pool = self.state.shm.as_ref().unwrap().create_pool(file.as_fd(),len as i32,&qh,());
                    let buffer = pool.create_buffer(0,width as i32,height as i32,width as i32*4,wl_shm::Format::Argb8888,&qh,()); pool.destroy();
                    let mut damage = Damage::new(width,height); damage.all();
                    w.slots.push(Slot {buffer,map,busy:false,damage});
                    if self.stats.is_some() { eprintln!("wayland window {}: {} buffers, {} bytes each", w.frame.id, w.slots.len(), len); }
                }
            }
            let Some(slot) = w.slots.iter_mut().find(|s|!s.busy) else {continue};
            if slot.damage.is_empty() { continue; }
            if let Some((_,frames,pixels)) = &mut self.stats {
                *frames += 1; *pixels += slot.damage.pixels();
            }
            copy_damage(w.frame.pixels.as_deref().ok_or_else(||err("new window missing initial pixels"))?,&mut slot.map,width,&slot.damage);
            w.surface.attach(Some(&slot.buffer),0,0);
            for r in slot.damage.areas() { w.surface.damage_buffer(r.x0 as i32,r.y0 as i32,(r.x1-r.x0) as i32,(r.y1-r.y0) as i32); }
            slot.damage.clear();
            w.xdg.set_window_geometry(0,0,width as i32,height as i32); w.surface.frame(&qh,false); w.surface.commit(); slot.busy=true;

        }
        if let Some((since,frames,pixels)) = &mut self.stats {
            if since.elapsed().as_secs_f64() >= 1. && *frames > 0 {
                eprintln!("wayland windows: {} commits, {} copied pixels in {:.2}s",*frames,*pixels,since.elapsed().as_secs_f64());
                *since = std::time::Instant::now(); *frames = 0; *pixels = 0;
            }
        }
        Ok(())
    }
/// Anchors a chrome surface and reserves its edge. The menu bar (id 0) spans the top edge and the Dock (id 2) sits on
/// the bottom edge, each with an exclusive zone, so the compositor keeps maximized and snapped windows clear of them
/// (an exclusive zone only counts for a surface anchored to one edge or to three). Everything else is placed from
/// the top-left corner, reserves nothing and ignores the others' zones.
fn place_chrome(l:&surface::ZwlrLayerSurfaceV1,frame:&ChromeFrame,screen:(u32,u32),top_reserved:i32) {
    match frame.id {
        0 => {l.set_anchor(surface::Anchor::Top|surface::Anchor::Left|surface::Anchor::Right);l.set_margin(frame.y,0,0,0);l.set_exclusive_zone(frame.height as i32+frame.y.max(0));}
        2 if screen.0>0&&screen.1>0&&frame.dock.is_some() => {
            let (edge,m,zone)=dock_placement(frame.x,frame.y,frame.width,frame.height,screen,frame.dock.unwrap(),top_reserved);
            l.set_anchor(match edge {
                DockEdge::Bottom=>surface::Anchor::Bottom|surface::Anchor::Left|surface::Anchor::Right,
                DockEdge::Left=>surface::Anchor::Left|surface::Anchor::Top|surface::Anchor::Bottom,
                DockEdge::Right=>surface::Anchor::Right|surface::Anchor::Top|surface::Anchor::Bottom,
            });
            l.set_margin(m[0],m[1],m[2],m[3]);l.set_exclusive_zone(zone);
        }
        // -1: placed from the screen's corner, not pushed below the menu bar's or the Dock's exclusive zone
        _ => {l.set_anchor(surface::Anchor::Top|surface::Anchor::Left);l.set_margin(frame.y,0,0,frame.x);l.set_exclusive_zone(-1);}
    }
}
/// Sets a window's input region: its `width` x `height` body inside the shadow `margin`, or only the `shape` rects
/// (window px) of a shaped window. Applies on the next commit.
fn set_window_input(compositor:Option<&wl_compositor::WlCompositor>,surface:&wl_surface::WlSurface,margin:i32,width:usize,height:usize,shape:Option<&[[i32;4]]>,qh:&QueueHandle<State>) {
    let Some(compositor)=compositor else {return};
    let region=compositor.create_region(qh,());
    for [x,y,w,h] in Self::input_rects(margin,width,height,shape) {region.add(x,y,w,h);}
    surface.set_input_region(Some(&region));region.destroy();
}
/// The rects of a window's input region in surface px (`set_window_input`): the body, or each shape rect moved by the margin.
fn input_rects(margin:i32,width:usize,height:usize,shape:Option<&[[i32;4]]>)->Vec<[i32;4]> {
    match shape {
        Some(rects)=>rects.iter().filter(|r|r[2]>0&&r[3]>0).map(|r|[r[0]+margin,r[1]+margin,r[2],r[3]]).collect(),
        None=>vec![[margin,margin,width as i32,height as i32]],
    }
}
/// Limits the pointer input of a chrome surface to `input` (x, y, w, h in surface px); `None` gives the whole surface
/// back. Applies on the next commit.
fn set_chrome_input(compositor:Option<&wl_compositor::WlCompositor>,surface:&wl_surface::WlSurface,input:Option<[i32;4]>,qh:&QueueHandle<State>) {
    match (input,compositor) {
        (Some([x,y,w,h]),Some(compositor))=>{let region=compositor.create_region(qh,());region.add(x,y,w,h);surface.set_input_region(Some(&region));region.destroy();}
        _=>surface.set_input_region(None),
    }
}

    pub fn chrome(&mut self, frames:Vec<ChromeFrame>) -> io::Result<()> {
        let top_reserved=frames.iter().find(|f|f.id==0).map_or(0,|f|f.height as i32+f.y.max(0));
        let ids:Vec<_>=frames.iter().map(|f|f.id).collect();
        let removed:Vec<_>=self.state.chrome.keys().filter(|id|!ids.contains(id)).copied().collect();
        for id in removed {let w=self.state.chrome.remove(&id).unwrap();w.layer.destroy();w.surface.destroy();for slot in w.slots {slot.buffer.destroy();}
            if let Some(slots)=self.state.gpu_slots.remove(&id) {for s in slots {s.buffer.destroy();}}
            if let Some(g)=&mut self.gpu {g.p.destroy_surface(id);}}
        let qh=self.queue.handle();
        for frame in frames {
            let screen=self.state.size;
            if let Some(w)=self.state.chrome.get_mut(&frame.id) {
                Self::place_chrome(&w.layer,&frame,screen,top_reserved);
                if w.frame.input!=frame.input {Self::set_chrome_input(self.state.compositor.as_ref(),&w.surface,frame.input,&qh);}
                if (w.frame.width,w.frame.height)!=(frame.width,frame.height) {
                    for slot in w.slots.drain(..) {slot.buffer.destroy();}
                    if let Some(slots)=self.state.gpu_slots.remove(&frame.id) {for s in slots {s.buffer.destroy();}}
                    if let Some(g)=&mut self.gpu {g.p.destroy_surface(frame.id);}
                    w.layer.set_size(frame.width as u32,frame.height as u32);w.configured=false;
                }
                let d=Damage::between(&w.frame.pixels,&frame.pixels,frame.width,frame.height);
                for slot in &mut w.slots {slot.damage.union(&d);}
                if w.pending.size()!=(frame.width,frame.height) {w.pending=Damage::new(frame.width,frame.height);w.pending.all();} else {w.pending.union(&d);}
                w.frame=frame;
                w.surface.commit();
            } else {
                let s=self.state.compositor.as_ref().unwrap().create_surface(&qh,());
                let l=self.state.layer.as_ref().unwrap().get_layer_surface(&s,None,if frame.id==0 {layer::Layer::Top}else{layer::Layer::Overlay},"bifrost-desktop-chrome".into(),&qh,frame.id);
                l.set_size(frame.width as u32,frame.height as u32);Self::place_chrome(&l,&frame,screen,top_reserved);
                if frame.input.is_some() {Self::set_chrome_input(self.state.compositor.as_ref(),&s,frame.input,&qh);}
                // id 3 is Search, id 6 the assistant panel, id 8 the Apps grid, id 9 Overview and id 11 the screenshot session
                // (Space and Escape are its keys), id 15 the system sheets and alerts, id 14 the Force Quit, About and Log Out card (Esc, Return and the arrows are its
                // keys; without this they would reach the app in front, which is the hung one): they take the keyboard from the
                // window in front while open; id 13, the Cmd+Tab strip, never does
                l.set_keyboard_interactivity(if frame.id==3 || frame.id==6 || frame.id==8 || frame.id==9 || frame.id==11 || frame.id==14 || frame.id==15 {surface::KeyboardInteractivity::Exclusive} else {surface::KeyboardInteractivity::None});s.commit();
                let mut pending=Damage::new(frame.width,frame.height);pending.all();
                self.state.chrome.insert(frame.id,ChromeWindow{surface:s,layer:l,slots:vec![],configured:false,frame,pending});
            }
        }
        self.render_chrome()?;transient(self.conn.flush())
    }
    /// Draws chrome that `chrome` left pending because the compositor still held both of its buffers (a keyboard
    /// sliding in changes its surface every frame): called on every pass of the main loop, which a buffer release
    /// wakes, so the last image of a change is not left out.
    pub fn chrome_flush(&mut self)->io::Result<()> {
        if self.state.chrome.values().any(|w|!w.pending.is_empty()) {self.render_chrome()?;transient(self.conn.flush())?;}
        Ok(())
    }
    fn render_chrome(&mut self)->io::Result<()> {
        let qh=self.queue.handle();
        if let Some(g)=&mut self.gpu {
            let ids:Vec<u32>=self.state.chrome.keys().copied().collect();
            for id in ids {
                let w=&self.state.chrome[&id];
                if !w.configured {continue;}
                let (width,height)=(w.frame.width,w.frame.height);
                if !g.p.has_surface(id) {
                    let infos=g.p.create_surface(id,width,height,false,2)?;
                    let slots=gpu_buffers(&self.state,&infos,&qh)?;
                    self.state.gpu_slots.insert(id,slots);
                }
                let Some(slot)=self.state.gpu_slots.get(&id).and_then(|v|v.iter().position(|s|!s.busy)) else {continue;};
                let w=self.state.chrome.get_mut(&id).unwrap();
                // drawn for new content, or when the presenter owes it a redraw of its own (the Night Shift fade)
                if w.pending.is_empty() && !g.p.owes(id) {continue;}
                let areas=g.p.draw(id,slot,&w.frame.pixels,width,&w.pending,Backdrop::GlassOnly{x:w.frame.x,y:w.frame.y,active:true},&g.glass);
                w.pending.clear();
                if areas.is_empty() {continue;}
                let s=&mut self.state.gpu_slots.get_mut(&id).unwrap()[slot];
                w.surface.attach(Some(&s.buffer),0,0);
                for a in &areas {w.surface.damage_buffer(a.x0 as i32,a.y0 as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
                w.surface.commit();s.busy=true;
                mark_chrome_frame(id);
            }
            if let Some(why)=g.p.failure().map(str::to_owned) { self.fall_back(&why); }
            return Ok(());
        }
        for (&id, w) in self.state.chrome.iter_mut() {
            if !w.configured {continue;}
            let width=w.frame.width;let height=w.frame.height;let len=width*height*4;
            let wanted=window_buffer_count(w.slots.len(),w.slots.iter().any(|s|!s.busy),w.slots.iter().any(|s|!s.damage.is_empty()));
            for _ in w.slots.len()..wanted {
                let file=tempfile::tempfile()?;file.set_len(len as u64)?;let map=unsafe{memmap2::MmapMut::map_mut(&file)?};
                let pool=self.state.shm.as_ref().unwrap().create_pool(file.as_fd(),len as i32,&qh,());
                let buffer=pool.create_buffer(0,width as i32,height as i32,width as i32*4,wl_shm::Format::Argb8888,&qh,());pool.destroy();
                let mut damage=Damage::new(width,height);damage.all();w.slots.push(Slot{buffer,map,busy:false,damage});
            }
            let Some(slot)=w.slots.iter_mut().find(|s|!s.busy&&!s.damage.is_empty()) else {continue;};
            copy_damage(&w.frame.pixels,&mut slot.map,width,&slot.damage);w.surface.attach(Some(&slot.buffer),0,0);
            for a in slot.damage.areas(){w.surface.damage_buffer(a.x0 as i32,a.y0 as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
            slot.damage.clear();w.surface.commit();slot.busy=true;
            mark_chrome_frame(id);
        }Ok(())
    }
    pub fn foreign_action(&mut self,id:u32,minimize:bool)->io::Result<()> {
        if let Some(w)=self.state.foreign.iter().find(|w|w.info.id==id) {if minimize {w.handle.set_minimized();}else{w.handle.close();}}
        transient(self.conn.flush())
    }
    /// Asks the window server to place the shell's toplevel `id` (a tile, the centre, the way back to the previous size;
    /// `bifrost_window_level_v1` version 3). Without the request the window stays where it is.
    pub fn tile_window(&mut self,id:u32,placement:super::window_level_client::Placement)->io::Result<()> {
        if let (Some(mgr),Some(w))=(self.state.level_mgr.as_ref(),self.state.windows.get(&id)) {
            if mgr.version()>=super::window_level_client::TILE_SINCE { mgr.tile(&w.surface,super::window_level_client::placement_wire(placement)); }
        }
        transient(self.conn.flush())
    }
    /// Asks the window server to place another client's window `id` (a foreign toplevel this shell listed;
    /// `bifrost_window_level_v1` version 7 `tile_toplevel`): the Window menu and the Ctrl+Cmd arrows on a native app's
    /// window. Without the request the window stays where it is.
    pub fn tile_foreign(&mut self,id:u32,placement:super::window_level_client::Placement)->io::Result<()> {
        if let (Some(mgr),Some(w))=(self.state.level_mgr.as_ref(),self.state.foreign.iter().find(|w|w.info.id==id)) {
            if mgr.version()>=super::window_level_client::TILE_TOPLEVEL_SINCE { mgr.tile_toplevel(&w.handle,super::window_level_client::placement_wire(placement)); }
        }
        transient(self.conn.flush())
    }
    pub fn foreign_maximize(&mut self,id:u32,maximized:bool)->io::Result<()> {if let Some(w)=self.state.foreign.iter().find(|w|w.info.id==id) {if maximized {w.handle.set_maximized();}else{w.handle.unset_maximized();}}transient(self.conn.flush())}
    pub fn inputs(&mut self, out: &mut Vec<Event>) {
        self.trim_idle();
        // a chrome surface was configured after its frame was queued: attach it now instead of waiting for the next shell
        // frame (an idle shell made the menu bar and Dock wait out the reveal gate's 4 s ceiling)
        if std::mem::take(&mut self.state.chrome_configured) {
            let _ = self.render_chrome();
            let _ = self.conn.flush();
        }
        // a report whose `frame` has not arrived yet is delivered as it stands
        self.state.fingers.frame();
        self.state.fingers.take(&mut self.state.inputs);
        out.append(&mut self.state.inputs);
    }
    /// The wallpaper the GPU draws under the desktop (uploaded and blurred once per change), always covering the output
    /// whatever the picture's size. `live` is the animated look a frame belongs to (its blur is built once per look and size).
    /// Returns false when there is no GPU presenter or the blur did not land (the shell asks again).
    pub fn wallpaper(&mut self, wp: crate::gfx::View, x: i32, live: Option<u8>) -> bool {
        let screen = (self.state.size.0 as usize, self.state.size.1 as usize);
        if let Some(g) = &mut self.gpu { g.p.set_wallpaper(wp, x, 0, screen, live) } else { false }
    }
    /// Night Shift on the GPU path: fades the presenter's white point towards `m` (`night_shift::multipliers`, None off)
    /// and redraws the shell's surfaces with it from what they already hold: windows and chrome now, the desktop through
    /// an empty `present` (nothing is uploaded, the CPU draws nothing). True while the fade runs, so the caller keeps
    /// waking every frame; false once settled or without a GPU presenter (the CPU path tints the frame itself).
    pub fn night(&mut self, m: Option<[u32; 3]>) -> io::Result<bool> {
        // the window server tints the whole output, apps included (`set_white_point`): it gets the target once per change
        // and fades it itself, and the presenter's own pass fades back to white so nothing is tinted twice
        let wm = self.state.wm_white_point;
        if wm && self.state.white_point_sent != Some(m) {
            if let Some(mgr) = self.state.level_mgr.as_ref().filter(|mgr| mgr.version() >= super::window_level_client::WHITE_POINT_SINCE) {
                let [r, g, b] = super::window_level_client::white_point_wire(m);
                mgr.set_white_point(r, g, b);
                self.state.white_point_sent = Some(m);
                transient(self.conn.flush())?;
            }
        }
        let Some(g) = &mut self.gpu else { return Ok(false) };
        let now = std::time::Instant::now();
        // a long idle gap counts as one frame, so a schedule that starts while nothing draws still fades in
        let dt = (now.duration_since(g.night_at).as_secs_f32() * 1000.0).min(50.0);
        g.night_at = now;
        let target = if wm { [1.0; 3] } else { crate::gpu::wl::night_factors(m) };
        let cur = g.p.night();
        if cur == target { return Ok(false); }
        g.p.set_night(crate::gpu::wl::night_step(cur, target, dt));
        let (w, h) = (self.state.size.0 as usize, self.state.size.1 as usize);
        self.render_windows()?;
        self.render_chrome()?;
        // a presenter that failed in the passes above fell back to SHM, which must not be handed an empty frame
        if self.gpu.is_some() { self.present(&[], &Damage::new(w, h))?; }
        transient(self.conn.flush())?;
        Ok(true)
    }
    /// True when the window server applies Night Shift to the whole output: the shell then tints nothing itself, on the
    /// GPU presenter or on the CPU copy of an SHM frame.
    pub fn wm_tints(&self) -> bool { self.state.wm_white_point }
    /// Frees the spare background drawable once the shell has drawn nothing for `GPU_POOL_IDLE_SECS`.
    pub fn trim_idle(&mut self) {
        let Some(g) = &mut self.gpu else { return };
        let Some(v) = self.state.gpu_slots.get_mut(&GPU_BG) else { return };
        let busy: Vec<bool> = v.iter().map(|s| s.busy).collect();
        if gpu_pool_step(&busy, false, g.last_frame.elapsed().as_secs_f32(), gpu_pool_max(self.state.lock_wanted)) == PoolStep::Shrink {
            if let Some(s) = v.pop() { s.buffer.destroy(); }
            g.p.pop_drawable(GPU_BG);
            let _ = self.conn.flush();
        }
    }
    /// Ms until `trim_idle` would free the spare drawable: a single one-shot deadline for the main loop's
    /// timeout (None when the pool is already minimal), so trimming needs no polling wake-up.
    pub fn trim_wake_ms(&self) -> Option<i32> {
        let g = self.gpu.as_ref()?;
        let v = self.state.gpu_slots.get(&GPU_BG)?;
        pool_trim_in_ms(v.len(), v.last().is_some_and(|s| s.busy), g.last_frame.elapsed().as_secs_f32())
    }
    pub fn gpu_active(&self) -> bool { self.gpu.is_some() }
    /// True once after the GPU presenter gave up: the shell then draws a full CPU frame for the SHM path.
    pub fn take_gpu_fallback(&mut self) -> bool { std::mem::take(&mut self.fell_back) }
    /// The GPU presenter failed (`why`): the shell leaves it for good and draws through SHM. The dma-buf buffers go
    /// first (the compositor keeps what it still holds), then the presenter is dropped, and the SHM buffers are marked
    /// fully damaged so the next frame repaints them whole.
    fn fall_back(&mut self, why: &str) {
        bifrost_core::log::shell_logger("presenter").error(format!("GPU presenter failed, falling back to the SHM path: {why}"));
        for (_, slots) in self.state.gpu_slots.drain() {
            for s in slots { s.buffer.destroy(); }
        }
        // glass is drawn by the GPU presenter only: with it gone, nothing is pushed to a presenter that no longer exists
        bifrost_render::glass::enable(false);
        self.gpu = None;
        // the CPU frame now carries the wallpaper itself: the shell stops leaving it to the presenter
        bifrost_render::glass::enable(false);
        for slot in &mut self.state.slots { slot.damage.all(); }
        self.fell_back = true;
        let _ = self.conn.flush();
    }
    pub fn note_draw(&mut self, ms: f64) { if let Some(g) = &mut self.gpu { g.draw.0 += 1; g.draw.1 += ms; } }
    /// Screen Zoom for the next present: the shell's zoom state and the pointer in screen px. The presenter magnifies
    /// the composed frame around the pointer. Without a GPU presenter (the CPU path) zoom is not drawn.
    ///
    /// Under a window server with `set_zoom` (bifrost-wm, protocol version 6) Full Screen zoom is the window server's:
    /// it magnifies the whole output, the wallpaper and every window included, around its own pointer, and the
    /// presenter magnifies nothing (W5-17: magnifying the shell's frame alone left the windows, bar and Dock at 1x and
    /// blacked out the wallpaper, which the presenter does not hold). The factor is sent once per change.
    pub fn set_zoom(&mut self, zoom: ScreenZoom, pointer: (f32, f32)) {
        let wm = self.state.level_mgr.as_ref().filter(|mgr| mgr.version() >= super::window_level_client::ZOOM_SINCE);
        let local = match wm {
            Some(mgr) => {
                let wire = super::window_level_client::zoom_wire(zoom);
                if self.state.zoom_sent != Some(wire) {
                    mgr.set_zoom(wire);
                    self.state.zoom_sent = Some(wire);
                    let _ = transient(self.conn.flush());
                }
                if zoom.mode == Mode::Full { ScreenZoom { on: false, ..zoom } } else { zoom }
            }
            None => zoom,
        };
        if let Some(g) = &mut self.gpu { g.zoom = local; g.pointer = pointer; }
    }
    pub fn present(&mut self, pixels: &[u32], damage: &Damage) -> io::Result<()> {
        if self.state.lock_wanted && !self.state.lock.as_ref().is_some_and(|l|l.configured) { return Ok(()); }
        if let Some(g) = &mut self.gpu {
            let (w,h) = (self.state.size.0 as usize, self.state.size.1 as usize);
            // a magnified frame changes every pixel, and the frame after zoom turns off still holds magnified pixels:
            // while zoom is on, or on the frame it goes off, the whole screen is presented
            let magnify = magnify_for(g.zoom, g.pointer, (w, h));
            let whole = (magnify.is_some() || g.zoomed).then(|| { let mut d = Damage::new(w, h); d.all(); d });
            g.zoomed = magnify.is_some();
            let damage = whole.as_ref().unwrap_or(damage);
            let fresh = bifrost_render::glass::take();
            // the regions the shell redrew count as redrawn even where their pixels came out the same: a panel that
            // fades out ends on identical pixels, and its blur shape must still go (OV1)
            let redrawn = bifrost_render::glass::take_redrawn();
            let before = g.glass.clone();
            if damage.pixels() >= w*h { g.glass = fresh; } else {
                let mut areas: Vec<_> = damage.areas().iter().map(|a| (a.x0 as f32, a.y0 as f32, a.x1 as f32, a.y1 as f32)).collect();
                areas.extend(redrawn);
                bifrost_render::glass::merge(&mut g.glass, fresh, &areas);
            }
            // a shape that came or went changes the backdrop under its bounds, so those bounds are drawn again
            let moved = bifrost_render::glass::changed_shapes(&before, &g.glass);
            // the chrome surfaces (the Apps grid, Overview, Search...) blur under the same shapes: a shape that came or went
            // is owed to each of them too, or the shapes of earlier frames stay on a surface whose pixels did not change (N11)
            for c in self.state.chrome.values_mut() { owe_glass(&mut c.pending, (c.frame.x, c.frame.y), &moved); }
            let grown;
            let damage = if moved.is_empty() || damage.is_all() { damage } else {
                let mut d = damage.clone();
                for (x, y, sw, sh) in moved { d.add(x.floor() as i32, y.floor() as i32, sw.ceil() as i32 + 1, sh.ceil() as i32 + 1); }
                grown = d;
                &grown
            };
            // always consume the flush flag, so an owed glass refresh is not left for a later empty present
            let flush = g.p.take_flush();
            if damage.is_empty() && !flush { return Ok(()); }
            g.last_frame = std::time::Instant::now();
            let mut free = self.state.gpu_slots.get(&GPU_BG).and_then(|v| v.iter().position(|s| !s.busy));
            if free.is_none() {
                // the compositor holds every drawable: allocate the spare one (never draw into a held buffer)
                let busy: Vec<bool> = self.state.gpu_slots.get(&GPU_BG).map(|v| v.iter().map(|s| s.busy).collect()).unwrap_or_default();
                if gpu_pool_step(&busy, true, 0.0, gpu_pool_max(self.state.lock_wanted)) == PoolStep::Grow {
                    if let Ok(info) = g.p.add_drawable(GPU_BG) {
                        let qh = self.queue.handle();
                        match gpu_buffers(&self.state, &[info], &qh) {
                            Ok(mut n) => match self.state.gpu_slots.get_mut(&GPU_BG) {
                                Some(v) => { v.append(&mut n); free = Some(v.len() - 1); }
                                None => g.p.pop_drawable(GPU_BG),
                            },
                            Err(_) => g.p.pop_drawable(GPU_BG),
                        }
                    }
                }
            }
            let Some(slot) = free else {
                // every drawable is still on screen: keep the damage, and redraw when one comes back
                g.p.owe(GPU_BG, damage);
                self.state.starved.dropped();
                return Ok(());
            };
            let areas = g.p.draw(GPU_BG, slot, pixels, w, damage, Backdrop::Full{x:0,y:0}, &g.glass);
            if let Some(why) = g.p.failure().map(str::to_owned) { self.fall_back(&why); return Ok(()); }
            // only a slot drawn this frame holds this frame's composition; an undrawn one must not be magnified again
            if let Some(m) = magnify.filter(|_| !areas.is_empty()) {
                if let Err(e) = g.p.magnify(GPU_BG, slot, m) { eprintln!("springboard: screen zoom: {e}"); }
            }
            let st = g.p.stats;
            g.log.1 += 1; g.log.2 += st.upload_ms; g.log.3 += st.render_ms; g.log.4 += st.uploaded_px;
            if g.log.0.elapsed().as_secs() >= 10 {
                let n = g.log.1.max(1) as f64;
                eprintln!("springboard: gpu present: {} frames, upload {:.2} ms, render {:.2} ms, {:.0} kpx uploaded per frame, cpu draw {:.2} ms", g.log.1, g.log.2/n, g.log.3/n, g.log.4 as f64/n/1e3, g.draw.1/g.draw.0.max(1) as f64);
                g.log = (std::time::Instant::now(), 0, 0., 0., 0); g.draw = (0, 0.);
            }
            if areas.is_empty() { return Ok(()); }
            let s = self.state.lock.as_ref().filter(|_|self.state.lock_wanted).map(|l|&l.surface).unwrap_or_else(||self.state.surface.as_ref().unwrap());
            let b = &mut self.state.gpu_slots.get_mut(&GPU_BG).unwrap()[slot];
            s.attach(Some(&b.buffer), 0, 0);
            for r in &areas { s.damage_buffer(r.x0 as i32,r.y0 as i32,(r.x1-r.x0) as i32,(r.y1-r.y0) as i32); }
            b.busy = true;
            s.frame(&self.queue.handle(), true); self.state.pending = true; s.commit();
            mark_first_frame();
            return transient(self.conn.flush());
        }
        for slot in &mut self.state.slots { slot.damage.union(damage); }
        let Some(i) = self.state.slots.iter().position(|s| !s.busy) else { self.state.starved.dropped(); return Ok(()); };
        let slot = &mut self.state.slots[i];
        if slot.damage.is_empty() { return Ok(()); }
        let w = self.state.size.0 as usize;
        copy_damage(pixels, &mut slot.map, w, &slot.damage);
        let s = self.state.lock.as_ref().filter(|_|self.state.lock_wanted).map(|l|&l.surface).unwrap_or_else(||self.state.surface.as_ref().unwrap());
        s.attach(Some(&slot.buffer), 0, 0);
        for r in slot.damage.areas() { s.damage_buffer(r.x0 as i32,r.y0 as i32,(r.x1-r.x0) as i32,(r.y1-r.y0) as i32); }
        slot.damage.clear(); slot.busy = true;
        s.frame(&self.queue.handle(), true); self.state.pending = true; s.commit();
        mark_first_frame();
        transient(self.conn.flush())
    }
}
/// Writes the menu bar's or the Dock's mark (`bootreveal::chrome_part`) the first time that chrome surface commits a
/// frame. The marks file is read once per part at most: a bit per chrome id, set on the first write.
fn mark_chrome_frame(id: u32) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEEN: AtomicU32 = AtomicU32::new(0);
    let Some(part) = crate::bootreveal::chrome_part(id) else { return };
    let bit = 1u32 << id;
    if SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        crate::bootsplash::mark(part);
    }
}
/// Writes `$XDG_RUNTIME_DIR/shell-ready` once, after the first desktop frame is committed, and the wallpaper's mark
/// (`bootreveal::DESKTOP_PARTS`): the wallpaper is drawn into the desktop surface. The boot frame's reveal waits for
/// all three parts so it never gives way to a partial desktop.
fn mark_first_frame() {
    if !crate::bootsplash::note_first_desktop_frame() { return; }
    crate::bootsplash::mark(crate::bootreveal::DESKTOP_PARTS[0]);
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let _ = std::fs::write(std::path::Path::new(&dir).join("shell-ready"), b"1\n");
    }
}
/// Adds the bounds of the glass shapes that came or went this frame (`moved`, screen px) to the damage of a chrome
/// surface placed at `origin`: its backdrop changes there whatever its pixels did. A surface masked clear around its
/// panel (the Apps grid) has no pixel damage outside it, so without this the blur of every shape it was first drawn
/// with stayed on it, a band of stale plates under the grid (N11).
fn owe_glass(pending:&mut Damage, origin:(i32,i32), moved:&[(f32,f32,f32,f32)]) {
    for &(x,y,w,h) in moved { pending.add(x.floor() as i32-origin.0, y.floor() as i32-origin.1, w.ceil() as i32+1, h.ceil() as i32+1); }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, reg: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, q: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name,interface,version } = e { match interface.as_str() {
            "bifrost_material_manager_v1" => s.mat_manager=Some(reg.bind(name,1,q,())),
            "bifrost_window_level_manager_v1" => s.level_mgr=Some(reg.bind(name,version.min(super::window_level_client::VERSION),q,())),
            "bifrost_window_capture_manager_v1" => s.cap_mgr=Some(reg.bind(name,version.min(super::window_capture_client::VERSION),q,())),
            "ext_background_effect_manager_v1" => s.bg_manager=Some(reg.bind(name,1,q,())),
            "zwlr_foreign_toplevel_manager_v1" => s.foreign_manager=Some(reg.bind(name,version.min(3),q,())),
            "ext_idle_notifier_v1" => s.idle_notifier=Some(reg.bind(name,version.min(2),q,())),
            "zwlr_output_power_manager_v1" => s.power_manager=Some(reg.bind(name,1,q,())),
            "zwlr_output_manager_v1" => s.output_manager=Some(reg.bind(name,version.min(4),q,())),
            "wp_cursor_shape_manager_v1" => s.cursor_manager=Some(reg.bind(name,version.min(1),q,())),
            "wl_output" if s.output.is_none() => s.output=Some(reg.bind(name,version.min(3),q,())),
            "ext_session_lock_manager_v1" => s.lock_manager=Some(reg.bind(name,1,q,())),
            "wl_compositor" => s.compositor = Some(reg.bind(name,version.min(4),q,())),
            "wl_shm" => s.shm = Some(reg.bind(name,1,q,())),
            "zwp_linux_dmabuf_v1" => s.dmabuf = Some(reg.bind(name,version.min(3),q,())),
            "zwlr_layer_shell_v1" => s.layer = Some(reg.bind(name,version.min(4),q,())),
            "wl_seat" => s.seat = Some(reg.bind(name,version.min(5),q,())),
            "xdg_activation_v1" => s.activation=Some(reg.bind(name,1,q,())),
            "xdg_wm_base" => s.wm = Some(reg.bind(name,version.min(6),q,())),
            "zwp_input_method_manager_v2" => s.im_manager = Some(reg.bind(name,1,q,())),
            "zwp_virtual_keyboard_manager_v1" => s.vk_manager = Some(reg.bind(name,1,q,())),
            _ => {}
        }}
    }
}
impl Dispatch<surface::ZwlrLayerSurfaceV1, ()> for State {
    fn event(s: &mut Self, obj: &surface::ZwlrLayerSurfaceV1, e: surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            surface::Event::Configure { serial,width,height } => {
                obj.ack_configure(serial);
                if s.configured && s.size != (width,height) { s.closed = true; }
                s.size = (width,height); s.configured = true;
            },
            surface::Event::Closed => s.closed = true,
            _ => {}
        }
    }
}
impl Dispatch<surface::ZwlrLayerSurfaceV1,u32> for State {
    fn event(s:&mut Self,obj:&surface::ZwlrLayerSurfaceV1,e:surface::Event,id:&u32,_:&Connection,_:&QueueHandle<Self>) {
        match e {surface::Event::Configure{serial,..}=>{obj.ack_configure(serial);
            // match the object, not just the id: a chrome surface destroyed and re-created under the same id (an
            // overlay closing and reopening on unlock) can still receive the old surface's configure, and marking the
            // new one configured let it attach a buffer first: "layer_surface has never been configured" (0.5.16)
            if let Some(w)=s.chrome.get_mut(id).filter(|w|w.layer==*obj){w.configured=true;s.chrome_configured=true;}},surface::Event::Closed=>s.closed=true,_=>{}}
    }
}
impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(s: &mut Self, obj: &wl_buffer::WlBuffer, e: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if matches!(e,wl_buffer::Event::Release) {
            // a desktop buffer: the full-screen SHM slots or the GPU background drawables
            let mut desktop = false;
            for slot in &mut s.slots { if &slot.buffer == obj { slot.busy = false; desktop = true; } }
            for slot in s.windows.values_mut().flat_map(|w| w.slots.iter_mut()).chain(s.chrome.values_mut().flat_map(|w|w.slots.iter_mut())) {
                if &slot.buffer == obj { slot.busy = false; }
            }
            for (id, v) in s.gpu_slots.iter_mut() {
                for slot in v { if &slot.buffer == obj { slot.busy = false; desktop |= *id == GPU_BG; } }
            }
            // a desktop frame was dropped for want of a buffer: draw it now (the loop takes `redraw` every iteration)
            if desktop && s.starved.released() { s.redraw = true; }
        }
    }
}
impl Dispatch<wl_callback::WlCallback, bool> for State {
    fn event(s: &mut Self, _: &wl_callback::WlCallback, ev: wl_callback::Event, desktop: &bool, _: &Connection, _: &QueueHandle<Self>) {
        // only the desktop surface's frame callback is a vblank for the frame clock (PR-1)
        if *desktop && matches!(ev, wl_callback::Event::Done { .. }) { s.pending = false; s.desktop_flip = Some(super::sys::now_ms()); }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(s: &mut Self, seat: &wl_seat::WlSeat, e: wl_seat::Event, _: &(), _: &Connection, q: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(c) } = e {
            if c.contains(wl_seat::Capability::Pointer) && s.pointer.is_none() {
                let pointer=seat.get_pointer(q,());
                if let Some(manager)=&s.cursor_manager { s.cursor_device=Some(manager.get_pointer(&pointer,q,())); }
                s.pointer=Some(pointer);
            }
            if c.contains(wl_seat::Capability::Keyboard) {
                // the seat repeats its capabilities whenever one changes (touch, a plugged keyboard): bind the keyboard once,
                // a second object delivered every key twice
                if s.keyboard.is_none() { s.keyboard=Some(seat.get_keyboard(q,())); }
            } else if let Some(keyboard)=s.keyboard.take() {
                for code in s.keys.drain() { s.inputs.push(Event::Key {code,down:false}); }
                if keyboard.version()>=3 { keyboard.release(); }
            }
            if c.contains(wl_seat::Capability::Touch) {
                if s.touch.is_none() { s.touch=Some(seat.get_touch(q,())); }
            } else if let Some(touch)=s.touch.take() {
                // the touch device went away: end its fingers, and give the object back where the protocol allows
                s.fingers.cancel(super::sys::now_ms());
                s.fingers.take(&mut s.inputs);
                if touch.version()>=3 { touch.release(); }
            }
        }
    }
}
/// The shape an app pushed with the `cursor` service, in the cursor-shape protocol's names.
fn wl_shape(k: bifrost_ui::cursor::Kind) -> cursor_device::Shape {
    use bifrost_ui::cursor::Kind as K;
    match k {
        K::Arrow => cursor_device::Shape::Default,
        K::IBeam | K::VerticalIBeam => cursor_device::Shape::Text,
        K::Crosshair => cursor_device::Shape::Crosshair,
        K::OpenHand => cursor_device::Shape::Grab,
        K::ClosedHand => cursor_device::Shape::Grabbing,
        K::PointingHand => cursor_device::Shape::Pointer,
        K::ContextualMenu => cursor_device::Shape::ContextMenu,
        K::OperationNotAllowed => cursor_device::Shape::NotAllowed,
        K::DragCopy => cursor_device::Shape::Copy,
        K::DragLink => cursor_device::Shape::Alias,
        K::DisappearingItem => cursor_device::Shape::NoDrop,
        K::ZoomIn => cursor_device::Shape::ZoomIn,
        K::ZoomOut => cursor_device::Shape::ZoomOut,
        K::ResizeLeftRight => cursor_device::Shape::EwResize,
        K::ResizeUpDown => cursor_device::Shape::NsResize,
        K::ResizeLeft => cursor_device::Shape::WResize,
        K::ResizeRight => cursor_device::Shape::EResize,
        K::ResizeUp => cursor_device::Shape::NResize,
        K::ResizeDown => cursor_device::Shape::SResize,
        K::ResizeNorthEastSouthWest => cursor_device::Shape::NeswResize,
        K::ResizeNorthWestSouthEast => cursor_device::Shape::NwseResize,
        K::ColumnResize => cursor_device::Shape::ColResize,
        K::RowResize => cursor_device::Shape::RowResize,
        K::Move => cursor_device::Shape::Move,
        // the cursor-shape protocol has no rotate or window-capture shape: the nearest named ones
        K::Rotate => cursor_device::Shape::Grabbing,
        K::ScreenshotWindow => cursor_device::Shape::Crosshair,
    }
}

/// Sets the pointer as the `cursor` service left it: hidden (no image), the shape an app pushed, or `edge`, the
/// shell's own resize shape over a window edge. A hidden pointer gets no image from `set_cursor`, and the next
/// `set_shape` shows it again.
fn show_pointer(s: &State, serial: u32, edge: cursor_device::Shape) {
    if !crate::shell::services::cursor_service::pointer_visible() {
        if let Some(p) = &s.pointer {
            p.set_cursor(serial, None, 0, 0);
        }
        return;
    }
    let shape = crate::shell::services::cursor_service::pointer_shape().map_or(edge, wl_shape);
    if let Some(cursor) = &s.cursor_device {
        cursor.set_shape(serial, shape);
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(s: &mut Self, _: &wl_pointer::WlPointer, e: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let t = super::sys::now_ms();
        let offset = s.target.and_then(|id| s.windows.get(&id)).map(|w| (w.frame.x as f32,w.frame.y as f32)).or_else(||s.chrome_target.and_then(|id|s.chrome.get(&id)).map(|w|(w.frame.x as f32,w.frame.y as f32))).unwrap_or((0.0,0.0));
        if let wl_pointer::Event::Leave{..}=e {crate::shell::services::screen_sample::pointer_left();}
        match e {
            wl_pointer::Event::Enter { surface,surface_x,surface_y,serial,.. } => {
                s.cursor_serial=serial;
                show_pointer(s,serial,cursor_device::Shape::Default);
                s.chrome_target=s.chrome.iter().find(|(_,w)|w.surface==surface).map(|(&id,_)|id);
                s.target = s.windows.iter().find(|(_,w)| w.surface == surface).map(|(&id,_)| id);
                let m = s.target.and_then(|id| s.windows.get(&id)).map_or(0.0, |w| w.margin as f32);
                s.xy = (surface_x as f32 - m,surface_y as f32 - m);
            },
            wl_pointer::Event::Motion { surface_x,surface_y,.. } => {
                crate::shell::services::cursor_service::pointer_moved();
                let m = s.target.and_then(|id| s.windows.get(&id)).map_or(0.0, |w| w.margin as f32);
                s.xy = (surface_x as f32 - m,surface_y as f32 - m);
                crate::shell::services::screen_sample::pointer_moved(s.xy.0+offset.0,s.xy.1+offset.1);
                {
                    let shape=s.target.and_then(|id|s.windows.get(&id)).map_or(cursor_device::Shape::Default,|w| {
                        let (x,y)=s.xy; let edge=6.0;
                        let left=x<edge;let right=x>w.frame.width as f32-edge;
                        let top=y<edge;let bottom=y>w.frame.height as f32-edge;
                        if (left&&top)||(right&&bottom) {cursor_device::Shape::NwseResize}
                        else if (right&&top)||(left&&bottom) {cursor_device::Shape::NeswResize}
                        else if left||right {cursor_device::Shape::EwResize}
                        else if top||bottom {cursor_device::Shape::NsResize}
                        else {cursor_device::Shape::Default}
                    });
                    show_pointer(&*s,s.cursor_serial,shape);
                }
                if s.pressed { s.inputs.push(Event::Move { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t }); }else{s.inputs.push(Event::Hover{x:s.xy.0+offset.0,y:s.xy.1+offset.1,t});}
            },
            wl_pointer::Event::Button {button:273,state:WEnum::Value(wl_pointer::ButtonState::Pressed),..}=>{if let Some(id)=s.target{s.refocus=keep_refocus(s.refocus,id);s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));}s.inputs.push(Event::Context{x:s.xy.0+offset.0,y:s.xy.1+offset.1,t});},
            wl_pointer::Event::Leave {..} if !s.pressed=>s.inputs.push(Event::Hover{x:-100.,y:-100.,t}),
            wl_pointer::Event::Button { button:272,state:WEnum::Value(v),serial,.. } => {
                s.serial=serial;
                s.pressed = v == wl_pointer::ButtonState::Pressed;
                if s.pressed {if let Some(id)=s.target {s.refocus=keep_refocus(s.refocus,id);s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));}}
                let drags=s.target.and_then(|id|s.windows.get(&id)).is_some_and(|w|{let (x,y)=(s.xy.0 as i32,s.xy.1 as i32);
                    s.xy.1<w.frame.drag_height as f32 && !w.frame.no_drag.iter().any(|r|x>=r[0]&&x<r[0]+r[2]&&y>=r[1]&&y<r[1]+r[3])});
                // the traffic lights are in `no_drag` (the shell lists them for every window), so a press on one still reaches
                // the shell as a click; a move grab takes the release, so the press is over for the shell as well
                if s.pressed && drags {
                    if let (Some(w),Some(seat)) = (s.target.and_then(|id| s.windows.get(&id)),s.seat.as_ref()) { w.top._move(seat,serial); s.pressed=false; return; }
                }
                s.inputs.push(if s.pressed { Event::Down { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t } } else { Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t } });
            },
            wl_pointer::Event::Leave {..} if s.pressed => { s.pressed = false; s.inputs.push(Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t }); },
            wl_pointer::Event::Axis { axis:WEnum::Value(wl_pointer::Axis::VerticalScroll),value,.. } => {
                s.inputs.push(Event::Wheel { x:s.xy.0+offset.0,y:s.xy.1+offset.1,dx:0.0,dy:-value as f32*3.0,t });
                s.inputs.extend([Event::Down { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t },Event::Move { x:s.xy.0+offset.0,y:s.xy.1+offset.1-value as f32*3.0,t:t+16.0 },Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1-value as f32*3.0,t:t+32.0 }]);
            },
            _ => {}
        }
    }
}
/// Where the surface a finger landed on sits on screen, and the window it belongs to (if it is an app window).
/// The desktop surface and the lock surface fill the screen.
fn touch_surface(s: &State, surface: &wl_surface::WlSurface) -> (Placement, Option<u32>) {
    if let Some((&id, w)) = s.windows.iter().find(|(_, w)| &w.surface == surface) {
        return (Placement { offset: (w.frame.x as f32, w.frame.y as f32), margin: w.margin as f32 }, Some(id));
    }
    if let Some(w) = s.chrome.values().find(|w| &w.surface == surface) {
        return (Placement { offset: (w.frame.x as f32, w.frame.y as f32), margin: 0.0 }, None);
    }
    (Placement::default(), None)
}
impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(s: &mut Self, _: &wl_touch::WlTouch, e: wl_touch::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let t = super::sys::now_ms();
        match e {
            wl_touch::Event::Down { serial,surface,id,x,y,.. } => {
                s.serial = serial;
                let (place, window) = touch_surface(s, &surface);
                if let Some(wid) = window { s.refocus = keep_refocus(s.refocus, wid); s.focused = Some(wid); s.window_events.push(WindowEvent::Focus(wid)); }
                // the first finger on a window's title area moves the window, as a pointer press there does
                let drags = s.fingers.is_idle() && window.and_then(|wid| s.windows.get(&wid)).is_some_and(|w| touch_model::starts_window_drag(place.local(x, y), w.frame.drag_height, &w.frame.no_drag));
                if drags {
                    if let (Some(w), Some(seat)) = (window.and_then(|wid| s.windows.get(&wid)), s.seat.as_ref()) { w.top._move(seat, serial); return; }
                }
                s.fingers.down(id, x, y, place, t);
            },
            wl_touch::Event::Motion { id,x,y,.. } => s.fingers.motion(id, x, y, t),
            wl_touch::Event::Up { serial,id,.. } => { s.serial = serial; s.fingers.up(id, t); },
            wl_touch::Event::Frame => { s.fingers.frame(); s.fingers.take(&mut s.inputs); },
            wl_touch::Event::Cancel => { s.fingers.cancel(t); s.fingers.take(&mut s.inputs); },
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(s: &mut Self, _: &wl_keyboard::WlKeyboard, e: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            wl_keyboard::Event::Enter {surface,keys,..}=>{
                // the keys already down as the shell gets the keyboard: a modifier held across the focus change counts
                // (the modifiers event that follows also carries the Caps Lock state)
                let held=super::input::mods_of_codes(keys.chunks_exact(4).map(|b|u32::from_ne_bytes([b[0],b[1],b[2],b[3]]) as u16));
                s.inputs.push(Event::Modifiers {held,caps:None});
                if let Some(id)=s.windows.iter().find(|(_,w)|w.surface==surface).map(|(&id,_)|id) {
                    let fresh=s.windows.get_mut(&id).is_some_and(|w|std::mem::take(&mut w.fresh));
                    if follows_keyboard_enter(fresh,id,s.focused) {
                        s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));
                    } else if let Some(front)=s.focused.filter(|f|s.windows.contains_key(f)) {
                        // the window server focused a window it just mapped (one that came back from Overview or a
                        // minimize behind the front): the front window the shell raised gets the keyboard back, after the
                        // events read with this one (`window_events`): the window server focuses a window on a press
                        // before the press reaches the client, and a press on the entered window keeps it (E2E N2)
                        s.refocus=Some((front,id));
                    }
                }
            },
            wl_keyboard::Event::Key {key,state:WEnum::Value(v),serial,..}=>{
                s.serial=serial; let down=v==wl_keyboard::KeyState::Pressed;
                if down {s.keys.insert(key as u16);} else {s.keys.remove(&(key as u16));}
                s.inputs.push(Event::Key {code:key as u16,down});
            },
            // the keyboard went to another client: its releases no longer reach the shell, so nothing counts as held (E2E3 F1)
            wl_keyboard::Event::Leave {..}=>{for code in s.keys.drain(){s.inputs.push(Event::Key {code,down:false});} s.inputs.push(Event::Modifiers {held:0,caps:None});},
            // the window server's modifier state is the authority: it corrects a counter whose press or release the shell missed
            wl_keyboard::Event::Modifiers {mods_depressed,mods_latched,mods_locked,..}=>{
                s.inputs.push(Event::Modifiers {held:super::input::mods_of_xkb(mods_depressed|mods_latched),caps:Some(super::input::xkb_caps(mods_locked))});
            },
            _=>{}
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: ignore wl_region::WlRegion);
wayland_client::delegate_noop!(State: ignore dmabuf_params::ZwpLinuxBufferParamsV1);
impl Dispatch<dmabuf::ZwpLinuxDmabufV1, ()> for State {
    fn event(s: &mut Self, _: &dmabuf::ZwpLinuxDmabufV1, e: dmabuf::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            dmabuf::Event::Modifier { format, modifier_hi, modifier_lo } => s.dmabuf_formats.push((format, (modifier_hi as u64) << 32 | modifier_lo as u64)),
            dmabuf::Event::Format { format } => s.dmabuf_formats.push((format, crate::gpu::desktop::MOD_INVALID)),
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(State: ignore layer::ZwlrLayerShellV1);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut Self, obj: &xdg_wm_base::XdgWmBase, e:xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping {serial} = e {obj.pong(serial);}
    }
}
impl Dispatch<xdg_surface::XdgSurface,u32> for State {
    fn event(s: &mut Self, obj: &xdg_surface::XdgSurface, e:xdg_surface::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        // a new toplevel's configures focus only the window the shell put in front: one that came back behind it (from
        // Overview or a minimize) and that the window server activated as it mapped must not lift itself over the
        // front one (W4-1, W4-5)
        let front=s.focused;
        if let xdg_surface::Event::Configure {serial} = e {obj.ack_configure(serial); if let Some(w)=s.windows.get_mut(id) {let focus=follows_keyboard_enter(w.fresh,*id,front)&&focus_on_configure(!w.configured,w.act,w.act_seen);w.configured=true;w.act_seen=w.act;if focus {s.window_events.push(WindowEvent::Focus(*id));}} }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel,u32> for State {
    fn event(s: &mut Self, _: &xdg_toplevel::XdgToplevel, e:xdg_toplevel::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        match e {
            xdg_toplevel::Event::Configure {width,height,states} => { let has=|state:xdg_toplevel::State|states.chunks_exact(4).any(|bytes|u32::from_ne_bytes(bytes.try_into().unwrap())==state as u32); let maximized=has(xdg_toplevel::State::Maximized); if let Some(w)=s.windows.get_mut(id) {w.act=has(xdg_toplevel::State::Activated);}
                // full screen: the answer to our own request settles it; any other configure is the window server's own decision.
                // It goes before the maximize state: a window leaving full screen comes back filled, and the shell must
                // see it leave full screen first, or it takes the maximize as a new zoom and saves the full-screen frame
                // as the way back (W3-6)
                let fullscreen=has(xdg_toplevel::State::Fullscreen);
                // a configure sent before the window server saw our own full-screen request (or one of a full-screen
                // window, which is never maximized) says nothing about the zoom
                let mut zoom_known=!fullscreen;
                if let Some(w)=s.windows.get_mut(id) { match w.fs_pending { Some(want) if want==fullscreen => w.fs_pending=None, Some(_) => zoom_known=false, None if w.frame.fullscreen!=fullscreen => { w.frame.fullscreen=fullscreen; s.window_events.push(WindowEvent::Fullscreen(*id,fullscreen)); }, None => {} } }
                if zoom_known {s.window_events.push(WindowEvent::Maximized(*id,maximized));}
                // every configure carries the frame the window server holds now: it keeps the pending size in step with
                // the size the window settled on, so an activation change no longer repeats an old tile size (W4-3), and
                // a configure is the size to take even when an earlier one carried the same (a tile again after a zoom)
                if width>0 && height>0 {s.window_events.push(WindowEvent::Resize(*id,width,height));} },
            xdg_toplevel::Event::Close => s.window_events.push(WindowEvent::Close(*id)),
            _=>{}
        }
    }
}

impl Dispatch<wl_output::WlOutput,()> for State {
    fn event(s:&mut Self,_:&wl_output::WlOutput,e:wl_output::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let wl_output::Event::Mode {flags:WEnum::Value(flags),refresh,..}=e {
            if flags.contains(wl_output::Mode::Current) {s.refresh=(refresh.max(1) as u32+500)/1000;crate::shell::services::display_services::OUTPUT_REFRESH_HZ.store(s.refresh,std::sync::atomic::Ordering::Relaxed);}
        }
    }
}
impl Dispatch<lock::ExtSessionLockV1,()> for State {
    fn event(s:&mut Self,_:&lock::ExtSessionLockV1,e:lock::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match e {lock::Event::Locked=>{if let Some(l)=s.lock.as_mut(){l.locked=true;}},lock::Event::Finished=>s.closed=true,_=>{}}
    }
}
impl Dispatch<lock_surface::ExtSessionLockSurfaceV1,()> for State {
    fn event(s:&mut Self,obj:&lock_surface::ExtSessionLockSurfaceV1,e:lock_surface::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let lock_surface::Event::Configure {serial,width,height}=e {
            obj.ack_configure(serial);
            if (width,height)!=s.size {s.closed=true;}
            if let Some(l)=s.lock.as_mut(){l.configured=true;}
            for slot in &mut s.slots {slot.damage.all();}
            s.redraw=true;
        }
    }
}
wayland_client::delegate_noop!(State: ignore lock_manager::ExtSessionLockManagerV1);

fn copy_damage(src:&[u32],dst:&mut [u8],width:usize,damage:&Damage) {
    assert_eq!(dst.len()%4,0);
    assert!(src.len()*4<=dst.len());
    // Row slices of 4-byte chunks: no unsafe pointer writes, no alignment
    // assumption, and a straight-line body the compiler can vectorize.
    // to_ne_bytes keeps the output bytes identical to a native u32 store.
    for r in damage.areas() {
        for y in r.y0..r.y1 {
            let start=y*width+r.x0; let end=y*width+r.x1;
            let out=&mut dst[start*4..end*4];
            for (o,&p) in out.chunks_exact_mut(4).zip(&src[start..end]) {
                let xrgb=(p&0xff000000) | (p&0xff00) | ((p&255)<<16) | ((p>>16)&255);
                o.copy_from_slice(&xrgb.to_ne_bytes());
            }
        }
    }
}
/// A desktop frame dropped because the compositor still held every buffer. Right after an unlock it holds the lock
/// surface's last frame and the one queued behind it (the pool had shrunk to one while the login window idled), and
/// nothing else redraws an idle shell: the desktop surface stayed without a buffer, black behind the bar and dock,
/// until the next minute's clock tick. The next release of a desktop buffer asks for a full redraw instead.
#[derive(Debug, Default, PartialEq, Clone, Copy)]
struct Starved(bool);
impl Starved {
    /// `present` found no free buffer and skipped the frame.
    fn dropped(&mut self) { self.0 = true; }
    /// A desktop buffer came back: true (once) when a skipped frame is owed.
    fn released(&mut self) -> bool { std::mem::take(&mut self.0) }
}

const GPU_POOL_MIN: usize = 1;
const GPU_POOL_MAX: usize = 2;
const GPU_POOL_IDLE_SECS: f32 = 5.0;
/// The drawables the background pool may hold. Under a session lock the desktop surface stays mapped and the window
/// server keeps its last buffer for as long as the lock lasts, while the lock surface holds the frame on screen: with two
/// drawables both were held after the first lock frame, every later frame was dropped (W5-16: typed dots, the shake
/// and "Incorrect password" never appeared, the clock froze), and the release that repays a dropped frame never came.
/// The lock surface gets one drawable of its own on top.
fn gpu_pool_max(locked: bool) -> usize { if locked { GPU_POOL_MAX + 1 } else { GPU_POOL_MAX } }
#[derive(Debug, PartialEq, Clone, Copy)]
enum PoolStep { Keep, Grow, Shrink }
/// Buffer-pool policy for the full-screen drawables. `busy[i]`: the compositor still holds drawable i.
/// Grow only when a frame needs drawing and every drawable is held; shrink the last one after the
/// idle period, and only if the compositor is not holding it.
fn gpu_pool_step(busy: &[bool], need_frame: bool, idle_secs: f32, max: usize) -> PoolStep {
    let n = busy.len();
    if need_frame && n < max && busy.iter().all(|&b| b) { PoolStep::Grow }
    else if !need_frame && n > GPU_POOL_MIN && idle_secs >= GPU_POOL_IDLE_SECS && !busy[n - 1] { PoolStep::Shrink }
    else { PoolStep::Keep }
}
/// Ms left before an idle pool of `n` drawables shrinks, None when it is already at its minimum or the spare is
/// still held by the compositor (it holds the on-screen buffer until the next attach; its release wakes the loop).
/// Arming a deadline for a held spare made it fire every millisecond (130 wakeups/s on Midget, 0.5.16).
fn pool_trim_in_ms(n: usize, spare_busy: bool, idle_secs: f32) -> Option<i32> {
    (n > GPU_POOL_MIN && !spare_busy).then(|| ((GPU_POOL_IDLE_SECS - idle_secs).max(0.0) * 1000.0).ceil() as i32 + 1)
}
/// The screen edge the Dock (chrome id 2) sits on; padding does not change its edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DockEdge {Bottom,Left,Right}
/// Where the Dock goes: its edge, the layer-surface margins (top, right, bottom, left) and the exclusive zone. A side Dock
/// is anchored to three edges (the side plus top and bottom) so its zone counts, and the margins put it at its own y:
/// with both opposite edges anchored and a fixed size the surface is centred in what the margins leave, so the margins
/// are the gap above and below it. The explicit work-area reserve excludes transparent surface padding; anchored margins are counted separately.
fn dock_placement(x:i32,y:i32,width:usize,height:usize,screen:(u32,u32),dock:DockPlacement,top_reserved:i32)->(DockEdge,[i32;4],i32) {
    let (w,h)=(width as i32,height as i32);let (sw,sh)=(screen.0 as i32,screen.1 as i32);
    // Positive exclusive zones are arranged below the menu bar. A non-reserving overlay uses the full output.
    let top=(y-if dock.reserve>0{top_reserved}else{0}).max(0);
    let bottom=(sh-y-h).max(0);let left=x.max(0);let right=(sw-x-w).max(0);
    // The layer protocol adds the anchored margin to its zone; count the edge gap exactly once.
    let zone=|margin:i32|if dock.reserve>0{(dock.reserve-margin).max(1)}else{-1};
    match dock.edge{
        DockEdge::Left=>(dock.edge,[top,0,bottom,left],zone(left)),
        DockEdge::Right=>(dock.edge,[top,right,bottom,0],zone(right)),
        DockEdge::Bottom=>(dock.edge,[0,right,bottom,left],zone(bottom)),
    }
}
/// Whether a configure of a toplevel tells the shell it has the focus: the first one (a new window is focused as it
/// appears) or one that newly carries the Activated state. A size, tile or state configure on a window in the
/// background must not lift it over the one the user is working in.
fn focus_on_configure(first:bool,activated:bool,was_activated:bool)->bool { first || (activated && !was_activated) }
/// Whether the shell follows the window server's keyboard focus (or activation) of window `id`. The window server raises
/// and focuses every toplevel it maps, so a window's first focus (`fresh`) while the shell has another window in front
/// (`front`: the last one it raised, or the one the person pressed) is the map of a window that came back behind it
/// (Overview, a minimize, a window covered until now), not a choice of the person: following it lifted that window over
/// the one the Dock, Overview or the Window menu had just brought forward (W4-1, W4-5).
fn follows_keyboard_enter(fresh:bool,id:u32,front:Option<u32>)->bool { !fresh || front.map_or(true,|f|f==id) }
/// The keyboard hand-back still owed after a press on window `pressed`: none when the press is on the fresh window that
/// just took the keyboard (the person chose it), else the one owed before.
fn keep_refocus(refocus:Option<(u32,u32)>,pressed:u32)->Option<(u32,u32)> { refocus.filter(|(_,entered)|*entered!=pressed) }
/// Asks the window server to activate (raise and focus) the shell's window `id`, with the shell's last input serial.
fn request_activation(s:&State,qh:&QueueHandle<State>,id:u32) {
    if let (Some(activation),Some(seat),Some(surface))=(s.activation.as_ref(),s.seat.as_ref(),s.surface.as_ref()) {
        let token=activation.get_activation_token(qh,id);
        token.set_serial(s.serial,seat);
        token.set_surface(surface);
        token.commit();
    }
}
// Keep quiet windows single-buffered. A pending update may allocate one spare
// while the compositor owns the first; never write into an in-flight buffer.
fn window_buffer_count(count:usize, available:bool, dirty:bool) -> usize {
    if count == 0 { 1 } else if count == 1 && !available && dirty { 2 } else { count }
}
#[cfg(test)]
mod tests {
    use super::{dock_placement,DockEdge,DockPlacement,focus_on_configure,keep_refocus,min_size_of,DEFAULT_MIN_SIZE};
    /// N11: a glass shape that went away is owed to a chrome surface under it, in the surface's own pixels and clipped
    /// to it; a shape elsewhere owes it nothing.
    #[test]
    fn a_glass_shape_that_changes_is_owed_to_the_chrome_surface_under_it() {
        let mut pending = crate::gfx::Damage::new(2560, 1300);
        super::owe_glass(&mut pending, (0, 35), &[(450.0, 1190.0, 1650.0, 150.0), (0.0, 0.0, 100.0, 30.0)]);
        let a = pending.areas();
        assert_eq!(a.len(), 1, "the shape above the surface owes it nothing: {a:?}");
        assert_eq!((a[0].x0, a[0].y0, a[0].x1, a[0].y1), (450, 1155, 2101, 1300), "moved to the surface and clipped to it");
        let mut none = crate::gfx::Damage::new(100, 100);
        super::owe_glass(&mut none, (0, 0), &[]);
        assert!(none.is_empty());
    }
    #[test]
    fn a_toplevel_advertises_the_frames_minimum_or_the_default() {
        assert_eq!(min_size_of((940,600)),(940,600), "the app's own minimum, e.g. Settings' split width");
        assert_eq!(min_size_of((0,0)),DEFAULT_MIN_SIZE, "a frame without one keeps the default");
        assert_eq!(min_size_of((500,0)),DEFAULT_MIN_SIZE, "a half-set minimum is not sent");
    }
    #[test]
    fn a_plain_configure_does_not_take_the_focus() {
        assert!(focus_on_configure(true,false,false), "a new window is focused");
        assert!(focus_on_configure(false,true,false), "a window that became active is focused");
        assert!(!focus_on_configure(false,false,false), "a resize of a background window");
        assert!(!focus_on_configure(false,true,true), "a resize of the active window raises nothing again");
    }
    /// E2E N2: a window that came back from Hide is fresh; a press on it gets the keyboard from the window server before
    /// the press reaches the shell, and that keyboard must stay with it. A press elsewhere still owes the front window.
    #[test]
    fn a_press_on_the_fresh_window_that_took_the_keyboard_keeps_it() {
        let (front, viewer, other) = (1, 0x0100_2002, 7);
        assert_eq!(keep_refocus(Some((front, viewer)), viewer), None, "the person clicked the window: no hand-back");
        assert_eq!(keep_refocus(Some((front, viewer)), other), Some((front, viewer)), "a press on another window keeps it owed");
        assert_eq!(keep_refocus(None, viewer), None);
    }
    /// W4-1/W4-5: the window server focuses every window it maps; a window that maps behind the front one (back from
    /// Overview or a minimize) must not take the front from the window the shell just raised.
    #[test]
    fn a_window_mapped_behind_the_front_one_does_not_take_the_focus() {
        assert!(!follows_keyboard_enter(true,7,Some(3)), "the map focus of a window behind the front is not followed");
        assert!(follows_keyboard_enter(true,3,Some(3)), "the front window's own first focus is");
        assert!(follows_keyboard_enter(true,7,None), "with nothing in front, the new window is the front");
        assert!(follows_keyboard_enter(false,7,Some(3)), "a window that had the keyboard before follows the window server");
    }
    #[test]
    fn a_shaped_window_takes_input_only_where_it_drew() {
        assert_eq!(super::Display::input_rects(12,300,200,None),vec![[12,12,300,200]],"the body inside the shadow");
        let shape=[[10,20,100,50],[0,0,0,5]];
        assert_eq!(super::Display::input_rects(0,300,200,Some(&shape)),vec![[10,20,100,50]],"empty rects are dropped");
        assert_eq!(super::Display::input_rects(4,300,200,Some(&shape)),vec![[14,24,100,50]]);
        assert!(super::Display::input_rects(0,300,200,Some(&[])).is_empty(),"nothing drawn: no input at all");
    }
    #[test]
    fn the_dock_is_anchored_to_its_own_edge() {
        let screen=(1920,1080);
        let at=|x,y,w,h,edge,reserve|dock_placement(x,y,w,h,screen,DockPlacement{edge,reserve},30);
        assert_eq!(at(500,1000,900,70,DockEdge::Bottom,80),(DockEdge::Bottom,[0,520,10,500],70));
        assert_eq!(at(12,200,70,600,DockEdge::Left,82),(DockEdge::Left,[170,0,280,12],70));
        assert_eq!(at(1838,200,70,600,DockEdge::Right,82),(DockEdge::Right,[170,12,280,0],70));
        assert_eq!(at(0,30,3,1050,DockEdge::Left,0),(DockEdge::Left,[30,0,0,0],-1));
        assert_eq!(at(1917,30,3,1050,DockEdge::Right,0),(DockEdge::Right,[30,0,0,0],-1));
    }
    #[test]
    fn transparent_dock_room_never_changes_its_edge_or_reserved_depth() {
        let screen=(1920,1080);
        let side=DockPlacement{edge:DockEdge::Left,reserve:82};
        // A label makes a side surface wider than it is tall: its edge still comes from the preference.
        assert_eq!(dock_placement(0,400,600,300,screen,side,30),(DockEdge::Left,[370,0,380,0],82));
        let bottom=DockPlacement{edge:DockEdge::Bottom,reserve:82};
        for height in [80,160,320] {
            assert_eq!(dock_placement(500,1080-height as i32,900,height,screen,bottom,30).2,82,
                "transparent room for labels or bounce reserves no extra space");
        }
    }
    #[test]
    fn a_frame_dropped_for_want_of_a_buffer_is_redrawn_on_the_next_release() {
        let mut s = super::Starved::default();
        assert!(!s.released(), "a release with nothing owed asks for nothing");
        s.dropped();
        s.dropped();
        assert!(s.released(), "the next release redraws the dropped frame");
        assert!(!s.released(), "once");
    }

    #[test]
    fn a_full_or_empty_socket_is_not_a_broken_connection() {
        use wayland_client::backend::WaylandError;
        let would_block: Result<(), WaylandError> = Err(WaylandError::Io(std::io::Error::from(std::io::ErrorKind::WouldBlock)));
        assert!(super::transient(would_block).is_ok());
        let broken: Result<(), WaylandError> = Err(WaylandError::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe)));
        assert!(super::transient(broken).is_err());
        assert!(super::transient::<usize>(Ok(3)).is_ok());
    }

    use super::*;
    #[test]
    fn trim_deadline_is_one_shot() {
        assert_eq!(pool_trim_in_ms(GPU_POOL_MIN, false, 0.0), None);
        assert_eq!(pool_trim_in_ms(GPU_POOL_MIN + 1, true, 9.0), None, "a held spare arms no deadline");
        assert_eq!(pool_trim_in_ms(GPU_POOL_MIN + 1, false, 1.0), Some(4001));
        assert_eq!(pool_trim_in_ms(GPU_POOL_MIN + 1, false, 9.0), Some(1));
    }
    #[test]
    fn gpu_pool_grows_only_when_all_held_and_shrinks_when_idle() {
        assert_eq!(gpu_pool_step(&[true], true, 0.0, GPU_POOL_MAX), PoolStep::Grow);
        assert_eq!(gpu_pool_step(&[false], true, 0.0, GPU_POOL_MAX), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[true, true], true, 0.0, GPU_POOL_MAX), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[false, false], false, 4.9, GPU_POOL_MAX), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[true, false], false, 6.0, GPU_POOL_MAX), PoolStep::Shrink);
        assert_eq!(gpu_pool_step(&[false, true], false, 6.0, GPU_POOL_MAX), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[false], false, 60.0, GPU_POOL_MAX), PoolStep::Keep);
    }
    #[test]
    fn under_a_lock_the_pool_grows_past_the_held_desktop_buffer() {
        // the desktop surface keeps one drawable, the lock surface the frame on screen: a third one draws the next frame
        assert_eq!(gpu_pool_step(&[true, true], true, 0.0, gpu_pool_max(false)), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[true, true], true, 0.0, gpu_pool_max(true)), PoolStep::Grow, "W5-16: typed dots must reach the screen");
        assert_eq!(gpu_pool_step(&[true, true, true], true, 0.0, gpu_pool_max(true)), PoolStep::Keep);
        // after the unlock the extra drawable goes once it is idle and free
        assert_eq!(gpu_pool_step(&[true, false, false], false, 6.0, gpu_pool_max(false)), PoolStep::Shrink);
    }
    #[test]
    fn window_buffers_grow_only_when_a_dirty_frame_would_wait() {
        assert_eq!(window_buffer_count(0,false,false),1);
        assert_eq!(window_buffer_count(1,false,false),1);
        assert_eq!(window_buffer_count(1,true,true),1);
        assert_eq!(window_buffer_count(1,false,true),2);
        assert_eq!(window_buffer_count(2,false,true),2);
    }
    #[test]
    fn shm_conversion_preserves_channels_and_undamaged_pixels() {
        let src=[crate::gfx::pack(255,0,0,255),crate::gfx::pack(0,255,0,255),crate::gfx::pack(0,0,255,255),crate::gfx::pack(12,34,56,255)];
        let mut dst=vec![77;16]; let mut d=Damage::new(2,2);d.add(0,1,2,1);
        copy_damage(&src,&mut dst,2,&d);
        assert_eq!(&dst[..8],&[77;8]);
        assert_eq!(&dst[8..],&[255,0,0,255,56,34,12,255]);
    }
    #[test]
    fn copy_damage_matches_scalar_reference() {
        fn reference(src:&[u32],dst:&mut [u8],width:usize,damage:&Damage) {
            for r in damage.areas() { for y in r.y0..r.y1 { for x in r.x0..r.x1 {
                let p=src[y*width+x];
                let xrgb=(p&0xff000000) | (p&0xff00) | ((p&255)<<16) | ((p>>16)&255);
                dst[(y*width+x)*4..(y*width+x)*4+4].copy_from_slice(&xrgb.to_ne_bytes());
            }}}
        }
        let (w,h)=(7usize,5usize);
        let src:Vec<u32>=(0..w*h).map(|i|(i as u32).wrapping_mul(0x9E3779B9)).collect();
        let mut d=Damage::new(w,h); d.add(1,0,6,2); d.add(0,3,3,5);
        let mut a=vec![0xAAu8;w*h*4]; let mut b=a.clone();
        copy_damage(&src,&mut a,w,&d);
        reference(&src,&mut b,w,&d);
        assert_eq!(a,b);
    }
}

wayland_client::delegate_noop!(State: ignore activation::XdgActivationV1);
impl Dispatch<activation_token::XdgActivationTokenV1,u32> for State {
    fn event(s:&mut Self,obj:&activation_token::XdgActivationTokenV1,e:activation_token::Event,id:&u32,_:&Connection,_:&QueueHandle<Self>) {
        if let activation_token::Event::Done {token}=e {
            if let (Some(a),Some(w))=(s.activation.as_ref(),s.windows.get(id)) {a.activate(token,&w.surface);}
            obj.destroy();
        }
    }
}

wayland_client::delegate_noop!(State: ignore idle_notifier::ExtIdleNotifierV1);
wayland_client::delegate_noop!(State: ignore power_manager::ZwlrOutputPowerManagerV1);
wayland_client::delegate_noop!(State: ignore output_power::ZwlrOutputPowerV1);
wayland_client::delegate_noop!(State: ignore om_mode::ZwlrOutputModeV1);
wayland_client::delegate_noop!(State: ignore om_config_head::ZwlrOutputConfigurationHeadV1);
// zwlr_output_manager_v1: the heads (the first one is the output the shell asks to change) and the serial of the list
impl Dispatch<om_manager::ZwlrOutputManagerV1, ()> for State {
    fn event(s:&mut Self,_:&om_manager::ZwlrOutputManagerV1,e:om_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match e {
            om_manager::Event::Head { head } => { if s.output_head.is_none() { s.output_head = Some(head); } }
            om_manager::Event::Done { serial } => s.output_serial = serial,
            om_manager::Event::Finished => s.output_manager = None,
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, om_manager::ZwlrOutputManagerV1, [om_manager::EVT_HEAD_OPCODE => (om_head::ZwlrOutputHeadV1, ())]);
}
impl Dispatch<om_head::ZwlrOutputHeadV1, ()> for State {
    fn event(_:&mut Self,_:&om_head::ZwlrOutputHeadV1,_:om_head::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
    wayland_client::event_created_child!(State, om_head::ZwlrOutputHeadV1, [om_head::EVT_MODE_OPCODE => (om_mode::ZwlrOutputModeV1, ())]);
}
// zwlr_output_configuration_v1: the answer to a mode request
impl Dispatch<om_config::ZwlrOutputConfigurationV1, ()> for State {
    fn event(s:&mut Self,_:&om_config::ZwlrOutputConfigurationV1,e:om_config::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        let ok = match e {
            om_config::Event::Succeeded => true,
            om_config::Event::Failed | om_config::Event::Cancelled => false,
            _ => return,
        };
        s.output_busy = false;
        *crate::shell::services::display_services::MODE_RESULT.lock().unwrap_or_else(|p| p.into_inner()) = Some(ok);
    }
}
impl Dispatch<idle_notification::ExtIdleNotificationV1,()> for State {
    fn event(s:&mut Self,_:&idle_notification::ExtIdleNotificationV1,e:idle_notification::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match e {idle_notification::Event::Idled=>s.idle=true,idle_notification::Event::Resumed=>{s.idle=false;s.resumed=true;},_=>{}}
    }
}

wayland_client::delegate_noop!(State: ignore cursor_manager::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(State: ignore cursor_device::WpCursorShapeDeviceV1);

impl Dispatch<foreign_manager::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(s:&mut Self,_:&foreign_manager::ZwlrForeignToplevelManagerV1,e:foreign_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let foreign_manager::Event::Toplevel { toplevel }=e {
            let id=toplevel.id().protocol_id();
            s.foreign.push(Foreign {handle:toplevel,info:ForeignWindow {id,title:String::new(),app_id:String::new(),active:false,maximized:false,minimized:false}});
        }
    }
    wayland_client::event_created_child!(State, foreign_manager::ZwlrForeignToplevelManagerV1, [0 => (foreign_handle::ZwlrForeignToplevelHandleV1, ())]);
}
impl Dispatch<foreign_handle::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(s:&mut Self,obj:&foreign_handle::ZwlrForeignToplevelHandleV1,e:foreign_handle::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let foreign_handle::Event::Closed=e {
            s.foreign.retain(|w|w.handle!=*obj); obj.destroy(); s.foreign_dirty=true; return;
        }
        if let Some(w)=s.foreign.iter_mut().find(|w|w.handle==*obj) { match e {
            foreign_handle::Event::Title {title} => w.info.title=title,
            foreign_handle::Event::AppId {app_id} => w.info.app_id=app_id,
            foreign_handle::Event::State {state} => {w.info.active=state.chunks_exact(4).any(|v|u32::from_ne_bytes(v.try_into().unwrap())==2);w.info.maximized=state.chunks_exact(4).any(|v|u32::from_ne_bytes(v.try_into().unwrap())==0);w.info.minimized=state.chunks_exact(4).any(|v|u32::from_ne_bytes(v.try_into().unwrap())==1);},
            foreign_handle::Event::Done => s.foreign_dirty=true,
            _ => {}
        }}
    }
}

/// A window's material rects as blur-region rectangles in surface pixels (x, y, w, h): the materials are
/// window-local and the surface starts `margin` px before the content (the shadow margin). The window's screen
/// position is the compositor's, so it does not enter the region.
fn blur_rects(materials:&[Glass], margin:i32) -> Vec<[i32;4]> {
    materials.iter().filter(|g|g.w>0.0&&g.h>0.0)
        .map(|g|[g.x.floor() as i32+margin,g.y.floor() as i32+margin,(g.w.ceil() as i32).max(1),(g.h.ceil() as i32).max(1)]).collect()
}

impl Dispatch<bg_manager::ExtBackgroundEffectManagerV1,()> for State {
    fn event(s:&mut Self,_:&bg_manager::ExtBackgroundEffectManagerV1,e:bg_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let bg_manager::Event::Capabilities{flags}=e { s.bg_blur=matches!(flags,WEnum::Value(f) if f.contains(bg_manager::Capability::Blur)); }
    }
}
impl Dispatch<im_manager::ZwpInputMethodManagerV2,()> for State {
    fn event(_:&mut Self,_:&im_manager::ZwpInputMethodManagerV2,_:im_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<vk_manager::ZwpVirtualKeyboardManagerV1,()> for State {
    fn event(_:&mut Self,_:&vk_manager::ZwpVirtualKeyboardManagerV1,_:vk_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<vk::ZwpVirtualKeyboardV1,()> for State {
    fn event(_:&mut Self,_:&vk::ZwpVirtualKeyboardV1,_:vk::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
/// Input method events accumulate into the batch being received; `done` ends it. The serial counts the `done`s, as
/// `commit` requires. `unavailable` means another input method holds the seat: the shell stops using it.
impl Dispatch<im::ZwpInputMethodV2,()> for State {
    fn event(s:&mut Self,_:&im::ZwpInputMethodV2,e:im::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match e {
            im::Event::Activate => s.im_pending.focus = Some(true),
            im::Event::Deactivate => s.im_pending.focus = Some(false),
            im::Event::SurroundingText { text, cursor, anchor } => s.im_pending.surrounding = Some((text, cursor, anchor)),
            im::Event::ContentType { purpose, .. } => s.im_pending.purpose = Some(match purpose {
                WEnum::Value(p) => p as u32,
                WEnum::Unknown(raw) => raw,
            }),
            im::Event::Done => {
                s.im_serial = s.im_serial.wrapping_add(1);
                s.im_batches.push(std::mem::take(&mut s.im_pending));
            }
            im::Event::Unavailable => s.im = None,
            _ => {}
        }
    }
}
impl Dispatch<mat_manager::BifrostMaterialManagerV1,()> for State {
    fn event(_:&mut Self,_:&mat_manager::BifrostMaterialManagerV1,_:mat_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<cap_manager::BifrostWindowCaptureManagerV1,()> for State {
    fn event(_:&mut Self,_:&cap_manager::BifrostWindowCaptureManagerV1,_:cap_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<cap_frame::BifrostWindowCaptureFrameV1,u64> for State {
    fn event(s:&mut Self,_:&cap_frame::BifrostWindowCaptureFrameV1,e:cap_frame::Event,token:&u64,_:&Connection,_:&QueueHandle<Self>) {
        let outcome=match e {
            cap_frame::Event::Ready{width,height,serial}=>{
                let picture=s.cap_jobs.get(token).and_then(|j|super::window_capture_client::read_picture(&j.map,j.stride,width as usize,height as usize));
                match picture { Some(px)=>ShotOutcome::Ready{image:crate::gfx::Image{w:width as usize,h:height as usize,px},serial}, None=>ShotOutcome::Failed }
            }
            cap_frame::Event::Unchanged{serial}=>ShotOutcome::Unchanged{serial},
            cap_frame::Event::Failed{..}=>ShotOutcome::Failed,
            _=>return,
        };
        if let Some(job)=s.cap_jobs.remove(token) { job.buffer.destroy(); job.frame.destroy(); }
        s.cap_results.push((*token,outcome));
    }
}
impl Dispatch<lvl_manager::BifrostWindowLevelManagerV1,()> for State {
    fn event(s:&mut Self,_:&lvl_manager::BifrostWindowLevelManagerV1,e:lvl_manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match e {
            // the window server's frame of one of our toplevels: the shell keeps its window rectangle from it
            lvl_manager::Event::Frame{surface,x,y,width,height} => {
                if let Some(id)=s.windows.iter().find(|(_,w)|w.surface==surface).map(|(&id,_)|id) {
                    s.window_events.push(WindowEvent::Frame(id,x,y,width,height));
                }
            }
            // how the window server applies Night Shift; a change of way sends the white point again
            lvl_manager::Event::WhitePointMode{mode} => { s.wm_white_point=super::window_level_client::wm_tints(mode); s.white_point_sent=None; }
        }
    }
}
impl Dispatch<mat_surface::BifrostMaterialSurfaceV1,()> for State {
    fn event(_:&mut Self,_:&mat_surface::BifrostMaterialSurfaceV1,_:mat_surface::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<bg_surface::ExtBackgroundEffectSurfaceV1,()> for State {
    fn event(_:&mut Self,_:&bg_surface::ExtBackgroundEffectSurfaceV1,_:bg_surface::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}

#[cfg(test)]
mod material_tests {
    use super::*;
    use bifrost_render::glass::Kind;
    #[test]
    fn blur_rects_are_offset_by_the_shadow_margin() {
        let g = Glass { x: 0.0, y: 30.5, w: 200.2, h: 400.0, r: 0.0, kind: Kind::Sidebar, active: None, detail: None, tint: [0.0; 4] };
        assert_eq!(blur_rects(&[g], 40), vec![[40, 70, 201, 400]]);
        assert!(blur_rects(&[], 40).is_empty());
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;

    const SCREEN: (usize, usize) = (1920, 1080);

    #[test]
    fn zoom_off_magnifies_nothing() {
        assert_eq!(magnify_for(ScreenZoom::default(), (960.0, 540.0), SCREEN), None);
    }

    #[test]
    fn full_screen_samples_the_pointer_centred_quarter_at_two_x() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Full };
        let m = magnify_for(z, (960.0, 540.0), SCREEN).unwrap();
        assert_eq!(m, Magnify { full: Some([0.25, 0.25, 0.75, 0.75]), lens: None });
    }

    #[test]
    fn lens_draws_its_rect_and_shows_the_same_factor() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Lens };
        let m = magnify_for(z, (500.0, 400.0), SCREEN).unwrap();
        assert_eq!(m.full, None);
        let (rect, uv) = m.lens.unwrap();
        assert_eq!(Some(rect), z.lens_rect((500.0, 400.0), (1920.0, 1080.0)));
        assert!((uv[2] - uv[0] - 180.0 / 1920.0).abs() < 1e-5, "the lens shows half its width");
    }
}
