//! News as an installed BifrOSt app: a full-size window with a source list of feeds and a unified toolbar.

mod feed;
mod feed_cache;
mod icon;
mod news;
mod pictures;
mod reader_size;
mod services;
mod wire;

use bifrost_app::controls::Rect;
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::share::{self, Payload};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use bifrost_core::json::Json;
use std::sync::Arc;

/// Edit > Translate Article: translates the open article (see `news::News::translate`).
const MENU_TRANSLATE: u32 = 1;
/// Edit > Share Article: the open article's page goes to the share menu.
const MENU_SHARE: u32 = 2;
/// View > Larger Text and View > Smaller Text: the reader's body text size (see `reader_size`).
const MENU_LARGER: u32 = 3;
const MENU_SMALLER: u32 = 4;

struct NewsApp {
    news: Option<news::News>,
    /// the network-path request sent at start; its reply sets the offline state
    path_request: Option<u32>,
    /// whether the keyboard is asked for (the search field has focus)
    kb: bool,
}

impl App for NewsApp {
    fn id(&self) -> &str {
        "is.olibuijr.news.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.news {
            Some(n) => n.set_size(w, h),
            None => {
                // the feed cache is rebuilt by the next fetch, so the app holds no unsaved state
                ctx.enable_sudden_termination();
                self.news = Some(news::News::new(&ctx.ui, w, h, ctx.data_dir.clone(), Arc::new(feed_cache::Conditional::default())));
                // once at start: the shell pushes path changes after this
                self.path_request = Some(services::request_path(ctx));
            }
        }
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.path_request != Some(request) {
            return false;
        }
        self.path_request = None;
        match (services::online_from_reply(result), &mut self.news) {
            (Some(online), Some(n)) => {
                n.set_online(online);
                true
            }
            _ => false,
        }
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        if let Some(online) = services::online_from_event(service, event, body) {
            return self.news.as_mut().is_some_and(|n| {
                n.set_online(online);
                true
            });
        }
        // the shell asks for a fresh top-stories timeline when its reload date passes
        if services::is_top_reload(service, event, body) {
            if let Some(n) = &self.news {
                services::publish_top(ctx, n.items(), services::now_secs());
            }
        }
        false
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let handled = self.input(ctx, ev);
        self.sync_keyboard(ctx);
        handled
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.news.as_mut().is_some_and(|n| {
            let pumped = n.pump();
            let ticked = n.tick(&ctx.ui, t);
            // only when the headlines changed: a fetch or a reload, never every frame
            if n.take_top_changed() {
                services::publish_top(ctx, n.items(), services::now_secs());
            }
            ticked | pumped
        })
    }

    /// (the body of `event`, before the keyboard is synced)
    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(n) = &mut self.news else { return false };
        let (kind, x, y, t) = match ev {
            Input::Touch { kind, x, y, t, .. } => (kind, x, y, t),
            Input::Activate(id) => return n.activate_control(&ctx.ui, id),
            Input::Key(k) => return n.key(k),
            _ => return false,
        };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        n.touch(&ctx.ui, e, t);
        true
    }

    /// Asks for the on-screen keyboard only while the toolbar search field has focus.
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.news.as_ref().is_some_and(|n| n.search_focused());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
    }

    /// Edit (translate and share) and View (text size), enabled while an article is open.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let news = self.news.as_ref();
        let open = news.is_some_and(|n| n.can_translate());
        let shareable = news.is_some_and(|n| n.share_link().is_some());
        vec![
            AppMenu {
                title: "Edit".into(),
                items: vec![
                    AppMenuItem::new(MENU_TRANSLATE, "Translate Article").enabled(open),
                    AppMenuItem::new(MENU_SHARE, "Share Article").enabled(shareable),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_LARGER, "Larger Text").enabled(news.is_some_and(|n| n.can_resize(true))),
                    AppMenuItem::new(MENU_SMALLER, "Smaller Text").enabled(news.is_some_and(|n| n.can_resize(false))),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_TRANSLATE => match &mut self.news {
                Some(n) if n.can_translate() => {
                    n.translate(&ctx.ui);
                    true
                }
                _ => false,
            },
            MENU_SHARE => match self.news.as_ref().and_then(|n| n.share_link()) {
                Some(url) => {
                    // a zero anchor, as the browser's Share sends it
                    let anchor = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
                    let _ = share::show_picker(ctx, &[Payload::Url(url)], anchor);
                    true
                }
                None => false,
            },
            MENU_LARGER | MENU_SMALLER => {
                let larger = id == MENU_LARGER;
                match &mut self.news {
                    Some(n) if n.can_resize(larger) => {
                        n.resize_text(&ctx.ui, larger);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.news.as_ref().map(|n| n.controls(&ctx.ui)).unwrap_or_default()
    }

    fn full_redraw(&self) -> bool {
        true
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.news.as_ref().and_then(|n| n.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(n) = &mut self.news {
            n.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(NewsApp { news: None, path_request: None, kb: false });
}
