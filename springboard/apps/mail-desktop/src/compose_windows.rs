//! Compose windows: each new message opens in a window of its own, and several can be open at once.
//!
//! The compose code works on the main window's sheet slot (`Mail::sheet`). So the window that is active (focused) keeps
//! its compose in that slot, and every other compose waits in its [`Slot`]. Every app call ends with [`Composes::sync`]:
//! it copies the sheet's compose back into its slot, opens a window for a compose the window just made (`fresh_compose`),
//! and closes the window of a compose that was sent or cancelled. The main window draws without the sheet while a compose
//! window is active, so a message is never drawn in two places.

use crate::state::{Compose, Sheet};
use crate::window::MailWindow;
use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::Ctx;

/// The window size of a compose window, in points.
pub const COMPOSE_W: f32 = 600.0;
pub const COMPOSE_H: f32 = 560.0;
/// The title of a compose window until it has a subject.
pub const COMPOSE_TITLE: &str = "New Message";

/// One compose window: its SDK window, the compose it holds while it is not active, and its size in pixels as it was
/// last drawn (the size the main window's hit-testing uses while the window is active).
pub struct Slot {
    pub id: WindowId,
    pub compose: Compose,
    pub w: f32,
    pub h: f32,
}

/// The compose windows of the app, and which one owns the sheet slot.
#[derive(Default)]
pub struct Composes {
    pub slots: Vec<Slot>,
    /// The compose window whose compose is in the sheet slot, if any.
    pub active: Option<WindowId>,
    /// The main window's size in points, restored when no compose window is active.
    pub main_size: (u32, u32),
}

impl Composes {
    /// True when `id` is one of the compose windows.
    pub fn owns(&self, id: WindowId) -> bool {
        self.slots.iter().any(|slot| slot.id == id)
    }

    /// Copies the sheet's compose into the active window's slot. Called before the sheet is replaced.
    fn keep_active(&mut self, mail: &MailWindow) {
        if let (Some(id), Some(Sheet::Compose(compose))) = (self.active, &mail.mail.sheet) {
            if let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) {
                slot.compose = compose.clone();
            }
        }
    }

    /// Makes `id` the active compose window: its compose takes the sheet slot and its size takes the main window's
    /// place for hit-testing. Returns false when the sheet slot is taken by a sheet that is not a compose.
    pub fn activate(&mut self, mail: &mut MailWindow, id: WindowId) -> bool {
        if self.active == Some(id) {
            return true;
        }
        let free = match &mail.mail.sheet {
            None => true,
            Some(Sheet::Compose(_)) => self.active.is_some(),
            Some(_) => false,
        };
        if !free {
            return false;
        }
        self.keep_active(mail);
        let Some(slot) = self.slots.iter().find(|slot| slot.id == id) else { return false };
        mail.mail.sheet = Some(Sheet::Compose(slot.compose.clone()));
        if slot.w >= 1.0 && slot.h >= 1.0 {
            mail.set_size(slot.w as u32, slot.h as u32);
        }
        self.active = Some(id);
        true
    }

    /// Takes the active compose out of the sheet slot, so the main window shows its own content again.
    pub fn deactivate(&mut self, mail: &mut MailWindow) {
        if self.active.is_none() {
            return;
        }
        self.keep_active(mail);
        if matches!(mail.mail.sheet, Some(Sheet::Compose(_))) {
            mail.mail.sheet = None;
        }
        self.active = None;
        let (w, h) = self.main_size;
        mail.set_size(w, h);
    }

    /// After an app call: keeps the active compose with its window, opens a window for a compose the window just made,
    /// and closes the window of a compose that was sent or cancelled. True when windows changed.
    pub fn sync(&mut self, ctx: &Ctx, mail: &mut MailWindow) -> bool {
        let mut changed = false;
        if mail.mail.fresh_compose {
            mail.mail.fresh_compose = false;
            if let Some(Sheet::Compose(compose)) = mail.mail.sheet.clone() {
                match ctx.open_window(WindowSpec::new(COMPOSE_TITLE, COMPOSE_W, COMPOSE_H)) {
                    Ok(id) => {
                        self.slots.push(Slot { id, compose, w: COMPOSE_W, h: COMPOSE_H });
                        self.active = Some(id);
                        changed = true;
                    }
                    Err(e) => {
                        // the draft stays in the sheet slot, as it did before compose windows
                        mail.mail.status = e.to_string();
                        self.active = None;
                    }
                }
            }
        }
        if let Some(id) = self.active {
            match &mail.mail.sheet {
                Some(Sheet::Compose(compose)) => {
                    let compose = compose.clone();
                    if let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) {
                        slot.compose = compose;
                    }
                }
                None => {
                    // sent or cancelled: the window goes
                    self.slots.retain(|slot| slot.id != id);
                    let _ = ctx.close_window(id);
                    self.active = None;
                    changed = true;
                }
                // another sheet took the slot; the compose waits in its window
                Some(_) => self.active = None,
            }
        }
        changed
    }

    /// The shell closed a compose window: the draft is dropped with it.
    pub fn closed(&mut self, mail: &mut MailWindow, id: WindowId) -> bool {
        let before = self.slots.len();
        self.slots.retain(|slot| slot.id != id);
        if self.active == Some(id) {
            self.active = None;
            if matches!(mail.mail.sheet, Some(Sheet::Compose(_))) {
                mail.mail.sheet = None;
            }
        }
        before != self.slots.len()
    }

    /// The compose a window holds, for drawing it while it is not active.
    pub fn compose_of(&self, id: WindowId) -> Option<Compose> {
        self.slots.iter().find(|slot| slot.id == id).map(|slot| slot.compose.clone())
    }

    /// Records the size a compose window was drawn at.
    pub fn resized(&mut self, id: WindowId, w: f32, h: f32) {
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) {
            slot.w = w;
            slot.h = h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_window_is_not_a_compose_window() {
        let composes = Composes::default();
        assert!(!composes.owns(WindowId(3)));
        assert!(composes.compose_of(WindowId(3)).is_none());
    }

    #[test]
    fn a_new_app_has_no_compose_window_and_no_active_one() {
        let composes = Composes::default();
        assert!(composes.slots.is_empty());
        assert_eq!(composes.active, None);
    }
}
