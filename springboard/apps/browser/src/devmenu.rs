//! Develop menu state: View Source, Reload From Origin, user agent presets and Responsive Design Mode
//! (Browser's Develop menu, `WebView.customUserAgent`).

pub const UA_PRESETS: [(&str, &str); 5] = [
    ("Default", ""),
    ("Browser Golden Gate", "Mozilla/5.0 (Macintosh; Intel Golden Gate 10_15_7) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Browser/605.1.15"),
    ("Browser bifrost", "Mozilla/5.0 (bifrost; CPU bifrost OS 17_5 like Golden Gate) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Browser/604.1"),
    ("Chrome Android", "Mozilla/5.0 (Linux; Android 14; Pixel 8) WebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Browser/537.36"),
    ("Firefox", "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"),
];

/// Responsive presets: name, CSS width, CSS height.
pub const SIZE_PRESETS: [(&str, u32, u32); 3] = [("bifrost 15", 393, 852), ("bifrost", 820, 1180), ("1280x800", 1280, 800)];

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
    /// Custom `WxH`; false when the text does not parse or the mode is off.
    pub fn custom(&mut self, text: &str) -> bool {
        let Some((w, h)) = text.trim().split_once(['x', 'X']) else { return false };
        match (w.trim().parse::<u32>(), h.trim().parse::<u32>(), &mut self.responsive) {
            (Ok(w), Ok(h), Some(r)) if w > 0 && h > 0 => {
                r.w = w.min(8192);
                r.h = h.min(8192);
                true
            }
            _ => false,
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
    fn responsive_custom_size_and_dpr() {
        let mut d = DevMenu::default();
        assert!(!d.custom("10x10"), "off by default");
        d.toggle_responsive();
        assert!(d.custom("1024 x 600"));
        d.set_dpr(9);
        assert_eq!(d.readout().as_deref(), Some("1024 x 600 @3x"));
        assert!(!d.custom("abc"));
    }

    #[test]
    fn source_document_numbers_lines_and_escapes() {
        let u = source_document("http://a", "<p>\nx");
        assert!(u.starts_with("data:text/html"));
        assert!(!u.contains('<'));
        assert!(not_secure("http://a") && !not_secure("https://a"));
    }
}
