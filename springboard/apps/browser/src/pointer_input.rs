//! Primary pointer and touch handling.
use super::*;

impl Browser {
    pub(super) fn on_pointer(&mut self, ctx: &Ctx, event: &bifrost_app::pointer_events::PointerEvent) -> bool {
        use bifrost_app::pointer_events::PointerEvent;
        if self.main_menu.open() {
            let anchor = self.main_menu_anchor(&ctx.ui, ctx.cfg.w);
            match *event {
                PointerEvent::Hover { x, y } => self.main_menu.hover(&ctx.ui, ctx.cfg.w, ctx.cfg.h, anchor, x, y),
                PointerEvent::Wheel { dy, .. } => self.main_menu.wheel(&ctx.ui, ctx.cfg.w, ctx.cfg.h, anchor, dy),
                _ => {}
            }
            return true;
        }
        if self.sheet.open || self.overview.open || self.pageui.active() || self.lib.open || self.downloads.open {
            return false;
        }
        match *event {
            PointerEvent::Hover { x, y } => {
                let previous = self.hover;
                self.hover = Some((x, y));
                self.update_pointer(ctx, x, y);
                if let Some(pool) = self.pool.as_mut() {
                    if pool.contains(x, y) {
                        pool.mouse(TouchKind::Move, x, y);
                    } else if previous.is_some_and(|(x, y)| pool.contains(x, y)) {
                        pool.mouse_leave();
                    }
                }
                true
            }
            PointerEvent::HoverExit => {
                self.hover = None;
                if let Some(pool) = self.pool.as_mut() { pool.mouse_leave(); }
                true
            }
            PointerEvent::Wheel { x, y, dx, dy, .. } => {
                if self.start_wheel(ctx, x, y, dy) {
                    return true;
                }
                if let Some(pool) = self.pool.as_mut().filter(|p| p.contains(x, y)) {
                    pool.scroll(dx, dy, x, y);
                }
                false
            }
            _ => false,
        }
    }

    pub(super) fn on_touch(&mut self, ctx: &Ctx, kind: TouchKind, id: u32, x: f32, y: f32) -> bool {
        let ui = &ctx.ui;
        self.hover = Some((x, y));
        let view = self.view();
        if self.inspector.touch(ctx, kind, x, y, view, self.pool.as_mut()) {
            if self.inspector.take_relayout() {
                self.place_web(ctx);
            }
            return true;
        }
        if kind == TouchKind::Down && self.downloads.open {
            let wpt = ctx.cfg.w as f32 / ui.pt(1.0);
            match self.downloads.hit(wpt, x / ui.pt(1.0), y / ui.pt(1.0)) {
                Some(a) => {
                    match a {
                        downloads::Action::Cancel(i) => {
                            self.downloads.cancel(i);
                        }
                        downloads::Action::Reveal(i) => self.downloads.reveal(i),
                        downloads::Action::Open(i) => self.downloads.open_file(i),
                        downloads::Action::Clear => self.downloads.clear_finished(),
                        downloads::Action::Inside => {}
                    }
                    return true;
                }
                None => {
                    if self.chrome_hit(ui, ctx.cfg.w, x, y) != Some(Press::Bar(DOWNLOADS)) {
                        self.downloads.open = false;
                    }
                }
            }
        }
        if self.start_touch(ctx, kind, x, y) {
            return true;
        }
        if kind == TouchKind::Down {
            self.inspector.blur();
            let hit = self.chrome_hit(ui, ctx.cfg.w, x, y);
            self.press = hit.map(|h| (h, x, y));
            self.drag = false;
            self.swallow = false;
            self.long = (hit == Some(Press::Web) && bifrost_app::controls::form_factor::FormFactor::current().is_tablet())
                .then(|| (self.clock.elapsed().as_secs_f64(), x, y, id));
            if hit == Some(Press::Web) {
                self.pageui.last_point = (x, y);
                self.editing = false;
                self.sync_address();
            }
        }
        let Some((press, x0, y0)) = self.press else { return false };
        if press == Press::Web {
            let up = matches!(kind, TouchKind::Up | TouchKind::Cancel);
            if self.swallow {
                if up {
                    self.swallow = false;
                    self.press = None;
                }
                return true;
            }
            let slop = ui.pt(TAP_SLOP_PT);
            if (x - x0).abs() > slop || (y - y0).abs() > slop {
                self.long = None;
            }
            // a held, unmoved finger lifting after the long-press time is also a long press
            if kind == TouchKind::Up && self.long.is_some_and(|(t0, ..)| self.clock.elapsed().as_secs_f64() - t0 >= LONG_PRESS_S) {
                self.fire_long_press();
                self.swallow = false;
                self.press = None;
                return true;
            }
            if up {
                self.long = None;
            }
            if let Some(p) = &mut self.pool {
                if bifrost_app::controls::form_factor::FormFactor::current().is_desktop() {
                    p.mouse(kind, x, y);
                } else {
                    p.touch(kind, id, x, y);
                }
            }
            if up {
                self.press = None;
            }
            return false;
        }
        if let (Press::Tab(i), TouchKind::Move) = (press, kind) {
            // dragging a tab reorders the strip (Browser); the dragged tab becomes the selected one
            if self.drag || (x - x0).abs() > ui.pt(TAP_SLOP_PT) {
                if !self.drag {
                    self.drag = true;
                    self.select(i);
                }
                if let Some(to) = self.tab_at(ui, ctx.cfg.w, x).filter(|&to| to != i) {
                    self.move_tab(i, to);
                    self.press = Some((Press::Tab(to), x0, y0));
                }
                return true;
            }
        }
        if matches!(kind, TouchKind::Up) {
            self.press = None;
            if std::mem::take(&mut self.drag) {
                return true;
            }
            let slop = ui.pt(TAP_SLOP_PT);
            if (x - x0).abs() > slop || (y - y0).abs() > slop {
                return true;
            }
            let layout_before = (self.strip_shown(), self.find.open);
            match press {
                Press::Bar(id) => self.activate_in(ctx, id),
                Press::Tab(i) => self.select(i),
                Press::TabClose(i) => self.close_tab(i),
                Press::Sugg(i) => {
                    self.sel = Some(i);
                    self.commit_address();
                }
                Press::BannerClose => self.banner_activate(crashbanner::BannerHit::Close),
                Press::BannerReload => self.banner_activate(crashbanner::BannerHit::Reload),
                Press::Web => {}
            }
            // the tab strip or the find bar came or went: the page moves
            if layout_before != (self.strip_shown(), self.find.open) {
                self.place_web(ctx);
            }
        } else if kind == TouchKind::Cancel {
            self.press = None;
            self.drag = false;
        }
        true
    }
}
