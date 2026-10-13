//! Process-wide appearance hooks shared by the shell and rendering kit.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
pub const REDUCED_MS: f64 = 150.0;

#[derive(Clone, Copy, Debug)]
pub struct Appearance {
    pub text_scale: f32,
    pub zoom: f32,
    pub hold_ms: u32,
    pub bold: bool,
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub button_shapes: bool,
}

pub fn publish(a: Appearance) {
    let before = display_options();
    TEXT_SCALE.store(a.text_scale.to_bits(), Relaxed);
    ZOOM.store(a.zoom.to_bits(), Relaxed);
    HOLD.store(a.hold_ms, Relaxed);
    BOLD.store(a.bold, Relaxed);
    MOTION.store(a.reduce_motion, Relaxed);
    CONTRAST.store(a.increase_contrast, Relaxed);
    TRANSPARENCY.store(a.reduce_transparency, Relaxed);
    SHAPES.store(a.button_shapes, Relaxed);
    let after = display_options();
    if after != before {
        DISPLAY_OBSERVERS.notify(after);
    }
}

// ---- Accessibility display options (NSWorkspace) --------------------------------------------------------------------
//
// `NSWorkspace.accessibilityDisplayShouldReduceTransparency`, `...ShouldIncreaseContrast` and
// `...ShouldDifferentiateWithoutColor`, with `accessibilityDisplayOptionsDidChangeNotification`. SwiftUI's
// `accessibilityReduceTransparency` and `colorSchemeContrast` read the same two values.

/// The display options an app reads, as one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DisplayOptions {
    /// `accessibilityDisplayShouldReduceTransparency` (also on while Increase Contrast is).
    pub reduce_transparency: bool,
    /// `accessibilityDisplayShouldIncreaseContrast`.
    pub increase_contrast: bool,
    /// `accessibilityDisplayShouldDifferentiateWithoutColor` (Settings > Accessibility > Display > Button Shapes).
    pub differentiate_without_color: bool,
}

/// SwiftUI `ColorSchemeContrast`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSchemeContrast {
    Standard,
    Increased,
}

pub fn display_options() -> DisplayOptions {
    DisplayOptions { reduce_transparency: reduce_transparency(), increase_contrast: increase_contrast(), differentiate_without_color: button_shapes() }
}
/// `NSWorkspace.accessibilityDisplayShouldDifferentiateWithoutColor`.
pub fn differentiate_without_color() -> bool {
    button_shapes()
}
/// SwiftUI `@Environment(\.colorSchemeContrast)`.
pub fn color_scheme_contrast() -> ColorSchemeContrast {
    if increase_contrast() { ColorSchemeContrast::Increased } else { ColorSchemeContrast::Standard }
}

/// Observers of display option changes (`accessibilityDisplayOptionsDidChangeNotification`).
pub struct Observers {
    list: std::sync::Mutex<(u64, Vec<(u64, std::sync::Arc<dyn Fn(DisplayOptions) + Send + Sync>)>)>,
}

impl Observers {
    pub const fn new() -> Self {
        Observers { list: std::sync::Mutex::new((0, Vec::new())) }
    }
    /// Registers `f`; the returned token removes it again.
    pub fn add(&self, f: impl Fn(DisplayOptions) + Send + Sync + 'static) -> u64 {
        let mut g = self.list.lock().unwrap_or_else(|e| e.into_inner());
        g.0 += 1;
        let id = g.0;
        g.1.push((id, std::sync::Arc::new(f)));
        id
    }
    pub fn remove(&self, token: u64) {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).1.retain(|(id, _)| *id != token);
    }
    /// Calls every observer (outside the lock, so an observer may add or remove observers).
    pub fn notify(&self, o: DisplayOptions) {
        let fs: Vec<_> = self.list.lock().unwrap_or_else(|e| e.into_inner()).1.iter().map(|(_, f)| f.clone()).collect();
        for f in fs {
            f(o);
        }
    }
}

static DISPLAY_OBSERVERS: Observers = Observers::new();

/// Calls `f` whenever a published change moves one of the display options. Returns a token for `remove_observer`.
pub fn observe_display_options(f: impl Fn(DisplayOptions) + Send + Sync + 'static) -> u64 {
    DISPLAY_OBSERVERS.add(f)
}
pub fn remove_observer(token: u64) {
    DISPLAY_OBSERVERS.remove(token)
}

