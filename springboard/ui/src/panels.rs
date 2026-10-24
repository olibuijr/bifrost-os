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

/// `NativeColorPanel.Mode`: the picker the panel shows. The raw values are the reference's (`None` is -1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelMode {
    None,
    Gray,
    Rgb,
    Cmyk,
    Hsb,
    CustomPalette,
    ColorList,
    Wheel,
    Crayon,
}

impl PanelMode {
    /// Every picker, in raw-value order.
    pub const ALL: [PanelMode; 8] = [
        PanelMode::Gray,
        PanelMode::Rgb,
        PanelMode::Cmyk,
        PanelMode::Hsb,
        PanelMode::CustomPalette,
        PanelMode::ColorList,
        PanelMode::Wheel,
        PanelMode::Crayon,
    ];

    pub fn raw(self) -> i32 {
        match self {
            PanelMode::None => -1,
            m => PanelMode::ALL.iter().position(|&x| x == m).unwrap_or(0) as i32,
        }
    }

    pub fn from_raw(raw: i32) -> Option<PanelMode> {
        if raw == -1 {
            return Some(PanelMode::None);
        }
        usize::try_from(raw).ok().and_then(|i| PanelMode::ALL.get(i).copied())
    }

    /// The bit of this picker in a [`PanelModes`] mask (`1 << raw`); `None` has no bit.
    pub fn bit(self) -> u32 {
        match self {
            PanelMode::None => 0,
            m => 1 << m.raw(),
        }
    }

    /// The slider labels of the pickers that edit the colour with sliders, in slider order; empty for the others.
    pub fn slider_labels(self) -> &'static [&'static str] {
        match self {
            PanelMode::Gray => &["Brightness"],
            PanelMode::Rgb => &["Red", "Green", "Blue"],
            PanelMode::Cmyk => &["Cyan", "Magenta", "Yellow", "Black"],
            PanelMode::Hsb => &["Hue", "Saturation", "Brightness"],
            _ => &[],
        }
    }
}

/// `NativeColorPanel.Options`: the pickers a panel offers, one bit per [`PanelMode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelModes(pub u32);

impl PanelModes {
    /// Every picker (the reference's all-modes mask).
    pub const ALL: PanelModes = PanelModes(0xffff);

    pub fn of(modes: &[PanelMode]) -> PanelModes {
        PanelModes(modes.iter().fold(0, |m, x| m | x.bit()))
    }

    pub fn contains(self, mode: PanelMode) -> bool {
        mode != PanelMode::None && self.0 & mode.bit() != 0
    }

    /// The pickers in the mask, in raw-value order.
    pub fn modes(self) -> Vec<PanelMode> {
        PanelMode::ALL.iter().copied().filter(|&m| self.contains(m)).collect()
    }
}

/// Device grey from RGB (Rec. 601 weights), 0..1.
pub fn rgb_to_gray((r, g, b): (u8, u8, u8)) -> f32 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
}

pub fn gray_to_rgb(white: f32) -> (u8, u8, u8) {
    let v = (white.clamp(0.0, 1.0) * 255.0).round() as u8;
    (v, v, v)
}

/// Naive device CMYK (no profile) from RGB: `[c, m, y, k]`, each 0..1.
pub fn rgb_to_cmyk((r, g, b): (u8, u8, u8)) -> [f32; 4] {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let f = |x: f32| (1.0 - x - k) / (1.0 - k);
    [f(r), f(g), f(b), k]
}

pub fn cmyk_to_rgb([c, m, y, k]: [f32; 4]) -> (u8, u8, u8) {
    let k = k.clamp(0.0, 1.0);
    let ch = |x: f32| ((1.0 - x.clamp(0.0, 1.0)) * (1.0 - k) * 255.0).round() as u8;
    (ch(c), ch(m), ch(y))
}

pub struct ColorPanel {
    color: (u8, u8, u8),
    alpha: f32,
    on_change: Option<Box<dyn FnMut((u8, u8, u8), f32)>>,
    mode: PanelMode,
    mask: PanelModes,
    shows_alpha: bool,
    continuous: bool,
    lists: Vec<String>,
}

