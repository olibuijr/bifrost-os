//! Preview: the preview panel of the focused file (picture, text, or icon and info).
use crate::*;

/// Pictures larger than this are not decoded for the preview.
const PREVIEW_MAX_BYTES: u64 = 32 << 20;

impl Files {
    pub(crate) fn peek(&mut self) {
        self.info = None;
        self.fx.preview_image = None;
        let Some(e) = self.focus_entry().cloned() else {
            return;
        };
        let ext = e.path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg") && e.size <= PREVIEW_MAX_BYTES {
            if let Some(img) = std::fs::read(&e.path).ok().and_then(|b| springboard::imgdec::decode(&b)) {
                self.fx.preview_image = Some(img);
            }
        }
        let content = if e.kind == Kind::Text {
            fs::read_text(&e.path)
                .map(|(s, _)| s.chars().take(4000).collect())
                .unwrap_or_else(|e| e.to_string())
        } else {
            let wall = Wall::sample(bifrost_app::proto::sys::now_ms());
            let rows = springboard::files::info::rows(&e, None, springboard::files::info::created_at(&e.path), &self.tag_index, wall.unix, wall.offset);
            let mut lines: Vec<String> = rows.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            lines.push(String::new());
            lines.push("Press Enter to open with its desktop application.".into());
            lines.join("\n")
        };
        self.preview = Some((e.name.clone(), content));
    }
    /// The Preview panel.
    pub(crate) fn paint_preview(&self, ui: &Ui, s: &mut Surface) {
        let (w, h) = (self.w, self.h);
        if let Some((name, content)) = &self.preview {
            let x = self.side() + 16.;
            let y = 72.;
            rect(s, ui, x, y, w - x - 16., h - y - 64., CARD_BG.now());
            text(s, ui, name, 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
            let body_y = y + 56.;
            if let Some(img) = &self.fx.preview_image {
                // fit the picture inside the panel, never enlarging past its own size
                let (max_w, max_h) = (ui.pt(w - x - 56.), ui.pt(h - body_y - 84.));
                let k = (max_w / img.w as f32).min(max_h / img.h as f32).min(1.);
                let (iw, ih) = (((img.w as f32 * k) as usize).max(1), ((img.h as f32 * k) as usize).max(1));
                let fitted = img.resized(iw, ih);
                s.blit(&fitted, ui.pt(x + 20.) as i32, ui.pt(body_y) as i32, 255, 255);
                return;
            }
            // no picture: the file's icon above its information
            let mut text_y = body_y;
            if let Some(e) = self.focus_entry().filter(|e| e.kind != Kind::Text) {
                if e.is_dir {
                    widgets::folder_icon(s, ui, Rect { x: x + 20., y: body_y, w: 64., h: 64. }, folder_emblem(&self.home, &e.path));
                } else {
                    icons::entry(s, (ui.pt(x + 26.), ui.pt(body_y + 4.), ui.pt(52.), ui.pt(56.)), e.kind);
                }
                text_y += 80.;
            }
            for (i, line) in content
                .lines()
                .take(((h - text_y - 74.) / 20.).max(0.) as usize)
                .enumerate()
            {
                text(s, ui, line, 13., LABEL_2.now(), x + 20., text_y + i as f32 * 20., w - x - 56.);
            }
        }
    }
}