/// The system-wide value file the shell writes (`/run/bifrost/user/accessibility`) for processes without the shell's
/// IPC (the window server).
pub const DISPLAY_FILE: &str = "/run/bifrost/user/accessibility";

impl DisplayOptions {
    /// `key=0|1` lines.
    pub fn to_text(self) -> String {
        format!("reduce_transparency={}\nincrease_contrast={}\ndifferentiate_without_color={}\n", self.reduce_transparency as u8, self.increase_contrast as u8, self.differentiate_without_color as u8)
    }
    /// Missing or unknown keys read as off. Increase Contrast implies Reduce Transparency, as in `reduce_transparency`.
    pub fn parse(text: &str) -> Self {
        let mut o = DisplayOptions::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let on = v.trim() == "1";
            match k.trim() {
                "reduce_transparency" => o.reduce_transparency = on,
                "increase_contrast" => o.increase_contrast = on,
                "differentiate_without_color" => o.differentiate_without_color = on,
                _ => {}
            }
        }
        o.reduce_transparency |= o.increase_contrast;
        o
    }
    /// Writes `path` atomically (temp file, rename). Best effort for the caller: a missing directory is an error.
    pub fn write_to(self, path: &str) -> std::io::Result<()> {
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
    /// Reads `path`; a missing or unreadable file is all off.
    pub fn read_from(path: &str) -> Self {
        std::fs::read_to_string(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }
}

/// The settings as they are now (a starting point for changing one field and publishing).
pub fn current() -> Appearance {
    Appearance {
        text_scale: text_scale(),
        zoom: display_zoom(),
        hold_ms: touch_hold_ms(),
        bold: bold_text(),
        reduce_motion: reduce_motion(),
        increase_contrast: increase_contrast(),
        reduce_transparency: reduce_transparency(),
        button_shapes: button_shapes(),
    }
}


// ---- Light / dark scheme -------------------------------------------------------------------------------------------
//
// One process-wide scheme position: 0.0 is Dark, 1.0 is Light. Changing the scheme starts an eased transition from
// wherever the position is now, so flipping again mid-fade reverses smoothly. Colours that follow the scheme are `Pair`s
// resolved with `Pair::now()`; text that follows it is rendered white and blitted with the resolved colour as the tint.
use std::sync::atomic::AtomicU64;

/// How long the dark/light cross-fade takes.
pub const SCHEME_FADE_MS: f64 = 280.0;

static SCHEME_FROM: AtomicU32 = AtomicU32::new(0); // f32 bits, position when the fade began
static SCHEME_TO: AtomicU32 = AtomicU32::new(0); // f32 bits, 0.0 dark / 1.0 light
static SCHEME_T0: AtomicU64 = AtomicU64::new(0); // f64 bits, start time in ms
static SCHEME_POS: AtomicU32 = AtomicU32::new(0); // f32 bits, position at the last `tick`

fn getf(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// What the user chose in Settings > Appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
    /// Light by day, dark by night (`auto_is_light`).
    Auto,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::Auto => "auto",
        }
    }
    pub fn parse(s: &str) -> Mode {
        match s {
            "light" => Mode::Light,
            "auto" => Mode::Auto,
            _ => Mode::Dark,
        }
    }
    /// Whether the scheme is light for local hour `hour` (0..24).
    pub fn is_light(self, hour: u32) -> bool {
        match self {
            Mode::Light => true,
            Mode::Dark => false,
            Mode::Auto => auto_is_light(hour),
        }
    }
}

/// Auto mode: light from 07:00 to 18:59.
pub fn auto_is_light(hour: u32) -> bool {
    (7..19).contains(&(hour % 24))
}

/// Starts a fade to light (`true`) or dark at `now_ms`. A no-op when already heading there. With Reduce Motion on the
/// scheme switches at once.
pub fn set_light(light: bool, now_ms: f64) {
    let to = if light { 1.0f32 } else { 0.0 };
    if getf(&SCHEME_TO) == to {
        return;
    }
    let from = getf(&SCHEME_POS);
    SCHEME_FROM.store(from.to_bits(), Relaxed);
    SCHEME_TO.store(to.to_bits(), Relaxed);
    SCHEME_T0.store(now_ms.to_bits(), Relaxed);
    if reduce_motion() {
        SCHEME_POS.store(to.to_bits(), Relaxed);
        SCHEME_FROM.store(to.to_bits(), Relaxed);
    }
}

