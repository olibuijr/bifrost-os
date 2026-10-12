//! Parameters with ramps. The control side holds a `ParamHandle` (any thread, wait-free: atomics); the unit owns
//! the `ParameterTree`, which on the RT thread picks up changes and ramps linearly to the target in frames.

use crate::unit::UnitError;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// What a parameter value means (for display and the host UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUnit {
    Generic,
    Decibels,
    Hertz,
    Percent,
    Seconds,
    Ratio,
    Boolean,
    /// Integer index into `value_strings`.
    Indexed,
}

/// The definition of one parameter. `id` equals the parameter's position in the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: u32,
    pub name: String,
    /// Slash-separated group path ("EQ/Band 1"); empty for the root.
    pub group: String,
    pub unit: ParamUnit,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub read_only: bool,
    pub value_strings: Vec<String>,
    /// The ramp length a plain `set` uses (0 = jump).
    pub ramp_frames: u32,
}

impl ParamSpec {
    pub fn new(id: u32, name: &str, unit: ParamUnit, min: f32, max: f32, default: f32) -> Result<ParamSpec, UnitError> {
        if !(min.is_finite() && max.is_finite() && default.is_finite()) || min >= max || default < min || default > max {
            return Err(UnitError::BadValue);
        }
        Ok(ParamSpec {
            id,
            name: name.to_string(),
            group: String::new(),
            unit,
            min,
            max,
            default,
            read_only: false,
            value_strings: Vec::new(),
            ramp_frames: 0,
        })
    }

    pub fn in_group(mut self, group: &str) -> ParamSpec {
        self.group = group.to_string();
        self
    }

    pub fn with_ramp(mut self, frames: u32) -> ParamSpec {
        self.ramp_frames = frames;
        self
    }

    pub fn read_only(mut self) -> ParamSpec {
        self.read_only = true;
        self
    }

    pub fn with_value_strings(mut self, strings: &[&str]) -> ParamSpec {
        self.value_strings = strings.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn clamp(&self, v: f32) -> f32 {
        if v.is_nan() {
            self.default
        } else {
            v.clamp(self.min, self.max)
        }
    }

    /// 0.0 at `min`, 1.0 at `max`.
    pub fn normalized(&self, v: f32) -> f32 {
        (self.clamp(v) - self.min) / (self.max - self.min)
    }

    pub fn value_from_normalized(&self, p: f32) -> f32 {
        let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
        self.min + p * (self.max - self.min)
    }
}

struct Slot {
    target: AtomicU32,
    ramp: AtomicU32,
    seq: AtomicU32,
}

/// The control-side view of a unit's parameters; cheap to clone, `Send + Sync`.
#[derive(Clone)]
pub struct ParamHandle {
    specs: Arc<[ParamSpec]>,
    slots: Arc<[Slot]>,
}

impl ParamHandle {
    fn new(specs: Vec<ParamSpec>) -> Result<ParamHandle, UnitError> {
        for (i, s) in specs.iter().enumerate() {
            if s.id as usize != i {
                return Err(UnitError::BadParameter);
            }
        }
        let slots: Vec<Slot> = specs
            .iter()
            .map(|s| Slot { target: AtomicU32::new(s.default.to_bits()), ramp: AtomicU32::new(0), seq: AtomicU32::new(0) })
            .collect();
        Ok(ParamHandle { specs: specs.into(), slots: slots.into() })
    }

    pub fn specs(&self) -> &[ParamSpec] {
        &self.specs
    }

