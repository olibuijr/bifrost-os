//! Drawing the chrome, grid, cursor, selection and panels.
use super::*;

impl Shell {
    pub(crate) fn draw_terminal_chrome(&mut self, s: &mut Surface, t: f64) {
        let rect = self.terminal_panel_rect();
        let ui = &self.ui;
        let width = self.w as f32;
        let row = ui.pt(ROW);
        let top = ui.pt(scene::STATUS_PT);
        let body = top + ui.pt(HEADER_PT);
        let g = layout::Header::new(width, ui.s, self.term.len());
        s.fill_rrect(0.0, top, width, row, 0.0, BAR);
        for (i, term) in self.term.iter().enumerate() {
            let x = i as f32 * g.tab_w;
            let selected = i == self.term.active_index();
            let color = if selected { WHITE } else { SECONDARY };
            if selected {
                s.fill_rrect(x, top, g.tab_w, row, 0.0, ACTIVE);
                s.fill_rrect(x, top + row - ui.pt(2.0), g.tab_w, ui.pt(2.0), 0.0, ACCENT);
            }
            let cw = g.close_width();
            text(
                s,
                ui,
                &term.title(i),
                x + ui.pt(12.0),
                top,
                (g.tab_w - cw - ui.pt(24.0)).max(0.0),
                color,
            );
            // A dot on a background tab: orange when it rang the bell, green when it printed.
            if (term.unread || term.bell) && !selected {
                s.fill_rrect(
                    x + g.tab_w - cw - ui.pt(10.0),
                    top + row / 2.0 - ui.pt(3.0),
                    ui.pt(6.0),
                    ui.pt(6.0),
                    ui.pt(3.0),
                    if term.bell { SYSTEM_ORANGE } else { ACCENT },
                );
            }
            text(
                s,
                ui,
                "×",
                x + g.tab_w - cw + ui.pt(10.0),
                top,
                (cw - ui.pt(10.0)).max(0.0),
                color,
            );
        }
        button(s, ui, "+", (g.plus, top, ui.pt(44.0), row), false);
        button(
            s,
            ui,
            "SSH",
            (g.ssh, top, ui.pt(52.0), row),
            self.terminal_ui.panel == Panel::Connections,
        );
        let sys = self
            .terminal_ui
            .system_used
            .map_or_else(|| "—".into(), |v| format!("{:.0}% used", v * 100.0));
        let rss = if self.terminal_ui.app_memory == 0 {
            "—".into()
        } else {
            termcore::format::memory(self.terminal_ui.app_memory)
        };
        for (x, w, tag, value, gauge) in [
            (
                g.sys,
                ui.pt(114.0),
                "SYS",
                sys,
                self.terminal_ui.system_used,
            ),
            (g.app, ui.pt(82.0), "APP", rss, None),
        ] {
            let y = top + ui.pt(9.0);
            let h = ui.pt(22.0);
            s.fill_rrect(x, y, w, h, 0.0, BADGE);
            let title = ui.chrome(tag, 10.0, Weight::Medium, SECONDARY);
            s.blit(
                &title,
                (x + ui.pt(6.0)) as i32,
                (y + (h - title.h as f32) / 2.0) as i32,
                255,
                255,
            );
            let val = ui.chrome(&value, 10.0, Weight::Medium, WHITE);
            s.blit(
                &val,
                (x + w - ui.pt(6.0) - val.w as f32) as i32,
                (y + (h - val.h as f32) / 2.0) as i32,
                255,
                255,
            );
            s.fill_rrect(x, y + h - ui.pt(2.0), w, ui.pt(2.0), 0.0, TRACK);
            let color = if gauge.is_some_and(|v| v > 0.9) {
                RED
            } else {
                ACCENT
            };
            s.fill_rrect(
                x,
                y + h - ui.pt(2.0),
                w * gauge.unwrap_or(1.0),
                ui.pt(2.0),
                0.0,
                color,
            );
        }
        for i in 0..3 {
            s.fill_rrect(
                g.more + ui.pt(12.0 + i as f32 * 8.0),
                top + row / 2.0 - ui.pt(2.0),
                ui.pt(4.0),
                ui.pt(4.0),
                ui.pt(2.0),
                WHITE,
            );
        }
        let panel = self.terminal_ui.panel;
        if panel == Panel::None {
            if !self.terminal_ui.message.is_empty() {
                let message = ui.fit(&self.terminal_ui.message, 13.0, width - ui.pt(48.0));
                let image = ui.text(&message, 13.0, Weight::Medium, WHITE);
                let w = (image.w as f32 + ui.pt(24.0)).min(width - ui.pt(24.0));
                let x = (width - w) / 2.0;
                s.fill_rrect(x, body + ui.pt(10.0), w, row, ui.pt(8.0), BADGE);
                s.blit(
                    &image,
                    (x + (w - image.w as f32) / 2.0) as i32,
                    (body + ui.pt(10.0) + (row - image.h as f32) / 2.0) as i32,
                    255,
                    255,
                );
            }
            return;
        }
        if panel == Panel::Find {
            s.fill_rrect(0.0, body, width, row, 0.0, BAR);
            let field_w = width - ui.pt(148.0);
            s.fill_rrect(
                ui.pt(8.0),
                body + ui.pt(5.0),
                field_w,
                ui.pt(30.0),
                ui.pt(6.0),
                BADGE,
            );
            s.stroke_rrect(
                ui.pt(15.0),
                body + ui.pt(13.0),
                ui.pt(10.0),
                ui.pt(10.0),
                ui.pt(5.0),
                ui.pt(1.5),
                SECONDARY,
            );
            for step in 0..5 {
                s.fill_rrect(
                    ui.pt(23.0 + step as f32 * 0.8),
                    body + ui.pt(21.0 + step as f32 * 0.8),
                    ui.pt(1.5),
                    ui.pt(1.5),
                    ui.pt(0.75),
                    SECONDARY,
                );
            }
            if let Some(field) = &self.terminal_ui.query {
                field.draw(s, ui, (ui.pt(34.0), body, field_w - ui.pt(36.0), row), t);
            }
            for (i, caption) in ["↑", "↓", "×"].into_iter().enumerate() {
                button(
                    s,
                    ui,
                    caption,
                    (
                        width - ui.pt(132.0) + i as f32 * ui.pt(44.0),
                        body,
                        ui.pt(44.0),
                        row,
                    ),
                    false,
                );
            }
            s.fill_rrect(0.0, body + row - ui.pt(1.0), width, ui.pt(1.0), 0.0, TRACK);
            if !self.terminal_ui.message.is_empty() {
                text(
                    s,
                    ui,
                    &self.terminal_ui.message,
                    ui.pt(12.0),
                    body + row,
                    width - ui.pt(24.0),
                    if find_count(&self.terminal_ui.message) { SECONDARY } else { RED },
                );
            }
            return;
        }
        let (px, py, pw, ph) = rect;
        let bottom = py + ph;
        let title_h = ui.pt(44.0);
        let footer_h = ui.pt(56.0);
        let content_top = py + title_h;
        let content_bottom = bottom - footer_h;
        // Alpha-blend the scrim over the real terminal, keeping it visible.
        let dim_bottom = if self.keyboard.shown() {
            self.kb_top()
        } else {
            self.h as f32 - super::super::ui::home_zone(ui)
        };
        s.fill_rrect(
            0.0,
            top,
            width,
            dim_bottom - top,
            0.0,
            crate::gfx::rgba(0, 0, 0, 153),
        );
        s.fill_rrect(px, py, pw, ph, 0.0, BODY);
        s.fill_rrect(px, py, pw, title_h, 0.0, BAR);
        s.fill_rrect(px, py + title_h - ui.pt(2.0), pw, ui.pt(2.0), 0.0, ACCENT);
        let caption = match panel {
            Panel::Connections => "SSH Connections",
            Panel::Form => {
                if self.terminal_ui.editing.is_some() {
                    "Edit Connection"
                } else {
                    "New Connection"
                }
            }
            Panel::Processes => "Processes",
            Panel::Command if self.terminal_ui.title_edit => "Edit Title",
            Panel::Command => "New Command",
            Panel::Look => "Profile Editor",
            _ => "Terminal",
        };
        if panel == Panel::Form {
            button(s, ui, "Cancel", (px, py, ui.pt(104.0), title_h), false);
            let title = ui.chrome(caption, 15.0, Weight::Semibold, WHITE);
            s.blit(
                &title,
                (px + (pw - title.w as f32) / 2.0) as i32,
                (py + (title_h - title.h as f32) / 2.0) as i32,
                255,
                255,
            );
            if self.terminal_ui.editing.is_some() {
                button(
                    s,
                    ui,
                    if self.terminal_ui.delete_confirm {
                        "Confirm delete"
                    } else {
                        "Delete"
                    },
                    (px + pw - ui.pt(208.0), py, ui.pt(104.0), title_h),
                    self.terminal_ui.delete_confirm,
                );
            }
            button(
                s,
                ui,
                "Save",
                (px + pw - ui.pt(104.0), py, ui.pt(104.0), title_h),
                true,
            );
        } else {
            let title = ui.chrome(caption, 16.0, Weight::Semibold, WHITE);
            s.blit(
                &title,
                (px + ui.pt(16.0)) as i32,
                (py + (title_h - title.h as f32) / 2.0) as i32,
                255,
                255,
            );
            button(
                s,
                ui,
                if panel == Panel::More { "Done" } else { "×" },
                (
                    px + pw - ui.pt(if panel == Panel::More { 80.0 } else { 44.0 }),
                    py,
                    ui.pt(if panel == Panel::More { 80.0 } else { 44.0 }),
                    title_h,
                ),
                false,
            );
        }
        match panel {
            Panel::Command => {
                if let Some(field) = &self.terminal_ui.command {
                    field.draw(s, ui, (px + ui.pt(16.0), content_top + ui.pt(16.0), pw - ui.pt(32.0), ui.pt(36.0)), t);
                }
            }
            Panel::Connections => {
                button(
                    s,
                    ui,
                    "+ New",
                    (px + pw - ui.pt(136.0), py, ui.pt(92.0), title_h),
                    false,
                );
                let mut list = s.rows(content_top as usize, content_bottom as usize);
                let entries = self.terminal_ui.config.entries();
                for (i, e) in entries.iter().enumerate() {
                    let y = content_top + i as f32 * ui.pt(52.0) - self.terminal_ui.scroll;
                    if y + ui.pt(52.0) < content_top || y >= content_bottom {
                        continue;
                    }
                    let alias = ui.text(
                        &ui.fit(&e.alias, 16.0, pw - ui.pt(110.0)),
                        16.0,
                        Weight::Semibold,
                        WHITE,
                    );
                    list.blit(
                        &alias,
                        (px + ui.pt(16.0)) as i32,
                        (y + ui.pt(6.0)) as i32,
                        255,
                        255,
                    );
                    let summary = ui.text(
                        &ui.fit(&termcore::sshconfig::summary(e), 12.0, pw - ui.pt(110.0)),
                        12.0,
                        Weight::Regular,
                        SECONDARY,
                    );
                    list.blit(
                        &summary,
                        (px + ui.pt(16.0)) as i32,
                        (y + ui.pt(30.0)) as i32,
                        255,
                        255,
                    );
                    button(
                        &mut list,
                        ui,
                        "Edit",
                        (
                            px + pw - ui.pt(76.0),
                            y + ui.pt(8.0),
                            ui.pt(64.0),
                            ui.pt(36.0),
                        ),
                        false,
                    );
                    list.fill_rrect(
                        px + ui.pt(16.0),
                        y + ui.pt(52.0) - 1.0,
                        pw - ui.pt(32.0),
                        1.0,
                        0.0,
                        BADGE,
                    );
                }
                if entries.is_empty() {
                    text(
                        &mut list,
                        ui,
                        "No saved connections",
                        px + ui.pt(20.0),
                        content_top + ui.pt(35.0),
                        pw - ui.pt(40.0),
                        WHITE,
                    );
                    text(
                        &mut list,
                        ui,
                        "Add a host with + New, or import an SSH config.",
                        px + ui.pt(20.0),
                        content_top + ui.pt(75.0),
                        pw - ui.pt(40.0),
                        SECONDARY,
                    );
                }
                let (ix, iy) =
                    termcore::connui::import_origin((pw / ui.s) as f64, (ph / ui.s) as f64);
                button(
                    s,
                    ui,
                    "Import from clipboard",
                    (
                        px + ui.pt(ix as f32),
                        py + ui.pt(iy as f32),
                        ui.pt(200.0),
                        ui.pt(34.0),
                    ),
                    false,
                );
            }
            Panel::Form => {
                let mut content = s.rows(content_top as usize, content_bottom as usize);
                for (i, field) in self.terminal_ui.fields.iter().enumerate() {
                    let (fx, fy, fw, fh) = termcore::connui::field_rect((pw / ui.s) as f64, i);
                    let r = (
                        px + ui.pt(fx as f32),
                        py + ui.pt(fy as f32),
                        ui.pt(fw as f32),
                        ui.pt(fh as f32),
                    );
                    text(
                        &mut content,
                        ui,
                        LABELS[i],
                        px + ui.pt(16.0),
                        r.1,
                        ui.pt(102.0),
                        SECONDARY,
                    );
                    content.fill_rrect(r.0, r.1, r.2, r.3, ui.pt(4.0), BADGE);
                    if field.focused {
                        content.stroke_rrect(r.0, r.1, r.2, r.3, ui.pt(4.0), ui.pt(1.0), ACCENT);
                    }
                    field.draw(
                        &mut content,
                        ui,
                        (r.0 + ui.pt(8.0), r.1, r.2 - ui.pt(16.0), r.3),
                        t,
                    );
                }
            }
            Panel::Processes => {
                for (offset, label, selected) in [
                    (136.0, "Memory", !self.terminal_ui.cpu_sort),
                    (218.0, "CPU", self.terminal_ui.cpu_sort),
                ] {
                    button(
                        s,
                        ui,
                        label,
                        (px + ui.pt(offset), py, ui.pt(82.0), title_h),
                        selected,
                    );
                }
                let total: u64 = self
                    .terminal_ui
                    .processes
                    .iter()
                    .map(|p| p.info.footprint)
                    .sum();
                let summary = format!(
                    "{} processes · {}",
                    self.terminal_ui.processes.len(),
                    termcore::format::memory(total)
                );
                text(
                    s,
                    ui,
                    &summary,
                    px + ui.pt(315.0),
                    py,
                    pw - ui.pt(365.0),
                    SECONDARY,
                );
                let order = self.terminal_ui.order();
                let largest = order.first().map_or(1.0, |&i| {
                    if self.terminal_ui.cpu_sort {
                        self.terminal_ui.cpu[i].max(1.0)
                    } else {
                        self.terminal_ui.processes[i].info.footprint.max(1) as f64
                    }
                });
                let mut list = s.rows(content_top as usize, content_bottom as usize);
                for (i, &index) in order.iter().enumerate() {
                    let y = content_top + i as f32 * row - self.terminal_ui.scroll;
                    if y + row < content_top || y >= content_bottom {
                        continue;
                    }
                    let p = &self.terminal_ui.processes[index];
                    let selected = self.terminal_ui.selected == Some((p.info.pid, p.start));
                    if selected {
                        list.fill_rrect(px, y, pw, row, 0.0, pack(34, 43, 39, 255));
                    }
                    let value = if self.terminal_ui.cpu_sort {
                        self.terminal_ui.cpu[index]
                    } else {
                        p.info.footprint as f64
                    };
                    let gw = ui.pt(74.0);
                    let gx = px + pw - ui.pt(90.0);
                    list.fill_rrect(gx, y + row / 2.0 - ui.pt(3.0), gw, ui.pt(6.0), 0.0, TRACK);
                    list.fill_rrect(
                        gx,
                        y + row / 2.0 - ui.pt(3.0),
                        gw * (value / largest).clamp(0.0, 1.0) as f32,
                        ui.pt(6.0),
                        0.0,
                        ACCENT,
                    );
                    text(
                        &mut list,
                        ui,
                        &p.info.pid.to_string(),
                        px + ui.pt(16.0),
                        y,
                        ui.pt(60.0),
                        SECONDARY,
                    );
                    text(
                        &mut list,
                        ui,
                        &p.info.name,
                        px + ui.pt(78.0),
                        y,
                        pw - ui.pt(340.0),
                        if selected { ACCENT } else { WHITE },
                    );
                    text(
                        &mut list,
                        ui,
                        &format!(
                            "{} · {:.1}%",
                            termcore::format::memory(p.info.footprint),
                            self.terminal_ui.cpu[index]
                        ),
                        px + pw - ui.pt(242.0),
                        y,
                        ui.pt(148.0),
                        SECONDARY,
                    );
                    list.fill_rrect(
                        px + ui.pt(16.0),
                        y + row - 1.0,
                        pw - ui.pt(32.0),
                        1.0,
                        0.0,
                        BADGE,
                    );
                }
                let killable = self
                    .terminal_ui
                    .selected
                    .is_some_and(|(pid, _)| termcore::procs::killable(pid, std::process::id()));
                button(
                    s,
                    ui,
                    if self.terminal_ui.pending_signal == Some(false) {
                        "Confirm terminate"
                    } else {
                        "Terminate"
                    },
                    (
                        px + pw - ui.pt(240.0),
                        bottom - ui.pt(44.0),
                        ui.pt(112.0),
                        ui.pt(32.0),
                    ),
                    killable,
                );
                button(
                    s,
                    ui,
                    if self.terminal_ui.pending_signal == Some(true) {
                        "Confirm kill"
                    } else {
                        "Force kill"
                    },
                    (
                        px + pw - ui.pt(120.0),
                        bottom - ui.pt(44.0),
                        ui.pt(104.0),
                        ui.pt(32.0),
                    ),
                    killable,
                );
            }
            Panel::Look => self.terminal_draw_look(s, ui, px, pw, content_top, content_bottom),
            Panel::More => {
                let mut list = s.rows(content_top as usize, content_bottom as usize);
                for (i, label) in MENU.iter().enumerate() {
                    let y = content_top + ui.pt(i as f32 * 44.0) - self.terminal_ui.scroll;
                    if y + ui.pt(44.0) < content_top || y >= content_bottom {
                        continue;
                    }
                    text(
                        &mut list,
                        ui,
                        label,
                        px + ui.pt(16.0),
                        y,
                        pw - ui.pt(150.0),
                        if matches!(i, 11 | 13) { RED } else { WHITE },
                    );
                    let detail = match i {
                        1 | 2 => format!(
                            "{} pt  {}",
                            termcore::zoom::SIZES[self.terminal_ui.settings.zoom],
                            menu_key(i).unwrap_or_default()
                        ),
                        3 => THEMES[self.terminal_ui.settings.theme].name.into(),
                        16 => self.terminal_ui.cursor.label().into(),
                        17 => String::from(if self.terminal_ui.blink { "On" } else { "Off" }),
                        18 => String::from(if self.terminal_ui.secure { "On" } else { "Off" }),
                        29 => String::from(if self.terminal_ui.mouse_reporting { "On" } else { "Off" }),
                        _ => menu_key(i).map_or_else(|| "›".into(), String::from),
                    };
                    text(
                        &mut list,
                        ui,
                        &detail,
                        px + pw - ui.pt(126.0),
                        y,
                        ui.pt(110.0),
                        SECONDARY,
                    );
                    list.fill_rrect(
                        px + ui.pt(16.0),
                        y + ui.pt(44.0) - 1.0,
                        pw - ui.pt(32.0),
                        1.0,
                        0.0,
                        BADGE,
                    );
                }
            }
            _ => {}
        }
        s.fill_rrect(px, content_bottom, pw, footer_h, 0.0, BAR);
        s.fill_rrect(px, content_bottom, pw, 1.0, 0.0, TRACK);
        // Footer controls are redrawn after the footer background.
        if panel == Panel::Connections {
            let (ix, iy) = termcore::connui::import_origin((pw / ui.s) as f64, (ph / ui.s) as f64);
            button(
                s,
                ui,
                "Import from clipboard",
                (
                    px + ui.pt(ix as f32),
                    py + ui.pt(iy as f32),
                    ui.pt(200.0),
                    ui.pt(34.0),
                ),
                false,
            );
        } else if panel == Panel::Processes {
            let killable = self
                .terminal_ui
                .selected
                .is_some_and(|(pid, _)| termcore::procs::killable(pid, std::process::id()));
            button(
                s,
                ui,
                if self.terminal_ui.pending_signal == Some(false) {
                    "Confirm terminate"
                } else {
                    "Terminate"
                },
                (
                    px + pw - ui.pt(240.0),
                    bottom - ui.pt(44.0),
                    ui.pt(112.0),
                    ui.pt(32.0),
                ),
                killable,
            );
            button(
                s,
                ui,
                if self.terminal_ui.pending_signal == Some(true) {
                    "Confirm kill"
                } else {
                    "Force kill"
                },
                (
                    px + pw - ui.pt(120.0),
                    bottom - ui.pt(44.0),
                    ui.pt(104.0),
                    ui.pt(32.0),
                ),
                killable,
            );
        }
        if panel == Panel::Command {
            button(s, ui, if self.terminal_ui.title_edit { "Set" } else { "Run" }, (px + pw - ui.pt(120.0), bottom - ui.pt(44.0), ui.pt(104.0), ui.pt(32.0)), true);
        }
        let message = if !self.terminal_ui.message.is_empty() {
            self.terminal_ui.message.as_str()
        } else {
            match panel {
                Panel::Connections => "Tap a host to connect in a new tab.",
                Panel::Form => "Enter a name and host. Port defaults to 22.",
                Panel::Processes => "Select a process to inspect or terminate.",
                Panel::Command if self.terminal_ui.title_edit => "An empty title goes back to the automatic one.",
                Panel::Command => "The command runs in a new tab that closes when it ends.",
                Panel::Look => "Tap a colour well, then use the steppers. Changes are saved and apply to every tab.",
                _ => "Changes to text size and theme are saved automatically.",
            }
        };
        let reserved = match panel {
            Panel::Connections => 236.0,
            Panel::Processes => 264.0,
            Panel::Command => 136.0,
            _ => 32.0,
        };
        text(
            s,
            ui,
            message,
            px + ui.pt(16.0),
            content_bottom + ui.pt(8.0),
            pw - ui.pt(reserved),
            SECONDARY,
        );
    }
}