/// Moves the scheme position to `now_ms`; true while the fade is still running (the caller needs another frame).
pub fn tick(now_ms: f64) -> bool {
    let (from, to) = (getf(&SCHEME_FROM), getf(&SCHEME_TO));
    let t0 = f64::from_bits(SCHEME_T0.load(Relaxed));
    let x = ((now_ms - t0) / SCHEME_FADE_MS).clamp(0.0, 1.0) as f32;
    let pos = if from == to { to } else { from + (to - from) * ease(x) };
    SCHEME_POS.store(pos.to_bits(), Relaxed);
    pos != to
}

static MODE: AtomicU32 = AtomicU32::new(1); // 0 light, 1 dark, 2 auto

/// Records the user's chosen mode so Settings can show it (the shell publishes it on every tick).
pub fn publish_mode(m: Mode) {
    MODE.store(match m { Mode::Light => 0, Mode::Dark => 1, Mode::Auto => 2 }, Relaxed);
}

/// The mode last published.
pub fn mode() -> Mode {
    match MODE.load(Relaxed) {
        0 => Mode::Light,
        2 => Mode::Auto,
        _ => Mode::Dark,
    }
}

/// The name Settings shows for a mode.
pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Light => "Light",
        Mode::Dark => "Dark",
        Mode::Auto => "Auto",
    }
}

/// True while the scheme is between its start and its target (frames are needed to draw the fade).
pub fn fading() -> bool {
    getf(&SCHEME_POS) != getf(&SCHEME_TO)
}

/// 0.0 dark .. 1.0 light, as of the last `tick`.
pub fn scheme() -> f32 {
    getf(&SCHEME_POS)
}

/// The scheme the fade is heading for.
pub fn target_is_light() -> bool {
    getf(&SCHEME_TO) >= 0.5
}

/// Sets the scheme with no fade (startup, tests).
pub fn jump_to(light: bool) {
    let v = if light { 1.0f32 } else { 0.0 };
    for a in [&SCHEME_FROM, &SCHEME_TO, &SCHEME_POS] {
        a.store(v.to_bits(), Relaxed);
    }
}

/// Linear mix of two premultiplied ARGB colours (`pack` order) at `p` (0 = `dark`, 1 = `light`).
pub fn mix(dark: u32, light: u32, p: f32) -> u32 {
    let p = p.clamp(0.0, 1.0);
    let ch = |i: u32| {
        let (a, b) = (((dark >> i) & 255) as f32, ((light >> i) & 255) as f32);
        ((a + (b - a) * p).round() as u32) << i
    };
    ch(0) | ch(8) | ch(16) | ch(24)
}

/// A colour with a dark and a light value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub dark: u32,
    pub light: u32,
}

impl Pair {
    pub const fn new(dark: u32, light: u32) -> Self {
        Pair { dark, light }
    }
    /// An opaque tinted fill from an accent colour (packed `r | g<<8 | b<<16 | a<<24`, opaque): the accent at 18%
    /// over white in Light and 32% over a dark grey in Dark, as Calendar event blocks and list colour plates
    /// (`NativeColor.withSystemEffect` / `color.withAlphaComponent` over the window background).
    pub fn tinted(accent: u32) -> Pair {
        Pair::tinted_by(accent, 0.32, 0.18)
    }
    /// As `tinted` with explicit accent shares for Dark and Light (0..=1).
    pub fn tinted_by(accent: u32, dark_share: f32, light_share: f32) -> Pair {
        let over = |base: u8, k: f32| -> u32 {
            let ch = |shift: u32| {
                let a = ((accent >> shift) & 255) as f32;
                ((base as f32 + (a - base as f32) * k.clamp(0.0, 1.0)).round() as u32) << shift
            };
            ch(0) | ch(8) | ch(16) | 0xff00_0000
        };
        Pair { dark: over(30, dark_share), light: over(255, light_share) }
    }
    /// The colour at the current scheme position.
    pub fn now(self) -> u32 {
        // CONTROL_ACCENT (and so LINK and SYSTEM_BLUE) follow Settings > Appearance > Accent colour
        if self.dark == ACCENT_DARK && self.light == ACCENT_LIGHT {
            return accent_now();
        }
        // SELECTION is the accent a step deeper in Dark
        if self.dark == SELECTION_DARK && self.light == SELECTION_LIGHT && !matches!(accent(), Accent::Blue | Accent::Multicolour) {
            let (dark, light) = accent().colours();
            return mix(deepen(dark), light, scheme());
        }
        mix(self.dark, self.light, scheme())
    }
}

