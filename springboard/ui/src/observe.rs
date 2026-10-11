//! Observable values (`UserDefaults` + KVO / `@Published`): a value that tells its subscribers when it changes,
//! and only when it really changes. Single-threaded and re-entrancy safe (a subscriber may unsubscribe itself).

pub struct Observable<T> {
    value: T,
    subs: Vec<(u64, Box<dyn FnMut(&T, &T)>)>,
    next: u64,
}

impl<T: PartialEq + Clone> Observable<T> {
    pub fn new(value: T) -> Self {
        Observable { value, subs: Vec::new(), next: 1 }
    }
    pub fn get(&self) -> &T {
        &self.value
    }
    /// Subscribes `f(old, new)`; returns a token for `unsubscribe`.
    pub fn subscribe(&mut self, f: impl FnMut(&T, &T) + 'static) -> u64 {
        let id = self.next;
        self.next += 1;
        self.subs.push((id, Box::new(f)));
        id
    }
    pub fn unsubscribe(&mut self, token: u64) {
        self.subs.retain(|s| s.0 != token);
    }
    /// Sets the value; returns whether it changed (equal values notify nobody).
    pub fn set(&mut self, new: T) -> bool {
        if self.value == new {
            return false;
        }
        let old = std::mem::replace(&mut self.value, new);
        for (_, f) in self.subs.iter_mut() {
            f(&old, &self.value);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[test]
    fn notifies_with_old_and_new_only_on_change() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let mut o = Observable::new(1);
        o.subscribe(move |a, b| l.borrow_mut().push((*a, *b)));
        assert!(!o.set(1));
        assert!(o.set(2));
        assert!(o.set(5));
        assert_eq!(*log.borrow(), vec![(1, 2), (2, 5)]);
    }
    #[test]
    fn unsubscribe_stops_notifications() {
        let n = Rc::new(RefCell::new(0));
        let c = n.clone();
        let mut o = Observable::new("a".to_string());
        let t = o.subscribe(move |_, _| *c.borrow_mut() += 1);
        o.set("b".into());
        o.unsubscribe(t);
        o.set("c".into());
        assert_eq!((*n.borrow(), o.get().as_str()), (1, "c"));
    }
}
