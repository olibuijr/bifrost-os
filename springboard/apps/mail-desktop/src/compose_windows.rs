//! Compose windows: each new message opens in a window of its own, and several can be open at once.
//!
//! The compose code works on the main window's sheet slot (`Mail::sheet`). So the window that is active (focused) keeps
//! its compose in that slot, and every other compose waits in its [`Slot`]. Every app call ends with [`Composes::sync`]:
//! it copies the sheet's compose back into its slot, opens a window for a compose the window just made (`fresh_compose`),
//! and closes the window of a compose that was sent or saved. Closing a message with changes asks first: the sheet
//! becomes a question (Save This Message, Don't Save, Cancel). A window the shell closed cannot be kept open, so it is
//! opened again with the question. Changed messages are saved to Drafts every [`AUTOSAVE_SECS`].

use crate::state::{Compose, Sheet};
use crate::window::MailWindow;
use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::Ctx;

/// The window size of a compose window, in points.
pub const COMPOSE_W: f32 = 600.0;
pub const COMPOSE_H: f32 = 560.0;
/// The title of a compose window until it has a subject.
pub const COMPOSE_TITLE: &str = "New Message";
/// How often a changed message is saved to Drafts while its window is open.
pub const AUTOSAVE_SECS: f64 = 30.0;

/// One compose window: its SDK window, the compose it holds while it is not active, and its size in pixels as it was
/// last drawn (the size the main window's hit-testing uses while the window is active).
pub struct Slot {
    pub id: WindowId,
    pub compose: Compose,
    pub w: f32,
    pub h: f32,
    /// The text the message had when its window opened: a reply's quote is not a change.
    pub opened: String,
    /// The text last saved to Drafts, and when (seconds of the app's clock).
    pub saved: String,
    pub saved_at: f64,
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

/// The text of a compose: every field's value and every file name.
pub fn fingerprint(compose: &Compose) -> String {
    let mut parts: Vec<&str> = compose.form.fields.iter().filter(|f| !f.choice).map(|f| f.value.as_str()).collect();
    parts.extend(compose.attachments.iter().map(|a| a.name.as_str()));
    parts.join("\u{1f}")
}

/// True when the compose has something in it (a field typed in, or a file), apart from its From address.
fn has_text(compose: &Compose) -> bool {
    compose.form.fields.iter().any(|f| !f.choice && f.label != "From" && !f.value.trim().is_empty()) || !compose.attachments.is_empty()
}

/// True when the message has text and has changed since its window opened.
pub fn changed(compose: &Compose, opened: &str) -> bool {
    has_text(compose) && fingerprint(compose) != opened
}

impl Composes {
    /// True when `id` is one of the compose windows.
    pub fn owns(&self, id: WindowId) -> bool {
        self.slots.iter().any(|slot| slot.id == id)
    }

