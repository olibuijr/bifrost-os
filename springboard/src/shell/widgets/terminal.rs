//! The the terminal core widget: the last lines of the running shell in SF Mono.

use super::{put, tile, Kind, PAD_PT};
use crate::font::{Font, Style};
use crate::gfx::Image;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};

/// How many lines the widget shows.
pub const LINES: usize = 3;
const MONO_PT: f32 = 13.0;
const LINE_PT: f32 = 22.0;

/// `lines` is None while the shell is not running.
pub fn key(lines: Option<&[String]>) -> String {
    match lines {
        Some(l) => format!("terminal:{}", l.join("\n")),
        None => "terminal:off".into(),
    }
}

pub fn render(ui: &Ui, mono: &Font, lines: Option<&[String]>) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Terminal.size(s);
    let mut img = tile(s, w, h);
    let pad = ui.pt(PAD_PT);
    let title = ui.text("Term", 13.0, Weight::Semibold, SECONDARY);
    put(&mut img, &title, pad, pad - ui.pt(2.0));
    let top = ui.pt(44.0);
    match lines {
        Some(lines) => {
            let px = ui.pt(MONO_PT);
            let max_w = w as f32 - 2.0 * pad;
            for (i, line) in lines.iter().rev().take(LINES).collect::<Vec<_>>().into_iter().rev().enumerate() {
                let text = mono.fit(line.trim_end(), px, max_w);
                let img_line = mono.render(&text, &Style { px, embolden: 0.0, color: WHITE, shadow: false });
                put(&mut img, &img_line, pad, top + i as f32 * ui.pt(LINE_PT));
            }
            let foot = ui.text("Running", 12.0, Weight::Medium, SECONDARY);
            put(&mut img, &foot, pad, h as f32 - pad - foot.h as f32);
        }
        None => {
            let off = ui.text("Not running", 17.0, Weight::Regular, SECONDARY);
            put(&mut img, &off, pad, top);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono() -> Font<'static> {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice());
        Font::parse(data).unwrap()
    }

    #[test]
    fn shows_the_last_three_lines_and_an_off_state() {
        let ui = crate::shell::tests::ui();
        let mono = mono();
        let lines: Vec<String> = (1..=5).map(|i| format!("line {i}")).collect();
        assert_eq!(key(Some(&lines[2..])), "terminal:line 3\nline 4\nline 5");
        assert_ne!(key(Some(&lines)), key(None));
        let img = render(&ui, &mono, Some(&lines));
        assert_eq!((img.w, img.h), Kind::Terminal.size(ui.s));
        let row = |y: f32| {
            let y = y as usize;
            img.px[y * img.w..(y + 1) * img.w].iter().filter(|&&p| p & 0xff > 200).count()
        };
        assert!(row(ui.pt(44.0 + 10.0)) > 0, "first shown line has ink");
        assert!(row(ui.pt(44.0 + 2.0 * 22.0 + 10.0)) > 0, "third shown line has ink");
        assert_eq!(row(ui.pt(44.0 + 3.0 * 22.0 + 10.0)), 0, "no fourth line");
        let off = render(&ui, &mono, None);
        assert_ne!(off.px, img.px);
    }
}
