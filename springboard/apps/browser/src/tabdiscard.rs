//! Background tab throttling and discarding (Browser: idle background tabs are unloaded).
//!
//! Throttling: Servo 0.7.0 already throttles hidden views. `WebView::hide` -> paint `hide_webview` ->
//! `EmbedderToConstellationMessage::SetWebViewHidden` -> constellation `set_webview_hidden` ->
//! `Pipeline::send_throttle_messages` -> `ScriptThreadMessage::SetThrottled` (components/constellation/
//! constellation.rs, pipeline.rs). The engine hides every view but the shown one, so background tabs already run
//! throttled and do not render; nothing more is needed in the app.
//!
//! Discarding: a background tab idle for `idle_s`, or the least recently used ones while the engine tree is over
//! `budget_kib`, has its engine view closed (`WebProcessPool::discard`). The tab keeps its id, URL, title and scroll
//! position; selecting it (or any control or inspector call that loads it) re-creates the view and reloads.
//! `BIFROST_TAB_DISCARD_MIN` (minutes, 0 = off) and `BIFROST_TAB_BUDGET_MIB` (0 = off) override the defaults.
use std::collections::HashMap;

use bifrost_app::webview::{WebViewEvent, WebViewId};
use bifrost_app::AppMenuItem;

use crate::{perf, Browser, START};

/// Prefix of the scroll probe's eval result.
pub const SCROLL_PREFIX: &str = "bifrost-scroll:";
/// First menu item id of the History menu: item `HISTORY_BASE + i` goes to entry `i` of the tab's list.
pub const HISTORY_BASE: u32 = 1000;
/// Entries shown on each side of the current one in the History menu.
const MENU_LIMIT: usize = 10;
/// Characters of a URL kept in a History menu label.
const LABEL_CHARS: usize = 60;

/// The History menu entry for an item id, when the id is one of ours.
pub fn history_target(id: u32) -> Option<usize> {
    let i = id.checked_sub(HISTORY_BASE)? as usize;
    (i < MAX_HIST).then_some(i)
}

/// The entries the History menu offers around the current one: `(index, url)` pairs, nearest first, at most `limit`
/// on each side. Back entries come before the current one, forward entries after it.
pub fn menu_entries(list: &[String], index: usize, limit: usize) -> (Vec<(usize, String)>, Vec<(usize, String)>) {
    let back = (0..index.min(list.len())).rev().take(limit).map(|i| (i, list[i].clone())).collect();
    let forward = (index + 1..list.len()).take(limit).map(|i| (i, list[i].clone())).collect();
    (back, forward)
}

/// A menu label for a URL: the address without the scheme and the trailing slash, cut to `LABEL_CHARS` characters.
pub fn history_label(url: &str) -> String {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return url.to_string();
    }
    let mut out: String = rest.chars().take(LABEL_CHARS).collect();
    if rest.chars().count() > LABEL_CHARS {
        out.push('\u{2026}');
    }
    out
}
/// Seconds between sweeps.
const SWEEP_S: f64 = 30.0;
/// Seconds to wait for a scroll probe before discarding without it.
const PROBE_TIMEOUT_S: f64 = 2.0;
/// Entries of app-side history kept per tab.
const MAX_HIST: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    /// Background seconds before a tab is discarded; `None` = never.
    pub idle_s: Option<f64>,
    /// Engine tree PSS (KiB) above which least recently used background tabs go; `None` = no budget.
    pub budget_kib: Option<u64>,
    /// The most recently used background tabs the budget never discards.
    pub keep_recent: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { idle_s: Some(15.0 * 60.0), budget_kib: Some(1536 * 1024), keep_recent: 2 }
    }
}

impl Policy {
    pub fn from_env() -> Self {
        let num = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<u64>().ok());
        let mut p = Policy::default();
        if let Some(m) = num("BIFROST_TAB_DISCARD_MIN") {
            p.idle_s = (m > 0).then_some(m as f64 * 60.0);
        }
        if let Some(m) = num("BIFROST_TAB_BUDGET_MIB") {
            p.budget_kib = (m > 0).then_some(m * 1024);
        }
        p
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub id: WebViewId,
    /// Seconds (app clock) when the tab was last the selected one.
    pub last_active: f64,
    pub current: bool,
    pub discarded: bool,
    /// Loading, blank or otherwise not worth or safe to discard.
    pub protected: bool,
}

