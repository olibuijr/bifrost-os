//! The Locate window: a sidebar of the four sections (People, Devices, Items, Me), the content of the selected section,
//! and the first-run sheet. The sidebar is the SDK's source list; the text is drawn with the shared Golden Gate
//! type and colour tokens. The state lives in `Panel` (bifrost-locate); this file only draws it and turns control
//! requests into queued cloud requests.

use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE3_PT, SEPARATOR_C};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;
use bifrost_core::json::Json;
use bifrost_locate::control::{self, ParseError, Request};
use bifrost_locate::panel::{Panel, Row, Section, LOCATION_UNAVAILABLE, ONBOARDING_LINES};

/// The sidebar's width in points.
pub const SIDEBAR_PT: f32 = 200.0;
/// The distance from the content edge to its text, in points.
const INSET_PT: f32 = 24.0;
/// The line pitch of the content text, in points.
const LINE_PT: f32 = 22.0;
/// The content's top edge, in points.
const TOP_PT: f32 = 56.0;

/// The sidebar row id of a section: the section's position plus one, as the source list publishes it.
pub fn row_id(section: Section) -> u32 {
    Section::ALL.iter().position(|s| *s == section).map_or(0, |i| i as u32 + 1)
}

fn section_of(id: u32) -> Option<Section> {
    Section::ALL.get((id as usize).checked_sub(1)?).copied()
}

fn symbol(section: Section) -> &'static str {
    match section {
        Section::People => "person.2",
        Section::Devices => "iphone",
        Section::Items => "tag",
        Section::Me => "person.crop.circle",
    }
}

pub struct LocateWindow {
    pub panel: Panel,
    /// Control requests waiting for the cloud client to send them (locate, play sound).
    pub outbox: Vec<Request>,
    labels: TextCache,
}

impl LocateWindow {
    pub fn new(onboarded: bool) -> LocateWindow {
        LocateWindow { panel: Panel::new(onboarded), outbox: Vec::new(), labels: TextCache::default() }
    }

    /// The sidebar: one row per section, the selected one highlighted.
    pub fn sidebar(&self) -> SourceList {
        let items = Section::ALL.iter().map(|s| SidebarItem::row(row_id(*s), s.title()).with_symbol(symbol(*s))).collect();
        let mut list = SourceList::new(items, 28.0, 26.0);
        list.set_selected(Some(row_id(self.panel.section)));
        list
    }

