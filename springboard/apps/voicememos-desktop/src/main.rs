//! Voice Memos as an installed BifrOSt app: the former in-shell Voice Memos, hosted through the SDK.
//! Recording and playback go through the audio service (`bifrost_app::audio`). The system services are glue in
//! `services`: the microphone permission (`privacy`), the audio devices (`audio-devices`) and Now Playing.

mod enhance;
mod services;
mod store;
mod voicememos;

use bifrost_app::audio_devices;
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::now_playing;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, TouchKind, KeyInput};
use bifrost_core::json::Json;
use springboard::shell::keyboard::Out;
use springboard::shell::clock::Wall;
use std::path::PathBuf;

struct VoiceMemosApp {
    memos: Option<voicememos::VoiceMemos>,
    last_t: f64,
    bus: services::Bus,
    /// a save panel is open for this recording: the request and the recording's file
    export: Option<(u32, PathBuf)>,
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl VoiceMemosApp {
    /// Sends the service calls the model queued on the last event, tick or reply.
    fn flush(&mut self, ctx: &Ctx) {
        if let Some(m) = &mut self.memos {
            for want in m.take_wants() {
                self.bus.send(ctx, want);
            }
        }
    }

    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let Input::Key(key) = &ev {
            let out = match key {
                KeyInput::Text(text) => Out::Text(text.clone()),
                KeyInput::Code(8) => Out::Backspace,
                KeyInput::Code(13) => Out::Enter,
                KeyInput::Code(9) => Out::Named("Tab"),
                KeyInput::Code(27) if self.memos.as_ref().is_some_and(|m| m.search_active()) => Out::Named("Escape"),
                KeyInput::Code(37) => Out::Named("Left"),
                KeyInput::Code(38) => Out::Named("Up"),
                KeyInput::Code(39) => Out::Named("Right"),
                KeyInput::Code(40) => Out::Named("Down"),
                _ => return false,
            };
            self.last_t = bifrost_app::proto::sys::now_ms();
            if let Some(memos) = &mut self.memos { memos.key(&ctx.ui, out, self.last_t); return true; }
            return false;
        }
        let (Some(m), Input::Touch { kind, x, y, t, .. }) = (&mut self.memos, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        self.last_t = t;
        m.touch(&ctx.ui, e, t);
        true
    }
}

impl App for VoiceMemosApp {
    fn id(&self) -> &str {
        "is.olibuijr.voicememos.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.memos {
            Some(m) => m.set_size(w, h),
            None => self.memos = Some(voicememos::VoiceMemos::new(&ctx.ui, w, h, ctx.data_dir.clone(), Wall::sample(0.0), &shell_dir())),
        }
        self.flush(ctx);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let needs_frame = self.input(ctx, ev);
        self.flush(ctx);
        needs_frame
    }

    fn suspend(&mut self, ctx: &Ctx) {
        if let Some(m) = &mut self.memos {
            m.suspend();
        }
        self.flush(ctx);
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.last_t = t;
        // the wall clock is re-read every tick, so day titles and timestamps follow midnight and time-zone changes
        let needs_frame = self.memos.as_mut().is_some_and(|m| {
            m.set_wall(Wall::sample(t));
            m.tick(&ctx.ui, t)
        });
        self.flush(ctx);
        needs_frame
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.memos.as_ref().and_then(|m| m.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.memos.as_ref().map(|m| m.controls(&ctx.ui)).unwrap_or_default()
    }

    /// A service answered. Only the replies to this app's questions are taken here; others stay for `take_reply`.
    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(ask) = self.bus.take(request) else { return false };
        if let Some(m) = &mut self.memos {
            m.reply(ask, result);
        }
        self.flush(ctx);
        true
    }