/// Tabs to discard now, oldest first. Idle ones always; over budget, one more least recently used tab per call
/// (memory is re-measured on the next sweep). Never the current tab, a discarded or protected one, nor the
/// `keep_recent` most recently used background tabs when over budget.
pub fn pick(policy: &Policy, now: f64, tabs: &[Candidate], total_kib: u64) -> Vec<WebViewId> {
    let mut live: Vec<&Candidate> = tabs.iter().filter(|c| !c.current && !c.discarded && !c.protected).collect();
    live.sort_by(|a, b| a.last_active.total_cmp(&b.last_active));
    let mut out: Vec<WebViewId> = Vec::new();
    if let Some(idle) = policy.idle_s {
        out.extend(live.iter().filter(|c| now - c.last_active >= idle).map(|c| c.id));
    }
    if policy.budget_kib.is_some_and(|b| total_kib > b) {
        let n = live.len().saturating_sub(policy.keep_recent);
        if let Some(c) = live[..n].iter().find(|c| !out.contains(&c.id)) {
            out.push(c.id);
        }
    }
    out
}

#[derive(Default)]
pub struct Discarder {
    policy: Option<Policy>,
    last_active: HashMap<WebViewId, f64>,
    scroll: HashMap<WebViewId, (f64, f64)>,
    /// Scroll to restore once this (revived) view finishes loading.
    restore: HashMap<WebViewId, (f64, f64)>,
    /// App-side history per tab: entries and the index of the current one. A revived view only has its last URL in
    /// the engine (Servo 0.7.0 cannot inject history), so Back past it loads the entry before it from here.
    hist: HashMap<WebViewId, (Vec<String>, usize)>,
    /// A History menu jump in progress: the entry index the tab is moving to, one engine step per finished load.
    jumps: HashMap<WebViewId, usize>,
    last_sweep: f64,
    /// Views waiting for their scroll probe: (id, sent at).
    probing: Vec<(WebViewId, f64)>,
}

impl Discarder {
    pub fn touch(&mut self, id: WebViewId, now: f64) {
        self.last_active.insert(id, now);
    }

    pub fn forget(&mut self, id: WebViewId) {
        self.last_active.remove(&id);
        self.scroll.remove(&id);
        self.restore.remove(&id);
        self.hist.remove(&id);
        self.jumps.remove(&id);
        self.probing.retain(|(v, _)| *v != id);
    }

    /// Follows the tab's URL: the same URL is a no-op, the previous or next entry moves the index (engine back or
    /// forward), anything else is a new entry that drops the forward ones. Returns true when a new entry was made.
    pub fn track(&mut self, id: WebViewId, url: &str) -> bool {
        if url.is_empty() || url.starts_with("data:") {
            return false;
        }
        let (list, i) = self.hist.entry(id).or_default();
        if list.is_empty() {
            list.push(url.to_string());
            *i = 0;
            false
        } else if list[*i] == url {
            false
        } else if *i > 0 && list[*i - 1] == url {
            *i -= 1;
            false
        } else if list.get(*i + 1).is_some_and(|u| u == url) {
            *i += 1;
            false
        } else {
            list.truncate(*i + 1);
            list.push(url.to_string());
            if list.len() > MAX_HIST {
                list.remove(0);
            }
            *i = list.len() - 1;
            true
        }
    }

    /// The tab's app-side list and the index of its current entry.
    pub fn entries(&self, id: WebViewId) -> Option<(&[String], usize)> {
        let (list, i) = self.hist.get(&id)?;
        Some((list.as_slice(), *i))
    }

    pub fn set_jump(&mut self, id: WebViewId, target: usize) {
        self.jumps.insert(id, target);
    }

    pub fn jump_target(&self, id: WebViewId) -> Option<usize> {
        self.jumps.get(&id).copied()
    }

    pub fn clear_jump(&mut self, id: WebViewId) {
        self.jumps.remove(&id);
    }

    /// The URL Back should load when the engine has nothing behind the current page (a revived tab).
    pub fn back_target(&self, id: WebViewId) -> Option<&str> {
        let (list, i) = self.hist.get(&id)?;
        list.get(i.checked_sub(1)?).map(|s| s.as_str())
    }

    pub fn probing(&self) -> bool {
        !self.probing.is_empty()
    }

    /// Records probe answers; returns the JS that restores a revived page's scroll once it has loaded.
    pub fn on_event(&mut self, ev: &WebViewEvent) -> Option<(WebViewId, String)> {
        match ev {
            WebViewEvent::EvalResult(id, text) => {
                let rest = text.trim().trim_matches('"').strip_prefix(SCROLL_PREFIX)?;
                let (x, y) = rest.split_once(',')?;
                self.scroll.insert(*id, (x.parse().ok()?, y.parse().ok()?));
                None
            }
            WebViewEvent::LoadFinished(id) => {
                let (x, y) = self.restore.remove(id)?;
                Some((*id, format!("window.scrollTo({x},{y})")))
            }
            _ => None,
        }
    }
}