    /// The section under a point in points, when the sidebar is there.
    pub fn section_at(&self, x: f32, y: f32, height_pt: f32) -> Option<Section> {
        if x >= SIDEBAR_PT {
            return None;
        }
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDEBAR_PT, h: height_pt };
        self.sidebar().hit(bounds, 0.0, x, y).and_then(section_of)
    }

    /// A control request from a script or the assistant. Returns the JSON reply; locate and play sound are queued.
    pub fn rpc(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        let request = control::parse(method, params).map_err(|e: ParseError| RpcError::new(INVALID_PARAMS, e.message()))?;
        match request {
            Request::Share(on) => {
                let changed = self.panel.set_sharing(on);
                Ok(Json::Obj(vec![("sharing".into(), Json::Bool(on)), ("changed".into(), Json::Bool(changed))]))
            }
            Request::Locate(ref id) | Request::PlaySound(ref id) => {
                let known = self.panel.device(id).is_some();
                let kind = if matches!(request, Request::PlaySound(_)) { "play_sound" } else { "locate" };
                self.outbox.push(request.clone());
                Ok(Json::Obj(vec![
                    ("device".into(), Json::Str(id.clone())),
                    ("command".into(), Json::Str(kind.into())),
                    ("known_device".into(), Json::Bool(known)),
                    ("status".into(), Json::Str("queued".into())),
                ]))
            }
        }
    }

    pub fn draw(&mut self, ui: &Ui, s: &mut Surface) {
        let (w_pt, h_pt) = (s.w as f32 / ui.pt(1.0), s.h as f32 / ui.pt(1.0));
        fill(s, ui, 0.0, 0.0, w_pt, h_pt, CONTENT_BG.now());
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDEBAR_PT, h: h_pt };
        widgets::source_list(s, ui, bounds, &self.sidebar(), 0.0, None);
        fill(s, ui, SIDEBAR_PT - 1.0, 0.0, 1.0, h_pt, SEPARATOR_C.now());

        let x = SIDEBAR_PT + INSET_PT;
        let max = w_pt - x - INSET_PT;
        let section = self.panel.section;
        self.line(ui, s, section.title(), MAC_TITLE3_PT, LABEL.now(), x, TOP_PT, max);
        let mut y = TOP_PT + LINE_PT * 2.0;
        for line in self.body(section) {
            self.line(ui, s, &line, MAC_BODY_PT, LABEL.now(), x, y, max);
            y += LINE_PT;
        }
        if self.panel.needs_onboarding() {
            self.draw_onboarding(ui, s, x, w_pt, h_pt);
        }
    }

    /// The lines of one section. An empty section says why, rather than showing nothing.
    fn body(&self, section: Section) -> Vec<String> {
        match section {
            Section::Devices => {
                let rows = self.panel.rows();
                if rows.is_empty() {
                    return vec!["No devices yet. A device appears here once it is signed in to your BifrOSt Account.".into()];
                }
                rows.iter().map(device_line).collect()
            }
            Section::People => vec![
                format!("Share My Location is {}.", if self.panel.sharing() { "on" } else { "off" }),
                "Contacts who share their location with you appear here.".into(),
            ],
            Section::Items => vec!["Tags and accessories are not supported on BifrOSt yet.".into()],
            Section::Me => vec![
                format!("Share My Location: {}", if self.panel.sharing() { "on" } else { "off" }),
                format!("Location: {}", self.panel.location_status.as_deref().unwrap_or(LOCATION_UNAVAILABLE)),
            ],
        }
    }

    fn draw_onboarding(&mut self, ui: &Ui, s: &mut Surface, x: f32, w_pt: f32, h_pt: f32) {
        let (cx, cy) = (x, h_pt / 3.0);
        let card_w = (w_pt - x - INSET_PT).max(0.0);
        let card_h = LINE_PT * (ONBOARDING_LINES.len() as f32 + 3.0);
        fill(s, ui, cx - 12.0, cy - 12.0, card_w + 24.0, card_h + 24.0, CARD_BG.now());
        self.line(ui, s, "Locate", MAC_TITLE3_PT, LABEL.now(), cx, cy, card_w);
        let mut y = cy + LINE_PT * 2.0;
        for line in ONBOARDING_LINES {
            self.line(ui, s, line, MAC_BODY_PT, LABEL.now(), cx, y, card_w);
            y += LINE_PT * 1.5;
        }
        self.line(ui, s, "Press Return to continue.", MAC_FOOTNOTE_PT, LABEL_2.now(), cx, y, card_w);
    }

    /// Text at a point, the top of the line, cut to `max` points.
    fn line(&mut self, ui: &Ui, s: &mut Surface, label: &str, pt: f32, color: u32, x: f32, top_pt: f32, max: f32) {
        let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
        let img = self.labels.get(ui, &fitted, pt, Weight::Regular, color);
        let top = ui.pt(top_pt);
        s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
    }
}

/// One device's line: its name, then its battery and whether a position is known.
fn device_line(row: &Row) -> String {
    let mut text = row.name.clone();
    if row.is_this_device {
        text.push_str(" (this device)");
    }
    if let Some(b) = &row.battery {
        text.push_str(" · ");
        text.push_str(b);
    }
    text.push_str(if row.located { " · Located" } else { " · No location yet" });
    text
}

/// A filled rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w.max(0.0)), ui.pt(h.max(0.0)), 0.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_rows_round_trip() {
        for s in Section::ALL {
            assert_eq!(section_of(row_id(s)), Some(s));
        }
        assert_eq!(section_of(0), None);
        assert_eq!(section_of(9), None);
    }

    #[test]
    fn the_sidebar_selects_the_current_section() {
        let mut w = LocateWindow::new(true);
        w.panel.select(Section::Me);
        assert_eq!(w.sidebar().selected(), Some(row_id(Section::Me)));
    }

    #[test]
    fn locate_is_queued_and_share_sets_the_switch() {
        let mut w = LocateWindow::new(true);
        let reply = w.rpc("locate.locate", &bifrost_core::json::parse(r#"{"device":"d1"}"#).unwrap()).unwrap();
        assert_eq!(reply.get("status").and_then(Json::as_str), Some("queued"));
        assert_eq!(w.outbox, vec![Request::Locate("d1".into())]);
        let on = w.rpc("locate.share", &bifrost_core::json::parse(r#"{"on":true}"#).unwrap()).unwrap();
        assert_eq!(on.get("changed"), Some(&Json::Bool(true)));
        assert!(w.panel.sharing());
    }

    #[test]
    fn a_bad_control_request_is_an_invalid_params_error() {
        let mut w = LocateWindow::new(true);
        let err = w.rpc("locate.locate", &Json::Obj(Vec::new())).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert!(w.outbox.is_empty());
    }

    #[test]
    fn an_empty_device_list_says_why() {
        let w = LocateWindow::new(true);
        let lines = w.body(Section::Devices);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("No devices yet"));
    }
}