static TEXT_SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0
static ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000);
static HOLD: AtomicU32 = AtomicU32::new(0);
static BOLD: AtomicBool = AtomicBool::new(false);
static MOTION: AtomicBool = AtomicBool::new(false);
static CONTRAST: AtomicBool = AtomicBool::new(false);
static TRANSPARENCY: AtomicBool = AtomicBool::new(false);
static SHAPES: AtomicBool = AtomicBool::new(false);

/// Larger Text: the factor for every text size.
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE.load(Relaxed))
}
/// Bold Text: render one weight heavier.
pub fn bold_text() -> bool {
    BOLD.load(Relaxed)
}
/// Display Zoom: the factor for geometry (1.0 standard).
pub fn display_zoom() -> f32 {
    f32::from_bits(ZOOM.load(Relaxed))
}
/// Reduce Motion: animations collapse to short cross-fades.
pub fn reduce_motion() -> bool {
    MOTION.load(Relaxed)
}
/// Increase Contrast: stronger separators, text and tints.
pub fn increase_contrast() -> bool {
    CONTRAST.load(Relaxed)
}
/// Reduce Transparency: frosted and translucent surfaces turn opaque. Increase Contrast implies it, as on the
/// reference desktop (Accessibility > Display turns Reduce transparency on, and locks it, with Increase contrast).
pub fn reduce_transparency() -> bool {
    TRANSPARENCY.load(Relaxed) || CONTRAST.load(Relaxed)
}
/// Button Shapes: tappable text gets a visible shape.
pub fn button_shapes() -> bool {
    SHAPES.load(Relaxed)
}
/// Touch Accommodations: the hold duration in ms, 0 when off.
pub fn touch_hold_ms() -> u32 {
    HOLD.load(Relaxed)
}

/// Increase Contrast: the high-contrast stand-in for one of the shell's
/// low-contrast greys (secondary and tertiary text, separators, control
/// tracks, chart axes), or `color` itself. The one place those are chosen:
/// `Ui` text and `Surface` rounded-rect fills route through it, so it is the
/// identity (and costs one atomic load) while the setting is off.
pub fn contrast(color: u32) -> u32 {
    if !increase_contrast() {
        return color;
    }
    contrast_on(color)
}

/// `contrast` with the setting taken as on.
pub fn contrast_on(color: u32) -> u32 {
    use crate::ui::{SECONDARY, SEPARATOR, TRACK};
    use crate::gfx::{pack, rgba};
    const MAP: [(u32, u32); 5] = [
        (SECONDARY, pack(190, 190, 196, 255)),
        (SEPARATOR, pack(110, 110, 114, 255)),
        (TRACK, pack(104, 104, 108, 255)),
        (pack(99, 99, 102, 255), pack(150, 150, 155, 255)), // tertiary label
        (pack(72, 72, 74, 255), pack(120, 120, 124, 255)),  // chart axis
    ];
    if let Some(&(_, to)) = MAP.iter().find(|(from, _)| *from == color) {
        return to;
    }
    // translucent tertiary labels (placeholders, disabled buttons)
    if color == rgba(235, 235, 245, 77) {
        return rgba(235, 235, 245, 170);
    }
    if color == rgba(235, 235, 245, 150) {
        return rgba(235, 235, 245, 210);
    }
    color
}

/// The system accent colour (Appearance > Accent colour): the sidebar selection, switches and slider fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Graphite,
    /// Multicolour: controls use the system blue and apps may tint with their own colours.
    Multicolour,
}

impl Accent {
    /// In the order Settings lists them (Multicolour first).
    pub const ALL: [Accent; 9] = [Accent::Multicolour, Accent::Blue, Accent::Purple, Accent::Pink, Accent::Red, Accent::Orange, Accent::Yellow, Accent::Green, Accent::Graphite];

