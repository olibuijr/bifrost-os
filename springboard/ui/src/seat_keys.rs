//! One logical seat keyboard over several physical key devices.
//!
//! A USB keyboard often shows up as more than one evdev node (a boot interface and an NKRO or extra interface), and
//! some report every key on two of them. Read as separate keyboards, each stroke would then be typed twice. A
//! `SeatKeys` merges the devices: it keeps the set of devices holding each key code, delivers a press only when the
//! first device takes the key down and a release only when the last one lets go, and releases a vanished device's
//! keys. Modifiers are key codes like any other, so a Shift held on two interfaces stays held until both let go.
//! Auto-repeat (evdev value 2) is not a press and never comes through here.

use std::borrow::Borrow;
use std::collections::HashMap;

/// The pressed keys of a seat, per key code the devices holding it. `D` names a device (a path, an fd, an id).
#[derive(Clone, Debug)]
pub struct SeatKeys<D> {
    held: HashMap<u32, Vec<D>>,
}

impl<D> Default for SeatKeys<D> {
    fn default() -> Self {
        SeatKeys { held: HashMap::new() }
    }
}

impl<D: Eq> SeatKeys<D> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `device` reports a press (`down`) or release of `code`: whether the seat's keyboard changes, so the event is
    /// delivered. A press of a key some device already holds and a release while another device still holds it are
    /// swallowed; so is a release of a key nothing holds (a press from before the device was opened).
    pub fn feed<Q>(&mut self, device: &Q, code: u32, down: bool) -> bool
    where
        D: Borrow<Q>,
        Q: ?Sized + Eq + ToOwned<Owned = D>,
    {
        if down {
            let holders = self.held.entry(code).or_default();
            let first = holders.is_empty();
            if !holders.iter().any(|d| Borrow::<Q>::borrow(d) == device) {
                holders.push(Q::to_owned(device));
            }
            first
        } else {
            let Some(holders) = self.held.get_mut(&code) else { return false };
            let Some(i) = holders.iter().position(|d| Borrow::<Q>::borrow(d) == device) else { return false };
            holders.swap_remove(i);
            if holders.is_empty() {
                self.held.remove(&code);
                true
            } else {
                false
            }
        }
    }

    /// `device` went away: the codes it was the last holder of, which the seat now releases (sorted).
    pub fn remove_device<Q>(&mut self, device: &Q) -> Vec<u32>
    where
        D: Borrow<Q>,
        Q: ?Sized + Eq,
    {
        let mut released = Vec::new();
        self.held.retain(|&code, holders| {
            holders.retain(|d| Borrow::<Q>::borrow(d) != device);
            if holders.is_empty() {
                released.push(code);
                false
            } else {
                true
            }
        });
        released.sort_unstable();
        released
    }

    /// The seat holds `code` down on some device.
    pub fn is_down(&self, code: u32) -> bool {
        self.held.contains_key(&code)
    }

    /// The codes the seat holds down (sorted).
    pub fn pressed(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.held.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Forgets every pressed key (a session pause, where the keys' releases go elsewhere).
    pub fn clear(&mut self) {
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 30;
    const S: u32 = 31;
    const SHIFT: u32 = 42;

    #[test]
    fn one_key_on_two_interfaces_is_one_press_and_one_release() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(k.feed(&1, A, true));
        assert!(!k.feed(&2, A, true));
        assert!(!k.feed(&1, A, false));
        assert!(k.is_down(A));
        assert!(k.feed(&2, A, false));
        assert!(!k.is_down(A));
    }

    #[test]
    fn the_release_order_does_not_matter() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(k.feed(&1, A, true));
        assert!(!k.feed(&2, A, true));
        assert!(!k.feed(&2, A, false));
        assert!(k.feed(&1, A, false));
    }

    #[test]
    fn rollover_across_devices_keeps_both_keys() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(k.feed(&1, A, true));
        assert!(k.feed(&2, S, true));
        assert_eq!(k.pressed(), vec![A, S]);
        assert!(k.feed(&1, A, false));
        assert!(k.feed(&2, S, false));
        assert!(k.pressed().is_empty());
    }

    #[test]
    fn a_repeated_press_from_one_device_is_not_delivered_again() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(k.feed(&1, A, true));
        assert!(!k.feed(&1, A, true));
        assert!(k.feed(&1, A, false));
    }

    #[test]
    fn a_release_nothing_holds_is_swallowed() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(!k.feed(&1, A, false));
        assert!(k.feed(&1, A, true));
        assert!(!k.feed(&2, A, false));
        assert!(k.is_down(A));
    }

    #[test]
    fn a_modifier_held_on_two_devices_stays_held_until_both_let_go() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        assert!(k.feed(&1, SHIFT, true));
        assert!(!k.feed(&2, SHIFT, true));
        assert!(k.feed(&1, A, true));
        assert!(!k.feed(&2, SHIFT, false));
        assert!(k.is_down(SHIFT));
        assert!(k.feed(&1, A, false));
        assert!(k.feed(&1, SHIFT, false));
    }

    #[test]
    fn a_vanished_device_releases_only_the_keys_it_alone_held() {
        let mut k: SeatKeys<i32> = SeatKeys::new();
        k.feed(&1, A, true);
        k.feed(&2, A, true);
        k.feed(&2, S, true);
        k.feed(&2, SHIFT, true);
        assert_eq!(k.remove_device(&2), vec![S, SHIFT]);
        assert_eq!(k.pressed(), vec![A]);
        assert!(k.feed(&1, A, false));
    }
}
