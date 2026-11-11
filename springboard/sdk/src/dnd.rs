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
//!
//! An app with several windows hears which one the pointer is over: `App::window_drag` gets the `Dest` events with the
//! window's id, and the point is in that window's pixels. The shell names the window from 0.10.52 on (the last field
//! of `ToApp::DragDest`); an older shell does not, and the window is `None`.

pub use crate::proto::{dragop, DragItem, DragPhase, DragPreview, Payload};
use crate::proto::ToShell;
use crate::image_metadata::Format;
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

/// The image of the session just begun, at device pixels: `w` by `h` straight RGBA, `proto::rich::MAX_DRAG_IMAGE` a
/// side at most. The shell draws it as it is, centred on the pointer, instead of the small `preview` of [`begin`]
/// scaled up, so draw it at the item's size in points times the display scale (`ctx.ui.s`). An image of another size
/// or with the wrong number of bytes is not sent, and the session keeps its preview.
pub fn set_image(ctx: &Ctx, session: u32, w: u32, h: u32, rgba: Vec<u8>) {
    if crate::proto::rich::drag_image_size_ok(w, h) && rgba.len() == (w as usize) * (h as usize) * 4 {
        ctx.send_to_shell(ToShell::DragImage { session, w, h, rgba });
    }
}

/// Whether a `Dest` event for `window` has a window to go to: the main window, one the app has open, or no window
/// named at all (an older shell). False for a window that closed while the drag was over it.
pub fn window_open(ctx: &Ctx, window: Option<crate::app_windows::WindowId>) -> bool {
    window.is_none_or(|w| w.is_main() || ctx.windows().route(w).is_some())
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

/// The picture format an item carries: from its first type that names a picture format, or, for a file item with no such
/// type, from the extension of its name. `None` for text, links and other data.
///
/// The item's bytes do not travel with the drag yet (the payload is a grant or text), so this tells a window what the
/// picture is before any data is read.
pub fn image_format_of(item: &DragItem) -> Option<Format> {
    let by_type = item.types.iter().find_map(|t| Format::from_type_identifier(t));
    by_type.or_else(|| match &item.payload {
        Payload::File { name, .. } => name.rsplit_once('.').and_then(|(_, ext)| Format::from_extension(ext)),
        _ => None,
    })
}

/// Whether a window that registered `types` (see [`register_types`]) takes a picture of `format`. `public.image` takes
/// every picture format; a picture type takes only its own format.
pub fn accepts_image(registered: &[&str], format: Format) -> bool {
    registered.iter().any(|t| *t == "public.image" || Format::from_type_identifier(t) == Some(format))
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
    fn a_dest_event_goes_to_an_open_window_and_not_to_a_closed_one() {
        use crate::app_windows::{WindowId, WindowSpec};
        let ctx = crate::runtime::test_ctx();
        assert!(window_open(&ctx, None), "an older shell names no window: the app gets the event");
        assert!(window_open(&ctx, Some(WindowId::MAIN)));
        assert!(!window_open(&ctx, Some(WindowId(41))), "a window the app never opened");
        let id = ctx.open_window(WindowSpec::new("Second", 300.0, 200.0)).unwrap();
        assert!(window_open(&ctx, Some(id)));
        ctx.close_window(id).unwrap();
        assert!(!window_open(&ctx, Some(id)), "closed while the drag was over it");
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