    pub fn key(self) -> &'static str {
        ["blue", "purple", "pink", "red", "orange", "yellow", "green", "graphite", "multicolour"][self as usize]
    }

    pub fn label(self) -> &'static str {
        ["Blue", "Purple", "Pink", "Red", "Orange", "Yellow", "Green", "Graphite", "Multicolour"][self as usize]
    }

    pub fn parse(s: &str) -> Accent {
        Accent::ALL.into_iter().find(|a| a.key() == s).unwrap_or(Accent::Blue)
    }

    /// (dark, light) colour of the accent.
    pub fn colours(self) -> (u32, u32) {
        use crate::gfx::pack;
        match self {
            Accent::Blue | Accent::Multicolour => (pack(10, 132, 255, 255), pack(0, 122, 255, 255)),
            Accent::Purple => (pack(191, 90, 242, 255), pack(160, 72, 190, 255)),
            Accent::Pink => (pack(255, 55, 110, 255), pack(238, 52, 118, 255)),
            Accent::Red => (pack(255, 69, 58, 255), pack(228, 40, 36, 255)),
            Accent::Orange => (pack(255, 159, 10, 255), pack(247, 130, 0, 255)),
            Accent::Yellow => (pack(255, 214, 10, 255), pack(252, 184, 0, 255)),
            Accent::Green => (pack(50, 215, 75, 255), pack(40, 190, 70, 255)),
            Accent::Graphite => (pack(152, 152, 157, 255), pack(120, 120, 126, 255)),
        }
    }
}

/// The values of `ui::CONTROL_ACCENT` and `ui::SELECTION`, which `Pair::now` swaps for the chosen accent.
const ACCENT_DARK: u32 = crate::gfx::pack(10, 132, 255, 255);
const ACCENT_LIGHT: u32 = crate::gfx::pack(0, 122, 255, 255);
const SELECTION_DARK: u32 = crate::gfx::pack(14, 100, 196, 255);
const SELECTION_LIGHT: u32 = crate::gfx::pack(0, 112, 245, 255);

/// An accent at 77% brightness: the Dark selection plate.
fn deepen(c: u32) -> u32 {
    let ch = |s: u32| ((((c >> s) & 0xff) as f32) * 0.77) as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16 | 0xff00_0000
}

static ACCENT: AtomicU32 = AtomicU32::new(0);

pub fn accent() -> Accent {
    let n = ACCENT.load(Relaxed);
    Accent::ALL.into_iter().find(|a| *a as u32 == n).unwrap_or(Accent::Blue)
}

pub fn set_accent(a: Accent) {
    ACCENT.store(a as u32, Relaxed);
}

/// The accent colour in the current scheme (mid-fade it is between the two).
pub fn accent_now() -> u32 {
    let (dark, light) = accent().colours();
    mix(dark, light, scheme())
}

/// Button Shapes: the corner radius (px) of a tappable text button's
/// backing plate (6 pt at most, never a pill), or `None` when the setting is off.
pub fn button_shape_radius(s: f32) -> Option<f32> {
    button_shapes().then_some(5.0 * s)
}

/// An animation's length: `base` normally, `REDUCED_MS` under Reduce Motion.
pub fn anim_ms(base: f64) -> f64 {
    if reduce_motion() {
        REDUCED_MS.min(base)
    } else {
        base
    }
}

/// A long-press delay: `base`, or the Touch Accommodations hold if longer.
pub fn long_press_ms(base: f64) -> f64 {
    base.max(touch_hold_ms() as f64)
}

#[cfg(test)]
mod scheme_tests {
    use super::*;

    #[test]
    fn accent_keys_round_trip_and_default_to_blue() {
        for a in Accent::ALL {
            assert_eq!(Accent::parse(a.key()), a);
        }
        assert_eq!(Accent::parse("nonsense"), Accent::Blue);
    }

