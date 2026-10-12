//! A chrome client's connection and surfaces: one Wayland connection per process with a Vulkan device and a
//! `bifrost_2d::Renderer`, and layer-shell surfaces whose content is drawn with `bifrost-2d` into dma-buf backing
//! stores and committed as a `bifrost_layer_v1` layer tree (the root layer shows the drawn content; extra layers
//! animate in the window server, such as the Dock's magnified icons).
//!
//! Owner: wave G2 agent `chrome-core`. The three clients (menu bar, Dock, panels) use only this API.
//!
//! What runs here: each surface has `SLOTS` exportable Vulkan textures, each wrapped as a linux-dmabuf `wl_buffer`
//! (linear modifier). `draw` picks a free slot, draws the display list into it, waits for the GPU, binds the slot to
//! the root contents of the surface's layer context and commits the context. The compositor sends `wl_buffer.release`
//! when it is done with a slot. Frame callbacks drive redraws.
//!
//! Known limits (see docs/graphics/g2-chrome-core.md): key events carry the evdev
//! code, not an xkb keysym; the scale is 1 until output scale is read; GPU waits are blocking, not explicit sync.

use bifrost_2d::Renderer;
use bifrost_gpu::dmabuf::MOD_LINEAR;
use bifrost_gpu::{CommandQueue, Device, DmaBuf, PixelFormat, Texture, TextureDescriptor, TextureUsage};
use bifrost_layers::protocol::client::bifrost_layer_context_v1::{self, BifrostLayerContextV1};
use bifrost_layers::protocol::client::bifrost_layer_manager_v1::BifrostLayerManagerV1;
use bifrost_layers::{codec, Change, ContentsId, LayerContents, LayerId, Transaction};
use bifrost_render::canvas::RectF;
use bifrost_render::display_list::DisplayList;
use std::collections::HashMap;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection as WlConnection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// The layer-shell layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

/// Edges a surface is anchored to (combine with `|`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Anchor(pub u32);

impl Anchor {
    pub const TOP: Anchor = Anchor(1);
    pub const BOTTOM: Anchor = Anchor(2);
    pub const LEFT: Anchor = Anchor(4);
    pub const RIGHT: Anchor = Anchor(8);
}

impl std::ops::BitOr for Anchor {
    type Output = Anchor;
    fn bitor(self, rhs: Anchor) -> Anchor {
        Anchor(self.0 | rhs.0)
    }
}

/// How to make a surface.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceSpec {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Requested size in logical pixels; 0 on an anchored axis stretches to the output.
    pub size: (u32, u32),
    /// Exclusive zone in logical pixels (-1: ignore others' zones, 0: none).
    pub exclusive_zone: i32,
    pub margin: (i32, i32, i32, i32),
    /// Whether the surface takes keyboard focus while shown (menus and search do).
    pub keyboard: bool,
}

/// A surface id within a `Display`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub u32);

/// What happened, for the client's loop.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The compositor sized the surface (logical pixels) and its output scale.
    Configured { surface: SurfaceId, width: u32, height: u32, scale: f32 },
    Closed(SurfaceId),
    PointerEnter { surface: SurfaceId, x: f32, y: f32 },
    PointerMotion { surface: SurfaceId, x: f32, y: f32 },
    PointerLeave(SurfaceId),
    /// Button press or release (Linux button code, BTN_LEFT = 0x110).
    PointerButton { surface: SurfaceId, button: u32, pressed: bool },
    Scroll { surface: SurfaceId, dx: f32, dy: f32 },
    /// A key press or release. `keysym` carries the Linux evdev key code until xkb keysyms are wired in.
    Key { surface: SurfaceId, keysym: u32, pressed: bool },
    /// A frame callback: the surface may draw again.
    Frame(SurfaceId),
    /// A layer animation ended (`bifrost_layer_context_v1.animation_finished`).
    AnimationFinished { surface: SurfaceId, id: u32 },
}

/// Backing stores per surface: one being drawn, one shown, one spare.
const SLOTS: usize = 3;

