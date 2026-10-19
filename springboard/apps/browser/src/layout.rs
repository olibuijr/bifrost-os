//! Window geometry, toolbar controls and hit testing.
use super::*;

impl Browser {
    pub(super) fn pt(ui: &Ui) -> f32 {
        ui.pt(1.0)
    }

    pub(super) fn strip_shown(&self) -> bool {
        self.tabs.len() > 1
    }

    /// Top of the Favorites bar (below the tab strip), points.
    pub(super) fn fav_top(&self) -> f32 {
        UNIFIED_BAR_PT + if self.strip_shown() { TAB_STRIP_PT } else { 0.0 }
    }

    /// Top of the web content, points.
    pub(super) fn content_top(&self) -> f32 {
        self.fav_top() + self.lib.bar_height() + self.find_height()
    }

    /// The tab slot under window pixel x (clamped to the strip).
    pub(super) fn tab_at(&self, ui: &Ui, w: u32, x: f32) -> Option<usize> {
        let px = x / ui.pt(1.0);
        let rects = self.tab_rects(ui, w);
        rects.iter().position(|r| px < r.x + r.w + 2.0).or(rects.len().checked_sub(1))
    }

    pub(super) fn toolbar(&self) -> Toolbar {
        let st = self.view().and_then(|v| self.pool.as_ref()?.state(v));
        let fwd = st.as_ref().is_some_and(|s| s.can_go_forward);
        let back = self.view().is_some_and(|v| self.can_back(v));
        Toolbar::new(vec![
            ToolbarItem::icon(BACK, "Back", "chevron.left").in_group(1).enabled(back),
            ToolbarItem::icon(FORWARD, "Forward", "chevron.right").in_group(1).enabled(fwd),
            ToolbarItem::flexible_space(),
            ToolbarItem::custom(ADDRESS, "Address", 0.0),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(RELOAD, "Reload", "arrow.clockwise").enabled(self.view().is_some()),
        ]
        .into_iter()
        // Browser shows the Downloads button once something was downloaded
        .chain((!self.downloads.items.is_empty()).then(|| ToolbarItem::icon(DOWNLOADS, "Downloads", "arrow.down.circle")))
        .chain(std::iter::once(ToolbarItem::icon(NEW_TAB, "New Tab", "plus")))
        .chain(std::iter::once(ToolbarItem::icon(MAIN_MENU, "Main Menu", "ellipsis")))
        .collect())
    }