    /// The shell pushed an event: a device or route change, or a transport command on the Now Playing tile.
    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(m) = &mut self.memos else { return false };
        if service == audio_devices::SERVICE && audio_devices::decode_event(event, body).is_some() {
            m.devices_changed();
        } else if let Some(command) = now_playing::command_from_event(service, event, body) {
            m.command(command);
        } else {
            return false;
        }
        self.flush(ctx);
        true
    }

    /// Two app menus between App and the shell's File, Edit, View, Window and Help: "Recording" (the take and the
    /// selected recording) and "Library" (favourites, folders).
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let m = self.memos.as_ref();
        let on = m.is_some_and(|m| m.has_selection());
        let split = m.is_some_and(|m| m.can_split());
        let skip = m.is_some_and(|m| m.skip_silence_on());
        let recording = m.is_some_and(|m| m.is_recording());
        let paused = m.is_some_and(|m| m.is_paused());
        let favorite = m.is_some_and(|m| m.selected_favorite());
        let in_folder = m.is_some_and(|m| m.in_folder());
        let folders: Vec<String> = m.map(|m| m.folders().to_vec()).unwrap_or_default();
        let current = m.map(|m| m.selected_folder().to_string()).unwrap_or_default();
        let inputs = m.map(|m| m.input_choices()).unwrap_or_default();

        let mut recording_items = vec![
            AppMenuItem::new(voicememos::MENU_PAUSE, if paused { "Resume Recording" } else { "Pause Recording" }).enabled(recording),
            AppMenuItem::separator(),
            AppMenuItem::new(voicememos::MENU_TRIM, "Trim\u{2026}").key("cmd+t").enabled(on),
            AppMenuItem::new(voicememos::MENU_REPLACE, "Replace\u{2026}").enabled(on),
            AppMenuItem::new(voicememos::MENU_SPLIT, "Split at Playhead").enabled(split),
            AppMenuItem::new(voicememos::MENU_ENHANCE, "Enhance Recording").enabled(on),
            AppMenuItem::new(voicememos::MENU_NOTES, "Notes\u{2026}").key("cmd+shift+n").enabled(on),
            AppMenuItem::separator(),
            AppMenuItem::new(voicememos::MENU_RENAME, "Rename\u{2026}").enabled(on),
            AppMenuItem::new(voicememos::MENU_EXPORT, "Export\u{2026}").key("cmd+shift+e").enabled(on),
            AppMenuItem::separator(),
            AppMenuItem::new(voicememos::MENU_SKIP, "Skip Silence").checked(skip).enabled(on),
        ];
        if !inputs.is_empty() {
            recording_items.push(AppMenuItem::separator());
            recording_items.push(AppMenuItem::submenu("Input"));
            for (n, (name, chosen)) in inputs.iter().enumerate() {
                recording_items.push(AppMenuItem::new(voicememos::MENU_INPUT + n as u32, name).checked(*chosen).enabled(!recording).depth(1));
            }
        }

        let mut library_items = vec![
            AppMenuItem::new(voicememos::MENU_FAVORITE, "Favourite").key("cmd+d").checked(favorite).enabled(on),
            AppMenuItem::separator(),
            AppMenuItem::new(voicememos::MENU_NEW_FOLDER, "New Folder\u{2026}").key("cmd+shift+f"),
            AppMenuItem::new(voicememos::MENU_DELETE_FOLDER, "Delete Folder").enabled(in_folder),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Move to Folder"),
            AppMenuItem::new(voicememos::MENU_MOVE, "No Folder").checked(on && current.is_empty()).enabled(on).depth(1),
        ];
        for (n, name) in folders.iter().enumerate() {
            library_items.push(AppMenuItem::new(voicememos::MENU_MOVE + 1 + n as u32, name).checked(on && *name == current).enabled(on).depth(1));
        }
        vec![AppMenu { title: "Recording".into(), items: recording_items }, AppMenu { title: "Library".into(), items: library_items }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(m) = &mut self.memos else { return false };
        if id == voicememos::MENU_EXPORT {
            // the save panel is the shell's: the answer comes back through `document`
            if let Some((name, path)) = m.export_source() {
                let request = ctx.save_document(&name, &["wav"]);
                self.export = Some((request, path));
            }
            return true;
        }
        let handled = m.menu(&ctx.ui, id);
        self.flush(ctx);
        handled
    }

    /// The answer of the save panel: write the recording to the file the shell hands over, then commit it.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        let Some(m) = &mut self.memos else { return false };
        match ev {
            DocumentEvent::SaveTo(mut doc) => {
                let Some((request, path)) = self.export.clone() else { return false };
                if doc.request != request {
                    return false;
                }
                let written = std::fs::File::open(&path).and_then(|mut source| std::io::copy(&mut source, &mut doc.file));
                match written {
                    Ok(_) => ctx.commit_document(request),
                    Err(e) => {
                        ctx.abort_document(request);
                        self.export = None;
                        m.set_error(&format!("Cannot export: {e}"));
                    }
                }
                true
            }
            DocumentEvent::Saved(doc) => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == doc.request) {
                    self.export = None;
                    m.set_notice(&format!("Exported {}", doc.name));
                    return true;
                }
                false
            }
            DocumentEvent::Failed { request, .. } => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    // the panel was cancelled (or refused)
                    self.export = None;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(m) = &mut self.memos {
            m.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(VoiceMemosApp { memos: None, last_t: 0.0, bus: services::Bus::default(), export: None });
}
