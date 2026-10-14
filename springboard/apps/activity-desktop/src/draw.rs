//! Paints the Activity Monitor window: the sidebar, the toolbar band, the tab control, the graph card, the table and
//! its header, the footer and the sheets over them. The geometry is `Monitor::geo` and `window.rs`, which the pointer
//! input shares, so a tap lands on what is drawn. Colours are the kit's semantic tokens resolved with `now()`, so
//! Light and Dark follow the appearance.

use crate::inspect::{self, Inspection, Section};
use crate::model::{bytes, cell_text, energy, percent, rate, Baseline, Column, Series, SortKey, Tab, HISTORY, SIGNALS};
use crate::sample::{IfaceRate, Snapshot};
use crate::window::{Geo, Monitor, Overlay, B_SEARCH, FOOT_PT, HEAD_PT, LINE_PT, ROW_PT, SIDE_PT, SIDE_SCROLL, SIGNAL_ROW_PT};
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
/// The share of the graph card the plot takes when figures are listed beside it, and the gap before the figures.
const PLOT_SHARE: f32 = 0.56;
const FIGURES_GAP: f32 = 24.0;
const FIGURE_LINE_PT: f32 = 15.0;

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

/// Text whose right edge is at `right`.
#[allow(clippy::too_many_arguments)]
fn text_right(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, right: f32, mid: f32, max: f32) {
    let w = width(ui, label, pt, weight).min(max);
    text(labels, s, ui, label, pt, weight, color, right - w, mid, max);
}

/// A horizontal progress bar in points: a track and the filled share of it.
fn bar(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, fraction: f32, color: u32) {
    rounded(s, ui, x, y, w, h, h / 2.0, SEPARATOR_C.now());
    let filled = (w * fraction.clamp(0.0, 1.0)).max(if fraction > 0.0 { h } else { 0.0 }).min(w);
    rounded(s, ui, x, y, filled, h, h / 2.0, color);
}

/// What one graph shows: its title, the figure and detail lines, the plotted series (oldest first, with colours), the
/// value that reaches the top of the plot, and the figures listed beside the plot.
struct Graph {
    title: &'static str,
    value: String,
    detail: String,
    plots: Vec<(Vec<f32>, u32)>,
    scale: f32,
    figures: Vec<(String, String)>,
}

fn figure(label: &str, value: String) -> (String, String) {
    (label.to_string(), value)
}

/// The text of one cell for an interface row on the Network tab, by the column's title.
fn iface_cell(i: &IfaceRate, title: &str) -> String {
    match title {
        "Received/s" => rate(i.rx_bps),
        "Sent/s" => rate(i.tx_bps),
        "Total Received" => bytes(i.rx_total),
        "Total Sent" => bytes(i.tx_total),
        "Packets In" => i.rx_packets.to_string(),
        "Packets Out" => i.tx_packets.to_string(),
        _ => i.name.clone(),
    }
}