    fn slot_mut(&mut self, id: WindowId) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|slot| slot.id == id)
    }

    /// Copies the sheet's compose into the active window's slot. Called before the sheet is replaced.
    fn keep_active(&mut self, mail: &MailWindow) {
        if let (Some(id), Some(Sheet::Compose(compose) | Sheet::Question(compose))) = (self.active, &mail.mail.sheet) {
            if let Some(slot) = self.slot_mut(id) {
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
        // a send on its way, or a question, keeps its window active until it is answered
        if self.active.is_some() && (mail.sending || matches!(mail.mail.sheet, Some(Sheet::Question(_)))) {
            return false;
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

    /// Takes the active compose out of the sheet slot, so the main window shows its own content again. Returns false, and
    /// keeps the compose where it is, while a send is on its way or a question is open.
    pub fn deactivate(&mut self, mail: &mut MailWindow) -> bool {
        if self.active.is_none() {
            return true;
        }
        if mail.sending || matches!(mail.mail.sheet, Some(Sheet::Question(_))) {
            return false;
        }
        self.keep_active(mail);
        if matches!(mail.mail.sheet, Some(Sheet::Compose(_))) {
            mail.mail.sheet = None;
        }
        self.active = None;
        let (w, h) = self.main_size;
        mail.set_size(w, h);
        true
    }

    /// After an app call: keeps the active compose with its window, opens a window for a compose the window just made,
    /// asks about a message with changes that was closed, and closes the window of a compose that was sent or saved.
    /// True when windows changed.
    pub fn sync(&mut self, ctx: &Ctx, mail: &mut MailWindow) -> bool {
        let mut changed_windows = false;
        if mail.mail.fresh_compose {
            mail.mail.fresh_compose = false;
            if let Some(Sheet::Compose(compose)) = mail.mail.sheet.clone() {
                match ctx.open_window(WindowSpec::new(COMPOSE_TITLE, COMPOSE_W, COMPOSE_H)) {
                    Ok(id) => {
                        let opened = fingerprint(&compose);
                        self.slots.push(Slot { id, compose, w: COMPOSE_W, h: COMPOSE_H, opened, saved: String::new(), saved_at: 0.0 });
                        self.active = Some(id);
                        changed_windows = true;
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
            match mail.mail.sheet.clone() {
                Some(Sheet::Compose(compose) | Sheet::Question(compose)) => {
                    if let Some(slot) = self.slot_mut(id) {
                        slot.compose = compose;
                    }
                }
                None => {
                    // sent, saved or discarded: the window goes. Closed by Escape or Cancel: the question comes first.
                    let done = std::mem::take(&mut mail.mail.sheet_done);
                    let asks = !done && self.slots.iter().any(|slot| slot.id == id && changed(&slot.compose, &slot.opened));
                    if asks {
                        let compose = self.slots.iter().find(|slot| slot.id == id).map(|slot| slot.compose.clone());
                        mail.mail.sheet = compose.map(Sheet::Question);
                    } else {
                        self.slots.retain(|slot| slot.id != id);
                        let _ = ctx.close_window(id);
                        self.active = None;
                        changed_windows = true;
                    }
                }
                // another sheet took the slot; the compose waits in its window
                Some(_) => self.active = None,
            }
        }
        mail.mail.sheet_done = false;
        changed_windows
    }

    /// Saves each changed message to Drafts, at most once every [`AUTOSAVE_SECS`] and only when its text has changed
    /// since the last save. A new message gets its draft id here.
    pub fn autosave(&mut self, mail: &mut MailWindow, t: f64) {
        for index in 0..self.slots.len() {
            let slot = &self.slots[index];
            let print = fingerprint(&slot.compose);
            if !changed(&slot.compose, &slot.opened) || print == slot.saved || t - slot.saved_at < AUTOSAVE_SECS {
                continue;
            }
            let id = slot.id;
            let mut compose = slot.compose.clone();
            if compose.draft_id.is_empty() {
                compose.draft_id = mail.new_draft_id();
            }
            mail.save_draft(&compose);
            let slot = &mut self.slots[index];
            slot.compose.draft_id = compose.draft_id.clone();
            slot.saved = print;
            slot.saved_at = t;
            if self.active == Some(id) {
                if let Some(Sheet::Compose(c) | Sheet::Question(c)) = &mut mail.mail.sheet {
                    c.draft_id = compose.draft_id.clone();
                }
            }
        }
    }

    /// The shell closed a compose window. A message with changes cannot be dropped silently: its window is opened again
    /// with the question (Save This Message, Don't Save, Cancel). True when windows changed.
    pub fn closed(&mut self, ctx: &Ctx, mail: &mut MailWindow, id: WindowId) -> bool {
        let Some(index) = self.slots.iter().position(|slot| slot.id == id) else { return false };
        let slot = self.slots.remove(index);
        let was_active = self.active == Some(id);
        if was_active {
            self.active = None;
        }
        // the active compose is the sheet's; its text is the one to keep
        let compose = match mail.mail.sheet.clone() {
            Some(Sheet::Compose(c) | Sheet::Question(c)) if was_active => {
                mail.mail.sheet = None;
                c
            }
            _ => slot.compose.clone(),
        };
        if !changed(&compose, &slot.opened) {
            return true;
        }
        let mut compose = compose;
        if compose.draft_id.is_empty() {
            compose.draft_id = mail.new_draft_id();
        }
        match ctx.open_window(WindowSpec::new(COMPOSE_TITLE, COMPOSE_W, COMPOSE_H)) {
            Ok(new_id) => {
                self.slots.push(Slot {
                    id: new_id,
                    compose: compose.clone(),
                    w: COMPOSE_W,
                    h: COMPOSE_H,
                    opened: slot.opened,
                    saved: slot.saved,
                    saved_at: slot.saved_at,
                });
                self.active = Some(new_id);
                mail.set_size(COMPOSE_W as u32, COMPOSE_H as u32);
                mail.mail.sheet = Some(Sheet::Question(compose));
            }
            Err(e) => {
                // no window to ask in: the message is saved to Drafts instead
                mail.mail.status = e.to_string();
                mail.save_draft(&compose);
            }
        }
        true
    }

    /// The compose a window holds, for drawing it while it is not active.
    pub fn compose_of(&self, id: WindowId) -> Option<Compose> {
        self.slots.iter().find(|slot| slot.id == id).map(|slot| slot.compose.clone())
    }

    /// Records the size a compose window was drawn at.
    pub fn resized(&mut self, id: WindowId, w: f32, h: f32) {
        if let Some(slot) = self.slot_mut(id) {
            slot.w = w;
            slot.h = h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_app_has_no_compose_window_and_no_active_one() {
        let composes = Composes::default();
        assert!(composes.slots.is_empty());
        assert_eq!(composes.active, None);
        assert!(!composes.owns(WindowId(3)));
    }

    #[test]
    fn autosave_is_every_thirty_seconds() {
        assert_eq!(AUTOSAVE_SECS, 30.0);
    }
}
