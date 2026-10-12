//! Per-app volume: a scalar 0..=1 per app id (default 1), applied by the mixer as the client's gain.

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct AppVolumes(BTreeMap<String, f32>);

impl AppVolumes {
    pub fn get(&self, app: &str) -> f32 {
        self.0.get(app).copied().unwrap_or(1.0)
    }

    /// Sets the volume, clamped to 0..=1 (NaN counts as 1). Returns the stored value.
    pub fn set(&mut self, app: &str, v: f32) -> f32 {
        let v = if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) };
        self.0.insert(app.to_string(), v);
        v
    }

    pub fn save(&self) -> String {
        self.0.iter().map(|(a, v)| format!("{a}={v}\n")).collect()
    }

    pub fn load(&mut self, text: &str) {
        for l in text.lines() {
            if let Some((a, v)) = l.split_once('=') {
                if let Ok(v) = v.parse::<f32>() {
                    self.set(a, v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_defaults_and_round_trips() {
        let mut v = AppVolumes::default();
        assert_eq!(v.get("x"), 1.0);
        assert_eq!(v.set("x", 1.7), 1.0);
        assert_eq!(v.set("y", -1.0), 0.0);
        assert_eq!(v.set("z", 0.5), 0.5);
        let mut w = AppVolumes::default();
        w.load(&v.save());
        assert_eq!((w.get("y"), w.get("z")), (0.0, 0.5));
    }
}
