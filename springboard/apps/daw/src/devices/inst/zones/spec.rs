//! Parameter spec per sampled device: the shared list minus what the device's wrapper hides, plus the flag
//! parameters from plugin.json and the 808 wrapper (`glide`, `glidecurve`, `artic`, `pedal` in multisample.js).

use super::SPEC;
use crate::devices::{p, Curve, P};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Spec for a sampled device. Built once per device; the list lives for the whole run.
pub fn spec_for(kind: &str) -> &'static [P] {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static [P]>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = cache.get(kind) {
        return *s;
    }
    let s: &'static [P] = Box::leak(build(kind).into_boxed_slice());
    cache.insert(kind.to_string(), s);
    s
}

fn build(kind: &str) -> Vec<P> {
    let kind = super::plugin_dir(kind);
    let hide: &[&str] = match kind {
        "808clean" | "808dist" | "808punch" => &["vibrato", "rate", "detune"],
        "boombapkit" | "trapkit" | "acoustickit" => &["attack", "release", "vibrato", "rate", "detune"],
        _ => &[],
    };
    let mut out: Vec<P> = SPEC.iter().copied().filter(|s| !hide.contains(&s.name)).collect();
    let (legato, artic, pedal) = plugin_flags(kind);
    if legato || kind.starts_with("808") {
        out.push(p("glide", 0.0, 0.5, 0.0, Curve::Lin, "s"));
        out.push(p("glidecurve", 0.0, 1.0, 0.5, Curve::Lin, ""));
    }
    if artic {
        out.push(p("artic", 0.0, 4.0, 0.0, Curve::Lin, ""));
    }
    if pedal {
        out.push(p("pedal", 0.0, 1.0, 0.0, Curve::Lin, ""));
    }
    if matches!(kind, "boombapkit" | "trapkit" | "acoustickit") {
        out.push(p("decay", 0.02, 4.0, 4.0, Curve::Log, "s"));
    }
    out
}

/// (legato, articulations, pedal) flags from `<plugins>/<kind>/plugin.json`.
pub(super) fn plugin_flags(kind: &str) -> (bool, bool, bool) {
    let root = std::env::var("DAW_PLUGINS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::assets::asset_root().join("plugins"));
    let json = std::fs::read_to_string(root.join(kind).join("plugin.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let Some(v) = json else { return (false, false, false) };
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    (flag("legato"), flag("articulations"), flag("pedal"))
}
