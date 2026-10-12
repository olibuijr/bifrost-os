//! Presets: named parameter states. Factory presets have numbers 0, 1, 2...; user presets get -1, -2, ...

use crate::param::ParamHandle;
use crate::unit::UnitError;

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub number: i32,
    pub name: String,
    pub values: Vec<(u32, f32)>,
}

#[derive(Debug, Default)]
pub struct PresetStore {
    factory: Vec<Preset>,
    user: Vec<Preset>,
    current: Option<i32>,
    next_user: i32,
}

impl PresetStore {
    /// Factory presets must have unique non-negative numbers.
    pub fn new(factory: Vec<Preset>) -> Result<PresetStore, UnitError> {
        for (i, p) in factory.iter().enumerate() {
            if p.number < 0 || factory[..i].iter().any(|q| q.number == p.number) {
                return Err(UnitError::NoPreset);
            }
        }
        Ok(PresetStore { factory, user: Vec::new(), current: None, next_user: -1 })
    }

    /// Factory presets first, then user presets in creation order.
    pub fn presets(&self) -> impl Iterator<Item = &Preset> {
        self.factory.iter().chain(self.user.iter())
    }

    pub fn preset(&self, number: i32) -> Option<&Preset> {
        self.presets().find(|p| p.number == number)
    }

    pub fn current(&self) -> Option<i32> {
        self.current
    }

    /// Stores the state as a new user preset and returns its (negative) number.
    pub fn save_user(&mut self, name: &str, values: Vec<(u32, f32)>) -> i32 {
        let number = self.next_user;
        self.next_user -= 1;
        self.user.push(Preset { number, name: name.to_string(), values });
        self.current = Some(number);
        number
    }

    pub fn delete_user(&mut self, number: i32) -> Result<(), UnitError> {
        let i = self.user.iter().position(|p| p.number == number).ok_or(UnitError::NoPreset)?;
        self.user.remove(i);
        if self.current == Some(number) {
            self.current = None;
        }
        Ok(())
    }

    /// Applies a preset's values to the parameters (jumping).
    pub fn select(&mut self, number: i32, params: &ParamHandle) -> Result<(), UnitError> {
        let p = self.preset(number).ok_or(UnitError::NoPreset)?;
        params.restore(&p.values)?;
        self.current = Some(number);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::{ParamSpec, ParamUnit, ParameterTree};

    fn tree() -> ParameterTree {
        ParameterTree::new(vec![
            ParamSpec::new(0, "A", ParamUnit::Generic, 0.0, 10.0, 1.0).unwrap(),
            ParamSpec::new(1, "B", ParamUnit::Generic, 0.0, 10.0, 2.0).unwrap(),
        ])
        .unwrap()
    }

    fn store() -> PresetStore {
        PresetStore::new(vec![
            Preset { number: 0, name: "Flat".into(), values: vec![(0, 1.0), (1, 2.0)] },
            Preset { number: 1, name: "Loud".into(), values: vec![(0, 8.0), (1, 9.0)] },
        ])
        .unwrap()
    }

    #[test]
    fn duplicate_or_negative_factory_numbers_are_refused() {
        let p = |n| Preset { number: n, name: "x".into(), values: vec![] };
        assert!(PresetStore::new(vec![p(0), p(0)]).is_err());
        assert!(PresetStore::new(vec![p(-1)]).is_err());
    }

    #[test]
    fn select_applies_values() {
        let mut t = tree();
        let h = t.handle();
        let mut s = store();
        s.select(1, &h).unwrap();
        t.sync();
        assert_eq!((t.value(0), t.value(1)), (8.0, 9.0));
        assert_eq!(s.current(), Some(1));
        assert_eq!(s.select(5, &h), Err(UnitError::NoPreset));
    }

    #[test]
    fn user_presets_count_down_and_delete() {
        let t = tree();
        let h = t.handle();
        let mut s = store();
        h.set(0, 4.0).unwrap();
        let n = s.save_user("Mine", h.snapshot());
        assert_eq!(n, -1);
        assert_eq!(s.save_user("Two", h.snapshot()), -2);
        assert_eq!(s.preset(-1).unwrap().values, vec![(0, 4.0), (1, 2.0)]);
        assert_eq!(s.presets().count(), 4);
        s.delete_user(-2).unwrap();
        assert_eq!(s.current(), None);
        assert_eq!(s.delete_user(-2), Err(UnitError::NoPreset));
        assert_eq!(s.delete_user(0), Err(UnitError::NoPreset));
    }
}
