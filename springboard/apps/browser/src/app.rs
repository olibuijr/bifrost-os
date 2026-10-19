//! SDK application boundary and process entry point.
use super::*;

impl App for Browser {
    fn id(&self) -> &str {
        "is.olibuijr.browser.desktop"
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menus()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.on_menu_action(ctx, id)
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.configure_window(ctx)
    }

    fn wake_fd(&self) -> Option<i32> {
        self.pool.as_ref().filter(|p| p.alive()).map(|p| p.fd())
    }

    /// The only periodic wake while the window is shown is the watchdog ping (and the short scroll probe of a tab
    /// being discarded); an idle static page costs nothing else. A hidden window is never asked.
    fn next_wake(&self, t: f64) -> Option<f64> {
        self.next_timed_ms().map(|ms| t + ms)
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.advance(ctx, t)
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        self.suspend_window(_ctx)
    }

    fn resume(&mut self, ctx: &Ctx) {
        self.resume_window(ctx)
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.on_service_reply(ctx, request, result)
    }

    fn document(&mut self, ctx: &Ctx, event: bifrost_app::DocumentEvent) -> bool {
        self.uploads.document(ctx, event, &mut self.pool)
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        self.on_service_event(_ctx, service, event, body)
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        self.on_input(ctx, ev)
    }

    fn commands(&self, ctx: &Ctx) -> bifrost_app::Commands {
        use bifrost_app::controls::Command as Edit;
        if self.main_menu.open() || self.sheet.open || self.overview.open || self.pageui.active() || self.lib.captures_keys() {
            return bifrost_app::Commands::default();
        }
        if self.editing {
            return self.address.commands(ctx.pasteboard_string().is_some());
        }
        bifrost_app::Commands::default().with(Edit::Cut, true).with(Edit::Copy, true)
            .with(Edit::Paste, ctx.pasteboard_string().is_some()).with(Edit::SelectAll, true)
    }

    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::HOVER | bifrost_app::pointer_events::WHEEL
    }

    fn pointer(&mut self, ctx: &Ctx, event: &bifrost_app::pointer_events::PointerEvent) -> bool {
        self.on_pointer(ctx, event)
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        self.ctl_methods()
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        self.ctl_rpc(method, params)
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.window_controls(ctx)
    }

    fn full_redraw(&self) -> bool {
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        self.draw_window(ctx, s, _t)
    }
}

pub(super) fn run() {
    if std::env::args().any(|a| a == "--version") {
        println!("BifrOSt Browser {} / Servo 0.7.0 (bifrost-webview)", env!("CARGO_PKG_VERSION"));
        return;
    }
    let private = std::env::args().any(|a| a == "--private");
    if private {
        remove_stale_private_profiles();
    }
    bifrost_app::run(Browser {
        pool: None,
        error: None,
        storm: Default::default(),
        watchdog: Default::default(),
        discard: Default::default(),
        clock: std::time::Instant::now(),
        crash_log: Vec::new(),
        crash: Default::default(),
        restore: Vec::new(),
        restore_sel: 0,
        renderer_fallback: false,
        tabs: Vec::new(),
        current: 0,
        address: TextField::new(),
        reader: reader::Reader::default(),
        hover: None,
        editing: false,
        press: None,
        bar_layout: None,
        main_menu: Default::default(),
        initial_url: std::env::args().skip(1).find(|a| !a.starts_with('-')),
        dark: None,
        store: if private {
            let mut st = store::Store::in_memory();
            st.private = true;
            st
        } else {
            store::Store::open(store::data_dir())
        },
        sugg: Vec::new(),
        sel: None,
        session: Default::default(),
        find: Default::default(),
        inspector: Inspector::new(),
        dev: devmenu::DevMenu::default(),
        ctl: Default::default(),
        sources: Vec::new(),
        downloads: Default::default(),
        uploads: Default::default(),
        lib: Default::default(),
        favicons: {
            let mut f = favicon::Favicons::default();
            f.set_private(private);
            f
        },
        autofill: Default::default(),
        overview: Default::default(),
        themes: Default::default(),
        pageui: pageui::PageUi::default(),
        prefs: prefs::Prefs::default(),
        sheet: prefs::Sheet::default(),
        private,
        long: None,
        swallow: false,
        drag: false,
        net: Default::default(),
        net_request: None,
        clip: None,
        pointer: Default::default(),
        dock: Default::default(),
    });
}
