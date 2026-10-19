//! Engine lifecycle and application timer work.
use super::*;

impl Browser {
    /// The engine profile: the persistent one, or in a private window a throwaway directory of this process.
    pub(super) fn profile(&self) -> PathBuf {
        if self.private {
            private_profile()
        } else {
            home().join(".local/share/bifrost-browser/profile")
        }
    }

    pub(super) fn start(&mut self) {
        if self.pool.is_some() {
            return;
        }
        match self.open_pool() {
            Ok(()) => {
                if self.restore.is_empty() {
                    match self.initial_url.take() {
                        Some(url) => self.new_tab(&url),
                        None if !self.private && self.prefs.restore && self.restore_session() => {}
                        None => {
                            let url = self.prefs.start_url(START);
                            self.new_tab(&url)
                        }
                    }
                } else {
                    for url in std::mem::take(&mut self.restore) {
                        self.new_tab(&url);
                    }
                    self.select(self.restore_sel.min(self.tabs.len().saturating_sub(1)));
                }
            }
            Err(e) => self.error = Some(format!("The web engine could not start: {e}")),
        }
    }

    fn open_pool(&mut self) -> io::Result<()> {
        let renderer = if self.renderer_fallback { WebRenderer::Cpu } else { self.prefs.renderer };
        let mut pool = WebProcessPool::spawn_with_renderer(&engine()?, &self.profile(), renderer)?;
        pool.dev_collector(&format!("{};{}", network::COLLECTOR_JS, pagetools::CONSOLE_JS));
        let profile = self.profile();
        self.ctl.bridge.attach(&mut pool, &profile);
        self.pool = Some(pool);
        self.watchdog.reset(self.clock.elapsed().as_secs_f64());
        self.error = None;
        Ok(())
    }

    /// Renderer changes are an explicit restart. Preserve tab order and the current tab, keep dormant tabs
    /// dormant, and leave the settings sheet available if the selected backend cannot start.
    pub(super) fn restart_renderer(&mut self) {
        // an explicit choice in Settings starts over: the GPU is tried again, with one fallback if it fails
        self.renderer_fallback = false;
        self.sheet.fallback = false;
        let mut pages: Vec<(String, String, bool)> = self.tabs.iter().filter_map(|tab| {
            let pool = self.pool.as_ref()?;
            let state = pool.state(tab.view)?;
            Some((state.url.clone(), state.title.clone(), pool.is_discarded(tab.view)))
        }).collect();
        let selected = if pages.is_empty() { self.restore_sel } else { self.current };
        if pages.is_empty() {
            pages = self.restore.iter().enumerate().map(|(i, url)| (url.clone(), String::new(), i != selected)).collect();
        }
        // Finish outstanding RPCs before their engine is destroyed, rather than waiting for their deadlines.
        self.ctl_on_event(&WebViewEvent::EngineExited);
        self.inspector.clear_all();
        for tab in &self.tabs {
            self.reader.forget(tab.view);
            self.autofill.forget(tab.view);
            self.favicons.forget(tab.view);
        }
        self.pageui.cancel_all(&mut self.pool);
        self.pool = None;
        self.uploads = Default::default();
        self.tabs.clear();
        self.pageui = Default::default();
        self.discard = Default::default();
        self.current = 0;
        self.restore = pages.iter().map(|page| page.0.clone()).collect();
        self.restore_sel = selected;
        match self.open_pool() {
            Ok(()) => {
                if let Some(pool) = self.pool.as_mut() {
                    for (url, title, dormant) in &pages {
                        let view = if *dormant { pool.create_discarded(url, title) } else { pool.create(url) };
                        self.tabs.push(Tab { view });
                    }
                }
                self.restore.clear();
                if self.tabs.is_empty() {
                    let url = self.prefs.start_url(START);
                    self.new_tab(&url);
                } else {
                    self.select(selected.min(self.tabs.len() - 1));
                }
                self.session.dirty = true;
            }
            Err(error) => self.error = Some(format!("The web engine could not start: {error}. Choose CPU fallback in Settings if GPU rendering is unavailable.")),
        }
    }

