//! The bare-metal shell: the BifrOSt desktop, drawing straight to KMS (or a Wayland surface)
//! and reading pointer, touch and keys from evdev.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    bifrost_core::host::tune_allocator();
    if let Err(e) = run() {
        eprintln!("springboard: {e}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
extern "C" {
    fn geteuid() -> u32;
}

#[cfg(target_os = "linux")]
fn run() -> std::io::Result<()> {
    springboard::gfx::affinity::ui_thread();
    use springboard::bare::gpu_comp::{GpuComp, Step as GpuStep};
    use springboard::bare::{input, kms, kms::Display, power, sleep, sys};
    use springboard::font::Font;
    use springboard::gfx::rotate::{rotate_areas, rotate_damage, Rotation};
    use springboard::gfx::{threads, Image, Surface, View};
    use springboard::media::mixer::Mixer;
    use springboard::media::route;
    use springboard::media::volume::{self, Volume};
    use springboard::shell::battery;
    use springboard::shell::low_battery;
    use springboard::shell::power_menu::{PowerAction, PowerKey};
    use springboard::shell::{Effect, Event, Info, Network, SavedNetwork, Shell, WifiAction};
    use std::fs;

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--version") {
        // build stamp for `bifrost pc` diagnostics
        let b = springboard::shell::pages::BUILD;
        println!("{} {} {}", b.version, b.git, b.date);
        return Ok(());
    }
    if args.get(1).map(String::as_str)==Some("--install-store-app") {
        if unsafe{geteuid()}!=0{return Err(std::io::Error::other("App installation requires root"));}
        use springboard::shell::store::{api,net,install};let id=args.get(2).ok_or_else(||std::io::Error::other("Expected App Store ID"))?;
        let bytes=net::get(&net::absolute("/api/v1/pc/apps.json"),4*1024*1024)?;
        let catalogue=api::parse_catalog(std::str::from_utf8(&bytes).map_err(std::io::Error::other)?).map_err(std::io::Error::other)?;
        let app=catalogue.iter().find(|a|a.id==*id&&springboard::shell::kinds::Kind::parse(&a.kind).is_some_and(|k|k.on_home_grid())).ok_or_else(||std::io::Error::other("App not found in PC App Store"))?;
        let root=std::path::PathBuf::from(std::env::var("SPRINGBOARD_DATA").unwrap_or_else(|_|"/var/lib/springboard".into()));
        let done=std::sync::atomic::AtomicU64::new(0);
        let installed=install::install_download_local(&root,&net::absolute(&app.package),app.size,&app.sha256,install::shell_version(),&done,&std::sync::atomic::AtomicBool::new(false)).map_err(std::io::Error::other)?;
        if installed.id!=*id {return Err(std::io::Error::other("Package identity mismatch"));}
        println!("Installed {} {} through App Store",installed.id,installed.version);return Ok(());
    }
    if args.get(1).map(String::as_str)==Some("--install-package") {
        // lab: install a local .aap (tools/pc-app.py build output) without publishing it; the running shell picks it
        // up on its next restart, like --install-store-app
        if unsafe{geteuid()}!=0{return Err(std::io::Error::other("App installation requires root"));}
        use springboard::shell::{store::install,sha256::{hex,Sha256}};
        let bytes=std::fs::read(args.get(2).ok_or_else(||std::io::Error::other("Expected a .aap path"))?)?;
        let mut h=Sha256::new();h.update(&bytes);
        let root=std::path::PathBuf::from(std::env::var("SPRINGBOARD_DATA").unwrap_or_else(|_|"/var/lib/springboard".into()));
        let installed=install::install_package(&root,&bytes,&hex(&h.finish()),install::shell_version()).map_err(std::io::Error::other)?;
        println!("Installed {} {} from a local package",installed.id,installed.version);return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--wpa") {
        // one-shot control command, like wpa_cli: springboard --wpa STATUS
        let iface = springboard::net::wifi::wireless_iface(std::path::Path::new("/sys/class/net")).unwrap_or_else(|| springboard::net::wifi::IFACE.to_string());
        let ctrl_dir = std::env::var("SPRINGBOARD_WPA_CTRL").unwrap_or_else(|_| "/run/wpa_supplicant".into());
        let c = springboard::net::wpa::Ctrl::connect(&ctrl_dir, &iface)?;
        print!("{}", c.request(&args[2..].join(" "))?);
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--wifi-test") {
        return wifi_test(args.get(2).ok_or_else(|| std::io::Error::other("usage: --wifi-test DIR"))?);
    }
    if matches!(args.get(1).map(String::as_str), Some("--backup" | "--restore")) {
        // offline data tools used by `bifrost pc` backup / restore: springboard --backup DIR OUT | --restore DIR ARCHIVE
        use springboard::shell::backup;
        let (dir, file) = (std::path::Path::new(args.get(2).ok_or_else(|| std::io::Error::other("missing DIR"))?), std::path::Path::new(args.get(3).ok_or_else(|| std::io::Error::other("missing FILE"))?));
        let n = if args[1] == "--backup" { backup::create(dir, file)? } else { backup::stage(dir, file)? };
        println!("{n} files");
        return Ok(());
    }
    // `--boot DIR`: the data directory comes first, as for a plain start.
    let boot = args.get(1).is_some_and(|a| a == "--boot");
    let dir = args.get(1 + boot as usize).cloned().unwrap_or_else(|| {
        // SAFETY: geteuid has no preconditions.
        let root = unsafe { geteuid() } == 0;
        springboard::bare::platform::data_dir(root, std::env::var("XDG_DATA_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
    });
    springboard::bare::crashlog::install(&dir);
    // a restore staged by `bifrost pc` restore or Settings › Backup applies before anything loads the data
    match springboard::shell::backup::apply_pending(std::path::Path::new(&dir)) {
        Some(Ok(n)) => eprintln!("springboard: restored {n} files from backup"),
        Some(Err(e)) => eprintln!("springboard: restore failed: {e}"),
        None => {}
    }
    // the crash-loop rule applies on every start
    if !springboard::bare::takeover::begin(&dir) {
        return Ok(());
    }
    if let Some(note) = springboard::bare::crashlog::previous_run_note(&dir) {
        springboard::bare::takeover::log(&dir, &note);
    }
    // recovery paths that must work even when the UI loop hangs
    springboard::bare::heartbeat::init(&dir);
    springboard::bare::heartbeat::watch(&dir);
    springboard::bare::chord_recovery::spawn(&dir);
    springboard::shell::apphost::register(std::path::Path::new(&dir));
    // Settings > Fonts overrides apply from the next start
    let font_data = bifrost_render::fontprefs::map_font(&dir, bifrost_render::fontprefs::FontRole::System)?;
    let mono = bifrost_render::fontprefs::map_font(&dir, bifrost_render::fontprefs::FontRole::Mono)?;
    let font = Font::parse(font_data).ok_or_else(|| std::io::Error::other("bad font"))?;
    // the shell owns settings.ini from here on; the bin only reads start-up values
    let mut prefs = springboard::shell::prefs::Prefs::load(format!("{dir}/settings.ini"));
    // the mixer carries the volume's gain only; amplifiers and routes stay as they are
    let mixer = Mixer::open(volume::CONTROL_DEVICE).map_err(|e| eprintln!("springboard: mixer: {e}")).ok();
    // the headset jack: its input device reports plug state as switch events
    let mut headset = input::Device::by_hint(route::JACK_DEVICE_HINTS).and_then(|p| input::Device::open(&p).map_err(|e| eprintln!("springboard: headset {p}: {e}")).ok());
    let mut jack = headset.as_ref().map_or_else(route::Jack::default, |d| route::Jack::from_bits(d.switches()));
    let mut router = route::Router::from_prefs(|k| prefs.get(k).map(String::from), jack.plugged());
    let mut vol = router.volume();
    apply_volume(vol, mixer.as_ref());
    let mut jack_events = Vec::new();
    // a fresh install (no wallpaper chosen yet) starts on a random Icelandic still, kept from then on
    if prefs.get("wallpaper").is_none() {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        if let Some(id) = springboard::shell::wallpaper_page::first_wallpaper(&dir, seed) {
            let _ = prefs.set("wallpaper", id);
        }
    }
    let mut current_wallpaper = prefs.get("wallpaper").unwrap_or("").to_string();
    let mut mapped = match map_wallpaper(&dir, &current_wallpaper) {
        Some(m) => m,
        None => {
            current_wallpaper.clear();
            map_file(&format!("{dir}/wallpaper.raw"))?
        }
    };
    let wallpaper = View::from_raw(mapped).ok_or_else(|| std::io::Error::other("bad wallpaper"))?;
    let icon = |name: &str, size: usize| fs::read(format!("{dir}/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));

    // SPRINGBOARD_VIRTUAL=1200x1920: an emulated panel with no KMS, evdev or power files (headless
    // emulator on a dev host); input and screenshots go through the control socket
    let emulated = kms::virtual_size(std::env::var("SPRINGBOARD_VIRTUAL").ok().as_deref());
    let wayland = std::env::var_os("SPRINGBOARD_WAYLAND").is_some();
    let mut disp = match emulated {
        #[cfg(feature = "desktop-wayland")]
        None if wayland => Display::wayland()?,
        Some((w, h, hz)) => Display::virtual_panel(w, h, hz),
        None => Display::open_best(8000)?,
    };
    let (w, h) = disp.size();
    // the pt -> px scale follows the panel (EDID size, else resolution) unless the user
    // chose one in Settings > Display (or SPRINGBOARD_SCALE=1.5 for an emulator)
    springboard::layout::set_adaptive(springboard::layout::auto_scale(w as i32, h as i32, disp.size_mm()));
    let chosen = std::env::var("SPRINGBOARD_SCALE").ok().or_else(|| prefs.get(springboard::shell::settings::DISPLAY_SCALE_PREF).map(String::from));
    springboard::layout::set_user_scale(chosen.and_then(|v| v.trim().parse::<f32>().ok()).filter(|s| s.is_finite() && *s > 0.0));
    let (mut touch, mut keys) = (input::Device::virtual_device(), input::Device::virtual_device());
    // every touchscreen, pointer, keyboard and power button by capability, hot-plugged;
    // grabbed only as the root OS shell (a session under a compositor shares them)
    let mut inputs = match emulated {
        None if !wayland => input::InputSet::desktop(w as f32, h as f32, unsafe { geteuid() } == 0),
        _ => input::InputSet::none(),
    };

    use springboard::net::wifi::Cmd;
    let wifi = springboard::net::wifi::Wifi::start(&dir);
    let bluetooth = springboard::bare::bluetooth::Bluetooth::start(&dir);
    // time zone and network time: the saved choices, then the sync task
    {
        let saved = springboard::shell::prefs::Prefs::load(format!("{dir}/settings.ini"));
        // the machine's own zone (/etc/localtime) wins over the saved one
        let system_tz = springboard::bare::timezone::system_name();
        if !system_tz.as_deref().or(saved.get("tz")).is_some_and(springboard::bare::timezone::set) {
            springboard::bare::timezone::set(springboard::bare::timezone::DEFAULT);
        }
        springboard::net::ntp::set_auto(saved.get("time_auto") != Some("0"));
        springboard::net::ntp::spawn(wifi.status.clone());
    }
    power::configure(emulated.is_some());
    springboard::shell::night_shift::NightShift::load(&prefs).publish();
    // Night Shift multipliers in force on the frame (None: untouched), and the tinted copy that is presented
    let mut night_mult: Option<[u32; 3]> = None;
    let mut tinted: Vec<u32> = Vec::new();
    let max = power::max_brightness() as f32;
    // the Control Center / Settings slider is remembered across restarts; 80 % until it is first moved
    let mut level = prefs.get("brightness").and_then(|v| v.parse::<f32>().ok()).map_or(DEFAULT_BRIGHTNESS, |v| v.clamp(0.05, 1.0));
    power::set_brightness((level * max).round().max(1.0) as i64);
    // when the level last changed and has not been saved yet (a drag moves it every frame)
    let mut brightness_unsaved: Option<f64> = None;
    let battery_log = std::path::PathBuf::from(format!("{dir}/battery.log"));
    let mut history = std::sync::Arc::new(battery::load(&battery_log));
    let mut last_sample = f64::NEG_INFINITY;
    let fixed = About::read(&dir);
    // facts that never change while the shell runs are read once; free space and battery health
    // only every 30 s (the shell redraws Storage/About from `Info`, which still carries them)
    let (model_text, kernel_text, memory_text) = (model(), fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().split('-').next().unwrap_or("").to_string(), memory());
    let mac_iface = springboard::net::wifi::wireless_iface(std::path::Path::new("/sys/class/net")).unwrap_or_else(|| springboard::net::wifi::IFACE.to_string());
    let mac_text = fs::read_to_string(format!("/sys/class/net/{mac_iface}/address")).map(|m| m.trim().to_uppercase()).unwrap_or_default();
    let slow: std::cell::RefCell<(f64, (u64, u64), Vec<(String, String)>)> = std::cell::RefCell::new((f64::NEG_INFINITY, (0, 0), Vec::new()));
    let panel_mode = (w as u32, h as u32, disp.mode.vrefresh);
    let brightness_supported = power::find_backlight(std::path::Path::new("/sys/class/backlight")).is_some();
    let battery_present=power::find_battery(std::path::Path::new("/sys/class/power_supply")).is_some();
    // timedatectl spawns (NTP state, timezone) and the uptime read: at most every 15 s, not on every info tick
    let clock_info: std::cell::RefCell<(f64, String, i64)> = std::cell::RefCell::new((f64::NEG_INFINITY, String::new(), 0));
    let gather = |level: f32, wallpaper: &str, history: &std::sync::Arc<Vec<battery::Sample>>| {
        use springboard::net::wifi::State;
        let (hour, min, _, wday, mday, mon) = sys::local_time();
        let (battery, charging) = power::battery();
        let ws = wifi.snapshot();
        let (storage_total, storage_free, battery_health) = {
            let mut slow = slow.borrow_mut();
            if sys::now_ms() - slow.0 >= 30_000.0 {
                *slow = (sys::now_ms(), sys::disk_space("/").unwrap_or((0, 0)), battery::health());
            }
            (slow.1 .0, slow.1 .1, slow.2.clone())
        };
        let (time_sync, uptime_min) = {
            let mut c = clock_info.borrow_mut();
            if sys::now_ms() - c.0 >= 15_000.0 {
                *c = (sys::now_ms(), springboard::net::ntp::status_text(), uptime_minutes());
            }
            (c.1.clone(), c.2)
        };
        let wifi_text = match ws.state {
            State::Connected => ws.ssid.clone().unwrap_or_else(|| "Connected".into()),
            // associated but no lease: say why instead of connecting forever
            State::Connecting => ws.ssid.as_ref().and_then(|s| ws.notes.get(s)).cloned().unwrap_or_else(|| "Connecting…".into()),
            State::Searching | State::Starting => "Not Connected".into(),
            // desktop without Wi-Fi: the wired link is the connection
            State::Off if ws.ethernet.is_some() => "Ethernet".into(),
            State::Off => "Off".into(),
        };
        let wired_ip = ws.ethernet.as_ref().and_then(|e| e.ip);
        let networks = ws.scan.iter().map(|r| Network { ssid: r.ssid.clone(), bars: r.bars(), secure: r.secure, saved: ws.saved.contains(&r.ssid) }).collect();
        Info {
            display_mode: Some(panel_mode),
            brightness_supported,
            battery_present,
            desktop_network: None,
            wifi: wifi_text,
            wifi_on: ws.state != State::Off,
            networks,
            saved: ws.known.iter().map(|k| SavedNetwork { ssid: k.ssid.clone(), auto_join: k.auto_join, note: ws.notes.get(&k.ssid).cloned().unwrap_or_default(), security: None }).collect(),
            enterprise: ws.scan.iter().filter(|r| r.enterprise).map(|r| r.ssid.clone()).collect(),
            current_ssid: ws.ssid.clone().filter(|_| ws.state == State::Connected),
            wifi_notice: None,
            ip: ws.ip.or(wired_ip).map(|i| i.to_string()).unwrap_or_else(|| "—".into()),
            hour,
            min,
            wday,
            mday,
            mon,
            battery,
            charging,
            online: ws.state == State::Connected,
            wired: wired_ip.is_some(),
            wifi_adapter: springboard::net::wifi::wireless_iface(std::path::Path::new("/sys/class/net")).is_some(),
            wifi_bars: ws.rssi.map(springboard::status::bars_for).unwrap_or(0),
            brightness: level,
            model: model_text.clone(),
            kernel: kernel_text.clone(),
            memory: memory_text.clone(),
            wallpaper: wallpaper.to_string(),
            name: fixed.name.clone(),
            mac: mac_text.clone(),
            router: ws.router.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            cpu: fixed.cpu.clone(),
            storage_total,
            storage_free,
            uptime_min,
            serial: fixed.serial.clone(),
            media_counts: fixed.media,
            timezone: springboard::bare::timezone::current_name(),
            time_manual: !springboard::net::ntp::auto(),
            time_sync,
            battery_history: history.clone(),
            battery_health,
            power_profile: String::new(),
            bt: bluetooth.snapshot(),
        }
    };
    springboard::bare::timezone::persist_to_system(true);
    springboard::shell::keyboard::set_predictive_default(true);
    // the PC has a hardware keyboard: no on-screen keyboard in the shell or in apps
    springboard::shell::keyboard::set_disabled(true);
    let mut shell = Shell::new(w as i32, h as i32, font, mono, wallpaper, icon, gather(level, &current_wallpaper, &history));
    shell.set_volume(vol.level());
    shell.set_route(router.route(), vol.level(), false, 0.0);
    shell.set_wallpapers(wallpapers(&dir));
    shell.set_wallpaper_dim(prefs.get("wallpaper_dim") != Some("0"));
    let mut lock_mapped: Option<&'static [u8]> = None;
    set_lock_wallpaper(&mut shell, &dir, prefs.get("wallpaper_lock").unwrap_or(""), &mut lock_mapped);
    shell.set_rotation_lock(prefs.get("rotation_lock") == Some("1"), sys::now_ms());
    shell.set_haptics_available(springboard::bare::vibrator::vibrator_available());
    let (unix, offset) = sys::wall_time();
    shell.set_wall(unix, offset, sys::now_ms());
    for fx in shell.set_data_dir(&dir, sys::now_ms()) {
        if let Effect::WakeAt(at) = fx {
            rtc_wake(at);
        }
    }
    // the logical frame the shell draws (turned on its side in landscape)
    // and the panel-sized buffer it is turned into before presenting
    let mut frame = vec![0u32; w * h];
    let (mut fw, mut fh) = (w, h);
    let mut panel: Vec<u32> = Vec::new();
    let mut events = Vec::new();
    let mut last_info = 0.0;
    let mut ready: Vec<bool> = Vec::new();
    let mut perf = Perf::default();
    let mut jank = springboard::bare::jank::Jank::default();
    // `touch $dir/trace` (or `trace on` over the control socket) logs input
    let mut trace = std::path::Path::new(&format!("{dir}/trace")).exists();
    let control = springboard::bare::control::Server::bind().map_err(|e| eprintln!("springboard: control socket: {e}")).ok();
    // synthetic touches from the control socket: (due ms, phase, x, y)
    let mut synthetic: std::collections::VecDeque<(f64, u8, f32, f32)> = Default::default();
    let period = disp.period_ms();
    let mut last_flip: Option<f64> = None;
    let mut sleeper = sleep::Sleeper::new(sleep::Host::new(emulated.is_some()), sleep::system_sleep_after(prefs.get(sleep::SYSTEM_SLEEP_PREF)));
    // the lid switch is logind's unless `lid_close=ignore`; the mains plug is polled
    let _lid = emulated.is_none().then(|| sleep::LidInhibitor::apply(prefs.get("lid_close")));
    let mut ac_watch = sleep::AcWatch::default();
    // Power + Volume Up within 150 ms is a screenshot; a lone Power press waits that long
    let mut chord = springboard::shell::chord::Chord::default();
    // holding Power for a second (screen on) opens the slide-to-power-off sheet
    let mut power_key = PowerKey::default();
    let mut hw_keys = springboard::shell::hwkeys::HwKeys::default();
    hw_keys.set_layout(springboard::installer::keymaps::configured_layout());
    let mut key_repeat = springboard::shell::hwkeys::KeyRepeat::default();
    // desktop pointer: the cursor stamped over presented CPU frames, when the pointer last moved or
    // clicked (it hides after a while), and the damage of a frame with the cursor's rectangles added
    let mut cursor = springboard::gfx::cursor::Cursor::default();
    let mut cursor_damage = springboard::gfx::Damage::new(w, h);
    let mut cursor_dim = (w, h);
    let mut last_motion = f64::NEG_INFINITY;
    let mut last_pointer = ((0.0f32, 0.0f32), false);
    // the last frame on the panel came from the CPU path, so `frame` is what the panel shows
    let mut cpu_shown = false;
    let mut halting = false;
    let mut low_power = false;
    let mut last_present = f64::NEG_INFINITY;
    let mut trimmed_after = f64::NAN;
    #[cfg(feature = "desktop-wayland")]
    let mut presenter_scale = 0.0f32;
    let mut last_frame_start = f64::NEG_INFINITY;
    let mut boost = springboard::bare::boost::Boost::new();
    let mut gpu_comp = GpuComp::new(w, h);
    if wayland { gpu_comp.disable(); } else { gpu_comp.warm(); }
    // Developer > Zero-Copy GPU Output (read once; any error keeps the copy path)
    let mut scan = springboard::bare::scanout::FrameSwitch::default();
    // zero-copy (dumb-dmabuf, else the glReadPixels copy) is on unless the preference says otherwise
    let scan_pref = prefs.get(springboard::shell::settings::GPU_SCANOUT_PREF);
    let desktop_default = scan_pref.is_none();
    if !wayland && (desktop_default || springboard::bare::scanout::pref_on(scan_pref)) {
        match gpu_comp.enable_scanout(&mut disp) {
            Ok(what) => {
                eprintln!("springboard: zero-copy scanout on: {what}");
                scan.enabled();
            }
            Err(e) => eprintln!("springboard: zero-copy scanout off: {e}"),
        }
    }
    // the last frame came from the layer compositor (video layer or GPU rotation): its target
    // still holds that frame, so the next composed frame may draw only what changed
    let mut last_layers = false;
    // CPU profile: the Power Mode pref plus battery and Low Power Mode pick it (see bare::cpuprofile)
    let mut profile_auto = springboard::bare::cpuprofile::Auto::default();
    let mut profile_pref = shell.power_pref();
    let mut profile_low_power = low_power;
    let mut last_profile_check = f64::NEG_INFINITY;
    boost.set_profile(springboard::bare::cpuprofile::resolve(profile_pref, &mut profile_auto, power::battery().0, power::battery().1, low_power));
    let chord_key = |code: u16| match code {
        input::KEY_POWER => Some(springboard::shell::chord::Key::Power),
        input::KEY_VOLUMEUP => Some(springboard::shell::chord::Key::VolumeUp),
        input::KEY_VOLUMEDOWN => Some(springboard::shell::chord::Key::VolumeDown),
        _ => None,
    };
    eprintln!("springboard: {w}x{h} @ {}Hz", disp.mode.vrefresh);

    let mut fds = Vec::new();
    let mut idle_stats = springboard::bare::heartbeat::IdleStats::default();
    // what drawing and copying a frame costs, to start it as late as the vblank allows
    let mut frame_cost = kms::FrameCost::new(period);
    loop {
        let present_at_start = last_present;
        #[cfg(feature = "desktop-wayland")]
        if wayland && disp.wayland_lock(shell.is_locked())? { shell.force_full_redraw(); }
        #[cfg(feature = "desktop-wayland")]
        if wayland {
            if let Some((wp, x)) = shell.desktop_wallpaper_once() { disp.wayland_wallpaper(wp, x); }
            if disp.wayland_gpu_active() { shell.note_wallpaper_uploaded(); }
        }
        #[cfg(feature = "desktop-wayland")]
        let previous_present = last_present;
        if disp.boot_fading() { shell.force_full_redraw(); }
        let animating = shell.needs_frame() || disp.boot_fading();
        // Animating: draw as soon as the previous flip has landed, otherwise
        // sleep until the flip event (paced by the display, not a timer).
        // Idle, the only timer is the status clock: once a minute, unless an
        // app wants live status or Wi-Fi is still settling.
        // the next frame is drawn while the previous flip is still pending: drawing goes to `frame`,
        // and present()/flip_scanout() wait for the flip only when handing the buffer over, so a
        // 16-20 ms frame no longer waits for a whole extra vblank (33 ms)
        let mut timeout = match (animating, disp.busy()) {
            // animating: the frame wait is computed below in fractional ms
            (true, _) => i32::MAX,
            _ if shell.screen_on() && !wayland && !wifi_settled(&wifi) => 1000,
            _ => (60 - sys::local_time().2) * 1000 + 50,
        };
        let now = sys::now_ms();
        springboard::bare::heartbeat::beat();
        boost.update(animating, now);
        timeout = timeout.min(springboard::bare::heartbeat::BEAT_CAP_MS);
        // the GPU pool's idle trim is one one-shot deadline, not a poll
        #[cfg(feature = "desktop-wayland")]
        if let Some(ms) = disp.wayland_trim_wake_ms() { timeout = timeout.min(ms); }
        // so is a glass backdrop rebuild the 30 Hz cap skipped (a window behind glass changed meanwhile)
        #[cfg(feature = "desktop-wayland")]
        if let Some(ms) = disp.wayland_glass_wake_ms() { timeout = timeout.min(ms); }
        inputs.rescan_if_due(now);
        timeout = timeout.min(inputs.next_scan().map_or(i32::MAX, |at| (at - now).ceil().max(0.0) as i32));
        // the pointer's wheel ease and right-click hold, key auto-repeat, and the cursor hiding
        let hide_cursor = (inputs.has_pointer() && cursor.is_shown()).then_some(last_motion + springboard::gfx::cursor::HIDE_MS).filter(|at| *at > now);
        for due in inputs.deadline().into_iter().chain(key_repeat.deadline()).chain(hide_cursor).chain(sleeper.deadline()).chain(synthetic.front().map(|s| s.0)).chain(shell.next_wake(now)).chain(chord.deadline()).chain(power_key.deadline()).chain(boost.deadline()).chain(brightness_unsaved.map(|at| at + BRIGHTNESS_SAVE_MS)) {
            timeout = timeout.min((due - now).ceil().max(0.0) as i32);
        }
        // fixed slots first: touch, keys, display, control; then the shell's
        fds.clear();
        fds.extend([touch.fd, keys.fd, disp.fd(), control.as_ref().map_or(-1, |c| c.fd())]);
        fds.extend(shell.poll_fds());
        let shell_fds_end = fds.len();
        fds.push(headset.as_ref().map_or(-1, |d| d.fd));
        fds.extend(inputs.fds());
        // animating: wait for the frame interval and, when the flips are predictable, for the
        // moment the frame must start to be done by the vblank (ppoll: sub-millisecond, 120 Hz)
        let mut wait_ms = timeout as f64;
        if animating {
            let interval = low_battery::frame_interval_ms(low_power).max(boost.frame_interval_ms());
            let mut frame_wait = last_present + interval - now;
            if disp.fd() >= 0 {
                if let Some(start) = kms::draw_start(last_flip, period, now, frame_cost.ms(), disp.busy()) {
                    frame_wait = frame_wait.max(start - now);
                }
            }
            wait_ms = wait_ms.min(frame_wait.max(0.0));
        }
        // idle after drawing (Snow Leopard's scavenger): give freed heap back once, re-armed by the next frame
        if !animating && wait_ms >= 1000.0 && (last_present != trimmed_after || shell.take_heap_trim_due()) {
            trimmed_after = last_present;
            shell.trim_caches();
            bifrost_core::host::pressure_relief();
        }
        kms::wait_precise(&fds, wait_ms, &mut ready);
        let ready = &ready;
        let t = sys::now_ms();
        if ready[4..shell_fds_end].contains(&true) {
            shell.pump();
        }
        if ready[2] {
            if let Some(v) = disp.handle_event() {
                last_flip = Some(v);
            }
        }
        // animation time = when the frame being drawn will reach the screen
        let vt = kms::next_vsync(last_flip, period, t);
        events.clear();
        #[cfg(feature = "desktop-wayland")]
        {
            disp.wayland_inputs(&mut events);
            if disp.wayland_take_glass_retry() { shell.request_frame(); }
            if let Some(windows)=disp.wayland_foreign_windows() { shell.wayland_foreign_windows(windows); }
            for ev in disp.wayland_window_events() { shell.wayland_window_event(ev); }
        }
        touch.read(&mut events);
        keys.read(&mut events);
        inputs.read(&mut events);
        inputs.tick(t, &mut events);
        // the pointer moving or a button changing shows the cursor again
        let pointer_now = (inputs.pointer(), inputs.is_down());
        if pointer_now != last_pointer {
            last_pointer = pointer_now;
            last_motion = t;
        }
        if !events.is_empty() {
            // the first frames of the response to a touch already need full clocks
            boost.update(true, t);
        }
        if !events.is_empty() && trace {
            eprintln!("springboard: input {events:?}");
        }
        let mut effects = Vec::new();
        #[cfg(feature = "desktop-wayland")]
        if wayland {
            let (active,resumed)=disp.wayland_activity();
            if active {shell.wayland_activity(t);}
            if resumed && !shell.screen_on() {effects.extend(shell.event(Event::Wake,t));}
        }
        if let Some(e) = power_key.filter(chord.due(t)) {
            effects.extend(shell.event(e, t));
        }
        if power_key.due(t) {
            shell.open_power_menu(t);
        }
        if let (true, Some(server)) = (ready[3], control.as_ref()) {
            use springboard::bare::control::{self, Key, Request};
            if let Some((stream, req)) = server.accept() {
                let mut path = |from, to, ms| synthetic.extend(control::path(from, to, ms).into_iter().map(|(dt, ph, x, y)| (t + dt, ph, x, y)));
                let body: Vec<u8> = match req {
                    Ok(Request::Shot(k)) => control::snapshot(&frame, fw, fh, k),
                    Ok(Request::Context(x,y))=>{let (x,y)=shell.orientation().to_logical(x,y,w as i32,h as i32);shell.desktop_pointer(x,y,true,t);b"ok\n".to_vec()},
                    Ok(Request::Hover(x,y))=>{let (x,y)=shell.orientation().to_logical(x,y,w as i32,h as i32);shell.desktop_pointer(x,y,false,t);b"ok\n".to_vec()},
                    Ok(Request::AxList(prefix))=>shell.ax_listing(&prefix,t).into_bytes(),
                    Ok(Request::AxActions(q))=>match shell.ax_actions(&q,t){Ok(names)=>names.into_bytes(),Err(e)=>format!("error: {e}\n").into_bytes()},
                    Ok(Request::AxAction(q,name))=>match shell.ax_action(&q,&name,t){Ok((address,fx))=>{effects.extend(fx);format!("ok {address}\n").into_bytes()},Err(e)=>format!("error: {e}\n").into_bytes()},
                    Ok(Request::AxPress(q))=>match shell.ax_press(&q,t){Ok((address,fx))=>{effects.extend(fx);format!("ok {address}\n").into_bytes()},Err(e)=>format!("error: {e}\n").into_bytes()},
                    Ok(Request::Appearance(mode))=>{match shell.set_appearance(&mode){Ok(())=>b"ok\n".to_vec(),Err(e)=>format!("error: {e}\n").into_bytes()}},
                    Ok(Request::OpenApp(id))=>{if shell.desktop_open_app(&id,t){b"ok\n".to_vec()}else{b"error: App not installed\n".to_vec()}},
                    Ok(Request::OpenFolder(id,folder))=>{match shell.desktop_open_folder(&id,&folder,t){Ok(())=>b"ok\n".to_vec(),Err(e)=>format!("error: {e}\n").into_bytes()}},
                    Ok(Request::OpenUrl(url))=>{match shell.desktop_open_url(&url,t){Ok(())=>b"ok\n".to_vec(),Err(e)=>format!("error: {e}\n").into_bytes()}},
                    Ok(Request::Tap(x, y)) => {
                        path((x, y), (x, y), 40.0);
                        b"ok\n".to_vec()
                    }
                    Ok(Request::Swipe { from, to, ms }) => {
                        path(from, to, ms);
                        b"ok\n".to_vec()
                    }
                    Ok(Request::Touch(phase, x, y)) => {
                        // a live finger: lands after anything already queued, in order
                        let at = synthetic.back().map_or(t, |b| b.0.max(t));
                        synthetic.push_back((at, phase, x, y));
                        b"ok\n".to_vec()
                    }
                    Ok(Request::DoubleTap(x, y)) => {
                        synthetic.extend(control::double_tap(x, y).into_iter().map(|(dt, ph, x, y)| (t + dt, ph, x, y)));
                        b"ok\n".to_vec()
                    }
                    Ok(Request::Key(k)) => {
                        // pressed, never released: `key volup` then `key power` is the screenshot chord
                        let k = match k {
                            Key::Power => springboard::shell::chord::Key::Power,
                            Key::VolumeUp => springboard::shell::chord::Key::VolumeUp,
                            Key::VolumeDown => springboard::shell::chord::Key::VolumeDown,
                        };
                        if let Some(e) = chord.key(k, true, t) {
                            effects.extend(shell.event(e, t));
                        }
                        // volume keys act on release; Power stays held for the chord window
                        if k != springboard::shell::chord::Key::Power {
                            if let Some(e) = chord.key(k, false, t) {
                                effects.extend(shell.event(e, t));
                            }
                        }
                        b"ok\n".to_vec()
                    }
                    Ok(Request::State) => format!("version={}\nready={}\n{}fps_draw_ms={:.1}\nfps_present_ms={:.1}\n", env!("CARGO_PKG_VERSION"), ready_marker() as u8, shell.describe(), perf.last.0, perf.last.1).into_bytes(),
                    Ok(Request::Version) => format!("version={}\n", env!("CARGO_PKG_VERSION")).into_bytes(),
                    Ok(Request::Ready) => format!("ready={}\n", ready_marker() as u8).into_bytes(),
                    Ok(Request::QuickSearch) => {
                        effects.extend(shell.hw_shortcut(springboard::shell::hwkeys::Shortcut::QuickSearch, t));
                        b"ok\n".to_vec()
                    }
                    Ok(Request::Lock) => {
                        effects.extend(shell.lock_screen());
                        b"ok\n".to_vec()
                    }
                    Ok(Request::Term(verb, args)) => match shell.term_control(&verb, &args, t) {
                        Ok(body) => body.into_bytes(),
                        Err(e) => format!("error: {e}\n").into_bytes(),
                    },
                    Ok(Request::Perf) => jank.describe().into_bytes(),
                    Ok(Request::Trace(on)) => {
                        trace = on;
                        b"ok\n".to_vec()
                    }
                    Err(e) => format!("error: {e}\n").into_bytes(),
                };
                control::reply(stream, &body);
            }
        }
        while synthetic.front().is_some_and(|s| s.0 <= t) {
            let (_, phase, x, y) = synthetic.pop_front().unwrap();
            let e = match phase {
                0 => Event::Down(x, y),
                1 => Event::Move(x, y),
                _ => Event::Up(x, y),
            };
            if trace {
                eprintln!("springboard: synthetic {e:?}");
            }
            effects.extend(shell.event(e, t));
        }
        // real touches arrive in panel coordinates and are turned into the
        // frame's; synthetic ones already name frame pixels
        let orientation = shell.orientation();
        let map = |x: f32, y: f32| orientation.to_logical(x, y, w as i32, h as i32);
        for ev in &events {
            let e = match *ev {
                input::Event::Down { x, y, .. } => {
                    let (x, y) = map(x, y);
                    Event::Down(x, y)
                }
                input::Event::Move { x, y, .. } => {
                    let (x, y) = map(x, y);
                    Event::Move(x, y)
                }
                input::Event::Up { x, y, .. } => {
                    let (x, y) = map(x, y);
                    Event::Up(x, y)
                }
                input::Event::Hover{x,y,..}=>{let (x,y)=map(x,y);shell.desktop_pointer(x,y,false,t);continue;},
                input::Event::Context{x,y,..}=>{let (x,y)=map(x,y);shell.desktop_pointer(x,y,true,t);continue;},
                input::Event::Switch { .. } => continue,
                input::Event::Key { code: input::KEY_WAKEUP, down: true } => Event::Wake,
                input::Event::Key { code, down } => {
                    let raw_sc = hw_keys.key_in(code, down, shell.term_front());
                    if trace {
                        eprintln!("springboard: key {code} {} shortcut={raw_sc:?}", if down { "down" } else { "up" });
                    }
                    if !down && !hw_keys.mods_held() {
                        shell.cmdtab_release(t);
                    }
                    if let Some(sc) = raw_sc.filter(|sc| shell.hw_window_shortcut(*sc) || !matches!(sc, springboard::shell::hwkeys::Shortcut::Close | springboard::shell::hwkeys::Shortcut::Minimize)) {
                        // a shortcut ends any key repeat in progress
                        key_repeat.press(code, None, t);
                        effects.extend(shell.hw_shortcut(sc, t));
                        continue;
                    }
                    if !down {
                        key_repeat.release(code);
                    }
                    // an open menu-bar menu takes the arrows, Return and Esc; other keys are swallowed
                    if let Some(fx) = shell.hw_menu_key(code, down, t) {
                        effects.extend(fx);
                        continue;
                    }
                    // Deildu or Files on screen: they take their own keys (power, volume and wake stay here)
                    if shell.hw_key(code, down, t) {
                        continue;
                    }
                    // Ctrl or Cmd with a key the shell did not take: an external app in front may use it
                    if let Some(mods) = hw_keys.chord_mods(code).filter(|_| down) {
                        if shell.hw_chord(code, mods) {
                            continue;
                        }
                    }
                    // a physical keyboard types into the terminal and text fields
                    // only a key-down types: a dead key keeps state between presses
                    let typed = if down { hw_keys.typed(code) } else { None };
                    if let Some(out) = typed {
                        key_repeat.press(code, Some(out.clone()), t);
                        effects.extend(shell.hw_type(out, t));
                        continue;
                    }
                    let key = chord_key(code);
                    if code == input::KEY_POWER {
                        if down {
                            power_key.down(t, shell.screen_on());
                        }
                    } else if down && key.is_some() {
                        power_key.taint();
                    }
                    let mut e = power_key.filter(key.and_then(|k| chord.key(k, down, t)));
                    if code == input::KEY_POWER && !down {
                        e = e.or_else(|| power_key.up());
                    }
                    match e {
                        Some(e) => e,
                        None => continue,
                    }
                }
            };
            effects.extend(shell.event(e, t));
        }
        // held keys repeat once their delay is up
        for out in key_repeat.due(t) {
            effects.extend(shell.hw_type(out, t));
        }
        // jack events: pause before switching so an unplug never blasts the speaker
        if let Some(d) = headset.as_mut() {
            jack_events.clear();
            d.read(&mut jack_events);
            for ev in &jack_events {
                let input::Event::Switch { code, on } = *ev else { continue };
                let Some(change) = jack.feed(code, on).and_then(|plugged| router.jack(plugged)) else { continue };
                eprintln!("springboard: audio route -> {} ({}%)", change.route.label(), (change.volume.level() * 100.0).round());
                if change.pause {
                    shell.pause_music();
                }
                vol = change.volume;
                apply_volume(vol, mixer.as_ref());
                shell.set_route(change.route, vol.level(), change.hud, t);
                shell.save_prefs(&change.prefs);
            }
        }
        if brightness_unsaved.is_some_and(|at| t - at >= BRIGHTNESS_SAVE_MS) {
            brightness_unsaved = None;
            shell.save_prefs(&[("brightness", format!("{level:.3}"))]);
        }
        effects.extend(shell.tick(vt));
        #[cfg(feature = "desktop-wayland")]
        effects.extend(shell.take_native_activations().into_iter().map(Effect::ActivateForeign));
        for fx in effects {
            match fx {
                Effect::Brightness(f) => {
                    level = f.max(0.02);
                    brightness_unsaved = Some(t);
                    power::set_brightness(panel_level(level, low_power, max));
                }
                Effect::ScreenOff => {
                    hw_keys.reset();
                    key_repeat = Default::default();
                    power::set_brightness(0);
                    if let Err(e) = disp.set_dpms(false) {
                        eprintln!("springboard: dpms off: {e}");
                    }
                    sleeper.screen_off(t);
                }
                Effect::SleepSystem => sleeper.sleep_now(t),
                // key clicks are not played yet (no sound unless the user asks for it)
                Effect::KeyClick(_) => {}
                #[cfg(feature = "desktop-wayland")]
                #[cfg(feature="desktop-wayland")]
                Effect::ForeignWindowMaximize(id,maximized)=>{if let Err(e)=disp.wayland_foreign_maximize(id,maximized){eprintln!("desktop maximize: {e}");}}
                #[cfg(feature="desktop-wayland")]
                Effect::ForeignWindowAction(id,minimize)=>{if let Err(e)=disp.wayland_foreign_action(id,minimize){eprintln!("desktop window command: {e}");}}
                #[cfg(feature="desktop-wayland")]
                Effect::DesktopShortcut(code)=>springboard::bare::desktop_input::shortcut(code),
                #[cfg(feature="desktop-wayland")]
                Effect::ActivateForeign(id) => { if let Err(e)=disp.wayland_activate_foreign(id) { eprintln!("BifrOSt dock activation: {e}"); } }
                Effect::RotationLock(on) => {
                    if let Err(e) = prefs.set("rotation_lock", if on { "1" } else { "0" }) {
                        eprintln!("springboard: settings.ini: {e}");
                    }
                }
                Effect::ScreenOn => {
                    hw_keys.reset();
                    key_repeat = Default::default();
                    sleeper.screen_on(t);
                    if let Err(e) = disp.set_dpms(true) {
                        eprintln!("springboard: dpms on: {e}");
                    }
                    power::set_brightness(panel_level(level, low_power, max));
                }
                Effect::Volume(v) => {
                    vol = Volume::new(v);
                    apply_volume(vol, mixer.as_ref());
                    shell.set_volume(v);
                    shell.save_prefs(&router.set_level(v));
                }
                Effect::Wifi(a) if wayland => springboard::shell::desktop_network::send(springboard::shell::desktop_network::Request::Wifi(a)),
                Effect::Wifi(a) => wifi.send(match a {
                    WifiAction::Scan => Cmd::Scan,
                    WifiAction::Enable(on) => Cmd::Enable(on),
                    WifiAction::Select(ssid) => Cmd::Select(ssid),
                    WifiAction::Join { ssid, psk } => Cmd::Join { ssid, psk },
                    WifiAction::Forget(ssid) => Cmd::Forget(ssid),
                    WifiAction::JoinOther(cfg) => Cmd::JoinOther(cfg),
                    WifiAction::SetAutoJoin { ssid, on } => Cmd::SetAutoJoin { ssid, on },
                    WifiAction::Move { ssid, up } => Cmd::Move { ssid, up },
                }),
                Effect::Bluetooth(c) => bluetooth.send(c),
                // sound comes later; the full-screen alert already shows
                Effect::Alarm => eprintln!("springboard: alarm"),
                Effect::WakeAt(at) => rtc_wake(at),
                // the shell applies the timeout itself; log it for the record
                Effect::SystemSleepAfter(min) => sleeper.set_after(sleep::system_sleep_after(Some(&min.to_string())), t),
                Effect::AutoLock(ms) => eprintln!("springboard: auto-lock {ms} ms"),
                // exit code 3 asks the service manager to start the shell again
                Effect::Restart => {
                    eprintln!("springboard: restart requested");
                    shell.session_save_for_exit();
                    shell.stop_apps();
                    springboard::bare::crashlog::write_last_exit(&dir, RESTART_EXIT, "restart requested");
                    power::set_brightness(panel_level(level, low_power, max));
                    boost.release();
                    std::process::exit(RESTART_EXIT);
                }
                Effect::Power(_) if halting => {}
                Effect::Power(action) => {
                    eprintln!("springboard: power {action:?}");
                    // app data and settings first; `halt` syncs the disks
                    shell.flush();
                    shell.session_save_for_exit();
                    shell.stop_apps();
                    match action {
                        PowerAction::Off => {
                            halting = true;
                            power::halt(power::Halt::Shutdown);
                        }
                        PowerAction::Restart => {
                            halting = true;
                            power::halt(power::Halt::Reboot);
                        }
                    }
                }
                Effect::Wallpaper(id) => match map_wallpaper(&dir, &id).and_then(|m| View::from_raw(m).map(|v| (m, v))) {
                    Some((m, view)) => {
                        shell.set_wallpaper(view);
                        unmap(std::mem::replace(&mut mapped, m));
                        current_wallpaper = id;
                    }
                    None => eprintln!("springboard: no wallpaper {id}"),
                },
                Effect::LockWallpaper(id) => set_lock_wallpaper(&mut shell, &dir, &id, &mut lock_mapped),
                Effect::WallpaperDim(on) => shell.set_wallpaper_dim(on),
            }
        }
        // Low Power Mode flipped (Settings, or the alert's button): re-cap the backlight now
        if shell.low_power() != low_power {
            low_power = shell.low_power();
            eprintln!("springboard: low power mode {}", if low_power { "on" } else { "off" });
            if shell.screen_on() {
                power::set_brightness(panel_level(level, low_power, max));
            }
        }
        // every 10 s, and at once when Power Mode or Low Power Mode changed
        if t - last_profile_check > 10_000.0 || shell.power_pref() != profile_pref || low_power != profile_low_power {
            last_profile_check = t;
            profile_pref = shell.power_pref();
            profile_low_power = low_power;
            let (percent, charging) = power::battery();
            let before = boost.profile();
            boost.set_profile(springboard::bare::cpuprofile::resolve(profile_pref, &mut profile_auto, percent, charging, low_power));
            if boost.profile() != before {
                eprintln!("springboard: power profile {}", boost.profile().label());
            }
        }
        if t - last_sample > battery::SAMPLE_MS {
            last_sample = t;
            // no battery, no sample: a desktop must not log a made-up 100%
            if let Some((percent, charging)) = power::battery_reading() {
                let (unix, _) = sys::wall_time();
                if let Err(e) = battery::record(&battery_log, std::sync::Arc::make_mut(&mut history), battery::Sample { unix, percent, charging }) {
                    eprintln!("springboard: battery.log: {e}");
                }
            }
        }
        sleeper.set_busy(shell.keeps_awake(), t);
        if emulated.is_none() {
            if let Some(plugged) = ac_watch.poll(t) {
                eprintln!("springboard: mains {}", if plugged { "plugged" } else { "unplugged" });
            }
        }
        if let Some(outcome) = sleeper.tick(t) {
            eprintln!("springboard: suspend {outcome:?}");
        }
        // after the effects, so a Wi-Fi switch flip is already reflected
        let network_changed=wayland && springboard::shell::desktop_network::take_changed();
        if t - last_info > 1000.0 || network_changed {
            last_info = t;
            let mut info = gather(level, &current_wallpaper, &history);
            info.power_profile = boost.profile().label().into();
            if wayland {
                let network=springboard::shell::desktop_network::snapshot();
                info.wifi_on=network.wifi_on;
                info.wifi=if !network.wifi_available {"No adapter".into()} else if !network.wifi_on {"Off".into()} else {network.ssid.clone().unwrap_or_else(||"Not Connected".into())};
                info.current_ssid=network.ssid.clone();info.networks=network.networks.clone();info.saved=network.saved.clone();
                info.wifi_notice=network.error.clone();
                // the link kinds decide the glyph: a wired cable never shows as Wi-Fi
                info.online=network.interfaces.iter().any(|i|i.kind=="wifi"&&i.connected());
                info.wired=network.interfaces.iter().any(|i|i.kind=="ethernet"&&i.connected());
                info.wifi_adapter=network.wifi_available;
                info.desktop_network=Some(network);
            }
            shell.set_info(info);
            let (unix, offset) = sys::wall_time();
            shell.set_wall(unix, offset, sys::now_ms());
            // Night Shift multipliers change by the minute or by a settings flip: once per info tick
            let m = springboard::shell::night_shift::multipliers(springboard::shell::notify::schedule::Schedule::minute_of_day(unix, offset));
            if m != night_mult {
                night_mult = m;
                shell.force_full_redraw();
            }
        }
        {
            let now = sys::now_ms();
            if let Some(l) = jank.idle(now) {
                eprintln!("{l}");
            }
            let (unix, off) = sys::wall_time();
            let (y, mo, d) = springboard::bare::timezone::civil_from_days((unix + off as i64).div_euclid(86_400));
            let (h, mi, sec, ..) = sys::local_time();
            if let Some(line) = jank.summary((y as i32, mo as i32, d as i32), (h, mi, sec), now) {
                springboard::bare::jank::append(&dir, &line);
            }
        }
        // draw once the frame interval has passed and, when flips are predictable, the late start
        // time has come (input was read just above, right before drawing)
        let frame_due = |now: f64| {
            let start = if disp.fd() >= 0 { kms::draw_start(last_flip, period, now, frame_cost.ms(), disp.busy()) } else { None };
            now + 0.5 >= last_present + low_battery::frame_interval_ms(low_power).max(boost.frame_interval_ms()) && start.map_or(true, |s| now + 0.2 >= s)
        };
        if shell.needs_frame() && frame_due(sys::now_ms()) {
            // the shell draws a logical frame (turned on its side in landscape); the panel gets it rotated
            let (lw, lh) = shell.size();
            (fw, fh) = (lw as usize, lh as usize);
            if frame.len() != fw * fh {
                frame = vec![0u32; fw * fh];
            }
            let rot = shell.orientation().rotation();
            cpu_shown = false;
            let reason = if trace { shell.frame_reason() } else { String::new() };
            let t0 = sys::now_ms();
            let jctx = shell.jank_context();
            let mut scan_failed = false;
            if scan.live && !gpu_comp.scanout_active() {
                // the GPU compositor went away with the ring: CPU frames from here on
                eprintln!("springboard: zero-copy scanout lost with the gpu compositor");
                scan.kill();
                shell.force_full_redraw();
            }
            // open/close zoom on the GPU when it works (AKURAI_GPU=0 or any failure keeps the CPU path);
            // the GPU path composes the portrait frame, so a turned screen always takes the CPU path
            let mut sw_frame = false;
            let step = if rot != Rotation::None || night_mult.is_some() {
                GpuStep::Cpu
            } else {
                let view = shell.zoom_view(vt).map(|z| (z.card, z.rect, z.radius, z.opening));
                let sh = &shell;
                let zoom = gpu_comp.step(view, &mut frame, &mut |px| sh.render_home(&mut Surface { w, h, stride: w, px, y0: 0 }));
                if matches!(zoom, GpuStep::Cpu) {
                    // the app switcher: quads over textures uploaded once (CPU path on any failure)
                    shell.switcher_prepare();
                    match shell.switcher_view(vt) {
                        Some(v) => {
                            let st = gpu_comp.switcher(&v, &mut frame);
                            sw_frame = matches!(st, GpuStep::Presented { .. });
                            st
                        }
                        None => {
                            if gpu_comp.switcher_over() {
                                shell.force_full_redraw();
                            }
                            zoom
                        }
                    }
                } else {
                    zoom
                }
            };
            if trace && shell.zoom_active() && matches!(step, GpuStep::Cpu) && rot == Rotation::None {
                eprintln!("springboard: zoom on cpu, blocked by {} [{} | {}]", shell.zoom_blocker().unwrap_or("-"), shell.zoom_debug(), gpu_comp.state());
            }
            if let GpuStep::Presented { damage: a, gpu_ms } = step {
                let mut dmg = springboard::gfx::Damage::new(w, h);
                dmg.add(a.x, a.y, a.w, a.h);
                let t1 = sys::now_ms();
                if scan.live {
                    scan_failed |= !scan_flip(&mut gpu_comp, &mut disp, &mut scan);
                } else {
                    disp.present(&frame, &dmg);
                }
                let t2 = sys::now_ms();
                let pct = dmg.pixels() as f64 / (w * h) as f64;
                perf.add(gpu_ms, t2 - t1, pct);
                slow_frame(&mut jank, &jctx, "gpu", Frame { total: t2 - t0, draw: (t1 - t0 - gpu_ms).max(0.0), gpu: gpu_ms, present: t2 - t1, damage_pct: 100.0 * pct, ..Frame::default() }, t2);
                shell.perf_frame(gpu_ms);
                if trace {
                    let path = if sw_frame { format!(" path switcher uploaded {}", gpu_comp.switcher_uploads()) } else { String::new() };
                    eprintln!("springboard: frame gpu {:.1} present {:.1} damage {:.0}% gap {:.1}{path}", gpu_ms, t2 - t1, 100.0 * pct, t0 - last_frame_start);
                }
                last_frame_start = t0;
                last_present = t1;
                frame_cost.note(t2 - t0);
                last_layers = false;
            } else if matches!(step, GpuStep::Ended) {
                shell.force_full_redraw();
            }
            // Video layers and landscape frames go through the GPU's layer compositor when it
            // works: the app's picture is sampled straight from the decoder's buffer and the CPU
            // frame (an overlay over the picture, or the whole frame) is uploaded where it
            // changed and drawn turned; only the GPU's output is copied to the panel.
            let mut composed = false;
            // the rotation turn on the GPU: both snapshots are textures, each frame draws two
            // turned, scaled quads cross-faded; the shell does not draw the CPU frame meanwhile
            let mut turned = false;
            if matches!(step, GpuStep::Cpu) {
                let fits = rot.size(fw, fh) == (w, h);
                match shell.turn_view(vt) {
                    Some(tv) if springboard::gpu::plan::turn_on_gpu(gpu_comp.caps(), fits && tv.new.w == fw && tv.new.h == fh, night_mult.is_some()) => {
                        if panel.len() != w * h {
                            panel = vec![0u32; w * h];
                        }
                        let k = tv.k;
                        if let Some(c) = gpu_comp.turn(tv.id, tv.old, tv.new, tv.from, tv.k, rot, &mut panel) {
                            let mut dmg = springboard::gfx::Damage::new(w, h);
                            dmg.all();
                            let t2 = sys::now_ms();
                            if scan.live {
                                scan_failed |= !scan_flip(&mut gpu_comp, &mut disp, &mut scan);
                            } else {
                                disp.present(&panel, &dmg);
                            }
                            let t3 = sys::now_ms();
                            perf.add(t2 - t0, t3 - t2, 1.0);
                            slow_frame(&mut jank, &jctx, "rotate", Frame { total: t3 - t0, upload: c.upload_ms, gpu: c.gpu_ms, present: t3 - t2, damage_pct: 100.0, ..Frame::default() }, t3);
                            shell.perf_frame(t2 - t0);
                            if trace {
                                eprintln!(
                                    "springboard: frame turn upload {:.1} gpu {:.1} copy {:.1} present {:.1} damage 100% uploaded {}k px k {:.2} gap {:.1}",
                                    c.upload_ms,
                                    c.gpu_ms,
                                    c.copy_ms,
                                    t3 - t2,
                                    c.uploaded_px / 1000,
                                    k,
                                    t0 - last_frame_start
                                );
                            }
                            shell.turn_drawn();
                            last_frame_start = t0;
                            last_present = t2;
                            frame_cost.note(t3 - t0);
                            last_layers = true;
                            turned = true;
                        }
                    }
                    Some(_) => {}
                    None => gpu_comp.turn_over(),
                }
            }
            if !turned && !matches!(step, GpuStep::Presented { .. }) {
                use springboard::gpu::plan::{choose, Path};
                let fits = rot.size(fw, fh) == (w, h);
                let video_ok = gpu_comp.video_capable();
                let layer = shell.video_layer(video_ok, fits);
                let path = choose(gpu_comp.caps(), layer.is_some(), rot, fits);
                // the Night Shift tint is applied on the CPU copy, which zero-copy output skips
                if path != Path::Cpu && !(scan.live && night_mult.is_some()) {
                    if panel.len() != w * h {
                        panel = vec![0u32; w * h];
                    }
                    if !last_layers && scan.cpu_frame() {
                        // zoom and switcher frames did not leave their pixels in `frame`
                        shell.force_full_redraw();
                    }
                    let damage = shell.draw(&mut Surface { w: fw, h: fh, stride: fw, px: &mut frame, y0: 0 }, vt);
                    let t1 = sys::now_ms();
                    let video = layer.as_ref().map(|l| springboard::bare::gpu_comp::VideoInput { picture: &l.frame, dst: l.dst });
                    if let Some(c) = gpu_comp.compose(video, &frame, fw, fh, damage.areas(), rot, !last_layers, &mut panel) {
                        let mut dmg = springboard::gfx::Damage::new(w, h);
                        for r in &c.damage {
                            dmg.add(r.x, r.y, r.w, r.h);
                        }
                        if let Some(m) = night_mult {
                            springboard::shell::night_shift::tint_in_place(&mut panel, w, &dmg, m);
                        }
                        let t2 = sys::now_ms();
                        // nothing changed on the panel (the shell redrew identical pixels): no copy, no flip
                        if scan.live {
                            scan_failed |= !scan_flip(&mut gpu_comp, &mut disp, &mut scan);
                        } else if !dmg.is_empty() {
                            disp.present(&panel, &dmg);
                        }
                        let t3 = sys::now_ms();
                        let pct = dmg.pixels() as f64 / (w * h) as f64;
                        perf.add(t2 - t0, t3 - t2, pct);
                        slow_frame(&mut jank, &jctx, if path == Path::Video { "video" } else { "rotate" }, Frame { total: t3 - t0, draw: t1 - t0, upload: c.upload_ms, gpu: c.gpu_ms, present: t3 - t2, damage_pct: 100.0 * pct }, t3);
                        shell.perf_frame(t2 - t0);
                        if trace {
                            let src = match c.source {
                                Some(springboard::gpu::plan::Source::External) => "nv12",
                                Some(springboard::gpu::plan::Source::Upload) => "upload",
                                None => "-",
                            };
                            let kind = if path == Path::Video { "video" } else { "rotate" };
                            eprintln!(
                                "springboard: frame {kind} draw {:.1} upload {:.1} gpu {:.1} copy {:.1} present {:.1} damage {:.0}% uploaded {}k px picture {src} gap {:.1} why {reason}",
                                t1 - t0,
                                c.upload_ms,
                                c.gpu_ms,
                                c.copy_ms,
                                t3 - t2,
                                100.0 * pct,
                                c.uploaded_px / 1000,
                                t0 - last_frame_start
                            );
                        }
                        last_frame_start = t0;
                        last_present = t2;
                        frame_cost.note(t3 - t0);
                        composed = true;
                    } else {
                        // the compositor could not take it: withdraw the layer (the app painted only
                        // its overlay) and let the CPU draw this frame in full
                        shell.video_layer(gpu_comp.video_capable(), false);
                        shell.force_full_redraw();
                    }
                } else if layer.is_some() {
                    shell.video_layer(video_ok, false);
                }
                if !composed && last_layers {
                    // the presenter's buffers hold GPU frames the CPU did not draw
                    shell.force_full_redraw();
                }
                last_layers = composed;
            }
            if !matches!(step, GpuStep::Presented { .. }) && !composed && !turned {
                if scan.cpu_frame() {
                    // the panel showed a GPU frame the shell's buffers do not hold
                    shell.force_full_redraw();
                }
                // the desktop pointer is stamped over CPU frames that are not turned (the GPU zoom,
                // the switcher and a rotated screen stay without it)
                let want = springboard::gfx::cursor::Spot::want(inputs.pointer(), inputs.is_down(), inputs.has_pointer(), shell.screen_on(), sys::now_ms(), last_motion);
                let damage = shell.draw(&mut Surface { w: fw, h: fh, stride: fw, px: &mut frame, y0: 0 }, vt);
                let t1 = sys::now_ms();
                if rot == Rotation::None && !panel.is_empty() {
                    // back to portrait: the rotated panel copy is no longer needed
                    panel = Vec::new();
                }
                let pointing = rot == Rotation::None && inputs.has_pointer();
                if pointing {
                    if cursor_dim != (fw, fh) {
                        cursor_damage = springboard::gfx::Damage::new(fw, fh);
                        cursor_dim = (fw, fh);
                    }
                    cursor_damage.clear();
                    cursor_damage.union(damage);
                    cursor.plan(want, &mut cursor_damage);
                } else {
                    cursor.forget();
                }
                let damage: &springboard::gfx::Damage = if pointing { &cursor_damage } else { damage };
                match rot {
                    Rotation::None => match night_mult {
                        Some(m) => {
                            if tinted.len() != frame.len() {
                                tinted = vec![0u32; frame.len()];
                            }
                            springboard::shell::night_shift::tint(&frame, &mut tinted, fw, damage, m);
                            cursor.stamp(&mut tinted, fw, fh);
                            disp.present(&tinted, damage);
                            cursor.restore(&mut tinted, fw, fh);
                        }
                        None => {
                            // Night Shift off: give its 9 MB copy back
                            if !tinted.is_empty() {
                                tinted = Vec::new();
                            }
                            cursor.stamp(&mut frame, fw, fh);
                            disp.present(&frame, damage);
                            cursor.restore(&mut frame, fw, fh);
                        }
                    },
                    rot => {
                        if panel.len() != w * h {
                            panel = vec![0u32; w * h];
                        }
                        rotate_areas(&frame, fw, fh, &mut panel, rot, damage.areas(), threads());
                        let dmg = rotate_damage(damage, rot, fw, fh);
                        if let Some(m) = night_mult {
                            springboard::shell::night_shift::tint_in_place(&mut panel, w, &dmg, m);
                        }
                        disp.present(&panel, &dmg);
                    }
                }
                let t2 = sys::now_ms();
                let pct = damage.pixels() as f64 / (fw * fh) as f64;
                perf.add(t1 - t0, t2 - t1, pct);
                #[cfg(feature = "desktop-wayland")]
                disp.wayland_note_draw(t1 - t0);
                slow_frame(&mut jank, &jctx, if rot == Rotation::None { "cpu" } else { "rotate" }, Frame { total: t2 - t0, draw: t1 - t0, present: t2 - t1, damage_pct: 100.0 * pct, ..Frame::default() }, t2);
                shell.perf_frame(t1 - t0);
                if trace {
                    // per-frame timing for animation work: draw, present, damage, and the period since the previous frame started
                    eprintln!("springboard: frame draw {:.1} present {:.1} damage {:.0}% gap {:.1} why {reason}", t1 - t0, t2 - t1, 100.0 * pct, t0 - last_frame_start);
                }
                last_frame_start = t0;
                last_present = t1;
                frame_cost.note(t2 - t0);
                cpu_shown = rot == Rotation::None;
            }
            // fail-safe: a refused flip puts one full CPU frame on the panel, then frees the ring
            if scan_failed {
                cursor.forget();
                scan.kill();
                shell.force_full_redraw();
                let damage = shell.draw(&mut Surface { w: fw, h: fh, stride: fw, px: &mut frame, y0: 0 }, vt);
                match rot {
                    Rotation::None => disp.present(&frame, damage),
                    rot => {
                        if panel.len() != w * h {
                            panel = vec![0u32; w * h];
                        }
                        rotate_areas(&frame, fw, fh, &mut panel, rot, damage.areas(), threads());
                        let mut dmg = springboard::gfx::Damage::new(w, h);
                        dmg.all();
                        disp.present(&panel, &dmg);
                    }
                }
                disp.wait_flip();
                disp.wait_flip();
                let bufs = gpu_comp.disable_scanout();
                let n = bufs.len();
                disp.scanout_free(bufs);
                eprintln!("springboard: zero-copy scanout flip failed: back on the copy path ({n} buffers freed)");
            }
        } else if cpu_shown && !shell.needs_frame() && night_mult.is_none() && inputs.has_pointer() && shell.orientation().rotation() == Rotation::None && frame.len() == fw * fh {
            // the pointer moved (or its cursor timed out) and the shell has nothing to draw: present
            // just the cursor's old and new rectangles over the frame the panel already shows
            let want = springboard::gfx::cursor::Spot::want(inputs.pointer(), inputs.is_down(), true, shell.screen_on(), t, last_motion);
            if cursor_dim != (fw, fh) {
                cursor_damage = springboard::gfx::Damage::new(fw, fh);
                cursor_dim = (fw, fh);
            }
            cursor_damage.clear();
            if cursor.plan(want, &mut cursor_damage) {
                cursor.stamp(&mut frame, fw, fh);
                disp.present(&frame, &cursor_damage);
                cursor.restore(&mut frame, fw, fh);
            }
        }
        #[cfg(feature = "desktop-wayland")]
        if wayland && last_present != previous_present {
            if shell.take_raise_front() { disp.wayland_raise_front(); }
            let pt = shell.points_px();
            if pt != presenter_scale { presenter_scale = pt; disp.wayland_set_scale(pt); }
            disp.wayland_window_materials(shell.take_wayland_materials());
            disp.wayland_windows(shell.take_wayland_windows())?;
            disp.wayland_chrome(shell.desktop_chrome_frames(&frame,fw,fh))?;
        }
        if let Some((wakeups, frames)) = idle_stats.note(sys::now_ms(), last_present != present_at_start) {
            bifrost_core::log::shell_logger("presenter").debug(format!("idle: {wakeups} wakeups, {frames} frames"));
        }
        // `touch $dir/snap` asks for the current frame as $dir/snap.raw
        if fs::remove_file(format!("{dir}/snap")).is_ok() {
            let mut out = b"RIMG".to_vec();
            out.extend((fw as u32).to_le_bytes());
            out.extend((fh as u32).to_le_bytes());
            out.extend(frame.iter().flat_map(|p| p.to_le_bytes()));
            let _ = fs::write(format!("{dir}/snap.raw"), out);
        }
    }
}

/// Backlight value for the user's `level` after Low Power Mode's cap.
#[cfg(target_os = "linux")]
/// Page-flips the GPU frame just drawn (nothing to flip when it changed nothing). False when
/// the flip was refused: the caller falls back to the copy path.
fn scan_flip(gpu: &mut springboard::bare::gpu_comp::GpuComp, disp: &mut springboard::bare::kms::Display, scan: &mut springboard::bare::scanout::FrameSwitch) -> bool {
    match gpu.take_flip() {
        Some(fb) => {
            if disp.flip_scanout(fb) {
                scan.gpu_frame();
                true
            } else {
                false
            }
        }
        None => true,
    }
}

fn panel_level(level: f32, low_power: bool, max: f32) -> i64 {
    (springboard::shell::low_battery::cap_brightness(level, low_power) * max).round().max(1.0) as i64
}

/// Connected or off: nothing the status bar is waiting to show.
#[cfg(target_os = "linux")]
fn wifi_settled(wifi: &springboard::net::wifi::Wifi) -> bool {
    use springboard::net::wifi::State;
    matches!(wifi.status.lock().map(|s| s.state), Ok(State::Connected | State::Off))
}

/// Sets the gain for `v`: on the mixer when it is there (software gain then only mutes), in the
/// player's software gain otherwise. Never an amplifier or a route; see media::volume.
#[cfg(target_os = "linux")]
fn apply_volume(v: springboard::media::volume::Volume, mixer: Option<&springboard::media::mixer::Mixer>) {
    use springboard::media::volume::software_gain;
    let pct = v.level() * 100.0;
    match v.apply(mixer) {
        Ok(Some(dg)) => eprintln!("springboard: volume {pct:.0}% -> mixer {dg}"),
        Ok(None) => eprintln!("springboard: volume {pct:.0}% (no mixer; software gain {:.3})", software_gain()),
        Err(e) => eprintln!("springboard: volume {pct:.0}%: mixer: {e} (software gain {:.3})", software_gain()),
    }
}

/// Arms the RTC to wake the device from suspend at Unix time `at` (0
/// disarms) so an alarm fires even while asleep. The kernel refuses a new
/// alarm while one is set, so the old one is cleared first.
#[cfg(target_os = "linux")]
fn rtc_wake(at: i64) {
    const WAKEALARM: &str = "/sys/class/rtc/rtc0/wakealarm";
    let _ = std::fs::write(WAKEALARM, "0");
    if at > 0 {
        if let Err(e) = std::fs::write(WAKEALARM, at.to_string()) {
            eprintln!("springboard: rtc wakealarm: {e}");
        }
    }
}

/// Runs only the Wi-Fi stack and logs every status change.
#[cfg(target_os = "linux")]
fn wifi_test(dir: &str) -> std::io::Result<()> {
    let wifi = springboard::net::wifi::Wifi::start(dir);
    let mut last = None;
    let start = std::time::Instant::now();
    loop {
        let s = wifi.snapshot();
        let line = format!("{:?} ssid={:?} ip={:?} err={:?}", s.state, s.ssid, s.ip, s.error);
        if last.as_ref() != Some(&line) {
            eprintln!("[{:5.1}s] {line}", start.elapsed().as_secs_f32());
            last = Some(line);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

use springboard::bare::jank::{Frame, Jank};

/// Logs a slow frame (over 16.7 ms) and counts every frame, trace or not.
fn slow_frame(jank: &mut Jank, ctx: &(String, &'static str), path: &str, f: Frame, now: f64) {
    for l in jank.frame(&ctx.0, ctx.1, path, f, now) {
        eprintln!("{l}");
    }
}

/// Frame timing, logged every 120 frames.
#[derive(Default)]
struct Perf {
    frames: u32,
    draw: f64,
    present: f64,
    /// fraction of the frame redrawn, summed
    damage: f64,
    /// averages from the last completed window (draw, present)
    last: (f64, f64),
}

impl Perf {
    fn add(&mut self, draw: f64, present: f64, damage: f64) {
        self.frames += 1;
        self.draw += draw;
        self.present += present;
        self.damage += damage;
        if self.frames == 120 {
            let n = self.frames as f64;
            self.last = (self.draw / n, self.present / n);
            eprintln!("springboard: frame draw {:.1} ms, present {:.1} ms, damage {:.0}%", self.last.0, self.last.1, 100.0 * self.damage / n);
            *self = Perf { last: self.last, ..Perf::default() };
        }
    }
}

/// Panel brightness until the user first moves the slider.
const DEFAULT_BRIGHTNESS: f32 = 0.8;
/// A brightness change is written to settings.ini once it has been still this long.
const BRIGHTNESS_SAVE_MS: f64 = 800.0;

/// Exit status that asks the service manager to start the shell again.
const RESTART_EXIT: i32 = 3;

/// Maps `wallpapers/<id>.raw`, or the default `wallpaper.raw` for "";
/// None for a missing file.
#[cfg(target_os = "linux")]
fn map_wallpaper(dir: &str, id: &str) -> Option<&'static [u8]> {
    if id.is_empty() {
        return map_file(&format!("{dir}/wallpaper.raw")).ok();
    }
    (!id.contains('/')).then(|| map_file(&format!("{dir}/wallpapers/{id}.raw")).ok()).flatten()
}

/// Minutes since boot, from /proc/uptime.
#[cfg(target_os = "linux")]
fn uptime_minutes() -> i64 {
    std::fs::read_to_string("/proc/uptime").ok().and_then(|u| u.split_whitespace().next()?.parse::<f64>().ok()).map_or(0, |s| (s / 60.0) as i64)
}

/// Device facts that never change while the shell runs.
#[cfg(target_os = "linux")]
struct About {
    name: String,
    cpu: String,
    serial: String,
    /// songs, videos, photos
    media: [u32; 3],
}

#[cfg(target_os = "linux")]
impl About {
    fn read(dir: &str) -> Self {
        let first = |path: &str| std::fs::read_to_string(path).map(|s| s.trim_end_matches('\0').trim().to_string()).unwrap_or_default();
        let hostname = first("/proc/sys/kernel/hostname");
        let cores = std::fs::read_to_string("/proc/cpuinfo").map(|c| c.lines().filter(|l| l.starts_with("processor")).count()).unwrap_or(0);
        let soc = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|c| c.lines().find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()))).unwrap_or_default();
        let serial = [first("/sys/class/dmi/id/product_serial"), first("/sys/firmware/devicetree/base/serial-number")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        About {
            name: if hostname.is_empty() || hostname == "localhost" { "BifrOSt PC".into() } else { hostname },
            cpu: match (soc.is_empty(), cores) {
                (true, 0) => String::new(),
                (true, n) => format!("{n} cores"),
                (false, 0) => soc,
                (false, n) => format!("{soc}, {n} cores"),
            },
            serial,
            media: [count_files(&format!("{dir}/Music")), count_files(&format!("{dir}/Videos")), count_files(&format!("{dir}/Pictures"))],
        }
    }
}

/// Regular files under `dir`, recursively (0 when missing).
#[cfg(target_os = "linux")]
fn count_files(dir: &str) -> u32 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten().map(|e| match e.file_type() {
        Ok(t) if t.is_dir() => count_files(&e.path().to_string_lossy()),
        Ok(t) if t.is_file() => 1,
        _ => 0,
    }).sum()
}

/// Gives the lock screen its own wallpaper: "" follows the home screen,
/// "default" is wallpaper.raw, anything else an id as in `map_wallpaper`.
#[cfg(target_os = "linux")]
fn set_lock_wallpaper(shell: &mut springboard::shell::Shell, dir: &str, id: &str, held: &mut Option<&'static [u8]>) {
    let next = match id {
        "" => None,
        "default" => map_wallpaper(dir, ""),
        id => map_wallpaper(dir, id),
    };
    match next.and_then(|m| springboard::gfx::View::from_raw(m).map(|v| (m, v))) {
        Some((m, view)) => {
            shell.set_lock_wallpaper(Some(view));
            if let Some(old) = held.replace(m) {
                unmap(old);
            }
        }
        None => {
            shell.set_lock_wallpaper(None);
            if let Some(old) = held.take() {
                unmap(old);
            }
        }
    }
}

/// Releases a mapping from `map_file` once nothing refers to it any more.
#[cfg(target_os = "linux")]
fn unmap(m: &'static [u8]) {
    // SAFETY: `m` came from map_file and the shell dropped its View of it in set_wallpaper.
    unsafe { springboard::bare::sys::munmap(m.as_ptr() as *mut _, m.len()) };
}

/// The packed stills: `wallpapers/index.ini` (id=name) with their thumbnails.
#[cfg(target_os = "linux")]
fn wallpapers(dir: &str) -> Vec<springboard::shell::wallpaper_page::Wallpaper> {
    use springboard::gfx::Image;
    use std::fs;
    let index = springboard::shell::prefs::Prefs::load(format!("{dir}/wallpapers/index.ini"));
    index
        .entries()
        .filter_map(|(id, name)| {
            let thumb = Image::from_raw(&fs::read(format!("{dir}/wallpapers/{id}.thumb.raw")).ok()?)?;
            Some(springboard::shell::wallpaper_page::Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
        })
        .chain(photo_wallpapers(dir))
        .collect()
}

/// Up to six pictures from `Pictures/`, each cover-cropped to the screen's
/// wallpaper size once and cached as `wallpapers/photo_<stem>.raw`.
#[cfg(target_os = "linux")]
fn photo_wallpapers(dir: &str) -> Vec<springboard::shell::wallpaper_page::Wallpaper> {
    use springboard::gfx::Image;
    use std::fs;
    let Some(base) = fs::read(format!("{dir}/wallpaper.raw")).ok().and_then(|b| Image::from_raw(&b)) else { return vec![] };
    let (tw, th) = (base.w, base.h);
    let mut files: Vec<_> = fs::read_dir(format!("{dir}/Pictures")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| springboard::files::fs::kind_of(n, false) == springboard::files::fs::Kind::Image)).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let stem: String = path.file_stem()?.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            let id = format!("photo_{stem}");
            let (raw, thumb_path) = (format!("{dir}/wallpapers/{id}.raw"), format!("{dir}/wallpapers/{id}.thumb.raw"));
            if !std::path::Path::new(&raw).exists() || !std::path::Path::new(&thumb_path).exists() {
                let img = springboard::imgdec::decode(&fs::read(&path).ok()?)?;
                let scale = (tw as f32 / img.w as f32).max(th as f32 / img.h as f32);
                let (cw, ch) = (((tw as f32 / scale) as usize).clamp(1, img.w), ((th as f32 / scale) as usize).clamp(1, img.h));
                let full = img.crop((img.w - cw) / 2, (img.h - ch) / 2, cw, ch).resized(tw, th);
                fs::create_dir_all(format!("{dir}/wallpapers")).ok()?;
                let rimg = |i: &Image| {
                    let mut b = b"RIMG".to_vec();
                    b.extend((i.w as u32).to_le_bytes());
                    b.extend((i.h as u32).to_le_bytes());
                    b.extend(i.px.iter().flat_map(|p| p.to_le_bytes()));
                    b
                };
                fs::write(&raw, rimg(&full)).ok()?;
                fs::write(&thumb_path, rimg(&springboard::shell::wallpaper_page::cover_fit(&full, 320, 180))).ok()?;
            }
            let thumb = Image::from_raw(&fs::read(&thumb_path).ok()?)?;
            Some(springboard::shell::wallpaper_page::Wallpaper { id, name: path.file_stem()?.to_string_lossy().into_owned(), thumb })
        })
        .take(6)
        .collect()
}

#[cfg(target_os = "linux")]
fn map_file(path: &str) -> std::io::Result<&'static [u8]> {
    springboard::bare::sys::map_file(path)
}

#[cfg(target_os = "linux")]
fn model() -> String {
    // a PC names itself in DMI; a devicetree board in its model node
    ["/sys/class/dmi/id/product_name", "/sys/firmware/devicetree/base/model"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim_end_matches('\0').trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "PC".into())
}

#[cfg(target_os = "linux")]
fn memory() -> String {
    let kb: f64 = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| m.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()))
        .unwrap_or(0.0);
    format!("{:.1} GB", kb / 1024.0 / 1024.0)
}

/// True once the desktop has committed its first frame (`mark_first_frame` in `bare/wayland.rs` writes the marker).
fn ready_marker() -> bool {
    std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| std::path::Path::new(&d).join("shell-ready").exists())
}
