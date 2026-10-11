//! Typed drag and drop (`NativeDraggingSource`, `NativeDraggingDestination`, `registerForDraggedTypes`).
//!
//! Destination: [`register_types`] with the UTTypes the window takes (`"public.image"` takes every image type). While a
//! session from another window (or this one) is over the window, `App::drag` gets [`Event::Dest`] with `Entered`, `Updated`
//! and `Exited`; answer `Entered` and `Updated` with [`reply`] (one of [`dragop`]`::COPY`, `LINK`, `MOVE`, or 0 to refuse).
//! On `Perform`, the files of the items have just arrived as `App::document` events with `request == session` (read-only
//! grants made for this app); the items in `Perform` carry those grant ids. Finish with [`performed`].
//!
//! Source: [`begin`] with the items, the operations allowed and a [`DragPreview`]. The shell tracks the pointer and ends the
//! session with [`Event::Ended`] (`op` 0 when nobody took the drop). Coordinates are window pixels, the space of `Touch`.

pub use crate::proto::{dragop, DragItem, DragPhase, DragPreview, Payload};
use crate::proto::ToShell;
use crate::Ctx;

/// What `App::drag` is called with.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `draggingEntered:` / `draggingUpdated:` / `draggingExited:` / `performDragOperation:`. `items` are set for `Entered`
    /// and `Perform`; `allowed` is the source's operation mask.
    Dest { session: u32, phase: DragPhase, x: f32, y: f32, allowed: u8, items: Vec<DragItem> },
    /// `draggingSession(_:endedAt:operation:)` for a session this app began.
    Ended { session: u32, op: u8 },
}

/// `registerForDraggedTypes`: the UTTypes this window accepts; an empty list stops accepting drags.
pub fn register_types(ctx: &Ctx, types: &[&str]) {
    ctx.send_to_shell(ToShell::DragRegister { types: types.iter().map(|t| t.to_string()).collect() });
}

/// `beginDraggingSession`: starts a session and returns its id. `allowed` is a mask of `dragop` bits. Call it from the
/// handler of the mouse-down that moved far enough to drag. The shell refuses (an immediate `Ended { op: 0 }`) when another
/// session runs, the app is not in front, or the items are empty.
pub fn begin(ctx: &Ctx, allowed: u8, items: Vec<DragItem>, preview: DragPreview) -> u32 {
    let session = ctx.next_request() | crate::share::INBOUND;
    ctx.send_to_shell(ToShell::DragBegin { session, allowed, items, preview });
    session
}

/// The operation this window will perform at the current point (a single `dragop` bit; 0 refuses).
pub fn reply(ctx: &Ctx, session: u32, op: u8) {
    ctx.send_to_shell(ToShell::DragReply { session, op });
}

/// The result of `performDragOperation`.
pub fn performed(ctx: &Ctx, session: u32, accepted: bool) {
    ctx.send_to_shell(ToShell::DragPerformed { session, accepted });
}

/// The source gives the session up (Escape).
pub fn cancel(ctx: &Ctx, session: u32) {
    ctx.send_to_shell(ToShell::DragCancel { session });
}

/// A drag image from straight RGBA pixels of any size: sampled down (nearest) to fit `proto::rich::MAX_PREVIEW` pixels per
/// side, aspect kept. A buffer that does not match `w * h * 4` gives an empty preview (no image).
pub fn preview_from_rgba(w: u32, h: u32, rgba: &[u8]) -> DragPreview {
    let max = crate::proto::rich::MAX_PREVIEW;
    if w == 0 || h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
        return DragPreview::default();
    }
    let k = (w.max(h) as f32 / max as f32).max(1.0);
    let (tw, th) = (((w as f32 / k).round() as u32).clamp(1, max), ((h as f32 / k).round() as u32).clamp(1, max));
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            let (sx, sy) = (((x as f32 + 0.5) * k) as u32, ((y as f32 + 0.5) * k) as u32);
            let i = ((sy.min(h - 1) * w + sx.min(w - 1)) * 4) as usize;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    DragPreview { w: tw, h: th, rgba: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_scaled_to_fit_and_stay_valid() {
        let big = vec![200u8; 100 * 50 * 4];
        let p = preview_from_rgba(100, 50, &big);
        assert!(p.valid());
        assert_eq!((p.w, p.h), (32, 16));
        let small = preview_from_rgba(2, 2, &[7; 16]);
        assert_eq!((small.w, small.h, small.rgba.len()), (2, 2, 16));
    }

    #[test]
    fn bad_buffers_give_no_preview() {
        assert_eq!(preview_from_rgba(4, 4, &[0; 10]), DragPreview::default());
        assert_eq!(preview_from_rgba(0, 4, &[]), DragPreview::default());
    }

    #[test]
    fn scaling_samples_the_right_pixels() {
        // a 64x1 strip where pixel i has red = i: the 32 samples are the even-ish pixels in order
        let strip: Vec<u8> = (0..64u32).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        let p = preview_from_rgba(64, 1, &strip);
        assert_eq!((p.w, p.h), (32, 1));
        let reds: Vec<u8> = p.rgba.chunks(4).map(|c| c[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(reds[0], 1);
    }
}
