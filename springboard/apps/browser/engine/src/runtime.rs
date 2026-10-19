//! Engine process startup, event-loop wakeups and shutdown.
use super::*;
pub(super) fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--content-process") {
        return servo::run_content_process(args.get(2).cloned().unwrap_or_default());
    }
    let event_loop = event_loop::EventLoop::new();
    let size = PhysicalSize::new(800, 600);
    let mut graphics = match graphics::Graphics::new(size) {
        Ok(graphics) => graphics,
        Err(error) => die(&format!("{error}; GPU is the default, CPU fallback is available in Browser Settings")),
    };
    let rc = graphics.context();
    let opts = configuration::options_from_env();
    let prefs = configuration::preferences_from_env();
    let servo = ServoBuilder::default().opts(opts).preferences(prefs).event_loop_waker(event_loop.waker()).build();
    servo.setup_logging();
    let mut e = Engine {
        servo,
        rc,
        renderer_mode: graphics.mode(),
        renderer_driver: graphics.driver().to_string(),
        delegate: Rc::new(Delegate::default()),
        views: HashMap::new(),
        visible: None,
        size,
        scale: 1.0,
        theme: Theme::Light,
        shm: Shm { fd: std::env::var("BIFROST_WEBVIEW_FD").ok().and_then(|v| v.parse().ok()).unwrap_or(3), ptr: std::ptr::null_mut(), len: 0 },
        awaiting_ack: false,
        dirty: false,
        pending_size: None,
        next_id: 0,
        shm_warned: false,
        devtools: true,
        suspended: false,
        controls: HashMap::new(),
        next_token: 1,
        unloads: HashMap::new(),
    };
    event_loop.run(&mut e, &mut graphics);
}