/// The contents id of a surface's root layer.
const ROOT_CONTENTS: u32 = 1;

/// The layer id of a surface's root layer.
const ROOT_LAYER: LayerId = LayerId(1);

/// The pixel size of a surface's backing stores: logical size times scale, rounded, never zero.
pub fn pixel_size(logical: (u32, u32), scale: f32) -> (u32, u32) {
    let px = |v: u32| ((v as f32 * scale).round().max(1.0)) as u32;
    (px(logical.0), px(logical.1))
}

/// The first free slot at or after `start`, wrapping around; `None` when every slot is busy.
pub fn pick_slot(busy: &[bool], start: usize) -> Option<usize> {
    let n = busy.len();
    (0..n).map(|i| (start + i) % n).find(|&idx| !busy[idx])
}

/// A `wl_buffer` user-data tag: the surface id in the high 32 bits, the slot in the low 32 bits.
pub fn slot_tag(surface: SurfaceId, slot: usize) -> u64 {
    ((surface.0 as u64) << 32) | slot as u64
}

/// The surface and slot a tag was made for.
pub fn split_slot_tag(tag: u64) -> (SurfaceId, usize) {
    (SurfaceId((tag >> 32) as u32), (tag & 0xffff_ffff) as usize)
}

/// The transaction that makes the root layer show the first bound backing store.
pub fn first_draw_transaction(seq: u64) -> Transaction {
    let mut tx = Transaction::new(seq);
    tx.push(Change::Create(ROOT_LAYER));
    tx.push(Change::SetRoot(ROOT_LAYER));
    tx.push(Change::SetContents(ROOT_LAYER, LayerContents::Buffer(ContentsId(ROOT_CONTENTS))));
    tx
}

