//! The BifrOSt installer: a native Rust program on the DRM/KMS display (no X, no browser). Run as root
//! on the live system (see packaging/archiso: bifrost-installer.service).

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = run() {
        eprintln!("bifrost-installer: {e}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
extern "C" {
    fn geteuid() -> u32;
}

#[cfg(target_os = "linux")]
fn run() -> std::io::Result<()> {
    use springboard::bare::{input, kms, sys};
    use springboard::font::Font;
    use springboard::gfx::{Damage, Surface};
    use springboard::installer::{app::App, disks, plan};
    use springboard::shell::ui::Ui;

    use springboard::installer::unattended;
    // at once, like the on-screen Restart: `--force` still killed processes and unmounted the live root, which released
    // the lab address under iVentoy's iSCSI root (I/O errors on a black screen)
    let reboot_now = || {
        let _ = std::process::Command::new("sync").status();
        let _ = std::process::Command::new("systemctl").args(["reboot", "-ff"]).status();
    };
    // Unattended lab reinstall: a preset on the target's ESP (written by `midget.py reinstall` or the MCP `lifecycle
    // reinstall`), docs/installer.md. It runs on the progress page below (headless only when no display opens).
    // BIFROST_NO_UNATTENDED=1 skips the search. A refused preset falls through to the on-screen installer.
    let find_preset = || {
        let mut preset = None;
        if std::env::var_os("BIFROST_NO_UNATTENDED").is_none() && std::env::var_os("BIFROST_AUTO").is_none() {
            if let Some((text, disk)) = unattended::find_stash() {
                eprintln!("bifrost-installer: unattended preset found on {disk}");
                match unattended::prepare(&text, &disk) {
                    Ok(cfg) => preset = Some((text, disk, cfg)),
                    Err(e) => eprintln!("bifrost-installer: unattended install refused or failed: {e}"),
                }
            }
        }
        preset
    };
    // BIFROST_AUTO=user:password:/dev/vda runs the whole install without a display (VM and CI tests)
    if let Ok(spec) = std::env::var("BIFROST_AUTO") {
        let mut it = spec.splitn(3, ':');
        let (user, pass, dev) = (it.next().unwrap_or("test"), it.next().unwrap_or("test"), it.next().unwrap_or(""));
        let disk = disks::probe().into_iter().find(|d| d.path == dev).ok_or_else(|| std::io::Error::other(format!("no disk {dev}")))?;
        // BIFROST_LAYOUT=<id> picks the keyboard the way the on-screen installer does; the default is the first layout
        let layout = std::env::var("BIFROST_LAYOUT").ok()
            .and_then(|id| springboard::installer::keymaps::layout_index(&id))
            .and_then(|i| springboard::installer::keymaps::layouts().nth(i))
            .unwrap_or_else(|| springboard::installer::keymaps::layouts().next().unwrap());
        let cfg = plan::Config {
            disk, mode: springboard::installer::disks::Mode::Erase, locale: "en_US.UTF-8".into(), layout, full_name: user.into(),
            username: user.into(), password: pass.into(), hostname: "bifrost".into(), timezone: String::new(),
            mirror: std::env::var("BIFROST_MIRROR").unwrap_or_default(),
            appearance: plan::DEFAULT_APPEARANCE.into(), accent: plan::DEFAULT_ACCENT.into(), wallpaper: String::new(),
        };
        let progress: plan::Shared = Default::default();
        plan::install(&cfg, &progress);
        let p = progress.lock().unwrap();
        return match &p.failed {
            Some(e) => Err(std::io::Error::other(e.clone())),
            None => {
                eprintln!("bifrost-installer: installed");
                Ok(())
            }
        };
    }

    // the live medium keeps its assets in the legacy directory; BIFROST_ASSETS still overrides (bifrost_core::system_assets)
    let assets = bifrost_core::system_assets::root_string();
    let branding = std::env::var("BIFROST_BRANDING").unwrap_or_else(|_| "/usr/share/bifrost/installer".into());
    let font = Font::parse(sys::map_file(&format!("{assets}/SFUI.ttf"))?).ok_or_else(|| std::io::Error::other("bad font"))?;
    let wayland = std::env::var_os("SPRINGBOARD_WAYLAND").is_some();
    #[cfg(feature = "desktop-wayland")]
    let opened = if wayland { kms::Display::wayland() } else { kms::Display::open_best(8000) };
    #[cfg(not(feature = "desktop-wayland"))]
    let opened = kms::Display::open_best(8000);
    // the preset search mounts disks (seconds): it runs after the boot frame is on screen, never before the display opens
    let mut disp = match opened {
        Ok(d) => d,
        Err(e) => {
            // no display: the preset still installs, headless
            let preset = find_preset();
            let Some((text, disk, _)) = &preset else { return Err(e) };
            eprintln!("bifrost-installer: no display ({e}); unattended install without one");
            return match unattended::run(text, disk) {
                Ok(()) => {
                    eprintln!("bifrost-installer: unattended install done; rebooting into the installed system");
                    reboot_now();
                    Ok(())
                }
                Err(e) => Err(std::io::Error::other(format!("unattended install refused or failed: {e}"))),
            };
        }
    };
    let (w, h) = disp.size();
    // the first KMS frame is the loader's boot frame, so taking the display over from simpledrm changes nothing visible
    use springboard::bootsplash;
    bootsplash::mark("installer-display");
    let mut splash = vec![0u32; w * h];
    bootweave_frame::render(&bootweave_frame::Frame::boot(w as u32, h as u32), &mut splash, w, bootweave_frame::PixelOrder::Bgrx);
    {
        let mut dmg = Damage::new(w, h);
        dmg.all();
        disp.present(&splash, &dmg);
    }
    let mut fade_from: Option<f64> = None;
    let preset = find_preset();
    let ui = Ui::new(font, (h as f32 / 720.0).clamp(1.0, 4.0));
    let mut inputs = if wayland { input::InputSet::none() } else { input::InputSet::desktop(w as f32, h as f32, unsafe { geteuid() } == 0) };
    let image = |name: &str| std::fs::read(format!("{branding}/{name}")).ok().and_then(|b| springboard::cg::imageio::decode(&b));
    let mut app = App::new(ui, w, h, disks::probe(), image("logo.png"), image("bg.jpg").or_else(|| image("bg.png")));
    app.set_wallpapers(springboard::installer::app::load_wallpapers(&assets));
    if let Ok(id) = std::env::var("BIFROST_LAYOUT") {
        if let Some(i) = springboard::installer::keymaps::layout_index(&id) {
            app.set_layout(i);
        }
    }
    let unattended_run = preset.is_some();
    if let Some((_, _, cfg)) = preset {
        app.begin(cfg);
    }
    let (mut reported, mut done_at) = (false, None::<f64>);

    // lab hooks (remote testing): `touch /run/bifrost-shot-on` makes the installer write /run/bifrost-screen.png about twice a
    // second; lines written to /run/bifrost-input become input events (down/move/up X Y, key CODE 1|0)
    let injected: std::sync::Arc<std::sync::Mutex<Vec<input::Event>>> = Default::default();
    {
        let _ = std::process::Command::new("mkfifo").args(["-m", "600", "/run/bifrost-input"]).status();
        let q = injected.clone();
        std::thread::spawn(move || loop {
            if let Ok(f) = std::fs::File::open("/run/bifrost-input") {
                for line in std::io::BufRead::lines(std::io::BufReader::new(f)).map_while(Result::ok) {
                    let p: Vec<&str> = line.split_whitespace().collect();
                    let num = |i: usize| p.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                    let ev = match p.first().copied() {
                        Some("down") => input::Event::Down { x: num(1), y: num(2), t: 0.0 },
                        Some("move") => input::Event::Move { x: num(1), y: num(2), t: 0.0 },
                        Some("up") => input::Event::Up { x: num(1), y: num(2), t: 0.0 },
                        Some("key") => input::Event::Key { code: num(1) as u16, down: num(2) != 0.0 },
                        _ => continue,
                    };
                    q.lock().unwrap().push(ev);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
    }
    let stats = std::env::var_os("BIFROST_STATS").is_some();
    let (mut shown, mut stat_t) = (0u32, sys::now_ms());
    let mut last_shot = f64::MIN;
    let mut frame = vec![0u32; w * h];
    let mut overlay = vec![0u32; w * h];
    let mut last_ui = f64::MIN;
    let mut events = vec![];
    let mut ready = vec![];
    let mut dirty = true;
    let hw_cursor = disp.set_cursor(&springboard::installer::app::cursor_argb(), 1.0);
    let mut installing = false;
    loop {
        let now = sys::now_ms();
        inputs.rescan_if_due(now);
        let mut wait: f64 = if app.animating() { 100.0 } else { 1000.0 };
        let fading = fade_from.is_none_or(|f| now - f < bootweave_frame::FADE_MS as f64 + 50.0);
        if fading { wait = wait.min(8.0); }
        if let Some(at) = inputs.next_scan() {
            wait = wait.min((at - now).max(0.0));
        }
        let mut fds: Vec<std::ffi::c_int> = vec![disp.fd()];
        fds.extend(inputs.fds());
        kms::wait_precise(&fds, wait, &mut ready);
        if ready[0] { disp.handle_event(); }
        let t = sys::now_ms();
        events.clear();
        #[cfg(feature = "desktop-wayland")]
        disp.wayland_inputs(&mut events);
        inputs.read(&mut events);
        inputs.tick(t, &mut events);
        events.append(&mut injected.lock().unwrap());
        for e in events.drain(..) {
            app.event(e);
            dirty = true;
        }
        if inputs.has_pointer() {
            let pt = inputs.pointer();
            if pt != app.pointer {
                app.pointer = pt;
                if hw_cursor {
                    disp.move_cursor(pt.0 as i32, pt.1 as i32);
                } else {
                    dirty = true;
                }
            }
        }
        // hw_cursor: the display draws the pointer (KMS plane, or the compositor under Wayland), so the app's own arrow stays off
        app.hw_cursor = hw_cursor || wayland;
        if let Some(cfg) = app.want_install.take() {
            if !installing {
                installing = true;
                let progress = app.progress.clone();
                std::thread::spawn(move || {
                    let mut cfg = cfg;
                    plan::install(&cfg, &progress);
                    cfg.password.clear();
                });
            }
        }
        if app.page == springboard::installer::app::Page::Failed {
            installing = false;
        }
        if unattended_run && !reported {
            let (done, failed) = { let p = app.progress.lock().unwrap(); (p.done, p.failed.clone()) };
            if let Some(e) = failed {
                // the failure stays on screen (Back offers the on-screen installer); the lab follower reads this line
                eprintln!("bifrost-installer: unattended install refused or failed: install failed: {e}");
                reported = true;
            } else if done {
                eprintln!("bifrost-installer: unattended install done; rebooting into the installed system");
                unattended::finish();
                reported = true;
                // the done page for a moment instead of a cut from 98 % to the firmware
                done_at = Some(t);
                dirty = true;
            }
        }
        if done_at.is_some_and(|at| t - at >= 1500.0) {
            done_at = None;
            app.want_restart = true;
        }
        if app.want_restart {
            // Reboot at once without stopping services: a normal shutdown kills this process (the console appears) and can
            // hang tearing down a network-mounted live root. The installed disk is already unmounted and synced.
            let _ = std::process::Command::new("sync").status();
            let _ = std::process::Command::new("systemctl").args(["reboot", "-ff"]).status();
            app.want_restart = false;
        }
        let need_ui = dirty || (app.animating() && t - last_ui >= 100.0) || t - last_ui >= 1000.0;
        if need_ui {
            overlay.fill(0);
            let mut s = Surface { w, h, stride: w, px: &mut overlay, y0: 0 };
            app.draw_ui(&mut s, t);
            last_ui = t;
        }
        if need_ui || fading {
            compose(app.background(), &overlay, &mut frame);
            let from = *fade_from.get_or_insert_with(|| { bootsplash::mark("installer-first-ui"); t });
            if fading {
                let fade_k = (t - from) / bootweave_frame::FADE_MS as f64;
                bootweave_frame::fade(&mut frame, &splash, fade_k as f32);
                if t - from >= bootweave_frame::FADE_MS as f64 { bootsplash::mark("installer-faded"); }
            }
            let mut dmg = Damage::new(w, h);
            dmg.all();
            disp.present(&frame, &dmg);
            dirty = false;
            shown += 1;
            if stats && t - stat_t >= 2000.0 {
                eprintln!("bifrost-installer: {:.1} fps presented", shown as f64 * 1000.0 / (t - stat_t));
                (shown, stat_t) = (0, t);
            }
            if t - last_shot >= 500.0 && std::path::Path::new("/run/bifrost-shot-on").exists() {
                last_shot = t;
                let png = springboard::pngenc::encode_rgb(w, h, &frame);
                let _ = std::fs::write("/run/bifrost-screen.png.tmp", png).and_then(|_| std::fs::rename("/run/bifrost-screen.png.tmp", "/run/bifrost-screen.png"));
            }
        }
    }
}

/// The screens (premultiplied, transparent where nothing is drawn) over the background.
#[cfg(target_os = "linux")]
fn compose(bg: &[u32], ov: &[u32], out: &mut [u32]) {
    for ((o, &b), &f) in out.iter_mut().zip(bg).zip(ov) {
        let a = f >> 24;
        *o = match a {
            0 => b,
            255 => f,
            _ => {
                let ia = 255 - a;
                let ch = |sh: u32| (((f >> sh) & 255) + (((b >> sh) & 255) * ia) / 255).min(255) << sh;
                ch(0) | ch(8) | ch(16) | 0xff00_0000
            }
        };
    }
}
