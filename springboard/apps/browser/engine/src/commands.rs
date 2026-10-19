//! App protocol dispatch.
use super::*;

impl Engine {

    /// Runs one command line; a panic inside it is reported and the engine carries on.
    pub(super) fn command(&mut self, line: &str) -> bool {
        // Never execute a truncated protocol command (especially an eval/userscript prefix).
        if line.len() > guard::MAX_LINE { return true; }
        let verb = line.split(' ').next().unwrap_or("").to_string();
        match guard::guarded(|| self.command_inner(line)) {
            Ok(keep) => keep,
            Err(msg) => {
                self.dirty = true;
                say(&format!("crashed 4294967295 command {verb} panicked: {}", escape_line(&msg).chars().take(200).collect::<String>()));
                true
            }
        }
    }

    pub(super) fn command_inner(&mut self, line: &str) -> bool {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let arg = |n: usize| rest.split(' ').nth(n).unwrap_or("");
        let id: u32 = arg(0).parse().unwrap_or(u32::MAX);
        let tail = |n: usize| rest.splitn(n + 1, ' ').nth(n).unwrap_or("").to_string();
        match verb {
            "size" => {
                let (w, h) = (dim(arg(0)), dim(arg(1)));
                // coalesced: only the last size of a burst is applied (apply_size)
                self.pending_size = Some((PhysicalSize::new(w, h), scale(arg(2))));
            }
            "new" => {
                if id == u32::MAX || self.views.contains_key(&id) || self.views.len() >= MAX_VIEWS {
                    return true;
                }
                // a new view takes its viewport from the context size when built (servo paint add_webview)
                self.apply_size();
                let Some(url) = Url::parse(&tail(1)).or_else(|_| Url::parse("about:blank")).ok() else { return true };
                let w = self.build(WebViewBuilder::new(&self.servo, self.rc.clone()).url(url));
                w.resize(self.size);
                // Servo builds views visible (paint webview_renderer hidden: false); every view draws into
                // the one shared scene, so only `show` may make one visible
                w.hide();
                self.next_id = self.next_id.max(id.saturating_add(1));
                self.views.insert(id, w);
            }
            "show" => {
                if let Some(old) = self.visible_view() {
                    old.hide();
                    old.set_focused(false);
                }
                if let Some(v) = self.views.get(&id) {
                    if !self.suspended {
                        v.show();
                        v.set_focused(true);
                    } else {
                        v.hide();
                    }
                    v.resize(self.size);
                    self.visible = Some(id);
                    self.dirty = true;
                }
            }
            "close" => {
                blocker::cancel_view(id);
                // Pending native controls keep the page alive until their responders are dropped.
                self.retire_prompts(Some(id));
                self.views.remove(&id);
                if self.visible == Some(id) {
                    self.visible = None;
                    self.dirty = false;
                }
                // last view gone: nothing is in flight any more, so a later view paints at once
                if self.views.is_empty() {
                    self.awaiting_ack = false;
                    self.pending_size = None;
                }
            }
            "load" => {
                if let (Some(v), Ok(url)) = (self.views.get(&id), Url::parse(&tail(1))) {
                    v.load(url);
                }
            }
            "back" => {
                if let Some(v) = self.views.get(&id).filter(|v| v.can_go_back()) {
                    v.go_back(1);
                }
            }
            "forward" => {
                if let Some(v) = self.views.get(&id).filter(|v| v.can_go_forward()) {
                    v.go_forward(1);
                }
            }
            "reload" => {
                if let Some(v) = self.views.get(&id) {
                    v.reload();
                }
            }
            // Servo 0.7.0 has no API to cancel a load; Stop is accepted and does nothing.
            "stop" => {}
            "file-reply" | "color-reply" | "unload-reply" => {
                let Ok(token) = arg(1).parse::<u32>() else { return true };
                self.picker_reply(verb, id, token, &tail(2));
            }
            "dialog-reply" => {
                let token: u32 = arg(1).parse().unwrap_or(0);
                if !self.controls.get(&token).is_some_and(|(view, _, pending)| *view == id && matches!(pending, Pending::Dialog(_))) { return true; }
                if let Some((_, _, Pending::Dialog(d))) = self.controls.remove(&token) {
                    if arg(2) == "ok" {
                        match d {
                            SimpleDialog::Prompt(mut p) => {
                                p.set_current_value(&unescape_line(&tail(3)));
                                p.confirm();
                            }
                            d => d.confirm(),
                        }
                    } else {
                        d.dismiss();
                    }
                    self.dirty = true;
                }
            }
            "menu-reply" => {
                let token: u32 = arg(1).parse().unwrap_or(0);
                let picks: Vec<usize> = rest.split(' ').skip(2).filter_map(|v| v.parse().ok()).collect();
                if !self.controls.get(&token).is_some_and(|(view, _, pending)| *view == id && matches!(pending, Pending::Menu(..) | Pending::Select(_))) { return true; }
                match self.controls.remove(&token) {
                    Some((_, _, Pending::Menu(m, actions))) => match picks.first().and_then(|i| actions.get(*i)) {
                        Some(a) => m.select(*a),
                        None => m.dismiss(),
                    },
                    Some((_, _, Pending::Select(mut s))) => {
                        s.select(picks);
                        s.submit();
                    }
                    Some(_) | None => {}
                }
                self.dirty = true;
            }
            // Servo 0.7.0 has no cache-bypass reload flag; the plain reload runs (gap noted in the README).
            "reload-hard" => {
                // Servo 0.7.0 has no cache-bypassing reload (WebView::reload sends Reload only; components/servo/webview.rs).
                // NetworkManager::clear_cache empties the whole in-memory HTTP cache (no per-origin API), then reload.
                self.servo.network_manager().clear_cache();
                if let Some(v) = self.views.get(&id) {
                    v.reload();
                }
            }
            "ua" => {
                let ua = if rest.is_empty() { DEFAULT_UA.to_string() } else { rest.to_string() };
                self.servo.set_preference("user_agent", servo::PrefValue::Str(ua));
            }
            "clipboard" => *self.delegate.clipboard.borrow_mut() = unescape_line(rest),
            "clear-history" => {
                if let Some(view) = self.views.get(&id) {
                    view.clear_session_history();
                }
            }
            "emulate" => return self.command_inner(&format!("size {rest}")),
            "theme" => {
                self.theme = if rest == "dark" { Theme::Dark } else { Theme::Light };
                for v in self.views.values() {
                    v.notify_theme_change(self.theme);
                }
            }
            "focus" => {
                if let Some(v) = self.visible_view() {
                    v.set_focused(rest == "1");
                }
            }
            "zoom" => {
                if let Some(v) = self.views.get(&id) {
                    v.set_page_zoom(arg(1).parse::<f32>().unwrap_or(1.0).clamp(0.3, 5.0));
                    self.dirty = true;
                }
            }
            "find" => self.find_in_page(id, arg(1), &tail(2)),
            "devtools" => self.devtools = rest == "on",
            "userscript" => devtools::set_user_script(&self.servo, rest),
            "rules-reset" => blocker::rules_reset(),
            "rules-add" => blocker::rules_add(&unescape_line(rest)),
            "rules-commit" => {
                blocker::rules_commit();
            }
            "navpolicy" => {
                blocker::set_ask_app(rest == "on");
                if rest != "on" {
                    blocker::release_all();
                }
            }
            "nav-reply" => {
                if let Some(url) = blocker::answer(arg(1).parse().unwrap_or(0), arg(2)) {
                    say(&format!("download {id} {url}"));
                }
            }
            "cookies" => {
                if let Some(url) = self.views.get(&id).and_then(|v| v.url()) {
                    let host = url.host_str().unwrap_or("").to_string();
                    let jar = self.servo.site_data_manager().cookies_for_url(url, servo::CookieSource::HTTP);
                    let records: Vec<String> = jar
                        .iter()
                        .filter_map(|c| {
                            let (name, value) = c.name_value();
                            let domain = c.domain().map(|d| d.to_string()).unwrap_or_else(|| host.clone());
                            let expires = c.expires_datetime().map(|t| t.unix_timestamp());
                            blocker::cookie_record(name, value, &domain, c.path().unwrap_or("/"), expires, c.secure().unwrap_or(false), c.http_only().unwrap_or(false))
                        })
                        .collect();
                    say(&blocker::cookies_line(id, arg(1), &records));
                }
            }
            "clear-cookies" => self.servo.site_data_manager().clear_cookies(None),
            "clear-site-data" => {
                let manager = self.servo.site_data_manager();
                let sites = manager.site_data(servo::StorageType::all());
                let names: Vec<String> = sites.iter().map(|site| site.name()).collect();
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                manager.clear_site_data(&names, servo::StorageType::all());
                manager.clear_cookies(None);
                self.servo.network_manager().clear_cache();
            }
            "eval" => {
                if self.delegate.evals.get() >= guard::MAX_EVALS {
                    say(&format!("evalresult {id} Error: too many evals in flight"));
                } else if let Some(v) = self.views.get(&id) {
                    self.delegate.evals.set(self.delegate.evals.get() + 1);
                    let delegate = self.delegate.clone();
                    v.evaluate_javascript(tail(1), move |r| {
                        let text = match r {
                            Ok(val) => format!("{val:?}"),
                            Err(e) => format!("Error: {e:?}"),
                        };
                        delegate.evals.set(delegate.evals.get().saturating_sub(1));
                        delegate.push(Note::Eval(id, text));
                    });
                }
            }
            "ack" => self.awaiting_ack = false,
            // heartbeat: the app sends `ping N` every 2 s and restarts an engine that stops answering
            "ping" => say(&format!("pong {}", arg(0).chars().take(20).collect::<String>())),
            // window suspended / resumed: stop painting, keep the views
            "suspend" => {
                self.suspended = true;
                if let Some(v) = self.visible_view() {
                    v.set_focused(false);
                    v.hide();
                }
            }
            "resume" => {
                self.suspended = false;
                if let Some(v) = self.visible_view() {
                    v.show();
                }
                self.awaiting_ack = false;
                self.dirty = true;
            }
            "dev" => self.dev_command(rest),
            "quit" => return false,
            _ => { self.input_command(verb, rest); }
        }
        true
    }
}
