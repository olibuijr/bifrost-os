//! Sharing (`NativeSharingServicePicker`): ask the shell to show the share menu for some items under a rect, and receive items
//! other apps share with you.
//!
//! Sending: [`show_picker`] with text, URLs and files (a file is named by the document grant the app already holds). The
//! shell lists the installed apps whose manifest `handles` (UTTypes) and `schemes` accept every item, plus Copy, and answers
//! `App::share_done` with the chosen app id (`"copy"` for Copy) or `None` when dismissed.
//!
//! Receiving: declare what you open in the manifest (`handles = ["public.png"]`, `schemes = ["https"]`). A shared URL arrives
//! as `Input::Open(url)`; shared text as `Input::Open` with a `data:` URL, which [`text_from_url`] decodes; shared files as
//! `App::document` events (`DocumentEvent::Opened` / `OpenedMany`) whose `request` has [`INBOUND`] set, each with a read-only
//! grant made for this app.

pub use crate::proto::Payload;
pub use springboard::shell::sharing::{text_from_url, COPY, INBOUND};

use crate::proto::{ToShell, UiRect};
use crate::Ctx;

/// `NativeSharingServicePicker.show(relativeTo:of:preferredEdge:)` for `items`, anchored under `anchor` (window points). The
/// answer arrives in `App::share_done` with the returned request id. At most 8 items.
pub fn show_picker(ctx: &Ctx, items: &[Payload], anchor: UiRect) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::Share { request, items: items.to_vec(), anchor });
    request
}

/// True when a document `request` was not asked for by this app: a file somebody shared or dropped on it.
pub fn is_inbound(request: u32) -> bool {
    request & INBOUND != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_requests_are_recognised() {
        assert!(is_inbound(INBOUND | 5));
        assert!(!is_inbound(5));
    }
}
