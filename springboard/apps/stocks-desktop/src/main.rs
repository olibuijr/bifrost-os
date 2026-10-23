//! Stocks as an installed BifrOSt app: a watchlist sidebar with the latest quote and a sparkline of each symbol, and
//! a price chart of the selected one, in a full-size window. Quotes and daily history come from Yahoo Finance's public
//! chart endpoint over HTTPS (no key, no account) and are cached on disk, so the last values show when the network
//! does not answer.

mod data;
mod feed;
mod model;
mod state;
mod store;
mod view;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use springboard::shell::keyboard::Out;
use springboard::shell::music::radio::{Fetch, Http};
use state::Stocks;
use std::sync::Arc;

struct StocksApp {
    stocks: Option<Stocks>,
    /// The keyboard the app last asked the shell for (it is shown while the add field is focused).
    kb: bool,
    /// The first `tick` starts the feed (fetches and the cached chart); it needs the loop's clock.
    started: bool,
}

impl StocksApp {
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.stocks.as_ref().is_some_and(|s| s.field.focused);
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
    }
}

/// The network, except under the testing kit (`BIFROST_TEST_PIN=1`): offline, so a snapshot never shows live prices.
fn pinned_fetcher() -> Arc<dyn Fetch> {
    struct Offline;
    impl Fetch for Offline {
        fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("offline"))
        }
    }
    if bifrost_app::testing::pinned() { Arc::new(Offline) } else { Arc::new(Http::default()) }
}

impl App for StocksApp {
    fn id(&self) -> &str {
        "is.olibuijr.stocks.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // every change is saved as it happens, so nothing is held back
        let (w, h) = (ctx.cfg.w as usize, ctx.cfg.h as usize);
        if let Some(s) = self.stocks.as_mut() {
            s.set_size(w, h);
            return;
        }
        let mut s = Stocks::new(&ctx.ui, ctx.data_dir.clone(), pinned_fetcher());
        s.set_size(w, h);
        self.stocks = Some(s);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(stocks) = &mut self.stocks else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                stocks.touch(&ctx.ui, e, t);
            }
            Input::Key(k) => {
                let out = match k {
                    KeyInput::Text(text) => Out::Text(text),
                    KeyInput::Code(8) => Out::Backspace,
                    KeyInput::Code(13) => Out::Enter,
                    KeyInput::Code(9) => Out::Named("Tab"),
                    KeyInput::Code(37) => Out::Named("Left"),
                    KeyInput::Code(39) => Out::Named("Right"),
                    KeyInput::Code(46) => Out::Named("Delete"),
                    KeyInput::Code(_) => return false,
                };
                stocks.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms());
            }
            // a right click on a row: Move Up, Move Down, Remove from Watchlist
            Input::Context { x, y } => {
                let items = stocks.context_items(&ctx.ui, x, y, bifrost_app::proto::sys::now_ms());
                if items.is_empty() {
                    return false;
                }
                ctx.context_menu(x, y, items);
            }
            Input::ContextAction(id) => {
                if !stocks.context_action(id, bifrost_app::proto::sys::now_ms()) {
                    return false;
                }
            }
            _ => return false,
        }
        self.sync_keyboard(ctx);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(stocks) = &mut self.stocks else { return false };
        if !self.started {
            self.started = true;
            stocks.start(t);
        }
        let changed = stocks.tick(t);
        self.sync_keyboard(ctx);
        changed
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.stocks.as_ref().and_then(|s| s.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, surface: &mut Surface, t: f64) -> Damage {
        if let Some(stocks) = &mut self.stocks {
            stocks.draw(surface, &ctx.ui, t);
        }
        let mut d = Damage::new(surface.w, surface.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(StocksApp { stocks: None, kb: false, started: false });
}

#[cfg(test)]
pub fn test_ui() -> bifrost_app::Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = bifrost_app::Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}
