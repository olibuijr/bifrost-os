//! Deliver engine events and native embedder controls.
use super::*;

impl Engine {

    pub(super) fn notes(&mut self) {
        // controls the page withdrew: forget them (Drop answers them) and tell the app to close its UI
        for cid in std::mem::take(&mut *self.delegate.hidden.borrow_mut()) {
            let tokens: Vec<u32> = self.controls.iter().filter(|(_, (_, id, _))| *id == cid).map(|(t, _)| *t).collect();
            for t in tokens {
                if let Some((view, _, _)) = self.controls.remove(&t) {
                    say(&format!("control-hide {view} {t}"));
                }
            }
        }
        let notes = std::mem::take(&mut *self.delegate.notes.borrow_mut());
        for note in notes {
            match note {
                Note::Frame(_) => {
                    // Any new frame repaints: after `show`, WebRender gets the new root display list
                    // asynchronously (servo paint/painter.rs set_webview_hidden), and the frame that carries
                    // it is not attributed to the shown view. Hidden views do not animate, so this is cheap.
                    if self.visible.is_some() {
                        self.dirty = true;
                    }
                }
                Note::Status(view, text) => {
                    if let Some(id) = self.id_of(&view) { say(&format!("status {id} {}", escape_line(&text))); }
                }
                Note::Unload(view, request) => {
                    if let Some(id) = self.id_of(&view) { self.show_unload(id, request); }
                }
                Note::Clipboard(text) => say(&format!("clipboard {}", devtools::esc(&text))),
                Note::Favicon(view) => {
                    if let (Some(id), Some(pixels)) = (self.id_of(&view), favicon::pixels(&view)) {
                        say(&format!("favicon {id} {pixels}"));
                    }
                }
                Note::Title(w, t) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("title {id} {t}"));
                    }
                }
                Note::Url(w, u) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("url {id} {u}"));
                    }
                }
                Note::Load(w, s) => {
                    if let Some(id) = self.id_of(&w) {
                        if s == "started" { self.retire_prompts(Some(id)); }
                        say(&format!("load {id} {s}"));
                    }
                }
                Note::History(w) => {
                    if let Some(id) = self.id_of(&w) {
                        say(&format!("history {id} {} {}", w.can_go_back() as u8, w.can_go_forward() as u8));
                    }
                }
                Note::Created(parent, request) => {
                    if self.views.len() >= MAX_VIEWS {
                        continue; // dropping the request denies it through Servo's responder
                    }
                    let Some(parent_id) = self.id_of(&parent) else { continue };
                    // engine-chosen ids count down from the top so they never meet the app's
                    let id = (u32::MAX / 2).saturating_add(self.next_id);
                    self.next_id = self.next_id.saturating_add(1);
                    let w = self.build(request.builder(self.rc.clone()));
                    w.resize(self.size);
                    w.hide();
                    let url = w.url().map(|u| u.to_string()).unwrap_or_default();
                    self.views.insert(id, w);
                    say(&format!("created {id} {parent_id} {url}"));
                }
                Note::Closed(w) => {
                    if let Some(id) = self.id_of(&w) {
                        self.retire_prompts(Some(id));
                        say(&format!("closed {id}"));
                    }
                }
                Note::Console(w, level, text) => {
                    if let Some(id) = self.id_of(&w) {
                        if let Some(rest) = text.strip_prefix(blocker::MESSAGE_PREFIX) {
                            // a page message for an app handler: never console output, and sent whatever `devtools` says
                            if let Some(line) = blocker::script_message_line(id, rest) {
                                say(&line);
                            }
                        } else if self.devtools {
                            say(&format!("console {id} {level} - 0 {}", escape_line(&text)));
                        }
                    }
                }
                Note::Nav(w, request) => {
                    let Some(id) = self.id_of(&w) else { request.deny(); continue };
                    let token = self.next_token;
                    self.next_token = self.next_token.wrapping_add(1).max(1);
                    let url = request.url.to_string();
                    blocker::hold(token, id, request);
                    say(&format!("navreq {id} {token} {url}"));
                }
                Note::Redirect(w, url) => {
                    w.load(url);
                    self.dirty = true;
                }
                Note::Control(w, control) => {
                    let Some(view) = self.id_of(&w) else { continue };
                    self.show_control(view, control);
                }
                Note::Eval(id, text) => say(&format!("evalresult {id} {}", escape_line(&text))),
                Note::Crashed(w, reason) => {
                    if let Some(id) = self.id_of(&w) {
                        self.retire_prompts(Some(id));
                        say(&format!("crashed {id} {reason}"));
                    }
                }
            }
        }
    }

    /// Forwards a script dialog, context menu or <select> popup to the app; anything else is dropped (answered by Drop).
    pub(super) fn show_control(&mut self, view: u32, control: EmbedderControl) {
        // dropping the oldest answers it with the default (Servo's Drop impls), so a flood cannot pile up
        if self.controls.len() >= MAX_CONTROLS {
            if let Some(k) = self.controls.keys().min().copied() {
                if let Some((view, _, _)) = self.controls.remove(&k) {
                    say(&format!("control-hide {view} {k}"));
                }
            }
        }
        let cid = control.id();
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let esc = |s: &str| escape_line(s);
        match control {
            EmbedderControl::SimpleDialog(d) => {
                let (kind, default) = match &d {
                    SimpleDialog::Alert(_) => ("alert", String::new()),
                    SimpleDialog::Confirm(_) => ("confirm", String::new()),
                    SimpleDialog::Prompt(p) => ("prompt", p.current_value().to_string()),
                };
                say(&format!("dialog {view} {token} {kind} {}\u{1}{}", esc(&default), esc(d.message())));
                self.controls.insert(token, (view, cid, Pending::Dialog(d)));
            }
            EmbedderControl::ContextMenu(m) => {
                let r = m.position();
                let mut actions = Vec::new();
                let mut recs = Vec::new();
                for it in m.items() {
                    match it {
                        ContextMenuItem::Separator => recs.push("0|4|".to_string()),
                        ContextMenuItem::Item { label, action, enabled } => {
                            recs.push(format!("{}|{}|{}", actions.len(), if *enabled { 1 } else { 0 }, esc(label)));
                            actions.push(*action);
                        }
                    }
                }
                say(&format!("menu {view} {token} context {} {} {} {} {}", r.min.x, r.min.y, r.width(), r.height(), recs.join("\u{1}")));
                self.controls.insert(token, (view, cid, Pending::Menu(m, actions)));
            }
            EmbedderControl::SelectElement(sel) => {
                let r = sel.position();
                let chosen = sel.selected_options();
                let mut recs = Vec::new();
                for o in sel.options() {
                    match o {
                        SelectElementOptionOrOptgroup::Option(o) => {
                            let f = if o.is_disabled { 0 } else { 1 } | if chosen.contains(&o.id) { 2 } else { 0 };
                            recs.push(format!("{}|{}|{}", o.id, f, esc(&o.label)));
                        }
                        SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                            recs.push(format!("0|8|{}", esc(label)));
                            for o in options {
                                let f = if o.is_disabled { 0 } else { 1 } | if chosen.contains(&o.id) { 2 } else { 0 };
                                recs.push(format!("{}|{}|{}", o.id, f, esc(&o.label)));
                            }
                        }
                    }
                }
                say(&format!("menu {view} {token} select {} {} {} {} {}", r.min.x, r.min.y, r.width(), r.height(), recs.join("\u{1}")));
                self.controls.insert(token, (view, cid, Pending::Select(sel)));
            }
            EmbedderControl::FilePicker(picker) => {
                let filters: Vec<&str> = picker.filter_patterns().iter().map(|filter| filter.0.as_str()).collect();
                if filters.len() > 64 || filters.iter().any(|filter| filter.len() > 1024 || filter.contains('\0')) { return; }
                let json = serde_json::json!({"multiple": picker.allow_select_multiple(), "filters": filters});
                let json = json.to_string();
                if json.len() > 64 * 1024 { return; }
                say(&format!("file-picker {view} {token} {json}"));
                self.controls.insert(token, (view, cid, Pending::File(picker)));
            }
            EmbedderControl::ColorPicker(picker) => {
                let color = picker.current_color().unwrap_or(servo::RgbColor { red: 0, green: 0, blue: 0 });
                say(&format!("color-picker {view} {token} {} {} {}", color.red, color.green, color.blue));
                self.controls.insert(token, (view, cid, Pending::Color(picker)));
            }
            // Input methods are handled separately; dropping a control resolves its native responder.
            _ => {}
        }
    }
}