    pub(super) fn bar_bounds(ui: &Ui, w: u32) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (w as f32 / ui.pt(1.0) - x - 12.0).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The address field takes the space between the navigation and the trailing buttons, centred, at most
    /// `ADDRESS_MAX_PT` wide.
    pub(super) fn address_rect(&self, ui: &Ui, w: u32) -> Rect {
        let bar = Self::bar_bounds(ui, w);
        let lead = bar.x + 2.0 * widgets::CAPSULE_PT + widgets::TOOLBAR_GAP_PT * 2.0;
        let extra = if self.downloads.items.is_empty() { 0.0 } else { widgets::CAPSULE_PT + widgets::TOOLBAR_GAP_PT };
        let trail = bar.x + bar.w - extra - 3.0 * widgets::CAPSULE_PT - widgets::TOOLBAR_GAP_PT * 4.0;
        let room = (trail - lead).max(0.0);
        let width = room.min(ADDRESS_MAX_PT);
        let window_mid = w as f32 / ui.pt(1.0) / 2.0;
        let x = (window_mid - width / 2.0).clamp(lead, (trail - width).max(lead));
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: width, h }
    }

    pub(super) fn tab_rects(&self, ui: &Ui, w: u32) -> Vec<Rect> {
        let total = w as f32 / ui.pt(1.0) - 16.0;
        let n = self.tabs.len().max(1) as f32;
        let tw = total / n;
        (0..self.tabs.len()).map(|i| Rect { x: 8.0 + i as f32 * tw, y: UNIFIED_BAR_PT + 3.0, w: tw - 4.0, h: TAB_STRIP_PT - 6.0 }).collect()
    }

    pub(super) fn close_rect(r: Rect) -> Rect {
        Rect { x: r.x + 6.0, y: r.y + (r.h - 16.0) / 2.0, w: 16.0, h: 16.0 }
    }

    pub(super) fn place_web(&mut self, ctx: &Ctx) {
        let top = ctx.ui.pt(self.content_top()) as i32;
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        let dark = ctx.cfg.dark;
        let reserved = self.inspector.reserved_px(&ctx.ui);
        if let Some(p) = &mut self.pool {
            let area = (0, top, w, h.saturating_sub(top as u32 + reserved));
            match &self.dev.responsive {
                Some(r) => {
                    p.set_viewport_override(area, r.w, r.h, r.dpr as f32);
                }
                None => p.set_frame(area.0, area.1, area.2, area.3, ctx.cfg.scale),
            }
            p.set_dark(dark);
        }
        self.dark = Some(dark);
    }

    pub(super) fn chrome_hit(&self, ui: &Ui, w: u32, x: f32, y: f32) -> Option<Press> {
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if self.editing {
            if let Some(i) = suggest::hit(self.address_rect(ui, w), self.sugg.len(), px, py) {
                return Some(Press::Sugg(i));
            }
        }
        if py < UNIFIED_BAR_PT {
            if !self.editing && self.view().is_some_and(|v| self.reader.button(v)) && reader::button_rect(self.address_rect(ui, w)).contains(px, py) {
                return Some(Press::Reader);
            }
            if !self.editing && self.view().is_some_and(|v| self.pool.as_ref().and_then(|p| p.state(v)).is_some()) && star_rect(self.address_rect(ui, w)).contains(px, py) {
                return Some(Press::Bookmark);
            }
            if self.address_rect(ui, w).contains(px, py) {
                return Some(Press::Bar(ADDRESS));
            }
            return self.bar_layout.as_ref().and_then(|l| l.hit(px, py)).map(Press::Bar);
        }
        if self.strip_shown() && py < UNIFIED_BAR_PT + TAB_STRIP_PT {
            for (i, r) in self.tab_rects(ui, w).iter().enumerate() {
                if Self::close_rect(*r).contains(px, py) {
                    return Some(Press::TabClose(i));
                }
                if r.contains(px, py) {
                    return Some(Press::Tab(i));
                }
            }
            return None;
        }
        match self.banner_hit(w as f32 / ui.pt(1.0), px, py) {
            Some(crashbanner::BannerHit::Close) => return Some(Press::BannerClose),
            Some(crashbanner::BannerHit::Reload) => return Some(Press::BannerReload),
            None => {}
        }
        Some(Press::Web)
    }

    pub(super) fn configure_window(&mut self, ctx: &Ctx) {
        ctx.wants_keyboard(true);
        self.inspector.load(ctx);
        self.prefs = prefs::Prefs::load(ctx);
        self.lib.load(ctx);
        // No unsaved state: the shell may stop the browser at once on quit (docs/bifrost-ui.md "Sudden termination").
        ctx.enable_sudden_termination();
        self.start();
        self.place_web(ctx);
        self.net_request = Some(network_path::request_path(ctx));
    }

    pub(super) fn window_controls(&self, ctx: &Ctx) -> Vec<Node> {
        let ui = &ctx.ui;
        if self.pageui.active() {
            return self.pageui.controls(ui, ctx.cfg.w, ctx.cfg.h);
        }
        if self.main_menu.open() {
            return self.main_menu.controls(ui, ctx.cfg.w, ctx.cfg.h, self.main_menu_anchor(ui, ctx.cfg.w));
        }
        let mut nodes: Vec<Node> = Vec::new();
        if let Some(l) = &self.bar_layout {
            let bar = self.toolbar();
            for (id, r) in &l.visible {
                if let Some(item) = bar.items.iter().find(|i| i.id == *id && *id != ADDRESS) {
                    nodes.push(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled });
                }
            }
        }
        nodes.push(Node { id: ADDRESS as u64, role: Role::TextField, label: "Address".into(), value: self.address.to_string(), bounds: self.address_rect(ui, ctx.cfg.w), enabled: true });
        if self.strip_shown() {
            for (i, r) in self.tab_rects(ui, ctx.cfg.w).into_iter().enumerate() {
                let title = self.pool.as_ref().and_then(|p| p.state(self.tabs.get(i)?.view)).map(|s| s.title.clone()).unwrap_or_default();
                nodes.push(Node { id: TAB0 + i as u64, role: Role::Button, label: if title.is_empty() { "Untitled".into() } else { title }, value: if i == self.current { "selected".into() } else { String::new() }, bounds: r, enabled: true });
            }
        }
        nodes
    }
}
