//! Interactive move and resize: pointer grabs that feed the pointer to the `wm` rules.

use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
        GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::protocol::wl_surface::WlSurface},
    utils::{Logical, Point},
};

use crate::{
    handlers::ResizeTarget,
    state::WmState,
    wm::{Edges, Grab, WindowId},
};

/// BTN_LEFT from linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

/// Everything but motion and button passes straight through while a window is grabbed.
macro_rules! forward_the_rest {
    () => {
        fn relative_motion(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
            handle.relative_motion(data, focus, event);
        }
        fn axis(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, details: AxisFrame) { handle.axis(data, details) }
        fn frame(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>) { handle.frame(data) }
        fn gesture_swipe_begin(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GestureSwipeBeginEvent) { handle.gesture_swipe_begin(data, e) }
        fn gesture_swipe_update(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GestureSwipeUpdateEvent) { handle.gesture_swipe_update(data, e) }
        fn gesture_swipe_end(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GestureSwipeEndEvent) { handle.gesture_swipe_end(data, e) }
        fn gesture_pinch_begin(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GesturePinchBeginEvent) { handle.gesture_pinch_begin(data, e) }
        fn gesture_pinch_update(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GesturePinchUpdateEvent) { handle.gesture_pinch_update(data, e) }
        fn gesture_pinch_end(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GesturePinchEndEvent) { handle.gesture_pinch_end(data, e) }
        fn gesture_hold_begin(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GestureHoldBeginEvent) { handle.gesture_hold_begin(data, e) }
        fn gesture_hold_end(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, e: &GestureHoldEndEvent) { handle.gesture_hold_end(data, e) }
        fn start_data(&self) -> &GrabStartData<WmState> { &self.start }
    };
}

pub struct MoveGrab {
    pub start: GrabStartData<WmState>,
    pub id: WindowId,
    pub grab: Grab,
}

impl PointerGrab<WmState> for MoveGrab {
    fn motion(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        handle.motion(data, None, event);
        let Some(r) = data.wm.motion(&self.grab, event.location.x as i32, event.location.y as i32) else { return };
        if let Some(window) = data.window(self.id) {
            data.space.map_element(window, (r.x, r.y), true);
        }
    }

    fn button(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn unset(&mut self, _data: &mut WmState) {}

    forward_the_rest!();
}

pub struct ResizeGrab {
    pub start: GrabStartData<WmState>,
    pub id: WindowId,
    pub grab: Grab,
    pub edges: Edges,
    pub window: Window,
}

impl PointerGrab<WmState> for ResizeGrab {
    fn motion(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        handle.motion(data, None, event);
        let Some(r) = data.wm.motion(&self.grab, event.location.x as i32, event.location.y as i32) else { return };
        if let Some(t) = self.window.user_data().get::<ResizeTarget>() {
            *t.0.borrow_mut() = Some((self.edges, r));
        }
        if let Some(toplevel) = self.window.toplevel() {
            toplevel.with_pending_state(|s| s.size = Some((r.w, r.h).into()));
            toplevel.send_pending_configure();
        }
    }

    fn button(&mut self, data: &mut WmState, handle: &mut PointerInnerHandle<'_, WmState>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    /// The grab ended: the client is told it no longer resizes; its next commit settles the frame.
    fn unset(&mut self, _data: &mut WmState) {
        if let Some(toplevel) = self.window.toplevel() {
            toplevel.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Resizing);
            });
            toplevel.send_pending_configure();
        }
        if let Some(t) = self.window.user_data().get::<ResizeTarget>() {
            t.0.borrow_mut().take();
        }
    }

    forward_the_rest!();
}