/// The lines of the open report or file list, for the scroll range of a sheet.
fn scroll_lines(overlay: &Option<Overlay>, inspector: &Option<(u32, Inspection)>, section: Section) -> usize {
    match overlay {
        Some(Overlay::Report { text, .. }) => text.lines().count(),
        Some(Overlay::Inspector) if section == Section::OpenFiles => inspector.as_ref().map_or(0, |(_, d)| inspect::file_lines(d).len()),
        _ => 0,
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
        self.draw_overlay(ui, s);
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
                let threads: u32 = snap.map_or(0, |s| s.procs.iter().map(|p| p.threads).sum());
                let procs = snap.map_or(0, |s| s.procs.len());
                let cores = snap.map_or(0, |s| s.cores.len());
                Graph {
                    title: "CPU Usage",
                    value: percent(c.busy),
                    detail: format!("{cores} cores"),
                    plots: vec![(h.cpu.values().collect(), SYSTEM_BLUE.now())],
                    scale: 100.0,
                    figures: vec![
                        figure("System", percent(c.system)),
                        figure("User", percent(c.user)),
                        figure("Idle", percent(c.idle)),
                        figure("Threads", threads.to_string()),
                        figure("Processes", procs.to_string()),
                    ],
                }
            }
            Tab::Gpu => {
                let busy = snap.map_or(0.0, |s| s.gpu_busy);
                let users = snap.map_or(0, |s| s.procs.iter().filter(|p| p.gpu > 0.0).count());
                Graph {
                    title: "GPU Usage",
                    value: percent(busy),
                    detail: "Per-process figures cover your own processes".into(),
                    plots: vec![(h.gpu.values().collect(), SYSTEM_GREEN.now())],
                    scale: 100.0,
                    figures: vec![figure("GPU", percent(busy)), figure("Processes using the GPU", users.to_string())],
                }
            }
            Tab::Memory => {
                let m = snap.map(|s| s.memory).unwrap_or_default();
                let pressure = snap.and_then(|s| s.pressure);
                Graph {
                    title: "Memory Used",
                    value: format!("{} of {}", bytes(m.used()), bytes(m.total)),
                    detail: pressure.map_or("Memory pressure is not reported".to_string(), |p| format!("Memory Pressure {}", percent(p))),
                    plots: vec![(h.memory.values().collect(), SYSTEM_PURPLE.now()), (h.pressure.values().collect(), SYSTEM_ORANGE.now())],
                    scale: 100.0,
                    figures: vec![
                        figure("Physical Memory", bytes(m.total)),
                        figure("Memory Used", bytes(m.used())),
                        figure("Cached Files", bytes(m.cached)),
                        figure("Shared Memory", bytes(m.shared)),
                        figure("Buffers", bytes(m.buffers)),
                        figure("Compressed", bytes(m.zswap)),
                        figure("Swap Used", bytes(m.swap_used())),
                    ],
                }
            }
            Tab::Energy => {
                let top = snap.and_then(|s| s.procs.iter().max_by(|a, b| energy(a).total_cmp(&energy(b))));
                let blockers = snap.map_or(0, |s| s.procs.iter().filter(|p| p.prevents_sleep).count());
                Graph {
                    title: "Energy Impact",
                    value: format!("{:.1}", h.energy.last()),
                    detail: top.map_or(String::new(), |p| format!("Highest: {} ({:.1})", p.name, energy(p))),
                    plots: vec![(h.energy.values().collect(), SYSTEM_ORANGE.now())],
                    scale: h.energy.peak().max(10.0),
                    figures: vec![figure("Energy Impact", format!("{:.1}", h.energy.last())), figure("Preventing Sleep", blockers.to_string())],
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
                    figures: Vec::new(),
                }
            }
            Tab::Network => {
                let ifaces: &[IfaceRate] = snap.map_or(&[], |s| s.ifaces.as_slice());
                let (rx, tx): (f64, f64) = (ifaces.iter().map(|i| i.rx_bps).sum(), ifaces.iter().map(|i| i.tx_bps).sum());
                let (rxp, txp): (f64, f64) = (ifaces.iter().map(|i| i.rx_pps).sum(), ifaces.iter().map(|i| i.tx_pps).sum());
                let (rxt, txt): (u64, u64) = (ifaces.iter().map(|i| i.rx_total).sum(), ifaces.iter().map(|i| i.tx_total).sum());
                Graph {
                    title: "Network",
                    value: format!("Received {}   Sent {}", rate(rx), rate(tx)),
                    detail: format!("{} interfaces, loopback not counted", ifaces.len()),
                    plots: vec![(h.net_rx.values().collect(), SYSTEM_BLUE.now()), (h.net_tx.values().collect(), SYSTEM_GREEN.now())],
                    scale: h.net_rx.peak().max(h.net_tx.peak()).max(NET_FLOOR),
                    figures: vec![
                        figure("Packets in/s", format!("{rxp:.0}")),
                        figure("Packets out/s", format!("{txp:.0}")),
                        figure("Data received", bytes(rxt)),
                        figure("Data sent", bytes(txt)),
                    ],
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
        let plot_w = if graph.figures.is_empty() { w } else { w * PLOT_SHARE };
        text(&mut self.labels, s, ui, graph.title, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x, r.y + 16.0, plot_w);
        text(&mut self.labels, s, ui, &graph.value, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), x, r.y + 34.0, plot_w);
        text(&mut self.labels, s, ui, &graph.detail, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, r.y + 50.0, plot_w);

        // the plot: one bar per sample, newest at the right, each series from the baseline
        let plot_y = r.y + 62.0;
        let plot_h = (r.h - 72.0).max(4.0);
        let slot = plot_w / HISTORY as f32;
        fill(s, ui, x, plot_y + plot_h, plot_w, 1.0, SEPARATOR_C.now());
        for (values, color) in &graph.plots {
            let offset = HISTORY.saturating_sub(values.len());
            for (i, v) in values.iter().enumerate() {
                let bh = (v / graph.scale).clamp(0.0, 1.0) * plot_h;
                let bx = x + (offset + i) as f32 * slot;
                fill(s, ui, bx, plot_y + plot_h - bh, (slot - 1.0).max(1.0), bh, *color);
            }
        }

        // the figures beside the plot: a label at the left, its value at the right edge of the card
        if !graph.figures.is_empty() {
            let fx = x + plot_w + FIGURES_GAP;
            let fw = (x + w - fx).max(0.0);
            for (i, (label, value)) in graph.figures.iter().enumerate() {
                let mid = r.y + 16.0 + i as f32 * FIGURE_LINE_PT;
                text(&mut self.labels, s, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), fx, mid, fw * 0.62);
                text_right(&mut self.labels, s, ui, value, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), x + w, mid, fw * 0.4);
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
        let baseline: Option<Baseline> = self.baseline.clone();
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
                text(&mut self.labels, s, ui, &cell_text(p, col.key, baseline.as_ref()), MAC_SUBHEAD_PT, Weight::Regular, color, cx + PAD, mid, cw - 2.0 * PAD);
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

    /// The sheet over the window, if one is open: its frame from the kit, then the content for the overlay.
    fn draw_overlay(&mut self, ui: &Ui, s: &mut Surface) {
        let Some(sheet) = self.current_sheet(ui) else { return };
        let parent = self.window_rect(ui);
        let content = widgets::sheet(s, ui, parent, UNIFIED_BAR_PT, &sheet, 1.0, self.sheet_press);

        // lists that scroll inside the sheet
        let lines = scroll_lines(&self.overlay, &self.inspector, self.inspector_section);
        let list_top = if matches!(self.overlay, Some(Overlay::Inspector)) { content.y + INSPECTOR_HEAD_PT } else { content.y };
        let list_h = (content.y + content.h - list_top).max(0.0);
        self.sheet_scroll.set_max((lines as f32 * LINE_PT - list_h).max(0.0));
        let offset = self.sheet_scroll.offset();

        let overlay = self.overlay.take();
        match &overlay {
            Some(Overlay::Signal { choice }) => draw_signals(&mut self.labels, s, ui, content, *choice),
            Some(Overlay::Inspector) => {
                if let Some((pid, data)) = &self.inspector {
                    draw_inspector(&mut self.labels, s, ui, content, self.inspector_section, *pid, data, offset);
                }
            }
            Some(Overlay::Sampling(job)) => {
                let message = format!("Sampling for {} seconds. Stop ends it early.", job.options.seconds);
                draw_progress(&mut self.labels, s, ui, content, &message, job.fraction());
            }
            Some(Overlay::Report { text: report, .. }) => draw_lines(&mut self.labels, s, ui, content, report, offset),
            Some(Overlay::Diagnostics(job)) => draw_progress(&mut self.labels, s, ui, content, "Collecting kernel counters, processes, disks and logs.", job.fraction()),
            Some(Overlay::CpuUsage) => {
                let cores: Vec<f32> = self.snapshot.as_ref().map(|s| s.cores.clone()).unwrap_or_default();
                draw_core_bars(&mut self.labels, s, ui, content, &cores);
            }
            Some(Overlay::CpuHistory) => draw_core_history(&mut self.labels, s, ui, content, &self.history.cores),
            Some(Overlay::GpuHistory) => {
                let busy = self.snapshot.as_ref().map_or(0.0, |s| s.gpu_busy);
                draw_strip(&mut self.labels, s, ui, content, &format!("GPU {}", percent(busy)), &self.history.gpu, SYSTEM_GREEN.now());
            }
            None => {}
        }
        self.overlay = overlay;
    }
}

/// Height of the inspector's segmented control and the two header lines under it (points).
const INSPECTOR_HEAD_PT: f32 = 24.0 + 50.0;

/// The signal list: the chosen row on the selection colour.
fn draw_signals(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, choice: usize) {
    for (i, sig) in SIGNALS.iter().enumerate() {
        let y = c.y + i as f32 * SIGNAL_ROW_PT;
        let chosen = i == choice;
        if chosen {
            rounded(s, ui, c.x, y, c.w, SIGNAL_ROW_PT, 5.0, SELECTION.now());
        }
        let ink = if chosen { WHITE } else { LABEL.now() };
        let quiet = if chosen { WHITE } else { LABEL_2.now() };
        text(labels, s, ui, &format!("SIG{}", sig.name), MAC_SUBHEAD_PT, Weight::Regular, ink, c.x + PAD, y + SIGNAL_ROW_PT / 2.0, c.w * 0.6);
        text_right(labels, s, ui, &sig.number.to_string(), MAC_SUBHEAD_PT, Weight::Regular, quiet, c.x + c.w - PAD, y + SIGNAL_ROW_PT / 2.0, c.w * 0.3);
    }
}

/// The inspector: the segments, the command and counts, then the figures of the chosen segment.
#[allow(clippy::too_many_arguments)]
fn draw_inspector(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, section: Section, pid: u32, data: &Inspection, offset: f32) {
    let names: Vec<&str> = Section::ALL.iter().map(|s| s.label()).collect();
    widgets::segmented(s, ui, Monitor::inspector_segments(c), &names, section.index(), true);
    let command = if data.command.is_empty() { &data.exe } else { &data.command };
    text(labels, s, ui, command, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), c.x, c.y + 24.0 + 16.0, c.w);
    let counts = format!("Process {pid}   Folder {}   Open Files {}   Sockets {}   Threads {}", data.cwd, data.files_total, data.sockets, data.threads_total);
    text(labels, s, ui, &counts, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), c.x, c.y + 24.0 + 34.0, c.w);

    let top = c.y + INSPECTOR_HEAD_PT;
    let bottom = c.y + c.h;
    match section {
        Section::OpenFiles => {
            let lines = inspect::file_lines(data);
            if lines.is_empty() {
                text(labels, s, ui, "No open files", MAC_SUBHEAD_PT, Weight::Regular, LABEL_3.now(), c.x, top + 10.0, c.w);
            }
            for (i, line) in lines.iter().enumerate() {
                let y = top + i as f32 * LINE_PT - offset;
                if y < top || y + LINE_PT > bottom {
                    continue;
                }
                text(labels, s, ui, line, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), c.x, y + LINE_PT / 2.0, c.w);
            }
        }
        _ => {
            for (i, (label, value)) in inspect::rows(section, data).iter().enumerate() {
                let y = top + i as f32 * 18.0;
                if y + 18.0 > bottom {
                    break;
                }
                text(labels, s, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), c.x, y + 9.0, c.w * 0.4);
                text(labels, s, ui, value, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), c.x + c.w * 0.42, y + 9.0, c.w * 0.58);
            }
        }
    }
}