fn wl_layer(layer: Layer) -> zwlr_layer_shell_v1::Layer {
    match layer {
        Layer::Background => zwlr_layer_shell_v1::Layer::Background,
        Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        Layer::Top => zwlr_layer_shell_v1::Layer::Top,
        Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

/// One backing store: its texture (kept alive for the dma-buf), its `wl_buffer`, and whether the compositor holds it.
struct Slot {
    texture: Texture,
    buffer: wl_buffer::WlBuffer,
    busy: bool,
}

struct Surf {
    wl: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    ctx: BifrostLayerContextV1,
    spec: SurfaceSpec,
    /// The last configured logical size, `None` until the first configure.
    size: Option<(u32, u32)>,
    scale: f32,
    slots: Vec<Slot>,
    slot_px: (u32, u32),
    next: usize,
    root_made: bool,
    seq: u64,
}

/// Events the protocol handlers collect; `Display::pump` turns them into `Event`s and updates the surfaces.
#[derive(Default)]
struct State {
    events: Vec<Event>,
    configures: Vec<(SurfaceId, u32, u32, u32)>,
    closed: Vec<SurfaceId>,
    released: Vec<u64>,
    pointer_on: Option<SurfaceId>,
    keyboard_on: Option<SurfaceId>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
}

/// The process's connection, GPU and surfaces.
pub struct Display {
    conn: WlConnection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    compositor: wl_compositor::WlCompositor,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    layer_manager: BifrostLayerManagerV1,
    dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    _seat: Option<wl_seat::WlSeat>,
    device: Device,
    renderer: Renderer,
    commands: CommandQueue,
    surfaces: HashMap<SurfaceId, Surf>,
    next_id: u32,
}

impl Display {
    /// Connects to `WAYLAND_DISPLAY`, binds the globals (compositor, layer shell, linux-dmabuf, bifrost_layer_v1,
    /// seat) and opens the GPU. Fails when the compositor lacks layer shell or `bifrost_layer_manager_v1`.
    pub fn connect() -> Result<Display, String> {
        let conn = WlConnection::connect_to_env().map_err(|e| format!("wayland: {e:?}"))?;
        let (list, queue): (GlobalList, EventQueue<State>) = registry_queue_init(&conn).map_err(|e| format!("registry: {e:?}"))?;
        let qh = queue.handle();
        let compositor = bind(&list, &qh, 4..=6, "wl_compositor")?;
        let layer_shell = bind(&list, &qh, 4..=5, "zwlr_layer_shell_v1")?;
        let layer_manager = bind(&list, &qh, 1..=1, "bifrost_layer_manager_v1")?;
        let dmabuf = bind(&list, &qh, 4..=5, "zwp_linux_dmabuf_v1")?;
        let seat = list.bind::<wl_seat::WlSeat, State, ()>(&qh, 5..=9, ()).ok();
        let device = Device::open(None).map_err(|e| format!("gpu: {e:?}"))?;
        let renderer = Renderer::new(&device).map_err(|e| format!("renderer: {e:?}"))?;
        let commands = device.new_command_queue();
        let mut display = Display {
            conn,
            queue,
            qh,
            state: State::default(),
            compositor,
            layer_shell,
            layer_manager,
            dmabuf,
            _seat: seat,
            device,
            renderer,
            commands,
            surfaces: HashMap::new(),
            next_id: 1,
        };
        display.queue.roundtrip(&mut display.state).map_err(|e| format!("roundtrip: {e:?}"))?;
        Ok(display)
    }

    /// Makes a layer-shell surface; it is shown after its first `Configured` and `draw`.
    pub fn create(&mut self, spec: SurfaceSpec) -> Result<SurfaceId, String> {
        let id = SurfaceId(self.next_id);
        self.next_id += 1;
        let wl = self.compositor.create_surface(&self.qh, id);
        let layer = self.layer_shell.get_layer_surface(
            &wl,
            None,
            wl_layer(spec.layer),
            spec.namespace.clone(),
            &self.qh,
            id,
        );
        apply_spec(&layer, &spec);
        let ctx = self.layer_manager.get_context(&wl, &self.qh, id);
        // The first commit without a buffer asks the compositor for the initial configure.
        wl.commit();
        self.conn.flush().map_err(|e| format!("flush: {e:?}"))?;
        self.surfaces.insert(
            id,
            Surf { wl, layer, ctx, spec, size: None, scale: 1.0, slots: Vec::new(), slot_px: (0, 0), next: 0, root_made: false, seq: 0 },
        );
        Ok(id)
    }

    /// Changes a surface's size, exclusive zone or margins (the Dock grows and moves).
    pub fn update(&mut self, id: SurfaceId, spec: SurfaceSpec) {
        if let Some(surf) = self.surfaces.get_mut(&id) {
            apply_spec(&surf.layer, &spec);
            surf.wl.commit();
            surf.spec = spec;
            let _ = self.conn.flush();
        }
    }

    pub fn destroy(&mut self, id: SurfaceId) {
        if let Some(surf) = self.surfaces.remove(&id) {
            for slot in &surf.slots {
                slot.buffer.destroy();
            }
            surf.ctx.destroy();
            surf.layer.destroy();
            surf.wl.destroy();
            let _ = self.conn.flush();
        }
    }

    /// Draws `list` (in device pixels of the configured size times scale) into the surface's next backing store,
    /// only `damage` when given, binds it to the root layer and commits.
    pub fn draw(&mut self, id: SurfaceId, list: &DisplayList, damage: Option<RectF>) -> Result<(), String> {
        let Display { conn, qh, device, renderer, commands, dmabuf, surfaces, .. } = self;
        let surf = surfaces.get_mut(&id).ok_or("unknown surface")?;
        let logical = surf.size.ok_or("surface is not configured yet")?;
        let px = pixel_size(logical, surf.scale);
        if surf.slot_px != px {
            for slot in surf.slots.drain(..) {
                slot.buffer.destroy();
            }
            surf.slot_px = px;
        }
        if surf.slots.is_empty() {
            for i in 0..SLOTS {
                let slot = make_slot(device, dmabuf, qh, id, i, px)?;
                surf.slots.push(slot);
            }
        }
        let busy: Vec<bool> = surf.slots.iter().map(|s| s.busy).collect();
        let index = pick_slot(&busy, surf.next).ok_or("every backing store is busy")?;

        let target = &surf.slots[index].texture;
        let mut cb = commands.command_buffer().map_err(|e| format!("command buffer: {e:?}"))?;
        renderer.draw(&mut cb, list, target, damage).map_err(|e| format!("draw: {e:?}"))?;
        let submission = cb.commit().map_err(|e| format!("commit: {e:?}"))?;
        submission.wait().map_err(|e| format!("gpu wait: {e:?}"))?;

        let slot = &mut surf.slots[index];
        slot.busy = true;
        surf.next = (index + 1) % SLOTS;
        surf.ctx.bind_contents(ROOT_CONTENTS, &slot.buffer);
        let tx = if surf.root_made { Transaction::new(surf.seq) } else { first_draw_transaction(surf.seq) };
        surf.root_made = true;
        surf.seq += 1;
        surf.ctx.commit(codec::encode(&tx));
        surf.wl.frame(qh, id);
        conn.flush().map_err(|e| format!("flush: {e:?}"))?;
        Ok(())
    }

    /// Commits a layer transaction for the surface's tree (extra layers beyond the root, animations).
    pub fn commit_layers(&mut self, id: SurfaceId, tx: &Transaction) -> Result<(), String> {
        let surf = self.surfaces.get(&id).ok_or("unknown surface")?;
        surf.ctx.commit(codec::encode(tx));
        self.conn.flush().map_err(|e| format!("flush: {e:?}"))
    }

    /// Waits up to `timeout_ms` (`-1` = until something happens) or until `extra_fd` (the shell link) is
    /// readable, and returns the events that arrived. Sleeps in poll(2) on the Wayland socket and `extra_fd`.
    pub fn dispatch(&mut self, timeout_ms: i32, extra_fd: Option<std::os::fd::RawFd>) -> Result<Vec<Event>, String> {
        let deadline = deadline_for(timeout_ms);
        loop {
            self.queue.dispatch_pending(&mut self.state).map_err(|e| format!("dispatch: {e:?}"))?;
            self.conn.flush().map_err(|e| format!("flush: {e:?}"))?;
            if !self.state.events.is_empty() || !self.state.configures.is_empty() || !self.state.closed.is_empty() {
                break;
            }
            let wait = match deadline {
                None => -1,
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    // round up so a sub-millisecond remainder does not become a busy 0 ms poll
                    left.as_millis().min(i32::MAX as u128 - 1) as i32 + i32::from(left.subsec_nanos() % 1_000_000 != 0)
                }
            };
            let mut link_ready = false;
            if let Some(guard) = self.conn.prepare_read() {
                use std::os::fd::AsRawFd;
                let (wl_ready, extra_ready) = poll_fds(guard.connection_fd().as_raw_fd(), extra_fd, wait);
                link_ready = extra_ready;
                if wl_ready {
                    match guard.read() {
                        Ok(_) => {}
                        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(format!("read: {e:?}")),
                    }
                } else {
                    drop(guard);
                }
            }
            self.queue.dispatch_pending(&mut self.state).map_err(|e| format!("dispatch: {e:?}"))?;
            if link_ready || deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
        }
        self.apply_state();
        Ok(std::mem::take(&mut self.state.events))
    }

    /// Turns what the handlers collected into surface changes and `Event`s.
    fn apply_state(&mut self) {
        let configures = std::mem::take(&mut self.state.configures);
        for (id, serial, width, height) in configures {
            if let Some(surf) = self.surfaces.get_mut(&id) {
                surf.layer.ack_configure(serial);
                surf.size = Some((width, height));
                let scale = surf.scale;
                self.state.events.push(Event::Configured { surface: id, width, height, scale });
                self.state.events.push(Event::Frame(id));
            }
        }
        let closed = std::mem::take(&mut self.state.closed);
        for id in closed {
            if self.surfaces.contains_key(&id) {
                self.destroy(id);
                self.state.events.push(Event::Closed(id));
            }
        }
        for tag in std::mem::take(&mut self.state.released) {
            let (id, index) = split_slot_tag(tag);
            if let Some(slot) = self.surfaces.get_mut(&id).and_then(|s| s.slots.get_mut(index)) {
                slot.busy = false;
            }
        }
    }
}

/// Binds a required global, mapping its absence to an error that names it.
fn bind<I>(list: &GlobalList, qh: &QueueHandle<State>, versions: std::ops::RangeInclusive<u32>, name: &str) -> Result<I, String>
where
    I: Proxy + 'static,
    State: Dispatch<I, ()>,
{
    list.bind(qh, versions, ()).map_err(|e| format!("{name}: {e:?}"))
}

fn apply_spec(layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, spec: &SurfaceSpec) {
    layer.set_size(spec.size.0, spec.size.1);
    layer.set_anchor(zwlr_layer_surface_v1::Anchor::from_bits_truncate(spec.anchor.0));
    layer.set_exclusive_zone(spec.exclusive_zone);
    let (top, right, bottom, left) = spec.margin;
    layer.set_margin(top, right, bottom, left);
    layer.set_keyboard_interactivity(if spec.keyboard {
        zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
    } else {
        zwlr_layer_surface_v1::KeyboardInteractivity::None
    });
}

/// Makes one backing store: an exportable texture, its dma-buf, and the `wl_buffer` tagged with (surface, slot).
fn make_slot(
    device: &Device,
    dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    qh: &QueueHandle<State>,
    id: SurfaceId,
    index: usize,
    px: (u32, u32),
) -> Result<Slot, String> {
    let texture = device
        .new_texture(&TextureDescriptor {
            width: px.0,
            height: px.1,
            format: PixelFormat::Bgra8Unorm,
            usage: TextureUsage::RENDER_TARGET | TextureUsage::SAMPLED | TextureUsage::COPY_SRC,
            exportable: true,
            modifiers: vec![MOD_LINEAR],
        })
        .map_err(|e| format!("texture: {e:?}"))?;
    let exported = texture.export_dmabuf().map_err(|e| format!("export: {e:?}"))?;
    let buffer = dmabuf_buffer(dmabuf, qh, &exported, slot_tag(id, index))?;
    Ok(Slot { texture, buffer, busy: false })
}

/// A `wl_buffer` for one dma-buf, through `zwp_linux_buffer_params_v1.create_immed`.
fn dmabuf_buffer(
    dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    qh: &QueueHandle<State>,
    buf: &DmaBuf,
    tag: u64,
) -> Result<wl_buffer::WlBuffer, String> {
    let width = i32::try_from(buf.width).map_err(|_| "width".to_string())?;
    let height = i32::try_from(buf.height).map_err(|_| "height".to_string())?;
    let params = dmabuf.create_params(qh, ());
    let hi = (buf.modifier >> 32) as u32;
    let lo = buf.modifier as u32;
    for (idx, plane) in buf.planes.iter().enumerate() {
        params.add(plane.fd.as_fd(), idx as u32, plane.offset, plane.stride, hi, lo);
    }
    let buffer = params.create_immed(width, height, buf.fourcc, zwp_linux_buffer_params_v1::Flags::empty(), qh, tag);
    params.destroy();
    Ok(buffer)
}

fn button_pressed(state: WEnum<wl_pointer::ButtonState>) -> bool {
    matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed))
}