impl ColorPanel {
    /// A panel on `initial`, opaque, showing the wheel with every picker allowed, continuous, with the opacity slider
    /// hidden (the reference's defaults; the alpha is still stored and reported).
    pub fn new(initial: (u8, u8, u8)) -> Self {
        ColorPanel {
            color: initial,
            alpha: 1.0,
            on_change: None,
            mode: PanelMode::Wheel,
            mask: PanelModes::ALL,
            shows_alpha: false,
            continuous: true,
            lists: Vec::new(),
        }
    }
    /// The picker on show.
    pub fn mode(&self) -> PanelMode {
        self.mode
    }
    /// Shows `mode`. Refused (false, nothing changes) when the mask does not offer it; `None` is always accepted.
    pub fn set_mode(&mut self, mode: PanelMode) -> bool {
        if mode != PanelMode::None && !self.mask.contains(mode) {
            return false;
        }
        self.mode = mode;
        true
    }
    pub fn mask(&self) -> PanelModes {
        self.mask
    }
    /// `setPickerMask`: the pickers offered. When the picker on show leaves the mask, the panel moves to the first one
    /// left (or `None` for an empty mask).
    pub fn set_mask(&mut self, mask: PanelModes) {
        self.mask = mask;
        if self.mode != PanelMode::None && !mask.contains(self.mode) {
            self.mode = mask.modes().first().copied().unwrap_or(PanelMode::None);
        }
    }
    /// `showsAlpha`: whether the opacity slider and field are drawn.
    pub fn shows_alpha(&self) -> bool {
        self.shows_alpha
    }
    pub fn set_shows_alpha(&mut self, on: bool) {
        self.shows_alpha = on;
    }
    /// `isContinuous`: whether a drag reports every step (true) or only the step where it ends.
    pub fn is_continuous(&self) -> bool {
        self.continuous
    }
    pub fn set_continuous(&mut self, on: bool) {
        self.continuous = on;
    }
    /// A step of a slider or wheel drag. The colour always changes; the callback runs on every step when the panel is
    /// continuous, otherwise only on the step with `done` (the release).
    pub fn track_rgb(&mut self, c: (u8, u8, u8), done: bool) {
        self.color = c;
        if self.continuous || done {
            self.fire();
        }
    }
    /// `attachColorList`: a colour list (by name) the list picker offers. Attaching a name twice keeps one entry.
    pub fn attach_color_list(&mut self, name: &str) -> bool {
        if name.is_empty() || self.lists.iter().any(|n| n == name) {
            return false;
        }
        self.lists.push(name.to_string());
        true
    }
    pub fn detach_color_list(&mut self, name: &str) -> bool {
        let before = self.lists.len();
        self.lists.retain(|n| n != name);
        self.lists.len() != before
    }
    /// The attached colour lists, in the order they were attached.
    pub fn color_lists(&self) -> &[String] {
        &self.lists
    }
    /// The slider values of the current colour in `mode` (see [`PanelMode::slider_labels`]): grey 0..1, RGB 0..255,
    /// CMYK 0..1, HSB hue 0..360 with saturation and brightness 0..1. Empty for the pickers without sliders.
    pub fn components(&self, mode: PanelMode) -> Vec<f32> {
        let (r, g, b) = self.color;
        match mode {
            PanelMode::Gray => vec![rgb_to_gray(self.color)],
            PanelMode::Rgb => vec![r as f32, g as f32, b as f32],
            PanelMode::Cmyk => rgb_to_cmyk(self.color).to_vec(),
            PanelMode::Hsb => {
                let h = rgb_to_hsb(r, g, b);
                vec![h.h, h.s, h.b]
            }
            _ => Vec::new(),
        }
    }
    /// Moves slider `index` of `mode` to `value` (in the units of [`components`](Self::components)), keeping the
    /// other components, as one drag step (`done` on the release). Returns false for a picker without sliders or an
    /// index past its sliders.
    pub fn set_component(&mut self, mode: PanelMode, index: usize, value: f32, done: bool) -> bool {
        let mut v = self.components(mode);
        if index >= v.len() {
            return false;
        }
        v[index] = value;
        let c = match mode {
            PanelMode::Gray => gray_to_rgb(v[0]),
            PanelMode::Rgb => {
                let u = |x: f32| x.clamp(0.0, 255.0).round() as u8;
                (u(v[0]), u(v[1]), u(v[2]))
            }
            PanelMode::Cmyk => cmyk_to_rgb([v[0], v[1], v[2], v[3]]),
            PanelMode::Hsb => hsb_to_rgb(Hsb { h: v[0], s: v[1], b: v[2] }),
            _ => return false,
        };
        self.track_rgb(c, done);
        true
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
    fn panel_modes_keep_the_reference_raw_values_and_mask_bits() {
        assert_eq!(PanelMode::None.raw(), -1);
        assert_eq!(PanelMode::Gray.raw(), 0);
        assert_eq!(PanelMode::Wheel.raw(), 6);
        assert_eq!(PanelMode::Crayon.raw(), 7);
        for m in PanelMode::ALL {
            assert_eq!(PanelMode::from_raw(m.raw()), Some(m));
        }
        assert_eq!(PanelMode::from_raw(8), None);
        assert_eq!(PanelModes::of(&[PanelMode::Gray, PanelMode::Rgb]).0, 0b11);
        assert_eq!(PanelModes::ALL.modes().len(), 8);
    }
    #[test]
    fn the_mask_limits_the_pickers_and_moves_a_hidden_one() {
        let mut p = ColorPanel::new((0, 0, 0));
        assert_eq!((p.mode(), p.is_continuous(), p.shows_alpha()), (PanelMode::Wheel, true, false));
        p.set_mask(PanelModes::of(&[PanelMode::Rgb, PanelMode::ColorList]));
        assert_eq!(p.mode(), PanelMode::Rgb, "the wheel left the mask");
        assert!(!p.set_mode(PanelMode::Hsb));
        assert!(p.set_mode(PanelMode::ColorList));
        p.set_mask(PanelModes(0));
        assert_eq!(p.mode(), PanelMode::None);
    }
    #[test]
    fn slider_pickers_edit_one_component_and_keep_the_rest() {
        let mut p = ColorPanel::new((10, 132, 255));
        assert_eq!(p.components(PanelMode::Rgb), vec![10.0, 132.0, 255.0]);
        assert!(p.set_component(PanelMode::Rgb, 0, 255.0, true));
        assert_eq!(p.color(), (255, 132, 255));
        assert!(p.set_component(PanelMode::Gray, 0, 0.5, true));
        assert_eq!(p.color(), (128, 128, 128));
        assert!(p.set_component(PanelMode::Cmyk, 3, 1.0, true));
        assert_eq!(p.color(), (0, 0, 0));
        assert!(!p.set_component(PanelMode::Wheel, 0, 1.0, true));
        assert!(!p.set_component(PanelMode::Rgb, 3, 1.0, true));
        assert_eq!(PanelMode::Cmyk.slider_labels().len(), p.components(PanelMode::Cmyk).len());
    }
    #[test]
    fn cmyk_round_trips_the_primaries() {
        for c in [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255), (0, 0, 0), (40, 80, 120)] {
            assert_eq!(cmyk_to_rgb(rgb_to_cmyk(c)), c);
        }
    }
    #[test]
    fn a_panel_that_is_not_continuous_reports_only_the_release() {
        let n = Rc::new(Cell::new(0));
        let n2 = n.clone();
        let mut p = ColorPanel::new((0, 0, 0));
        p.on_change(move |_, _| n2.set(n2.get() + 1));
        p.set_continuous(false);
        p.track_rgb((1, 1, 1), false);
        p.track_rgb((2, 2, 2), false);
        assert_eq!((n.get(), p.color()), (0, (2, 2, 2)));
        p.track_rgb((3, 3, 3), true);
        assert_eq!(n.get(), 1);
        p.set_continuous(true);
        p.track_rgb((4, 4, 4), false);
        assert_eq!(n.get(), 2);
    }
    #[test]
    fn colour_lists_attach_once_and_detach() {
        let mut p = ColorPanel::new((0, 0, 0));
        assert!(p.attach_color_list("System"));
        assert!(!p.attach_color_list("System"));
        assert!(p.attach_color_list("Brand"));
        assert_eq!(p.color_lists(), ["System".to_string(), "Brand".to_string()]);
        assert!(p.detach_color_list("System"));
        assert!(!p.detach_color_list("System"));
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
