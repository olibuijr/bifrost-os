//! Settings panes an app declares in `manifest.toml` as `[[settings_pane]]` tables (`id`, `title`, optional
//! `symbol`). The shell lists them under the app in Settings. When the person picks one, the shell sends the event
//! `ae.settings.openPane` with the pane id as the `id` parameter over the app's control socket. An app that declares
//! panes installs a handler for it with `install_open_handler`, which opens the pane and answers the request.
//!
//! ```ignore
//! settings_panes::install_open_handler(&mut handlers, |id| { show_pane(id); Ok(()) });
//! ```

use crate::app_events::{AppEvent, AppEventHandlers};
use crate::control::{RpcError, INVALID_PARAMS};
use bifrost_core::json::Json;

/// The event class of the open request.
pub const CLASS: &str = "settings";
/// The event id of the open request.
pub const OPEN_PANE: &str = "openPane";
/// The parameter that carries the pane id.
pub const PARAM_ID: &str = "id";

/// The pane id an open request carries, or None when the parameter is missing or empty.
pub fn pane_id(event: &AppEvent) -> Option<String> {
    let id = event.params.get(PARAM_ID)?.as_str()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Installs the handler for the open request. `open` gets the pane id; an error from it is sent back to the shell.
/// The reply on success is an empty record.
pub fn install_open_handler(handlers: &mut AppEventHandlers, mut open: impl FnMut(&str) -> Result<(), RpcError> + 'static) {
    handlers.install(CLASS, OPEN_PANE, move |event| {
        let id = pane_id(event).ok_or_else(|| RpcError::new(INVALID_PARAMS, "openPane needs an id"))?;
        open(&id)?;
        Ok(Json::Obj(Vec::new()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::METHOD_NOT_FOUND;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn event(id: &str) -> AppEvent {
        AppEvent::new(CLASS, OPEN_PANE, Json::Obj(vec![(PARAM_ID.into(), Json::Str(id.into()))]))
    }

    #[test]
    fn the_pane_id_is_read_from_the_params() {
        assert_eq!(pane_id(&event("accounts")), Some("accounts".to_string()));
        assert_eq!(pane_id(&event("")), None);
        let bare = AppEvent::new(CLASS, OPEN_PANE, Json::Obj(Vec::new()));
        assert_eq!(pane_id(&bare), None);
    }

    #[test]
    fn the_method_is_the_shell_event_name() {
        assert_eq!(event("x").method(), "ae.settings.openPane");
    }

    #[test]
    fn the_handler_opens_the_named_pane() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        let mut handlers = AppEventHandlers::new();
        install_open_handler(&mut handlers, move |id| {
            seen.borrow_mut().push(id.to_string());
            Ok(())
        });
        assert!(handlers.process(&event("signatures")).is_ok());
        assert_eq!(*opened.borrow(), vec!["signatures".to_string()]);
    }

    #[test]
    fn a_request_without_an_id_is_refused_before_opening() {
        let opened = Rc::new(RefCell::new(0));
        let count = opened.clone();
        let mut handlers = AppEventHandlers::new();
        install_open_handler(&mut handlers, move |_| {
            *count.borrow_mut() += 1;
            Ok(())
        });
        let err = handlers.process(&AppEvent::new(CLASS, OPEN_PANE, Json::Obj(Vec::new()))).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert_eq!(*opened.borrow(), 0);
    }

    #[test]
    fn an_app_without_the_handler_answers_not_found() {
        let mut handlers = AppEventHandlers::new();
        let err = handlers.process(&event("accounts")).unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND);
    }
}
