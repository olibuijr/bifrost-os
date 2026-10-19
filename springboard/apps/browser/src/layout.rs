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
        ]
        .into_iter()
        // Browser shows the Downloads button once something was downloaded
        .chain((!self.downloads.items.is_empty()).then(|| ToolbarItem::icon(DOWNLOADS, "Downloads", "arrow.down.circle")))
        // Share, New Tab and Show All Tabs are one glass capsule at the trailing edge. Reload and the page menu are in
        // the address field, the bookmark star is gone (Bookmark This Page is in the menus and on Cmd+D).
        .chain([
            ToolbarItem::icon(SHARE, "Share", "square.and.arrow.up").in_group(2).enabled(self.web_url().is_some()),
            ToolbarItem::icon(NEW_TAB, "New Tab", "plus").in_group(2),
            ToolbarItem::icon(OVERVIEW, "Show All Tabs", "square.on.square").in_group(2).selected(self.overview.open).enabled(!self.tabs.is_empty()),
        ])
        .collect())
    }

    pub(super) fn bar_bounds(ui: &Ui, w: u32) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (w as f32 / ui.pt(1.0) - x - 12.0).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The address field takes the space between the navigation capsule and the trailing buttons, centred, at most
    /// `ADDRESS_MAX_PT` wide. The limits come from the toolbar's own layout, so a Downloads button, or a group that
    /// moved behind the overflow button in a narrow window, moves the field's edge with it.
    pub(super) fn address_rect(&self, ui: &Ui, w: u32) -> Rect {
        let bar = Self::bar_bounds(ui, w);
        let placed = widgets::toolbar_layout(ui, bar, &self.toolbar());
        let gap = widgets::TOOLBAR_GAP_PT;
        let lead = placed.rect_of(FORWARD).map_or(bar.x, |r| r.right() + gap);
        let trail = [DOWNLOADS, SHARE, NEW_TAB, OVERVIEW]
            .iter()
            .filter_map(|id| placed.rect_of(*id))
            .map(|r| r.x)
            .chain(placed.overflow_button.map(|r| r.x))
            .fold(bar.x + bar.w, f32::min)
            - gap;
        let room = (trail - lead).max(0.0);
        let width = room.min(ADDRESS_MAX_PT);
        let window_mid = w as f32 / ui.pt(1.0) / 2.0;
        let x = (window_mid - width / 2.0).clamp(lead, (trail - width).max(lead));
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: width, h }
    }

    /// True while the address field shows a loaded page: the page menu icon is at its leading edge and reload (stop
    /// while the page loads) at its trailing edge. Editing and an empty field show the search field instead.
    pub(super) fn field_shows_page(&self) -> bool {
        !self.editing && !self.address.is_empty() && self.view().is_some_and(|v| self.pool.as_ref().is_some_and(|p| p.state(v).is_some()))
    }

    /// The selected page is still loading: the field's button is Stop.
    pub(super) fn page_loading(&self) -> bool {
        self.view().and_then(|v| self.pool.as_ref()?.state(v)).is_some_and(|s| s.loading)
    }

    /// The page menu icon at the field's leading edge (a square the height of the field, points).
    pub(super) fn page_menu_rect(field: Rect) -> Rect {
        Rect { x: field.x, y: field.y, w: field.h, h: field.h }
    }

    /// The reload / stop button at the field's trailing edge (a square the height of the field, points).
    pub(super) fn reload_rect(field: Rect) -> Rect {
        Rect { x: field.x + field.w - field.h, y: field.y, w: field.h, h: field.h }
    }

    /// `r` shrunk by `k` of its size on every side: the box a glyph is drawn in inside its button.
    pub(super) fn glyph_box(r: Rect, k: f32) -> Rect {
        Rect { x: r.x + r.w * k, y: r.y + r.h * k, w: r.w * (1.0 - 2.0 * k), h: r.h * (1.0 - 2.0 * k) }
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
            let field = self.address_rect(ui, w);
            if self.field_shows_page() {
                if Self::page_menu_rect(field).contains(px, py) {
                    return Some(Press::Bar(MAIN_MENU));
                }
                if Self::reload_rect(field).contains(px, py) {
                    return Some(Press::Bar(if self.page_loading() { STOP } else { RELOAD }));
                }
            }
            if field.contains(px, py) {
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
        if self.find.open {
            if let Some(id) = findbar::hit(w as f32 / ui.pt(1.0), self.content_top() - findbar::FIND_BAR_PT, px, py) {
                return Some(Press::Bar(id));
            }
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
        let field = self.address_rect(ui, ctx.cfg.w);
        nodes.push(Node { id: ADDRESS as u64, role: Role::TextField, label: "Address".into(), value: self.address.to_string(), bounds: field, enabled: true });
        if self.field_shows_page() {
            let (id, label) = if self.page_loading() { (STOP, "Stop") } else { (RELOAD, "Reload") };
            nodes.push(Node { id: MAIN_MENU as u64, role: Role::Button, label: "Page Menu".into(), value: String::new(), bounds: Self::page_menu_rect(field), enabled: true });
            nodes.push(Node { id: id as u64, role: Role::Button, label: label.into(), value: String::new(), bounds: Self::reload_rect(field), enabled: true });
        }
        if self.find.open {
            nodes.extend(self.find_controls(ui, ctx.cfg.w));
        }
        if self.strip_shown() {
            for (i, r) in self.tab_rects(ui, ctx.cfg.w).into_iter().enumerate() {
                let title = self.pool.as_ref().and_then(|p| p.state(self.tabs.get(i)?.view)).map(|s| s.title.clone()).unwrap_or_default();
                nodes.push(Node { id: TAB0 + i as u64, role: Role::Button, label: if title.is_empty() { "Untitled".into() } else { title }, value: if i == self.current { "selected".into() } else { String::new() }, bounds: r, enabled: true });
            }
        }
        nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> Rect {
        Rect { x: 100.0, y: 8.0, w: 400.0, h: 36.0 }
    }

    #[test]
    fn the_page_menu_and_reload_squares_sit_at_the_two_edges_of_the_field() {
        let f = field();
        let (menu, reload) = (Browser::page_menu_rect(f), Browser::reload_rect(f));
        assert_eq!((menu.x, menu.w, menu.h), (f.x, f.h, f.h));
        assert_eq!((reload.x + reload.w, reload.w, reload.h), (f.x + f.w, f.h, f.h));
        assert!(menu.right() <= reload.x, "the two buttons do not overlap");
        assert_eq!((menu.y, reload.y), (f.y, f.y));
    }

    #[test]
    fn a_glyph_box_is_centred_in_its_button() {
        let b = Browser::glyph_box(Rect { x: 10.0, y: 20.0, w: 40.0, h: 40.0 }, 0.25);
        assert_eq!((b.x, b.y, b.w, b.h), (20.0, 30.0, 20.0, 20.0));
    }
}
