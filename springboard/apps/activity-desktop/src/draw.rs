//! Paints the Activity Monitor window: the sidebar, the toolbar band, the tab control, the graph card, the table and
//! its header, and the footer. The geometry is `Monitor::geo` and `window.rs`, which the pointer input shares, so a
//! tap lands on what is drawn. Colours are the kit's semantic tokens resolved with `now()`, so Light and Dark follow
//! the appearance.

use crate::model::{bytes, energy, percent, rate, Column, SortKey, Tab, HISTORY};
use crate::sample::{ProcSample, Snapshot};
use crate::window::{Geo, Monitor, B_SEARCH, FOOT_PT, HEAD_PT, ROW_PT, SIDE_PT, SIDE_SCROLL};
use bifrost_app::controls::Rect;
use bifrost_app::kit::colors::{SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PURPLE};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, SELECTION, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Text inset in a table cell and in the graph card.
const PAD: f32 = 8.0;
const CARD_PAD: f32 = 14.0;
/// Corner radius of the graph card (points): a grouped card.
const CARD_RADIUS_PT: f32 = 13.0;
/// Floors for the disk and network graphs, so a quiet machine does not look busy (bytes per second).
const DISK_FLOOR: f32 = 1024.0 * 1024.0;
const NET_FLOOR: f32 = 64.0 * 1024.0;

/// A filled rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w.max(0.0)), ui.pt(h.max(0.0)), 0.0, color);
}

/// A filled rounded rectangle in points.
fn rounded(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w.max(0.0)), ui.pt(h.max(0.0)), ui.pt(radius), color);
}

/// The width of a label in points.
fn width(ui: &Ui, label: &str, pt: f32, weight: Weight) -> f32 {
    ui.measure(label, pt, weight) / ui.pt(1.0)
}