    /// Cmd+Shift+N: a private window is a second browser process started with `--private`.
    pub(super) fn open_private_window(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let launcher = std::env::var_os("BIFROST_BROWSER_LAUNCHER").map(PathBuf::from).unwrap_or(exe);
            if let Ok(mut child) = Command::new(launcher)
                .arg("--private")
                // its own control socket: binding the default path would take over the main window's
                .env("BIFROST_CONTROL", home().join(format!(".cache/bifrost-browser/private-{}-{}.sock", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()))))
                .spawn() {
                bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                    let _ = child.wait();
                });
            }
        }
    }

    /// Clear Website Data: stops the engine, deletes the profile (cookies, storage, cache) and restarts it on the
    /// same pages.
    pub(super) fn clear_website_data(&mut self) -> io::Result<()> {
        if self.pool.as_ref().is_some_and(|pool| pool.alive()) && !profile_data::needs_reset(&self.profile()) {
            if let Some(pool) = &mut self.pool {
                pool.clear_website_data();
                for tab in &self.tabs { pool.reload(tab.view); }
            }
            return Ok(());
        }
        // IndexedDB, CacheStorage and HTTP authentication have no public clearing API in Servo 0.7.
        let urls: Vec<String> = self.tabs.iter().filter_map(|tab| self.pool.as_ref()?.state(tab.view).map(|state| state.url.clone())).collect();
        let selected = self.current;
        self.pool = None;
        let result = profile_data::reset(&self.profile());
        self.pageui = pageui::PageUi::default();
        self.tabs.clear();
        self.discard = Default::default();
        self.ctl = Default::default();
        self.restore = urls;
        self.restore_sel = selected;
        self.current = 0;
        self.start();
        result
    }

    pub(super) fn advance(&mut self, ctx: &Ctx, t: f64) -> bool {
        if self.dark != Some(ctx.cfg.dark) {
            self.place_web(ctx);
        }
        let Some(p) = &mut self.pool else { return false };
        let (events, frame) = p.poll();
        // heartbeat: ping the engine, and kill it when it stops answering (poll reports the exit; handle restarts)
        let now_s = self.clock.elapsed().as_secs_f64();
        if p.alive() {
            if p.take_pong() {
                self.watchdog.pong(now_s);
            }
            match self.watchdog.check(now_s) {
                session::recover::Action::SendPing => p.send_ping(now_s as u64),
                session::recover::Action::KillWedged => {
                    self.crash_log.push(session::recover::crash_line("", "engine wedged: no pong", self.storm.restarts_in_window()));
                    p.kill_engine();
                }
                session::recover::Action::None => {}
            }
        }
        let long_fired = self.long.is_some_and(|(t0, ..)| now_s - t0 >= LONG_PRESS_S);
        if long_fired {
            self.fire_long_press();
        }
        let committed: Vec<WebViewId> = events.iter().filter_map(|e| if let WebViewEvent::LoadCommitted(v) = e { Some(*v) } else { None }).collect();
        // a tab that committed a page other than the offline page (a data: URL) no longer waits for the network
        for v in &committed {
            let offline_page = self.pool.as_ref().and_then(|p| p.state(*v)).is_some_and(|s| s.url.starts_with("data:"));
            if !offline_page {
                self.net.forget(*v);
            }
        }
        let mut changed = !events.is_empty() || long_fired;
        for ev in &events {
            changed |= self.inspector.on_event(ev);
            self.ctl_on_event(ev);
            self.discard_event(ev);
        }
        for line in self.inspector.take_log() {
            ctx.log(&line);
        }
        for line in std::mem::take(&mut self.crash_log) {
            ctx.log(&line);
        }
        let fr = self.pool.as_ref().map(|p| p.frame()).unwrap_or_default();
        let visible = self.view();
        for ev in &events {
            self.uploads.on_event(ev, ctx, &mut self.pool, visible);
            let modal = self.pageui.active();
            changed |= self.pageui.on_event(ev, &mut self.pool, fr, &ctx.ui);
            if !modal && self.pageui.active() {
                self.main_menu.close();
                self.sheet.open = false;
                self.overview.open = false;
                self.downloads.open = false;
                self.lib.close_all(&mut self.store);
                self.editing = false;
                self.hover = None;
                self.sync_address();
                if let Some(pool) = self.pool.as_mut() { pool.mouse_leave(); }
            }
        }
        changed |= self.uploads.tick(&mut self.pool);
        self.handle(events);
        changed |= self.ctl_tick();
        self.inspector.flush_prefs(ctx);
        if self.ctl.take_relayout() | self.inspector.take_relayout() {
            self.place_web(ctx);
            changed = true;
        }
        for v in committed {
            if let Some(i) = self.tabs.iter().position(|t| t.view == v) {
                self.apply_zoom(i);
            }
        }
        self.save_session(t);
        let mut ready = Vec::new();
        self.sources.retain(|(url, slot)| match inbox::take(slot) {
            Some(src) => {
                ready.push(devmenu::source_document(url, &src));
                false
            }
            None => true,
        });
        let opened = !ready.is_empty();
        for doc in ready {
            self.new_tab(&doc);
        }
        let (dl_changed, stop) = self.downloads.tick();
        let dock = system::dock_progress(&self.downloads.items);
        if system::dock_changed(self.dock, dock) {
            self.dock = dock;
            system::send_dock(ctx, dock);
        }
        if let Some(p) = &mut self.pool {
            // Servo 0.7.0 cannot cancel a load (the engine's `stop` is a no-op), so leave the download's page:
            // back to where the link was, else a blank page
            for v in stop {
                if p.state(v).is_some_and(|st| st.can_go_back) {
                    p.go_back(v);
                } else {
                    p.load(v, "about:blank");
                }
            }
        }
        perf::tick(&mut self.store);
        self.discard_tick();
        frame || changed || opened || dl_changed
    }

    pub(super) fn suspend_window(&mut self, _ctx: &Ctx) {
        if let Some(p) = &mut self.pool {
            p.set_focused(false);
            p.set_suspended(true);
        }
        perf::on_exit(&mut self.store);
    }

    pub(super) fn resume_window(&mut self, ctx: &Ctx) {
        self.net_request = Some(network_path::request_path(ctx));
        // no pings ran while suspended: do not read the gap as a wedged engine
        self.watchdog.reset(self.clock.elapsed().as_secs_f64());
        if let Some(p) = &mut self.pool {
            p.set_focused(true);
            p.set_suspended(false);
        }
    }
}

/// The renderer to retry with after the engine exited during startup. The GPU engine can fail where the app
/// sandbox hides the device, and the prebuilt engine does not fall back by itself, so one startup failure with
/// the GPU requested is retried with software rendering. Never twice per session, never for a CPU failure.
pub(super) fn startup_retry(requested: WebRenderer, startup_error: bool, fell_back: bool) -> Option<WebRenderer> {
    (startup_error && !fell_back && requested == WebRenderer::Gpu).then_some(WebRenderer::Cpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_startup_error_retries_with_cpu() {
        assert_eq!(startup_retry(WebRenderer::Gpu, true, false), Some(WebRenderer::Cpu));
    }

    #[test]
    fn cpu_startup_error_shows_the_error() {
        assert_eq!(startup_retry(WebRenderer::Cpu, true, false), None);
    }

    #[test]
    fn second_failure_does_not_retry() {
        assert_eq!(startup_retry(WebRenderer::Gpu, true, true), None);
    }

    #[test]
    fn exit_without_startup_error_is_not_a_fallback() {
        assert_eq!(startup_retry(WebRenderer::Gpu, false, false), None);
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if self.private {
            self.pool = None;
            let _ = fs::remove_dir_all(private_profile());
        }
    }
}