fn key_pressed(state: WEnum<wl_keyboard::KeyState>) -> bool {
    matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed))
}

fn surface_id(surface: &wl_surface::WlSurface) -> Option<SurfaceId> {
    surface.data::<SurfaceId>().copied()
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut State, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut State, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_surface::WlSurface, SurfaceId> for State {
    fn event(_: &mut State, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &SurfaceId, _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<wl_callback::WlCallback, SurfaceId> for State {
    fn event(state: &mut State, _: &wl_callback::WlCallback, event: wl_callback::Event, id: &SurfaceId, _: &WlConnection, _: &QueueHandle<State>) {
        if let wl_callback::Event::Done { .. } = event {
            state.events.push(Event::Frame(*id));
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, u64> for State {
    fn event(state: &mut State, _: &wl_buffer::WlBuffer, event: wl_buffer::Event, tag: &u64, _: &WlConnection, _: &QueueHandle<State>) {
        if let wl_buffer::Event::Release = event {
            state.released.push(*tag);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(state: &mut State, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &WlConnection, qh: &QueueHandle<State>) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let caps = match capabilities {
                WEnum::Value(c) => c.bits(),
                WEnum::Unknown(u) => u,
            };
            if caps & wl_seat::Capability::Pointer.bits() != 0 && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps & wl_seat::Capability::Keyboard.bits() != 0 && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(state: &mut State, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } => {
                if let Some(id) = surface_id(&surface) {
                    state.pointer_on = Some(id);
                    state.events.push(Event::PointerEnter { surface: id, x: surface_x as f32, y: surface_y as f32 });
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if let Some(id) = surface_id(&surface) {
                    state.pointer_on = None;
                    state.events.push(Event::PointerLeave(id));
                }
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                if let Some(id) = state.pointer_on {
                    state.events.push(Event::PointerMotion { surface: id, x: surface_x as f32, y: surface_y as f32 });
                }
            }
            wl_pointer::Event::Button { button, state: s, .. } => {
                if let Some(id) = state.pointer_on {
                    state.events.push(Event::PointerButton { surface: id, button, pressed: button_pressed(s) });
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                if let Some(id) = state.pointer_on {
                    let v = value as f32;
                    let (dx, dy) = match axis {
                        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => (v, 0.0),
                        _ => (0.0, v),
                    };
                    state.events.push(Event::Scroll { surface: id, dx, dy });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(state: &mut State, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => state.keyboard_on = surface_id(&surface),
            wl_keyboard::Event::Leave { .. } => state.keyboard_on = None,
            wl_keyboard::Event::Key { key, state: s, .. } => {
                if let Some(id) = state.keyboard_on {
                    state.events.push(Event::Key { surface: id, keysym: key, pressed: key_pressed(s) });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(_: &mut State, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, SurfaceId> for State {
    fn event(state: &mut State, _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, id: &SurfaceId, _: &WlConnection, _: &QueueHandle<State>) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => state.configures.push((*id, serial, width, height)),
            zwlr_layer_surface_v1::Event::Closed => state.closed.push(*id),
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for State {
    fn event(_: &mut State, _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _: zwp_linux_dmabuf_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for State {
    fn event(_: &mut State, _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, _: zwp_linux_buffer_params_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<BifrostLayerManagerV1, ()> for State {
    fn event(_: &mut State, _: &BifrostLayerManagerV1, _: bifrost_layers::protocol::client::bifrost_layer_manager_v1::Event, _: &(), _: &WlConnection, _: &QueueHandle<State>) {}
}

impl Dispatch<BifrostLayerContextV1, SurfaceId> for State {
    fn event(state: &mut State, _: &BifrostLayerContextV1, event: bifrost_layer_context_v1::Event, id: &SurfaceId, _: &WlConnection, _: &QueueHandle<State>) {
        if let bifrost_layer_context_v1::Event::AnimationFinished { animation_id } = event {
            state.events.push(Event::AnimationFinished { surface: *id, id: animation_id });
        }
    }
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, n: u64, timeout: i32) -> i32;
}

/// The deadline for a `dispatch` timeout: negative waits forever (`None`).
fn deadline_for(timeout_ms: i32) -> Option<Instant> {
    (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64))
}

/// poll(2) on the Wayland socket and an optional second descriptor; returns (first readable, second readable).
/// `timeout_ms < 0` waits forever. A signal or error reads as "nothing ready".
fn poll_fds(first: i32, second: Option<i32>, timeout_ms: i32) -> (bool, bool) {
    const POLLIN: i16 = 1;
    let mut fds = [PollFd { fd: first, events: POLLIN, revents: 0 }, PollFd { fd: second.unwrap_or(-1), events: POLLIN, revents: 0 }];
    // SAFETY: `fds` is a live array of two `pollfd`s; a negative fd is ignored by poll.
    let n = unsafe { poll(fds.as_mut_ptr(), 2, timeout_ms) };
    if n <= 0 {
        return (false, false);
    }
    // POLLERR/POLLHUP also count so a closed link wakes the loop
    (fds[0].revents != 0, fds[1].revents != 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_negative_timeout_has_no_deadline() {
        assert!(super::deadline_for(-1).is_none());
        assert!(super::deadline_for(0).is_some());
    }

    #[test]
    fn poll_fds_sleeps_until_the_extra_fd_is_written() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        let (a, mut b) = UnixStream::pair().unwrap();
        let (w, _w2) = UnixStream::pair().unwrap();
        let t = std::time::Instant::now();
        assert_eq!(super::poll_fds(w.as_raw_fd(), Some(a.as_raw_fd()), 30), (false, false));
        assert!(t.elapsed() >= std::time::Duration::from_millis(25), "idle poll waits, it does not spin");
        b.write_all(b"x").unwrap();
        assert_eq!(super::poll_fds(w.as_raw_fd(), Some(a.as_raw_fd()), -1), (false, true));
    }

    use super::*;

    #[test]
    fn pixel_size_scales_and_rounds_without_zero() {
        assert_eq!(pixel_size((100, 40), 1.0), (100, 40));
        // 100 x 1.5625 = 156.25 rounds to 156; 40 x 1.5625 = 62.5 rounds half away from zero to 63.
        assert_eq!(pixel_size((100, 40), 1.5625), (156, 63));
        assert_eq!(pixel_size((0, 0), 1.0), (1, 1));
        assert_eq!(pixel_size((2, 3), 2.0), (4, 6));
    }

    #[test]
    fn pick_slot_takes_the_first_free_slot_from_the_start_and_wraps() {
        assert_eq!(pick_slot(&[false, false, false], 0), Some(0));
        assert_eq!(pick_slot(&[false, false, false], 2), Some(2));
        assert_eq!(pick_slot(&[true, false, true], 0), Some(1));
        // Starting at 2 with slot 2 busy wraps to slot 0.
        assert_eq!(pick_slot(&[false, true, true], 2), Some(0));
    }

    #[test]
    fn pick_slot_is_none_when_all_busy_or_no_slots() {
        assert_eq!(pick_slot(&[true, true, true], 1), None);
        assert_eq!(pick_slot(&[], 0), None);
    }

    #[test]
    fn slot_tags_round_trip_surface_and_slot() {
        for (id, slot) in [(1u32, 0usize), (7, 2), (u32::MAX, 1)] {
            let tag = slot_tag(SurfaceId(id), slot);
            assert_eq!(split_slot_tag(tag), (SurfaceId(id), slot));
        }
        assert_eq!(slot_tag(SurfaceId(3), 2), (3u64 << 32) | 2);
    }

    #[test]
    fn anchors_combine_like_the_protocol_bits() {
        assert_eq!((Anchor::TOP | Anchor::LEFT | Anchor::RIGHT).0, 1 | 4 | 8);
        assert_eq!(Anchor::BOTTOM | Anchor::TOP, Anchor(3));
        assert_eq!(Anchor::default(), Anchor(0));
    }

    #[test]
    fn layers_map_to_the_protocol_layers() {
        assert_eq!(wl_layer(Layer::Background), zwlr_layer_shell_v1::Layer::Background);
        assert_eq!(wl_layer(Layer::Bottom), zwlr_layer_shell_v1::Layer::Bottom);
        assert_eq!(wl_layer(Layer::Top), zwlr_layer_shell_v1::Layer::Top);
        assert_eq!(wl_layer(Layer::Overlay), zwlr_layer_shell_v1::Layer::Overlay);
    }

    #[test]
    fn the_first_draw_builds_the_root_layer() {
        let tx = first_draw_transaction(5);
        assert_eq!(tx.seq, 5);
        assert_eq!(
            tx.changes,
            vec![
                Change::Create(LayerId(1)),
                Change::SetRoot(LayerId(1)),
                Change::SetContents(LayerId(1), LayerContents::Buffer(ContentsId(1))),
            ]
        );
        assert!(tx.implicit.is_none());
    }
}
