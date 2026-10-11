//! `NativeFontPanel` and `NativeColorPanel` models: shared panels with change callbacks. Pure state; the shell draws them with
//! the Golden Gate kit (named colour tokens and `MAC_*` sizes only, Light and Dark through the usual `Pair`s).

/// `NativeFontManager` selection: family, style and size.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSelection {
    pub family: String,
    pub style: String,
    pub size: f32,
}

/// Sizes in the panel's size list.
pub const FONT_SIZES: [f32; 14] = [9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 18.0, 24.0, 36.0, 48.0, 64.0, 72.0, 96.0, 144.0];

pub struct FontPanel {
    /// `(family, styles)` from the fonts the renderer has.
    pub families: Vec<(String, Vec<String>)>,
    selection: FontSelection,
    on_change: Option<Box<dyn FnMut(&FontSelection)>>,
}

impl FontPanel {
    pub fn new(families: Vec<(String, Vec<String>)>, initial: FontSelection) -> Self {
        FontPanel { families, selection: initial, on_change: None }
    }
    /// `NativeFontManager.target` action: called after every change.
    pub fn on_change(&mut self, f: impl FnMut(&FontSelection) + 'static) {
        self.on_change = Some(Box::new(f));
    }
    pub fn selection(&self) -> &FontSelection {
        &self.selection
    }
    fn fire(&mut self) {
        if let Some(f) = self.on_change.as_mut() {
            f(&self.selection);
        }
    }
    /// Picks a family; keeps the style when the family has it, else its first style.
    pub fn set_family(&mut self, family: &str) -> bool {
        let Some((_, styles)) = self.families.iter().find(|(f, _)| f == family) else { return false };
        if !styles.contains(&self.selection.style) {
            self.selection.style = styles.first().cloned().unwrap_or_default();
        }
        self.selection.family = family.into();
        self.fire();
        true
    }
    pub fn set_style(&mut self, style: &str) -> bool {
        let ok = self.families.iter().any(|(f, s)| *f == self.selection.family && s.iter().any(|x| x == style));
        if ok {
            self.selection.style = style.into();
            self.fire();
        }
        ok
    }
    /// Size is clamped to 1..=288 like the panel's field.
    pub fn set_size(&mut self, size: f32) {
        self.selection.size = size.clamp(1.0, 288.0);
        self.fire();
    }
}

/// HSB colour, hue 0..360, saturation and brightness 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsb {
    pub h: f32,
    pub s: f32,
    pub b: f32,
}

pub fn hsb_to_rgb(c: Hsb) -> (u8, u8, u8) {
    let h = c.h.rem_euclid(360.0) / 60.0;
    let (s, v) = (c.s.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
    let f = h - h.floor();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match h.floor() as u32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let u = |x: f32| (x * 255.0).round() as u8;
    (u(r), u(g), u(b))
}

pub fn rgb_to_hsb(r: u8, g: u8, b: u8) -> Hsb {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Hsb { h, s: if max == 0.0 { 0.0 } else { d / max }, b: max }
}

/// `#RRGGBB` (or `RRGGBB`, `#RGB`) to opaque RGB.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    let full: String = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        6 => s.into(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// The wheel's hit-test: point `(x, y)` relative to the wheel centre with radius `r` gives hue and saturation.
pub fn wheel_pick(x: f32, y: f32, r: f32, brightness: f32) -> Option<Hsb> {
    let d = (x * x + y * y).sqrt();
    if r <= 0.0 || d > r {
        return None;
    }
    Some(Hsb { h: y.atan2(x).to_degrees().rem_euclid(360.0), s: d / r, b: brightness })
}

pub struct ColorPanel {
    color: (u8, u8, u8),
    alpha: f32,
    on_change: Option<Box<dyn FnMut((u8, u8, u8), f32)>>,
}

impl ColorPanel {
    pub fn new(initial: (u8, u8, u8)) -> Self {
        ColorPanel { color: initial, alpha: 1.0, on_change: None }
    }
    /// `NativeColorPanel.setTarget/setAction`.
    pub fn on_change(&mut self, f: impl FnMut((u8, u8, u8), f32) + 'static) {
        self.on_change = Some(Box::new(f));
    }
    pub fn color(&self) -> (u8, u8, u8) {
        self.color
    }
    pub fn alpha(&self) -> f32 {
        self.alpha
    }
    fn fire(&mut self) {
        let (c, a) = (self.color, self.alpha);
        if let Some(f) = self.on_change.as_mut() {
            f(c, a);
        }
    }
    pub fn set_rgb(&mut self, c: (u8, u8, u8)) {
        self.color = c;
        self.fire();
    }
    pub fn set_hsb(&mut self, c: Hsb) {
        self.set_rgb(hsb_to_rgb(c));
    }
    pub fn set_alpha(&mut self, a: f32) {
        self.alpha = a.clamp(0.0, 1.0);
        self.fire();
    }
    /// Hex field commit; invalid text changes nothing.
    pub fn set_hex(&mut self, s: &str) -> bool {
        match parse_hex(s) {
            Some(c) => {
                self.set_rgb(c);
                true
            }
            None => false,
        }
    }
    pub fn hex(&self) -> String {
        to_hex(self.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn hsb_and_rgb_round_trip() {
        for c in [(255, 0, 0), (10, 132, 255), (48, 209, 88), (0, 0, 0), (255, 255, 255)] {
            assert_eq!(hsb_to_rgb(rgb_to_hsb(c.0, c.1, c.2)), c);
        }
    }
    #[test]
    fn hex_parses_short_long_and_rejects_junk() {
        assert_eq!(parse_hex("#0A84FF"), Some((10, 132, 255)));
        assert_eq!(parse_hex("f00"), Some((255, 0, 0)));
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(to_hex((10, 132, 255)), "#0A84FF");
    }
    #[test]
    fn wheel_pick_is_inside_the_disc_only() {
        assert!(wheel_pick(10.0, 0.0, 5.0, 1.0).is_none());
        let p = wheel_pick(0.0, 5.0, 10.0, 1.0).unwrap();
        assert_eq!((p.h, p.s), (90.0, 0.5));
    }
    #[test]
    fn color_panel_fires_callback_on_change() {
        let n = Rc::new(Cell::new(0));
        let n2 = n.clone();
        let mut p = ColorPanel::new((0, 0, 0));
        p.on_change(move |_, _| n2.set(n2.get() + 1));
        p.set_hex("#FF0000");
        assert!(!p.set_hex("nope"));
        p.set_alpha(2.0);
        assert_eq!((n.get(), p.alpha(), p.hex()), (2, 1.0, "#FF0000".to_string()));
    }
    #[test]
    fn font_panel_keeps_style_when_family_has_it() {
        let fams = vec![("Sans".into(), vec!["Regular".into(), "Bold".into()]), ("Mono".into(), vec!["Regular".into()])];
        let mut p = FontPanel::new(fams, FontSelection { family: "Sans".into(), style: "Bold".into(), size: 13.0 });
        assert!(p.set_family("Mono"));
        assert_eq!(p.selection().style, "Regular");
        assert!(!p.set_family("Nope"));
        assert!(!p.set_style("Bold"));
        p.set_size(1000.0);
        assert_eq!(p.selection().size, 288.0);
    }
}
