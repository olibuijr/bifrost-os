//! Instruments, by family: sampled zones (`zones`), subtractive synths (`subtractive`), and the procedural and
//! sample-pad devices (`misc`). Unknown types have no spec, so their tracks are silent.
pub mod misc;
pub mod subtractive;
pub mod zones;

use super::{Ctx, Instrument, Params, P};
use crate::model::Instrument as Model;

pub fn spec(kind: &str) -> Option<&'static [P]> {
    if zones::is_sampled(kind) {
        return Some(zones::spec_for(kind));
    }
    if let Some(s) = subtractive::spec(kind) {
        return Some(s);
    }
    misc::spec(kind)
}

/// Wrapper defaults for a device (they replace the spec defaults, song values still win).
pub fn defaults(kind: &str) -> &'static [(&'static str, f64)] {
    if zones::is_sampled(kind) {
        zones::defaults(kind)
    } else {
        &[]
    }
}

pub fn new(kind: &str, params: &Params, model: &Model, ctx: &Ctx) -> Option<Box<dyn Instrument>> {
    if zones::is_sampled(kind) {
        return zones::new(kind, params, ctx);
    }
    if subtractive::spec(kind).is_some() {
        return subtractive::new(kind, params, ctx);
    }
    misc::new(kind, params, model, ctx)
}