impl Browser {
    /// Marks the selected tab as active now (`select`).
    pub(crate) fn discard_touch(&mut self) {
        if let Some(v) = self.view() {
            let now = self.clock.elapsed().as_secs_f64();
            self.discard.touch(v, now);
        }
    }

    /// Whether Back can go: the engine has an entry, or the app-side history has one before a revived page.
    pub(crate) fn can_back(&self, v: WebViewId) -> bool {
        self.pool.as_ref().and_then(|p| p.state(v)).is_some_and(|s| s.can_go_back) || self.discard.back_target(v).is_some()
    }

    pub(crate) fn discard_event(&mut self, ev: &WebViewEvent) {
        if let WebViewEvent::UrlChanged(v) = ev {
            let url = self.pool.as_ref().and_then(|p| p.state(*v)).map(|s| s.url.clone()).unwrap_or_default();
            // a page the user went to from here (not a Back or Forward step) ends a History menu jump
            if self.discard.track(*v, &url) {
                self.discard.clear_jump(*v);
            }
        }
        if let WebViewEvent::LoadFinished(v) = ev {
            self.jump_step(*v);
        }
        if let Some((id, js)) = self.discard.on_event(ev) {
            if let Some(p) = &mut self.pool {
                p.eval(id, &js);
            }
        }
    }

    /// One Back step: the engine's previous entry, or for a revived tab the app-side previous entry (the Back
    /// button's rule).
    pub(crate) fn step_back(&mut self, v: WebViewId) {
        let engine = self.pool.as_ref().and_then(|p| p.state(v)).is_some_and(|s| s.can_go_back);
        let fallback = self.discard.back_target(v).map(str::to_string);
        let Some(p) = self.pool.as_mut() else { return };
        match fallback.filter(|_| !engine) {
            Some(url) => p.load(v, &url),
            None => p.go_back(v),
        }
    }

    /// History menu: moves the selected tab to entry `target` of its app-side list, one engine step at a time.
    pub(crate) fn jump_to(&mut self, target: usize) {
        let Some(v) = self.view() else { return };
        let len = self.discard.entries(v).map_or(0, |(list, _)| list.len());
        if target >= len {
            return;
        }
        self.discard.set_jump(v, target);
        self.jump_step(v);
    }

    /// Takes the next engine step of a History menu jump. Called when the jump starts and after each finished load;
    /// the jump ends at its target, or when the engine has no step left in that direction.
    pub(crate) fn jump_step(&mut self, v: WebViewId) {
        let Some(target) = self.discard.jump_target(v) else { return };
        let Some(cur) = self.discard.entries(v).map(|(_, i)| i) else {
            self.discard.clear_jump(v);
            return;
        };
        if cur == target {
            self.discard.clear_jump(v);
            return;
        }
        let (engine_back, engine_fwd) = match self.pool.as_ref().and_then(|p| p.state(v)) {
            Some(s) => (s.can_go_back, s.can_go_forward),
            None => (false, false),
        };
        let back = cur > target;
        let can = if back { engine_back || self.discard.back_target(v).is_some() } else { engine_fwd };
        if !can {
            self.discard.clear_jump(v);
            return;
        }
        if back {
            self.step_back(v);
        } else if let Some(p) = self.pool.as_mut() {
            p.go_forward(v);
        }
    }

    /// The History menu: Back and Forward submenus of the selected tab's entries, nearest first. Item ids are
    /// `HISTORY_BASE` plus the entry index (see `jump_to`).
    pub(crate) fn history_menu(&self) -> Vec<AppMenuItem> {
        let none: &[String] = &[];
        let (list, index) = self.view().and_then(|v| self.discard.entries(v)).unwrap_or((none, 0));
        let (back, forward) = menu_entries(list, index, MENU_LIMIT);
        let mut items = vec![AppMenuItem::submenu("Back")];
        if back.is_empty() {
            items.push(AppMenuItem::new(0, "No Earlier Pages").enabled(false).depth(1));
        }
        for (i, url) in back {
            items.push(AppMenuItem::new(HISTORY_BASE + i as u32, &history_label(&url)).depth(1));
        }
        items.push(AppMenuItem::separator());
        items.push(AppMenuItem::submenu("Forward"));
        if forward.is_empty() {
            items.push(AppMenuItem::new(0, "No Later Pages").enabled(false).depth(1));
        }
        for (i, url) in forward {
            items.push(AppMenuItem::new(HISTORY_BASE + i as u32, &history_label(&url)).depth(1));
        }
        items
    }

