//! Automator as an installed BifrOSt desktop app: a workflow builder that chains file, text, image and shell actions,
//! runs them in the background and keeps a log. `model.rs` holds the workflows and the runner, `actions.rs` what each
//! step does, `imageio.rs` the picture formats, `window.rs` the state and the geometry, and `draw.rs` the painting.
//! This file connects them to the shell: the window's events, and the open and save panels.

mod actions;
mod draw;
mod imageio;
mod model;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, Ctx, DocumentEvent, Input, KeyInput};
use std::io::{Read, Write};
use window::{Automator, Request};

/// The largest workflow file Open reads (bytes).
const OPEN_MAX: u64 = 1 << 20;

/// Key codes the editor uses (the ASCII codes the shell sends as `KeyInput::Code`).
const KEY_TAB: u32 = 9;
const KEY_ENTER: u32 = 13;
const KEY_BACKSPACE: u32 = 8;
const KEY_ESCAPE: u32 = 27;

struct AutomatorApp {
    auto: Automator,
    /// Whether the first configure has run (the keyboard is asked for once).
    started: bool,
    /// The open panel asked for by Open.
    open: Option<u32>,
    /// The save panel asked for by Save, with the bytes it will write.
    save: Option<(u32, Vec<u8>)>,
}

impl AutomatorApp {
    fn new() -> AutomatorApp {
        AutomatorApp {
            auto: Automator::default(),
            started: false,
            open: None,
            save: None,
        }
    }

    /// Does what a request from the window asks of the shell. Always asks for a frame.
    fn perform(&mut self, ctx: &Ctx, request: Request) -> bool {
        match request {
            Request::None => {}
            Request::Open => {
                self.open = Some(ctx.open_document(DocMode::Read, "Open Workflow", &[]))
            }
            Request::Save => {
                let name = self.auto.suggested_file_name();
                let request = ctx.save_document(&name, &[]);
                self.save = Some((request, self.auto.save_bytes()));
            }
        }
        true
    }

    /// Typing and editing keys: text goes to the focused field; Backspace, Tab, Enter and Escape edit and move focus.
    fn key(&mut self, key: KeyInput) -> bool {
        match key {
            KeyInput::Text(text) => self.auto.type_text(&text),
            KeyInput::Code(KEY_BACKSPACE) => self.auto.backspace(),
            KeyInput::Code(KEY_TAB) | KeyInput::Code(KEY_ENTER) => self.auto.next_field(),
            KeyInput::Code(KEY_ESCAPE) => self.auto.unfocus(),
            _ => return false,
        }
        true
    }
}

impl App for AutomatorApp {
    fn id(&self) -> &str {
        "is.olibuijr.automator.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.auto.resize(ctx.cfg.w as f32, ctx.cfg.h as f32);
        if !self.started {
            self.started = true;
            ctx.wants_keyboard(true);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let request = self.auto.touch(&ctx.ui, kind, x / scale, y / scale);
                self.perform(ctx, request)
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => {
                let request = self.auto.activate_id(id as u64);
                self.perform(ctx, request)
            }
            Input::Key(key) => self.key(key),
            _ => false,
        }
    }

    /// Takes what the run has reported since the last frame.
    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        self.auto.poll()
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.auto.wake_at(t)
    }

    fn wake_fd(&self) -> Option<i32> {
        self.auto.wake_fd()
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(&self.auto, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(&mut self.auto, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    /// The open panel's file is read and parsed; the save panel's file gets the bytes of the workflow, and is then
    /// committed. Answers that belong to no request of this app are released, and a cancelled panel says nothing.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) if self.open == Some(doc.request) => {
                self.open = None;
                let mut text = String::new();
                let result = if doc.size > OPEN_MAX {
                    Err(format!("{} is too large to be a workflow", doc.name))
                } else {
                    let mut reader = (&mut doc.file).take(OPEN_MAX);
                    reader
                        .read_to_string(&mut text)
                        .map_err(|e| format!("{}: {e}", doc.name))
                        .and_then(|_| self.auto.open_text(&text))
                };
                // one open is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                if let Err(error) = result {
                    self.auto.status = error;
                }
                true
            }
            DocumentEvent::Opened(doc) => {
                ctx.release_document(doc.grant);
                false
            }
            DocumentEvent::SaveTo(mut doc) => {
                match self.save.take() {
                    Some((request, bytes)) if request == doc.request => {
                        match doc.file.write_all(&bytes) {
                            Ok(()) => {
                                ctx.commit_document(request);
                                self.auto.named_after(&doc.name);
                            }
                            Err(e) => {
                                ctx.abort_document(request);
                                self.auto.status = format!("Could not save: {e}");
                            }
                        }
                    }
                    other => {
                        self.save = other;
                        ctx.abort_document(doc.request);
                    }
                }
                true
            }
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                false
            }
            DocumentEvent::Failed { request, code } => {
                if self.open == Some(request) {
                    self.open = None;
                }
                if self.save.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.save = None;
                }
                if !matches!(code, DocErrorCode::Cancelled) {
                    self.auto.status = "The file could not be opened or saved".into();
                }
                true
            }
            _ => false,
        }
    }
}

fn main() {
    bifrost_app::run(AutomatorApp::new());
}
