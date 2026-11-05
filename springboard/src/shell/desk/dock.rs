//! The Dock: its tile press areas, the dot under an icon, and drawing the capsule, icons and hover label.

use super::*;

/// The press areas of the Dock's tiles (`rects`, laid out along the Dock's axis, `vertical` for a side Dock): each
/// icon grown along the axis by half the gap between two icons, so a press between two tiles opens the nearer one.
/// The pointer resting in that gap magnifies both neighbours alike, and a press there opened nothing (E2E N5: the Files
/// icon "did nothing" twice). The wider gap at a divider keeps a dead strip in its middle.
pub(in crate::shell) fn dock_press_rects(rects: &[Rect], vertical: bool) -> Vec<Rect> {
    let gap = rects
        .windows(2)
        .map(|p| if vertical { p[1].y - (p[0].y + p[0].h) } else { p[1].x - (p[0].x + p[0].w) })
        .filter(|g| *g > 0)
        .min()
        .unwrap_or(0);
    let half = (gap + 1) / 2;
    rects.iter().map(|r| if vertical { Rect { y: r.y - half, h: r.h + 2 * half, ..*r } } else { Rect { x: r.x - half, w: r.w + 2 * half, ..*r } }).collect()
}

/// Whether the release of a Dock press at (`x`, `y`) opens the pressed icon: the icon as it was hit at the press
/// (`pressed`) decides, so a row that re-centred under a still pointer cannot drop the click or move it to a neighbour
/// (W2-23); the slot's current rect counts too while it still holds the same entry (`now`, `now_entry`).
pub(in crate::shell) fn dock_release_hits(pressed: Rect, now: Option<Rect>, now_entry: Option<usize>, entry: usize, x: f32, y: f32) -> bool {
    pressed.contains(x, y) || (now_entry == Some(entry) && now.is_some_and(|r| r.contains(x, y)))
}

/// The dot under a dock icon: white for an app with a window, systemGray for an app whose process still runs
/// with no window (as Golden Gate shows a quit app that is still running), none for an app that is not running.
pub(in crate::shell) fn dock_dot(has_window: bool, background: bool) -> Option<u32> {
    if has_window {
        Some(WHITE)
    } else if background {
        Some(SECONDARY)
    } else {
        None
    }
}

impl Shell {
    /// Records the Dock capsule this frame pushed as a glass shape (`None`: none was). A capsule that differs from the
    /// last one is kept in `dock_glass_vacated`: the presenter drops a stored glass shape only when redrawn pixels wholly
    /// cover it, and a Dock that lost an icon or hid changes just a few rows of its old area (the middle of the plate
    /// comes out the same), so the old, wider shape stayed as a pale slab at both ends.
    pub(in crate::shell) fn note_dock_glass(&self, bar: Option<Rect>) {
        let now = bar.map(|b| (b.x, b.y, b.w, b.h));
        if let Some(prev) = self.dock_glass_bar.replace(now).filter(|p| Some(*p) != now) {
            self.dock_glass_vacated.borrow_mut().push(prev);
        }
    }