    /// Called from `tick`: discards tabs whose probe answered or timed out, and every `SWEEP_S` picks new ones.
    pub(crate) fn discard_tick(&mut self) {
        let now = self.clock.elapsed().as_secs_f64();
        let Some(p) = self.pool.as_mut().filter(|p| p.alive()) else { return };
        let d = &mut self.discard;
        let ready: Vec<WebViewId> = d.probing.iter().filter(|(v, at)| d.scroll.contains_key(v) || now - at >= PROBE_TIMEOUT_S).map(|(v, _)| *v).collect();
        for v in ready {
            d.probing.retain(|(x, _)| *x != v);
            if p.discard(v) {
                if let Some(s) = d.scroll.get(&v).copied().filter(|s| *s != (0.0, 0.0)) {
                    d.restore.insert(v, s);
                }
            }
        }
        if now - d.last_sweep < SWEEP_S {
            return;
        }
        d.last_sweep = now;
        let policy = *d.policy.get_or_insert_with(Policy::from_env);
        let mut cands = Vec::with_capacity(self.tabs.len());
        for (i, t) in self.tabs.iter().enumerate() {
            let st = p.state(t.view);
            let blank = st.is_none_or(|s| s.url.is_empty() || s.url == START || s.url == "about:blank");
            cands.push(Candidate {
                id: t.view,
                last_active: *d.last_active.entry(t.view).or_insert(now),
                current: i == self.current,
                discarded: p.is_discarded(t.view),
                protected: blank || st.is_some_and(|s| s.loading) || d.probing.iter().any(|(v, _)| *v == t.view),
            });
        }
        let kib = if policy.budget_kib.is_some() { perf::tree_pss_kib(p.pid()) } else { 0 };
        for id in pick(&policy, now, &cands, kib) {
            d.scroll.remove(&id);
            p.eval(id, &format!("'{SCROLL_PREFIX}'+Math.round(scrollX)+','+Math.round(scrollY)"));
            d.probing.push((id, now));
        }
    }

    /// Milliseconds until the next timed work: the watchdog ping while the engine runs (2 s), sooner while a scroll
    /// probe is out. The runtime does not ask while the window is hidden, so a hidden window sleeps.
    pub(crate) fn next_timed_ms(&self) -> Option<f64> {
        if self.long.is_some() {
            return Some(100.0); // a long-press is timed in tick
        }
        let alive = self.pool.as_ref().is_some_and(|p| p.alive());
        alive.then(|| if self.discard.probing() { 250.0 } else { crate::session::recover::PING_EVERY_S * 1000.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: u32, last: f64) -> Candidate {
        Candidate { id, last_active: last, current: false, discarded: false, protected: false }
    }
    fn pol() -> Policy {
        Policy { idle_s: Some(600.0), budget_kib: Some(1000), keep_recent: 1 }
    }

    #[test]
    fn idle_tabs_go_oldest_first() {
        let t = [c(1, 100.0), c(2, 50.0), c(3, 900.0)];
        assert_eq!(pick(&pol(), 1000.0, &t, 0), vec![2, 1]);
    }

    #[test]
    fn current_discarded_and_protected_are_kept() {
        let mut cur = c(1, 0.0);
        cur.current = true;
        let mut gone = c(2, 0.0);
        gone.discarded = true;
        let mut busy = c(3, 0.0);
        busy.protected = true;
        assert!(pick(&pol(), 9999.0, &[cur, gone, busy], 5000).is_empty());
    }

    #[test]
    fn over_budget_drops_one_lru_but_keeps_recent() {
        let t = [c(1, 10.0), c(2, 20.0), c(3, 30.0)];
        assert_eq!(pick(&pol(), 40.0, &t, 2000), vec![1]);
        assert!(pick(&pol(), 40.0, &[c(3, 30.0)], 2000).is_empty());
        assert!(pick(&pol(), 40.0, &t, 500).is_empty());
    }

    #[test]
    fn budget_does_not_double_count_idle_picks() {
        let t = [c(1, 0.0), c(2, 700.0), c(3, 800.0)];
        assert_eq!(pick(&pol(), 1000.0, &t, 2000), vec![1, 2]);
    }

    #[test]
    fn disabled_policy_discards_nothing() {
        let p = Policy { idle_s: None, budget_kib: None, keep_recent: 0 };
        assert!(pick(&p, 1e9, &[c(1, 0.0)], u64::MAX).is_empty());
    }

    #[test]
    fn app_history_survives_and_steps_back() {
        let mut d = Discarder::default();
        for u in ["a", "b", "c"] {
            d.track(1, u);
        }
        d.track(1, "c"); // revive reloads the same URL
        assert_eq!(d.back_target(1), Some("b"));
        d.track(1, "b"); // Back loaded it
        assert_eq!(d.back_target(1), Some("a"));
        d.track(1, "x"); // new entry drops forward ones
        assert_eq!(d.back_target(1), Some("b"));
        assert_eq!(d.back_target(2), None);
    }

    #[test]
    fn scroll_probe_is_recorded_and_restored_after_load() {
        let mut d = Discarder::default();
        d.probing.push((7, 0.0));
        assert!(d.on_event(&WebViewEvent::EvalResult(7, format!("{SCROLL_PREFIX}12,340"))).is_none());
        assert_eq!(d.scroll.get(&7), Some(&(12.0, 340.0)));
        d.restore.insert(7, (12.0, 340.0));
        assert_eq!(d.on_event(&WebViewEvent::LoadFinished(7)), Some((7, "window.scrollTo(12,340)".into())));
        assert!(d.on_event(&WebViewEvent::LoadFinished(7)).is_none());
    }

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|u| u.to_string()).collect()
    }

