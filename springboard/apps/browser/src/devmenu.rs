//! Develop menu state: View Source, Reload From Origin, user agent presets and Responsive Design Mode
//! (Browser's Develop menu, `WebView.customUserAgent`).

use bifrost_app::AppMenuItem;

pub const UA_PRESETS: [(&str, &str); 5] = [
    ("Default", ""),
    ("Browser Golden Gate", "Mozilla/5.0 (Macintosh; Intel Golden Gate 10_15_7) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Browser/605.1.15"),
    ("Browser bifrost", "Mozilla/5.0 (bifrost; CPU bifrost OS 17_5 like Golden Gate) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Browser/604.1"),
    ("Chrome Android", "Mozilla/5.0 (Linux; Android 14; Pixel 8) WebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Browser/537.36"),
    ("Firefox", "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"),
];

/// Responsive presets: name, CSS width, CSS height.
pub const SIZE_PRESETS: [(&str, u32, u32); 3] = [("bifrost 15", 393, 852), ("bifrost", 820, 1180), ("1280x800", 1280, 800)];
/// Device pixel ratios the Develop menu offers.
pub const DPRS: [u32; 3] = [1, 2, 3];

/// Develop menu item ids. The user agent, size and pixel-ratio items are `base + index` into their table.
pub const MENU_RESPONSIVE: u32 = 1400;
const MENU_UA_BASE: u32 = 1410;
const MENU_SIZE_BASE: u32 = 1420;
const MENU_DPR_BASE: u32 = 1430;

/// What a Develop menu item picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Responsive,
    UserAgent(usize),
    Size(usize),
    Dpr(u32),
}

/// The choice a Develop menu item id stands for, when the id is one of ours.
pub fn pick(id: u32) -> Option<Pick> {
    if id == MENU_RESPONSIVE {
        return Some(Pick::Responsive);
    }
    let within = |base: u32, len: usize| id.checked_sub(base).map(|i| i as usize).filter(|&i| i < len);
    if let Some(i) = within(MENU_UA_BASE, UA_PRESETS.len()) {
        return Some(Pick::UserAgent(i));
    }
    if let Some(i) = within(MENU_SIZE_BASE, SIZE_PRESETS.len()) {
        return Some(Pick::Size(i));
    }
    within(MENU_DPR_BASE, DPRS.len()).map(|i| Pick::Dpr(DPRS[i]))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Responsive {
    pub w: u32,
    pub h: u32,
    pub dpr: u32,
}

#[derive(Default)]
pub struct DevMenu {
    pub ua: usize,
    pub responsive: Option<Responsive>,
}

impl DevMenu {
    pub fn user_agent(&self) -> Option<&'static str> {
        Some(UA_PRESETS[self.ua].1).filter(|s| !s.is_empty())
    }
    pub fn set_ua(&mut self, i: usize) {
        self.ua = i.min(UA_PRESETS.len() - 1);
    }
    /// The Develop menu: the user agent presets, Responsive Design Mode and, while it is on, its size and pixel ratio.
    /// `page` says whether a page is open.
    pub fn menu_items(&self, page: bool) -> Vec<AppMenuItem> {
        let on = self.responsive.is_some();
        let mut items = vec![AppMenuItem::submenu("User Agent")];
        for (i, (name, _)) in UA_PRESETS.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_UA_BASE + i as u32, name).depth(1).checked(i == self.ua));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::new(MENU_RESPONSIVE, "Responsive Design Mode").key("cmd+alt+r").checked(on).enabled(page));
        items.push(AppMenuItem::submenu("Responsive Size").enabled(on));
        for (i, (name, w, h)) in SIZE_PRESETS.iter().enumerate() {
            let same = self.responsive.as_ref().is_some_and(|r| (r.w, r.h) == (*w, *h));
            items.push(AppMenuItem::new(MENU_SIZE_BASE + i as u32, name).depth(1).checked(same).enabled(on));
        }
        items.push(AppMenuItem::submenu("Device Pixel Ratio").enabled(on));
        for (i, d) in DPRS.iter().enumerate() {
            let same = self.responsive.as_ref().is_some_and(|r| r.dpr == *d);
            items.push(AppMenuItem::new(MENU_DPR_BASE + i as u32, &format!("{d}x")).depth(1).checked(same).enabled(on));
        }
        items
    }
    pub fn toggle_responsive(&mut self) {
        self.responsive = match self.responsive {
            Some(_) => None,
            None => Some(Responsive { w: SIZE_PRESETS[0].1, h: SIZE_PRESETS[0].2, dpr: 2 }),
        };
    }
    pub fn preset(&mut self, i: usize) {
        if let (Some(r), Some(&(_, w, h))) = (&mut self.responsive, SIZE_PRESETS.get(i)) {
            r.w = w;
            r.h = h;
        }
    }
    pub fn set_dpr(&mut self, dpr: u32) {
        if let Some(r) = &mut self.responsive {
            r.dpr = dpr.clamp(1, 3);
        }
    }
    pub fn readout(&self) -> Option<String> {
        self.responsive.as_ref().map(|r| format!("{} x {} @{}x", r.w, r.h, r.dpr))
    }
}

