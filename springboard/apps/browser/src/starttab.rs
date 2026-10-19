//! The Start Page's place in the Browser: when it covers the page area, how a touch, the wheel and an accessibility
//! action reach it, and how a tile opens in the tab. The data, layout and drawing are in `startpage.rs`.
use super::*;

impl Browser {
    /// The Start Page covers the page area: the current tab is blank (no address or `about:blank`) and the engine, an
    /// error message and Responsive Design Mode are not showing something else.
    pub(super) fn start_visible(&self) -> bool {
        self.error.is_none()
            && self.dev.responsive.is_none()
            && self.view().and_then(|v| self.pool.as_ref()?.state(v)).is_some_and(|st| startpage::is_blank(&st.url))
    }

    /// The page area in window pixels.
    fn start_area(&self) -> Option<(i32, i32, u32, u32)> {
        self.pool.as_ref().map(|p| p.frame())
    }

    /// Draws the Start Page over the page area. Called right after the engine's frame, so the toolbar, the tab strip and
    /// every popover are drawn over it.
    pub(super) fn draw_start_page(&mut self, ctx: &Ctx, s: &mut Surface) {
        if !self.start_visible() {
            return;
        }
        let (Some(view), Some(area)) = (self.view(), self.start_area()) else { return };
        self.start_page.show(view);
        self.start_page.refresh(&self.store, self.private, store::now());
        self.start_page.draw(s, &ctx.ui, area, &mut self.favicons);
    }

    /// A touch or the desktop pointer at window pixel (`x`, `y`). A press on the page area that no chrome covers is the
    /// Start Page's (it never reaches the blank page under it); the address field loses focus, as on any page. The
    /// rest of that press, its drag and its release, follow it. True when used.
    pub(super) fn start_touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let ui = &ctx.ui;
        let visible = self.start_visible();
        if kind == TouchKind::Down && (!visible || self.chrome_hit(ui, ctx.cfg.w, x, y) != Some(Press::Web)) {
            return false;
        }
        let Some(area) = self.start_area() else { return false };
        // a press that outlived the page (the tab changed under the finger) is dropped, not delivered
        let kind_for_page = if visible { kind } else { TouchKind::Cancel };
        match self.start_page.touch(kind_for_page, x, y, area, ui.pt(1.0), ui.pt(TAP_SLOP_PT)) {
            startpage::Outcome::Pass => false,
            startpage::Outcome::Used => {
                if kind == TouchKind::Down {
                    self.editing = false;
                    self.sync_address();
                }
                true
            }
            startpage::Outcome::Open(url) => {
                self.start_open(&url);
                true
            }
        }
    }

    /// The wheel or a two-finger scroll over the Start Page. True when the page area took it.
    pub(super) fn start_wheel(&mut self, ctx: &Ctx, x: f32, y: f32, dy: f32) -> bool {
        if !self.start_visible() || !self.pool.as_ref().is_some_and(|p| p.contains(x, y)) {
            return false;
        }
        let Some(area) = self.start_area() else { return false };
        self.start_page.wheel(dy, area, ctx.ui.pt(1.0));
        true
    }

    /// A tile or row opens in this tab. The address shows it at once; the engine reports the load a moment later.
    pub(super) fn start_open(&mut self, url: &str) {
        self.editing = false;
        if let Some(v) = self.view() {
            self.load_chosen(v, url);
        }
        self.address.clear();
        self.address.push_str(url);
    }

    /// The tiles and rows in view as accessibility buttons.
    pub(super) fn start_nodes(&self, ui: &Ui) -> Vec<Node> {
        match self.start_area() {
            Some(area) if self.start_visible() => self.start_page.nodes(area, ui.pt(1.0)),
            _ => Vec::new(),
        }
    }

    /// The page menu has nothing to offer on the Start Page: a secondary click over it is swallowed.
    pub(super) fn start_swallows_context(&self, x: f32, y: f32) -> bool {
        self.start_visible() && self.pool.as_ref().is_some_and(|p| p.contains(x, y))
    }
}