    #[test]
    fn menu_lists_back_nearest_first_and_forward_after_current() {
        let list = s(&["a", "b", "c", "d"]);
        // current is "b" (index 1): one back entry, two forward entries
        let (back, fwd) = menu_entries(&list, 1, 2);
        assert_eq!(back, vec![(0, "a".to_string())]);
        assert_eq!(fwd, vec![(2, "c".to_string()), (3, "d".to_string())]);
        // current is "d" with a limit of one: the nearest back entry only
        let (back, fwd) = menu_entries(&list, 3, 1);
        assert_eq!(back, vec![(2, "c".to_string())]);
        assert!(fwd.is_empty());
        // no list: both sides empty
        let (back, fwd) = menu_entries(&[], 0, 10);
        assert!(back.is_empty() && fwd.is_empty());
    }

    #[test]
    fn menu_limit_keeps_the_nearest_entries() {
        let list = s(&["a", "b", "c", "d", "e"]);
        let (back, _) = menu_entries(&list, 4, 2);
        assert_eq!(back, vec![(3, "d".to_string()), (2, "c".to_string())]);
    }

    #[test]
    fn menu_labels_drop_scheme_and_trailing_slash() {
        assert_eq!(history_label("https://example.com/a/"), "example.com/a");
        assert_eq!(history_label("http://x"), "x");
        assert_eq!(history_label("https://"), "https://");
        let long = format!("https://{}", "a".repeat(70));
        assert_eq!(history_label(&long), format!("{}\u{2026}", "a".repeat(60)));
    }

    #[test]
    fn history_menu_ids_map_back_to_entries() {
        assert_eq!(history_target(HISTORY_BASE), Some(0));
        assert_eq!(history_target(HISTORY_BASE + 49), Some(49));
        assert_eq!(history_target(HISTORY_BASE + 50), None);
        assert_eq!(history_target(HISTORY_BASE - 1), None);
        assert_eq!(history_target(1), None);
    }

    #[test]
    fn track_reports_only_new_entries() {
        let mut d = Discarder::default();
        assert!(!d.track(1, "a")); // first page: the list starts
        assert!(d.track(1, "b")); // a new entry
        assert!(!d.track(1, "a")); // Back step
        assert!(!d.track(1, "b")); // Forward step
        assert!(d.track(1, "c"));
        assert_eq!(d.entries(1), Some((s(&["a", "b", "c"]).as_slice(), 2)));
        assert!(!d.track(1, "")); // empty and data URLs are never entries
        assert!(!d.track(1, "data:text/html,x"));
        assert_eq!(d.entries(2), None);
    }

    #[test]
    fn jump_target_is_kept_until_cleared_or_forgotten() {
        let mut d = Discarder::default();
        assert_eq!(d.jump_target(1), None);
        d.set_jump(1, 0);
        assert_eq!(d.jump_target(1), Some(0));
        d.clear_jump(1);
        assert_eq!(d.jump_target(1), None);
        d.set_jump(1, 2);
        d.forget(1);
        assert_eq!(d.jump_target(1), None);
    }
}
