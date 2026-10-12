//! Shortcuts as an installed BifrOSt app. Actions that need the shell
//! (Wi-Fi, Low Power, DND, brightness, open app, screenshot, timer) cannot be
//! sent over the app protocol yet and are only logged.

mod automation;
mod icon;
mod model;
mod widget;

use bifrost_app::control::RpcError;
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
use std::path::Path;

struct ShortcutsApp {
    w: Option<widget::Shortcuts>,
}

impl ShortcutsApp {
    /// The shortcut list: the window's copy when it is open, otherwise the saved file.
    fn shortcuts(&self, data_dir: &Path) -> Vec<model::Shortcut> {
        match &self.w {
            Some(c) => c.list().to_vec(),
            None => automation::stored(data_dir),
        }
    }
}

impl App for ShortcutsApp {
    fn id(&self) -> &str {
        "is.olibuijr.shortcuts.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.w {
            Some(c) => c.set_size(w, h),
            None => self.w = Some(widget::Shortcuts::new(&ctx.ui, w, h, ctx.data_dir.join("shortcuts.tsv"))),
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.w, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        if !c.take_runs().is_empty() {
            c.show_run_unavailable(t);
        }
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.w.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.w.as_ref().and_then(|c| c.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.w.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn intents(&self) -> Vec<Intent> {
        automation::declared()
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        automation::perform(&self.shortcuts(data_dir), id, params)
    }

    fn entities(&mut self, data_dir: &Path, ty: &str, query: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
        automation::entities(&self.shortcuts(data_dir), ty, query)
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["script.dictionary", "script.get", "script.set"]
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            "script.dictionary" => Ok(automation::dictionary_json()),
            "script.get" => automation::script_get(&self.shortcuts(data_dir), params),
            "script.set" => match self.w.as_mut() {
                Some(c) => c.edit_list(|l| automation::script_set(l, params)),
                None => {
                    // no window yet: change the saved file, so the next launch shows the change
                    let mut list = automation::stored(data_dir);
                    let out = automation::script_set(&mut list, params)?;
                    model::save(&data_dir.join(automation::FILE), &list)
                        .map_err(|e| intents::failed(format!("could not save the list: {e}")))?;
                    Ok(out)
                }
            },
            _ => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.w {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ShortcutsApp { w: None });
}
