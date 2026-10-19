//! Browser window drawing.
use super::*;

impl Browser {
    pub(super) fn draw_window(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        let wpt = w as f32 / Self::pt(ui);
        let has_full_web = self.pool.as_ref().is_some_and(|p| p.has_frame()) && self.dev.responsive.is_none() && self.error.is_none();
        if !has_full_web {
            s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        }
        if let Some(p) = &mut self.pool {
            p.draw(s);
        }
        self.draw_start_page(ctx, s);
        self.inspector.draw(ctx, s, self.view());
        self.draw_page_feedback(ctx, s);
        if let (Some(r), Some(text)) = (&self.dev.responsive, self.dev.readout()) {
            let _ = r;
            let img = ui.text(&text, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, (w as i32 - img.w as i32) / 2, ui.pt(self.content_top() + 2.0) as i32, 255, 255);
        }
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if devmenu::not_secure(&st.url) {
                let img = ui.text("Not Secure", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
                s.blit(&img, w as i32 - img.w as i32 - ui.pt(SPACE_S) as i32, ui.pt(UNIFIED_BAR_PT + 2.0) as i32, 255, 255);
            }
        }
        if let Some(msg) = &self.error {
            let img = ui.text(&ui.fit(msg, MAC_BODY_PT, w as f32 * 0.8), MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, (w as i32 - img.w as i32) / 2, (h as i32) / 2, 255, 255);
        }

        // the unified toolbar band
        widgets::toolbar_band(s, ui, Rect { x: 0.0, y: 0.0, w: wpt, h: UNIFIED_BAR_PT });
        if self.private {
            // a private window has a visibly dark toolbar (Browser)
            s.fill_rrect(0.0, 0.0, s.w as f32, ui.pt(UNIFIED_BAR_PT), 0.0, tokens::scrim(140));
        }
        let pressed = match self.press {
            Some((Press::Bar(id), _, _)) => Some(id),
            _ => None,
        };
        let bar = self.toolbar();
        self.bar_layout = Some(widgets::toolbar(s, ui, Self::bar_bounds(ui, w), &bar, pressed));
        let field = self.address_rect(ui, w);
        let placeholder = if self.private { "Private Browsing" } else { "Search or enter website name" };
        if self.editing || self.address.is_empty() {
            widgets::search_field(s, ui, &self.address, field, placeholder, self.editing);
        } else {
            // not editing: only the host, centred (Browser); the full URL returns on focus
            widgets::search_field(s, ui, &TextField::new(), field, "", false);
            let url = self.address.to_string();
            let host = host_of(&url);
            // site icon left of the host, the pair centred
            let icon_px = ui.pt(favicon::ICON_PT).round() as usize;
            let gap = ui.pt(favicon::GAP_PT) as i32;
            let has_icon = favicon::host_key(&url).is_some_and(|h| self.favicons.get(&h, icon_px).is_some());
            let lead = if has_icon { icon_px as i32 + gap } else { 0 };
            let room = (ui.pt(field.w - field.h * 2.0) - lead as f32).max(0.0);
            let shown = ui.fit(&host, MAC_BODY_PT, room);
            let img = ui.text(&shown, MAC_BODY_PT, Weight::Regular, LABEL.now());
            // measure with ui.measure: the rendered image carries side padding
            let tw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular) as i32;
            let (cx, cy) = (ui.pt(field.x + field.w / 2.0) as i32, ui.pt(field.y + field.h / 2.0) as i32);
            let left = cx - (lead + tw) / 2;
            if let Some(ic) = favicon::host_key(&url).and_then(|h| self.favicons.get(&h, icon_px)) {
                s.blit(ic, left, cy - icon_px as i32 / 2, 255, 255);
            }
            s.blit(&img, left + lead + (tw - img.w as i32) / 2, cy - img.h as i32 / 2, 255, 255);
        }
        // Reader button at the leading edge when the page has an article (Browser)
        if let Some(v) = self.view().filter(|&v| !self.editing && self.reader.button(v)) {
            let r = reader::button_rect(field);
            glyphs::glyph(s, ui, "doc", Rect { x: r.x + r.w * 0.2, y: r.y + r.h * 0.2, w: r.w * 0.6, h: r.h * 0.6 }, if self.reader.is_active(v) { TINT } else { LABEL_2.now() });
        }
        // the star at the field's trailing edge: filled for a bookmarked page, an outline otherwise (Browser)
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if !self.editing {
                let marked = self.store.is_bookmarked(&st.url);
                let (name, color) = if marked { ("star.fill", TINT) } else { ("star", LABEL_2.now()) };
                glyphs::glyph(s, ui, name, star_rect(field), color);
            }
        }
        // load progress along the bottom of the address field (Browser)
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if st.loading {
                let (x, y) = (ui.pt(field.x + field.h / 2.0), ui.pt(field.y + field.h) - ui.pt(SPACE_XXS));
                let len = (ui.pt(field.w - field.h) * st.progress.clamp(0.0, 1.0)) as usize;
                s.fill_rect(x as usize, y as usize, len, ui.pt(SPACE_XXS).max(1.0) as usize, TINT);
            }
        }

        self.draw_find(s, ui, wpt);
        self.draw_banner(s, ui, wpt);

        // tabs, only when there are two or more (Browser)
        if self.strip_shown() {
            let top = ui.pt(UNIFIED_BAR_PT) as usize;
            let sh = ui.pt(TAB_STRIP_PT).ceil() as usize;
            s.fill_rect(0, top, s.w, sh, CARD_BG.now());
            s.fill_rect(0, top + sh - 1, s.w, 1, SEPARATOR_C.now());
            for (i, r) in self.tab_rects(ui, w).into_iter().enumerate() {
                let selected = i == self.current;
                let pressed = matches!(self.press, Some((Press::Tab(j), _, _)) if j == i);
                if selected || pressed {
                    let fill = if selected { SEGMENT_ON.now() } else { ROW_PRESSED.now() };
                    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(m::SELECTION_RADIUS), fill);
                } else if i + 1 != self.current && i + 1 < self.tabs.len() {
                    s.fill_rect(ui.pt(r.x + r.w + 2.0) as usize, ui.pt(r.y + 5.0) as usize, 1, ui.pt(r.h - 10.0) as usize, SEPARATOR_C.now());
                }
                let hovered = self.hover.is_some_and(|(hx, hy)| r.contains(hx / ui.pt(1.0), hy / ui.pt(1.0)));
                let show_x = selected || hovered;
                if show_x {
                    glyphs::glyph(s, ui, "xmark", Self::close_rect(r), LABEL_2.now());
                }
                let st = self.pool.as_ref().and_then(|p| p.state(self.tabs.get(i)?.view));
                let title = st.map(|s| if s.title.is_empty() { s.url.clone() } else { s.title.clone() }).unwrap_or_default();
                let title = if title.is_empty() || title == START { "New Tab".to_string() } else { title };
                // site icon (16 pt) left of the title; the pair is centred (Browser)
                let icon_px = ui.pt(favicon::ICON_PT).round() as usize;
                let gap = ui.pt(favicon::GAP_PT) as i32;
                let host = st.and_then(|s| favicon::host_key(&s.url));
                let has_icon = host.as_deref().is_some_and(|h| self.favicons.get(h, icon_px).is_some());
                let lead = if has_icon { icon_px as i32 + gap } else { 0 };
                let room = ui.pt(r.w - if show_x { 44.0 } else { 16.0 }).max(0.0) - lead as f32;
                let weight = if selected { Weight::Semibold } else { Weight::Regular };
                let img = ui.text(&ui.fit(&title, MAC_SUBHEAD_PT, room.max(0.0)), MAC_SUBHEAD_PT, weight, if selected { LABEL.now() } else { LABEL_2.now() });
                let cx = ui.pt(r.x + r.w / 2.0) as i32;
                let cy = ui.pt(r.y + r.h / 2.0) as i32;
                // measure with ui.measure: the rendered image carries side padding
                let tw = ui.measure(&ui.fit(&title, MAC_SUBHEAD_PT, room.max(0.0)), MAC_SUBHEAD_PT, weight) as i32;
                let left = cx - (lead + tw) / 2;
                if let Some(ic) = host.as_deref().filter(|_| has_icon).and_then(|h| self.favicons.get(h, icon_px)) {
                    s.blit(ic, left, cy - icon_px as i32 / 2, 255, 255);
                }
                s.blit(&img, left + lead + (tw - img.w as i32) / 2, cy - img.h as i32 / 2, 255, 255);
            }
        }
        self.lib.draw_bar(s, ui, wpt, self.fav_top(), &self.store);
        if self.editing {
            suggest::draw(s, ui, field, &self.sugg, self.sel, &mut self.favicons);
        }
        if self.overview.open {
            let tabs: Vec<(String, String)> = self
                .tabs
                .iter()
                .map(|t| {
                    let st = self.pool.as_ref().and_then(|p| p.state(t.view));
                    (st.map(|s| s.title.clone()).unwrap_or_default(), st.map(|s| s.url.clone()).unwrap_or_default())
                })
                .collect();
            self.overview.draw(s, ui, wpt, self.content_top(), &tabs, self.current);
        }
        let anchor = self.bar_layout.as_ref().and_then(|l| l.rect_of(DOWNLOADS)).map(|r| r.x + r.w / 2.0);
        self.downloads.draw(s, ui, wpt, anchor);
        self.lib.draw_overlays(s, ui, w, h, self.fav_top(), &self.store, store::now());
        self.pageui.draw(s, ui, w, h);
        self.sheet.draw(s, ui, w, h, &self.prefs);
        self.main_menu.draw(s, ui, w, h, self.main_menu_anchor(ui, w));
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    /// Servo reports the hovered destination. Keep it outside page content styling and cover it
    /// with browser sheets/menus; no DOM script or layout query is needed on pointer motion.
    fn draw_page_feedback(&self, ctx: &Ctx, s: &mut Surface) {
        if self.editing || self.sheet.open || self.pageui.active() || self.main_menu.open() || self.overview.open || self.lib.open || self.downloads.open { return; }
        let Some(pool) = &self.pool else { return };
        let status = self.view().and_then(|view| pool.state(view)).map(|state| state.status_text.as_str()).unwrap_or("");
        let over_page = self.hover.is_some_and(|(x, y)| pool.contains(x, y));
        let message = self.uploads.error.as_deref().or_else(|| self.uploads.copying().then_some("Preparing files…"))
            .or_else(|| (over_page && !status.is_empty()).then_some(status));
        let Some(message) = message else { return };
        let ui = &ctx.ui;
        let (x, y, w, h) = pool.frame();
        let pad = ui.pt(SPACE_S);
        let room = (w as f32 * 0.8 - 2.0 * pad).max(0.0);
        if room < pad || h as f32 <= 4.0 * pad { return; }
        let text = ui.fit(message, MAC_SUBHEAD_PT, room);
        let image = ui.text(&text, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now());
        let width = (image.w as f32 + 2.0 * pad).min(w as f32);
        let height = image.h as f32 + 2.0 * pad;
        let bottom = y as f32 + h as f32;
        s.fill_rrect(x as f32, bottom - height, width, height, ui.pt(m::SELECTION_RADIUS), CARD_BG.now());
        s.blit(&image, x + pad as i32, (bottom - height + pad) as i32, 255, 255);
    }
}
