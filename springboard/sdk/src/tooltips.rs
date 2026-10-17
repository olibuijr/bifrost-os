//! Tooltips for SDK apps: a pointer that rests on a control for the delay shows that control's tooltip.
//!
//! The state machine is `bifrost_ui::tooltip::Tooltips` and is re-exported here. The app passes the owner under the
//! pointer and the time in seconds on each pointer event and tick; `visible` says which owner's tooltip shows. The
//! tooltip panel uses `Material::ToolTip` from `widgets`. Drawing the text on hover still needs the pointer wiring in
//! the shell, which is checked on the lab machine.

pub use bifrost_ui::tooltip::{Timing, Tooltips};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resting_pointer_shows_the_tooltip_after_the_delay() {
        let mut t = Tooltips::new(Timing::default());
        t.enter(7, 0.0);
        assert_eq!(t.tick(0.5), None);
        assert_eq!(t.tick(1.5), Some(7));
        assert_eq!(t.visible(), Some(7));
    }

    #[test]
    fn leaving_the_control_hides_its_tooltip() {
        let mut t = Tooltips::new(Timing::default());
        t.enter(7, 0.0);
        t.tick(2.0);
        t.leave(7, 2.1);
        assert_eq!(t.visible(), None);
    }
}
