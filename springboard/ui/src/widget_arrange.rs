//! The widgets a user placed on a surface (Today view, desktop or lock screen) and the edit mode that changes them:
//! add a widget of a kind at one of its sizes, remove it, move it to another place in the order, or change its size.
//! Pure model over a `WidgetBundle` (the kinds the apps registered): every change is checked against the kinds and
//! sizes the bundle offers and against the surface's limit, and a refused change leaves the arrangement as it was.

use super::widget_kind::{Family, WidgetBundle};

/// One widget placed on the surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub kind: String,
    pub family: Family,
}

/// Why an arrangement change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrangeError {
    /// No registered kind has this identifier.
    UnknownKind(String),
    /// The kind does not come in this size.
    UnsupportedSize { kind: String, family: Family },
    /// The surface already holds as many widgets as its limit.
    Full { limit: usize },
    /// There is no widget at this index.
    OutOfRange(usize),
}

/// The ordered widgets on one surface, with the most it can hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrangement {
    placements: Vec<Placement>,
    limit: usize,
}

impl Arrangement {
    /// An empty arrangement that holds at most `limit` widgets.
    pub fn new(limit: usize) -> Self {
        Arrangement { placements: Vec::new(), limit }
    }

    /// The placed widgets in order, first on top.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    pub fn len(&self) -> usize {
        self.placements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Adds a widget of `kind` at `family` at the end of the order and returns its index. A kind may be placed more
    /// than once, as each placement can be configured separately.
    pub fn add(&mut self, bundle: &WidgetBundle, kind: &str, family: Family) -> Result<usize, ArrangeError> {
        check(bundle, kind, family)?;
        if self.placements.len() >= self.limit {
            return Err(ArrangeError::Full { limit: self.limit });
        }
        self.placements.push(Placement { kind: kind.to_string(), family });
        Ok(self.placements.len() - 1)
    }

    /// Removes the widget at `index` and returns it. The widgets after it move up one place.
    pub fn remove(&mut self, index: usize) -> Result<Placement, ArrangeError> {
        if index >= self.placements.len() {
            return Err(ArrangeError::OutOfRange(index));
        }
        Ok(self.placements.remove(index))
    }

    /// Moves the widget at `from` so that it ends up at `to` in the order.
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<(), ArrangeError> {
        if from >= self.placements.len() {
            return Err(ArrangeError::OutOfRange(from));
        }
        if to >= self.placements.len() {
            return Err(ArrangeError::OutOfRange(to));
        }
        let p = self.placements.remove(from);
        self.placements.insert(to, p);
        Ok(())
    }

    /// Changes the size of the widget at `index`. The kind must come in the new size.
    pub fn resize(&mut self, bundle: &WidgetBundle, index: usize, family: Family) -> Result<(), ArrangeError> {
        let kind = match self.placements.get(index) {
            Some(p) => p.kind.clone(),
            None => return Err(ArrangeError::OutOfRange(index)),
        };
        check(bundle, &kind, family)?;
        self.placements[index].family = family;
        Ok(())
    }
}

fn check(bundle: &WidgetBundle, kind: &str, family: Family) -> Result<(), ArrangeError> {
    let k = bundle.get(kind).ok_or_else(|| ArrangeError::UnknownKind(kind.to_string()))?;
    if !k.supports(family) {
        return Err(ArrangeError::UnsupportedSize { kind: kind.to_string(), family });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::widget_kind::WidgetKind;
    use super::*;

    fn bundle() -> WidgetBundle {
        let mut b = WidgetBundle::new();
        b.add(WidgetKind::new("weather", "Weather", &[Family::Small, Family::Medium]).unwrap()).unwrap();
        b.add(WidgetKind::new("calendar", "Calendar", &[Family::Large]).unwrap()).unwrap();
        b
    }

    #[test]
    fn add_checks_the_kind_and_the_size() {
        let b = bundle();
        let mut a = Arrangement::new(4);
        assert_eq!(a.add(&b, "weather", Family::Medium), Ok(0));
        assert_eq!(a.add(&b, "calendar", Family::Small), Err(ArrangeError::UnsupportedSize { kind: "calendar".into(), family: Family::Small }));
        assert_eq!(a.add(&b, "clock", Family::Small), Err(ArrangeError::UnknownKind("clock".into())));
        assert_eq!(a.len(), 1, "refused adds change nothing");
    }

    #[test]
    fn the_limit_holds_and_a_kind_can_be_placed_twice() {
        let b = bundle();
        let mut a = Arrangement::new(2);
        assert!(a.add(&b, "weather", Family::Small).is_ok());
        assert!(a.add(&b, "weather", Family::Small).is_ok());
        assert_eq!(a.add(&b, "calendar", Family::Large), Err(ArrangeError::Full { limit: 2 }));
    }

    #[test]
    fn remove_and_move_keep_the_order_consistent() {
        let b = bundle();
        let mut a = Arrangement::new(5);
        a.add(&b, "weather", Family::Small).unwrap();
        a.add(&b, "calendar", Family::Large).unwrap();
        a.add(&b, "weather", Family::Medium).unwrap();
        a.move_to(2, 0).unwrap();
        assert_eq!(a.placements()[0].family, Family::Medium);
        assert_eq!(a.placements()[1].kind, "weather");
        let gone = a.remove(1).unwrap();
        assert_eq!(gone.family, Family::Small);
        assert_eq!(a.len(), 2);
        assert_eq!(a.remove(5), Err(ArrangeError::OutOfRange(5)));
        assert_eq!(a.move_to(0, 9), Err(ArrangeError::OutOfRange(9)));
    }

    #[test]
    fn resize_needs_a_size_the_kind_supports() {
        let b = bundle();
        let mut a = Arrangement::new(3);
        a.add(&b, "weather", Family::Small).unwrap();
        assert!(a.resize(&b, 0, Family::Medium).is_ok());
        assert_eq!(a.placements()[0].family, Family::Medium);
        assert!(a.resize(&b, 0, Family::Large).is_err());
        assert_eq!(a.placements()[0].family, Family::Medium);
        assert_eq!(a.resize(&b, 3, Family::Small), Err(ArrangeError::OutOfRange(3)));
    }
}