/// A line of text and a progress bar under it.
fn draw_progress(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, message: &str, fraction: f32) {
    text(labels, s, ui, message, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), c.x, c.y + 8.0, c.w);
    bar(s, ui, c.x, c.y + 24.0, c.w, 6.0, fraction, SYSTEM_BLUE.now());
}

/// Text lines from `offset` points down, as many as fit.
fn draw_lines(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, body: &str, offset: f32) {
    for (i, line) in body.lines().enumerate() {
        let y = c.y + i as f32 * LINE_PT - offset;
        if y < c.y || y + LINE_PT > c.y + c.h {
            continue;
        }
        text(labels, s, ui, line, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), c.x, y + LINE_PT / 2.0, c.w);
    }
}

/// One bar per core with its share.
fn draw_core_bars(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, cores: &[f32]) {
    if cores.is_empty() {
        text(labels, s, ui, "No per-core readings yet", MAC_SUBHEAD_PT, Weight::Regular, LABEL_3.now(), c.x, c.y + 10.0, c.w);
        return;
    }
    for (i, busy) in cores.iter().enumerate() {
        let y = c.y + i as f32 * 20.0;
        if y + 20.0 > c.y + c.h + 1.0 {
            break;
        }
        text(labels, s, ui, &format!("Core {}", i + 1), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), c.x, y + 10.0, 54.0);
        bar(s, ui, c.x + 60.0, y + 6.0, (c.w - 60.0 - 52.0).max(10.0), 8.0, busy / 100.0, SYSTEM_BLUE.now());
        text_right(labels, s, ui, &percent(*busy), MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), c.x + c.w, y + 10.0, 48.0);
    }
}

