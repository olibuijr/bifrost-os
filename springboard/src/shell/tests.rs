    use super::*;
    use crate::font::Font;

    pub fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
        ui.symbols = crate::symbols::Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
        ui
    }

    #[cfg(test)]
    pub(crate) fn shell() -> Shell {
        shell_sized(1200, 1920)
    }

    /// For a test of the shell placing windows itself (tiling): with the window server's variable set, the compositor owns
    /// window positions and a tile is only queued as a request (`compositor_places_windows`). The guard holds the
    /// crate-wide test lock, so no parallel test has the variable set meanwhile, and puts the variable back when dropped.
    #[cfg(feature = "desktop-wayland")]
    pub(crate) struct ShellPlacesWindows {
        saved: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    #[cfg(feature = "desktop-wayland")]
    impl ShellPlacesWindows {
        pub(crate) fn new() -> Self {
            let lock = crate::test_support::global_lock();
            let saved = std::env::var_os("SPRINGBOARD_WAYLAND");
            std::env::remove_var("SPRINGBOARD_WAYLAND");
            ShellPlacesWindows { saved, _lock: lock }
        }
    }

    #[cfg(feature = "desktop-wayland")]
    impl Drop for ShellPlacesWindows {
        fn drop(&mut self) {
            if let Some(v) = self.saved.take() {
                std::env::set_var("SPRINGBOARD_WAYLAND", v);
            }
        }
    }

    /// A shell in desktop mode (menu bar, dock, windows).
    #[allow(dead_code)]
    pub(crate) fn desktop_shell() -> Shell {
        build(1200, 1920, true)
    }

    #[allow(dead_code)]
    pub(crate) fn shell_sized(w: usize, h: usize) -> Shell {
        build(w, h, false)
    }

    fn build(w: usize, h: usize, desktop: bool) -> Shell {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let px: &'static [u32] = Box::leak(vec![rgba(20, 20, 30, 255); w * h].into_boxed_slice());
        let wp = View { w, h, px };
        let info = Info { hour: 9, min: 41, wday: 2, mday: 6, mon: 0, battery: 80, brightness: 0.5, ..Info::default() };
        let mono: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice());
        let mut sh = Shell::new(w as i32, h as i32, Font::parse(data).unwrap(), mono, wp, |_, _| None, info);
        // `Shell::new` starts in desktop mode; the default test shell is the bifrost presentation (no desk) that most
        // tests were written for. Tests of the desktop use `desktop_shell()`.
        if !desktop {
            sh.desk = None;
        }
        // tests do not depend on BIFROST_FORM or /etc/bifrost/form of the machine they run on. The touch presentation
        // (no desk) is the tablet form: it is the one that raises the on-screen keyboard. The desktop shell is a desktop.
        sh.form = if desktop { bifrost_ui::form_factor::FormFactor::Desktop } else { bifrost_ui::form_factor::FormFactor::Tablet };
        // the keyboard client starts out shown; the shell raises it when a field asks, so start every test from down
        sh.keyboard.set_shown(false, 0.0);
        sh
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_menu_bar_text_turns_dark_over_a_light_wallpaper() {
        let mut sh = desktop_shell();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let light: &'static [u32] = Box::leak(vec![rgba(240, 236, 228, 255); w * h].into_boxed_slice());
        sh.set_wallpaper(View { w, h, px: light });
        assert_eq!(sh.bar_ink().text, super::ui::BAR_TEXT_ON_LIGHT, "dark text on a light sky");
        let dark: &'static [u32] = Box::leak(vec![rgba(20, 24, 30, 255); w * h].into_boxed_slice());
        sh.set_wallpaper(View { w, h, px: dark });
        assert_eq!(sh.bar_ink().text, super::ui::BAR_TEXT, "white text on a dark wallpaper");
    }

    #[test]
    fn unlock_releases_the_lock_wallpaper_buffers() {
        let mut sh = desktop_shell();
        let px: &'static [u32] = Box::leak(vec![rgba(1, 2, 3, 255); 64 * 64].into_boxed_slice());
        sh.wpx.lock = Some(View { w: 64, h: 64, px });
        sh.wpx.dim = true;
        sh.screen = Screen::Lock;
        sh.lazy_wallpapers();
        assert!(sh.fit_lock.is_some() || sh.wpx.lock_dim.is_some(), "lock buffers are built while locked");
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.pager.jump_to(0);
        sh.lazy_wallpapers();
        assert!(sh.fit_lock.is_none() && sh.wpx.lock_dim.is_none() && sh.wpx.home_dim.is_none());
        assert!(sh.take_heap_trim_due(), "unlock asks the loop for an allocator trim");
        assert!(!sh.take_heap_trim_due());
    }

    #[test]
    fn unlocking_the_desktop_gives_the_wallpaper_to_the_presenter_again() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.note_wallpaper_uploaded();
        sh.tick(1000.);
        assert!(sh.desktop_wallpaper_once().is_none(), "the presenter holds the wallpaper before the lock");
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Ok("tok".into()))));
        let fx = sh.lock_screen();
        assert_eq!(fx, vec![Effect::ScreenOn], "a desktop lock keeps the display on");
        sh.tick(1100.);
        assert!(sh.is_locked() && sh.login_open(), "the login window is up");
        // the password goes through the login window; the unlock slide then runs to its end
        sh.hw_type(keyboard::Out::Text("pw".into()), 1200.);
        sh.hw_type(keyboard::Out::Enter, 1210.);
        let mut t = 1220.;
        for _ in 0..400 {
            t += 20.;
            sh.tick(t);
            if !sh.is_locked() { break; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!sh.is_locked(), "the right password unlocked the desktop");
        sh.tick(t + 60_000.);
        assert_eq!(sh.screen, Screen::Home, "the unlock finished");
        let (wp, x) = sh.desktop_wallpaper_once().expect("the unlocked desktop owes its wallpaper to the presenter");
        assert_eq!((wp.w, wp.h, x), (sh.w as usize, sh.h as usize, 0), "a full fitted still");
        // the desktop background is the wallpaper, not the empty fill
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut img = Image::new(w, h);
        sh.draw_desktop_background(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 });
        assert_eq!(img.px[(h / 2) * w + w / 2], rgba(20, 20, 30, 255));
        // once uploaded it is not sent again, frame after frame
        sh.note_wallpaper_uploaded();
        for k in 0..5 {
            sh.tick(t + 70_000. + k as f64 * 16.);
            assert!(sh.desktop_wallpaper_once().is_none(), "frame {k}");
        }
    }

    #[test]
    fn the_desktop_never_raises_the_on_screen_keyboard() {
        // Grey boxes along the bottom of app windows (0.4.58-0.4.62) were this keyboard's top row: bringing an app
        // with a focused text field to the front raised it on the desktop.
        let mut sh=shell();
        sh.set_keyboard(true,1000.);
        assert!(sh.keyboard.shown(),"the touch presentation (a tablet) still gets its keyboard");
        sh.set_keyboard(false,1100.);
        sh.form=bifrost_ui::form_factor::FormFactor::Desktop;
        sh.desk=Some(desk::Desk::new());
        sh.set_keyboard(true,1200.);
        assert!(!sh.keyboard.shown(),"a desktop has a hardware keyboard");
    }

    #[test]
    fn a_desktop_never_raises_the_keyboard_without_the_desktop_ui_either() {
        // the old touch-only presentation on a desktop-form machine used to raise it; the form alone decides now
        let mut sh=shell();
        sh.form=bifrost_ui::form_factor::FormFactor::Desktop;
        assert!(sh.desk.is_none());
        sh.set_keyboard(true,1000.);
        assert!(!sh.keyboard.shown());
        assert!(!sh.desk_keyboard_visible());
    }

    #[test]
    fn on_a_desktop_no_automatic_path_shows_the_keyboard() {
        keyboard::fake_process(500.0);
        // a Wayland client's text field takes focus
        let mut sh=desktop_shell();
        sh.input_method_batch(textinput::ImBatch{focus:Some(true),..textinput::ImBatch::default()},1000.);
        assert!(!sh.keyboard.shown(),"a focused text field of a client");
        assert!(!sh.desk_keyboard_visible());
        // Terminal, Search, sheets and the rest all raise it through `set_keyboard`
        sh.set_keyboard(true,1100.);
        assert!(!sh.keyboard.shown(),"Terminal, Search or a sheet");
        // the lock screen's password field, even when the window asks for a touch keyboard
        keyboard::fake_process(500.0);
        let mut sh=login_shell(login::Reply::Unavailable);
        sh.form=bifrost_ui::form_factor::FormFactor::Desktop;
        sh.set_keyboard(false,0.);
        sh.anim=None;
        sh.login_ensure(0.0);
        sh.login.as_mut().unwrap().touch_keyboard=true;
        let picture=(600.0,1920.0*0.62+sh.ui.pt(30.0));
        tap(&mut sh,picture.0,picture.1,100.0);
        assert!(!sh.keyboard.shown(),"the lock screen");
        assert!(!sh.login_owns_keyboard());
    }

    #[test]
    fn a_desktop_gets_the_keyboard_only_when_the_user_opens_it_and_can_close_it() {
        keyboard::fake_process(500.0);
        let mut sh=desktop_shell();
        unlock(&mut sh);
        sh.keyboard_open_explicit(1000.);
        assert!(sh.keyboard.shown() && sh.keyboard_opened_explicitly(),"the Input menu row opens it");
        assert!(sh.desk_keyboard_visible(),"and the desktop draws it");
        sh.set_keyboard(false,1100.);
        assert!(sh.keyboard.shown(),"tapping outside a field does not close the one the user opened");
        sh.keyboard_close_explicit(1200.);
        assert!(!sh.keyboard.shown() && !sh.keyboard_opened_explicitly(),"the same row closes it");
        assert!(!sh.desk_keyboard_visible(),"nothing is drawn or reserved afterwards");
        sh.set_keyboard(true,1300.);
        assert!(!sh.keyboard.shown(),"and it does not come back by itself");
    }

    #[test]
    fn the_input_menu_rule_is_several_sources_an_open_keyboard_or_the_setting() {
        assert!(!desktop_bar::input_menu_visible(1, false, false), "one source and nothing else: no menu");
        assert!(desktop_bar::input_menu_visible(2, false, false), "a choice of sources");
        assert!(desktop_bar::input_menu_visible(1, true, false), "the keyboard is open, so its row must be reachable");
        assert!(desktop_bar::input_menu_visible(1, false, true), "the user turned the menu on");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn show_input_menu_in_menu_bar_makes_the_menu_and_its_keyboard_rows_reachable_with_one_source() {
        use desktop_bar::Menu;
        keyboard::fake_process(500.0);
        let mut sh = desktop_shell();
        // the enabled sources are process-wide and other tests change them: the menu follows them while the switch is off
        let several = super::kb_layouts::enabled().len() > 1;
        assert_eq!(sh.menu_extra_shown(Menu::Input), several, "off by default: only a choice of sources shows it");
        sh.prefs.set("show_input_menu", "1").unwrap();
        assert!(sh.menu_extra_shown(Menu::Input), "the switch shows it with any number of sources");
        let rows = sh.menu_row_texts(Menu::Input);
        assert!(rows.iter().any(|r| r == "Show Keyboard Viewer"), "{rows:?}");
        assert!(rows.iter().any(|r| r == "Show On-Screen Keyboard"), "{rows:?}");
        sh.keyboard_open_explicit(1000.);
        let open = sh.menu_row_texts(Menu::Input);
        assert!(open.iter().any(|r| r == "Hide On-Screen Keyboard"), "the same row closes it: {open:?}");
        sh.prefs.set("show_input_menu", "0").unwrap();
        assert!(sh.menu_extra_shown(Menu::Input), "an open keyboard keeps the menu up so it can be closed");
        sh.keyboard_close_explicit(1100.);
        assert_eq!(sh.menu_extra_shown(Menu::Input), several, "switch off and keyboard closed: back to the sources rule");
    }

    #[test]
    fn the_keyboards_hide_key_closes_the_keyboard_a_desktop_user_opened() {
        keyboard::fake_process(500.0);
        let mut sh=desktop_shell();
        sh.keyboard_open_explicit(1000.);
        keyboard::fake_typed(keyboard::Out::Hide);
        sh.tick(1100.);
        assert!(!sh.keyboard.shown() && !sh.keyboard_opened_explicitly());
    }

    #[test]
    fn a_tablet_raises_the_on_screen_keyboard_even_with_the_desktop_ui_up() {
        let mut sh=shell();
        sh.form=bifrost_ui::form_factor::FormFactor::Tablet;
        sh.desk=Some(desk::Desk::new());
        sh.set_keyboard(true,1000.);
        assert!(sh.keyboard.shown(),"a tablet has no physical keys");
        sh.set_keyboard(false,1100.);
        assert!(!sh.keyboard.shown(),"tapping outside still dismisses it");
    }

    #[test]
    fn an_attached_hardware_keyboard_hides_the_tablet_keyboard_and_keeps_it_down() {
        let mut sh=shell();
        sh.form=bifrost_ui::form_factor::FormFactor::Tablet;
        sh.desk=Some(desk::Desk::new());
        sh.set_keyboard(true,1000.);
        assert!(sh.keyboard.shown());
        sh.set_hardware_keyboard(true,1100.);
        assert!(!sh.keyboard.shown(),"attaching a keyboard hides the on-screen one");
        sh.set_keyboard(true,1200.);
        assert!(!sh.keyboard.shown(),"and a text field cannot raise it while it is attached");
        sh.set_hardware_keyboard(false,1300.);
        sh.set_keyboard(true,1400.);
        assert!(sh.keyboard.shown(),"detaching it brings the on-screen keyboard back");
    }

    #[test]
    fn a_desktop_ignores_the_hardware_keyboard_state() {
        let mut sh=shell();
        sh.form=bifrost_ui::form_factor::FormFactor::Desktop;
        sh.desk=Some(desk::Desk::new());
        sh.set_hardware_keyboard(false,1000.);
        sh.set_keyboard(true,1100.);
        assert!(!sh.keyboard.shown(),"the desktop form never raises it");
        assert_eq!(sh.form(),bifrost_ui::form_factor::FormFactor::Desktop);
    }

    /// A tablet with the desktop up and one window (the Terminal) placed so that its bottom edge runs under the
    /// on-screen keyboard.
    #[cfg(test)]
    fn tablet_desk_with_window_under_keyboard() -> Shell {
        let mut sh = desktop_shell();
        sh.form = bifrost_ui::form_factor::FormFactor::Tablet;
        // the user unlocks before opening a window; a locked screen is never left by opening one
        unlock(&mut sh);
        sh.desk_open(AppId::Terminal, 0.);
        // `Desk::place` clamps windows into the visible work area; assign the edge case directly so the bottom really
        // runs under the keyboard and the lift/restore path has a meaningful original position.
        sh.desk.as_mut().unwrap().wins[0].r.y = 1200;
        sh
    }

    #[test]
    fn the_tablet_desktop_keyboard_follows_the_app_store_search_field() {
        // the keyboard policy is process-global and another test switches it off (keyboard::disabled_tests)
        let _lock = crate::test_support::global_lock();
        let mut sh = tablet_desk_with_window_under_keyboard();
        sh.desk_open(AppId::AppStore, 0.);
        assert_eq!(sh.desk_text_focus(), Some(false), "the Today tab has no field");
        sh.store.as_mut().unwrap().tab = super::store::Tab::Search;
        assert_eq!(sh.desk_text_focus(), Some(true), "the Search tab's query field takes the typing");
        sh.store.as_mut().unwrap().tab = super::store::Tab::Today;
        assert_eq!(sh.desk_text_focus(), Some(false));
    }

    #[test]
    fn a_tablet_keyboard_lifts_the_front_window_clear_of_it_and_lets_it_back_down() {
        // the keyboard policy is process-global and another test switches it off (keyboard::disabled_tests)
        let _lock = crate::test_support::global_lock();
        let mut sh = tablet_desk_with_window_under_keyboard();
        sh.set_keyboard(true, 1000.);
        assert!(sh.keyboard.shown(), "a tablet raises the keyboard over the desktop");
        let lifted = sh.desk.as_ref().unwrap().wins[0].r;
        assert!(lifted.y < 1200, "the front window moves up");
        assert!(lifted.y + lifted.h <= sh.kb_top() as i32, "its bottom edge clears the keyboard");
        assert_eq!(sh.desk_kb_lift, Some((AppId::Terminal, 1200, lifted.y)), "the original position is recorded for restoration");
        sh.set_keyboard(false, 1100.);
        assert_eq!(sh.desk.as_ref().unwrap().wins[0].r.y, 1200, "it goes back down when the keyboard does");
        assert!(sh.desk_kb_lift.is_none());
    }

    #[test]
    fn a_still_tablet_desktop_with_the_keyboard_up_stops_asking_for_frames() {
        // the keyboard policy is process-global and another test switches it off (keyboard::disabled_tests)
        let _lock = crate::test_support::global_lock();
        let mut sh = tablet_desk_with_window_under_keyboard();
        sh.set_keyboard(true, 1000.);
        let mut buf = vec![0u32; 1200 * 1920];
        // past the dock's launch bounce for the window (at most 10 s)
        // the frame loop ticks before it draws (the keyboard's slide ends in `tick`)
        for t in [20000., 20016.] {
            sh.tick(t);
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        }
        assert!(sh.keyboard.shown(), "the keyboard is up (form {:?}, allowed {})", sh.form, sh.keyboard_allowed());
        assert!(!sh.keyboard.animating(), "the keyboard has settled: {}", sh.keyboard.animation_state());
        assert!(!sh.needs_frame(), "nothing moves: {}", sh.frame_reason());
    }

    #[test]
    fn a_window_the_user_moved_while_lifted_stays_where_the_user_put_it() {
        // the keyboard policy is process-global and another test switches it off (keyboard::disabled_tests)
        let _lock = crate::test_support::global_lock();
        let mut sh = tablet_desk_with_window_under_keyboard();
        sh.set_keyboard(true, 1000.);
        sh.desk.as_mut().unwrap().wins[0].r.y = 500;
        sh.set_keyboard(false, 1100.);
        assert_eq!(sh.desk.as_ref().unwrap().wins[0].r.y, 500);
    }

    #[test]
    fn the_desktop_form_never_lifts_a_window_for_the_keyboard() {
        // the keyboard policy is process-global and another test switches it off (keyboard::disabled_tests)
        let _lock = crate::test_support::global_lock();
        let mut sh = tablet_desk_with_window_under_keyboard();
        sh.form = bifrost_ui::form_factor::FormFactor::Desktop;
        sh.set_keyboard(true, 1000.);
        assert!(!sh.keyboard.shown(), "a desktop has a hardware keyboard");
        assert_eq!(sh.desk.as_ref().unwrap().wins[0].r.y, 1200);
    }

    #[cfg(feature="desktop-wayland")]
    #[test]
    fn shared_ui_desktop_menu_precedes_drawer_and_dispatches_sdk_commands() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let (nx,nw)=sh.menu_buttons().into_iter().find(|b|b.0==desktop_bar::Menu::Network).map(|b|(b.1,b.2)).expect("network button");
        let centre=nx+nw/2.;
        sh.event(Event::Down(centre,10.),1000.);sh.event(Event::Up(centre,10.),1001.);
        assert!(sh.desktop_menu==Some(desktop_bar::Menu::Network));assert!(!sh.center_active());
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.commands=bifrost_proto::Commands(63);sh.ext.push(Some(host));sh.screen=Screen::App(AppId::External(0));sh.desktop_menu=Some(desktop_bar::Menu::Edit);
        let (x,y,_,_)=sh.desktop_popup_rect().unwrap();let rows=sh.menu_rows(desktop_bar::Menu::Edit);let (spans,_)=desktop_bar::menu_spans(&rows);
        let i=rows.iter().position(|r|r.text=="Select All").expect("Select All row");let (top,h)=spans[i];let row=y as f32+sh.ui.pt(top+h/2.);
        sh.event(Event::Down(x as f32+12.,row),1002.);sh.event(Event::Up(x as f32+12.,row),1003.);
        let bifrost_proto::sys::Recv::Packet(packet,_)=theirs.recv(false).unwrap() else{panic!("missing command")};
        assert_eq!(bifrost_proto::ToApp::decode(&packet),Ok(bifrost_proto::ToApp::Command(bifrost_proto::Command::SelectAll)));
    }

    /// The menu bar follows the Golden Gate order (App, File, Edit, View, Window, Help). Window lists every open window by name, with
    /// the front one checked, and choosing a row brings that window forward.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn desktop_menu_bar_follows_the_golden_gate_order_and_the_window_menu_lists_only_the_front_app() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        let order:Vec<desktop_bar::Menu>=sh.menu_buttons().into_iter().map(|b|b.0).filter(|m|!matches!(m,desktop_bar::Menu::Cpu|desktop_bar::Menu::Memory|desktop_bar::Menu::Network|desktop_bar::Menu::Power|desktop_bar::Menu::ControlCenter)).collect();
        assert_eq!(order,vec![desktop_bar::Menu::System,desktop_bar::Menu::App,desktop_bar::Menu::File,desktop_bar::Menu::Edit,desktop_bar::Menu::View,desktop_bar::Menu::Window,desktop_bar::Menu::Help]);
        sh.desk_open(AppId::Terminal,0.);sh.desk_open(AppId::Settings,0.);
        let rows=sh.menu_row_texts(desktop_bar::Menu::Window);
        assert!(rows.iter().any(|t|t=="\u{2713} Settings"),"front window checked: {rows:?}");
        // the Window menu lists the front app's windows only (bf8aa3ab6), so Terminal is not offered here
        let name=apps().iter().find(|a|a.id==AppId::Terminal).unwrap().name;
        assert!(!rows.iter().any(|t|t.ends_with(name)),"{name} is not the front app: {rows:?}");
    }

    /// The Apps grid hides the desk windows under it and gives them back, as they were, when it closes.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn appgrid_hides_desk_windows_and_restores_them_on_close() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);sh.desk_open(AppId::Settings,0.);
        sh.desk.as_mut().unwrap().wins[0].minimized=true;
        sh.appgrid_toggle(0.);
        assert!(sh.overview.grid.visible());
        assert!(sh.desk.as_ref().unwrap().wins.iter().all(|w|w.minimized),"the grid shows over no window");
        sh.appgrid_toggle(0.);
        let d=sh.desk.as_ref().unwrap();
        assert!(d.wins[0].minimized,"a window the person had minimized stays minimized");
        assert!(!d.wins[1].minimized,"a window that was open comes back");
    }

    /// Window > Zoom acts on the front visible window, not on a minimized one that still sits last in the stack.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn window_zoom_skips_minimized_windows() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);sh.desk_open(AppId::Settings,0.);
        sh.desk_window_close_or_minimize(true);
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w|w.app),Some(AppId::Terminal));
        sh.run_menu_action(desktop_bar::Action::Zoom,1.);
        let d=sh.desk.as_ref().unwrap();
        assert!(d.wins[d.index_of(AppId::Terminal).unwrap()].maximized,"the visible window zooms");
        assert!(!d.wins[d.index_of(AppId::Settings).unwrap()].maximized,"the minimized one does not");
    }

    /// Keyboard operation of the menu bar (CTL-04): arrows skip label rows and wrap, Left/Right switch menus,
    /// Return chooses, Esc closes; the pointer highlights the row under it.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn desktop_menus_work_from_the_keyboard_and_highlight_under_the_pointer() {
        use desktop_bar::Menu;
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);
        sh.desktop_menu=Some(Menu::Window);
        let rows=sh.menu_row_texts(Menu::Window);
        let first=sh.menu_step(Menu::Window,1).unwrap();
        sh.hw_menu_key(108,true,1.);assert_eq!(sh.menu_row_of(Menu::Window),Some(first),"Down selects the first actionable row of {rows:?}");
        sh.hw_menu_key(103,true,1.);assert_ne!(sh.menu_row_of(Menu::Window),Some(first),"Up wraps to the last actionable row");
        sh.hw_menu_key(106,true,1.);assert_eq!(sh.desktop_menu,Some(Menu::Help),"Right moves to the next menu");
        sh.hw_menu_key(106,true,1.);assert_eq!(sh.desktop_menu,Some(Menu::System),"and wraps to the system menu");
        assert!(sh.hw_menu_key(65,false,1.).is_some_and(|f|f.is_empty()),"a release is swallowed");
        sh.hw_menu_key(1,true,1.);assert_eq!(sh.desktop_menu,None,"Esc closes");
        assert!(sh.hw_menu_key(108,true,1.).is_none(),"with no menu the key is not taken");
        // Return runs the highlighted row: Window > Minimize hides the front window
        sh.desktop_menu=Some(Menu::Window);sh.hw_menu_key(108,true,2.);sh.hw_menu_key(28,true,2.);
        assert_eq!(sh.desktop_menu,None);
        assert!(sh.desk.as_ref().unwrap().front().is_none(),"Return chose Minimize");
        // the pointer highlights, and a bar title switches the open menu
        sh.desktop_menu=Some(Menu::Edit);
        let (x,y,_,_)=sh.desktop_popup_rect().unwrap();
        sh.desktop_menu_hover(x as f32+12.,y as f32+sh.ui.pt(sh.row_top_pt(Menu::Edit,0)+18.));
        assert!(sh.desktop_menu_row.is_none(),"a label row is not highlighted");
        let (_,hx,hw,_)=sh.menu_buttons().into_iter().find(|b|b.0==Menu::Help).unwrap();
        sh.desktop_menu_hover(hx+hw/2.,2.);assert_eq!(sh.desktop_menu,Some(Menu::Help));
    }

    /// Menus group their items with separators (HIG "The menu bar"): they are short, not selectable and never run.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn menu_separators_are_short_and_skipped_by_the_keyboard() {
        use desktop_bar::Menu;
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        // About | Settings | Services (a submenu) | Hide, Hide Others, Show All | Quit
        let rows=sh.menu_row_texts(Menu::App);
        let groups="About | Settings | Services | Hide | Quit";
        assert_eq!(rows.iter().filter(|t|t.is_empty()).count(),groups.matches('|').count(),"{groups}: {rows:?}");
        sh.desktop_menu=Some(Menu::App);
        let (_,_,_,h)=sh.desktop_popup_rect().unwrap();
        assert!((h as f32)<sh.ui.pt(8.+36.*rows.len() as f32),"separators are shorter than rows");
        sh.hw_menu_key(108,true,1.);sh.hw_menu_key(108,true,1.);
        assert_eq!(sh.menu_row_of(Menu::App),Some(2),"Down from About skips the separator to Settings");
        assert!(sh.menu_row_texts(Menu::Edit)[2].is_empty(),"Edit: Undo, Redo | Cut ...");
    }

    /// The front native Linux app's windows are listed in the Window menu (the front one checked, an untitled one by its
    /// app id); another app's windows are not (E2E N2); choosing one activates it.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn window_menu_lists_native_linux_windows_and_activates_them() {
        use desktop_bar::Menu;
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk.as_mut().unwrap().native_windows=vec![crate::bare::wayland::ForeignWindow{id:7,title:"Notes".into(),app_id:"x.plain".into(),active:true,maximized:false},crate::bare::wayland::ForeignWindow{id:8,title:String::new(),app_id:"x.plain".into(),active:false,maximized:false},crate::bare::wayland::ForeignWindow{id:9,title:"Elsewhere".into(),app_id:"x.other".into(),active:false,maximized:false}];
        let rows=sh.menu_row_texts(Menu::Window);
        assert!(rows.contains(&"\u{2713} Notes".to_string())&&rows.contains(&"  x.plain".to_string()),"{rows:?}");
        assert!(!rows.iter().any(|t|t.ends_with("Elsewhere")),"another app's window is not in this app's Window menu: {rows:?}");
        sh.desktop_menu=Some(Menu::Window);
        let i=rows.iter().position(|t|t.ends_with("x.plain")).unwrap();sh.desktop_menu_row=Some((Menu::Window,i));
        let fx=sh.desktop_menu_key(28,1.).unwrap();
        assert!(fx.iter().any(|e|matches!(e,Effect::ActivateForeign(8))),"Return activates window 8");
    }

    /// `NativeMenuItem.keyEquivalent`: the shell's key table parses, has no conflicts (Close/Minimize share a command name only
    /// where the menus do), and the menus show it in Golden Gate notation, widening the menu when a title and key would touch.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn menu_key_equivalents_parse_and_do_not_conflict_and_show_in_menus() {
        use desktop_bar::Menu;
        let mut reg=bifrost_ui::keys::Shortcuts::default();
        for (i,(name,combo)) in desktop_bar::MENU_KEYS.iter().enumerate() {
            let c=bifrost_ui::keys::KeyCombo::parse(combo).unwrap_or_else(||panic!("{name}: {combo} parses"));
            // Minimize and Minimize Window are the same command in two menus
            let id=if *name=="Minimize Window"{1}else{i as u32};
            reg.bind(c,id).unwrap_or_else(|e|panic!("{name}: {combo} conflicts with {e:?}"));
        }
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        let keys=|m:Menu|sh.menu_row_keys(m);
        assert!(keys(Menu::Edit).contains(&"\u{21E7}\u{2318}Z".to_string()),"Redo shows shift-command-Z: {:?}",keys(Menu::Edit));
        assert!(keys(Menu::File).contains(&"\u{2318}W".to_string()));
        assert!(keys(Menu::App).contains(&"\u{2318}Q".to_string()));
        sh.desktop_menu=Some(Menu::Edit);
        let (_,_,w,_)=sh.desktop_popup_rect().unwrap();
        assert!(w as f32>=sh.ui.pt(264.),"menus are at least 264 pt wide");
    }

    /// A very long window title is cut to the menu (it never makes the Window menu run off the screen).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn long_window_titles_are_truncated_in_the_window_menu() {
        use desktop_bar::Menu;
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        let title="A very long document title ".repeat(20);
        sh.desk.as_mut().unwrap().native_windows=vec![crate::bare::wayland::ForeignWindow{id:9,title:title.clone(),app_id:"x.long".into(),active:true,maximized:false}];
        let rows=sh.menu_row_texts(Menu::Window);
        let row=rows.last().unwrap();
        assert!(row.chars().count()<title.chars().count()/2,"cut: {row}");
        sh.desktop_menu=Some(Menu::Window);
        let (_,_,w,_)=sh.desktop_popup_rect().unwrap();
        assert!((w as f32)<sh.ui.pt(400.),"menu width stays reasonable: {w}");
    }

    /// Type-to-select in an open bar menu (`NativeMenu`): letters jump to a title, a repeated letter cycles, Esc closes.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn typing_in_an_open_menu_selects_by_title() {
        use desktop_bar::Menu;
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desktop_menu=Some(Menu::App);
        sh.hw_menu_key(31,true,1.); // s: Settings…
        let rows=sh.menu_row_texts(Menu::App);
        assert_eq!(sh.desktop_menu_row.map(|(_,i)|rows[i].clone()),Some("Settings\u{2026}".to_string()));
        sh.hw_menu_key(30,true,3.); // a, after the type-ahead pause: About
        assert!(sh.desktop_menu_row.map(|(_,i)|rows[i].clone()).is_some_and(|t|t.starts_with("About")));
        assert_eq!(sh.desktop_menu,Some(Menu::App),"typing keeps the menu open");
    }

    /// Cmd+, opens Settings from the keyboard (the App menu's Settings\u{2026} key equivalent).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn command_comma_opens_settings() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        assert!(sh.desk.as_ref().unwrap().front().is_none());
        // on the desktop the application menu's Settings\u{2026} (System Settings) carries the key equivalent
        assert!(sh.menu_row_keys(desktop_bar::Menu::App).contains(&"\u{2318},".to_string()));
        sh.hw_shortcut(hwkeys::Shortcut::Settings,1.);
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w|w.app),Some(AppId::Settings));
        // System Settings in front has no settings of its own, so its application menu has no Settings\u{2026} row
        assert!(!sh.menu_row_keys(desktop_bar::Menu::App).contains(&"\u{2318},".to_string()));
    }

    /// The status items sit right beside the clock whatever its width or the screen size (no fixed reserve), and the
    /// CPU and RAM readouts give way when the bar is too narrow for them.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn desktop_status_items_follow_the_measured_clock_on_any_width() {
        for w in [640, 1280, 1920, 3440] {
            let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.w=w;
            for (hour,min) in [(3,7),(23,59)] {
                sh.info.hour=hour;sh.info.min=min;
                let items=sh.menu_buttons();
                let power=items.iter().find(|b|b.0==desktop_bar::Menu::ControlCenter);
                if w>=1280 {assert!(power.is_some(),"width {w}: the Control Center extra must fit");}
                if let Some((_,x,wd,_))=power {
                    let clock_left=w as f32-sh.ui.pt(16.)-sh.ui.measure(&sh.clock_text(),12.,Weight::Medium);
                    let gap=clock_left-(x+wd);
                    assert!(gap>=0.0&&gap<=sh.ui.pt(12.),"width {w}: {gap}px between Control Center and the clock");
                }
                let first_status=items.iter().filter(|b|!matches!(b.0,desktop_bar::Menu::System|desktop_bar::Menu::App|desktop_bar::Menu::File|desktop_bar::Menu::Edit|desktop_bar::Menu::View|desktop_bar::Menu::Window|desktop_bar::Menu::Help)).map(|b|b.1).fold(f32::MAX,f32::min);// MAX when none fit
                let menus_end=items.iter().filter(|b|matches!(b.0,desktop_bar::Menu::System|desktop_bar::Menu::App|desktop_bar::Menu::File|desktop_bar::Menu::Edit|desktop_bar::Menu::View|desktop_bar::Menu::Window|desktop_bar::Menu::Help)).map(|b|b.1+b.2).fold(0.0,f32::max);
                assert!(first_status>=menus_end,"width {w}: status items overlap the app menus ({first_status} < {menus_end})");
            }
        }
    }

    #[cfg(feature="desktop-wayland")]
    #[test]
    fn review_desktop_network_menu_opens_real_settings() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desktop_menu=Some(desktop_bar::Menu::Network);
        let (x,y,w,h)=sh.desktop_popup_rect().unwrap();
        sh.desktop_bar_event(Event::Up((x+w/2) as f32,(y+h-12) as f32),1000.);
        assert_eq!(sh.screen,Screen::App(AppId::Settings));assert!(sh.desktop_menu.is_none());
        assert_eq!(sh.settings.as_ref().unwrap().page(),pages::PageId::Network);
    }

    /// System menu > Lock Screen locks the desktop (a click on the row, its action, and the compositor handing focus back
    /// to the window as the menu closes), and the row names its key equivalent.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_system_menu_lock_screen_row_locks_the_desktop_and_shows_ctrl_cmd_q() {
        use desktop_bar::Menu;
        let lock_row=|sh:&Shell|sh.menu_rows(Menu::System).iter().position(|r|r.text=="Lock Screen").expect("a Lock Screen row");
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        assert_eq!(sh.screen,Screen::App(AppId::Settings));
        let rows=sh.menu_rows(Menu::System);let row=&rows[lock_row(&sh)];
        assert_eq!(row.key_text(),Some("\u{2303}\u{2318}Q"),"the row shows control-command-Q");
        // the row's action
        let action=row.action.clone().expect("Lock Screen is enabled");
        sh.run_menu_action(action,10.);
        assert!(sh.is_locked(),"the action locks the screen");
        // a click on the row (menu open, release inside the popup)
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        sh.desktop_menu=Some(Menu::System);
        let (x,top,w,_)=sh.desktop_popup_rect().unwrap();
        let (spans,_)=desktop_bar::menu_spans(&sh.menu_rows(Menu::System));
        let (t,h)=spans[lock_row(&sh)];
        let y=top as f32+sh.ui.pt(t+h/2.);
        let fx=sh.desktop_bar_event(Event::Up((x+w/2) as f32,y),20.);
        assert!(fx.is_some());
        assert!(sh.is_locked(),"a click on Lock Screen locks the desktop");
        assert!(sh.desktop_menu.is_none());
        // the compositor gives the window focus back as the menu closes: the lock stays
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        sh.wayland_window_event(crate::bare::wayland::WindowEvent::Focus(id));
        assert!(sh.is_locked(),"a focus event must not lift a window over the lock screen");
    }

    /// The window server places and moves toplevels, so the shell's window rectangle follows the frame it reports:
    /// what is placed from the rectangle (an app's context menu opens at the pointer) is then where the window really
    /// is. Before, `state` said 852,339 for a window drawn at 800,360 and menus opened 52 px right and 21 px high
    /// (E2E N1).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_frame_from_the_window_server_moves_the_shell_rectangle_and_the_menus_placed_from_it() {
        use crate::bare::wayland::WindowEvent;
        use desktop_bar::{ContextMenu,Menu};
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        let m=sh.desk_metrics();
        let i=sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        let before=sh.desk.as_ref().unwrap().wins[i].r;
        let body_before=sh.desk.as_ref().unwrap().wins[i].body(&m);
        // the window server drew it 52 px to the right and 21 px up from where the shell believed it was
        let (x,y)=(before.x+52,before.y-21);
        sh.dirty=false;
        sh.wayland_window_event(WindowEvent::Frame(id,x,y,before.w,before.h));
        let after=sh.desk.as_ref().unwrap().wins[i].r;
        assert_eq!((after.x,after.y),(x,y),"the position is the window server's");
        assert_eq!((after.w,after.h),(before.w,before.h),"the size stays with the configure");
        assert!(sh.dirty,"the presenter gets the new frame");
        let body=sh.desk.as_ref().unwrap().wins[i].body(&m);
        assert_eq!((body.x-body_before.x,body.y-body_before.y),(52,-21),"the body, which menus are placed from, moved with it");
        // a context menu the app asks for at (40, 30) in its body opens at that point on the screen
        sh.desktop_context=Some(ContextMenu{slot:0,x:(body.x+40) as f32,y:(body.y+30) as f32,items:vec![bifrost_proto::ContextItem::new(1,"Open")]});
        sh.desktop_menu=Some(Menu::Context);
        let (px,py,_,_)=sh.desktop_popup_rect().unwrap();
        assert_eq!((px as i32,py as i32),(body.x+40,body.y+30),"top-left at the pointer");
        // the same frame again changes nothing and asks for no redraw
        sh.dirty=false;
        sh.wayland_window_event(WindowEvent::Frame(id,x,y,before.w,before.h));
        assert!(!sh.dirty,"a repeated frame is not a change");
        // a frame for a window the shell does not have is ignored
        sh.wayland_window_event(WindowEvent::Frame(9999,1,2,3,4));
        assert_eq!(sh.desk.as_ref().unwrap().wins[i].r,after);
        // an app's extra window (a flagged window-server id) follows its frame too
        let extra=AppId::ExternalWindow(0,3);
        sh.desk.as_mut().unwrap().open(&m,extra,(600,400));
        let j=sh.desk.as_ref().unwrap().index_of(extra).unwrap();
        sh.wayland_window_event(WindowEvent::Frame(desk::win_id(extra).unwrap(),321,123,600,400));
        let moved=sh.desk.as_ref().unwrap().wins[j].r;
        assert_eq!((moved.x,moved.y),(321,123),"the extra window is where the window server put it");
    }

    /// An app's extra window reports Focus with a flagged id; the screen stays on the app that opened it (hardware keys and
    /// input providers match `Screen::App(AppId::External(_))`, never the extra-window variant).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn focus_for_an_extra_window_keeps_the_screen_on_its_parent_app() {
        use crate::bare::wayland::WindowEvent;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let m=sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        let extra=AppId::ExternalWindow(0,4);
        sh.desk.as_mut().unwrap().open(&m,extra,(600,400));
        sh.wayland_window_event(WindowEvent::Focus(desk::win_id(extra).unwrap()));
        assert_eq!(sh.screen,Screen::App(AppId::External(0)),"the parent app keeps the screen");
    }

    /// E2E W2: green on Music made the window 2560x1440 but left it at its old origin, so it ran off the right and
    /// bottom, and the Dock stayed over it. The window server puts the toplevel on the whole output and says so
    /// (`xdg_toplevel` full-screen state); the shell's window then fills the output at 0,0 whatever size that is, the
    /// menu bar hides, the Dock slides away, and leaving puts the frame back.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn full_screen_from_the_window_server_fills_the_output_and_hides_the_menu_bar_and_dock() {
        use crate::bare::wayland::WindowEvent;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        let m=sh.desk_metrics();
        let i=sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        let before=sh.desk.as_ref().unwrap().wins[i].r;
        assert!(!sh.desk_bar_hidden(),"an ordinary window leaves the menu bar");
        sh.wayland_window_event(WindowEvent::Fullscreen(id,true));
        let w=sh.desk.as_ref().unwrap().wins[i];
        assert!(w.full);
        assert_eq!((w.r.x,w.r.y,w.r.w,w.r.h),(0,0,m.w,m.h),"the whole output at its origin");
        assert!(sh.desk.as_ref().unwrap().front_full());
        assert!(sh.desk_bar_hidden(),"the menu bar auto-hides");
        assert!(sh.desk.as_ref().unwrap().dock_autohides(),"the Dock auto-hides");
        assert!(sh.desk.as_ref().unwrap().dock_hide_at.is_some(),"and starts to slide away");
        sh.dirty=false;
        sh.wayland_window_event(WindowEvent::Fullscreen(id,true));
        assert!(!sh.dirty,"the same state again is no change");
        sh.wayland_window_event(WindowEvent::Fullscreen(id,false));
        assert_eq!(sh.desk.as_ref().unwrap().wins[i].r,before,"the frame from before");
        assert!(!sh.desk.as_ref().unwrap().front_full());
        assert!(!sh.desk_bar_hidden());
        assert!(sh.dock_is_visible(),"the Dock is back");
    }

    /// A running screen recording keeps its indicator on screen: the menu bar does not hide over a full-screen window, and the
    /// indicator is among the bar's buttons however little room is left.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_screen_recording_keeps_the_menu_bar_and_its_indicator_over_a_full_screen_window() {
        use crate::bare::wayland::WindowEvent;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        sh.wayland_window_event(WindowEvent::Fullscreen(id,true));
        assert!(sh.desk_bar_hidden(),"without a recording the bar auto-hides");
        let dir=crate::shell::photos::library::tests::scratch("recording-indicator");
        let area=crate::shell::screenrec::Area{x:0,y:0,w:4,h:2};
        sh.rec=Some(crate::shell::screenrec::Session::start(dir.join("a.avi"),crate::shell::screenrec::Encoder::Avi,area,10,0.).unwrap());
        assert!(!sh.desk_bar_hidden(),"the recording keeps the bar over the full-screen window");
        assert!(sh.menu_buttons().iter().any(|b|b.0==desktop_bar::Menu::Recording),"the indicator is in the bar");
        if let Some(r)=sh.rec.take(){let _=r.finish();}
        assert!(sh.desk_bar_hidden(),"and the bar hides again once the recording has ended");
        let _=std::fs::remove_dir_all(&dir);
    }

    /// E2E round 3: the menu bar over a full-screen window follows the pointer, not the last press. A pointer that only
    /// moves (no button) away from the top hides the bar again, and moving to the top reveals it.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_bar_over_a_full_screen_window_follows_the_pointer_when_it_only_moves() {
        use crate::bare::wayland::WindowEvent;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        sh.wayland_window_event(WindowEvent::Fullscreen(id,true));
        assert!(sh.desk_bar_hidden(),"no pointer seen: hidden");
        sh.dirty=false;
        sh.desktop_pointer(300.,2.,false,10.);
        assert!(!sh.desk_bar_hidden(),"the pointer reached the top edge: the bar is there");
        assert!(sh.dirty,"and it repaints");
        sh.dirty=false;
        sh.desktop_pointer(300.,sh.h as f32/2.,false,20.);
        assert!(sh.desk_bar_hidden(),"the pointer moved away without a button: hidden again, not left over the window");
        assert!(sh.dirty,"and it repaints");
    }

    /// A rest on the green button that began before the click must not open the one-item full-screen menu on the window
    /// the click took full screen.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_rest_on_the_green_button_before_the_click_opens_no_menu_on_the_full_screen_window() {
        use crate::bare::wayland::WindowEvent;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let id=apps().iter().position(|a|a.id==AppId::Settings).unwrap() as u32+1;
        let (gx,gy)=green_button(&sh);
        sh.desktop_pointer(gx,gy,false,1000.);
        assert!(sh.desk.as_ref().unwrap().arrange_at.is_some(),"the rest has begun");
        sh.wayland_window_event(WindowEvent::Fullscreen(id,true));
        assert!(sh.desk.as_ref().unwrap().arrange_at.is_none(),"going full screen ends the rest");
        sh.arrange_tick(1000.+desk::ARRANGE_DWELL_MS+1.);
        assert!(sh.desktop_menu.is_none(),"so no menu opens on the full-screen window");
    }

    /// The same on an output of any size: the frame is the output, never a number of pixels.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn full_screen_fills_outputs_of_any_size() {
        for (w,h) in [(1000usize,700usize),(1920,1080),(2560,1440),(3840,2160)] {
            let mut sh=shell_sized(w,h);sh.screen=Screen::Home;sh.anim=None;
            sh.desk=Some(desk::Desk::new());
            sh.desk_open(AppId::Settings,0.);
            sh.desk_set_full_screen(AppId::Settings,true);
            let m=sh.desk_metrics();
            let r=sh.desk.as_ref().unwrap().wins[0].r;
            assert_eq!((r.x,r.y,r.w,r.h),(0,0,m.w,m.h),"{w}x{h}");
        }
    }

    /// The first pixel of a window's green button, for the pointer tests.
    #[cfg(feature="desktop-wayland")]
    fn green_button(sh:&Shell)->(f32,f32) {
        let m=sh.desk_metrics();let d=sh.desk.as_ref().unwrap();let r=d.wins[0].r;
        for y in (r.y..r.y+80).step_by(2) {for x in (r.x..r.x+200).step_by(2) {
            if let Some((_,desk::Hit::Light(desk::Light::Zoom)))=d.pick(&m,x as f32,y as f32) {return (x as f32,y as f32);}
        }}
        panic!("the window has no green button");
    }

    /// E2E W3: resting the pointer on the green button opens the window menu under it (Fill, Centre, Move & Resize,
    /// Full Screen); the menu stays while the pointer is on the button or the menu and closes when it leaves both.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn resting_on_the_green_button_opens_the_window_menu_and_leaving_it_closes_it() {
        use desktop_bar::Menu;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let (gx,gy)=green_button(&sh);
        sh.desktop_pointer(gx,gy,false,1000.);
        sh.arrange_tick(1000.+desk::ARRANGE_DWELL_MS-1.);
        assert!(sh.desktop_menu.is_none(),"not before the dwell is over");
        assert!(sh.next_wake(1000.).is_some_and(|w|w<=1000.+desk::ARRANGE_DWELL_MS),"the shell wakes for it");
        sh.arrange_tick(1000.+desk::ARRANGE_DWELL_MS);
        assert_eq!(sh.desktop_menu,Some(Menu::Arrange));
        let texts=sh.menu_row_texts(Menu::Arrange);
        for want in ["Fill","Center","Return to Previous Size","Enter Full Screen"] {
            assert!(texts.iter().any(|t|t.trim()==want),"{want} is in {texts:?}");
        }
        let (px,py,pw,ph)=sh.desktop_popup_rect().unwrap();
        assert!(py as f32>=gy,"the menu opens under the button");
        // on the way to the menu and inside it the menu stays
        sh.desktop_pointer(gx,gy+sh.ui.pt(10.),false,1700.);
        assert_eq!(sh.desktop_menu,Some(Menu::Arrange));
        sh.desktop_pointer((px+pw/2) as f32,(py+ph/2) as f32,false,1800.);
        assert_eq!(sh.desktop_menu,Some(Menu::Arrange));
        // away from both it closes, and resting again opens it again
        sh.desktop_pointer(sh.w as f32-4.,sh.h as f32-4.,false,1900.);
        assert!(sh.desktop_menu.is_none());
        assert!(sh.desk.as_ref().unwrap().arrange.is_none());
        sh.desktop_pointer(gx,gy,false,2000.);
        sh.arrange_tick(2000.+desk::ARRANGE_DWELL_MS);
        assert_eq!(sh.desktop_menu,Some(Menu::Arrange));
    }

    /// A press on the green button while its menu is open is still the button's own click, so full screen keeps
    /// working for anyone who rests on the button before clicking.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_press_on_the_green_button_under_its_open_menu_is_not_swallowed() {
        use desktop_bar::Menu;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let (gx,gy)=green_button(&sh);
        sh.desktop_pointer(gx,gy,false,1000.);
        sh.arrange_tick(1000.+desk::ARRANGE_DWELL_MS);
        assert_eq!(sh.desktop_menu,Some(Menu::Arrange));
        assert!(sh.desktop_bar_event(Event::Down(gx,gy),2000.).is_none(),"the press goes on to the window");
        assert!(sh.desktop_menu.is_none(),"and the menu closes");
    }

    /// Choosing in the menu tiles the window it hangs from (the shell places it itself outside the Wayland
    /// session; inside, it asks the window server, `tile_requests`).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn choosing_in_the_window_menu_tiles_that_window_or_takes_it_full_screen() {
        let _placement=ShellPlacesWindows::new();
        use desktop_bar::Action;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let m=sh.desk_metrics();
        let area=desk::Desk::area(&m);
        sh.run_menu_action(Action::Arrange(AppId::Settings,desk::Arrange::Tile(desk::Tile::Left)),1000.);
        let i=sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        assert_eq!(sh.desk.as_ref().unwrap().wins[i].r,desk::tile_rect(area,desk::Tile::Left));
        sh.run_menu_action(Action::Arrange(AppId::Settings,desk::Arrange::Previous),1100.);
        assert_ne!(sh.desk.as_ref().unwrap().wins[i].r,desk::tile_rect(area,desk::Tile::Left),"back to the previous size");
        sh.run_menu_action(Action::Arrange(AppId::Settings,desk::Arrange::FullScreen(true)),1200.);
        let r=sh.desk.as_ref().unwrap().wins[i].r;
        assert_eq!((r.x,r.y,r.w,r.h),(0,0,m.w,m.h));
        sh.run_menu_action(Action::Arrange(AppId::Settings,desk::Arrange::FullScreen(false)),1300.);
        assert!(!sh.desk.as_ref().unwrap().wins[i].full);
    }

    /// "Tile with Next Window": this window on the left half and the one behind it on the right half.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_window_menu_can_tile_two_windows_side_by_side() {
        let _placement=ShellPlacesWindows::new();
        use desktop_bar::Action;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);
        sh.desk_open(AppId::Settings,10.);
        let m=sh.desk_metrics();
        let area=desk::Desk::area(&m);
        sh.run_menu_action(Action::Arrange(AppId::Settings,desk::Arrange::SideBySide),1000.);
        let d=sh.desk.as_ref().unwrap();
        assert_eq!(d.wins[d.index_of(AppId::Settings).unwrap()].r,desk::tile_rect(area,desk::Tile::Left));
        assert_eq!(d.wins[d.index_of(AppId::Terminal).unwrap()].r,desk::tile_rect(area,desk::Tile::Right));
    }

    /// Ctrl+Cmd+Q is the lock chord: it maps to `Shortcut::Lock` (the plain Cmd+Q still closes) and locks the desktop.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn ctrl_cmd_q_locks_the_desktop() {
        let mut keys=hwkeys::HwKeys::default();
        let mut chord=|mods:&[u16],key:u16,term:bool|{for m in mods{keys.key_front(*m,true,term,false);}let r=keys.key_front(key,true,term,false);keys.key_front(key,false,term,false);for m in mods.iter().rev(){keys.key_front(*m,false,term,false);}r};
        const CTRL:u16=29;const META:u16=125;const Q:u16=16;
        assert_eq!(chord(&[CTRL,META],Q,false),Some(hwkeys::Shortcut::Lock));
        assert_eq!(chord(&[CTRL,META],Q,true),Some(hwkeys::Shortcut::Lock),"also with the Terminal in front");
        assert_eq!(chord(&[META],Q,false),Some(hwkeys::Shortcut::Quit),"Cmd+Q alone quits the app");
        assert_eq!(chord(&[META],17,false),Some(hwkeys::Shortcut::Close),"Cmd+W still only closes the window");
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let fx=sh.hw_shortcut(hwkeys::Shortcut::Lock,5.);
        assert!(sh.is_locked(),"the chord locks the screen");
        assert_eq!(fx,vec![Effect::ScreenOn],"the desktop keeps the display on and shows the login window");
    }

    /// A window whose app has committed no frame is not drawn and not hit: it used to show as an empty black window
    /// (three grey lights, no content) behind the real ones.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_window_whose_app_has_drawn_nothing_is_not_mapped() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let (mine,_theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        sh.ext.push(Some(host));
        let m=sh.desk_metrics();
        let ghost=AppId::External(0);
        let ghost_extra=AppId::ExternalWindow(0,3);
        sh.desk.as_mut().unwrap().open(&m,ghost,(600,400));
        sh.desk.as_mut().unwrap().open(&m,ghost_extra,(600,400));
        sh.desk_open(AppId::Settings,0.);
        sh.desk_refresh_unmapped();
        let d=sh.desk.as_ref().unwrap();
        assert!(d.unmapped.contains(&ghost),"an app window with no frame is unmapped");
        assert!(d.unmapped.contains(&ghost_extra),"an extra window the table does not hold has no frame");
        assert!(!d.unmapped.contains(&AppId::Settings),"a built-in draws its own content");
        // with Settings out of the way the ghost's rectangle takes no pointer
        let r=d.wins[d.index_of(ghost).unwrap()].r;
        let (cx,cy)=((r.x+r.w/2) as f32,(r.y+r.h/2) as f32);
        let i=d.index_of(AppId::Settings).unwrap();
        sh.desk.as_mut().unwrap().wins[i].minimized=true;
        assert_eq!(sh.desk.as_ref().unwrap().pick(&m,cx,cy),None,"an unmapped window takes no pointer");
        let (w,h)=(sh.w as usize,sh.h as usize);
        let mut px=vec![0u32;w*h];
        sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut px,y0:0},0.);
        assert!(sh.desk.as_ref().unwrap().unmapped.contains(&ghost),"drawing keeps the list");
    }

    /// A fixture host standing for a running external app in slot 0.
    #[cfg(feature="desktop-wayland")]
    #[allow(dead_code)]
    fn fixture_app(sh:&mut Shell)->bifrost_proto::sys::Channel{
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        sh.ext.push(Some(host));
        theirs
    }

    /// An app that exits without the shell's `pump` seeing it (a failed send, a stall kill) takes its main and extra
    /// windows with it: nothing stays on the desk, in the Window menu or in front.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn an_app_that_exits_leaves_no_window_behind() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let _peer=fixture_app(&mut sh);
        let m=sh.desk_metrics();
        let before=sh.desk_dock(&m).0.len();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        sh.desk.as_mut().unwrap().open(&m,AppId::ExternalWindow(0,3),(500,300));
        sh.screen=Screen::App(AppId::External(0));
        // a live app keeps its main window (the extra one is the table's to keep: this fixture's table holds none)
        sh.ext_tick(1.0);
        assert!(sh.desk.as_ref().unwrap().index_of(AppId::External(0)).is_some());
        // the process goes away outside `pump`
        sh.ext[0].as_mut().unwrap().kill();
        assert!(!sh.ext[0].as_ref().unwrap().running());
        // an extra window is back on the desk, as a stale one would be
        sh.desk.as_mut().unwrap().open(&m,AppId::ExternalWindow(0,4),(500,300));
        sh.ext_tick(2.0);
        let d=sh.desk.as_ref().unwrap();
        assert!(d.wins.is_empty(),"main and extra windows are removed: {:?}",d.wins.iter().map(|w|w.app).collect::<Vec<_>>());
        assert!(sh.screen==Screen::Home,"the front moves on to the desktop");
        let rows=sh.menu_rows(desktop_bar::Menu::Window);
        assert!(!rows.iter().any(|r|matches!(r.action,Some(desktop_bar::Action::Open(AppId::External(_)|AppId::ExternalWindow(..))))),"the Window menu lists no dead window");
        assert_eq!(sh.desk_dock(&m).0.len(),before,"the Dock gained no tile for the dead app");
        // drawn once more: no frame for it either
        let (w,h)=(sh.w as usize,sh.h as usize);
        let mut px=vec![0u32;w*h];
        sh.dirty=true;
        sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut px,y0:0},3.);
        assert!(sh.take_wayland_windows().is_empty(),"no frame is presented for a dead app");
    }

    /// A window restored at boot whose app never relaunches (its host is gone) is not drawn, and does not stay.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_restored_window_whose_app_does_not_come_back_is_not_drawn() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let _peer=fixture_app(&mut sh);
        let m=sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        sh.ext[0].as_mut().unwrap().kill();
        sh.desk_refresh_unmapped();
        assert!(sh.desk.as_ref().unwrap().unmapped.contains(&AppId::External(0)),"a window whose process is gone is not mapped");
        sh.ext_tick(1.0);
        assert!(sh.desk.as_ref().unwrap().wins.is_empty());
    }

    /// The pixels of the app in front are not by themselves a reason to draw the whole desktop, and the same state an app
    /// sends again is no reason at all.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn an_external_apps_pixels_ask_for_a_window_frame_and_its_repeated_state_for_none() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let peer=fixture_app(&mut sh);
        let m=sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        sh.screen=Screen::App(AppId::External(0));
        // the first tick settles the foreground change
        sh.ext_tick(1.0);
        let a=bifrost_proto::sys::shm_create("quiet-a",48).unwrap();
        let b=bifrost_proto::sys::shm_create("quiet-b",48).unwrap();
        peer.send(&bifrost_proto::ToShell::Buffers{w:4,h:3,generation:1}.encode(),&[a.0,b.0]).unwrap();
        sh.ext_tick(2.0);
        sh.dirty=false;sh.ext_frame=false;
        // pixels alone
        peer.send(&bifrost_proto::ToShell::Frame{buf:0,damage:vec![[0,0,4,3]]}.encode(),&[]).unwrap();
        sh.ext_tick(3.0);
        assert!(sh.ext_frame,"the new pixels ask for a frame");
        assert!(!sh.dirty,"and for nothing else: no whole desktop frame");
        assert!(sh.frame_reason().contains("ext-frame"),"the trace names the cause: {}",sh.frame_reason());
        // the same undo titles twice: the first changes what the menus show, the second nothing
        let titles=bifrost_proto::ToShell::UndoTitles{undo:"Undo Typing".into(),redo:String::new()};
        sh.dirty=false;sh.ext_frame=false;
        peer.send(&titles.encode(),&[]).unwrap();
        sh.ext_tick(4.0);
        assert!(sh.dirty,"a changed title is drawn by the shell");
        sh.dirty=false;sh.ext_frame=false;
        peer.send(&titles.encode(),&[]).unwrap();
        sh.ext_tick(5.0);
        assert!(!sh.dirty&&!sh.ext_frame,"identical state costs no frame");
    }

    /// New pixels from the app in front redraw the windows and nothing else: the shell's own surface is not touched and
    /// reports no damage.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_frame_for_an_apps_pixels_alone_draws_no_shell_surface() {
        let _lock=crate::test_support::global_lock();
        let saved=std::env::var_os("SPRINGBOARD_WAYLAND");std::env::set_var("SPRINGBOARD_WAYLAND","1");
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.drawn=Drawn::App;
        let (w,h)=(sh.w as usize,sh.h as usize);
        let mut pixels=vec![0;w*h];
        // the first frame draws the desktop whole (it is what sets desk_drawn)
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},0.);
        assert!(sh.desk_drawn);
        // settled, as the shell is between ticks: the test shell's keyboard client still slides out (it starts shown), and an
        // animating desktop is by design a whole-frame desktop
        sh.dirty=false;
        sh.keyboard.tick(1.0e6);
        assert!(!sh.desk_animating(1000.),"nothing animates on the settled desktop");
        let before=pixels.clone();
        sh.ext_frame=true;
        assert!(sh.needs_frame(),"the pixels ask for a frame");
        let damage=sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},1000.);
        // (a colour fade of the appearance is process-wide state another test may be running: that frame is a full one)
        if !bifrost_render::appearance::fading() {
            assert!(damage.is_empty(),"no damage on the shell's own surface");
            assert!(pixels==before,"nothing of it was drawn");
        }
        assert!(!sh.ext_frame,"the frame settled it");
        // another cause pending: the ordinary frame
        sh.ext_frame=true;sh.dirty=true;
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},2000.);
        assert!(!sh.ext_frame);
        if let Some(v)=saved {std::env::set_var("SPRINGBOARD_WAYLAND",v)} else {std::env::remove_var("SPRINGBOARD_WAYLAND")};
    }

    /// A menu, panel or overlay that is only open does not ask for frames: input and the overlays' own timers mark the
    /// shell dirty when something changes (an idle open menu used to redraw the whole desktop at every vsync).
    #[test]
    fn an_open_menu_that_is_idle_asks_for_no_frames() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.drawn=Drawn::App;
        let (w,h)=(sh.w as usize,sh.h as usize);
        let mut pixels=vec![0;w*h];
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},0.);
        sh.dirty=false;
        sh.keyboard.tick(1.0e6);
        assert!(!sh.desk_animating(1000.),"nothing animates on the settled desktop");
        sh.desktop_menu=Some(super::desktop_bar::Menu::Help);
        assert!(!sh.desk_animating(1000.),"an open menu is not motion");
        assert!(!sh.quick_look.wants_frames(),"a closed preview wants nothing");
    }

    /// One tile per app: a key kept in the Dock twice, or an app with two windows, never makes a second tile.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_dock_shows_one_tile_per_app() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let icon=apps().iter().find(|a|a.id==AppId::Settings).unwrap().icon.to_string();
        sh.desk.as_mut().unwrap().dock_preferences.pinned=vec![icon.clone(),icon];
        let m=sh.desk_metrics();
        let index=apps().iter().position(|a|a.id==AppId::Settings).unwrap();
        assert_eq!(sh.desk_dock(&m).0.iter().filter(|&&i|i==index).count(),1,"a key pinned twice is one tile");
        sh.desk_open(AppId::Settings,0.);
        let _peer=fixture_app(&mut sh);
        let one=sh.desk_dock(&m).0.len();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        sh.desk.as_mut().unwrap().open(&m,AppId::ExternalWindow(0,3),(500,300));
        assert_eq!(sh.desk_dock(&m).0.len(),one,"a second window of an app adds no tile");
    }

    /// On the login lock a session starts on, opening or refocusing a window (a restored window, an app opening
    /// itself) never leaves the lock: only a successful unlock changes the screen.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_window_opened_on_the_login_lock_does_not_leave_it() {
        let mut sh=desktop_shell();
        assert!(sh.is_locked(),"a session starts on the login lock");
        let _peer=fixture_app(&mut sh);
        sh.desk_open(AppId::Settings,0.);
        assert!(sh.is_locked(),"desk_open does not unlock");
        sh.desk_open(AppId::External(0),1.);
        assert!(sh.is_locked(),"an external app opening itself does not unlock");
        sh.desk_refocus();
        assert!(sh.is_locked(),"a refocus does not unlock");
        for k in 0..3 { sh.ext_tick(10.0+k as f64); assert!(sh.is_locked(),"tick {k}"); }
    }

    /// The lock stays up while a window's app lives, dies or changes the desk: no tick lifts a window over it.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_lock_survives_the_ticks_of_a_desk_with_external_windows() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let _peer=fixture_app(&mut sh);
        let m=sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m,AppId::External(0),(600,400));
        sh.screen=Screen::App(AppId::External(0));
        sh.lock_screen();
        assert!(sh.is_locked());
        for k in 0..5 { sh.ext_tick(10.0+k as f64); assert!(sh.is_locked(),"live app, tick {k}"); }
        sh.ext[0].as_mut().unwrap().kill();
        for k in 0..5 { sh.ext_tick(20.0+k as f64); assert!(sh.is_locked(),"dead app, tick {k}"); }
        sh.desk_open(AppId::Settings,30.);
        assert!(sh.is_locked(),"opening a window does not unlock");
    }

    /// Desktop frame cost at 1920x1080 with two windows open. Run through `tools/bench.py`.
    #[test]
    #[ignore]
    fn bench_desktop_frame() {
        let (w, h) = (1920usize, 1080usize);
        let mut sh = shell_sized(w, h);
        sh.desk = Some(desk::Desk::new());
        unlock(&mut sh);
        sh.desk_open(AppId::Settings, 0.);
        sh.desk_open(apps()[1].id, 0.);
        settle(&mut sh, 0.0);
        let mut buf = vec![0u32; w * h];
        let mut run = |sh: &mut Shell, runs: usize, dirty: bool| {
            for t in 0..3 { sh.dirty = true; sh.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, 1e9 + t as f64); }
            let t0 = std::time::Instant::now();
            for t in 0..runs {
                if dirty { sh.dirty = true; }
                sh.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, 1e9 + 10.0 + t as f64 * 16.0);
            }
            t0.elapsed().as_secs_f64() * 1000.0 / runs as f64
        };
        println!("BENCH desktop_frame_dirty_1080p {:.3} ms", run(&mut sh, 30, true));
        println!("BENCH desktop_frame_idle_1080p {:.3} ms", run(&mut sh, 30, false));
    }

    #[test]
    fn soft_shadow_surrounds_the_shape_and_leaves_its_inside_clear() {
        let ui = ui();
        let (w, h) = (300usize, 160usize);
        for _ in 0..2 { // second pass draws from the cache
            let mut buf = vec![0u32; w * h];
            super::desk::soft_shadow(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 80.0, 50.0, 140.0, 40.0, ui.pt(8.0));
            assert_eq!(buf[70 * w + 150], 0, "inside the shape stays transparent");
            assert!(buf[(50 + 40 + 4) * w + 150] >> 24 > 0, "below the shape is shaded");
            assert_eq!(buf[10 * w + 10], 0, "far away is untouched");
        }
    }

    #[test]
    #[ignore]
    fn bench_chrome_text() {
        let ui = ui();
        let labels = ["BifrOSt", "File", "Edit", "Help", "CPU 3%", "RAM 6%", "Fri 9 Oct  1:52", "Settings", "Close Window", "Minimize Window", "Maximize / Restore Window"];
        let time = |runs: usize, f: &mut dyn FnMut()| {
            f();
            let t0 = std::time::Instant::now();
            for _ in 0..runs { f(); }
            t0.elapsed().as_secs_f64() * 1000.0 / runs as f64
        };
        let fresh = time(200, &mut || { for l in labels { std::hint::black_box(ui.text(l, 13.0, Weight::Regular, WHITE)); } });
        println!("BENCH chrome_text_uncached_11 {:.3} ms", fresh);
        let mut cache = bifrost_render::cache::TextCache::default();
        let cached = time(200, &mut || { for l in labels { std::hint::black_box(cache.get(&ui, l, 13.0, Weight::Regular, WHITE)); } });
        println!("BENCH chrome_text_cached_11 {:.3} ms", cached);
    }

    #[test]
    #[ignore]
    fn bench_chrome_effects() {
        let ui = ui();
        let (w, h) = (1920usize, 1080usize);
        let mut buf = vec![0u32; w * h];
        let time = |runs: usize, f: &mut dyn FnMut()| {
            f();
            let t0 = std::time::Instant::now();
            for _ in 0..runs { f(); }
            t0.elapsed().as_secs_f64() * 1000.0 / runs as f64
        };
        let mut s = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
        let dock = time(200, &mut || super::desk::soft_shadow(&mut s, &ui, 400.0, 960.0, 1100.0, 90.0, ui.pt(18.0)));
        println!("BENCH dock_shadow_1080p {:.3} ms", dock);
        let menu = time(200, &mut || super::desk::soft_shadow(&mut s, &ui, 100.0, 30.0, 264.0, 150.0, ui.pt(8.0)));
        println!("BENCH menu_shadow_1080p {:.3} ms", menu);
        let dim = time(100, &mut || super::desk::dim_rect(s.px, w, 200, 100, 1000, 700, super::desk::BACKGROUND_DIM));
        println!("BENCH window_dim_800x600 {:.3} ms", dim);
    }

    /// Cost of an open menu: hover hit-testing and a keyboard step (each rebuilds the rows) and the popup draw.
    #[cfg(feature="desktop-wayland")]
    #[test]
    #[ignore]
    fn bench_menu_popup() {
        use desktop_bar::Menu;
        let mut sh=shell_sized(1920,1080);sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);sh.desktop_menu=Some(Menu::Window);
        let (x,y,_,_)=sh.desktop_popup_rect().unwrap();
        let time=|runs:usize,f:&mut dyn FnMut()|{f();let t0=std::time::Instant::now();for _ in 0..runs{f();}t0.elapsed().as_secs_f64()*1000.0/runs as f64};
        let hover=time(2000,&mut ||{sh.desktop_menu_hover(x as f32+12.,y as f32+sh.ui.pt(60.));sh.desktop_menu_hover(x as f32+12.,y as f32+sh.ui.pt(100.));});
        println!("BENCH menu_hover_2moves {:.4} ms",hover);
        sh.desktop_menu_row=None;let key=time(2000,&mut ||{sh.hw_menu_key(108,true,1.);});
        assert!(sh.menu_row_of(Menu::Window).is_some(),"the key bench selects rows");sh.desktop_menu_row=None;sh.hw_menu_key(108,true,1.);let a=sh.menu_row_of(Menu::Window);sh.hw_menu_key(108,true,1.);assert_ne!(a,sh.menu_row_of(Menu::Window));
        println!("BENCH menu_key_step {:.4} ms",key);
        let (w,h)=(1920usize,1080usize);let mut buf=vec![0u32;w*h];
        let draw=time(100,&mut ||{let mut s=Surface{w,h,stride:w,px:&mut buf,y0:0};sh.draw_desktop_popup(&mut s);});
        println!("BENCH menu_popup_draw_cpu {:.3} ms",draw);
    }

    /// Developer preview: `PREVIEW_DIR=/tmp cargo test preview_settings -- --ignored` writes settings-dark.png and settings-light.png.
    #[test]
    #[ignore]
    fn preview_settings() {
        let _lock=crate::test_support::global_lock();
        let Some(dir)=std::env::var_os("PREVIEW_DIR") else {return};
        let pages=std::env::var("PREVIEW_PAGES").unwrap_or_else(|_|"General".into());
        for (name,light) in [("dark",false),("light",true)].into_iter().filter(|(n,_)|std::env::var("PREVIEW_ONLY").map_or(true,|o|o==*n)) {
            for (n,page) in pages.split(',').enumerate() {
                bifrost_render::appearance::jump_to(light);
                let mut sh=shell_sized(1800,1100);sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
                {let mut info=sh.info.clone();info.display_mode=Some((2560,1440,165));sh.set_info(info);}
                sh.desk_open(AppId::Settings,0.);
                let (w,h)=(sh.w as usize,sh.h as usize);let mut img=Image::new(w,h);
                sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},0.);
                if let Some(mut st)=sh.settings.take(){st.preview_open(&sh.ui,page,10.);sh.settings=Some(st);}
                for k in 0..30 {sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},1000.+k as f64*100.);}
                std::fs::write(std::path::Path::new(&dir).join(format!("settings-{name}-{n}.png")),crate::shell::screenshot::encode(&img)).unwrap();
            }
        }
    }

    /// A login backend that answers `reply` for any password.
    #[cfg(test)]
    struct FixedLogin(login::Reply);
    #[cfg(test)]
    impl login::Backend for FixedLogin {
        fn authenticate(&self, _: &str) -> login::Reply { self.0.clone() }
        fn account(&self) -> Option<login::AccountInfo> {
            Some(login::AccountInfo { full_name: "Owner".into(), username: "owner".into(), has_password: true, locked_out_secs: 0, password_hint: String::new() })
        }
    }

    #[cfg(test)]
    fn login_shell(reply: login::Reply) -> Shell {
        let mut sh = shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(reply)));
        sh
    }

    #[test]
    fn desktop_lock_screen_shows_the_login_window_with_the_display_on() {
        let mut sh = desktop_shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Unavailable)));
        sh.screen = Screen::Home;
        sh.anim = None;
        let fx = sh.lock_screen();
        assert_eq!(sh.screen, Screen::Lock);
        assert!(sh.screen_on, "a desktop lock keeps the display on");
        assert!(sh.login_open(), "the login window is up at once");
        assert_eq!(fx, vec![Effect::ScreenOn]);
    }

    #[test]
    fn desktop_lock_screen_field_takes_typing_shows_caps_lock_and_refuses_a_wrong_password() {
        let mut sh = desktop_shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Wrong { retry_secs: 0 })));
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.lock_screen();
        sh.hw_type(keyboard::Out::Text("w".into()), 10.0);
        sh.hw_type(keyboard::Out::Text("p".into()), 20.0);
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(2), "each key is a dot in the field");
        sh.set_key_mods(bifrost_proto::ModifierFlags::CAPS_LOCK);
        assert!(sh.login.as_ref().is_some_and(|l| l.caps), "Caps Lock on shows the hint");
        sh.set_key_mods(bifrost_proto::ModifierFlags::EMPTY);
        assert!(sh.login.as_ref().is_some_and(|l| !l.caps));
        sh.hw_type(keyboard::Out::Enter, 30.0);
        for k in 0..200 {
            sh.tick(40.0 + k as f64 * 20.0);
            if sh.login.as_ref().is_some_and(|l| !l.busy()) { break; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(sh.is_locked(), "a wrong password never unlocks");
        assert!(sh.session_token.is_none());
        let l = sh.login.as_ref().expect("the window stays up");
        assert!(l.is_empty() && l.msg == login::Msg::Wrong, "the field is cleared and says so");
    }

    /// E2E N0: a lock after a login from the System menu's Lock Screen row (also the Lock Screen hot corner and
    /// AssistiveTouch's Lock) went through `sleep_screen`, which on a desktop turned the display off: every key was
    /// dropped, so no dot appeared and the session could not be unlocked. It now keeps the display on, opens the login
    /// window and signs in again.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_desktop_lock_after_login_from_the_menu_can_be_unlocked() {
        use desktop_bar::Action;
        let mut sh = desktop_shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Ok("tok2".into()))));
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.session_token = Some("tok".into());
        let fx = sh.run_menu_action(Action::Lock, 1000.0);
        assert!(sh.is_locked(), "the menu row locks");
        assert!(sh.screen_on, "a desktop lock keeps the display on");
        assert!(!fx.contains(&Effect::ScreenOff), "and never turns it off");
        assert!(sh.session_token.is_none(), "the old token is dropped");
        assert!(sh.login_open(), "the login window is up at once");
        sh.hw_type(keyboard::Out::Text("p".into()), 1010.0);
        sh.hw_type(keyboard::Out::Text("w".into()), 1020.0);
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(2), "typing shows dots");
        sh.hw_type(keyboard::Out::Enter, 1030.0);
        for k in 0..200 {
            sh.tick(1040.0 + k as f64 * 20.0);
            if !sh.is_locked() { break; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!sh.is_locked(), "the right password unlocks the second lock");
        assert_eq!(sh.session_token.as_deref(), Some("tok2"));
    }

    /// A desktop signed in, unlocked, with a login backend that takes any password, as the E2E N0 session had.
    #[cfg(test)]
    fn signed_in_desktop() -> Shell {
        let mut sh = desktop_shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Ok("tok2".into()))));
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.session_token = Some("tok".into());
        sh
    }

    /// E2E N0, the chord path: Ctrl+Cmd+Q with one of the shell's layers that hold the keyboard up (the character viewer,
    /// the assistant panel, the sheets' surface) left it over the lock screen, and `hw_type` returned before the lock
    /// screen's keys: no dot, no unlock. The lock closes them and the login window takes the typing.
    #[test]
    fn ctrl_cmd_q_with_a_keyboard_layer_up_still_types_into_the_login_window() {
        let mut sh = signed_in_desktop();
        sh.char_viewer_open();
        sh.modal_layer = Some(capture_flow::Layer { x: 0, y: 0, image: Image::new(1, 1), input: None });
        sh.hw_shortcut(hwkeys::Shortcut::Lock, 1000.0);
        assert!(sh.is_locked(), "Ctrl+Cmd+Q locks");
        assert!(sh.screen_on, "a desktop lock keeps the display on");
        assert!(sh.char_viewer.is_none(), "the character viewer goes with the lock");
        assert!(sh.modal_layer.is_none(), "the sheets' surface goes with the lock");
        assert!(!sh.assistant.is_shown(), "the assistant panel goes with the lock");
        assert!(sh.login_open(), "the login window is up at once");
        sh.hw_type(keyboard::Out::Text("p".into()), 1010.0);
        sh.hw_type(keyboard::Out::Text("w".into()), 1020.0);
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(2), "typing shows dots");
    }

    /// An app's password sheet (or open or save panel, or alert) waits under the lock, as its app still expects an
    /// answer, but the lock screen takes the keys before it: they reach the login window, never the app's field.
    #[test]
    fn an_apps_password_sheet_under_the_lock_leaves_the_keys_to_the_login_window() {
        let mut sh = signed_in_desktop();
        let sheet = wifi_page::Sheet::form(&sh.ui, sh.w, 0.0, auth_sheet::prompt_for("Notes", "unlock a note", 0));
        sh.auth_sheet = Some(auth_sheet::AuthSheet { app: "Notes".into(), reason: "unlock a note".into(), attempt: 0, sheet });
        sh.lock_screen();
        sh.hw_type(keyboard::Out::Text("p".into()), 1010.0);
        sh.hw_type(keyboard::Out::Text("w".into()), 1020.0);
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(2), "the login window gets the typing");
        assert!(sh.auth_sheet.is_some(), "the app's sheet is still there for after the unlock");
    }

    /// An open menu is closed by the lock, and neither the menu-key path nor Overview's keys run over the lock screen
    /// (the binary sends keys there before `hw_type`).
    #[test]
    fn menus_and_overview_keys_never_run_over_the_lock_screen() {
        let mut sh = signed_in_desktop();
        sh.desktop_menu = Some(desktop_bar::Menu::File);
        sh.lock_screen();
        assert!(sh.desktop_menu.is_none(), "an open menu closes with the lock");
        assert_eq!(sh.hw_menu_key(28, true, 1010.0), None, "Return is not a menu key on the lock screen");
        let mods = overview::KeyMods { ctrl: false, cmd: false, shift: false };
        assert!(!sh.hw_overview_key(overview::KEY_F3, true, mods, false, 1010.0), "F3 does not open Overview over the lock");
        assert!(sh.login_open());
    }

    /// A locked desktop whose display went dark (idle) wakes on a key, and the key goes to the login window.
    #[test]
    fn a_key_wakes_a_dark_locked_desktop_into_the_login_window() {
        let mut sh = signed_in_desktop();
        sh.lock_screen();
        sh.screen_on = false;
        let fx = sh.hw_type(keyboard::Out::Text("p".into()), 1010.0);
        assert!(sh.screen_on, "the key turns the display on");
        assert!(fx.contains(&Effect::ScreenOn));
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(1), "and types into the login window");
    }

    /// A lock that finds no login service says so once (`login_missing_logged`) and opens the window on a later key once
    /// the service is there, instead of leaving the lock screen with no way in.
    #[test]
    fn a_lock_without_a_login_service_is_logged_and_retried_on_the_next_key() {
        let mut sh = signed_in_desktop();
        sh.login_backend = None;
        sh.lock_screen();
        assert!(!sh.login_open(), "no service, no window");
        assert!(sh.login_missing_logged, "the failed open is logged");
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Ok("tok2".into()))));
        sh.hw_type(keyboard::Out::Text("p".into()), 1010.0);
        assert!(sh.login_open(), "the next key opens the window");
        assert!(!sh.login_missing_logged);
        assert_eq!(sh.login.as_ref().map(login::Login::len), Some(1));
    }

    #[test]
    fn tablet_lock_screen_still_turns_the_screen_off() {
        let mut sh = shell();
        sh.login_backend = Some(std::sync::Arc::new(FixedLogin(login::Reply::Unavailable)));
        sh.screen = Screen::Home;
        sh.anim = None;
        let fx = sh.lock_screen();
        assert_eq!(sh.screen, Screen::Lock);
        assert!(!sh.screen_on, "a tablet lock is the power button: the screen goes off");
        assert_eq!(fx, vec![Effect::ScreenOff]);
    }

    #[test]
    fn with_a_login_backend_return_before_the_first_tick_does_not_unlock() {
        let mut sh = login_shell(login::Reply::Unavailable);
        assert!(sh.is_locked() && sh.login.is_none(), "no tick yet: the window is not up");
        sh.hw_type(keyboard::Out::Enter, 0.0);
        settle(&mut sh, 2000.0);
        assert!(sh.is_locked(), "Return alone must not unlock");
        assert!(sh.login.is_some(), "the login window came up instead");
    }

    #[test]
    fn with_a_login_backend_a_swipe_or_a_notification_tap_does_not_unlock() {
        let mut sh = login_shell(login::Reply::Unavailable);
        swipe(&mut sh, 600.0, 1800.0, 200.0, 0.0);
        settle(&mut sh, 2000.0);
        assert!(sh.is_locked(), "a swipe must not unlock");
        assert!(sh.lock_drag.is_none(), "and must not lift the lock screen over the desktop");
        sh.begin_unlock(0.0, 3000.0);
        settle(&mut sh, 5000.0);
        assert!(sh.is_locked(), "the notification path (begin_unlock) must not unlock");
    }

    #[test]
    fn with_a_login_backend_only_privd_s_token_unlocks() {
        let mut sh = login_shell(login::Reply::Ok("tok".into()));
        sh.hw_type(keyboard::Out::Text("pw".into()), 0.0);
        sh.hw_type(keyboard::Out::Enter, 10.0);
        for k in 0..200 {
            sh.tick(20.0 + k as f64 * 20.0);
            if !sh.is_locked() { break; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!sh.is_locked(), "a right password unlocks");
        assert_eq!(sh.session_token.as_deref(), Some("tok"));
    }

    /// In the touch form the lock screen is usable with fingers alone: a tap on the user picture raises the keyboard,
    /// its keys type into the password field and Return signs in.
    #[test]
    fn the_lock_screen_signs_in_with_touch_alone() {
        keyboard::fake_process(500.0);
        let mut sh = login_shell(login::Reply::Ok("tok".into()));
        sh.set_keyboard(false, 0.0);
        sh.anim = None;
        sh.login_ensure(0.0);
        sh.login.as_mut().unwrap().touch_keyboard = true;
        let picture = (600.0, 1920.0 * 0.62 + sh.ui.pt(30.0));
        tap(&mut sh, picture.0, picture.1, 100.0);
        assert!(sh.keyboard.shown(), "the tap raised the keyboard");
        assert!(sh.login_owns_keyboard());
        tap_key(&mut sh, "p", 300.0);
        tap_key(&mut sh, "w", 400.0);
        assert_eq!(sh.login.as_ref().map(|l| l.len()), Some(2), "keys type into the password field");
        keyboard::fake_typed(keyboard::Out::Text("x".into()));
        sh.tick(500.0);
        assert_eq!(sh.login.as_ref().map(|l| l.len()), Some(3), "keys from the keyboard process reach it too");
        tap_key(&mut sh, "delete", 600.0);
        assert_eq!(sh.login.as_ref().map(|l| l.len()), Some(2), "backspace deletes");
        tap_key(&mut sh, "return", 700.0);
        for k in 0..200 {
            sh.tick(800.0 + k as f64 * 20.0);
            if !sh.is_locked() { break; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!sh.is_locked(), "return signed in");
        assert_eq!(sh.session_token.as_deref(), Some("tok"));
        assert!(!sh.keyboard.shown(), "the keyboard goes down with the login window");
    }

    /// In the desktop form a click on the picture leaves the keyboard down.
    #[test]
    fn a_click_on_the_lock_screen_picture_keeps_the_keyboard_down_on_a_desktop() {
        keyboard::fake_process(500.0);
        let mut sh = login_shell(login::Reply::Unavailable);
        sh.set_keyboard(false, 0.0);
        sh.anim = None;
        sh.login_ensure(0.0);
        sh.login.as_mut().unwrap().touch_keyboard = false;
        let picture = (600.0, 1920.0 * 0.62 + sh.ui.pt(30.0));
        tap(&mut sh, picture.0, picture.1, 100.0);
        assert!(!sh.keyboard.shown());
    }

    /// Developer preview: `PREVIEW_DIR=DIR cargo test preview_login -- --ignored` writes login-dark.png and login-light.png
    /// (the boot login window over the lock screen, with a stub account).
    #[test]
    #[ignore]
    fn preview_login() {
        struct Stub;
        impl login::Backend for Stub {
            fn authenticate(&self, _: &str) -> login::Reply { login::Reply::Unavailable }
            fn account(&self) -> Option<login::AccountInfo> {
                Some(login::AccountInfo { full_name: "Ada Lovelace".into(), username: "ada".into(), has_password: true, locked_out_secs: 0, password_hint: String::new() })
            }
        }
        let _lock = crate::test_support::global_lock();
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        for (name, light) in [("dark", false), ("light", true)] {
            bifrost_render::appearance::jump_to(light);
            let mut sh = shell_sized(1800, 1100);
            sh.desk = Some(desk::Desk::new());
            sh.screen = Screen::Lock;
            sh.anim = None;
            sh.login_backend = Some(std::sync::Arc::new(Stub));
            sh.login_ensure(0.0);
            let (w, h) = (sh.w as usize, sh.h as usize);
            let mut img = Image::new(w, h);
            for k in 0..5 {
                sh.draw(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 }, 1000.0 + k as f64 * 100.0);
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("login-{name}.png")), crate::shell::screenshot::encode(&img)).unwrap();
        }
    }

    /// The first key after locking goes into the password field, even before a frame opened the login window
    /// (the shell idles after a lock, so no tick may have run). It was dropped, so every login after a lock failed once.
    #[test]
    fn the_first_key_after_a_lock_reaches_the_password_field() {
        struct Stub;
        impl login::Backend for Stub {
            fn authenticate(&self, _: &str) -> login::Reply { login::Reply::Unavailable }
            fn account(&self) -> Option<login::AccountInfo> {
                Some(login::AccountInfo { full_name: "Ada Lovelace".into(), username: "ada".into(), has_password: true, locked_out_secs: 0, password_hint: String::new() })
            }
        }
        let mut sh = shell();
        sh.screen = Screen::Lock;
        sh.anim = None;
        sh.login_backend = Some(std::sync::Arc::new(Stub));
        assert!(!sh.login_open(), "no tick yet: the window is not open");
        sh.hw_type(crate::shell::keyboard::Out::Text("a".into()), 1.0);
        sh.hw_type(crate::shell::keyboard::Out::Text("b".into()), 1.1);
        assert_eq!(sh.login.as_ref().map(|l| l.len()), Some(2), "both keys typed, the first one too");
    }

    #[test]
    fn cmd_tab_cycles_while_held_and_raises_the_selection_on_release() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);sh.desk_open(AppId::Files,1.);
        // Cmd+Tab is the switcher (all windows, most recent first); Cmd+` is `desk_cycle_window`
        sh.switcher_cycle(false);
        assert_eq!(sh.desk.as_ref().unwrap().cmdtab.as_ref().map(|c|c.1),Some(1));
        let (w,h)=(sh.w as usize,sh.h as usize);let mut buf=vec![0u32;w*h];
        sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut buf,y0:0},2.);
        sh.cmdtab_release(3.);
        assert!(sh.desk.as_ref().unwrap().cmdtab.is_none());
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w|w.app),Some(AppId::Settings));
    }

    /// The strip is chrome surface 13 above the windows, only while Cmd+Tab is open; it takes neither pointer nor keys.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_cmd_tab_strip_is_an_overlay_surface_only_while_it_is_open() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);sh.desk_open(AppId::Files,1.);
        let (w,h)=(sh.w as usize,sh.h as usize);let pixels=vec![0xff20_2020u32;w*h];
        let strip=|sh:&Shell|sh.desktop_chrome_frames(&pixels,w,h).into_iter().find(|f|f.id==13);
        assert!(strip(&sh).is_none(),"closed: no surface");
        assert!(sh.cmdtab_rect().is_none());
        sh.switcher_cycle(false);
        let f=strip(&sh).expect("the open strip is a chrome surface above the windows");
        assert_eq!(f.pixels.len(),f.width*f.height);
        assert_eq!(Some((f.x as usize,f.y as usize,f.width,f.height)),sh.cmdtab_rect());
        assert!(f.width<w&&f.height<h&&f.x>0&&f.y>0,"a centred capsule, not the whole screen");
        assert_eq!(f.input,Some([0,0,0,0]),"an empty input region: the pointer reaches what is below");
        if bifrost_render::glass::gpu() {
            let r=sh.desk_regions_now().unwrap();
            assert!(r.contains(&(f.x,f.y,f.width as i32,f.height as i32)),"the presenter keeps its region");
        }
        // the corners are transparent (the capsule's rounded ends), the middle is the strip
        assert_eq!(f.pixels[0]>>24,0);
        assert_eq!(f.pixels[(f.height/2)*f.width+f.width/2]>>24,0xff);
        // the selection moving keeps the surface; releasing removes it
        sh.switcher_cycle(false);
        assert!(strip(&sh).is_some());
        sh.cmdtab_release(3.);
        assert!(strip(&sh).is_none(),"the release drops the surface");
        // Esc cancels without a ghost, too
        sh.switcher_cycle(false);
        assert!(strip(&sh).is_some());
        sh.switcher_key(crate::shell::overview::KEY_ESC,crate::shell::overview::KeyMods::default());
        assert!(strip(&sh).is_none(),"Esc drops the surface");
    }

    #[test]
    fn pointer_over_traffic_lights_sets_hover() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Files,0.);
        let m=sh.desk_metrics();
        let r=sh.desk.as_ref().unwrap().wins[0].r;
        let (x,y)=((r.x+m.light_x+m.light/2) as f32,(r.y+m.title/2) as f32);
        sh.desktop_pointer(x,y,false,1.);
        assert_eq!(sh.desk.as_ref().unwrap().lights_hover,Some(sh.desk.as_ref().unwrap().wins[0].app));
        sh.desktop_pointer(x,(r.y+m.title+40) as f32,false,2.);
        assert_eq!(sh.desk.as_ref().unwrap().lights_hover,None);
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn status_items_sit_close_to_the_clock_and_never_overlap() {
        let mut sh = shell();
        sh.desk = Some(desk::Desk::new());
        sh.screen = Screen::Home;
        let b = sh.menu_buttons();
        let status: Vec<_> = b.iter().filter(|(m, ..)| matches!(m, desktop_bar::Menu::Power | desktop_bar::Menu::Network | desktop_bar::Menu::ControlCenter)).collect();
        assert!(!status.is_empty());
        let clock = sh.ui.measure(&sh.clock_text(), 12., Weight::Medium);
        let clock_left = sh.w as f32 - sh.ui.pt(16.) - clock;
        let power = status.iter().find(|(m, ..)| *m == desktop_bar::Menu::ControlCenter).unwrap();
        let gap = clock_left - (power.1 + power.2);
        assert!(gap >= 0.0 && gap <= sh.ui.pt(4.), "Control Center ends {gap} px before the clock");
        for w in status.windows(2) {
            assert!(w[0].1 + w[0].2 <= w[1].1 + 0.01, "status items overlap");
        }
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_desktop_without_a_battery_shows_no_battery_or_plug_in_the_menu_bar() {
        let mut sh = shell();
        sh.desk = Some(desk::Desk::new());
        sh.screen = Screen::Home;
        sh.info.desktop_network = Some(Default::default());
        sh.info.battery_present = true;
        assert!(sh.menu_buttons().iter().any(|b| b.0 == desktop_bar::Menu::Power), "a desktop with a battery shows its item");
        sh.info.battery_present = false;
        assert!(!sh.menu_buttons().iter().any(|b| b.0 == desktop_bar::Menu::Power), "no battery, no item (and no plug)");
    }

    #[allow(dead_code)]
    fn dock_centre(sh: &Shell, app: AppId) -> (f32, f32) {
        let m = sh.desk_metrics();
        let (list, _, icons) = sh.desk_dock(&m);
        let k = list.iter().position(|&i| i < apps().len() && apps()[i].id == app).expect("app in the dock");
        ((icons[k].x + icons[k].w / 2) as f32, (icons[k].y + icons[k].h / 2) as f32)
    }

    #[test]
    fn pressing_a_dock_icon_focuses_its_window_at_once_and_a_second_click_keeps_it() {
        let mut sh = shell();
        sh.desk = Some(desk::Desk::new());
        sh.screen = Screen::Home;
        sh.anim = None;
        let other = apps().iter().map(|a| a.id).find(|&a| a != AppId::Settings && a != AppId::Files && a != AppId::Browser).unwrap();
        sh.desk_open(AppId::Settings, 0.);
        sh.desk_open(other, 1.);
        assert_eq!(sh.screen, Screen::App(other));
        let (x, y) = dock_centre(&sh, AppId::Settings);
        sh.desk_event(Event::Down(x, y), 10.); // the press alone brings Settings forward
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        sh.desk_event(Event::Up(x, y), 20.);
        let w = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == AppId::Settings).unwrap();
        assert!(!w.minimized, "clicking the frontmost app's icon must not minimize it");
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
    }

    #[test]
    fn dragging_a_dock_icon_slides_its_neighbours_and_dropping_reorders_after_the_target() {
        let mut sh = shell();
        sh.desk = Some(desk::Desk::new());
        sh.screen = Screen::Home;
        sh.anim = None;
        let m = sh.desk_metrics();
        let (list, _, icons) = sh.desk_dock(&m);
        let (a, b) = (icons[0], icons[2]);
        let (ax, ay) = ((a.x + a.w / 2) as f32, (a.y + a.h / 2) as f32);
        let (bx, by) = ((b.x + b.w / 2) as f32, (b.y + b.h / 2) as f32);
        sh.desk_event(Event::Down(ax, ay), 0.);
        sh.desk_event(Event::Move(bx, by), 16.);
        let slide = sh.desk.as_ref().unwrap().dock_slide.borrow();
        assert!(slide.active(0.016), "neighbours start sliding");
        assert!(slide.offset(1, 10.0).0 < 0.0, "the icon between moves left to fill the gap");
        drop(slide);
        sh.desk_event(Event::Up(bx, by), 32.);
        assert!(!sh.desk.as_ref().unwrap().dock_slide.borrow().active(10.0), "motion state is cleared on drop");
        let pinned = &sh.desk.as_ref().unwrap().dock_preferences.pinned;
        let key = |i: usize| sh.dock_app_key(list[i]).unwrap();
        let (p0, p2) = (pinned.iter().position(|s| *s == key(0)), pinned.iter().position(|s| *s == key(2)));
        assert!(p0 > p2 && p0.is_some(), "dragged forward, it lands after the target: {pinned:?}");
    }

    #[test]
    fn the_performance_hud_draws_on_the_desktop() {
        let mut sh = shell();
        sh.desk = Some(desk::Desk::new());
        sh.screen = Screen::Home;
        sh.anim = None;
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut off = vec![0u32; w * h];
        sh.draw_desk(&mut Surface { w, h, stride: w, px: &mut off, y0: 0 }, 1.0);
        sh.perf.enabled = true;
        let mut on = vec![0u32; w * h];
        sh.draw_desk(&mut Surface { w, h, stride: w, px: &mut on, y0: 0 }, 1.0);
        assert!(sh.perf_drawn);
        let (x, y, pw, ph) = sh.perf_rect();
        let (cx, cy) = ((x + pw / 2.0) as usize, (y + ph / 2.0) as usize);
        assert_ne!(on[cy * w + cx], off[cy * w + cx], "the HUD box is painted");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn review_identical_foreign_windows_do_not_dirty_desktop() {
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.dirty=false;
        sh.wayland_foreign_windows(vec![]);assert!(!sh.dirty);
    }

    /// An idle second used to cost a whole desktop frame whenever the CPU or RAM reading changed the bar's text.
    #[test]
    fn a_changed_reading_alone_asks_for_a_bar_only_frame_and_anything_else_for_a_full_one() {
        let mut sh=shell();
        sh.dirty=false;sh.bar_only=false;sh.metrics_changed=true;
        sh.note_metrics_frame();
        assert!(sh.dirty&&sh.bar_only,"nothing else wanted a frame: only the bar's rows");
        assert!(!sh.metrics_changed,"the reading is settled");
        sh.dirty=true;sh.bar_only=false;sh.metrics_changed=true;
        sh.note_metrics_frame();
        assert!(sh.dirty&&!sh.bar_only,"something else is dirty: the ordinary frame");
        sh.dirty=false;sh.bar_only=false;sh.metrics_changed=false;
        sh.note_metrics_frame();
        assert!(!sh.dirty&&!sh.bar_only,"no new reading, no frame");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_bar_only_frame_repaints_only_the_menu_bar_band() {
        let _lock=crate::test_support::global_lock();
        let saved=std::env::var_os("SPRINGBOARD_WAYLAND");std::env::set_var("SPRINGBOARD_WAYLAND","1");
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.drawn=Drawn::App;
        let mut pixels=vec![0;sh.w as usize*sh.h as usize];
        let (w,h)=(sh.w as usize,sh.h as usize);
        // a bar-only repaint always follows a full desktop frame (it is what sets desk_drawn)
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},0.);
        let band=sh.desktop_bar_height().min(h);
        let mut expected=pixels.clone();
        sh.draw_desktop_background(&mut Surface{w,h,stride:w,px:&mut expected,y0:0}.rows(0,band));
        sh.bar_only=true;
        let damage=sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},1000.);
        assert!(!damage.is_all());assert!(damage.pixels()<=w*band,"nothing below the bar is repainted");
        assert!(pixels[..w*band]==expected[..w*band],"the bar's rows equal a desktop repaint of them");
        if let Some(v)=saved {std::env::set_var("SPRINGBOARD_WAYLAND",v)} else {std::env::remove_var("SPRINGBOARD_WAYLAND")};
    }

    /// A window that windows in front cover keeps no cache entry on purpose; that must not make every partial desktop
    /// frame (the bar-only repaint of the once-a-second readings) fall back to a whole desktop frame.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_covered_window_does_not_stop_the_partial_frames() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let m=sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m,AppId::Settings,(600,400));
        let id=desk::win_id(AppId::Settings).unwrap();
        assert!(!sh.retain_wayland_windows(),"a window that was never drawn needs a full desktop frame");
        sh.desk.as_mut().unwrap().hidden_ids.push(id);
        assert!(sh.retain_wayland_windows(),"one the last frame skipped because it is covered does not");
        assert_eq!(sh.take_wayland_windows().len(),1,"its frame is still published, without pixels");
    }

    /// A new reading changes one label: the frame reports that label's rectangle as damage, not the bar's whole width.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_bar_only_frame_reports_damage_for_the_changed_items_alone() {
        let _lock=crate::test_support::global_lock();
        let saved=std::env::var_os("SPRINGBOARD_WAYLAND");std::env::set_var("SPRINGBOARD_WAYLAND","1");
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.drawn=Drawn::App;
        let (w,h)=(sh.w as usize,sh.h as usize);
        let mut pixels=vec![0;w*h];
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},0.);
        let band=sh.desktop_bar_height().min(h);
        // a changed reading asks for the frame (`note_metrics_frame` sets `dirty` with `bar_only`); the keyboard client of the
        // test shell still slides out until a tick, which only the settled desktop skips
        sh.keyboard.tick(1.0e6);
        sh.dirty=true;sh.bar_only=true;
        sh.bar_rects=vec![(w as i32/2,0,100,band as i32)];
        let damage=sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},1000.);
        assert!(!damage.is_all());
        assert_eq!(damage.pixels(),100*band,"only the changed item's rectangle");
        assert!(sh.bar_rects.is_empty(),"the rectangles belong to one frame");
        if let Some(v)=saved {std::env::set_var("SPRINGBOARD_WAYLAND",v)} else {std::env::remove_var("SPRINGBOARD_WAYLAND")};
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn review_performance_overlay_repaints_only_its_desktop_band() {
        let _lock=crate::test_support::global_lock();
        let saved=std::env::var_os("SPRINGBOARD_WAYLAND");std::env::set_var("SPRINGBOARD_WAYLAND","1");
        let mut sh=shell();sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;sh.perf.enabled=true;sh.drawn=Drawn::App;
        let mut pixels=vec![0;sh.w as usize*sh.h as usize];
        let (w,h)=(sh.w as usize,sh.h as usize);
        // a perf-only repaint always follows a full desktop frame (it is what sets desk_drawn)
        sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},0.);
        sh.draw_desktop_background(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0});
        sh.draw_perf(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0});
        sh.perf.fps=60.;let mut expected=pixels.clone();
        sh.draw_desktop_background(&mut Surface{w,h,stride:w,px:&mut expected,y0:0});
        sh.draw_perf(&mut Surface{w,h,stride:w,px:&mut expected,y0:0});
        // as `tick` sets them when only the HUD's reading changed: the perf-only frame, and dirty
        sh.perf_only=true;sh.dirty=true;
        let damage=sh.draw(&mut Surface{w,h,stride:w,px:&mut pixels,y0:0},1000.);
        assert!(!damage.is_all());assert!(damage.pixels()<w*h/10);
        assert!(pixels==expected,"partial HUD repaint differs from full desktop");
        if let Some(v)=saved {std::env::set_var("SPRINGBOARD_WAYLAND",v)} else {std::env::remove_var("SPRINGBOARD_WAYLAND")};
    }

    #[cfg(test)]
    pub(crate) fn swipe(sh: &mut Shell, x: f32, y0: f32, y1: f32, t: f64) -> Vec<Effect> {
        let mut fx = sh.event(Event::Down(x, y0), t);
        for i in 1..=10 {
            fx.extend(sh.event(Event::Move(x, y0 + (y1 - y0) * i as f32 / 10.0), t + i as f64 * 10.0));
        }
        fx.extend(sh.event(Event::Up(x, y1), t + 110.0));
        fx
    }

    #[cfg(test)]
    pub(crate) fn settle(sh: &mut Shell, t: f64) {
        sh.tick(t + 1000.0);
    }

    #[cfg(test)]
    pub(crate) fn unlock(sh: &mut Shell) {
        swipe(sh, 600.0, 1800.0, 900.0, 0.0);
        settle(sh, 200.0);
    }

    /// A horizontal swipe at height `y` from `x0` to `x1`.
    #[cfg(test)]
    pub(crate) fn hswipe(sh: &mut Shell, y: f32, x0: f32, x1: f32, t: f64) {
        sh.event(Event::Down(x0, y), t);
        for i in 1..=10 {
            sh.event(Event::Move(x0 + (x1 - x0) * i as f32 / 10.0, y), t + i as f64 * 10.0);
        }
        sh.event(Event::Up(x1, y), t + 110.0);
    }

    #[test]
    fn swipe_right_from_the_first_page_opens_the_today_view_which_refreshes_and_edits() {
        let mut sh = shell();
        unlock(&mut sh);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t).clone();
        draw(&mut sh, 2500.0);
        hswipe(&mut sh, 900.0, 100.0, 1000.0, 3000.0);
        assert!(sh.needs_frame());
        assert!(draw(&mut sh, 3120.0).is_all(), "the sliding page frosts the whole frame");
        settle(&mut sh, 3110.0);
        assert!(sh.pager.leading());
        assert!(sh.describe().contains("today=true"));
        draw(&mut sh, 4200.0);
        assert!(draw(&mut sh, 4216.0).is_empty(), "settled page draws nothing new");
        // information changes re-render the tiles and repaint the page
        let g = sh.today.gen();
        let mut next = sh.info.clone();
        next.battery -= 5;
        sh.set_info(next);
        sh.tick(4300.0);
        assert!(sh.today.gen() > g, "battery change refreshed the widgets");
        assert!(draw(&mut sh, 4300.0).is_all());
        // and so does the next minute
        let g = sh.today.gen();
        sh.set_wall(1_000_000, 0, 4300.0);
        sh.tick(4300.0 + 60_000.0);
        assert!(sh.today.gen() > g, "a minute later the clocks move");
        // taps: the search bar opens quick search (a modal overlay that takes the next touches), so it is dismissed
        // before Edit, which enters edit mode
        let sr = sh.today.search_rect();
        tap(&mut sh, sr.x as f32 + 50.0, sr.y as f32 + 10.0, 70_000.0);
        assert_eq!(sh.screen, Screen::Home);
        assert!(sh.quicksearch.active(), "the search bar opens quick search");
        sh.quicksearch.hide();
        let er = sh.today.edit_rect();
        tap(&mut sh, er.x as f32 + er.w as f32 / 2.0, er.y as f32 + er.h as f32 / 2.0, 71_000.0);
        assert!(sh.today.editing());
        assert!(sh.describe().contains("today_editing=true"));
        // a vertical drag on the page never turns the pager
        swipe(&mut sh, 600.0, 1200.0, 900.0, 72_000.0);
        settle(&mut sh, 72_110.0);
        assert!(sh.pager.leading());
        // swipe left: back to the first page, edit mode ends
        hswipe(&mut sh, 900.0, 1000.0, 100.0, 73_000.0);
        settle(&mut sh, 73_110.0);
        assert!(!sh.pager.leading() && sh.pager.page() == 0);
        assert!(!sh.today.editing(), "leaving the page ends editing");
        assert!(sh.describe().contains("today=false"));
    }

    #[test]
    fn today_view_opens_deildu_on_the_remembered_title() {
        use crate::deildu::http::tests::Fake;
        let mut sh = shell();
        sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-today-deildu").into();
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
        sh.deildu_transport = Fake::new(vec![]);
        unlock(&mut sh);
        sh.today.note_played(&Played { id: "12".into(), title: "Film".into(), url: "https://www.deildu.net/details.php?id=12".into() });
        sh.tick(2500.0);
        hswipe(&mut sh, 900.0, 100.0, 1000.0, 3000.0);
        settle(&mut sh, 3110.0);
        let r = sh.today.placed().iter().find(|p| p.kind == widgets::Kind::Deildu).unwrap().rect;
        tap(&mut sh, r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0, 5000.0);
        settle(&mut sh, 5030.0);
        assert_eq!(sh.screen, Screen::App(AppId::Deildu));
        assert!(sh.describe().contains("deildu_screen=detail"), "{}", sh.describe());
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
    }

    #[test]
    fn lock_screen_swipes_right_into_the_today_view_and_still_unlocks() {
        let mut sh = shell();
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t).clone();
        draw(&mut sh, 0.0);
        hswipe(&mut sh, 900.0, 100.0, 1000.0, 1000.0);
        assert!(draw(&mut sh, 1050.0).is_all());
        settle(&mut sh, 1110.0);
        assert!(sh.is_locked() && sh.lock_pager.leading());
        assert!(sh.describe().contains("today=true"));
        draw(&mut sh, 2200.0);
        assert!(draw(&mut sh, 2216.0).is_empty());
        // on the page a vertical drag scrolls instead of unlocking
        swipe(&mut sh, 600.0, 1800.0, 900.0, 3000.0);
        settle(&mut sh, 3110.0);
        assert!(sh.is_locked());
        // the Deildu tile does nothing on the lock screen
        sh.today.note_played(&Played { id: "1".into(), title: "T".into(), url: String::new() });
        sh.tick(3200.0);
        let r = sh.today.placed().iter().find(|p| p.kind == widgets::Kind::Deildu).unwrap().rect;
        tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, 3300.0);
        settle(&mut sh, 3330.0);
        assert!(sh.is_locked());
        // back to the lock screen, then unlock as usual; the home pager starts on page 0
        hswipe(&mut sh, 900.0, 1000.0, 100.0, 4000.0);
        settle(&mut sh, 4110.0);
        assert!(!sh.lock_pager.leading());
        unlock(&mut sh);
        assert_eq!(sh.screen, Screen::Home);
        assert!(!sh.pager.leading() && !sh.lock_pager.leading());
        // sleeping drops the page
        hswipe(&mut sh, 900.0, 100.0, 1000.0, 6000.0);
        settle(&mut sh, 6110.0);
        assert!(sh.pager.leading());
        sh.event(Event::Power, 7000.0);
        assert!(!sh.pager.leading() && sh.is_locked());
    }

    #[test]
    fn starts_locked_and_swipe_up_unlocks() {
        let mut sh = shell();
        assert!(sh.is_locked());
        swipe(&mut sh, 600.0, 1800.0, 1750.0, 0.0);
        settle(&mut sh, 200.0);
        assert!(sh.is_locked(), "a short swipe springs back");
        unlock(&mut sh);
        assert_eq!(sh.screen, Screen::Home);
    }

    #[test]
    fn tapping_a_dock_icon_opens_the_app_and_swipe_up_goes_home() {
        let mut sh = shell();
        unlock(&mut sh);
        let r = sh.layout.dock_icons[0];
        let (x, y) = (r.x as f32 + 10.0, r.y as f32 + 10.0);
        sh.event(Event::Down(x, y), 5000.0);
        sh.event(Event::Up(x, y), 5050.0);
        settle(&mut sh, 5050.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        swipe(&mut sh, 600.0, 1915.0, 1500.0, 8000.0);
        settle(&mut sh, 8200.0);
        assert_eq!(sh.screen, Screen::Home);
    }

    /// Taps the Settings row labelled `label` and lets the page slide in.
    #[cfg(test)]
    fn tap_settings_row(sh: &mut Shell, label: &str, t: f64) -> Vec<Effect> {
        let st = sh.settings.as_ref().unwrap();
        // root rows sit in the sidebar of the bifrost split layout
        let (x, y) = st.row_centre(&sh.ui, label).or_else(|| st.sidebar_centre(&sh.ui, label)).unwrap_or_else(|| panic!("no row {label}"));
        let mut fx = sh.event(Event::Down(x, y), t);
        fx.extend(sh.event(Event::Up(x, y), t + 20.0));
        sh.tick(t + 1000.0);
        fx
    }

    #[test]
    fn brightness_slider_emits_effect() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.open(AppId::Settings, 3000.0);
        settle(&mut sh, 3000.0);
        tap_settings_row(&mut sh, "Display & Brightness", 5000.0);
        let st = sh.settings.as_ref().unwrap();
        let (tx, ty, tw) = st.list().unwrap().track(&sh.ui, 0, 0);
        let tx = tx + st.pane_x();
        let fx = sh.event(Event::Down(tx + tw * 0.25, ty), 9000.0);
        assert_eq!(fx, vec![Effect::Brightness(0.25)]);
        let fx = sh.event(Event::Move(tx + tw * 0.75, ty + 300.0), 9010.0);
        assert_eq!(fx, vec![Effect::Brightness(0.75)]);
        assert_eq!(sh.info.brightness, 0.75);
        sh.event(Event::Up(tx + tw * 0.75, ty), 9020.0);
        assert!(sh.event(Event::Move(tx, ty + 600.0), 9030.0).is_empty());
    }

    #[test]
    fn settings_keeps_its_page_across_info_refreshes() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.open(AppId::Settings, 3000.0);
        settle(&mut sh, 3000.0);
        assert_eq!(tap_settings_row(&mut sh, "Wi-Fi", 9000.0), vec![Effect::Wifi(WifiAction::Scan)]);
        let mut next = sh.info.clone();
        next.min += 1;
        next.networks = vec![Network { ssid: "Cafe".into(), bars: 2, secure: false, saved: false }];
        sh.set_info(next);
        assert_eq!(sh.settings.as_ref().unwrap().page(), pages::PageId::Wifi);
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1e9);
    }

    #[test]
    fn settings_options_reach_the_shell() {
        let mut sh = shell();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-options");
        let _ = std::fs::remove_dir_all(dir);
        let fx = sh.set_data_dir(dir, 0.0);
        assert!(fx.contains(&Effect::AutoLock(0)), "{fx:?}");
        unlock(&mut sh);
        sh.open(AppId::Settings, 3000.0);
        settle(&mut sh, 3000.0);
        tap_settings_row(&mut sh, "Display & Brightness", 5000.0);
        tap_settings_row(&mut sh, "Auto-Lock", 6000.0);
        assert_eq!(tap_settings_row(&mut sh, "Never", 7000.0), vec![Effect::AutoLock(0)]);
        assert_eq!(sh.idle_off_ms, 0.0);
        sh.event(Event::Down(600.0, 5.0), 8000.0);
        sh.event(Event::Up(600.0, 5.0), 8001.0);
        assert!(sh.tick(8001.0 + 1e7).is_empty(), "Never means never");
        assert_eq!(Prefs::load(format!("{dir}/settings.ini")).get("auto_lock_ms"), Some("0"));
        // back to Display: the text size slider rescales the Ui
        let (x, y) = (sh.settings.as_ref().unwrap().pane_x() + sh.ui.pt(40.0), sh.ui.pt(42.0));
        sh.event(Event::Down(x, y), 9000.0);
        sh.event(Event::Up(x, y), 9020.0);
        sh.tick(10_000.0);
        let st = sh.settings.as_ref().unwrap();
        let (g, i) = st.list().unwrap().find("Bold Text").map(|(g, i)| (g, i - 1)).unwrap();
        let (tx, ty, tw) = st.list().unwrap().track(&sh.ui, g, i);
        let tx = tx + st.pane_x();
        sh.event(Event::Down(tx + tw, ty), 11_000.0);
        sh.event(Event::Up(tx + tw, ty), 11_050.0);
        assert_eq!(sh.ui.text_scale, ui::TEXT_SCALE_MAX);
        tap_settings_row(&mut sh, "Bold Text", 12_000.0);
        assert!(sh.ui.bold);
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 13_000.0);
        // a fresh shell reads the same options back from the file
        let mut again = shell();
        again.set_data_dir(dir, 0.0);
        assert_eq!(again.ui.text_scale, ui::TEXT_SCALE_MAX);
        assert!(again.ui.bold && again.idle_off_ms == 0.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn power_toggles_screen_and_relocks() {
        let mut sh = shell();
        unlock(&mut sh);
        assert_eq!(sh.event(Event::Power, 1.0), vec![Effect::ScreenOff]);
        assert!(!sh.screen_on() && sh.is_locked());
        assert!(!sh.needs_frame());
        assert_eq!(sh.event(Event::Power, 2.0), vec![Effect::ScreenOn]);
        assert!(sh.needs_frame());
    }

    #[test]
    fn tap_or_wake_request_turns_the_dark_screen_on() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.event(Event::Power, 1.0);
        assert!(!sh.screen_on());
        // a slow press is not a tap
        sh.event(Event::Down(100.0, 100.0), 10.0);
        assert!(sh.event(Event::Up(100.0, 100.0), 600.0).is_empty());
        // a drag is not a tap
        sh.event(Event::Down(100.0, 100.0), 700.0);
        sh.event(Event::Move(300.0, 100.0), 750.0);
        assert!(sh.event(Event::Up(300.0, 100.0), 800.0).is_empty());
        assert!(!sh.screen_on());
        sh.event(Event::Down(100.0, 100.0), 900.0);
        assert_eq!(sh.event(Event::Up(105.0, 98.0), 1000.0), vec![Effect::ScreenOn]);
        assert!(sh.screen_on() && sh.is_locked());
        // Wake never turns the screen off
        assert!(sh.event(Event::Wake, 1100.0).is_empty());
        assert!(sh.screen_on());
        sh.event(Event::Power, 1200.0);
        assert_eq!(sh.event(Event::Wake, 1300.0), vec![Effect::ScreenOn]);
    }

    #[test]
    fn idle_turns_screen_off() {
        let mut sh = shell();
        sh.idle_off_ms = 120_000.0; // the default is Never
        sh.event(Event::Down(1.0, 1.0), 1000.0);
        sh.event(Event::Up(1.0, 1.0), 1001.0);
        assert!(sh.tick(60_000.0).is_empty());
        assert_eq!(sh.tick(1001.0 + sh.idle_off_ms + 1.0), vec![Effect::ScreenOff]);
    }

    #[test]
    fn draws_every_screen() {
        let mut sh = shell();
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell| {
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1e9);
        };
        draw(&mut sh);
        unlock(&mut sh);
        draw(&mut sh);
        sh.term_cmd = &["/bin/sh"];
        sh.deildu_transport = crate::deildu::http::tests::Fake::new(vec![]);
        sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-draws").into();
        sh.data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-draws").into();
        sh.photo_dirs = vec![sh.data_dir.join("Pictures")];
        for app in apps().iter().map(|a| a.id) {
        sh.files_locations = vec![files::fs::Location { name: "Tmp", path: concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-draws/files").into(), glyph: files::fs::Glyph::Computer }];
            sh.open(app, 0.0);
            settle(&mut sh, 0.0);
            draw(&mut sh);
            sh.go_home(0.0);
            settle(&mut sh, 0.0);
        }
    }

    #[test]
    fn new_apps_launch_from_icons_and_resume_from_switcher() {
        let mut sh = shell();
        sh.data_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-new-apps"));
        sh.set_wall(1_791_072_000, 0, 0.0);
        sh.act(Action::OpenCalendar, 0.0);
        assert!(sh.is_locked());
        unlock(&mut sh);
        let mut pixels = vec![0u32; 1200 * 1920];
        for (i, app) in [AppId::Browser].into_iter().enumerate() {
            let t = 3000.0 + i as f64 * 5000.0;
            let r = sh.icon_rect(app);
            let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
            sh.event(Event::Down(x, y), t);
            sh.event(Event::Up(x, y), t + 40.0);
            settle(&mut sh, t + 40.0);
            assert_eq!(sh.screen, Screen::App(app), "home icon launches {app:?}");
            assert!(sh.describe().contains(&format!("screen=app:{}", format!("{app:?}").to_lowercase())));
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut pixels, y0: 0 }, t + 1100.0);
            sh.go_home(t + 1200.0);
            settle(&mut sh, t + 1200.0);
            assert!(sh.switcher.apps().contains(&app), "{app:?} has a switcher card");
            sh.open(app, t + 2300.0);
            settle(&mut sh, t + 2300.0);
            assert_eq!(sh.screen, Screen::App(app));
            sh.go_home(t + 3400.0);
            settle(&mut sh, t + 3400.0);
            sh.close_app(app);
            assert!(!sh.switcher.apps().contains(&app));
        }
        assert!(sh.browser.is_none());
    }

    #[test]
    fn quicksearch_finds_the_new_apps() {
        for (name, app) in [("Browser", AppId::Browser)] {
            let groups = quicksearch::results(name, &[]);
            assert_eq!(groups[0].hits.first(), Some(&quicksearch::Hit::App(app)));
        }
    }

    #[test]
    fn assistive_menu_touches_do_not_reach_the_open_app() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.open(AppId::Files, 3000.0);
        settle(&mut sh, 3000.0);
        sh.prefs.set("assistive_touch", "1").unwrap();
        sh.apply_options();
        let (x, y, w, h) = sh.assistive_touch.button_rect(&sh.ui, sh.w, sh.h);
        tap(&mut sh, x + w / 2.0, y + h / 2.0, 5000.0);
        assert!(sh.assistive_touch.shown());
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        let (x, y, w, h) = assistive_touch::AssistiveTouch::action_rect(&sh.ui, sh.w, sh.h, assistive_touch::Action::Home);
        tap(&mut sh, x + w / 2.0, y + h / 2.0, 7000.0);
        settle(&mut sh, 7000.0);
        assert!(!sh.assistive_touch.shown());
        assert_eq!(sh.screen, Screen::Home);
        assert!(sh.switcher.apps().contains(&AppId::Files));
    }

    #[test]
    fn assistive_actions_open_real_shell_screens_and_lock() {
        use assistive_touch::Action as Assistive;
        let mut sh = shell();
        assert!(!sh.assistive_visible(), "the floating control starts disabled");
        sh.prefs.set("assistive_touch", "1").unwrap();
        sh.apply_options();
        assert!(!sh.assistive_visible(), "the floating control stays hidden while locked");
        unlock(&mut sh);
        assert!(sh.assistive_visible());
        sh.assistive_action(Assistive::ControlCenter, 3000.0);
        settle(&mut sh, 3000.0);
        assert!(sh.cc_active());
        sh.assistive_action(Assistive::NotificationCenter, 5000.0);
        settle(&mut sh, 5000.0);
        assert!(sh.center_active() && !sh.cc_active());
        sh.assistive_action(Assistive::Home, 7000.0);
        assert_eq!(sh.screen, Screen::Home);
        assert!(!sh.center_active());
        sh.open(AppId::Files, 8000.0);
        settle(&mut sh, 8000.0);
        sh.assistive_action(Assistive::AppSwitcher, 10000.0);
        settle(&mut sh, 10000.0);
        assert_eq!(sh.screen, Screen::Switcher);
        assert!(sh.switcher.apps().contains(&AppId::Files));
        sh.assistive_action(Assistive::Home, 12000.0);
        assert_eq!(sh.screen, Screen::Home);
        assert_eq!(sh.assistive_action(Assistive::Lock, 13000.0), vec![Effect::ScreenOff]);
        assert!(!sh.screen_on() && sh.is_locked() && !sh.assistive_visible());
    }

    #[test]
    fn assistive_overlay_repaints_after_drag_and_disable() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.prefs.set("assistive_touch", "1").unwrap();
        sh.apply_options();
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell| {
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 3000.0).clone()
        };
        assert!(draw(&mut sh).is_all());
        sh.prefs.set("assistive_touch", "0").unwrap();
        sh.apply_options();
        assert!(draw(&mut sh).is_all(), "removing the floating control erases its last frame");
        assert!(draw(&mut sh).is_empty(), "ordinary damage tracking resumes when disabled");
    }

    #[test]
    fn frames_report_only_what_changed() {
        let mut sh = shell();
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t).clone();
        assert!(draw(&mut sh, 0.0).is_all(), "first lock frame");
        assert!(draw(&mut sh, 16.0).is_empty(), "lock screen unchanged");
        let mut next = sh.info.clone();
        next.min += 1;
        sh.set_info(next.clone());
        assert!(draw(&mut sh, 32.0).is_all(), "the lock clock is part of the whole");

        unlock(&mut sh);
        assert!(draw(&mut sh, 2000.0).is_all(), "first home frame");
        assert!(draw(&mut sh, 2016.0).is_empty(), "home unchanged");
        next.battery -= 1;
        sh.set_info(next.clone());
        let d = draw(&mut sh, 2032.0);
        assert_eq!(d.areas().len(), 1);
        let a = d.areas()[0];
        assert_eq!((a.x0, a.y0, a.x1, a.y1), (0, 0, 1200, sh.layout.status_h as usize), "status change redraws the status strip");
        next.model = "other".into();
        sh.set_info(next);
        assert!(draw(&mut sh, 2048.0).is_empty(), "info the status bar does not show changes nothing");

        let r = sh.layout.dock_icons[1];
        sh.event(Event::Down(r.x as f32 + 5.0, r.y as f32 + 5.0), 5000.0);
        let d = draw(&mut sh, 5000.0);
        assert_eq!(d.areas().len(), 1, "pressing an icon dirties that icon: {:?}", d.areas());
        sh.event(Event::Up(r.x as f32 + 5.0, r.y as f32 + 5.0), 5050.0);
        sh.tick(5100.0);
        let d = draw(&mut sh, 5100.0);
        assert!(!d.is_all() && !d.is_empty(), "the launch card grows from the icon: {:?}", d.areas());
        settle(&mut sh, 5100.0);
        assert!(draw(&mut sh, 7000.0).is_all(), "apps repaint in full");
    }

    #[test]
        fn set_wallpaper_rebuilds_the_dock_backdrop_once() {
        let mut sh = shell();
        let before = sh.dock_bg.clone().unwrap();
        let px: &'static [u32] = Box::leak(vec![rgba(200, 40, 40, 255); 1200 * 1920].into_boxed_slice());
        sh.dirty = false;
        sh.set_wallpaper(View { w: 1200, h: 1920, px });
        assert!(sh.dirty);
        let after = sh.dock_bg.clone().unwrap();
        assert_ne!(before, after);
        assert!(after.px[after.w * after.h / 2 + after.w / 2] & 0xff > 40, "backdrop takes the new wallpaper's red");
        assert_eq!(sh.wallpaper().px[0], rgba(200, 40, 40, 255));
        sh.set_wallpapers(vec![Wallpaper { id: "x".into(), name: "X".into(), thumb: Image::new(160, 256) }]);
        assert_eq!(sh.wallpapers.len(), 1);
    }

    #[test]
    fn info_change_rerenders_only_on_difference() {
        let mut sh = shell();
        sh.dirty = false;
        let same = sh.info.clone();
        sh.set_info(same);
        assert!(!sh.dirty);
        let mut next = sh.info.clone();
        next.min += 1;
        sh.set_info(next);
        assert!(sh.dirty);
    }

    /// Unlocked shell with the Terminal open running `cmd`.
    #[cfg(test)]
    pub(super) fn terminal(cmd: &'static [&'static str]) -> Shell {
        let mut sh = shell();
        unlock(&mut sh);
        sh.term_cmd = cmd;
        sh.open(AppId::Terminal, 1000.0);
        settle(&mut sh, 1000.0);
        sh
    }

    #[cfg(test)]
    fn screen(sh: &Shell) -> String {
        sh.term.as_ref().map(|t| t.viewport_text()).unwrap_or_default()
    }

    /// Feeds pty output through the shell until `done` holds (or 5 s pass).
    #[cfg(test)]
    pub(super) fn pump_until(sh: &mut Shell, done: impl Fn(&Shell) -> bool) {
        let start = std::time::Instant::now();
        while !done(sh) && start.elapsed().as_secs() < 15 {
            let fds = sh.poll_fds();
            if crate::bare::sys::wait(&fds, 50).contains(&true) {
                sh.pump();
            }
        }
    }

    #[cfg(test)]
    fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) {
        sh.event(Event::Down(x, y), t);
        sh.event(Event::Up(x, y), t + 30.0);
    }

    #[cfg(test)]
    fn tap_key(sh: &mut Shell, label: &str, t: f64) {
        let (x, y) = sh.keyboard.centre(label).unwrap_or_else(|| panic!("no key {label}"));
        tap(sh, x, sh.kb_top() + y, t);
    }

    #[test]
    fn terminal_sits_in_the_dock_and_deildu_on_the_grid() {
        let names: Vec<_> = apps()[..IN_DOCK].iter().map(|a| a.name).collect();
        assert_eq!(names, ["Settings", "Term"]);
        let term = apps().iter().find(|a| a.id == AppId::Terminal).unwrap();
        assert_eq!(term.icon, "is.olibuijr.term");
        let grid: Vec<_> = apps()[IN_DOCK..].iter().map(|a| (a.name, a.icon)).collect();
        assert_eq!(grid, [("Deildu", "is.olibuijr.deildu"), ("Photos", "bifrost.photos"), ("Files", "bifrost.files"), ("Camera", "bifrost.camera"), ("Browser", "is.olibuijr.browser"), ("Magnifier", "bifrost.magnifier"), ("App Store", "bifrost.appstore")], "the stock apps that moved to packages are no longer built in");
    }

    #[test]
    fn deildu_opens_from_the_grid_pumps_jobs_and_keeps_running_at_home() {
        use crate::deildu::http::tests::{ok, Fake};
        let mut sh = shell();
        sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-deildu").into();
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
        let fake = Fake::new(vec![&ok("<html></html>")]);
        sh.deildu_transport = fake;
        unlock(&mut sh);
        let r = sh.layout.cells[0];
        tap(&mut sh, r.x as f32 + 10.0, r.y as f32 + 10.0, 5000.0);
        settle(&mut sh, 5050.0);
        assert_eq!(sh.screen, Screen::App(AppId::Deildu));
        assert_eq!(sh.app_poll_fds().len(), 1, "the job pipe is polled");
        sh.dirty = false;
        pump_until(&mut sh, |sh| sh.dirty);
        assert!(sh.dirty, "a finished job repaints the open app");
        assert!(sh.describe().contains("deildu_items=0"));
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1e9);
        sh.event(Event::Down(600.0, 900.0), 6000.0);
        sh.event(Event::Up(600.0, 900.0), 6030.0);
        swipe(&mut sh, 600.0, 1915.0, 1500.0, 8000.0);
        settle(&mut sh, 8200.0);
        assert_eq!(sh.screen, Screen::Home);
        assert!(sh.deildu.is_some() && sh.app_poll_fds().len() == 1, "still alive at home");
        let _ = std::fs::remove_dir_all(&sh.deildu_dir);
    }

    #[test]
    fn terminal_area_sits_between_status_bar_and_keyboard() {
        let mut sh = terminal(&["/bin/sh"]);
        let top = sh.ui.pt(scene::STATUS_PT + terminal::HEADER_PT).round() as i32;
        assert!(sh.keyboard.shown());
        assert_eq!(sh.kb_top(), sh.h as f32 - home_zone(&sh.ui) - sh.keyboard.h());
        assert_eq!(sh.term_area(), (0, top, sh.w, sh.kb_top() as i32 - top));
        tap_key(&mut sh, "hide", 2000.0);
        assert!(!sh.keyboard.shown());
        assert!(sh.needs_frame() && sh.keyboard.animating(), "the keyboard slides out");
        let bottom = (sh.h as f32 - home_zone(&sh.ui)) as i32;
        assert_eq!(sh.term_area(), (0, top, sh.w, bottom - top), "the terminal grows at once");
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 2100.0);
        sh.tick(2300.0);
        assert!(!sh.keyboard.animating());
        tap(&mut sh, 600.0, 600.0, 3000.0);
        assert!(sh.keyboard.shown(), "tapping the terminal brings the keyboard back");
    }

    #[test]
    fn key_clicks_follow_the_keyboard_clicks_pref() {
        let mut sh = shell();
        sh.set_keyboard(true, 1000.0);
        sh.tick(3000.0);
        sh.keyboard.take_clicks();
        keyboard::fake_click(kb_clicks::Click::Key);
        assert!(sh.tick(4100.0).contains(&Effect::KeyClick(kb_clicks::Click::Key)), "click asked for while the pref is on");
        sh.prefs.set("keyboard_clicks", "0").unwrap();
        keyboard::fake_click(kb_clicks::Click::Key);
        assert!(!sh.tick(5100.0).iter().any(|e| matches!(e, Effect::KeyClick(_))), "no click when Keyboard Clicks is off");
    }

    #[test]
    fn the_terminal_bell_plays_the_alert_sound() {
        let mut sh = terminal(&["/bin/sh", "-c", "printf '\\a'; read x"]);
        sh.cc.volume = 0.5;
        pump_until(&mut sh, |sh| sh.bell.pending());
        let want = sh.options().alert_sound;
        let fx = sh.tick(5000.0);
        assert!(fx.contains(&Effect::AlertSound(want)), "{fx:?}");
        assert_eq!(sh.bell.flash_alpha(5000.0), 0, "no flash while the sound can play");
    }

    #[test]
    fn a_muted_terminal_bell_flashes_instead_of_sounding() {
        let mut sh = terminal(&["/bin/sh", "-c", "printf '\\a'; read x"]);
        sh.cc.volume = 0.0;
        pump_until(&mut sh, |sh| sh.bell.pending());
        let fx = sh.tick(5000.0);
        assert!(!fx.iter().any(|e| matches!(e, Effect::AlertSound(_))), "{fx:?}");
        assert_eq!(sh.bell.flash_alpha(5000.0), bell::FLASH_ALPHA);
        assert_eq!(sh.bell_alpha, bell::FLASH_ALPHA, "the flash repaints the terminal");
        sh.tick(5000.0 + bell::FLASH_MS + 1.0);
        assert_eq!(sh.bell_alpha, 0, "and the veil is gone after it");
    }

    #[test]
    fn typing_on_the_keyboard_reaches_the_shell() {
        let mut sh = terminal(&["/bin/sh", "-c", "read x; echo got:$x; read y"]);
        for (i, k) in ["h", "i", "return"].iter().enumerate() {
            tap_key(&mut sh, k, 2000.0 + i as f64 * 100.0);
        }
        pump_until(&mut sh, |sh| screen(sh).contains("got:hi"));
        assert!(screen(&sh).contains("got:hi"), "{}", screen(&sh));
    }

    #[test]
    fn named_keys_and_backspace_reach_the_shell() {
        let mut sh = terminal(&["/bin/sh", "-c", "/bin/stty raw -echo; echo ready; /bin/head -c 3 | /bin/od -An -c; read x"]);
        pump_until(&mut sh, |sh| screen(sh).contains("ready"));
        for (i, k) in ["esc", "tab", "⌫"].iter().enumerate() {
            tap_key(&mut sh, k, 2000.0 + i as f64 * 100.0);
        }
        pump_until(&mut sh, |sh| screen(sh).contains("177"));
        let out = screen(&sh);
        assert!(out.contains("033") && out.contains("\\t") && out.contains("177"), "{out}");
    }

    #[test]
    fn keyboard_touches_never_go_home_but_the_zone_below_does() {
        let mut sh = terminal(&["/bin/sh"]);
        let kb_y = sh.kb_top() + sh.keyboard.h() - 10.0;
        swipe(&mut sh, 600.0, kb_y, 200.0, 2000.0);
        settle(&mut sh, 2200.0);
        assert_eq!(sh.screen, Screen::App(AppId::Terminal));
        let zone_y = sh.h as f32 - 5.0;
        swipe(&mut sh, 600.0, zone_y, 1200.0, 3000.0);
        settle(&mut sh, 3200.0);
        assert_eq!(sh.screen, Screen::Home);
    }

    #[test]
    fn dragging_the_terminal_scrolls_history_and_typing_returns_live() {
        let mut sh = terminal(&["/bin/sh", "-c", "i=0; while [ $i -lt 120 ]; do i=$((i+1)); echo line$i; done; read x"]);
        pump_until(&mut sh, |sh| screen(sh).contains("line120"));
        swipe(&mut sh, 600.0, 300.0, 1000.0, 2000.0);
        assert!(!screen(&sh).contains("line120"), "scrolled back");
        assert!(sh.keyboard.shown());
        tap_key(&mut sh, "a", 3000.0);
        assert!(screen(&sh).contains("line120"), "typing jumps back to live");
    }

    #[test]
    fn terminal_keeps_running_at_home_and_relaunches_after_exit() {
        let mut sh = terminal(&["/bin/sh", "-c", "read x; echo bye:$x"]);
        assert_eq!(sh.app_poll_fds().len(), 1);
        tap_key(&mut sh, "q", 2000.0);
        sh.go_home(2100.0);
        settle(&mut sh, 2100.0);
        assert_eq!(sh.app_poll_fds().len(), 1, "still running at home");
        sh.term.as_mut().unwrap().key("Enter");
        sh.dirty = false;
        pump_until(&mut sh, |sh| sh.term.is_none());
        assert!(sh.term.is_none() && sh.app_poll_fds().is_empty(), "exited shell is dropped");
        assert!(sh.dirty && sh.notify.banner().is_some(), "the exit is announced with a banner");
        assert_eq!(sh.notify.store().groups()[0].latest().body, "The shell exited.");
        sh.term_cmd = &["/bin/sh"];
        sh.open(AppId::Terminal, 5000.0);
        assert!(sh.term.as_ref().is_some_and(|t| t.alive()), "relaunched on open");
    }

    // Auto-repeat of a held key belongs to the keyboard app: `holding_backspace_repeats_after_a_delay` in apps/keyboard.

    #[test]
    fn a_timer_the_clock_app_started_wakes_the_screen_and_the_alert_covers_the_lock_screen() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-service");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clock.ini").to_string_lossy().into_owned();
        let mut sh = shell();
        unlock(&mut sh);
        sh.set_wall(1_000_000, 0, 1000.0);
        sh.clock.load(&path, 1000.0);
        // the app writes a one-second timer beside the alarms
        let mut timers = clock::timer::Timers::default();
        timers.start("", 1000, 2000.0, 1_000_000_000 + 1000).unwrap();
        clock::store::write_atomic(&clock::store::sibling(&path, "clock-timers.ini"), &timers.serialize()).unwrap();
        sh.tick(2000.0);
        assert_eq!(sh.event(Event::Power, 8100.0), vec![Effect::ScreenOff]);
        assert!(sh.next_wake(8200.0).is_some(), "wake for the timer even with the screen off");
        let fx = sh.tick(9100.0);
        assert!(fx.contains(&Effect::Alarm) && fx.contains(&Effect::ScreenOn), "{fx:?}");
        assert!(sh.screen_on() && sh.clock.alert().is_some());
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 9200.0);
        assert!(buf.contains(&clock::ORANGE), "the alert covers the lock screen");
        let (stop_x, stop_y) = (sh.w as f32 / 2.0, sh.h as f32 - sh.ui.pt(200.0 - 28.0));
        tap(&mut sh, stop_x, stop_y, 9300.0);
        assert!(sh.clock.alert().is_none());
        assert!(sh.is_locked(), "back to the lock screen underneath");
    }

    /// E2E N6: the full-screen alert paints the whole frame, so the frame after Stop repaints the whole desktop, not
    /// only the regions the desktop's partial frame would clear (the desktop stayed black but for the Stop strip).
    #[test]
    fn the_frame_after_the_clock_alert_repaints_the_whole_desktop() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-alert-damage").join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clock.ini").to_string_lossy().into_owned();
        let mut sh = desktop_shell();
        // a tablet with the desktop presentation keeps the full-screen alert (E2E N5)
        sh.form = bifrost_ui::form_factor::FormFactor::Tablet;
        sh.screen = Screen::Home;
        sh.set_wall(1_000_000, 0, 1000.0);
        sh.clock.load(&path, 1000.0);
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1100.0);
        let orange_before = buf.contains(&clock::ORANGE);
        assert!(sh.clock.start_timer("", 1000, 1200.0).is_some());
        sh.tick(2300.0);
        assert!(sh.clock_takeover());
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 2400.0);
        assert!(buf.contains(&clock::ORANGE), "the alert is drawn");
        assert!(sh.desk_regions.is_none() && !sh.desk_drawn && sh.drawn == Drawn::Nothing, "the frame no longer holds the desktop");
        // Stop (the button's touch path is covered by the lock screen test above)
        sh.clock.stop_alert(false, 2500.0);
        assert!(sh.clock.alert().is_none());
        let damage = sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 2600.0);
        assert!(damage.is_all(), "the whole desktop is repainted after Stop");
        assert_eq!(buf.contains(&clock::ORANGE), orange_before, "nothing of the alert is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E N5: the desktop form shows a ringing alert as a banner (when the Clock app is installed); a tablet and the
    /// lock screen keep the full-screen alert.
    #[test]
    fn the_full_screen_clock_alert_is_for_the_tablet_and_the_lock_screen() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        assert_eq!(sh.clock_as_banner(), apphost::slot_of(clock::MANIFEST_ID).is_some(), "an unlocked desktop shows a banner");
        sh.screen = Screen::Lock;
        assert!(!sh.clock_as_banner(), "the lock screen keeps the full-screen alert");
        sh.screen = Screen::Home;
        sh.form = bifrost_ui::form_factor::FormFactor::Tablet;
        assert!(!sh.clock_as_banner(), "a tablet keeps the full-screen alert");
        let tablet = shell();
        assert!(!tablet.clock_as_banner());
    }

    /// Unlocked shell with Files open on a temp root holding one text file and a video.
    #[cfg(test)]
    fn files_app(tag: &str) -> (Shell, std::path::PathBuf) {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-files")).join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        std::fs::write(dir.join("clip.mp4"), "nope").unwrap();
        let mut sh = shell();
        sh.term_cmd = &["/bin/sh"];
        sh.files_locations = vec![files::fs::Location { name: "Tmp", path: dir.clone(), glyph: files::fs::Glyph::Computer }];
        unlock(&mut sh);
        sh.open(AppId::Files, 1000.0);
        settle(&mut sh, 1000.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        let first_row = files::pages::LocationsPage::row_top(&sh.ui, 0) + 10.0;
        tap(&mut sh, 600.0, first_row, 2000.0);
        settle(&mut sh, 2000.0);
        pump_until(&mut sh, |sh| sh.files.as_ref().unwrap().describe().contains("files_depth=2") && !sh.dirty_listing());
        (sh, dir)
    }

    impl Shell {
        /// True while the Files listing on top is still loading.
        #[cfg(test)]
        fn dirty_listing(&self) -> bool {
            self.files.as_ref().is_some_and(|f| f.loading())
        }
    }

    #[test]
    fn files_opens_the_terminal_at_a_folder_and_plays_videos() {
        let (mut sh, dir) = files_app("term");
        assert_eq!(sh.app_poll_fds().len(), 1, "the job pipe is polled");
        let row = |k: usize| sh.ui.pt(files::pages::CONTENT_TOP_PT) + (k as f32 + 0.5) * sh.ui.pt(files::pages::ROW_PT);
        // entries sort: a.txt, clip.mp4, Sub; a file opens the terminal in its folder
        let y = row(0);
        sh.event(Event::Down(600.0, y), 3000.0);
        assert_eq!(sh.next_wake(3000.0), Some(3000.0 + files::LONG_PRESS_MS));
        sh.tick(3000.0 + files::LONG_PRESS_MS + 1.0);
        sh.event(Event::Up(600.0, y), 3600.0);
        let f = sh.files.as_ref().unwrap();
        assert!(f.describe().contains("files_overlay=menu"));
        let bottom = sh.h as f32 - home_zone(&sh.ui);
        let share = f.menu_row(&sh.ui, files::menu::Action::Share, bottom).unwrap();
        tap(&mut sh, share.0, share.1, 4000.0);
        let open = sh.files.as_ref().unwrap().menu_row(&sh.ui, files::menu::Action::OpenInTerminal, bottom).unwrap();
        tap(&mut sh, open.0, open.1, 4500.0);
        assert_eq!(sh.screen, Screen::App(AppId::Terminal));
        assert!(sh.term.as_ref().is_some_and(|t| t.alive()));
        sh.term.as_mut().unwrap().write(b"echo \"${PWD##*/}\"\n");
        let at_dir = |sh: &Shell| screen(sh).lines().any(|l| l.trim() == "term");
        pump_until(&mut sh, at_dir);
        assert!(at_dir(&sh), "{}", screen(&sh));
        // the Files card was kept and the app still runs
        assert!(sh.switcher.apps().contains(&AppId::Files));
        sh.play_file(&dir.join("clip.mp4"));
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        assert!(sh.files.as_ref().unwrap().is_playing());
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 5000.0);
        sh.go_home(6000.0);
        settle(&mut sh, 6000.0);
        assert!(sh.files.is_some(), "still alive at home");
        sh.close_app(AppId::Files);
        assert!(sh.files.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_key_and_long_press_menu_type_the_clipboard_into_the_shell() {
        let _g = clipboard::tests::exclusive();
        let mut sh = terminal(&["/bin/sh", "-c", "read x; echo got:$x; read y; echo again:$y; read z"]);
        clipboard::set_text("abc");
        tap_key(&mut sh, "paste", 2000.0);
        tap_key(&mut sh, "return", 2100.0);
        pump_until(&mut sh, |sh| screen(sh).contains("got:abc"));
        assert!(screen(&sh).contains("got:abc"), "{}", screen(&sh));
        // a long press on the grid offers Paste
        sh.event(Event::Down(300.0, 400.0), 3000.0);
        assert_eq!(sh.next_wake(3000.0), Some(3000.0 + textfield::LONG_PRESS_MS));
        sh.dirty = false;
        sh.tick(3400.0);
        assert!(sh.dirty, "the fired long press repaints");
        sh.event(Event::Up(300.0, 400.0), 3450.0);
        let (mx, my, _, mh) = sh.term_touch.menu().expect("Paste menu").rect();
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 3500.0);
        assert!(buf.contains(&editmenu::BG), "the menu is drawn over the terminal");
        tap(&mut sh, mx + 5.0, my + mh / 2.0, 4000.0);
        tap_key(&mut sh, "return", 4100.0);
        pump_until(&mut sh, |sh| screen(sh).contains("again:abc"));
        assert!(screen(&sh).contains("again:abc"), "{}", screen(&sh));
        assert!(sh.term_touch.menu().is_none(), "typing dismisses the menu");
    }

    #[test]
        fn screenshot_chord_saves_a_png_shows_the_thumbnail_and_opens_photos() {
        let mut sh = shell();
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shell-shot"));
        let _ = std::fs::remove_dir_all(&dir);
        sh.data_dir = dir.clone();
        sh.photo_dirs = vec![dir.join("Pictures")];
        // pictures go to the account's Desktop unless told otherwise; this test looks at the library's folder
        sh.shot_dir = Some(dir.join("Pictures"));
        sh.set_wall(1_791_073_395, 0, 1000.0);
        unlock(&mut sh);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |sh: &mut Shell, t: f64| {
            sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
            buf.clone()
        };
        draw(&mut sh, 1900.0);
        assert!(sh.event(Event::Screenshot, 2000.0).is_empty());
        let path = dir.join("Pictures/Screenshot-20261004-002316.png");
        assert_eq!(sh.shot.as_ref().map(|o| o.path.clone()), Some(path.clone()));
        let frame = draw(&mut sh, 2000.0);
        assert!(frame.iter().all(|&p| p == WHITE), "the flash covers the screen");
        sh.tick(2300.0);
        assert!(sh.dirty, "the shrink animates");
        let start = std::time::Instant::now();
        while !path.exists() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let saved = crate::cg::imageio::decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((saved.w, saved.h), (1200, 1920));
        assert_eq!(saved.px[0], frame_pixel(&mut sh), "the file holds the home screen, not the flash");
        // resting thumbnail: a tap opens the capture in Markup
        let rest = 2000.0 + screenshot::FLASH_MS + screenshot::SHRINK_MS + 200.0;
        sh.tick(rest);
        draw(&mut sh, rest);
        assert_eq!(sh.next_wake(rest), Some(2000.0 + screenshot::HOLD_MS));
        let r = sh.shot.as_ref().unwrap().thumb_rect();
        tap(&mut sh, r.x as f32 + 10.0, r.y as f32 + 10.0, rest + 10.0);
        settle(&mut sh, rest + 50.0);
        assert!(sh.shot.is_none());
        assert!(sh.markup.is_some(), "the thumbnail opens the Markup editor");
        sh.markup = None;
        draw(&mut sh, rest + 2000.0);
        // a second capture while Photos is open joins the library; it ages out on its own
        sh.event(Event::Screenshot, 9000.0);
        sh.tick(9000.0 + screenshot::HOLD_MS + screenshot::OUT_MS);
        assert!(sh.shot.is_none());
        assert_eq!(sh.photos.as_ref().map(|p| p.photos().len()), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The top-left pixel of the shell's current frame.
    #[cfg(test)]
    fn frame_pixel(sh: &mut Shell) -> u32 {
        let mut buf = vec![0u32; 1200 * 1920];
        let shot = sh.shot.take();
        sh.drawn = Drawn::Nothing;
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1e9);
        sh.shot = shot;
        sh.drawn = Drawn::Nothing;
        buf[0]
    }

    #[test]
    fn exiting_in_the_foreground_goes_home() {
        let mut sh = terminal(&["/bin/sh", "-c", "echo hi"]);
        pump_until(&mut sh, |sh| sh.term.is_none());
        settle(&mut sh, 9000.0);
        assert_eq!(sh.screen, Screen::Home);
    }

    #[test]
    fn output_marks_the_open_terminal_dirty() {
        let mut sh = terminal(&["/bin/sh", "-c", "read x; echo out; read y"]);
        sh.term.as_mut().unwrap().key("Enter");
        sh.dirty = false;
        pump_until(&mut sh, |sh| screen(sh).contains("out"));
        assert!(sh.dirty);
    }

    #[test]
    fn volume_keys_step_on_the_lock_screen_and_show_the_hud() {
        let mut sh = shell();
        sh.set_volume(0.5);
        assert!(!sh.hud.shown(), "syncing the level shows nothing");
        assert!(sh.is_locked());
        assert_eq!(sh.event(Event::VolumeUp, 1000.0), vec![Effect::Volume(9.0 / 16.0)]);
        assert_eq!(sh.volume(), 9.0 / 16.0);
        assert!(sh.hud.shown() && sh.hud.visible(1000.0));
        assert_eq!(sh.hud.level(), 9.0 / 16.0);
        assert_eq!(sh.event(Event::VolumeDown, 1100.0), vec![Effect::Volume(0.5)]);
        assert!(sh.describe().contains("volume=0.500\nhud=shown"));
        assert_eq!(sh.next_wake(1200.0), Some(1100.0 + volume_hud::HOLD_MS), "wake for the fade");
        let mut buf = vec![0u32; 1200 * 1920];
        let d = sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1200.0).clone();
        assert!(d.is_all(), "the lock screen's first frame");
        let (x, y, w, h) = VolumeHud::rect(&sh.ui, sh.w);
        let inside = buf[(y + h / 2.0) as usize * 1200 + (x + w - sh.ui.pt(20.0)) as usize];
        assert_ne!(inside, rgba(20, 20, 30, 255), "the HUD covers the wallpaper");
        let d = sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1300.0).clone();
        assert_eq!(d.areas().len(), 1, "a steady HUD redraws only its box: {:?}", d.areas());
        let a = d.areas()[0];
        assert!(a.x0 as f32 <= x && a.x1 as f32 >= x + w && a.y0 as f32 <= y && a.y1 as f32 >= y + h);
        assert!(sh.tick(1100.0 + volume_hud::HOLD_MS + 100.0).is_empty());
        assert!(sh.needs_frame(), "fading needs frames");
        sh.tick(1100.0 + volume_hud::HOLD_MS + volume_hud::FADE_MS + 1.0);
        assert!(!sh.hud.shown());
        let d = sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 3000.0).clone();
        assert_eq!(d.areas().len(), 1, "the frame that removes it repaints its box");
        assert_eq!(buf[(y + h / 2.0) as usize * 1200 + (x + w - sh.ui.pt(20.0)) as usize], rgba(20, 20, 30, 255), "wallpaper is back");
        assert!(sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 3016.0).is_empty());
    }

    #[test]
    fn volume_keys_work_in_apps_and_with_the_screen_off_without_waking_it() {
        let mut sh = shell();
        sh.set_volume(1.0);
        unlock(&mut sh);
        sh.open(AppId::Files, 3000.0);
        settle(&mut sh, 3000.0);
        assert_eq!(sh.event(Event::VolumeUp, 4000.0), vec![Effect::Volume(1.0)], "full stays full");
        assert_eq!(sh.event(Event::VolumeDown, 4100.0), vec![Effect::Volume(15.0 / 16.0)]);
        assert!(sh.hud.shown());
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 4100.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files), "the app keeps the screen");
        assert_eq!(sh.event(Event::Power, 5000.0), vec![Effect::ScreenOff]);
        assert!(!sh.hud.shown(), "sleeping drops the HUD");
        assert_eq!(sh.event(Event::VolumeDown, 6000.0), vec![Effect::Volume(14.0 / 16.0)]);
        assert!(!sh.screen_on() && !sh.hud.shown(), "no wake, no HUD in the dark");
        assert!(!sh.needs_frame());
        assert_eq!(sh.next_wake(6000.0), None);
        for i in 0..20 {
            sh.event(Event::VolumeDown, 7000.0 + i as f64);
        }
        assert_eq!(sh.volume(), 0.0, "stops at mute");
        assert_eq!(sh.event(Event::VolumeDown, 8000.0), vec![Effect::Volume(0.0)]);
    }

    #[test]
    fn brightness_keys_step_the_panel_and_show_the_same_hud_as_volume() {
        use bifrost_ui::osd::Level;
        let (down, up) = (224, 225);
        let mut sh = shell();
        assert!(sh.is_locked());
        assert_eq!(sh.info.brightness, 0.5);
        assert_eq!(sh.hw_brightness_key(up, true, 1000.0), Some(vec![Effect::Brightness(9.0 / 16.0)]), "works on the lock screen");
        assert_eq!(sh.info.brightness, 9.0 / 16.0);
        assert!(sh.hud.shown() && sh.hud.kind() == Level::Brightness && sh.hud.level() == 9.0 / 16.0);
        assert_eq!(sh.hw_brightness_key(up, false, 1010.0), Some(vec![]), "a release is taken and does nothing");
        assert_eq!(sh.hw_brightness_key(down, true, 1100.0), Some(vec![Effect::Brightness(0.5)]));
        let mut buf = vec![0u32; 1200 * 1920];
        let d = sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1200.0).clone();
        assert!(d.is_all());
        let (x, y, w, h) = VolumeHud::rect(&sh.ui, sh.w);
        assert_ne!(buf[(y + h / 2.0) as usize * 1200 + (x + w - sh.ui.pt(20.0)) as usize], rgba(20, 20, 30, 255), "the HUD covers the wallpaper");
        assert_eq!(sh.next_wake(1200.0), Some(1100.0 + volume_hud::HOLD_MS));
        // a volume key brings the speaker back on the same panel
        sh.event(Event::VolumeUp, 1300.0);
        assert!(sh.hud.kind() == Level::Volume);
        // the lowest step stays lit, the highest is full
        for i in 0..40 {
            sh.hw_brightness_key(down, true, 2000.0 + i as f64);
        }
        assert_eq!(sh.info.brightness, 1.0 / 16.0);
        for i in 0..40 {
            sh.hw_brightness_key(up, true, 3000.0 + i as f64);
        }
        assert_eq!(sh.info.brightness, 1.0);
        assert_eq!(sh.hw_brightness_key(30, true, 4000.0), None, "other keys are not ours");
    }

    #[test]
    fn brightness_keys_leave_a_dark_screen_and_the_control_center_alone() {
        let mut sh = shell();
        unlock(&mut sh);
        swipe(&mut sh, 1100.0, 5.0, 500.0, 1000.0);
        settle(&mut sh, 1110.0);
        assert!(sh.cc.is_shown());
        assert_eq!(sh.hw_brightness_key(225, true, 2000.0), Some(vec![Effect::Brightness(9.0 / 16.0)]));
        assert!(!sh.hud.shown(), "the sheet's slider moves instead of the HUD");
        sh.set_volume(0.5);
        sh.cc.hide();
        assert_eq!(sh.event(Event::Power, 5000.0), vec![Effect::ScreenOff]);
        assert_eq!(sh.hw_brightness_key(225, true, 6000.0), Some(vec![]), "no wake, no backlight in the dark");
        assert!(!sh.hud.shown());
        assert_eq!(sh.info.brightness, 9.0 / 16.0);
    }

    #[test]
    fn keyboard_backlight_keys_need_a_backlight_and_use_its_steps() {
        use bifrost_ui::osd::Level;
        let (down, up) = (229, 230);
        let mut sh = shell();
        assert_eq!(sh.keyboard_backlight(), None);
        assert_eq!(sh.hw_brightness_key(up, true, 1000.0), None, "no LED: the key is left to the next handler");
        sh.set_keyboard_backlight(0.0, 3);
        assert_eq!(sh.hw_brightness_key(up, true, 1000.0), Some(vec![Effect::KeyboardBacklight(1.0 / 3.0)]));
        assert!(sh.hud.shown() && sh.hud.kind() == Level::KeyboardBrightness);
        assert_eq!(sh.info.brightness, 0.5, "the panel is untouched");
        sh.hw_brightness_key(up, true, 1100.0);
        sh.hw_brightness_key(up, true, 1200.0);
        assert_eq!(sh.keyboard_backlight(), Some(1.0), "stops at the top step");
        assert_eq!(sh.hw_brightness_key(down, true, 1300.0), Some(vec![Effect::KeyboardBacklight(2.0 / 3.0)]));
        for _ in 0..5 {
            sh.hw_brightness_key(down, true, 1400.0);
        }
        assert_eq!(sh.keyboard_backlight(), Some(0.0), "the backlight can go fully off");
        sh.set_keyboard_backlight(0.5, 0);
        assert_eq!(sh.keyboard_backlight(), None, "zero steps means none");
    }

    #[test]
    fn control_center_slider_and_keys_share_one_level() {
        let mut sh = shell();
        unlock(&mut sh);
        swipe(&mut sh, 1100.0, 5.0, 500.0, 1000.0);
        settle(&mut sh, 1110.0);
        assert!(sh.cc.is_shown());
        let (x, y, w, h) = control_center::tile_rect(&sh.ui, sh.w, control_center::Tile::Volume);
        sh.event(Event::Down(x + w / 2.0, y + h * 0.25), 2000.0);
        sh.event(Event::Up(x + w / 2.0, y + h * 0.25), 2030.0);
        assert_eq!(sh.volume(), 0.75);
        assert_eq!(sh.event(Event::VolumeUp, 3000.0), vec![Effect::Volume(13.0 / 16.0)]);
        assert!(!sh.hud.shown(), "the sheet's slider moves instead of the HUD");
        assert_eq!(sh.cc.volume, 13.0 / 16.0);
        sh.set_volume(0.25);
        assert_eq!(sh.cc.volume, 0.25);
    }

    #[test]
    fn voice_control_presses_the_named_element_only_for_a_final_command() {
        use bifrost_ui::whisper::Transcript;
        let mut sh = desktop_shell();
        let hypothesis = Transcript { text: "press General > Software Update".into(), is_final: false };
        assert!(sh.voice_press(&hypothesis, 1.0).is_none(), "a hypothesis is not a command");
        let chatter = Transcript { text: "good morning".into(), is_final: true };
        assert!(sh.voice_press(&chatter, 1.0).is_none(), "a sentence without a press verb is not a command");
        let heard = Transcript { text: "press os/settings/general/software-update.".into(), is_final: true };
        let answer = sh.voice_press(&heard, 2.0).expect("a command").expect("the element is pressable");
        assert_eq!(answer.0, "os/settings/general/software-update");
    }

    /// A `speech_recognizer` that logs the calls it gets and answers a scripted poll, in place of the real service.
    // Voice Control is being reworked; this rig is kept for it.
    #[allow(dead_code)]
    struct ScriptedRecognizer {
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        refuse: bool,
    }

    impl super::services::SystemService for ScriptedRecognizer {
        fn name(&self) -> &'static str {
            "speech_recognizer"
        }
        fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, _params: &bifrost_core::json::Json, _t: f64) -> Result<bifrost_core::json::Json, String> {
            use bifrost_core::json::{parse, Json};
            self.calls.lock().unwrap().push(format!("{app_id} {method}"));
            match method {
                "start" if self.refuse => Err("speech recognizer: no consent".into()),
                "start" => Ok(Json::Obj(vec![("id".into(), Json::Num(1.0))])),
                "poll" => parse(r#"{"transcripts":[{"text":"press os/settings/general/software-update.","final":true},{"text":"press nothing","final":false}],"running":true}"#),
                "stop" => Ok(Json::Obj(vec![("stopped".into(), Json::Bool(true))])),
                other => Err(format!("no method {other}")),
            }
        }
    }

    // Voice Control is being reworked; this rig is kept for it.
    #[allow(dead_code)]
    fn voice_rig(refuse: bool) -> (Shell, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sh = desktop_shell();
        sh.services = vec![Box::new(ScriptedRecognizer { calls: calls.clone(), refuse })];
        (sh, calls)
    }

    #[test]
    fn the_shell_tick_runs_no_recognition_of_its_own_while_voice_control_is_on() {
        // the recognizer service holds the one recognition (`services::recognizer`); the shell never starts, polls or stops one
        let (mut sh, calls) = voice_rig(false);
        sh.prefs.set("a11y_voice_control", "1").unwrap();
        for i in 0..8 {
            sh.tick(1000.0 + i as f64 * 400.0);
        }
        assert!(calls.lock().unwrap().is_empty(), "no start, poll or stop from the shell: {:?}", calls.lock().unwrap());
        sh.prefs.set("a11y_voice_control", "0").unwrap();
        sh.tick(5000.0);
        assert!(calls.lock().unwrap().is_empty(), "switching it off stops nothing from the shell either");
    }

    #[test]
    fn the_shell_consumes_final_transcripts_the_recognizer_heard() {
        use bifrost_ui::whisper::Transcript;
        let mut sh = desktop_shell();
        let heard = [
            Transcript { text: "good morning".into(), is_final: true },
            Transcript { text: "press os/settings/general/software-update.".into(), is_final: true },
        ];
        sh.voice_consume(&heard, 1000.0);
        assert_eq!(sh.settings.as_ref().expect("Settings opened").page(), super::pages::PageId::SoftwareUpdate, "the command ran, the chatter did not");
    }

    #[test]
    fn a_transcript_that_cannot_run_is_announced_and_a_non_command_is_silent() {
        use bifrost_ui::whisper::Transcript;
        let mut sh = desktop_shell();
        let fx = sh.voice_consume(&[Transcript { text: "good morning".into(), is_final: true }], 1.0);
        assert!(fx.is_empty());
        assert!(!sh.announce.tick(1.0), "a sentence with no press verb says nothing");
        sh.voice_consume(&[Transcript { text: "press os/no/such/element".into(), is_final: true }], 2.0);
        assert!(sh.announce.tick(2.0));
        assert!(sh.announce.shown().is_some_and(|t| t.contains("such")), "{:?}", sh.announce.shown());
    }

    #[test]
    fn switch_control_runs_the_reader_cursor_without_the_screen_reader() {
        let mut sh = desktop_shell();
        assert!(!sh.reader_active());
        sh.prefs.set("a11y_switch_control", "1").unwrap();
        assert!(sh.switch_control_on() && sh.reader_active() && !sh.screen_reader_on());
        sh.prefs.set("a11y_switch_control", "0").unwrap();
        sh.prefs.set("a11y_screen_reader", "1").unwrap();
        assert!(sh.reader_active() && !sh.switch_control_on(), "the reader alone keeps the cursor on");
    }

    #[test]
    fn ax_press_walks_a_label_path_in_settings_like_a_tap() {
        let mut sh = desktop_shell();
        let map = sh.ax_map(0.0);
        assert!(map.get("os/settings/general/software-update").is_some());
        let (address, _) = sh.ax_press("General > Software Update", 1.0).expect("pressed");
        assert_eq!(address, "os/settings/general/software-update");
        assert_eq!(sh.settings.as_ref().unwrap().page(), super::pages::PageId::SoftwareUpdate);
        // a bare name that is unique works, an ambiguous or unknown one says so
        assert!(sh.ax_press("nonexistent-row", 2.0).unwrap_err().contains("no element"));
        let err = sh.ax_press("about", 3.0);
        assert!(err.is_ok() || err.unwrap_err().contains("ambiguous"));
    }

    // the menu bar exists only on the desktop build; without it `ax_menus` is empty by design
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_os_map_covers_menus_and_apps_and_presses_them() {
        let mut sh = desktop_shell();
        let map = sh.ax_map(0.0);
        assert!(map.get("os/menubar/file").is_some_and(|e| e.role == "menu"));
        assert!(map.under("os/menubar/edit").any(|e| e.name == "Select All"), "{}", map.listing("os/menubar"));
        assert!(map.under("os/apps").any(|e| e.name == "Settings"));
        let (address, _) = sh.ax_press("os/apps/settings", 1.0).expect("opens");
        assert_eq!(address, "os/apps/settings");
        assert!(sh.desk.as_ref().unwrap().wins.iter().any(|w| w.app == super::AppId::Settings));
        assert!(sh.ax_map(2.0).get("os/apps/settings").is_some_and(|e| e.value == "open"));
    }

    // an open window is an element whose title-bar actions are listed and run through `ax action`
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn an_open_window_lists_and_runs_its_title_bar_actions() {
        let mut sh = desktop_shell();
        sh.ax_press("os/apps/settings", 1.0).expect("opens");
        let map = sh.ax_map(2.0);
        assert!(map.get("os/windows/settings").is_some_and(|e| e.role == "window" && e.handle == "win:0"), "{}", map.listing("os/windows"));
        assert_eq!(map.custom_actions("os/windows/settings").collect::<Vec<_>>(), ["Zoom", "Minimize", "Close"]);
        assert!(sh.ax_press("os/windows/settings", 3.0).unwrap_err().contains("is a window"));
        assert!(sh.ax_action("os/windows/settings", "Fly", 4.0).unwrap_err().contains("has no action"));
        let (address, _) = sh.ax_action("os/windows/settings", "Minimize", 5.0).expect("minimizes");
        assert_eq!(address, "os/windows/settings");
        assert!(sh.desk.as_ref().unwrap().wins.iter().any(|w| w.app == super::AppId::Settings && w.minimized));
        let listing = sh.ax_listing("os/windows", 6.0);
        assert!(listing.starts_with("os/windows/settings\twindow\tSettings\tminimized\tenabled\t\t\t\n"), "{listing}");
        assert_eq!(sh.ax_actions("os/windows/settings", 7.0).unwrap(), "Zoom\nMinimize\nClose\n");
    }

    #[test]
    fn an_alert_is_answered_through_the_map_and_blocks_other_presses() {
        let mut sh = desktop_shell();
        sh.ax_press("General > Software Update", 1.0).expect("page");
        let map = sh.ax_map(2.0);
        assert!(map.get("os/settings/general/software-update/roll-back-to-previous-system").is_some(), "{}", map.listing("os/settings/general/software-update"));
        let pressed = sh.ax_press("roll-back-to-previous-system", 3.0).expect("opens the alert");
        let map = sh.ax_map(4.0);
        assert!(pressed.0.ends_with("roll-back-to-previous-system"));
        assert!(map.get("os/settings/alert").is_some_and(|e| e.value == "Roll Back to Previous System?"));
        assert!(map.get("os/settings/alert/roll-back").is_some_and(|e| e.role == "button"), "the confirm button carries its label");
        let refused = sh.ax_press("General > About", 5.0).unwrap_err();
        assert!(refused.contains("alert is open"), "{refused}");
        sh.ax_press("os/settings/alert/cancel", 6.0).expect("cancel");
        assert!(sh.ax_map(7.0).get("os/settings/alert").is_none(), "the alert is gone");
        assert!(sh.ax_press("General > About", 8.0).is_ok(), "presses work again");
    }

    /// E2E: an app that owns Print (`AppMenuItem::print`, `printDocument:`) gets one File > Print…, its own, in
    /// the place of the shell's window snapshot, and Cmd+P chooses it. An app without one keeps the shell's.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn an_apps_own_print_replaces_the_shells_print_row() {
        use bifrost_proto::{AppMenu,AppMenuItem,CHORD_META};
        let mut sh=desktop_shell();sh.anim=None;
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.app_menus=vec![AppMenu{title:"File".into(),items:vec![AppMenuItem::new(1,"New Card").key("cmd+n")]}];
        sh.ext.push(Some(host));sh.ext_fg=Some(0);sh.screen=Screen::App(AppId::External(0));
        let prints=|sh:&Shell|sh.menu_rows(desktop_bar::Menu::File).into_iter().filter(|r|r.text.starts_with("Print")).collect::<Vec<_>>();
        let shell_print=prints(&sh);
        assert_eq!(shell_print.len(),1,"the shell's own Print… for an app without one");
        assert!(matches!(shell_print[0].action,None|Some(desktop_bar::Action::Print)));
        sh.ext[0].as_mut().unwrap().app_menus[0].items.push(AppMenuItem::print(7));
        let rows=sh.menu_rows(desktop_bar::Menu::File);
        let print:Vec<&desktop_bar::Row>=rows.iter().filter(|r|r.text.starts_with("Print")).collect();
        assert_eq!(print.len(),1,"one Print…, not the app's and the shell's: {:?}",rows.iter().map(|r|r.text.as_str()).collect::<Vec<_>>());
        assert_eq!(print[0].text,"Print\u{2026}");
        assert!(matches!(print[0].action,Some(desktop_bar::Action::AppMenu(0,7))),"the row is the app's command");
        assert_eq!(rows.last().map(|r|r.text.as_str()),Some("Print\u{2026}"),"it stands where the shell's row was, at the end of File");
        assert_eq!(rows[0].text,"New Card","the app's other File items still lead");
        // Cmd+P (evdev P is 25) is the app's Print
        assert!(sh.ext_chord(25,CHORD_META));
        let bifrost_proto::sys::Recv::Packet(packet,_)=theirs.recv(false).unwrap() else{panic!("missing key equivalent")};
        assert_eq!(bifrost_proto::ToApp::decode(&packet),Ok(bifrost_proto::ToApp::MenuAction(7)));
    }

    #[cfg(feature="desktop-wayland")]
    #[test]
    fn app_published_menus_follow_the_app_name_and_send_the_chosen_item_back() {
        use bifrost_proto::{AppMenu,AppMenuItem,CHORD_META,CHORD_SHIFT};
        let menus=vec![AppMenu{title:"Note".into(),items:vec![
            AppMenuItem::new(1,"New Note").key("cmd+n"),AppMenuItem::separator(),
            AppMenuItem::new(2,"Pin").checked(true),AppMenuItem::new(3,"Lock").enabled(false).key("cmd+shift+l"),
            AppMenuItem::submenu("Sort By"),AppMenuItem::new(4,"Name").depth(1)]}];
        assert_eq!(desktop_bar::app_menu_chord(&menus,'n',CHORD_META),Some(1));
        assert_eq!(desktop_bar::app_menu_chord(&menus,'n',0),None,"the modifier must match");
        assert_eq!(desktop_bar::app_menu_chord(&menus,'l',CHORD_META|CHORD_SHIFT),None,"a disabled item has no key equivalent");
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        assert!(!sh.menu_buttons().iter().any(|b|matches!(b.0,desktop_bar::Menu::Custom(_))),"no app in front, no app menus");
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.app_menus=menus;sh.ext.push(Some(host));sh.ext_fg=Some(0);sh.screen=Screen::App(AppId::External(0));
        let order:Vec<desktop_bar::Menu>=sh.menu_buttons().into_iter().map(|b|b.0).filter(|m|matches!(m,desktop_bar::Menu::App|desktop_bar::Menu::Custom(_)|desktop_bar::Menu::File)).collect();
        assert_eq!(order,vec![desktop_bar::Menu::App,desktop_bar::Menu::File,desktop_bar::Menu::Custom(0)],"the app's own menus follow View, not the app name");
        let rows=sh.menu_rows(desktop_bar::Menu::Custom(0));
        assert_eq!(rows.iter().map(|r|r.text.as_str()).collect::<Vec<_>>(),vec!["New Note","","\u{2713} Pin","Lock","Sort By"],"a submenu's items are in the submenu, not the menu");
        assert!(matches!(rows[4].action,Some(desktop_bar::Action::Submenu(desktop_bar::Menu::AppSub(0,4)))),"Sort By opens its submenu");
        let sub=sh.menu_rows(desktop_bar::Menu::AppSub(0,4));
        assert_eq!(sub.iter().map(|r|r.text.as_str()).collect::<Vec<_>>(),vec!["Name"],"the submenu's items, no row back: it cascades beside the menu");
        assert!(matches!(sub[0].action,Some(desktop_bar::Action::AppMenu(0,4))));
        assert_eq!(sh.bar_anchor(desktop_bar::Menu::AppSub(0,4)),desktop_bar::Menu::Custom(0),"it hangs from the menu's bar item");
        assert_eq!(sh.menu_row_keys(desktop_bar::Menu::Custom(0)),vec!["\u{2318}N".to_string(),"\u{21E7}\u{2318}L".to_string()]);
        // an app menu titled like a shell menu merges into it (NativeApp.mainMenu owns File on Golden Gate) instead of repeating it
        sh.ext[0].as_mut().unwrap().app_menus.push(AppMenu{title:"File".into(),items:vec![AppMenuItem::new(9,"Print...").key("cmd+p")]});
        assert_eq!(sh.menu_buttons().iter().filter(|b|b.3=="File").count(),1,"one File menu");
        let file:Vec<String>=sh.menu_rows(desktop_bar::Menu::File).iter().map(|r|r.text.clone()).collect();
        assert_eq!(&file[..2],&["Print...".to_string(),String::new()],"the app's File items lead, then a separator: {file:?}");
        sh.ext[0].as_mut().unwrap().app_menus.pop();
        sh.desktop_menu=Some(desktop_bar::Menu::Custom(0));
        let action=sh.menu_rows(desktop_bar::Menu::Custom(0)).remove(2).action.expect("Pin is enabled");
        sh.run_menu_action(action,10.);
        let bifrost_proto::sys::Recv::Packet(packet,_)=theirs.recv(false).unwrap() else{panic!("missing menu action")};
        assert_eq!(bifrost_proto::ToApp::decode(&packet),Ok(bifrost_proto::ToApp::MenuAction(2)));
        assert!(sh.ext_chord(49,CHORD_META),"a key equivalent is consumed as a menu action");
        let bifrost_proto::sys::Recv::Packet(packet,_)=theirs.recv(false).unwrap() else{panic!("missing key equivalent")};
        assert_eq!(bifrost_proto::ToApp::decode(&packet),Ok(bifrost_proto::ToApp::MenuAction(1)));
    }

    /// The app's own menus sit after View and before Window (the Browser: File, Edit, View, History, Bookmarks, Develop, Window, Help),
    /// and the document apps' Format menus sit right after Edit. The app's order is kept within each group.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn an_apps_own_menus_follow_view_and_format_menus_follow_edit() {
        use bifrost_proto::AppMenu;
        use desktop_bar::Menu;
        let menu=|title:&str|AppMenu{title:title.into(),items:Vec::new()};
        // the Browser: File and View merge into the shell's menus, the others keep the app's order
        let browser=vec![menu("File"),menu("View"),menu("History"),menu("Bookmarks"),menu("Develop")];
        assert_eq!(desktop_bar::bar_after_app(&browser,false),vec![Menu::File,Menu::Edit,Menu::View,Menu::Custom(2),Menu::Custom(3),Menu::Custom(4),Menu::Window,Menu::Help]);
        // a document app: Format right after Edit, its other menu after View
        let notes=vec![menu("Note"),menu("Format")];
        assert_eq!(desktop_bar::bar_after_app(&notes,false),vec![Menu::File,Menu::Edit,Menu::Custom(1),Menu::View,Menu::Custom(0),Menu::Window,Menu::Help]);
        // no app menus: the shell's six; Photos adds Image before View
        assert_eq!(desktop_bar::bar_after_app(&[],false),vec![Menu::File,Menu::Edit,Menu::View,Menu::Window,Menu::Help]);
        assert_eq!(desktop_bar::bar_after_app(&[],true),vec![Menu::File,Menu::Edit,Menu::Image,Menu::View,Menu::Window,Menu::Help]);
    }

    /// Key equivalents go to the app in front first. A tabbed app that publishes Close Tab on Cmd+W keeps the chord while the item is
    /// enabled; the shell closes the window with it only when the app does not take it, and the File menu then moves Close Window to
    /// Shift+Cmd+W so two items never claim Cmd+W.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn cmd_w_goes_to_an_app_that_defines_it_and_closes_the_window_otherwise() {
        use bifrost_proto::{AppMenu,AppMenuItem,CHORD_META,CHORD_SHIFT};
        const W:u16=17;
        let menus=|enabled:bool|vec![AppMenu{title:"File".into(),items:vec![AppMenuItem::new(7,"Close Tab").key("cmd+w").enabled(enabled)]}];
        assert!(desktop_bar::chord_in_menus(&menus(true),W,CHORD_META),"an enabled Close Tab takes Cmd+W");
        assert!(!desktop_bar::chord_in_menus(&menus(false),W,CHORD_META),"a disabled one leaves Cmd+W to the shell");
        assert!(!desktop_bar::chord_in_menus(&menus(true),W,CHORD_META|CHORD_SHIFT),"Shift+Cmd+W is not Cmd+W");
        assert!(!desktop_bar::chord_in_menus(&menus(true),16,CHORD_META),"another key is not the chord");
        assert!(!desktop_bar::chord_in_menus(&[],W,CHORD_META),"an app without the chord never owns it");
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        assert!(!sh.app_owns_close(),"no app in front, the shell closes the window");
        let (mine,_theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.app_menus=menus(true);sh.ext.push(Some(host));sh.ext_fg=Some(0);sh.screen=Screen::App(AppId::External(0));
        assert!(sh.app_owns_close(),"the front app defines Cmd+W");
        assert!(sh.app_owns_chord(W,CHORD_META));
        assert!(!sh.app_owns_chord(W,CHORD_META|CHORD_SHIFT),"Shift+Cmd+W is the window's");
        let keys=sh.menu_row_keys(desktop_bar::Menu::File);
        assert!(keys.contains(&"\u{21E7}\u{2318}W".to_string()),"Close Window moves to Shift+Cmd+W: {keys:?}");
        assert_eq!(keys.iter().filter(|k|k.as_str()=="\u{2318}W").count(),1,"one item claims Cmd+W: {keys:?}");
        // the app's last tab: Close Tab is disabled, so Cmd+W is the shell's again
        sh.ext[0].as_mut().unwrap().app_menus=menus(false);
        assert!(!sh.app_owns_close(),"a disabled Close Tab does not keep the chord");
        sh.screen=Screen::Home;
        assert!(!sh.app_owns_chord(W,CHORD_META),"only the app in front owns chords");
    }

    /// Cmd+A, Cmd+C, Cmd+X, Cmd+V and Cmd+Z are the SDK app's: the shell used to take them as shortcuts for a text control of its
    /// own and drop them whenever an external app was in front, so a Text Editor or a Mail field never saw them. The app now gets the
    /// standard command when it reported that it takes it, the chord when it did not, its own menu item on the chord first; Ctrl+A
    /// is the line start, so it reaches the app as a chord and never as Select All. The keys that move or delete in text reach an SDK
    /// app only while one of its text fields has the focus.
    #[test]
    fn the_editing_shortcuts_reach_the_sdk_app_in_front_instead_of_being_swallowed() {
        use bifrost_proto::{AppMenu,AppMenuItem,Command,Commands,KeyInput,ToApp,CHORD_ALT,CHORD_CTRL,CHORD_META,CHORD_SHIFT};
        use hwkeys::{HwKeys,Shortcut};
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.commands=Commands::default().with(Command::SelectAll,true).with(Command::Copy,true).with(Command::Cut,true).with(Command::Paste,true).with(Command::Undo,true);
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.ext.push(Some(host));sh.ext_fg=Some(0);sh.screen=Screen::App(AppId::External(0));
        let sent=||{
            let mut all=Vec::new();
            while let bifrost_proto::sys::Recv::Packet(packet,_)=theirs.recv(false).unwrap(){all.push(bifrost_proto::ToApp::decode(&packet).unwrap());}
            all
        };
        let _=sent();
        // each shortcut reaches the app as its command
        for (shortcut,command) in [(Shortcut::SelectAll,Command::SelectAll),(Shortcut::Copy,Command::Copy),(Shortcut::Cut,Command::Cut),(Shortcut::Paste,Command::Paste),(Shortcut::Undo,Command::Undo)] {
            sh.hw_shortcut(shortcut,1.0);
            assert!(sent().contains(&ToApp::Command(command)),"{shortcut:?} reaches the app as {command:?}");
        }
        // a command the app did not report goes as the Cmd chord of the same key: Redo is Shift+Cmd+Z (evdev 44)
        sh.hw_shortcut(Shortcut::Redo,2.0);
        assert!(sent().contains(&ToApp::Key(KeyInput::chord(44,CHORD_META|CHORD_SHIFT))),"Redo, unreported, is Shift+Cmd+Z");
        // the app's own enabled menu item on the chord wins over the command
        sh.ext[0].as_mut().unwrap().app_menus=vec![AppMenu{title:"Edit".into(),items:vec![AppMenuItem::new(7,"Select All").key("cmd+a")]}];
        sh.hw_shortcut(Shortcut::SelectAll,3.0);
        assert_eq!(sent(),vec![ToApp::MenuAction(7)],"performKeyEquivalent: first");
        // the physical keys reach `hw_shortcut` as the shortcuts the table names
        let mut keys=HwKeys::default();
        let pressed=|keys:&mut HwKeys,held:&[u16],code:u16|{
            for k in held{keys.key_front(*k,true,false,false);}
            let sc=keys.key_front(code,true,false,false);
            keys.key_front(code,false,false,false);
            for k in held{keys.key_front(*k,false,false,false);}
            sc
        };
        assert_eq!(pressed(&mut keys,&[125],30),Some(Shortcut::SelectAll),"Cmd+A");
        assert_eq!(pressed(&mut keys,&[125],46),Some(Shortcut::Copy),"Cmd+C");
        assert_eq!(pressed(&mut keys,&[125],45),Some(Shortcut::Cut),"Cmd+X");
        assert_eq!(pressed(&mut keys,&[125],47),Some(Shortcut::Paste),"Cmd+V");
        assert_eq!(pressed(&mut keys,&[125],44),Some(Shortcut::Undo),"Cmd+Z");
        assert_eq!(pressed(&mut keys,&[125,42],44),Some(Shortcut::Redo),"Shift+Cmd+Z");
        // Ctrl+A is Select All as a shortcut; the binary withholds it from an SDK app, which gets the chord (the line start)
        assert_eq!(pressed(&mut keys,&[29],30),Some(Shortcut::SelectAll));
        // Ctrl+Y is still Redo in the table, and the binary withholds it from an SDK app the same way, where it is the yank
        assert_eq!(pressed(&mut keys,&[29],21),Some(Shortcut::Redo));
        assert!(sh.sdk_app_front());
        assert!(sh.hw_chord(30,CHORD_CTRL));
        assert_eq!(sent(),vec![ToApp::Key(KeyInput::chord(30,CHORD_CTRL))]);
        // Option+Backspace and Option+Delete are chords; the editing keys go to an app only with a text field focused
        let mut k=HwKeys::default();
        k.key_front(56,true,false,false);
        assert_eq!(k.chord_mods(14),Some(CHORD_ALT),"Option+Backspace");
        assert_eq!(k.chord_mods(111),Some(CHORD_ALT),"Option+Delete");
        assert_eq!(k.nav_mods(106),Some(CHORD_ALT),"Option+Right");
        assert!(!sh.ext_text_focused(),"no text field is focused: an app without `edit_keys` keeps its plain arrows");
        assert!(!sh.ext_takes_edit_keys(),"so Ctrl+A stays Select All and Ctrl+Y Redo");
        sh.ext[0].as_mut().unwrap().caret_rect=Some((0.,0.,1.,16.));
        assert!(sh.ext_text_focused());
        assert!(sh.ext_takes_edit_keys(),"with a field focused Ctrl+A is the line start");
        assert!(sh.hw_nav(106,CHORD_ALT),"a focused field takes Option+Right");
        assert_eq!(sent(),vec![ToApp::Key(KeyInput::chord(106,CHORD_ALT))]);
        assert!(sh.hw_nav(102,0),"Home alone, too");
        assert_eq!(sent(),vec![ToApp::Key(KeyInput::chord(102,0))]);
        // Search open over the app: it takes the typing, so Delete and Option+Backspace never reach the hidden document
        assert!(!sh.shell_takes_typing());
        sh.open_quicksearch_hw(10.0);
        let h=sh.h as i32;
        sh.quicksearch.tick(&sh.ui,h,12_000.0);
        assert!(sh.quicksearch.is_shown() && sh.shell_takes_typing());
        assert!(sh.sdk_app_front() && sh.ext_text_focused());
        assert!(!sh.hw_nav(111,0),"forward Delete stays with Search");
        assert!(!sh.hw_chord(14,CHORD_ALT),"Option+Backspace stays with Search");
        assert!(!sh.hw_chord(30,CHORD_CTRL),"Ctrl+A stays with Search");
        assert!(sent().is_empty(),"nothing reached the app behind Search");
        sh.quicksearch.hide();
        let h=sh.h as i32;
        sh.quicksearch.tick(&sh.ui,h,24_000.0);
        assert!(!sh.shell_takes_typing());
        assert!(sh.hw_nav(111,0),"with Search closed the field takes it again");
        let _=sent();
        // the shell's own screens take none of it
        sh.screen=Screen::Home;
        assert!(!sh.sdk_app_front());
        assert!(!sh.hw_nav(106,CHORD_ALT));
    }

    /// Cmd+Q quits the front app and Cmd+W only closes its window. The decision (`desktop_bar::quit_step`) is pure: a window server
    /// toplevel of a running SDK app quits the app, a native or Linux app closes all its windows (as the Dock's Quit does), and a
    /// window of the shell's own desk quits its app or, for a built-in window, closes as before.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn cmd_q_decides_what_to_quit_from_what_is_in_front() {
        use desktop_bar::{quit_step,QuitStep};
        let sdk=|id:&str|(id=="files").then_some(3u8);
        // the toplevel of a direct SDK app is in front: the app is quit, whatever else the window server lists
        assert_eq!(quit_step(&[(10,"files",true),(11,"other",false)],sdk,Some(AppId::Settings)),QuitStep::App(3));
        // a native or Linux app: every window of that app closes and no other app's
        assert_eq!(quit_step(&[(20,"org.example.edit",false),(21,"org.example.edit",true),(22,"files",false)],sdk,None),QuitStep::Windows(vec![20,21]));
        // no toplevel in front: the shell's own front window decides
        assert_eq!(quit_step(&[(10,"files",false)],sdk,Some(AppId::Settings)),QuitStep::Window(AppId::Settings));
        assert_eq!(quit_step(&[],sdk,Some(AppId::Terminal)),QuitStep::Window(AppId::Terminal));
        assert_eq!(quit_step(&[],sdk,Some(AppId::External(2))),QuitStep::App(2),"a shell-drawn SDK app window quits its app");
        assert_eq!(quit_step(&[],sdk,Some(AppId::ExternalWindow(2,7))),QuitStep::App(2),"an extra window quits its app, not just itself");
        assert_eq!(quit_step(&[],sdk,None),QuitStep::Nothing);
    }

    /// Cmd+Q is the app's when its menus publish an enabled item on it (the app saves and exits itself, `app_owns_chord`);
    /// otherwise it is the shell's and quits the app. Cmd+W is not affected by an app that defines only Cmd+Q, and the
    /// shortcuts that are not window keys are always the shell's.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn cmd_q_goes_to_an_app_that_defines_it_and_quits_the_app_otherwise() {
        use bifrost_proto::{AppMenu,AppMenuItem,CHORD_META,CHORD_SHIFT};
        const Q:u16=16;const W:u16=17;
        let menus=|enabled:bool|vec![AppMenu{title:"File".into(),items:vec![AppMenuItem::new(9,"Quit Notes").key("cmd+q").enabled(enabled)]}];
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        assert!(sh.shortcut_is_shells(hwkeys::Shortcut::Quit,false),"no app takes Cmd+Q: the shell quits the app");
        assert!(!sh.shortcut_is_shells(hwkeys::Shortcut::Quit,true),"an app that took the chord keeps it");
        let (mine,_theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let mut host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        host.app_menus=menus(true);sh.ext.push(Some(host));sh.ext_fg=Some(0);sh.screen=Screen::App(AppId::External(0));
        assert!(sh.app_owns_chord(Q,CHORD_META),"the front app defines Cmd+Q");
        assert!(!sh.shortcut_is_shells(hwkeys::Shortcut::Quit,sh.app_owns_chord(Q,CHORD_META)),"the app's Quit runs, not the shell's");
        assert!(!sh.app_owns_chord(Q,CHORD_META|CHORD_SHIFT),"Shift+Cmd+Q is not Cmd+Q");
        assert!(!sh.app_owns_chord(W,CHORD_META),"the app defines only Cmd+Q");
        assert!(sh.shortcut_is_shells(hwkeys::Shortcut::Close,sh.app_owns_chord(W,CHORD_META)),"Cmd+W still closes the window");
        assert!(sh.shortcut_is_shells(hwkeys::Shortcut::QuickSearch,true),"only window keys wait for the app");
        // the app disables its Quit: Cmd+Q is the shell's again
        sh.ext[0].as_mut().unwrap().app_menus=menus(false);
        assert!(sh.shortcut_is_shells(hwkeys::Shortcut::Quit,sh.app_owns_chord(Q,CHORD_META)));
    }

    /// Quitting an SDK app sends it `Quit` (it can save its state) and takes it off the app list; closing its window (Cmd+W,
    /// the traffic light) leaves the app running and sends nothing.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn quitting_an_sdk_app_sends_it_quit_and_closing_its_window_does_not() {
        use bifrost_proto::{sys::Recv,ToApp};
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let (mine,theirs)=bifrost_proto::sys::socketpair_pair().unwrap();
        let host=apphost::AppHost::with_channel("fixture",mine,None,bifrost_proto::Configure{w:100,h:100,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false});
        sh.ext.push(Some(host));
        let sent=||{let mut v=Vec::new();while let Ok(Recv::Packet(p,_))=theirs.recv(false){v.push(ToApp::decode(&p).unwrap());}v};
        sh.desk_close_app(AppId::External(0));
        assert!(sh.ext[0].is_some(),"closing the window leaves the app running");
        assert!(!sent().contains(&ToApp::Quit),"and does not ask it to quit");
        sh.desk_quit_app(0);
        assert!(sh.ext[0].is_none(),"the quit app left the list");
        assert!(sent().contains(&ToApp::Quit),"the app is asked to quit, so it can save its state");
    }

    /// Cmd+Q and the App menu's Quit close a built-in window as before: only an SDK app has a process to quit.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn quit_closes_a_built_in_window_as_before() {
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);sh.desk_open(AppId::Settings,0.);
        let quit=sh.menu_rows(desktop_bar::Menu::App).into_iter().find(|r|r.text.starts_with("Quit ")).expect("the App menu has a Quit row");
        assert!(matches!(quit.action,Some(desktop_bar::Action::Quit)),"the Quit row quits, it does not only close the window");
        sh.hw_shortcut(hwkeys::Shortcut::Quit,1.);
        let d=sh.desk.as_ref().unwrap();
        assert!(d.index_of(AppId::Settings).is_none(),"Cmd+Q closed the front built-in window");
        assert!(d.index_of(AppId::Terminal).is_some(),"and only that one");
        sh.run_menu_action(desktop_bar::Action::Quit,2.);
        assert!(sh.desk.as_ref().unwrap().index_of(AppId::Terminal).is_none(),"the menu's Quit closes the next one");
    }

    /// Every app's application menu carries the standard items (HIG "The menu bar", App menu): About, Services, Hide,
    /// Hide Others, Show All and Quit with their keys; Settings\u{2026} only for an app that has settings (a built-in window
    /// has none; the desktop's opens System Settings).
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_application_menu_has_the_standard_items() {
        use desktop_bar::{Action,Menu};
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        let home=sh.menu_row_texts(Menu::App);
        assert!(home.contains(&"Settings\u{2026}".to_string()),"the desktop's Settings opens System Settings: {home:?}");
        assert!(sh.menu_rows(Menu::App).iter().any(|r|r.text.starts_with("Hide ")&&r.action.is_none()),"nothing to hide on the desktop");
        sh.desk_open(AppId::Terminal,0.);
        let rows=sh.menu_rows(Menu::App);
        let texts:Vec<&str>=rows.iter().map(|r|r.text.as_str()).collect();
        let name=texts[0].strip_prefix("About ").expect("About comes first").to_string();
        for want in ["Services".to_string(),format!("Hide {name}"),"Hide Others".to_string(),"Show All".to_string(),format!("Quit {name}")] {
            assert!(texts.contains(&want.as_str()),"{want}: {texts:?}");
        }
        assert!(!texts.contains(&"Settings\u{2026}"),"a built-in window has no settings of its own: {texts:?}");
        assert!(sh.front_app_settings().is_none(),"so Cmd+, does nothing for it");
        let keys=sh.menu_row_keys(Menu::App);
        assert!(keys.contains(&"\u{2318}H".to_string())&&keys.contains(&"\u{2325}\u{2318}H".to_string())&&keys.contains(&"\u{2318}Q".to_string()),"{keys:?}");
        let row=|t:&str|rows.iter().find(|r|r.text==t).unwrap().action.clone();
        assert!(matches!(row(&format!("Hide {name}")),Some(Action::Hide)));
        assert!(row("Hide Others").is_none(),"no other app shows a window");
        assert!(row("Show All").is_none(),"nothing is hidden");
        assert!(matches!(row("Services"),Some(Action::Submenu(Menu::Services))),"Services is a submenu");
        assert!(!sh.menu_rows(Menu::Services).iter().any(|r|matches!(r.action,Some(Action::Submenu(_)))),"the submenu cascades beside the application menu: no row leads back");
    }

    /// Hide (Cmd+H) puts away every window of the app in front without a Dock thumbnail, Hide Others the windows of every
    /// other app, and Show All brings them back where they were.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn hide_hide_others_and_show_all_act_on_whole_apps() {
        use desktop_bar::{Action,Menu};
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Terminal,0.);sh.desk_open(AppId::Settings,0.);
        let fx=sh.hw_shortcut(hwkeys::Shortcut::Hide,1.);
        assert!(fx.is_empty());
        let d=sh.desk.as_ref().unwrap();
        assert!(d.is_hidden(AppId::Settings),"Cmd+H hid Settings");
        assert_eq!(d.front().map(|w|w.app),Some(AppId::Terminal),"the next app takes the front");
        assert!(!d.is_hidden(AppId::Terminal));
        let show=sh.menu_rows(Menu::App).into_iter().find(|r|r.text=="Show All").unwrap();
        assert!(matches!(show.action,Some(Action::UnhideAll)),"Show All is enabled while an app is hidden");
        sh.run_menu_action(Action::UnhideAll,2.);
        let d=sh.desk.as_ref().unwrap();
        assert!(!d.is_hidden(AppId::Settings)&&!d.wins[d.index_of(AppId::Settings).unwrap()].minimized,"Show All brought it back");
        assert_eq!(d.front().map(|w|w.app),Some(AppId::Settings),"in its place in the stack");
        let others=sh.menu_rows(Menu::App).into_iter().find(|r|r.text=="Hide Others").unwrap();
        assert!(matches!(others.action,Some(Action::HideOthers)),"Terminal shows a window behind Settings");
        sh.hw_shortcut(hwkeys::Shortcut::HideOthers,3.);
        let d=sh.desk.as_ref().unwrap();
        assert!(d.is_hidden(AppId::Terminal),"Option+Cmd+H hid the other app");
        assert_eq!(d.front().map(|w|w.app),Some(AppId::Settings),"and kept the front one");
        // activating a hidden app shows it again
        sh.desk_open(AppId::Terminal,4.);
        let d=sh.desk.as_ref().unwrap();
        assert!(!d.is_hidden(AppId::Terminal)&&!d.any_hidden(),"opening a hidden app unhides it");
    }

    /// An app's View menu carries no shell-wide items: the Dock's settings live in the Dock's menu and Settings, and the
    /// menu ends with Enter Full Screen.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn the_view_menu_has_no_dock_settings() {
        use desktop_bar::Menu;
        let mut sh=desktop_shell();sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let rows=sh.menu_row_texts(Menu::View);
        assert!(!rows.iter().any(|t|t.contains("Dock")||t.contains("Icons")),"{rows:?}");
        assert_eq!(rows.last().map(String::as_str),Some("Enter Full Screen"),"{rows:?}");
    }

    /// The Dock's hover label shows the app's name: the text is tinted with the label colour over the menu material, so it
    /// contrasts with the plate in Dark and in Light (it was blitted white, which left an empty pill on Light's pale plate).
    #[test]
    fn the_dock_hover_label_draws_its_name_in_both_schemes() {
        use super::ui::{LABEL,MENU_TINT};
        let sh=desktop_shell();
        let lum=|c:u32|((c&255)+((c>>8)&255)+((c>>16)&255))/3;
        let (w,h)=(400usize,400usize);
        let backdrop=rgba(90,120,150,255);
        // each scheme's own colours from the pairs, so the process-wide scheme is not touched
        for (scheme,plate,ink) in [("Dark",MENU_TINT.dark,LABEL.dark),("Light",MENU_TINT.light,LABEL.light)] {
            let mut px=vec![backdrop;w*h];
            {
                let mut s=Surface{w,h,stride:w,px:&mut px,y0:0};
                sh.draw_dock_label(&mut s,"Files",desk::Rect{x:176,y:300,w:48,h:48},desktop_dock::Position::Bottom,1.0,plate,ink);
            }
            let drawn:Vec<usize>=(0..w*h).filter(|&i|px[i]!=backdrop).collect();
            assert!(!drawn.is_empty(),"{scheme}: the label plate is drawn");
            let (x0,x1)=(drawn.iter().map(|i|i%w).min().unwrap(),drawn.iter().map(|i|i%w).max().unwrap());
            let (y0,y1)=(drawn.iter().map(|i|i/w).min().unwrap(),drawn.iter().map(|i|i/w).max().unwrap());
            // the plate's own colour, left of the text at mid height; the name's pixels are the ones that stand out from it
            let plate_px=px[((y0+y1)/2)*w+x0+3];
            let text=(y0..=y1).flat_map(|y|(x0..=x1).map(move|x|(x,y))).filter(|&(x,y)|lum(px[y*w+x]).abs_diff(lum(plate_px))>80).count();
            assert!(text>20,"{scheme}: the name shows in the label colour on the plate ({text} text pixels)");
        }
    }

    #[test]
    fn work_area_starts_under_the_drawn_menu_bar() {
        let sh = desktop_shell();
        assert_eq!(sh.desk_metrics().menubar, sh.desktop_bar_height() as i32);
    }

    #[test]
    fn a_drag_that_started_on_the_empty_desktop_is_swallowed() {
        let mut sh = desktop_shell();
        assert_eq!(sh.desk_event(Event::Move(600.0, 900.0), 10.0), Some(vec![]));
    }

    #[test]
    fn open_quick_search_action_shows_search() {
        let mut sh = desktop_shell();
        sh.act(Action::OpenQuickSearch, 10.0);
        assert!(sh.quicksearch.active());
    }

    #[test]
    fn desktop_open_leaves_the_phone_dock_recents_alone() {
        let mut sh = desktop_shell();
        sh.desk_open(AppId::Settings, 10.0);
        sh.desk_open(AppId::Settings, 20.0);
        assert!(sh.recent_apps.is_empty());
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn bluetooth_menu_lists_paired_devices_and_connects_or_disconnects_them() {
        use crate::bare::bluetooth::{Cmd as BtCmd, Device, State, Status};
        let dev = |addr: &str, name: &str, paired: bool, connected: bool| Device { addr: addr.into(), kind: 0, name: name.into(), paired, connected, audio: false, rssi: None, class: None };
        let mut sh = shell();
        // without a controller the extra is not on the bar at all
        assert!(!sh.menu_buttons().iter().any(|b| b.0 == desktop_bar::Menu::Bluetooth));
        sh.info.bt = Status {
            state: State::On,
            devices: vec![dev("AA:BB:CC:DD:EE:01", "Buds", true, true), dev("AA:BB:CC:DD:EE:02", "Mouse", true, false), dev("AA:BB:CC:DD:EE:03", "Found", false, false)],
            ..Status::default()
        };
        assert!(sh.menu_buttons().iter().any(|b| b.0 == desktop_bar::Menu::Bluetooth), "the extra shows once the radio exists");
        let rows = sh.menu_row_texts(desktop_bar::Menu::Bluetooth);
        assert_eq!(rows[0], "Turn Bluetooth Off");
        assert!(rows.iter().any(|t| t == "\u{2713} Buds"), "connected device checked: {rows:?}");
        assert!(rows.iter().any(|t| t == "  Mouse"), "paired device listed: {rows:?}");
        assert!(!rows.iter().any(|t| t.contains("Found")), "a scanned, unpaired device stays out of the menu: {rows:?}");
        let action_of = |sh: &Shell, text: &str| sh.menu_rows(desktop_bar::Menu::Bluetooth).into_iter().find(|r| r.text == text).and_then(|r| r.action);
        let mouse = action_of(&sh, "  Mouse").expect("paired device row acts");
        assert_eq!(sh.run_menu_action(mouse, 0.0), vec![Effect::Bluetooth(BtCmd::Connect("AA:BB:CC:DD:EE:02".into()))]);
        let buds = action_of(&sh, "\u{2713} Buds").expect("connected device row acts");
        assert_eq!(sh.run_menu_action(buds, 0.0), vec![Effect::Bluetooth(BtCmd::Disconnect("AA:BB:CC:DD:EE:01".into()))]);
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn battery_menu_switches_low_power_mode_on_and_off() {
        let mut sh = shell();
        let row = |sh: &Shell, text: &str| sh.menu_rows(desktop_bar::Menu::Power).into_iter().find(|r| r.text == text).and_then(|r| r.action);
        assert!(sh.menu_row_texts(desktop_bar::Menu::Power).iter().any(|t| t == "  Low Power Mode"), "the Battery menu offers Low Power Mode");
        assert!(!sh.low_power());
        let on = row(&sh, "  Low Power Mode").expect("the row acts");
        sh.run_menu_action(on, 0.0);
        assert!(sh.low_power(), "the row turns Low Power Mode on");
        assert!(sh.menu_row_texts(desktop_bar::Menu::Power).iter().any(|t| t == "\u{2713} Low Power Mode"), "checked while on");
        let off = row(&sh, "\u{2713} Low Power Mode").expect("the checked row acts");
        sh.run_menu_action(off, 0.0);
        assert!(!sh.low_power(), "and turns it off again");
    }

    /// Settings window repaint cost at the lab panel's size (`cargo test --profile fast --lib -- --ignored
    /// bench_settings_window_draw --nocapture`). Each frame counts as fresh input, so the window is drawn in full.
    #[test]
    #[ignore]
    fn bench_settings_window_draw() {
        let mut sh=shell_sized(2560,1440);sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);
        let (w,h)=(sh.w as usize,sh.h as usize);let mut img=Image::new(w,h);
        for k in 0..5 {sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},100.+k as f64*20.);}
        let n=200;let start=std::time::Instant::now();
        for k in 0..n {let t=1000.+k as f64*16.;sh.last_input=t;sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},t);}
        eprintln!("settings window repaint: {:.2} ms a frame at {w}x{h}",start.elapsed().as_secs_f64()*1000./n as f64);
    }

    /// Focus switch between two built-in windows at the lab panel's size (`--ignored bench_focus_switch --nocapture`):
    /// each frame brings the other window to the front, as Cmd-Tab and a click on a back window do.
    #[test]
    #[ignore]
    fn bench_focus_switch() {
        let mut sh=shell_sized(2560,1440);sh.desk=Some(desk::Desk::new());sh.screen=Screen::Home;sh.anim=None;
        sh.desk_open(AppId::Settings,0.);sh.desk_open(AppId::Files,10.);
        let (w,h)=(sh.w as usize,sh.h as usize);let mut img=Image::new(w,h);
        for k in 0..5 {sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},100.+k as f64*20.);}
        let n=100;let start=std::time::Instant::now();
        for k in 0..n {let t=1000.+k as f64*16.;let app=if k%2==0 {AppId::Settings} else {AppId::Files};sh.desk_open(app,t);sh.draw_desk(&mut Surface{w,h,stride:w,px:&mut img.px,y0:0},t+8.);}
        eprintln!("focus switch: {:.2} ms a frame at {w}x{h}",start.elapsed().as_secs_f64()*1000./n as f64);
    }