    pub fn spec(&self, id: u32) -> Option<&ParamSpec> {
        self.specs.get(id as usize)
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// The target value.
    pub fn get(&self, id: u32) -> Result<f32, UnitError> {
        let s = self.slots.get(id as usize).ok_or(UnitError::BadParameter)?;
        Ok(f32::from_bits(s.target.load(Ordering::Acquire)))
    }

    /// Sets the target, ramping over the spec's default ramp. Returns the clamped value.
    pub fn set(&self, id: u32, value: f32) -> Result<f32, UnitError> {
        let r = self.spec(id).ok_or(UnitError::BadParameter)?.ramp_frames;
        self.set_ramped(id, value, r)
    }

    /// Sets the target, reaching it `frames` frames after the unit next picks it up.
    pub fn set_ramped(&self, id: u32, value: f32, frames: u32) -> Result<f32, UnitError> {
        let spec = self.spec(id).ok_or(UnitError::BadParameter)?;
        if spec.read_only {
            return Err(UnitError::ReadOnly);
        }
        let v = spec.clamp(value);
        let s = &self.slots[id as usize];
        s.ramp.store(frames, Ordering::Relaxed);
        s.target.store(v.to_bits(), Ordering::Relaxed);
        s.seq.fetch_add(1, Ordering::Release);
        Ok(v)
    }

    /// Calls `f(id, target, ramp_frames)` for each parameter changed since `seen` (one counter per parameter,
    /// updated here). Wait-free, no allocation.
    pub fn poll_changes(&self, seen: &mut [u32], mut f: impl FnMut(u32, f32, u32)) {
        for (i, s) in self.slots.iter().enumerate() {
            let q = s.seq.load(Ordering::Acquire);
            if q != seen[i] {
                seen[i] = q;
                f(i as u32, f32::from_bits(s.target.load(Ordering::Relaxed)), s.ramp.load(Ordering::Relaxed));
            }
        }
    }

    /// All target values (for presets), skipping read-only parameters.
    pub fn snapshot(&self) -> Vec<(u32, f32)> {
        self.specs
            .iter()
            .filter(|s| !s.read_only)
            .map(|s| (s.id, f32::from_bits(self.slots[s.id as usize].target.load(Ordering::Acquire))))
            .collect()
    }

    /// Restores values from a snapshot (jumping, no ramp).
    pub fn restore(&self, state: &[(u32, f32)]) -> Result<(), UnitError> {
        for &(id, v) in state {
            self.set_ramped(id, v, 0)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Rt {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

/// The unit's RT-side parameters: current values and ramps.
pub struct ParameterTree {
    handle: ParamHandle,
    rt: Vec<Rt>,
    seen: Vec<u32>,
}

impl ParameterTree {
    /// `specs[i].id` must equal `i`.
    pub fn new(specs: Vec<ParamSpec>) -> Result<ParameterTree, UnitError> {
        let handle = ParamHandle::new(specs)?;
        let rt = handle.specs.iter().map(|s| Rt { value: s.default, target: s.default, step: 0.0, remaining: 0 }).collect();
        let seen = vec![0; handle.len()];
        Ok(ParameterTree { handle, rt, seen })
    }

    pub fn handle(&self) -> ParamHandle {
        self.handle.clone()
    }

    pub fn specs(&self) -> &[ParamSpec] {
        self.handle.specs()
    }

    /// Picks up control-side changes and starts their ramps (call at the start of a block).
    pub fn sync(&mut self) {
        let rt = &mut self.rt;
        self.handle.poll_changes(&mut self.seen, |id, target, ramp| {
            let r = &mut rt[id as usize];
            r.target = target;
            if ramp == 0 {
                r.value = target;
                r.remaining = 0;
                r.step = 0.0;
            } else {
                r.step = (target - r.value) / ramp as f32;
                r.remaining = ramp;
            }
        });
    }

    /// The current value (at the start of the block, before `advance`/`fill`).
    pub fn value(&self, id: u32) -> f32 {
        self.rt[id as usize].value
    }

    /// Returns the value at the start of the block and moves the ramp on by `frames`.
    pub fn advance(&mut self, id: u32, frames: usize) -> f32 {
        let r = &mut self.rt[id as usize];
        let v0 = r.value;
        if r.remaining > 0 {
            let n = (frames as u64).min(r.remaining as u64) as u32;
            r.remaining -= n;
            r.value = if r.remaining == 0 { r.target } else { r.value + r.step * n as f32 };
        }
        v0
    }

    /// Writes per-frame values (`out[k]` = start + step * k while ramping) and moves the ramp on.
    pub fn fill(&mut self, id: u32, out: &mut [f32]) {
        let r = &mut self.rt[id as usize];
        for o in out.iter_mut() {
            *o = r.value;
            if r.remaining > 0 {
                r.remaining -= 1;
                r.value = if r.remaining == 0 { r.target } else { r.value + r.step };
            }
        }
    }

    /// True while a ramp is running.
    pub fn is_ramping(&self, id: u32) -> bool {
        self.rt[id as usize].remaining > 0
    }

    /// Jumps every parameter to its default.
    pub fn reset(&mut self) {
        for (i, s) in self.handle.specs.iter().enumerate() {
            self.rt[i] = Rt { value: s.default, target: s.default, step: 0.0, remaining: 0 };
        }
        for (i, s) in self.handle.slots.iter().enumerate() {
            s.target.store(self.handle.specs[i].default.to_bits(), Ordering::Relaxed);
            self.seen[i] = s.seq.load(Ordering::Acquire);
        }
    }
}

impl std::fmt::Debug for ParamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ParamHandle({} parameters)", self.specs.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> ParameterTree {
        ParameterTree::new(vec![
            ParamSpec::new(0, "Gain", ParamUnit::Ratio, 0.0, 2.0, 1.0).unwrap(),
            ParamSpec::new(1, "Level", ParamUnit::Generic, -10.0, 10.0, 0.0).unwrap().read_only(),
        ])
        .unwrap()
    }

    #[test]
    fn spec_validation_and_normalization() {
        assert!(ParamSpec::new(0, "x", ParamUnit::Generic, 1.0, 1.0, 1.0).is_err());
        assert!(ParamSpec::new(0, "x", ParamUnit::Generic, 0.0, 1.0, 2.0).is_err());
        let s = ParamSpec::new(0, "x", ParamUnit::Hertz, 20.0, 120.0, 20.0).unwrap();
        assert_eq!(s.normalized(70.0), 0.5);
        assert_eq!(s.value_from_normalized(0.25), 45.0);
        assert_eq!(s.clamp(500.0), 120.0);
        assert_eq!(s.clamp(f32::NAN), 20.0);
    }

    #[test]
    fn dense_ids_are_required() {
        let bad = ParameterTree::new(vec![ParamSpec::new(3, "x", ParamUnit::Generic, 0.0, 1.0, 0.0).unwrap()]);
        assert!(matches!(bad, Err(UnitError::BadParameter)));
    }

    #[test]
    fn jump_set_applies_at_next_sync() {
        let mut t = tree();
        let h = t.handle();
        assert_eq!(h.set(0, 1.5).unwrap(), 1.5);
        assert_eq!(t.value(0), 1.0);
        t.sync();
        assert_eq!(t.value(0), 1.5);
        assert_eq!(h.set(0, 9.0).unwrap(), 2.0);
        assert_eq!(h.get(0).unwrap(), 2.0);
    }

    #[test]
    fn read_only_and_unknown_are_refused() {
        let t = tree();
        let h = t.handle();
        assert_eq!(h.set(1, 1.0), Err(UnitError::ReadOnly));
        assert_eq!(h.set(7, 1.0), Err(UnitError::BadParameter));
    }

    #[test]
    fn fill_ramps_linearly_and_lands_on_target() {
        let mut t = tree();
        let h = t.handle();
        h.set_ramped(0, 2.0, 4).unwrap();
        t.sync();
        let mut out = [9.0f32; 6];
        t.fill(0, &mut out);
        // start 1.0, step (2 - 1) / 4 = 0.25
        assert_eq!(out, [1.0, 1.25, 1.5, 1.75, 2.0, 2.0]);
        assert!(!t.is_ramping(0));
    }

    #[test]
    fn advance_splits_a_ramp_across_blocks_exactly() {
        let mut t = tree();
        let h = t.handle();
        h.set_ramped(0, 0.0, 8).unwrap();
        t.sync();
        // step = -1/8 = -0.125
        assert_eq!(t.advance(0, 4), 1.0);
        assert_eq!(t.value(0), 0.5);
        assert_eq!(t.advance(0, 8), 0.5);
        assert_eq!(t.value(0), 0.0);
    }

    #[test]
    fn a_new_target_during_a_ramp_restarts_from_the_current_value() {
        let mut t = tree();
        let h = t.handle();
        h.set_ramped(0, 0.0, 4).unwrap();
        t.sync();
        t.advance(0, 2); // 1.0 -> 0.5
        h.set_ramped(0, 1.0, 2).unwrap();
        t.sync();
        let mut out = [0.0; 3];
        t.fill(0, &mut out);
        assert_eq!(out, [0.5, 0.75, 1.0]);
    }

    #[test]
    fn snapshot_skips_read_only_and_restore_jumps() {
        let mut t = tree();
        let h = t.handle();
        h.set(0, 0.5).unwrap();
        assert_eq!(h.snapshot(), vec![(0, 0.5)]);
        h.restore(&[(0, 1.25)]).unwrap();
        t.sync();
        assert_eq!(t.value(0), 1.25);
        t.reset();
        assert_eq!(t.value(0), 1.0);
        assert_eq!(h.get(0).unwrap(), 1.0);
    }

    #[test]
    fn poll_changes_reports_each_change_once() {
        let t = tree();
        let h = t.handle();
        let mut seen = vec![0u32; 2];
        h.set_ramped(0, 0.5, 3).unwrap();
        let mut got = vec![];
        h.poll_changes(&mut seen, |i, v, r| got.push((i, v, r)));
        assert_eq!(got, vec![(0, 0.5, 3)]);
        got.clear();
        h.poll_changes(&mut seen, |i, v, r| got.push((i, v, r)));
        assert!(got.is_empty());
    }
}