/// A history plot over the width of `c`: one bar per sample on a 0 to 100 scale, with the label above it.
fn plot_history(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, series: &Series, color: u32) {
    fill(s, ui, x, y + h, w, 1.0, SEPARATOR_C.now());
    let values: Vec<f32> = series.values().collect();
    let slot = w / HISTORY as f32;
    let offset = HISTORY.saturating_sub(values.len());
    for (i, v) in values.iter().enumerate() {
        let bh = (v / 100.0).clamp(0.0, 1.0) * h;
        fill(s, ui, x + (offset + i) as f32 * slot, y + h - bh, (slot - 1.0).max(1.0), bh, color);
    }
}

/// One strip per core, each with its label.
fn draw_core_history(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, cores: &[Series]) {
    if cores.is_empty() {
        text(labels, s, ui, "No per-core readings yet", MAC_SUBHEAD_PT, Weight::Regular, LABEL_3.now(), c.x, c.y + 10.0, c.w);
        return;
    }
    let strip = (c.h / cores.len() as f32).clamp(12.0, 48.0);
    for (i, series) in cores.iter().enumerate() {
        let y = c.y + i as f32 * strip;
        if y + strip > c.y + c.h + 1.0 {
            break;
        }
        text(labels, s, ui, &format!("Core {}", i + 1), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), c.x, y + strip / 2.0, 52.0);
        plot_history(s, ui, c.x + 58.0, y + 2.0, c.w - 58.0, strip - 6.0, series, SYSTEM_BLUE.now());
    }
}

/// One labelled history strip filling the content.
fn draw_strip(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: Rect, label: &str, series: &Series, color: u32) {
    text(labels, s, ui, label, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), c.x, c.y + 10.0, c.w);
    plot_history(s, ui, c.x, c.y + 28.0, c.w, (c.h - 34.0).max(8.0), series, color);
}
