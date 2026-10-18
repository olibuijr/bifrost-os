//! Renders every Settings page to PNG on the host, with the status bar and
//! home indicator the shell draws around it: `cargo run --example
//! settings_preview [OUT_DIR]` (default target/settings). The pages are
//! reached the way a finger would, by tapping rows.
use springboard::font::Font;
use springboard::gfx::{Image, Surface};
use springboard::shell::battery::Sample;
use springboard::shell::keyboard::Out;
use springboard::shell::prefs::Prefs;
use springboard::shell::settings::{AlertHit, Settings};
use springboard::shell::ui::{home_indicator, Ui};
use springboard::shell::wallpaper_page::Wallpaper;
use springboard::shell::{Event, Info, Network};
use springboard::symbols::Symbols;
use std::fs;
use std::rc::Rc;

const W: usize = 1200;
const H: usize = 1920;

/// The packed stills (tools/pack.py), if any.
fn wallpapers() -> Vec<Wallpaper> {
    let index = Prefs::load("assets/wallpapers/index.ini");
    index
        .entries()
        .filter_map(|(id, name)| {
            let thumb = Image::from_raw(&fs::read(format!("assets/wallpapers/{id}.thumb.raw")).ok()?)?;
            Some(Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
        })
        .collect()
}

struct Preview {
    ui: Ui,
    prefs: Prefs,
    out: String,
    chrome: springboard::scene::Chrome,
}

impl Preview {
    fn save(&self, st: &mut Settings, name: &str, t: f64) {
        let mut buf = vec![0u32; W * H];
        let mut s = Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 };
        st.draw(&mut s, &self.ui, t);
        let mid = (self.ui.pt(springboard::scene::STATUS_PT) / 2.0) as i32;
        if let Some(tm) = &self.chrome.time {
            s.blit(tm, self.ui.pt(springboard::scene::STATUS_LEFT_PT) as i32, mid - tm.h as i32 / 2, 255, 255);
        }
        springboard::scene::draw_status_right(&mut s, W as i32, mid, self.ui.s, &self.chrome);
        home_indicator(&mut s, &self.ui, W as i32, H as i32, 255);
        let path = format!("{}/{name}.png", self.out);
        let file = fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let rgb: Vec<u8> = buf.iter().flat_map(|p| [*p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
        w.write_image_data(&rgb).unwrap();
        println!("{path}");
    }

    fn tap(&mut self, st: &mut Settings, x: f32, y: f32) {
        st.touch(&self.ui, &mut self.prefs, Event::Down(x, y), 0.0);
        st.touch(&self.ui, &mut self.prefs, Event::Up(x, y), 10.0);
        st.tick(&self.ui, 1e9);
    }

    /// Taps the row labelled `label` on the top page.
    fn row(&mut self, st: &mut Settings, label: &str) {
        let (x, y) = st.row_centre(&self.ui, label).unwrap_or_else(|| panic!("no row {label}"));
        self.tap(st, x, y);
    }

    fn back(&mut self, st: &mut Settings) {
        let (x, y) = (self.ui.pt(40.0), self.ui.pt(42.0));
        self.tap(st, x, y);
    }

    /// Drags the list up by `dy` px and lets it settle.
    fn scroll(&mut self, st: &mut Settings, dy: f32) {
        st.touch(&self.ui, &mut self.prefs, Event::Down(600.0, 1500.0), 0.0);
        for k in 1..=10 {
            st.touch(&self.ui, &mut self.prefs, Event::Move(600.0, 1500.0 - dy * k as f32 / 10.0), k as f64 * 30.0);
        }
        st.touch(&self.ui, &mut self.prefs, Event::Up(600.0, 1500.0 - dy), 600.0);
        st.tick(&self.ui, 1e9);
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/settings".into());
    fs::create_dir_all(&out).unwrap();
    let data: &'static [u8] = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), W as f32 / 768.0);
    ui.symbols = Symbols::load("assets");
    let net = |ssid: &str, bars, secure, saved| Network { ssid: ssid.into(), bars, secure, saved };
    // a day of battery: drains overnight, charges after lunch
    let now = 1_759_536_000i64; // 2025-10-04 00:00 UTC
    let history: Vec<Sample> = (0..288)
        .map(|i| {
            let unix = now - 24 * 3600 + i * 300;
            let h = i as f32 / 12.0;
            let (percent, charging) = if h < 14.0 { ((92.0 - h * 4.2).max(28.0) as u8, false) } else if h < 17.0 { ((28.0 + (h - 14.0) * 22.0) as u8, true) } else { ((94.0 - (h - 17.0) * 4.0) as u8, false) };
            Sample { unix, percent, charging }
        })
        .collect();
    let info = Info {
        captive_portal: None,
        display_sizes: Vec::new(),
        battery_present: true,
        desktop_network: Default::default(),
        display_mode: None,
        display_rates: Vec::new(),
        brightness_supported: true,
        ambient_light_supported: false,
        motion_sensor_supported: false,
        power_profile: "Auto".into(),
        wifi_notice: None,
        battery_health: Vec::new(),
        media_counts: [0; 3],
        bt: Default::default(),
        hour: 14,
        min: 41,
        wday: 6,
        mday: 4,
        mon: 9,
        battery: 63,
        charging: true,
        online: true,
        wired: false,
        wifi_adapter: true,
        wifi_bars: 3,
        wifi: "WIFI-5G".into(),
        wifi_on: true,
        current_ssid: Some("WIFI-5G".into()),
        networks: vec![net("WIFI-5G", 3, true, true), net("WIFI", 3, true, true), net("Siminn-5G", 2, true, false), net("Café Open", 2, false, false), net("Vodafone-2.4", 1, true, false)],
        ip: "192.168.1.40".into(),
        router: "192.168.1.1".into(),
        brightness: 0.6,
        model: "Midget".into(),
        kernel: "6.12.8".into(),
        memory: "3.6 GB".into(),
        wallpaper: "wwdc_2021_dark".into(),
        name: "Midget".into(),
        mac: "AC:E4:B5:35:20:F0".into(),
        cpu: "x86_64, 8 cores".into(),
        storage_total: 32_000_000_000,
        storage_free: 17_870_000_000,
        uptime_min: 1_507,
        serial: "R9XR40ABCDE".into(),
        timezone: "Atlantic/Reykjavik".into(),
        saved: Vec::new(),
        enterprise: Vec::new(),
        time_manual: false,
        time_sync: String::new(),
        battery_history: history.into(),
    };
    let chrome = springboard::scene::Chrome {
        time: Some(ui.chrome("14:41", 12.0, springboard::shell::ui::Weight::Semibold, springboard::shell::ui::WHITE)),
        date: None,
        percent: Some(ui.chrome("63%", 12.0, springboard::shell::ui::Weight::Medium, springboard::shell::ui::WHITE)),
        battery: 63,
        charging: true,
        online: true,
        wifi_bars: 3,
        ..Default::default()
    };
    let mut p = Preview { ui, prefs: Prefs::memory(), out, chrome };
    let wallpapers = Rc::new(wallpapers());
    let mut st = Settings::new(&p.ui, &info, &p.prefs, W as i32, H as i32, wallpapers.clone());
    p.save(&mut st, "split-general", 0.0);
    let (x, y) = st.sidebar_centre(&p.ui, "Display & Brightness").unwrap();
    st.touch(&p.ui, &mut p.prefs, Event::Down(x, y), 0.0);
    st.touch(&p.ui, &mut p.prefs, Event::Up(x, y), 10.0);
    p.save(&mut st, "split-display", 20.0);
    st.allow_split(&p.ui, false);
    p.save(&mut st, "root", 0.0);
    // the whole root fits a 1920 px screen; a shorter one shows the large title collapse
    let mut short = Settings::new(&p.ui, &info, &p.prefs, W as i32, 1100, wallpapers.clone());
    short.allow_split(&p.ui, false);
    p.scroll(&mut short, 300.0);
    p.save(&mut short, "root-collapsed", 0.0);

    // the push slide, mid-way
    let (x, y) = st.row_centre(&p.ui, "General").unwrap();
    st.touch(&p.ui, &mut p.prefs, Event::Down(x, y), 0.0);
    st.touch(&p.ui, &mut p.prefs, Event::Up(x, y), 10.0);
    p.save(&mut st, "push-mid", 10.0 + springboard::shell::settings::PUSH_MS * 0.4);
    st.tick(&p.ui, 1e9);
    p.save(&mut st, "general", 0.0);
    for (label, name) in [("About", "about"), ("Software Update", "software-update"), ("Date & Time", "date-time"), ("Keyboard", "keyboard"), ("Language & Region", "language"), ("Transfer or Reset Tab", "reset")] {
        p.row(&mut st, label);
        p.save(&mut st, name, 0.0);
        if name == "reset" {
            p.row(&mut st, "Reset All Settings");
            p.save(&mut st, "reset-alert", 0.0);
            let (cx, cy) = st.alert().unwrap().button(&p.ui, W as i32, H as i32, AlertHit::Cancel);
            p.tap(&mut st, cx, cy);
        }
        p.back(&mut st);
    }
    p.back(&mut st);

    for (label, name) in [("Sounds", "sounds"), ("Control Centre", "control-centre"), ("Home Screen & Dock", "home-screen"), ("Accessibility", "accessibility"), ("Wallpaper", "wallpaper"), ("Battery", "battery"), ("Privacy", "privacy")] {
        p.row(&mut st, label);
        p.save(&mut st, name, 0.0);
        p.back(&mut st);
    }

    p.row(&mut st, "Display & Brightness");
    p.save(&mut st, "display", 0.0);
    p.row(&mut st, "Auto-Lock");
    p.save(&mut st, "auto-lock", 0.0);
    p.back(&mut st);
    p.back(&mut st);

    p.row(&mut st, "Wi-Fi");
    p.save(&mut st, "wifi", 0.0);
    p.row(&mut st, "WIFI-5G");
    p.save(&mut st, "wifi-detail", 0.0);
    p.row(&mut st, "Forget This Network");
    p.save(&mut st, "wifi-forget", 0.0);
    let (cx, cy) = st.alert().unwrap().button(&p.ui, W as i32, H as i32, AlertHit::Cancel);
    p.tap(&mut st, cx, cy);
    p.back(&mut st);
    p.row(&mut st, "Siminn-5G");
    st.key(&p.ui, Out::Text("secret".into()), 0.0);
    p.save(&mut st, "wifi-sheet", 1e9);
}