    #[test]
    fn the_control_accent_pair_follows_the_chosen_accent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = accent();
        set_accent(Accent::Green);
        jump_to(false);
        assert_eq!(crate::ui::system::CONTROL_ACCENT.now(), Accent::Green.colours().0);
        set_accent(Accent::Multicolour);
        assert_eq!(crate::ui::system::LINK.now(), Accent::Blue.colours().0);
        set_accent(keep);
    }

    use crate::gfx::pack;
    use std::sync::Mutex;

    // the scheme is process-wide: keep these tests from interleaving
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn fade_runs_dark_to_light_and_back_monotonically() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        jump_to(false);
        set_light(true, 1000.0);
        let mut last = -1.0;
        for ms in [1000.0, 1070.0, 1140.0, 1210.0, 1280.0] {
            tick(ms);
            assert!(scheme() >= last, "monotonic");
            last = scheme();
        }
        assert_eq!(scheme(), 1.0);
        assert!(!tick(1500.0), "done");
        // mid fade is strictly between
        set_light(false, 2000.0);
        assert!(tick(2140.0));
        assert!(scheme() > 0.0 && scheme() < 1.0);
        jump_to(false);
    }

    #[test]
    fn reversing_mid_fade_starts_from_where_it_is() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        jump_to(false);
        set_light(true, 0.0);
        tick(140.0);
        let mid = scheme();
        set_light(false, 140.0);
        tick(140.0);
        assert_eq!(scheme(), mid, "no jump when reversing");
        tick(10_000.0);
        assert_eq!(scheme(), 0.0);
    }

    #[test]
    fn mix_and_pair_hit_both_ends() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (d, l) = (pack(20, 20, 22, 255), pack(235, 235, 235, 255));
        assert_eq!(mix(d, l, 0.0), d);
        assert_eq!(mix(d, l, 1.0), l);
        assert_eq!(mix(d, l, 0.5), pack(128, 128, 129, 255));
        jump_to(true);
        assert_eq!(Pair::new(d, l).now(), l);
        jump_to(false);
        assert_eq!(Pair::new(d, l).now(), d);
    }

    #[test]
    fn auto_follows_the_hour_and_modes_round_trip() {
        assert!(!auto_is_light(6) && auto_is_light(7) && auto_is_light(18) && !auto_is_light(19) && !auto_is_light(0));
        for m in [Mode::Light, Mode::Dark, Mode::Auto] {
            assert_eq!(Mode::parse(m.key()), m);
        }
        assert_eq!(Mode::parse("junk"), Mode::Dark);
        assert!(Mode::Light.is_light(3) && !Mode::Dark.is_light(12) && Mode::Auto.is_light(12));
    }
}

#[cfg(test)]
mod tinted_tests {
    use super::Pair;

    #[test]
    fn a_tinted_fill_is_pale_in_light_and_deep_in_dark_and_opaque() {
        let red = 0xff00_00ffu32; // r = 255
        let p = Pair::tinted(red);
        assert_eq!(p.light >> 24, 255);
        assert_eq!(p.dark >> 24, 255);
        assert_eq!(p.light & 255, 255, "red stays full over white");
        assert_eq!((p.light >> 8) & 255, (255.0f32 * 0.82).round() as u32, "green is pulled toward the accent");
        assert_eq!(p.dark & 255, (30.0 + 225.0f32 * 0.32).round() as u32);
        assert_eq!(Pair::tinted_by(red, 0.0, 0.0), Pair { dark: 0xff1e_1e1e, light: 0xffff_ffff });
    }
}

#[cfg(test)]
mod display_option_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn text_round_trip_and_implication() {
        let o = DisplayOptions { reduce_transparency: false, increase_contrast: true, differentiate_without_color: true };
        let back = DisplayOptions::parse(&o.to_text());
        assert!(back.increase_contrast && back.differentiate_without_color && back.reduce_transparency);
        assert_eq!(DisplayOptions::parse(""), DisplayOptions::default());
    }

    #[test]
    fn file_round_trip() {
        let dir = std::env::temp_dir().join(format!("bifrost-a11y-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accessibility");
        let path = path.to_str().unwrap();
        let o = DisplayOptions { reduce_transparency: true, ..Default::default() };
        o.write_to(path).unwrap();
        assert_eq!(DisplayOptions::read_from(path), o);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(DisplayOptions::read_from(path), DisplayOptions::default());
    }

    #[test]
    fn observers_are_called_and_removable() {
        let obs = Observers::new();
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let t = obs.add(move |o| {
            assert!(o.increase_contrast);
            n2.fetch_add(1, Ordering::SeqCst);
        });
        obs.notify(DisplayOptions { increase_contrast: true, ..Default::default() });
        obs.remove(t);
        obs.notify(DisplayOptions { increase_contrast: true, ..Default::default() });
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
}
