//! Zone manifest (`data/plugins/<id>/zones.json`): parsing and the normalised zone records (`norm` in multisample.js).

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug)]
pub struct RawZone {
    file: String,
    root: f64,
    lo: Option<f64>,
    hi: Option<f64>,
    vlo: Option<f64>,
    vhi: Option<f64>,
    gain: Option<f64>,
    tune: Option<f64>,
    #[serde(rename = "loopStart")]
    loop_start: Option<f64>,
    #[serde(rename = "loopEnd")]
    loop_end: Option<f64>,
    #[serde(rename = "oneShot")]
    one_shot: Option<bool>,
    pitched: Option<bool>,
    choke: Option<String>,
    rr: Option<u32>,
    artic: Option<String>,
}

#[derive(Deserialize, Debug)]
struct RawManifest {
    attack: Option<f64>,
    release: Option<f64>,
    #[serde(rename = "loopUnits")]
    loop_units: Option<String>,
    #[serde(rename = "sampleRate")]
    sample_rate: Option<f64>,
    #[serde(rename = "oneShot")]
    one_shot: Option<bool>,
    pitched: Option<bool>,
    #[serde(rename = "monoSub")]
    mono_sub: Option<bool>,
    #[serde(rename = "chokeTime")]
    choke_time: Option<f64>,
    #[serde(rename = "velXfade")]
    vel_xfade: Option<f64>,
    #[serde(rename = "autoLoop")]
    auto_loop: Option<bool>,
    #[serde(rename = "velCurve")]
    vel_curve: Option<f64>,
    #[serde(rename = "pedalRelease")]
    pedal_release: Option<f64>,
    #[serde(rename = "pedalMax")]
    pedal_max: Option<f64>,
    #[serde(rename = "loopXfade")]
    loop_xfade: Option<bool>,
    #[serde(rename = "releaseNoise")]
    release_noise: Option<RawNoise>,
    #[serde(rename = "softAttack")]
    soft_attack: Option<f64>,
    #[serde(rename = "releaseKeytrack")]
    release_keytrack: Option<bool>,
    #[serde(rename = "rrNeighbor")]
    rr_neighbor: Option<bool>,
    #[serde(rename = "brassEnv")]
    brass_env: Option<bool>,
    #[serde(default)]
    zones: Vec<RawZone>,
}

/// `releaseNoise` of the manifest: the damper thump on note-off.
#[derive(Deserialize, Debug)]
struct RawNoise {
    gain: f64,
    tone: Option<f64>,
}

/// A zone with its defaults filled in.
#[derive(Clone, Debug)]
pub struct Zone {
    pub file: PathBuf,
    pub root: f64,
    pub lo: f64,
    pub hi: f64,
    pub vlo: f64,
    pub vhi: f64,
    pub gain: f64,
    pub tune: f64,
    pub loop_start: Option<f64>,
    pub loop_end: Option<f64>,
    pub one_shot: Option<bool>,
    pub pitched: Option<bool>,
    pub choke: Option<String>,
    pub rr: Option<u32>,
    pub artic: Option<String>,
}

impl Zone {
    /// Zones without an articulation tag are the arco (default) set.
    pub fn is_arco(&self) -> bool {
        self.artic.as_deref().map_or(true, |a| a == "arco")
    }
}

#[derive(Debug)]
pub struct Manifest {
    pub attack: f64,
    pub release: f64,
    pub loop_units_samples: bool,
    pub manifest_sr: f64,
    pub one_shot: bool,
    pub pitched: bool,
    pub mono_sub: bool,
    pub choke_time: f64,
    pub vel_xfade: f64,
    pub auto_loop: bool,
    pub vel_curve: Option<f64>,
    /// sustain-pedal release time in seconds (None = the note's own release)
    pub pedal_release: Option<f64>,
    /// longest pedal ring of a held voice, seconds
    pub pedal_max: f64,
    /// false disables the loop-seam crossfade
    pub loop_xfade: bool,
    /// (gain, lowpass Hz) of the damper thump on note-off, None when the manifest has none
    pub release_noise: Option<(f64, f64)>,
    /// extra attack lengthening for soft notes (`softAttack`, multiplier)
    pub soft_attack: f64,
    pub release_keytrack: bool,
    /// alternate the neighbouring root for in-between pitches (`rrNeighbor`)
    pub rr_neighbor: bool,
    /// lowpass opens from half to full cutoff over the attack (`brassEnv`)
    pub brass_env: bool,
    pub zones: Vec<Zone>,
}

/// Reads and normalises `<dir>/zones.json`. None when missing or unreadable.
pub fn load(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join("zones.json")).ok()?;
    let raw: RawManifest = serde_json::from_str(&text).ok()?;
    let zones = raw
        .zones
        .into_iter()
        .map(|z| Zone {
            file: dir.join(&z.file),
            root: z.root,
            lo: z.lo.unwrap_or(z.root),
            hi: z.hi.unwrap_or(z.root),
            vlo: z.vlo.unwrap_or(0.0),
            vhi: z.vhi.unwrap_or(1.0),
            gain: z.gain.unwrap_or(1.0),
            tune: z.tune.unwrap_or(0.0),
            loop_start: z.loop_start,
            loop_end: z.loop_end,
            one_shot: z.one_shot,
            pitched: z.pitched,
            choke: z.choke,
            rr: z.rr,
            artic: z.artic,
        })
        .collect();
    Some(Manifest {
        attack: raw.attack.unwrap_or(0.005).max(1e-4),
        release: raw.release.unwrap_or(0.3).max(1e-3),
        loop_units_samples: raw.loop_units.as_deref() == Some("samples"),
        manifest_sr: raw.sample_rate.unwrap_or(44_100.0),
        one_shot: raw.one_shot.unwrap_or(false),
        pitched: raw.pitched.unwrap_or(true),
        mono_sub: raw.mono_sub.unwrap_or(false),
        choke_time: raw.choke_time.unwrap_or(0.008).max(0.004),
        vel_xfade: raw.vel_xfade.unwrap_or(0.08),
        auto_loop: raw.auto_loop.unwrap_or(false),
        vel_curve: raw.vel_curve,
        pedal_release: raw.pedal_release.map(|r| r.max(0.01)),
        pedal_max: raw.pedal_max.unwrap_or(8.0).max(0.0),
        loop_xfade: raw.loop_xfade.unwrap_or(true),
        release_noise: raw.release_noise.filter(|n| n.gain > 0.0).map(|n| (n.gain, n.tone.unwrap_or(900.0))),
        soft_attack: raw.soft_attack.unwrap_or(0.0),
        release_keytrack: raw.release_keytrack.unwrap_or(false),
        rr_neighbor: raw.rr_neighbor.unwrap_or(false),
        brass_env: raw.brass_env.unwrap_or(false),
        zones,
    })
}
