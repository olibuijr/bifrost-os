//! BifrOSt's native Wayland display/input backend. The compositor owns KMS and
//! evdev; external apps submit their GPU buffers directly to it. Only the shell
//! raster uses SHM, with damage tracking and released-buffer ownership.
use std::{io, os::fd::AsFd};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, Proxy};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_output, wl_region, wl_seat, wl_shm, wl_shm_pool, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1 as layer, zwlr_layer_surface_v1 as surface};
use crate::gfx::Damage;
use wayland_protocols::xdg::shell::client::{xdg_wm_base, xdg_surface, xdg_toplevel};
use std::collections::{HashMap,HashSet};
use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notifier_v1 as idle_notifier, ext_idle_notification_v1 as idle_notification};
use wayland_protocols_wlr::output_power_management::v1::client::{zwlr_output_power_manager_v1 as power_manager,zwlr_output_power_v1 as output_power};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_v1 as activation,xdg_activation_token_v1 as activation_token};
use wayland_protocols::ext::session_lock::v1::client::{ext_session_lock_manager_v1 as lock_manager,ext_session_lock_v1 as lock,ext_session_lock_surface_v1 as lock_surface};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{zwlr_foreign_toplevel_manager_v1 as foreign_manager, zwlr_foreign_toplevel_handle_v1 as foreign_handle};
use wayland_protocols::ext::background_effect::v1::client::{ext_background_effect_manager_v1 as bg_manager, ext_background_effect_surface_v1 as bg_surface};
#[derive(Clone, PartialEq, Eq)]
pub struct ForeignWindow { pub id:u32, pub title:String, pub app_id:String, pub active:bool,pub maximized:bool }
struct Foreign { handle:foreign_handle::ZwlrForeignToplevelHandleV1, info:ForeignWindow }
struct Lock { obj:lock::ExtSessionLockV1, surface:wl_surface::WlSurface, role:lock_surface::ExtSessionLockSurfaceV1, configured:bool, locked:bool }


pub struct ChromeFrame {pub id:u32,pub x:i32,pub y:i32,pub width:usize,pub height:usize,pub pixels:Vec<u32>}
struct ChromeWindow {surface:wl_surface::WlSurface,layer:surface::ZwlrLayerSurfaceV1,slots:Vec<Slot>,configured:bool,frame:ChromeFrame,pending:Damage}
#[derive(Clone)]
pub struct WindowFrame { pub id:u32, pub title:String, pub x:i32, pub y:i32, pub width:usize, pub height:usize, pub pixels:Option<std::sync::Arc<[u32]>>, pub damage:Damage, pub maximized:bool, pub title_height:i32,
    /// A press above `drag_height` (window px) moves the window, unless it lands in one of `no_drag` (x, y, w, h):
    /// the traffic lights and an app's toolbar controls in a full-size-content window.
    pub drag_height:i32, pub no_drag:Vec<[i32;4]>,
    /// Open, close, minimize and zoom transform of the window (identity at rest), applied by the GPU presenter.
    pub fx:bifrost_render::glass::Fx }
