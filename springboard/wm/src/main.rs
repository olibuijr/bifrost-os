//! `bifrost-wm [--drm | --nested | --headless WxH [--screenshot PATH --after MS]] [--socket NAME] [-- COMMAND ARGS...]`

use std::{error::Error, time::Duration};

use bifrost_wm::spawn;

use bifrost_wm::{headless::Compositor, state::WmState};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let command: Vec<String> = args.iter().position(|a| a == "--").map(|i| args[i + 1..].to_vec()).unwrap_or_default();
    let socket = flag("--socket");

    #[cfg(feature = "drm")]
    if args.iter().any(|a| a == "--drm") {
        return bifrost_wm::drm::run(socket.as_deref(), &command);
    }

    if let Some(size) = flag("--headless") {
        let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).ok_or("--headless WxH")?;
        let wm = Compositor::start(socket.as_deref().unwrap_or("wayland-bifrost"), w, h)?;
        eprintln!("bifrost-wm: headless {w}x{h} on {}", wm.socket);
        spawn(&command, &wm.socket);
        let after = flag("--after").and_then(|v| v.parse().ok()).unwrap_or(1500);
        std::thread::sleep(Duration::from_millis(after));
        if let Some(path) = flag("--screenshot") {
            let path = std::path::PathBuf::from(path);
            wm.run(move |_, b| b.screenshot(&path).map_err(|e| e.to_string()))?;
        } else {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        return Ok(());
    }

    #[cfg(feature = "nested")]
    {
        let mut event_loop: EventLoop<'static, WmState> = EventLoop::try_new()?;
        let display: Display<WmState> = Display::new()?;
        let mut state = WmState::new(&mut event_loop, display, socket.as_deref());
        bifrost_wm::nested::init(&mut event_loop, &mut state)?;
        let name = state.socket_name.to_string_lossy().into_owned();
        eprintln!("bifrost-wm: nested on {name}");
        spawn(&command, &name);
        event_loop.run(None, &mut state, |_| {})?;
        Ok(())
    }
    #[cfg(not(feature = "nested"))]
    Err("built without a nested backend; use --headless WxH".into())
}

