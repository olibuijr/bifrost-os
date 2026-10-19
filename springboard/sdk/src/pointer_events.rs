//! Pointer events for apps: the scroll wheel, the pointer moving over the window, and the two-finger magnify and rotate
//! gestures, with the tracking areas that turn hover into enter, exit and move events.
//!
//! An app opts in by returning the bits it takes from [`App::pointer_interest`](crate::App::pointer_interest) and
//! handles the events in [`App::pointer`](crate::App::pointer). The runtime tells the shell once. The shell then sends
//! the wheel as an event (and no longer emulates a drag for it), the hover as the pointer moves over the window and
//! `HoverExit` when it leaves, and the gestures. An app that does not opt in keeps the touch stream as before.
//!
//! Locations are surface pixels, like touches; [`PointerEvent::to_points`] converts. A [`PointerRouter`] keeps the
//! tracking areas of the app and turns the events into [`Delivered`] values:
//!
//! ```ignore
//! impl App for Canvas {
//!     fn pointer_interest(&self) -> u32 { pointer_events::WHEEL | pointer_events::HOVER }
//!     fn pointer(&mut self, ctx: &Ctx, ev: &PointerEvent) -> bool {
//!         let mut redraw = false;
//!         for d in self.router.route(&ev.to_points(ctx.cfg.scale)) {
//!             match d {
//!                 Delivered::Scroll { event, .. } => { self.offset -= event.scrolling_delta_y; redraw = true }
//!                 Delivered::Tracking(tracking::Event::Entered(id)) => { self.hot = Some(id); redraw = true }
//!                 Delivered::Tracking(tracking::Event::Exited(_)) => { self.hot = None; redraw = true }
//!                 _ => {}
//!             }
//!         }
//!         redraw
//!     }
//! }
//! ```

pub use bifrost_ui::pointer_event::{
    from_gesture, phase_from_wire, phase_to_wire, Delivered, PointerEvent, PointerRouter, ALL, GESTURES, HOVER, WHEEL,
};
pub use bifrost_ui::tracking::{self, Area, Event as TrackingEvent};
pub use bifrost_ui::wheel::{Phase, ScrollEvent};