pub enum WindowEvent { Focus(u32), Resize(u32,i32,i32), Close(u32), Maximized(u32,bool) }
struct Window { surface:wl_surface::WlSurface, xdg:xdg_surface::XdgSurface, top:xdg_toplevel::XdgToplevel, slots:Vec<Slot>, buffer_size:(usize,usize), configured:bool, frame:WindowFrame,
    /// GPU path: content damage not presented yet, the shadow margin around the content, the drawable size
    pending:Damage, margin:i32, gpu_size:(usize,usize),
    /// window-server materials (ext-background-effect): the effect object and the blur rects last sent (surface px)
    bg:Option<bg_surface::ExtBackgroundEffectSurfaceV1>, bg_rects:Vec<[i32;4]> }
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
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_manager_v1 as cursor_manager, wp_cursor_shape_device_v1 as cursor_device};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{zwp_linux_dmabuf_v1 as dmabuf, zwp_linux_buffer_params_v1 as dmabuf_params};
use crate::gpu::wl::{Backdrop, BufferInfo, Presenter};
use bifrost_render::glass::Rect as Glass;
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
#[derive(Default)]
struct State {
    bg_manager:Option<bg_manager::ExtBackgroundEffectManagerV1>, bg_blur:bool,
    foreign_manager:Option<foreign_manager::ZwlrForeignToplevelManagerV1>, foreign:Vec<Foreign>, foreign_dirty:bool,
    cursor_manager: Option<cursor_manager::WpCursorShapeManagerV1>, cursor_device: Option<cursor_device::WpCursorShapeDeviceV1>, pointer: Option<wl_pointer::WlPointer>, cursor_serial:u32,
    lock_manager:Option<lock_manager::ExtSessionLockManagerV1>, output:Option<wl_output::WlOutput>, lock:Option<Lock>, lock_wanted:bool, redraw:bool, refresh:u32,
    idle_notifier:Option<idle_notifier::ExtIdleNotifierV1>, idle:bool, resumed:bool, keys:HashSet<u16>, power_manager:Option<power_manager::ZwlrOutputPowerManagerV1>, output_power:Option<output_power::ZwlrOutputPowerV1>,
    activation:Option<activation::XdgActivationV1>, focused:Option<u32>, serial:u32, raise_front:bool, z:Vec<u32>,
    wm:Option<xdg_wm_base::XdgWmBase>, seat:Option<wl_seat::WlSeat>, windows:HashMap<u32,Window>, materials:HashMap<u32,Vec<Glass>>, scale:f32, window_events:Vec<WindowEvent>, target:Option<u32>,
    compositor: Option<wl_compositor::WlCompositor>, shm: Option<wl_shm::WlShm>,
    layer: Option<layer::ZwlrLayerShellV1>, surface: Option<wl_surface::WlSurface>,
    chrome:HashMap<u32,ChromeWindow>, chrome_target:Option<u32>, chrome_configured:bool,
    slots: Vec<Slot>, size: (u32,u32), configured: bool, closed: bool, main: MainMap, starved: Starved,
    pending: bool, desktop_flip: Option<f64>, glass_retry: bool, inputs: Vec<Event>, xy: (f32,f32), pressed: bool,
    dmabuf: Option<dmabuf::ZwpLinuxDmabufV1>, dmabuf_formats: Vec<(u32,u64)>, gpu_slots: HashMap<u32,Vec<GpuSlot>>,
}
/// The GPU presenter and what it needs between frames.
struct Gpu { p: Presenter, glass: Vec<Glass>, log: (std::time::Instant, usize, f64, f64, usize), draw: (usize, f64), last_frame: std::time::Instant }
pub struct Display { conn: Connection, queue: EventQueue<State>, state: State,
    stats: Option<(std::time::Instant,usize,usize)>, gpu: Option<Gpu>,
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
        let conn = Connection::connect_to_env().map_err(err)?;
        let mut queue = conn.new_event_queue(); let qh = queue.handle();
        let mut state = State::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state).map_err(err)?;
        let seat=state.seat.as_ref().ok_or_else(||err("Wayland seat unavailable"))?;
        state.idle_notifier.as_ref().ok_or_else(||err("Wayland idle notifier unavailable"))?.get_idle_notification(1000,seat,&qh,());
        if let (Some(manager),Some(output))=(state.power_manager.as_ref(),state.output.as_ref()) {state.output_power=Some(manager.get_output_power(output,&qh,()));}
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
                    Some(Gpu { p, glass: Vec::new(), log: (std::time::Instant::now(), 0, 0., 0., 0), draw: (0, 0.), last_frame: std::time::Instant::now() })
                }
                Err(e) => { bifrost_core::log::shell_logger("presenter").error(format!("presenter unavailable, SHM path: {e}")); state.gpu_slots.clear(); None }
            }
        };
        transient(conn.flush())?;
        Ok(Self { conn, queue, state, stats, gpu })
    }
    pub fn active(&self) -> bool {!self.state.idle}
    pub fn resumed(&mut self) -> bool {std::mem::take(&mut self.state.resumed)}
    pub fn power(&mut self,on:bool) -> io::Result<()> {
        if let Some(power)=&self.state.output_power {power.set_mode(if on {output_power::Mode::On} else {output_power::Mode::Off});transient(self.conn.flush())?;}
        Ok(())
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
            self.state.main.lock();
            self.state.surface.as_ref().unwrap().attach(None,0,0); self.state.surface.as_ref().unwrap().commit();
            self.state.lock = Some(Lock {obj,surface,role,configured:false,locked:false});
        } else if let Some(lock) = self.state.lock.take() {
            if !lock.locked { return Err(err("unlock attempted before compositor confirmed lock")); }
            lock.obj.unlock_and_destroy(); lock.role.destroy(); lock.surface.destroy();
            for slot in &mut self.state.slots { slot.damage.all(); }
            // the lock unmapped the desktop surface: map it again through a fresh initial commit and configure
            if self.state.main.unlock() { self.state.surface.as_ref().unwrap().commit(); }
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
    pub fn activate_foreign(&mut self,id:u32) -> io::Result<()> {
        if let (Some(w),Some(seat))=(self.state.foreign.iter().find(|w|w.info.id==id),self.state.seat.as_ref()) { w.handle.unset_minimized(); w.handle.activate(seat); transient(self.conn.flush())?; }
        Ok(())
    }
    pub fn window_events(&mut self) -> Vec<WindowEvent> { std::mem::take(&mut self.state.window_events) }
    pub fn windows(&mut self, frames: Vec<WindowFrame>) -> io::Result<()> {
        let ids:Vec<u32> = frames.iter().map(|f|f.id).collect();
        let front=ids.last().copied();
        self.state.z=ids.clone();
        let activate=front.filter(|id|Some(*id)!=self.state.focused);
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
                if w.frame.maximized != frame.maximized { if frame.maximized { w.top.set_maximized(); } else { w.top.unset_maximized(); } }
                for slot in &mut w.slots { slot.damage.union(&frame.damage); }
                if w.pending.size()==frame.damage.size() { w.pending.union(&frame.damage); } else { w.pending=Damage::new(frame.width,frame.height); w.pending.all(); }
                // a moved window with vibrancy shows a different part of the wallpaper
                if (w.frame.x,w.frame.y)!=(frame.x,frame.y) && self.state.materials.contains_key(&frame.id) { w.pending=Damage::new(frame.width,frame.height); w.pending.all(); }
                let pixels = frame.pixels.take().or_else(||w.frame.pixels.take());
                w.frame = frame;
                w.frame.pixels = pixels;
            } else {
                let surface = self.state.compositor.as_ref().unwrap().create_surface(&qh,());
                let xdg = self.state.wm.as_ref().ok_or_else(||err("xdg-shell unavailable"))?.get_xdg_surface(&surface,&qh,frame.id);
                let top = xdg.get_toplevel(&qh,frame.id); top.set_title(frame.title.clone()); top.set_app_id(format!("is.olibuijr.bifrost.{}",frame.id));
                top.set_min_size(320,220); surface.commit();
                let mut pending=Damage::new(frame.width,frame.height); pending.all();
                self.state.windows.insert(frame.id,Window { surface,xdg,top,slots:vec![],buffer_size:(0,0),configured:false,frame,pending,margin:0,gpu_size:(0,0),bg:None,bg_rects:Vec::new() });
            }
        }
        if let (Some(id),Some(activation),Some(seat))=(activate,self.state.activation.as_ref(),self.state.seat.as_ref()) {
            let token=activation.get_activation_token(&qh,id);
            token.set_serial(self.state.serial,seat);
            token.set_surface(self.state.surface.as_ref().unwrap());
            token.commit();
        }
        // an explicit open or focus: xdg-activation needs a fresh input serial that control-socket and dock opens
        // lack, and a native client may hold focus; wlroots compositors always obey a foreign-toplevel activate
        if std::mem::take(&mut self.state.raise_front) {
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
            let ids:Vec<u32>=self.state.windows.keys().copied().collect();
            for id in ids {
                let key=GPU_WINDOW+id;
                let focused=self.state.focused==Some(id);
                let w=self.state.windows.get_mut(&id).unwrap();
                if !w.configured || w.frame.pixels.is_none() { continue; }
                let (width,height)=(w.frame.width,w.frame.height);
                if width>8192||height>8192 { return Err(err("window too large")); }
                // the margins hold the GPU shadow (a dedicated reach, longer below); maximized windows have none
                let scale=if self.state.scale>0.0 {self.state.scale} else {(w.frame.title_height as f32/TITLE_PT).max(1.0)};
                let (margin,bottom)=if w.frame.maximized {(0,0)} else {((SHADOW_SIDE_PT*scale).round() as i32,(SHADOW_BOTTOM_PT*scale).round() as i32)};
                let (bw,bh)=(width+2*margin as usize,height+(margin+bottom) as usize);
                if w.gpu_size!=(bw,bh) || !g.p.has_surface(key) {
                    if let Some(slots)=self.state.gpu_slots.remove(&key) {for s in slots {s.buffer.destroy();}}
                    let infos=g.p.create_framed(key,bw,bh,(margin as usize,margin as usize,width,height),false,2)?;
                    let slots=gpu_buffers(&self.state,&infos,&qh)?;
                    let w=self.state.windows.get_mut(&id).unwrap();
                    w.gpu_size=(bw,bh); w.margin=margin; w.pending=Damage::new(width,height); w.pending.all();
                    w.xdg.set_window_geometry(margin,margin,width as i32,height as i32);
                    // the shadow is not part of the window: clicks there reach what is below
                    let region=self.state.compositor.as_ref().unwrap().create_region(&qh,());
                    region.add(margin,margin,width as i32,height as i32);
                    w.surface.set_input_region(Some(&region)); region.destroy();
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
                let wm_blur=self.state.bg_blur && self.state.bg_manager.is_some();
                if wm_blur {
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
                    Some(m) => (Backdrop::GlassOnly{x:w.frame.x-margin,y:w.frame.y-margin,active:focused},
                        m.iter().map(|r|Glass{x:r.x+w.frame.x as f32,y:r.y+w.frame.y as f32,..*r}).collect()),
                    None => (Backdrop::None,Vec::new()),
                };
                let areas=g.p.draw(key,slot,pixels,width,&w.pending,back,&glass);
                w.pending.clear();
                if areas.is_empty() {continue;}
                let s=&mut self.state.gpu_slots.get_mut(&key).unwrap()[slot];
                w.surface.attach(Some(&s.buffer),0,0);
                for a in &areas {w.surface.damage_buffer(a.x0 as i32,a.y0 as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
                w.surface.frame(&qh,false); w.surface.commit(); s.busy=true;
            }
            // live materials: tell the presenter which windows sit behind the glass (z order, content origin)
            let stack:Vec<(u32,i32,i32)>=self.state.z.iter().filter_map(|id|self.state.windows.get(id).filter(|w|w.configured).map(|w|(GPU_WINDOW+*id,w.frame.x,w.frame.y))).collect();
            g.p.set_window_stack(stack);
            if g.p.take_live_changed() {
                // glass shapes pick up the new backdrop: the shell surfaces owe those shapes (the content is untouched)
                let rects:Vec<[f32;4]>=g.glass.iter().map(|r|[r.x,r.y,r.w,r.h]).collect();
                g.p.owe_rects(GPU_BG,&rects);
                // chrome layer surfaces (menus, panels) draw the same glass list: they owe the part over them
                for c in self.state.chrome.values_mut() {
                    let (cx,cy,cw,ch)=(c.frame.x as f32,c.frame.y as f32,c.frame.width as f32,c.frame.height as f32);
                    for r in &rects {
                        let (x0,y0)=(r[0].max(cx),r[1].max(cy));
                        let (x1,y1)=((r[0]+r[2]).min(cx+cw),(r[1]+r[3]).min(cy+ch));
                        if x1>x0 && y1>y0 { c.pending.add((x0-cx) as i32,(y0-cy) as i32,(x1-x0).ceil() as i32,(y1-y0).ceil() as i32); }
                    }
                }
                self.state.pending=true;
            }
            // a window under a vibrant window changed: that window's materials blur it, so they owe a redraw
            for key in g.p.take_under_changed() {
                let Some(id)=key.checked_sub(GPU_WINDOW) else {continue};
                if let (Some(m),Some(w))=(self.state.materials.get(&id),self.state.windows.get(&id)) {
                    let o=w.margin as f32;
                    let rects:Vec<[f32;4]>=m.iter().map(|r|[r.x+o,r.y+o,r.w,r.h]).collect();
                    g.p.owe_rects(key,&rects);
                    self.state.pending=true;
                }
            }
            return Ok(());
        }
        for w in self.state.windows.values_mut() {
            if !w.configured { continue; }
            let width = w.frame.width; let height = w.frame.height; let len = width*height*4;
            if width > 8192 || height > 8192 { return Err(err("window too large")); }
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
fn place_chrome(l:&surface::ZwlrLayerSurfaceV1,frame:&ChromeFrame,screen:(u32,u32)) {
    match frame.id {
        0 => {l.set_anchor(surface::Anchor::Top|surface::Anchor::Left|surface::Anchor::Right);l.set_margin(frame.y,0,0,0);l.set_exclusive_zone(frame.height as i32+frame.y.max(0));}
        2 if screen.1>0 => {
            let bottom=(screen.1 as i32-frame.y-frame.height as i32).max(0);
            l.set_anchor(surface::Anchor::Bottom);l.set_margin(0,0,bottom,0);l.set_exclusive_zone(frame.height as i32+bottom);
        }
        // -1: placed from the screen's corner, not pushed below the menu bar's or the Dock's exclusive zone
        _ => {l.set_anchor(surface::Anchor::Top|surface::Anchor::Left);l.set_margin(frame.y,0,0,frame.x);l.set_exclusive_zone(-1);}
    }
}

    pub fn chrome(&mut self, frames:Vec<ChromeFrame>) -> io::Result<()> {
        let ids:Vec<_>=frames.iter().map(|f|f.id).collect();
        let removed:Vec<_>=self.state.chrome.keys().filter(|id|!ids.contains(id)).copied().collect();
        for id in removed {let w=self.state.chrome.remove(&id).unwrap();w.layer.destroy();w.surface.destroy();for slot in w.slots {slot.buffer.destroy();}
            if let Some(slots)=self.state.gpu_slots.remove(&id) {for s in slots {s.buffer.destroy();}}
            if let Some(g)=&mut self.gpu {g.p.destroy_surface(id);}}
        let qh=self.queue.handle();
        for frame in frames {
            let screen=self.state.size;
            if let Some(w)=self.state.chrome.get_mut(&frame.id) {
                Self::place_chrome(&w.layer,&frame,screen);
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
                l.set_size(frame.width as u32,frame.height as u32);Self::place_chrome(&l,&frame,screen);
                // id 3 is Search: it takes the keyboard from the window in front while it is open
                l.set_keyboard_interactivity(if frame.id==3 {surface::KeyboardInteractivity::Exclusive} else {surface::KeyboardInteractivity::None});s.commit();
                let mut pending=Damage::new(frame.width,frame.height);pending.all();
                self.state.chrome.insert(frame.id,ChromeWindow{surface:s,layer:l,slots:vec![],configured:false,frame,pending});
            }
        }
        self.render_chrome()?;transient(self.conn.flush())
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
                if w.pending.is_empty() {continue;}
                let areas=g.p.draw(id,slot,&w.frame.pixels,width,&w.pending,Backdrop::GlassOnly{x:w.frame.x,y:w.frame.y,active:true},&g.glass);
                w.pending.clear();
                if areas.is_empty() {continue;}
                let s=&mut self.state.gpu_slots.get_mut(&id).unwrap()[slot];
                w.surface.attach(Some(&s.buffer),0,0);
                for a in &areas {w.surface.damage_buffer(a.x0 as i32,a.y0 as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
                w.surface.commit();s.busy=true;
                mark_chrome_frame(id);
            }
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
    pub fn foreign_maximize(&mut self,id:u32,maximized:bool)->io::Result<()> {if let Some(w)=self.state.foreign.iter().find(|w|w.info.id==id) {if maximized {w.handle.set_maximized();}else{w.handle.unset_maximized();}}transient(self.conn.flush())}
    pub fn inputs(&mut self, out: &mut Vec<Event>) {
        self.trim_idle();
        // a chrome surface was configured after its frame was queued: attach it now instead of waiting for the next shell
        // frame (an idle shell made the menu bar and Dock wait out the reveal gate's 4 s ceiling)
        if std::mem::take(&mut self.state.chrome_configured) {
            let _ = self.render_chrome();
            let _ = self.conn.flush();
        }
        // a backdrop rebuild the 30 Hz cap skipped is due: owe the glass that samples it and ask for a frame
        if self.gpu.as_mut().is_some_and(|g| g.p.retry_now()) {
            self.state.glass_retry = true;
            let _ = self.render_windows();
            let _ = self.conn.flush();
        }
        out.append(&mut self.state.inputs);
    }
    /// Ms until a skipped glass backdrop rebuild is due (a one-shot timeout for the main loop).
    pub fn glass_wake_ms(&self) -> Option<i32> { self.gpu.as_ref().and_then(|g| g.p.retry_in_ms()) }
    /// True once after a glass retry: the shell draws a frame so the desktop's glass is presented.
    pub fn take_glass_retry(&mut self) -> bool { std::mem::take(&mut self.state.glass_retry) }
    /// The wallpaper the GPU draws under the desktop (uploaded and blurred once per change).
    pub fn wallpaper(&mut self, wp: crate::gfx::View, x: i32) {
        if let Some(g) = &mut self.gpu { g.p.set_wallpaper(wp, x, 0); }
    }
    /// Frees the spare background drawable once the shell has drawn nothing for `GPU_POOL_IDLE_SECS`.
    pub fn trim_idle(&mut self) {
        let Some(g) = &mut self.gpu else { return };
        let Some(v) = self.state.gpu_slots.get_mut(&GPU_BG) else { return };
        let busy: Vec<bool> = v.iter().map(|s| s.busy).collect();
        if gpu_pool_step(&busy, false, g.last_frame.elapsed().as_secs_f32()) == PoolStep::Shrink {
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
    pub fn note_draw(&mut self, ms: f64) { if let Some(g) = &mut self.gpu { g.draw.0 += 1; g.draw.1 += ms; } }
    pub fn present(&mut self, pixels: &[u32], damage: &Damage) -> io::Result<()> {
        if self.state.lock_wanted && !self.state.lock.as_ref().is_some_and(|l|l.configured) { return Ok(()); }
        // after an unlock the desktop surface takes no buffer until its new configure (the configure asks for a full redraw)
        if !self.state.lock_wanted && !self.state.main.may_attach() { return Ok(()); }
        if let Some(g) = &mut self.gpu {
            let (w,h) = (self.state.size.0 as usize, self.state.size.1 as usize);
            let fresh = bifrost_render::glass::take();
            if damage.pixels() >= w*h { g.glass = fresh; } else { let areas: Vec<_> = damage.areas().iter().map(|a| (a.x0 as f32, a.y0 as f32, a.x1 as f32, a.y1 as f32)).collect(); bifrost_render::glass::merge(&mut g.glass, fresh, &areas); }
            // always consume the flush flag, so an owed glass refresh is not left for a later empty present
            let flush = g.p.take_flush();
            if damage.is_empty() && !flush { return Ok(()); }
            g.last_frame = std::time::Instant::now();
            let mut free = self.state.gpu_slots.get(&GPU_BG).and_then(|v| v.iter().position(|s| !s.busy));
            if free.is_none() {
                // the compositor holds every drawable: allocate the spare one (never draw into a held buffer)
                let busy: Vec<bool> = self.state.gpu_slots.get(&GPU_BG).map(|v| v.iter().map(|s| s.busy).collect()).unwrap_or_default();
                if gpu_pool_step(&busy, true, 0.0) == PoolStep::Grow {
                    if let Ok(info) = g.p.add_drawable(GPU_BG) {
                        let qh = self.queue.handle();
                        match gpu_buffers(&self.state, &[info], &qh) {
                            Ok(mut n) => { let v = self.state.gpu_slots.get_mut(&GPU_BG).unwrap(); v.append(&mut n); free = Some(v.len() - 1); }
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
            if !self.state.lock_wanted { self.state.main.attached(); }
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
        if !self.state.lock_wanted { self.state.main.attached(); }
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
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) { return; }
    crate::bootsplash::mark(crate::bootreveal::DESKTOP_PARTS[0]);
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let _ = std::fs::write(std::path::Path::new(&dir).join("shell-ready"), b"1\n");
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, reg: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, q: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name,interface,version } = e { match interface.as_str() {
            "ext_background_effect_manager_v1" if wm_materials_enabled() => s.bg_manager=Some(reg.bind(name,1,q,())),
            "zwlr_foreign_toplevel_manager_v1" => s.foreign_manager=Some(reg.bind(name,version.min(3),q,())),
            "ext_idle_notifier_v1" => s.idle_notifier=Some(reg.bind(name,version.min(2),q,())),
            "zwlr_output_power_manager_v1" => s.power_manager=Some(reg.bind(name,1,q,())),
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
                if !s.main.may_attach() { s.main.configured(); s.redraw = true; }
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
            if c.contains(wl_seat::Capability::Keyboard) { seat.get_keyboard(q,()); }
        }
    }
}
/// The shape an app pushed with the `cursor` service, in the cursor-shape protocol's names.
fn wl_shape(k: bifrost_ui::cursor::Kind) -> cursor_device::Shape {
    use bifrost_ui::cursor::Kind as K;
    match k {
        K::Arrow => cursor_device::Shape::Default,
        K::IBeam => cursor_device::Shape::Text,
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
            wl_pointer::Event::Button {button:273,state:WEnum::Value(wl_pointer::ButtonState::Pressed),..}=>{if let Some(id)=s.target{s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));}s.inputs.push(Event::Context{x:s.xy.0+offset.0,y:s.xy.1+offset.1,t});},
            wl_pointer::Event::Leave {..} if !s.pressed=>s.inputs.push(Event::Hover{x:-100.,y:-100.,t}),
            wl_pointer::Event::Button { button:272,state:WEnum::Value(v),serial,.. } => {
                s.serial=serial;
                s.pressed = v == wl_pointer::ButtonState::Pressed;
                if s.pressed {if let Some(id)=s.target {s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));}}
                let drags=s.target.and_then(|id|s.windows.get(&id)).is_some_and(|w|{let (x,y)=(s.xy.0 as i32,s.xy.1 as i32);
                    s.xy.1<w.frame.drag_height as f32 && !w.frame.no_drag.iter().any(|r|x>=r[0]&&x<r[0]+r[2]&&y>=r[1]&&y<r[1]+r[3])});
                if s.pressed && drags && s.xy.0 > 80.0 {
                    if let (Some(w),Some(seat)) = (s.target.and_then(|id| s.windows.get(&id)),s.seat.as_ref()) { w.top._move(seat,serial); return; }
                }
                s.inputs.push(if s.pressed { Event::Down { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t } } else { Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t } });
            },
            wl_pointer::Event::Leave {..} if s.pressed => { s.pressed = false; s.inputs.push(Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t }); },
            wl_pointer::Event::Axis { axis:WEnum::Value(wl_pointer::Axis::VerticalScroll),value,.. } => {
                s.inputs.extend([Event::Down { x:s.xy.0+offset.0,y:s.xy.1+offset.1,t },Event::Move { x:s.xy.0+offset.0,y:s.xy.1+offset.1-value as f32*3.0,t:t+16.0 },Event::Up { x:s.xy.0+offset.0,y:s.xy.1+offset.1-value as f32*3.0,t:t+32.0 }]);
            },
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(s: &mut Self, _: &wl_keyboard::WlKeyboard, e: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            wl_keyboard::Event::Enter {surface,..}=>{
                if let Some(id)=s.windows.iter().find(|(_,w)|w.surface==surface).map(|(&id,_)|id) {s.focused=Some(id);s.window_events.push(WindowEvent::Focus(id));}
            },
            wl_keyboard::Event::Key {key,state:WEnum::Value(v),serial,..}=>{
                s.serial=serial; let down=v==wl_keyboard::KeyState::Pressed;
                if down {s.keys.insert(key as u16);} else {s.keys.remove(&(key as u16));}
                s.inputs.push(Event::Key {code:key as u16,down});
            },
            wl_keyboard::Event::Leave {..}=>{for code in s.keys.drain(){s.inputs.push(Event::Key {code,down:false});}},
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
        if let xdg_surface::Event::Configure {serial} = e {obj.ack_configure(serial); if let Some(w)=s.windows.get_mut(id) {w.configured=true; s.window_events.push(WindowEvent::Focus(*id));} }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel,u32> for State {
    fn event(s: &mut Self, _: &xdg_toplevel::XdgToplevel, e:xdg_toplevel::Event, id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        match e {
            xdg_toplevel::Event::Configure {width,height,states} => { let maximized=states.chunks_exact(4).any(|bytes|u32::from_ne_bytes(bytes.try_into().unwrap())==xdg_toplevel::State::Maximized as u32); s.window_events.push(WindowEvent::Maximized(*id,maximized)); if width>0 && height>0 {s.window_events.push(WindowEvent::Resize(*id,width,height));} },
            xdg_toplevel::Event::Close => s.window_events.push(WindowEvent::Close(*id)),
            _=>{}
        }
    }
}

impl Dispatch<wl_output::WlOutput,()> for State {
    fn event(s:&mut Self,_:&wl_output::WlOutput,e:wl_output::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if let wl_output::Event::Mode {flags:WEnum::Value(flags),refresh,..}=e {
            if flags.contains(wl_output::Mode::Current) {s.refresh=(refresh.max(1) as u32+500)/1000;}
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
/// The desktop layer surface's mapping across the session lock. Committing a null buffer unmaps a layer surface;
/// before it may show a buffer again the client must make a fresh initial commit and wait for a new configure
/// (wlr-layer-shell). Attaching earlier is a protocol error: the compositor disconnects the shell (0.5.27).
#[derive(Debug, Default, PartialEq, Clone, Copy)]
struct MainMap { mapped: bool, remap_due: bool, awaiting_configure: bool }
impl MainMap {
    /// A buffer was attached and committed.
    fn attached(&mut self) { self.mapped = true; }
    /// Locking: true when the surface is mapped and the null-buffer commit will unmap it.
    fn lock(&mut self) -> bool {
        let unmaps = self.mapped;
        if unmaps { self.mapped = false; self.remap_due = true; }
        unmaps
    }
    /// Unlocking: true when the client must make the initial commit again and wait for a configure.
    fn unlock(&mut self) -> bool {
        let remap = std::mem::take(&mut self.remap_due);
        if remap { self.awaiting_configure = true; }
        remap
    }
    /// A configure arrived for the desktop surface (one that comes while locked does not count for the remap).
    fn configured(&mut self) { self.awaiting_configure = false; }
    /// Whether a frame may attach a buffer to the desktop surface now.
    fn may_attach(&self) -> bool { !self.awaiting_configure }
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
#[derive(Debug, PartialEq, Clone, Copy)]
enum PoolStep { Keep, Grow, Shrink }
/// Buffer-pool policy for the full-screen drawables. `busy[i]`: the compositor still holds drawable i.
/// Grow only when a frame needs drawing and every drawable is held; shrink the last one after the
/// idle period, and only if the compositor is not holding it.
fn gpu_pool_step(busy: &[bool], need_frame: bool, idle_secs: f32) -> PoolStep {
    let n = busy.len();
    if need_frame && n < GPU_POOL_MAX && busy.iter().all(|&b| b) { PoolStep::Grow }
    else if !need_frame && n > GPU_POOL_MIN && idle_secs >= GPU_POOL_IDLE_SECS && !busy[n - 1] { PoolStep::Shrink }
    else { PoolStep::Keep }
}
/// Ms left before an idle pool of `n` drawables shrinks, None when it is already at its minimum or the spare is
/// still held by the compositor (it holds the on-screen buffer until the next attach; its release wakes the loop).
/// Arming a deadline for a held spare made it fire every millisecond (130 wakeups/s on Midget, 0.5.16).
fn pool_trim_in_ms(n: usize, spare_busy: bool, idle_secs: f32) -> Option<i32> {
    (n > GPU_POOL_MIN && !spare_busy).then(|| ((GPU_POOL_IDLE_SECS - idle_secs).max(0.0) * 1000.0).ceil() as i32 + 1)
}
// Keep quiet windows single-buffered. A pending update may allocate one spare
// while the compositor owns the first; never write into an in-flight buffer.
fn window_buffer_count(count:usize, available:bool, dirty:bool) -> usize {
    if count == 0 { 1 } else if count == 1 && !available && dirty { 2 } else { count }
}
#[cfg(test)]
mod tests {
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

    #[test]
    fn relocking_a_shown_desktop_waits_for_a_new_configure_before_attaching() {
        let mut m = MainMap::default();
        assert!(m.may_attach(), "the first frames attach once the initial configure arrived");
        m.attached();
        assert!(m.lock(), "the null-buffer commit unmaps a mapped surface");
        assert!(m.unlock(), "unlocking makes the initial commit again");
        assert!(!m.may_attach(), "attaching before the new configure is a protocol error");
        m.configured();
        assert!(m.may_attach());
        m.attached();
        assert!(m.lock() && m.unlock(), "every lock of a shown desktop needs the remap");
    }

    #[test]
    fn a_desktop_never_shown_before_the_lock_needs_no_remap() {
        // boot opens on the lock screen: the surface only had its initial commit, the compositor keeps it configured
        let mut m = MainMap::default();
        assert!(!m.lock());
        assert!(!m.unlock());
        assert!(m.may_attach());
    }

    #[test]
    fn a_configure_while_locked_does_not_skip_the_remap() {
        let mut m = MainMap::default();
        m.attached();
        assert!(m.lock());
        m.configured();
        assert!(m.unlock());
        assert!(!m.may_attach());
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
        assert_eq!(gpu_pool_step(&[true], true, 0.0), PoolStep::Grow);
        assert_eq!(gpu_pool_step(&[false], true, 0.0), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[true, true], true, 0.0), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[false, false], false, 4.9), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[true, false], false, 6.0), PoolStep::Shrink);
        assert_eq!(gpu_pool_step(&[false, true], false, 6.0), PoolStep::Keep);
        assert_eq!(gpu_pool_step(&[false], false, 60.0), PoolStep::Keep);
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
            s.foreign.push(Foreign {handle:toplevel,info:ForeignWindow {id,title:String::new(),app_id:String::new(),active:false,maximized:false}});
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
            foreign_handle::Event::State {state} => {w.info.active=state.chunks_exact(4).any(|v|u32::from_ne_bytes(v.try_into().unwrap())==2);w.info.maximized=state.chunks_exact(4).any(|v|u32::from_ne_bytes(v.try_into().unwrap())==0);},
            foreign_handle::Event::Done => s.foreign_dirty=true,
            _ => {}
        }}
    }
}

/// Switch for the window-server material path (default on; `BIFROST_WM_MATERIALS=0` keeps the shell's own blur).
fn wm_materials_enabled() -> bool { std::env::var("BIFROST_WM_MATERIALS").ok().as_deref() != Some("0") }

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
impl Dispatch<bg_surface::ExtBackgroundEffectSurfaceV1,()> for State {
    fn event(_:&mut Self,_:&bg_surface::ExtBackgroundEffectSurfaceV1,_:bg_surface::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}

#[cfg(test)]
mod material_tests {
    use super::*;
    use bifrost_render::glass::Kind;
    #[test]
    fn blur_rects_are_offset_by_the_shadow_margin() {
        let g = Glass { x: 0.0, y: 30.5, w: 200.2, h: 400.0, r: 0.0, kind: Kind::Sidebar, active: None };
        assert_eq!(blur_rects(&[g], 40), vec![[40, 70, 201, 400]]);
        assert!(blur_rects(&[], 40).is_empty());
    }
}
