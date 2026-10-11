//! Zone selection: nearest zone for pitch and velocity, round robin, and the velocity-layer crossfade pair
//! (`pick`, `rrOf` and `pickLayers` in multisample.js).

use super::manifest::Manifest;
use std::collections::HashMap;

const ARTIC_KEYS: [&str; 5] = ["arco", "stacc", "pizz", "trem", "marc"];

/// Zones an articulation can play (`poolOf` in multisample.js): the tagged zones of that articulation, else the
/// arco/default set, else every zone. Manifests without articulation tags use every zone.
pub fn pool_indices(man: &Manifest, artic: usize) -> Vec<usize> {
    if !man.zones.iter().any(|z| z.artic.is_some()) {
        return (0..man.zones.len()).collect();
    }
    let want = ARTIC_KEYS[artic.min(4)];
    if want != "arco" {
        let hit: Vec<usize> = (0..man.zones.len()).filter(|&i| man.zones[i].artic.as_deref() == Some(want)).collect();
        if !hit.is_empty() {
            return hit;
        }
    }
    let arco: Vec<usize> = (0..man.zones.len()).filter(|&i| man.zones[i].is_arco()).collect();
    if arco.is_empty() {
        (0..man.zones.len()).collect()
    } else {
        arco
    }
}

/// The zone nearest to (pitch, velocity), as in multisample.js `pick`.
pub fn pick(man: &Manifest, pool: &[usize], pitch: f64, vel: f64) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for &i in pool {
        let z = &man.zones[i];
        let dp = if pitch < z.lo {
            z.lo - pitch
        } else if pitch > z.hi {
            pitch - z.hi
        } else {
            0.0
        };
        let dv = if vel < z.vlo {
            z.vlo - vel
        } else if vel > z.vhi {
            vel - z.vhi
        } else {
            0.0
        };
        let d = dp * 10.0 + dv + (pitch - z.root).abs() * 1e-3 + z.rr.map_or(0.0, |r| 1e-6 * r as f64);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}

/// Round robin among zones sharing region and velocity range. `counters` keeps one cursor per region.
pub fn round_robin(man: &Manifest, counters: &mut HashMap<(i64, i64), usize>, pool: &[usize], zi: usize) -> usize {
    let z = man.zones[zi].clone();
    if z.rr.is_none() {
        return zi;
    }
    let mut set: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|&i| {
            let y = &man.zones[i];
            y.rr.is_some() && y.lo == z.lo && y.hi == z.hi && y.vlo == z.vlo && y.vhi == z.vhi
        })
        .collect();
    if set.len() < 2 {
        return zi;
    }
    set.sort_by_key(|&i| man.zones[i].rr);
    let key = ((z.lo * 1000.0) as i64, (z.vlo * 1000.0) as i64);
    let counter = counters.entry(key).or_insert(0);
    let pick = set[*counter % set.len()];
    *counter += 1;
    pick
}

/// Up to two velocity layers with equal-power weights (`pickLayers` in multisample.js).
pub fn layers_for(
    man: &Manifest,
    counters: &mut HashMap<(i64, i64), usize>,
    artic: usize,
    pitch: f64,
    vel: f64,
) -> Vec<(usize, f32)> {
    let pool = pool_indices(man, artic);
    let Some(zi) = pick(man, &pool, pitch, vel) else { return vec![] };
    let mut zi = zi;
    // rrNeighbor: for in-between pitches with no round robin, alternate with the neighbouring root
    if man.rr_neighbor && !man.zones.iter().any(|z| z.rr.is_some()) && man.zones[zi].root != pitch {
        let (root, vlo, vhi) = (man.zones[zi].root, man.zones[zi].vlo, man.zones[zi].vhi);
        let alt = pool.iter().copied().find(|&i| {
            let y = &man.zones[i];
            i != zi
                && y.vlo == vlo
                && y.vhi == vhi
                && y.root != root
                && (y.root - pitch).abs() <= 2.0
                && (y.root - pitch).signum() != (root - pitch).signum()
        });
        if let Some(a) = alt {
            let c = counters.entry((pitch as i64 * 1000, i64::MIN)).or_insert(0);
            *c += 1;
            if *c % 2 == 1 {
                zi = a;
            }
        }
    }
    let zi = round_robin(man, counters, &pool, zi);
    let z = man.zones[zi].clone();
    let xf = man.vel_xfade;
    if !(xf > 0.0) || vel < z.vlo || vel > z.vhi {
        return vec![(zi, 1.0)];
    }
    let region: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|&i| {
            let y = &man.zones[i];
            y.lo == z.lo && y.hi == z.hi && i != zi
        })
        .collect();
    let (mut lo, mut hi, mut boundary) = (None, None, 0.0);
    if vel > z.vhi - xf && z.vhi < 1.0 {
        hi = region.iter().copied().find(|&i| (man.zones[i].vlo - z.vhi).abs() < 0.02);
        if hi.is_some() {
            lo = Some(zi);
            boundary = z.vhi;
        }
    }
    if hi.is_none() && vel < z.vlo + xf && z.vlo > 0.0 {
        lo = region.iter().copied().find(|&i| (man.zones[i].vhi - z.vlo).abs() < 0.02);
        if lo.is_some() {
            hi = Some(zi);
            boundary = z.vlo;
        }
    }
    match (lo, hi) {
        (Some(lo), Some(hi)) => {
            let t = ((vel - (boundary - xf)) / (2.0 * xf)).clamp(0.0, 1.0) as f32;
            let (wl, wh) = ((std::f32::consts::FRAC_PI_2 * t).cos(), (std::f32::consts::FRAC_PI_2 * t).sin());
            [(lo, wl), (hi, wh)].into_iter().filter(|(_, w)| *w > 0.01).collect()
        }
        _ => vec![(zi, 1.0)],
    }
}