#[cfg(test)]
mod live_wallpaper_tests {
    use super::*;
    use crate::shell::live_wallpaper::{Look, FRAME_MS};

    fn centre(sh: &Shell) -> u32 {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut img = Image::new(w, h);
        sh.draw_desktop_background(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 });
        img.px[(h / 2) * w + w / 2]
    }

    #[test]
    fn a_chosen_look_is_what_the_desktop_draws_under_the_windows() {
        let mut sh = desktop_shell();
        let still = centre(&sh);
        assert_eq!(still, rgba(20, 20, 30, 255), "the still photo");
        assert!(!sh.live_wallpaper_tick(0.0), "the still look makes no frame");
        sh.set_live_look(Look::Drift);
        assert_eq!(sh.live_look(), Look::Drift);
        assert_eq!(centre(&sh), still, "until the first frame is due the still stays");
        assert!(sh.live_wallpaper_tick(0.0), "the first frame is rendered");
        let first = centre(&sh);
        assert_ne!(first, still, "the desktop now shows the drift frame");
        assert_eq!(first, sh.wallpaper_frame().px[(sh.h as usize / 2) * sh.w as usize + sh.w as usize / 2]);
        assert!(!sh.live_wallpaper_tick(FRAME_MS / 2.0), "within a frame interval the cached frame is reused");
        assert!(sh.live_wallpaper_tick(FRAME_MS * 40.0), "a later frame moves the picture");
        assert_ne!(centre(&sh), first);
        sh.set_live_look(Look::Still);
        assert_eq!(centre(&sh), still, "choosing Still brings the photo back");
    }

    #[test]
    fn a_new_frame_redraws_and_asks_the_presenter_to_upload_it_again() {
        let mut sh = desktop_shell();
        sh.set_live_look(Look::Waves);
        sh.note_wallpaper_uploaded();
        sh.dirty = false;
        assert!(sh.desktop_wallpaper_once().is_none(), "uploaded");
        assert!(sh.live_wallpaper_tick(0.0));
        assert!(sh.dirty, "the desktop redraws");
        let (wp, x) = sh.desktop_wallpaper_once().expect("the presenter takes the new frame");
        assert_eq!((wp.w, wp.h, x), (sh.w as usize, sh.h as usize, 0));
        assert_eq!(wp.px.as_ptr(), sh.wallpaper_frame().px.as_ptr(), "it is the live frame, not the still");
        let next = sh.next_wake(0.0).expect("an animated look schedules its next frame");
        assert!(next <= FRAME_MS + 1.0, "{next}");
    }

    #[test]
    fn low_power_mode_freezes_an_animated_look_on_one_frame() {
        let mut sh = desktop_shell();
        sh.prefs.set("low_power", "1").unwrap();
        assert!(sh.low_power());
        sh.set_live_look(Look::Dither);
        assert!(sh.live_wallpaper_tick(0.0));
        let frozen = centre(&sh);
        assert_ne!(frozen, rgba(20, 20, 30, 255), "the fallback is a drawn still frame, not a blank");
        assert!(!sh.live_wallpaper_tick(60_000.0), "no further frames");
        assert_eq!(centre(&sh), frozen);
        assert_eq!(sh.live_still_only(), true);
    }

    #[test]
    fn the_time_of_day_poster_follows_the_clock() {
        let mut sh = desktop_shell();
        sh.set_live_look(Look::TimeOfDay);
        let day = 86_400 * 20_000;
        sh.set_wall(day, 0, 0.0);
        assert!(sh.live_wallpaper_tick(0.0));
        let night = centre(&sh);
        sh.set_wall(day + 12 * 3600, 0, 0.0);
        assert!(sh.live_wallpaper_tick(0.0), "noon is a different minute");
        let noon = centre(&sh);
        let sum = |p: u32| (p & 0xff) + (p >> 8 & 0xff) + (p >> 16 & 0xff);
        assert!(sum(noon) > sum(night) + 150, "noon is brighter than midnight");
        assert!(!sh.live_wallpaper_tick(1.0), "the same minute keeps the cached poster");
    }

    #[test]
    fn the_look_does_not_run_off_the_desktop_or_with_the_screen_off() {
        let mut sh = shell();
        sh.set_live_look(Look::Drift);
        assert!(!sh.live_wallpaper_tick(0.0), "the bifrost home screen has no live wallpaper");
        let mut sh = desktop_shell();
        sh.set_live_look(Look::Drift);
        sh.screen_on = false;
        assert!(!sh.live_wallpaper_tick(0.0), "the screen is off");
    }

    /// W5-4: on the GPU presenter's path the shell's surface is transparent over the wallpaper and a desktop frame clears
    /// only the regions it redraws. A banner sliding in and out over the desktop must clear its own rect each frame (and
    /// its last one after it left): every frame equals the same frame drawn whole, so no slide frame of the card stays.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_banner_sliding_out_over_the_desktop_leaves_no_card_behind() {
        use crate::shell::notify::banner::{HOLD_MS, SLIDE_MS};
        struct Cpu;
        impl Drop for Cpu {
            fn drop(&mut self) {
                bifrost_render::glass::set_thread_gpu(None);
            }
        }
        let _cpu = Cpu;
        bifrost_render::glass::set_thread_gpu(Some(true));
        let mut sh = desktop_shell();
        sh.anim = None;
        sh.screen_on = true;
        sh.screen = Screen::Home;
        sh.drawn = Drawn::App;
        sh.desk.as_mut().unwrap().wins.clear();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.tick(1000.0);
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
        sh.keyboard.tick(1.0e6);
        sh.notify(crate::shell::notify::Notification::new(AppId::Terminal, "Term", "The shell exited.", 0));
        assert!(sh.notify.banner().is_some(), "the desktop shows a banner");
        let whole = |sh: &mut Shell, t: f64| {
            sh.force_full_redraw();
            let mut fresh = vec![0u32; w * h];
            sh.draw(&mut Surface { w, h, stride: w, px: &mut fresh, y0: 0 }, t);
            fresh
        };
        let out = 1000.0 + SLIDE_MS + HOLD_MS;
        let mut card = None;
        for t in [1000.0 + SLIDE_MS / 2.0, 1000.0 + SLIDE_MS + 10.0, out + SLIDE_MS * 0.2, out + SLIDE_MS * 0.4, out + SLIDE_MS * 0.6, out + SLIDE_MS * 0.8] {
            sh.tick(t);
            sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, t);
            card = card.or(sh.banner_damage_rect(t));
            let expected = whole(&mut sh, t);
            let off = px.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(off, 0, "frame at {t}: {off} px differ from the same frame drawn whole");
        }
        // gone: the frame after it left clears its last rect
        let end = out + SLIDE_MS + 1.0;
        sh.tick(end);
        assert!(sh.notify.banner().is_none());
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, end);
        assert!(!sh.notify_damage_all(), "nothing of the banner is left to repaint");
        let expected = whole(&mut sh, end);
        let (x, y, cw, ch) = card.expect("the card was on screen");
        for row in y.max(0) as usize..((y + ch) as usize).min(h) {
            for col in x.max(0) as usize..((x + cw) as usize).min(w) {
                assert_eq!(px[row * w + col], expected[row * w + col], "a pixel of the old card at ({col}, {row})");
            }
        }
        assert!(px == expected, "the desktop is as if no banner had been there");
    }

    /// W5-4: a notification posted between ticks (a terminal exit or an app's post, served before the loop's tick) starts
    /// its banner at the loop's clock, not at the last tick: on an idle desktop that was seconds earlier, and the banner
    /// left after half its rest while `state` and the screen disagreed.
    #[test]
    fn a_banner_posted_after_an_idle_gap_rests_its_full_time() {
        use crate::shell::notify::banner::{Phase, HOLD_MS, SLIDE_MS};
        let mut sh = desktop_shell();
        sh.anim = None;
        sh.screen_on = true;
        sh.screen = Screen::Home;
        sh.tick(1000.0);
        // idle for 3 s, then the loop wakes for the post
        sh.notify_note_time(4000.0);
        sh.notify(crate::shell::notify::Notification::new(AppId::Terminal, "Term", "The shell exited.", 0));
        sh.tick(4000.0 + SLIDE_MS + HOLD_MS - 1.0);
        assert_eq!(sh.notify.banner().map(|b| b.phase()), Some(Phase::Hold), "still resting just before its five seconds are up");
        assert!(sh.describe().contains("banner=hold"), "and `state` says so");
        sh.tick(4000.0 + SLIDE_MS + HOLD_MS + 1.0);
        assert_eq!(sh.notify.banner().map(|b| b.phase()), Some(Phase::Out));
        // the loop's clock never takes the notification clock back
        let before = sh.notify.t;
        sh.notify_note_time(10.0);
        assert_eq!(sh.notify.t, before);
    }

    /// W5-1: a desktop frame asked for by motion the settled check did not count (a Dock tile's indeterminate progress bar
    /// while a Clock timer runs, the hover label's fade) still hands the window server every window: an empty window
    /// list destroys all the toplevels, and the desktop showed only the wallpaper, the menu bar and the Dock.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn every_desktop_frame_publishes_the_windows() {
        let _lock = crate::test_support::global_lock();
        let saved = std::env::var_os("SPRINGBOARD_WAYLAND");
        std::env::set_var("SPRINGBOARD_WAYLAND", "1");
        let mut sh = desktop_shell();
        sh.anim = None;
        sh.screen_on = true;
        sh.screen = Screen::Home;
        sh.drawn = Drawn::App;
        let m = sh.desk_metrics();
        sh.desk.as_mut().unwrap().wins.clear();
        sh.desk.as_mut().unwrap().open(&m, AppId::Settings, (600, 400));
        let id = crate::shell::desk::win_id(AppId::Settings).unwrap();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 0.0);
        assert!(sh.take_wayland_windows().iter().any(|f| f.id == id), "the first frame publishes the window");
        sh.keyboard.tick(1.0e6);
        sh.dirty = false;
        // the Dock's hover label fades: the shell asks for frames without being dirty
        sh.desk.as_mut().unwrap().dock_label_ms = 1000.0;
        sh.desk_t = 1100.0;
        assert!(sh.dock_animating(1100.0) && sh.needs_frame(), "Dock motion asks for frames");
        assert!(sh.desk_animating(1100.0), "and counts as desktop motion, so the frame is not the settled one");
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1100.0);
        assert!(sh.take_wayland_windows().iter().any(|f| f.id == id), "a frame for Dock motion keeps the window");
        // the settled frame (nothing new) keeps the windows as the last frame left them
        sh.dirty = false;
        sh.ext_frame = false;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 5000.0);
        assert!(sh.take_wayland_windows().iter().any(|f| f.id == id), "a settled frame never publishes an empty window list");
        if let Some(v) = saved {
            std::env::set_var("SPRINGBOARD_WAYLAND", v)
        } else {
            std::env::remove_var("SPRINGBOARD_WAYLAND")
        };
    }
}
