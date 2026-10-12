//! A chrome client's connection and surfaces: one Wayland connection per process with a Vulkan device and a
//! `bifrost_2d::Renderer`, and layer-shell surfaces whose content is drawn with `bifrost-2d` into dma-buf backing
//! stores and committed as a `bifrost_layer_v1` layer tree (the root layer shows the drawn content; extra layers
//! animate in the window server, such as the Dock's magnified icons).
//!
//! Owner: wave G2 agent `chrome-core`. The three clients (menu bar, Dock, panels) use only this API.

use bifrost_layers::Transaction;
use bifrost_render::canvas::RectF;
use bifrost_render::display_list::DisplayList;

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
    Key { surface: SurfaceId, keysym: u32, pressed: bool },
    /// A frame callback: the surface may draw again.
    Frame(SurfaceId),
    /// A layer animation ended (`bifrost_layer_context_v1.animation_finished`).
    AnimationFinished { surface: SurfaceId, id: u32 },
}

/// The process's connection, GPU and surfaces.
pub struct Display {}

impl Display {
    /// Connects to `WAYLAND_DISPLAY`, binds the globals (compositor, layer shell, linux-dmabuf, bifrost_layer_v1,
    /// seat) and opens the GPU. Fails when the compositor lacks layer shell or `bifrost_layer_manager_v1`.
    pub fn connect() -> Result<Display, String> {
        todo!("chrome-core")
    }

    /// Makes a layer-shell surface; it is shown after its first `Configured` and `draw`.
    pub fn create(&mut self, spec: SurfaceSpec) -> Result<SurfaceId, String> {
        let _ = spec;
        todo!("chrome-core")
    }

    /// Changes a surface's size, exclusive zone or margins (the Dock grows and moves).
    pub fn update(&mut self, id: SurfaceId, spec: SurfaceSpec) {
        let _ = (id, spec);
        todo!("chrome-core")
    }

    pub fn destroy(&mut self, id: SurfaceId) {
        let _ = id;
        todo!("chrome-core")
    }

    /// Draws `list` (in device pixels of the configured size times scale) into the surface's next backing store,
    /// only `damage` when given, binds it to the root layer and commits.
    pub fn draw(&mut self, id: SurfaceId, list: &DisplayList, damage: Option<RectF>) -> Result<(), String> {
        let _ = (id, list, damage);
        todo!("chrome-core")
    }

    /// Commits a layer transaction for the surface's tree (extra layers beyond the root, animations).
    pub fn commit_layers(&mut self, id: SurfaceId, tx: &Transaction) -> Result<(), String> {
        let _ = (id, tx);
        todo!("chrome-core")
    }

    /// Waits up to `timeout_ms` (or for `extra_fd` to be readable, the shell link) and returns what happened.
    pub fn dispatch(&mut self, timeout_ms: i32, extra_fd: Option<std::os::fd::RawFd>) -> Result<Vec<Event>, String> {
        let _ = (timeout_ms, extra_fd);
        todo!("chrome-core")
    }
}
