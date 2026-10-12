//! `bifrost_material_v1` (protocol/bifrost-material-v1.xml, docs/window-server.md "Materials"): a material kind, state,
//! blending, emphasis and corner radii per region, applied on the surface's commit. The renderer reads `materials`
//! for each surface it draws; with none, the plain `ext-background-effect-v1` blur region is used
//! (`material_kind::effective`).

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
pub mod protocol {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocol/bifrost-material-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocol/bifrost-material-v1.xml");
}

use std::sync::Mutex;

use smithay::{
    reexports::wayland_server::{
        backend::GlobalId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
        Resource, Weak,
    },
    wayland::compositor::{get_region_attributes, with_states},
};

use self::protocol::{
    bifrost_material_manager_v1::{self, BifrostMaterialManagerV1},
    bifrost_material_surface_v1::{self, BifrostMaterialSurfaceV1},
};
use crate::{background_effect::resolve, material_kind::*, state::WmState};

/// The material regions of a surface: the committed ones and the ones waiting for the next commit.
#[derive(Default)]
struct Materials {
    pending: Option<Vec<MaterialSpec>>,
    current: Vec<MaterialSpec>,
    claimed: bool,
}

/// Most material regions one surface may hold (a client controls the input).
const MAX_REGIONS: usize = 16;

pub fn init(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<WmState, BifrostMaterialManagerV1, ()>(1, ())
}

/// The committed material regions of `surface` (surface coordinates), empty when it sent none.
pub fn materials(surface: &WlSurface) -> Vec<MaterialSpec> {
    with_states(surface, |s| s.data_map.get::<Mutex<Materials>>().map(|m| m.lock().unwrap().current.clone()).unwrap_or_default())
}

/// The regions the renderer draws for `surface`: its materials, else its plain blur regions.
pub fn effective_regions(surface: &WlSurface) -> Vec<MaterialSpec> {
    effective(materials(surface), crate::background_effect::blur_region(surface))
}

/// Called on every surface commit: pending regions become the current ones.
pub fn commit(surface: &WlSurface) {
    with_states(surface, |s| {
        if let Some(m) = s.data_map.get::<Mutex<Materials>>() {
            let mut m = m.lock().unwrap();
            if let Some(p) = m.pending.take() {
                m.current = p;
            }
        }
    });
}

impl GlobalDispatch<BifrostMaterialManagerV1, ()> for WmState {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<BifrostMaterialManagerV1>, _: &(), init: &mut DataInit<'_, Self>) {
        init.init(resource, ());
    }
}

impl Dispatch<BifrostMaterialManagerV1, ()> for WmState {
    fn request(_: &mut Self, _: &Client, manager: &BifrostMaterialManagerV1, request: bifrost_material_manager_v1::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        if let bifrost_material_manager_v1::Request::GetMaterial { id, surface } = request {
            let taken = with_states(&surface, |s| {
                s.data_map.insert_if_missing_threadsafe(|| Mutex::new(Materials::default()));
                let mut m = s.data_map.get::<Mutex<Materials>>().unwrap().lock().unwrap();
                std::mem::replace(&mut m.claimed, true)
            });
            if taken {
                manager.post_error(bifrost_material_manager_v1::Error::MaterialExists, "the surface already has a material object");
                return;
            }
            init.init(id, surface.downgrade());
        }
    }
}

impl Dispatch<BifrostMaterialSurfaceV1, Weak<WlSurface>> for WmState {
    fn request(_: &mut Self, _: &Client, obj: &BifrostMaterialSurfaceV1, request: bifrost_material_surface_v1::Request, surface: &Weak<WlSurface>, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        use bifrost_material_surface_v1::{Error, Request};
        let Ok(surface) = surface.upgrade() else { return };
        // None: leave pending alone; Some: replace it
        let mut edit: Option<Box<dyn FnOnce(&mut Vec<MaterialSpec>)>> = None;
        let mut destroyed = false;
        match request {
            Request::Destroy => {
                destroyed = true;
                edit = Some(Box::new(|v| v.clear()));
            }
            Request::Clear => edit = Some(Box::new(|v| v.clear())),
            Request::AddRegion {
                region,
                kind,
                state,
                blending,
                emphasized,
                radius_top_left,
                radius_top_right,
                radius_bottom_right,
                radius_bottom_left,
            } => {
                let Some(kind) = MaterialKind::from_wire(kind) else {
                    obj.post_error(Error::InvalidKind, "unknown material kind");
                    return;
                };
                let Some(state) = MaterialState::from_wire(state) else {
                    obj.post_error(Error::InvalidState, "unknown material state");
                    return;
                };
                let Some(blending) = Blending::from_wire(blending) else {
                    obj.post_error(Error::InvalidBlending, "unknown blending mode");
                    return;
                };
                let rects = resolve(&get_region_attributes(&region).rects);
                let spec = MaterialSpec {
                    rects,
                    kind,
                    state,
                    blending,
                    emphasized: emphasized != 0,
                    radii: [radius_top_left, radius_top_right, radius_bottom_right, radius_bottom_left].map(|r| r.min(4096)),
                };
                edit = Some(Box::new(move |v| {
                    if v.len() < MAX_REGIONS && !spec.rects.is_empty() {
                        v.push(spec);
                    }
                }));
            }
            _ => {}
        }
        with_states(&surface, |s| {
            if let Some(m) = s.data_map.get::<Mutex<Materials>>() {
                let mut m = m.lock().unwrap();
                if let Some(edit) = edit {
                    // start from the pending list when there is one, else from the committed one
                    let mut list = m.pending.take().unwrap_or_else(|| m.current.clone());
                    edit(&mut list);
                    m.pending = Some(list);
                }
                if destroyed {
                    m.claimed = false;
                }
            }
        });
    }
}