    pub(in crate::shell) fn draw_desk_dock(&self, s: &mut Surface, m: &Metrics, t: f64) {
        if !self.dock_is_visible(){self.note_dock_glass(None);return;}
        let (list, bar0, rects0) = self.desk_dock(m);
        // autohide slide, then magnification around the pointer (both pure functions of the clock)
        let (bar, slid) = self.dock_slid(bar0, &rects0, t / 1000.0);
        let rects = self.dock_magnified(&slid, t / 1000.0);
        // the plate grows along the axis with the magnified row, so the end icons stay on it (W2-24)
        let bar = crate::shell::desktop_dock::dock_plate(bar, &slid, &rects, self.dock_preferences().position != crate::shell::desktop_dock::Position::Bottom);
        self.note_dock_glass(bifrost_render::glass::gpu().then_some(bar));
        let ui = &self.ui;
        // frosted material: blur what is behind the dock, tint it, and edge it with a hairline
        let (bx, by) = (bar.x.max(0) as usize, bar.y.max(0) as usize);
        let (bw, bh) = ((bar.w as usize).min(s.w.saturating_sub(bx)), (bar.h as usize).min(s.h.saturating_sub(by)));
        if bw > 0 && bh > 0 && bifrost_render::appearance::reduce_transparency() {
            // Reduce Transparency: an opaque plate, nothing blurred or seen through it; Increase Contrast outlines it
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            soft_shadow(s, ui, bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad);
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, crate::shell::ui::DOCK_SOLID.now());
            match bifrost_render::ui::material_border() {
                Some(b) => s.stroke_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, ui.pt(1.0).max(1.0), b),
                None => dock_rim(s, bar, rad),
            }
        } else if bw > 0 && bh > 0 && bifrost_render::glass::gpu() {
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            bifrost_render::glass::push(crate::shell::control_center::shell_glass_rect(&bifrost_ui::material_request::ShellSurface::Dock.request((bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32), rad)));
            soft_shadow(s, ui, bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad);
            s.fill_rrect(bar.x as f32 - 1.0, bar.y as f32 - 1.0, bar.w as f32 + 2.0, bar.h as f32 + 2.0, rad + 1.0, rgba(255, 255, 255, 36));
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, bifrost_render::glass::tint(crate::shell::ui::DOCK_TINT.now()));
            dock_rim(s, bar, rad);
        } else if bw > 0 && bh > 0 {
            let mut img = Image::new(bw, bh);
            for y in 0..bh {
                let from = (by + y) * s.stride + bx;
                img.px[y * bw..(y + 1) * bw].copy_from_slice(&s.px[from..from + bw]);
            }
            img.blur_material(ui.pt(10.0) as usize);
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            // clip the blurred patch to the capsule by drawing it, then the tint over it
            let mut patch = Surface { w: s.w, h: s.h, stride: s.stride, px: s.px, y0: s.y0 };
            patch.blit(&img, bx as i32, by as i32, 255, 255);
            s.fill_rrect(bar.x as f32 - 1.0, bar.y as f32 - 1.0, bar.w as f32 + 2.0, bar.h as f32 + 2.0, rad + 1.0, rgba(255, 255, 255, 36));
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, crate::shell::ui::DOCK_TINT.now());
            dock_rim(s, bar, rad);
        }
        // dividers: pinned | recents | minimized windows and the Trash
        for k in self.dock_seps(&list) {
            // placed between the icons as drawn (magnified), at the resting icons' depth
            let (a, b) = if slid[k].y == slid[k + 1].y {
                (Rect { x: rects[k].x, w: rects[k].w, ..slid[k] }, Rect { x: rects[k + 1].x, ..slid[k + 1] })
            } else {
                (Rect { y: rects[k].y, h: rects[k].h, ..slid[k] }, Rect { y: rects[k + 1].y, ..slid[k + 1] })
            };
            // a hairline of the Dock's own divider colour: the separator grey vanished into the dark glass
            let t = ui.pt(1.0).round().max(1.0);
            if a.y == b.y {
                let h = (a.h as f32 * 0.6).round();
                s.fill_rrect((((a.x + a.w + b.x) / 2) as f32 - t / 2.0).round().max(0.0), a.y as f32 + (a.h as f32 - h) / 2.0, t, h, 0.0, crate::shell::ui::DOCK_DIVIDER.now());
            } else {
                let w = (a.w as f32 * 0.6).round();
                s.fill_rrect(a.x as f32 + (a.w as f32 - w) / 2.0, (((a.y + a.h + b.y) / 2) as f32 - t / 2.0).round().max(0.0), w, t, 0.0, crate::shell::ui::DOCK_DIVIDER.now());
            }
        }
        let desk = self.desk.as_ref().unwrap();
        let lifted = match desk.grab { Some(Grab::DockDrag { from, moved: true, .. }) => Some(from), _ => None };
        let slide = desk.dock_slide.borrow();
        for (k, r) in rects.iter().enumerate() {
            let i = list[k];
            if lifted == Some(k) { continue; } // its slot stays empty; the icon follows the pointer
            let (ox, oy) = slide.offset(k, t / 1000.0);
            let (bx, by) = self.dock_bounce_offset(i, t / 1000.0, r.w);
            let r = &Rect { x: r.x + ox.round() as i32 + bx, y: r.y + oy.round() as i32 + by, ..*r };
            if i == DOCK_TRASH { draw_trash(s, r); continue; }
            if i == crate::shell::dock_stack::DOCK_STACK { crate::shell::dock_stack::draw_stack(s, r, &desk.stack_files); continue; }
            if let Some(wi) = crate::shell::desktop_dock::dock_min_index(i) {
                // a minimized window, to the right of the last divider: a thumbnail frame at the window's aspect with
                // the window's own picture in it (a capture taken as it was minimized) or, without one, its app's icon, and a small badge of the app
                if let Some(win) = desk.wins.get(wi) {
                    let frame = thumbnail_frame(*r, win.r.w, win.r.h);
                    let (fx, fy, fw, fh) = (frame.x as f32, frame.y as f32, frame.w as f32, frame.h as f32);
                    let radius = ui.pt(MINIMIZED_THUMB_RADIUS_PT).min(fw.min(fh) / 2.0);
                    s.fill_rrect(fx, fy, fw, fh, radius, SEPARATOR_C.now());
                    let hair = ui.pt(1.0).max(1.0);
                    s.fill_rrect(fx + hair, fy + hair, (fw - 2.0 * hair).max(1.0), (fh - 2.0 * hair).max(1.0), (radius - hair).max(0.0), FIELD_BG.now());
                    // the window's own picture fills the frame when the window server gave one; else the app's icon is in it
                    let picture = self.dock_picture(win.app);
                    if let Some(img) = picture {
                        let inset = hair.round() as i32;
                        let inner = Rect { x: frame.x + inset, y: frame.y + inset, w: (frame.w - 2 * inset).max(1), h: (frame.h - 2 * inset).max(1) };
                        let inner = crate::layout::Rect { x: inner.x, y: inner.y, w: inner.w, h: inner.h };
                        crate::shell::switcher::draw_scaled(s, img, inner, (radius - hair).max(0.0), 255);
                    }
                    if let Some(ai) = apps().iter().position(|a| a.id == win.app) {
                        let base = &self.base_icons[ai];
                        let key = |size: usize| DockIconKey { owner: DockIconOwner::App(ai), generation: self.icon_rebuilds, size };
                        if picture.is_none() {
                            let body = ((frame.w.min(frame.h) as f32) * 0.6).round().max(1.0) as usize;
                            desk.with_scaled_icon(key(body), base, body, body, |icon| {
                                s.blit(icon, frame.x + (frame.w - body as i32) / 2, frame.y + (frame.h - body as i32) / 2, 255, 255);
                            });
                        }
                        let badge = ((r.w as f32) * MINIMIZED_BADGE_K).round().max(1.0) as usize;
                        desk.with_scaled_icon(key(badge), base, badge, badge, |icon| {
                            s.blit(icon, frame.x + frame.w - badge as i32 + badge as i32 / 4, frame.y + frame.h - badge as i32 + badge as i32 / 4, 255, 255);
                        });
                    }
                }
                continue;
            }
            if i == apps().len() {
                s.fill_rrect(r.x as f32,r.y as f32,r.w as f32,r.h as f32,ui.pt(10.0),crate::shell::ui::CELL);
                let dot = ui.pt(8.0); let gap = ui.pt(6.0);
                for y in 0..3 { for x in 0..3 {
                    s.fill_rrect(r.x as f32+ui.pt(6.0)+(dot+gap)*x as f32,r.y as f32+ui.pt(6.0)+(dot+gap)*y as f32,dot,dot,ui.pt(2.0),WHITE);
                }}
                continue;
            }
            #[cfg(feature = "desktop-wayland")]
            if i > apps().len() {
                let app=&desk.linux_apps[i-apps().len()-1];
                let pressed=matches!(desk.grab,Some(Grab::DockDrag{from:g,..}) if g==k);
                let key=DockIconKey{owner:DockIconOwner::Linux(app.app_id.clone()),generation:self.icon_rebuilds,size:r.w as usize};
                desk.with_scaled_icon(key,&app.icon,r.w as usize,r.h as usize,|icon|s.blit(icon,r.x,r.y,255,if pressed {150} else {255}));
                if !app.windows.is_empty() {
                    let d=ui.pt(4.0);
                    s.fill_rrect((r.x+r.w/2) as f32-d/2.0,(r.y+r.h) as f32+ui.pt(3.0),d,d,d/2.0,WHITE);
                }
                continue;
            }
            let base = &self.base_icons[i];
            let pressed = matches!(desk.grab, Some(Grab::DockDrag{from:g,..}) if g == k);
            if base.w as i32 == r.w {
                s.blit(base, r.x, r.y, 255, if pressed { 150 } else { 255 });
            } else {
                let key = DockIconKey { owner: DockIconOwner::App(i), generation: self.icon_rebuilds, size: r.w as usize };
                desk.with_scaled_icon(key, base, r.w as usize, r.h as usize, |icon| s.blit(icon, r.x, r.y, 255, if pressed { 150 } else { 255 }));
            }
            if apps()[i].icon == CALENDAR_TILE {
                self.calendar_tile_face(s, ui.s, (r.x as f32, r.y as f32, r.w as f32, r.h as f32), (self.info.wday, self.info.mday), 255);
            }
            let id = apps()[i].id;
            #[cfg(feature = "desktop-wayland")]
            let native_running=if let AppId::External(slot)=id {self.foreign_running(slot as usize)} else {false};
            #[cfg(not(feature = "desktop-wayland"))]
            let native_running=false;
            let background = matches!(id, AppId::External(slot) if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running()));
            if let Some(c) = dock_dot(desk.index_of(id).is_some() || native_running, background) {
                let d = ui.pt(4.0);
                s.fill_rrect((r.x + r.w / 2) as f32 - d / 2.0, (r.y + r.h) as f32 + ui.pt(3.0), d, d, d / 2.0, c);
            }
            if let AppId::External(slot) = id {
                if let Some(h) = self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h| !h.badge.is_empty()) {
                    self.label(&h.badge, 11.0, Weight::Semibold, WHITE, |im| {
                        let (bh, pad) = (ui.pt(16.0), ui.pt(5.0));
                        let bw = (im.w as f32 + 2.0 * pad).max(bh);
                        let (bx, by) = ((r.x + r.w) as f32 + ui.pt(4.0) - bw, r.y as f32 - ui.pt(4.0));
                        s.fill_rrect(bx, by, bw, bh, bh / 2.0, SYSTEM_RED);
                        s.blit(im, (bx + (bw - im.w as f32) / 2.0) as i32, (by + (bh - im.h as f32) / 2.0) as i32, 255, 255);
                    });
                }
            }
            self.dock_tile_progress(s, i, r, t);
        }
        // the hover label fades in and out (Reduce Motion: it just appears); an open Dock menu or stack hides it, as the
        // reference does, so it never shows through the stack (E2E8 7)
        let (label_slot, label_alpha) = self.dock_label_state(t);
        if let Some(k) = label_slot.filter(|_| desk.grab.is_none() && desk.dock_label_off.is_none() && !self.dock_label_hidden_by_menu()) {
            if let (Some(r), Some(&i)) = (rects.get(k), list.get(k)) {
                let name = self.dock_label_text(i);
                if !name.is_empty() && label_alpha > 0.0 {
                    self.draw_dock_label(s, &name, *r, self.dock_preferences().position, label_alpha, MENU_TINT.now(), LABEL.now());
                }
            }
        }
        if let Some(Grab::DockDrag{from,at_x,at_y,moved:true,..})=desk.grab {if let Some(&index)=list.get(from){if let Some(base)=self.base_icons.get(index){let size=rects[from].w as usize;let icon=base.resized(size,size);s.blit(&icon,at_x as i32-size as i32/2,at_y as i32-size as i32/2,255,210);}}}
    }

    /// The Dock's hover label: `name` on the menu material beside the icon slot `r`, faded by `alpha` (0..1). The text is
    /// rendered white and tinted with `ink` by `blit_colored`, so it follows Light and Dark: a plain white blit read on Dark's
    /// plate but left an empty pill on Light's pale one. `plate` and `ink` are the resolved `MENU_TINT` and `LABEL`.
    pub(in crate::shell) fn draw_dock_label(&self, s: &mut Surface, name: &str, r: Rect, pos: crate::shell::desktop_dock::Position, alpha: f32, plate: u32, ink: u32) {
        let ui = &self.ui;
        let a = (alpha * 255.0).round() as u32;
        let px = ui.pt(crate::shell::desktop_dock::DOCK_LABEL_PAD_X_PT);
        let gap = ui.pt(crate::shell::desktop_dock::DOCK_LABEL_GAP_PT);
        let room = match pos {
            crate::shell::desktop_dock::Position::Bottom => s.w as f32 - 2.0 * px,
            crate::shell::desktop_dock::Position::Left => s.w as f32 - (r.x + r.w) as f32 - gap - 2.0 * px,
            crate::shell::desktop_dock::Position::Right => r.x as f32 - gap - 2.0 * px,
        };
        let name = ui.fit(name, MAC_CALLOUT_PT, ui.pt(crate::shell::desktop_dock::DOCK_LABEL_MAX_WIDTH_PT).min(room.max(1.0)));
        self.label(&name, MAC_CALLOUT_PT, Weight::Regular, WHITE, |im| {
            let (px, py) = (ui.pt(crate::shell::desktop_dock::DOCK_LABEL_PAD_X_PT), ui.pt(crate::shell::desktop_dock::DOCK_LABEL_PAD_Y_PT));
            let (w, h) = (im.w as f32 + 2.0 * px, im.h as f32 + 2.0 * py);
            let (x, y) = match pos {
                crate::shell::desktop_dock::Position::Bottom => (((r.x + r.w / 2) as f32 - w / 2.0).clamp(0.0, (s.w as f32 - w).max(0.0)), r.y as f32 - h - ui.pt(crate::shell::desktop_dock::DOCK_LABEL_GAP_PT)),
                crate::shell::desktop_dock::Position::Left => ((r.x + r.w) as f32 + ui.pt(crate::shell::desktop_dock::DOCK_LABEL_GAP_PT), (r.y + r.h / 2) as f32 - h / 2.0),
                crate::shell::desktop_dock::Position::Right => (r.x as f32 - w - ui.pt(crate::shell::desktop_dock::DOCK_LABEL_GAP_PT), (r.y + r.h / 2) as f32 - h / 2.0),
            };
            s.fill_rrect(x, y, w, h, ui.pt(MENU_RADIUS_PT), crate::shell::desktop_dock::fade(plate, alpha));
            s.blit_colored(im, (x + px) as i32, (y + py) as i32, ink, a);
        });
    }
}