/// Fetches the page source with curl (the engine has no raw-response verb); empty text on failure.
pub fn fetch_source(url: &str) -> String {
    std::process::Command::new("curl")
        .args(["-sL", "--max-time", "15", "--max-filesize", "4000000", url])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// True for plain `http:` pages (the "Not Secure" indicator).
pub fn not_secure(url: &str) -> bool {
    url.starts_with("http://")
}

/// A plain preformatted document with line numbers for `source` (opened as a `data:` URL).
pub fn source_document(url: &str, source: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let rows: String = source.lines().enumerate().map(|(i, l)| format!("<tr><td class=n>{}</td><td>{}</td></tr>", i + 1, esc(l))).collect();
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>view-source:{}</title><style>body{{margin:0;font:12px ui-monospace,monospace;background:Canvas;color:CanvasText}}\
         td{{white-space:pre;padding:0 8px}}.n{{text-align:right;color:GrayText;user-select:none}}</style><table cellspacing=0>{rows}</table>",
        esc(url)
    );
    let enc: String = html.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("data:text/html;charset=utf-8,{enc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_size_and_dpr_follow_the_menu() {
        let mut d = DevMenu::default();
        d.set_dpr(9);
        assert_eq!(d.readout(), None, "off by default");
        d.toggle_responsive();
        d.preset(1);
        d.set_dpr(9);
        assert_eq!(d.readout().as_deref(), Some("820 x 1180 @3x"));
        d.preset(9);
        assert_eq!(d.readout().as_deref(), Some("820 x 1180 @3x"), "an unknown preset is ignored");
    }

    #[test]
    fn the_default_user_agent_is_none_and_choices_clamp() {
        let mut d = DevMenu::default();
        assert_eq!(d.user_agent(), None);
        d.set_ua(99);
        assert_eq!(d.ua, UA_PRESETS.len() - 1);
        assert!(d.user_agent().is_some());
    }

    #[test]
    fn menu_ids_map_back_to_their_choice() {
        assert_eq!(pick(MENU_RESPONSIVE), Some(Pick::Responsive));
        assert_eq!(pick(MENU_UA_BASE + 2), Some(Pick::UserAgent(2)));
        assert_eq!(pick(MENU_UA_BASE + UA_PRESETS.len() as u32), None);
        assert_eq!(pick(MENU_SIZE_BASE + 1), Some(Pick::Size(1)));
        assert_eq!(pick(MENU_DPR_BASE + 2), Some(Pick::Dpr(3)));
        assert_eq!(pick(10), None);
    }

    #[test]
    fn source_document_numbers_lines_and_escapes() {
        let u = source_document("http://a", "<p>\nx");
        assert!(u.starts_with("data:text/html"));
        assert!(!u.contains('<'));
        assert!(not_secure("http://a") && !not_secure("https://a"));
    }
}
