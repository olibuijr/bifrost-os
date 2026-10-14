//! Podcasts as an installed BifrOSt app: the former in-shell Podcasts, hosted through the SDK.
//! Playback goes through the audio service (`bifrost_app::audio`); text comes from the hardware keyboard.

mod chapters;
mod feed;
mod icon;
mod info;
mod library;
mod media;
mod opml;
mod podcasts;
mod prefs;
mod refresh;
mod search;
mod transcript;
mod transport;
mod wrap;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use bifrost_app::now_playing;
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// The largest subscriptions file the import reads.
const IMPORT_MAX: u64 = 4 << 20;

struct PodcastsApp {
    podcasts: Option<podcasts::Podcasts>,
}

/// Opens the file panel a menu command asked for (it needs the runtime context).
fn open_file_panel(p: &mut podcasts::Podcasts, ctx: &Ctx) {
    match p.take_file_job() {
        Some(podcasts::FileJob::Import) => p.begin_import(ctx.open_document(DocMode::Read, "Import Subscriptions", &["opml", "xml"])),
        Some(podcasts::FileJob::Export) => p.begin_export(ctx.save_document("Podcasts.opml", &["opml"])),
        None => {}
    }
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl App for PodcastsApp {
    fn id(&self) -> &str {
        "is.olibuijr.podcasts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // subscriptions and resume positions are written when they change
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.podcasts {
            Some(p) => p.set_size(w, h),
            None => self.podcasts = Some(podcasts::Podcasts::new(&ctx.ui, w, h, ctx.data_dir.clone(), Arc::new(springboard::shell::music::radio::Http::default()), &shell_dir())),
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match &ev {
            Input::Context { x, y } => {
                let Some(p) = &mut self.podcasts else { return false };
                let items = p.context_items(&ctx.ui, *x, *y);
                if !items.is_empty() {
                    ctx.context_menu(*x, *y, items);
                }
                return true;
            }
            Input::ContextAction(id) => {
                let Some(p) = &mut self.podcasts else { return false };
                p.menu_action(&ctx.ui, *id);
                open_file_panel(p, ctx);
                return true;
            }
            _ => {}
        }
        if let Input::Key(key) = &ev {
            let out = match key {
                KeyInput::Text(text) => Out::Text(text.clone()),
                KeyInput::Code(8) => Out::Backspace,
                KeyInput::Code(13) => Out::Enter,
                KeyInput::Code(9) => Out::Named("Tab"),
                KeyInput::Code(27) => Out::Hide,
                KeyInput::Code(38) => Out::Named("Up"),
                KeyInput::Code(40) => Out::Named("Down"),
                KeyInput::Code(37) => Out::Named("Left"),
                KeyInput::Code(39) => Out::Named("Right"),
                _ => return false,
            };
            if let Some(app) = &mut self.podcasts { app.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms()); return true; }
            return false;
        }
        let (Some(p), Input::Touch { kind, x, y, t, .. }) = (&mut self.podcasts, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        p.touch(&ctx.ui, e, t);
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.podcasts.as_ref().map_or_else(Vec::new, |p| p.controls(&ctx.ui))
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.podcasts.as_ref().map_or_else(Vec::new, |p| p.main_menu())
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(p) = &mut self.podcasts else { return false };
        p.menu_action(&ctx.ui, id);
        open_file_panel(p, ctx);
        true
    }

    /// The subscriptions file the user chose, or the place to write the export.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let Some(p) = &mut self.podcasts else { return false };
        match ev {
            DocumentEvent::Opened(mut doc) if p.is_import(doc.request) => {
                let mut bytes = Vec::new();
                let read = (&mut doc.file).take(IMPORT_MAX).read_to_end(&mut bytes);
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                match read {
                    Ok(_) => p.imported(&bytes),
                    Err(e) => {
                        ctx.log(&format!("import: {e}"));
                        p.file_failed(doc.request);
                    }
                }
                true
            }
            DocumentEvent::SaveTo(mut doc) => {
                let Some(bytes) = p.take_export(doc.request) else { return false };
                match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(doc.request),
                    Err(e) => {
                        ctx.log(&format!("export: {e}"));
                        ctx.abort_document(doc.request);
                    }
                }
                false
            }
            DocumentEvent::Saved(_) => {
                p.exported();
                true
            }
            DocumentEvent::Failed { request, .. } => {
                p.file_failed(request);
                false
            }
            _ => false,
        }
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(p) = &mut self.podcasts {
            p.suspend();
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(p) = &mut self.podcasts else { return false };
        let dirty = p.tick(&ctx.ui, t);
        p.sync_services(ctx);
        dirty
    }

    /// The reply to one of the app's service calls (Now Playing, Dock). Replies are not retried: an older shell's
    /// "no service" answer leaves the app as it is.
    fn service_reply(&mut self, _ctx: &Ctx, request: u32, _result: &Result<Json, String>) -> bool {
        self.podcasts.as_mut().is_some_and(|p| p.service_reply(request))
    }

    /// A transport tap on the Control Center tile, sent by the shell for the session the app published.
    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let (Some(p), Some(command)) = (&mut self.podcasts, now_playing::command_from_event(service, event, body)) else { return false };
        p.remote(command);
        p.sync_services(ctx);
        true
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.podcasts.as_ref().and_then(|p| p.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(p) = &mut self.podcasts {
            p.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(PodcastsApp { podcasts: None });
}