/// Text at a point, vertically centred on `mid`, cut to `max` points.
#[allow(clippy::too_many_arguments)]
fn text(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// What one graph shows: its title, the figure and detail lines, the plotted series (oldest first, with colours) and
/// the value that reaches the top of the plot.
struct Graph {
    title: &'static str,
    value: String,
    detail: String,
    plots: Vec<(Vec<f32>, u32)>,
    scale: f32,
}

/// The text of one table cell for a process row.
fn cell(p: &ProcSample, key: Option<SortKey>) -> String {
    match key {
        Some(SortKey::Name) => p.name.clone(),
        Some(SortKey::User) => p.user.clone(),
        Some(SortKey::Cpu) => format!("{:.1}", p.cpu),
        Some(SortKey::Memory) => bytes(p.rss),
        Some(SortKey::Energy) => format!("{:.1}", energy(p)),
        Some(SortKey::DiskRead) => rate(p.read_bps),
        Some(SortKey::DiskWrite) => rate(p.write_bps),
        Some(SortKey::Threads) => p.threads.to_string(),
        Some(SortKey::Pid) => p.pid.to_string(),
        None => String::new(),
    }
}

/// The text of one cell for an interface row on the Network tab, by the column's title.
fn iface_cell(i: &crate::sample::IfaceRate, title: &str) -> String {
    match title {
        "Received/s" => rate(i.rx_bps),
        "Sent/s" => rate(i.tx_bps),
        "Total Received" => bytes(i.rx_total),
        "Total Sent" => bytes(i.tx_total),
        _ => i.name.clone(),
    }
}

impl Monitor {
    /// Paints the whole window. Also keeps the scroll range in step with the rows.
    pub fn draw(&mut self, ui: &Ui, s: &mut Surface) {
        let g = self.geo(ui);
        self.fit_scroll(ui);
        fill(s, ui, 0.0, 0.0, g.w, g.h, CONTENT_BG.now());
        self.draw_sidebar(ui, s, &g);
        self.draw_graph(ui, s);
        self.draw_tabs(ui, s);
        self.draw_rows(ui, s, &g);
        self.draw_chrome(ui, s, &g);
    }

    fn draw_sidebar(&self, ui: &Ui, s: &mut Surface, g: &Geo) {
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: g.h };
        widgets::source_list(s, ui, bounds, &self.sidebar(), SIDE_SCROLL, None);
        s.fill_rect(ui.pt(SIDE_PT) as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
    }

    fn draw_tabs(&self, ui: &Ui, s: &mut Surface) {
        let labels: Vec<&str> = Tab::ALL.iter().map(|t| t.label()).collect();
        widgets::segmented(s, ui, self.seg_rect(ui), &labels, self.tab.index(), true);
    }

    /// The figures a tab's graph shows, from the last reading and the history.
    fn graph(&self) -> Graph {
        let h = &self.history;
        let snap: Option<&Snapshot> = self.snapshot.as_ref();
        match self.tab {
            Tab::Cpu => {
                let c = snap.map(|s| s.cpu).unwrap_or_default();
                Graph {
                    title: "CPU Usage",
                    value: percent(c.busy),
                    detail: format!("User {}   System {}   Idle {}", percent(c.user), percent(c.system), percent(c.idle)),
                    plots: vec![(h.cpu.values().collect(), SYSTEM_BLUE.now())],
                    scale: 100.0,
                }
            }
            Tab::Memory => {
                let m = snap.map(|s| s.memory).unwrap_or_default();
                Graph {
                    title: "Memory Used",
                    value: format!("{} of {}", bytes(m.used()), bytes(m.total)),
                    detail: format!("Cached Files {}   Swap Used {}", bytes(m.cached), bytes(m.swap_used())),
                    plots: vec![(h.memory.values().collect(), SYSTEM_PURPLE.now())],
                    scale: 100.0,
                }
            }
            Tab::Energy => {
                let top = snap.and_then(|s| s.procs.iter().max_by(|a, b| energy(a).total_cmp(&energy(b))));
                Graph {
                    title: "Energy Impact",
                    value: format!("{:.1}", h.energy.last()),
                    detail: top.map_or(String::new(), |p| format!("Highest: {} ({:.1})", p.name, energy(p))),
                    plots: vec![(h.energy.values().collect(), SYSTEM_ORANGE.now())],
                    scale: h.energy.peak().max(10.0),
                }
            }
            Tab::Disk => {
                let (r, w): (f64, f64) = snap.map_or((0.0, 0.0), |s| (s.disk_read_bps, s.disk_write_bps));
                Graph {
                    title: "Disk Activity",
                    value: format!("Read {}   Written {}", rate(r), rate(w)),
                    detail: "All disks together".into(),
                    plots: vec![(h.disk_read.values().collect(), SYSTEM_BLUE.now()), (h.disk_write.values().collect(), SYSTEM_GREEN.now())],
                    scale: h.disk_read.peak().max(h.disk_write.peak()).max(DISK_FLOOR),
                }
            }
            Tab::Network => {
                let (rx, tx, n): (f64, f64, usize) = snap.map_or((0.0, 0.0, 0), |s| (s.ifaces.iter().map(|i| i.rx_bps).sum(), s.ifaces.iter().map(|i| i.tx_bps).sum(), s.ifaces.len()));
                Graph {
                    title: "Network",
                    value: format!("Received {}   Sent {}", rate(rx), rate(tx)),
                    detail: format!("{n} interfaces, loopback not counted"),
                    plots: vec![(h.net_rx.values().collect(), SYSTEM_BLUE.now()), (h.net_tx.values().collect(), SYSTEM_GREEN.now())],
                    scale: h.net_rx.peak().max(h.net_tx.peak()).max(NET_FLOOR),
                }
            }
        }
    }

    fn draw_graph(&mut self, ui: &Ui, s: &mut Surface) {
        let r = self.graph_rect(ui);
        rounded(s, ui, r.x, r.y, r.w, r.h, CARD_RADIUS_PT, CARD_BG.now());
        let graph = self.graph();
        let x = r.x + CARD_PAD;
        let w = r.w - 2.0 * CARD_PAD;
        text(&mut self.labels, s, ui, graph.title, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x, r.y + 16.0, w);
        text(&mut self.labels, s, ui, &graph.value, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), x, r.y + 34.0, w);
        text(&mut self.labels, s, ui, &graph.detail, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, r.y + 50.0, w);

        // the plot: one bar per sample, newest at the right, each series from the baseline
        let plot_y = r.y + 62.0;
        let plot_h = (r.h - 72.0).max(4.0);
        let slot = w / HISTORY as f32;
        fill(s, ui, x, plot_y + plot_h, w, 1.0, SEPARATOR_C.now());
        for (values, color) in &graph.plots {
            let offset = HISTORY.saturating_sub(values.len());
            for (i, v) in values.iter().enumerate() {
                let bh = (v / graph.scale).clamp(0.0, 1.0) * plot_h;
                let bx = x + (offset + i) as f32 * slot;
                fill(s, ui, bx, plot_y + plot_h - bh, (slot - 1.0).max(1.0), bh, *color);
            }
        }
    }

    /// The table rows that are on screen. Rows scrolled above the table are left out; the header covers the rest.
    fn draw_rows(&mut self, ui: &Ui, s: &mut Surface, g: &Geo) {
        let cols = self.col_frames(ui);
        let table_w = g.content_w;
        let x0 = g.content_x;
        let count = self.line_count();
        if count == 0 {
            let msg = match (&self.snapshot, self.tab) {
                (None, _) => "Reading the system",
                (Some(_), Tab::Network) => "No network interfaces",
                (Some(_), _) => "No matching processes",
            };
            let mid = (g.table_top + g.table_bottom) / 2.0;
            let mx = x0 + ((table_w - width(ui, msg, MAC_TITLE3_PT, Weight::Regular)) / 2.0).max(PAD);
            text(&mut self.labels, s, ui, msg, MAC_TITLE3_PT, Weight::Regular, LABEL_3.now(), mx, mid, table_w - 2.0 * PAD);
            return;
        }
        let offset = self.scroll.offset();
        let first = (offset / ROW_PT).floor().max(0.0) as usize;
        let visible = ((g.table_bottom - g.table_top) / ROW_PT).ceil() as usize + 2;
        let last = (first + visible).min(count);
        for i in first..last {
            let y = g.table_top + i as f32 * ROW_PT - offset;
            if y < g.table_top {
                continue;
            }
            let mid = y + ROW_PT / 2.0;
            if self.tab == Tab::Network {
                let Some(iface) = self.snapshot.as_ref().and_then(|s| s.ifaces.get(i)) else { continue };
                for (col, cx, cw) in &cols {
                    let quiet = col.key.is_none() && !matches!(col.title, "Interface");
                    let color = if quiet { LABEL_2.now() } else { LABEL.now() };
                    text(&mut self.labels, s, ui, &iface_cell(iface, col.title), MAC_SUBHEAD_PT, Weight::Regular, color, cx + PAD, mid, cw - 2.0 * PAD);
                }
                continue;
            }
            let Some(p) = self.rows.get(i) else { continue };
            let selected = self.selected == Some(p.pid);
            if selected {
                fill(s, ui, x0, y, table_w, ROW_PT, SELECTION.now());
            }
            let ink = if selected { WHITE } else { LABEL.now() };
            let quiet = if selected { WHITE } else { LABEL_2.now() };
            for (col, cx, cw) in &cols {
                let color = if col.key == Some(SortKey::Name) { ink } else { quiet };
                text(&mut self.labels, s, ui, &cell(p, col.key), MAC_SUBHEAD_PT, Weight::Regular, color, cx + PAD, mid, cw - 2.0 * PAD);
            }
        }
    }

    /// The toolbar band, the table header (with the sort arrow), the footer and the toolbar's search field.
    fn draw_chrome(&mut self, ui: &Ui, s: &mut Surface, g: &Geo) {
        let x0 = SIDE_PT;
        widgets::toolbar_band(s, ui, Rect { x: x0, y: 0.0, w: g.w - x0, h: UNIFIED_BAR_PT });
        let (bar, layout) = self.bar_layout(ui);
        widgets::toolbar(s, ui, self.band(ui), &bar, None);
        if let Some(r) = layout.rect_of(B_SEARCH) {
            widgets::search_field(s, ui, &self.query, Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 }, "Search", self.search_focused);
        }

        let cols: Vec<(Column, f32, f32)> = self.col_frames(ui);
        fill(s, ui, g.content_x, g.head_y, g.content_w, HEAD_PT, CONTENT_BG.now());
        for (col, cx, cw) in &cols {
            let sorted = col.key.is_some() && col.key == Some(self.sort.key);
            let title = if sorted {
                format!("{} {}", col.title, if self.sort.descending { "▼" } else { "▲" })
            } else {
                col.title.to_string()
            };
            let color = if sorted { LABEL.now() } else { LABEL_2.now() };
            text(&mut self.labels, s, ui, &title, MAC_FOOTNOTE_PT, Weight::Semibold, color, cx + PAD, g.head_y + HEAD_PT / 2.0, cw - 2.0 * PAD);
        }
        fill(s, ui, g.content_x, g.head_y + HEAD_PT - 1.0, g.content_w, 1.0, SEPARATOR_C.now());

        let fy = g.h - FOOT_PT;
        fill(s, ui, x0, fy, g.w - x0, FOOT_PT, CONTENT_BG.now());
        fill(s, ui, x0, fy, g.w - x0, 1.0, SEPARATOR_C.now());
        let footer = self.footer_text();
        text(&mut self.labels, s, ui, &footer, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0 + 16.0, fy + FOOT_PT / 2.0, g.w - x0 - 32.0);
        if !self.status.is_empty() {
            let status = self.status.clone();
            let max = (g.w - x0) * 0.4;
            text(&mut self.labels, s, ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, SYSTEM_ORANGE.now(), g.w - 16.0 - max, fy + FOOT_PT / 2.0, max);
        }
    }
}
