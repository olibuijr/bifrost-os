//! Engine and system service callbacks.
use super::*;

impl Browser {
    pub(super) fn handle(&mut self, events: Vec<WebViewEvent>) {
        for ev in events {
            match ev {
                WebViewEvent::ClipboardChanged(text) => bifrost_app::kit::clipboard::set_text(&text),
                WebViewEvent::LoadStarted(v) | WebViewEvent::UrlChanged(v) if self.pool.as_ref().and_then(|p| p.state(v)).is_some() => {
                    self.session.dirty = true;
                    // a response the browser cannot display is saved instead (downloads.rs)
                    let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
                    self.downloads.ua = self
                        .ctl
                        .ua
                        .clone()
                        .or_else(|| self.dev.user_agent().map(String::from))
                        .or_else(|| std::env::var("BIFROST_BROWSER_UA").ok())
                        .unwrap_or_else(|| downloads::DEFAULT_UA.to_string());
                    self.downloads.consider(v, &url);
                    if Some(v) == self.view() {
                        self.sync_address();
                    }
                }
                WebViewEvent::TitleChanged(_) => {
                    self.session.dirty = true;
                }
                ev @ WebViewEvent::ScriptMessage { .. } => {
                    self.ctl.bridge.on_event(&ev);
                }
                WebViewEvent::Cookies { id, cookies, .. } => self.downloads.on_cookies(id, &cookies),
                WebViewEvent::FaviconChanged { id, rgba } => {
                    if let Some(url) = self.pool.as_ref().and_then(|pool| pool.state(id)).map(|state| state.url.clone()) {
                        self.favicons.on_native(id, &url, &rgba);
                    }
                }
                WebViewEvent::EvalResult(v, text) if self.autofill.on_eval(v, &text) => {}
                WebViewEvent::LoadFinished(v) => {
                    // cookies for the next download probe from this page (downloads.rs); password field (autofill.rs)
                    if let Some(p) = self.pool.as_mut() {
                        p.request_cookies(v, 0);
                        p.eval(v, autofill::FORM_JS);
                    }
                    let loaded = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone());
                    if let (Some(u), Some(p)) = (loaded, self.pool.as_mut()) {
                        self.reader.on_load(p, v, &u);
                    }
                    if let Some(s) = self.pool.as_ref().and_then(|p| p.state(v)) {
                        let (u, t) = (s.url.clone(), s.title.clone());
                        if !u.starts_with(reader::DATA_PREFIX) {
                            self.store.visit(&u, &t, store::now());
                        }
                    }
                }
                WebViewEvent::CreateWebView { id, .. } => {
                    self.session.dirty = true;
                    self.tabs.push(Tab { view: id });
                    self.select(self.tabs.len() - 1);
                }
                WebViewEvent::Closed(v) => {
                    self.session.dirty = true;
                    self.inspector.forget(v);
                    self.downloads.forget(v);
                    self.reader.forget(v);
                    self.favicons.forget(v);
                    self.autofill.forget(v);
                    if self.pool.as_ref().is_none_or(|p| p.state(v).is_none()) && !self.tabs.iter().any(|t| t.view == v) {
                        continue;
                    }
                    if let Some(i) = self.tabs.iter().position(|t| t.view == v) {
                        self.close_tab(i);
                    }
                }
                WebViewEvent::EngineExited => {
                    let startup_error = self.pool.as_ref().and_then(|p| p.startup_error()).map(str::to_owned);
                    self.session.dirty = true;
                    self.inspector.clear_all();
                    let urls: Vec<String> = self
                        .tabs
                        .iter()
                        .filter_map(|t| self.pool.as_ref().and_then(|p| p.state(t.view)).map(|s| s.url.clone()))
                        .filter(|u| !u.is_empty())
                        .collect();
                    let now_s = self.clock.elapsed().as_secs_f64();
                    let last = urls.first().cloned().unwrap_or_default();
                    self.pool = None;
                    self.pageui = pageui::PageUi::default();
                    self.tabs.clear();
                    self.restore = urls;
                    self.restore_sel = self.current;
                    self.current = 0;
                    let retry = lifecycle::startup_retry(self.prefs.renderer, startup_error.is_some(), self.renderer_fallback);
                    if let (Some(reason), Some(_)) = (&startup_error, retry) {
                        self.crash_log.push(format!("Engine initialization failed ({reason}); falling back to software rendering"));
                        self.renderer_fallback = true;
                        self.sheet.fallback = true;
                        self.start();
                    } else if let Some(reason) = startup_error {
                        self.crash_log.push(format!("Engine initialization failed: {reason}"));
                        self.error = Some(format!("{reason}. Open Browser Settings to select CPU fallback."));
                    } else if self.storm.allow_restart(now_s) {
                        let line = session::recover::crash_line(&last, "engine exited", self.storm.restarts_in_window());
                        self.crash_log.push(line);
                        self.start();
                    } else {
                        self.crash_log.push(session::recover::crash_line(&last, "restart storm: engine not restarted", self.storm.restarts_in_window()));
                        self.error = Some("The web engine stopped. Choose Reload or New Tab to restart it.".into());
                    }
                }
                WebViewEvent::ContentProcessTerminated(v, _) => {
                    self.pageui.forget(v);
                    // webViewWebContentProcessDidTerminate: reload only this tab and say so in a banner (crashbanner.rs);
                    // a tab that keeps crashing is left alone behind a Reload button
                    let now = self.clock.elapsed().as_secs_f64();
                    if self.crash.on_crash(v, now) {
                        if let Some(p) = &mut self.pool {
                            p.reload(v);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn on_service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        // AutoFill: the logins listing, then the chosen password, go into the page (autofill.rs)
        if let Some((view, js)) = self.autofill.on_reply(ctx, request, result) {
            if let Some(p) = self.pool.as_mut() {
                p.eval(view, &js);
            }
            return false;
        }
        if self.net_request == Some(request) {
            self.net_request = None;
            match network_path::decode_reply(result.clone()) {
                Ok(path) => return self.set_path(path.status),
                Err(e) => ctx.log(&format!("network path unavailable: {e}")),
            }
            return false;
        }
        // a copy the service refused (an older shell) goes to the synchronous pasteboard instead
        if let Some((id, clip)) = self.clip.take() {
            if id == request {
                if result.is_err() {
                    system::fall_back(ctx, &clip);
                }
            } else {
                self.clip = Some((id, clip));
            }
        }
        false
    }

    pub(super) fn on_service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match network_path::path_event(service, event, body) {
            Some(Ok(path)) => self.set_path(path.status),
            _ => false,
        }
    }
}
