//! Tab ownership, address editing and navigation.
use super::*;

impl Browser {
    pub(super) fn clear_browsing_history(&mut self) {
        self.store.clear_history();
        self.discard.clear_history();
        if let Some(pool) = &mut self.pool {
            for tab in &self.tabs {
                pool.clear_session_history(tab.view);
            }
        }
    }
    pub(super) fn view(&self) -> Option<WebViewId> {
        self.tabs.get(self.current).map(|t| t.view)
    }

    /// Loads a bookmark or history entry in the current tab.
    pub(super) fn lib_open(&mut self, url: &str) {
        self.editing = false;
        if let Some(v) = self.view() {
            self.load_chosen(v, url);
        }
        self.sync_address();
    }

    pub(super) fn new_tab(&mut self, url: &str) {
        let Some(pool) = &mut self.pool else { return };
        let view = pool.create(url);
        self.tabs.push(Tab { view });
        self.session.dirty = true;
        self.select(self.tabs.len() - 1);
        if url == START {
            self.begin_editing();
        }
    }

    pub(super) fn select(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if self.current != i {
            self.uploads.cancel_pending(&mut self.pool);
            self.pageui.cancel_all(&mut self.pool);
        }
        self.current = i;
        self.session.dirty = true;
        self.editing = false;
        self.downloads.open = false;
        let Some(view) = self.tabs.get(i).map(|t| t.view) else { return };
        if let Some(p) = &mut self.pool {
            p.show(view);
        }
        self.discard_touch();
        self.sync_address();
    }

    pub(super) fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.session.dirty = true;
        self.remember_closed(i);
        let tab = self.tabs.remove(i);
        self.pageui.cancel_view(tab.view, &mut self.pool);
        self.uploads.forget(tab.view, &mut self.pool);
        self.inspector.forget(tab.view);
        self.pageui.forget(tab.view);
        self.discard.forget(tab.view);
        self.crash.forget(tab.view);
        self.reader.forget(tab.view);
        if let Some(p) = &mut self.pool {
            p.close(tab.view);
        }
        if self.tabs.is_empty() {
            self.new_tab(START);
        } else {
            self.select(self.current.min(self.tabs.len() - 1).saturating_sub(usize::from(i < self.current)));
        }
    }

    /// Cmd+1 ... Cmd+8 pick that tab, Cmd+9 the last one (Browser).
    pub(super) fn select_numbered(&mut self, n: usize) {
        let last = self.tabs.len().saturating_sub(1);
        self.select(if n >= 9 { last } else { (n - 1).min(last) });
    }

    /// Ctrl+Tab and Cmd+Shift+] go to the next tab, Ctrl+Shift+Tab and Cmd+Shift+[ to the previous one, wrapping.
    pub(super) fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 1 {
            self.select(if forward { (self.current + 1) % n } else { (self.current + n - 1) % n });
        }
    }

    /// Duplicates the current tab next to it.
    pub(super) fn duplicate_tab(&mut self) {
        let Some(url) = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| s.url.clone()) else { return };
        let at = self.current + 1;
        self.new_tab(&url);
        let last = self.tabs.len() - 1;
        if at < last {
            let t = self.tabs.remove(last);
            self.tabs.insert(at, t);
            self.select(at);
        }
    }

    /// Moves tab `from` to index `to`, keeping the selection on the same page.
    pub(super) fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.current = if self.current == from {
            to
        } else if from < self.current && self.current <= to {
            self.current - 1
        } else if to <= self.current && self.current < from {
            self.current + 1
        } else {
            self.current
        };
    }

    /// Loads an address the user chose. While the network is down the tab shows the offline page instead, and the
    /// address loads again when the network comes back. Campaign and click-identifier parameters (`utm_source`,
    /// `fbclid`, ...) are dropped from the address first (`bifrost_core::link_tracking`).
    pub(super) fn load_chosen(&mut self, view: WebViewId, url: &str) {
        let cleaned = bifrost_core::link_tracking::strip_tracking(url);
        let url = cleaned.url.as_str();
        let offline = self.net.offline();
        let Some(p) = self.pool.as_mut() else { return };
        if offline {
            self.net.hold(view, url);
            p.load(view, &system::offline_url(url));
        } else {
            self.net.forget(view);
            p.load(view, url);
        }
    }

    /// Applies a path status. Coming back online loads the tabs that were waiting for the network.
    pub(super) fn set_path(&mut self, status: Status) -> bool {
        if !self.net.set_status(status) || self.net.offline() {
            return false;
        }
        for (view, url) in self.net.take_waiting() {
            if let Some(p) = self.pool.as_mut().filter(|p| p.state(view).is_some()) {
                p.load(view, &url);
            }
        }
        true
    }

    /// Clicking the address field's star: bookmarks the open page, or removes its bookmark when it has one.
    pub(super) fn toggle_bookmark(&mut self) {
        let Some((url, title)) = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|st| (st.url.clone(), st.title.clone())) else { return };
        self.store.toggle_bookmark(&url, &title, store::now());
    }

    /// The address of the page in the current tab, when it is a web page (http or https).
    pub(super) fn web_url(&self) -> Option<String> {
        let v = self.view()?;
        let url = self.pool.as_ref()?.state(v)?.url.clone();
        (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
    }

    pub(super) fn sync_address(&mut self) {
        if self.editing {
            return;
        }
        let mut url = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| s.url.clone()).unwrap_or_default();
        if let Some(o) = self.view().and_then(|v| self.reader.original(v)) {
            url = o.to_string();
        }
        self.address.clear();
        if url != START {
            self.address.push_str(&url);
        }
    }

    pub(super) fn refresh_suggestions(&mut self) {
        // an empty field on the Start Page lists nothing: the Favorites are on the page already
        let listing = self.editing && !(self.address.is_empty() && self.start_visible());
        self.sugg = if listing { suggest::suggestions(&self.store, &self.address.to_string(), store::now()) } else { Vec::new() };
        self.sel = None;
    }

    pub(super) fn begin_editing(&mut self) {
        self.editing = true;
        let url = self.address.to_string();
        self.address.clear();
        self.address.push_str(&url);
        self.address.command(bifrost_app::controls::Command::SelectAll, "");
        self.refresh_suggestions();
    }

    pub(super) fn commit_address(&mut self) {
        let text = match self.sel.and_then(|i| self.sugg.get(i)) {
            Some(s) => s.target.clone(),
            None => self.address.to_string(),
        };
        self.editing = false;
        self.downloads.open = false;
        self.sugg.clear();
        self.sel = None;
        if text.trim().is_empty() {
            self.sync_address();
            return;
        }
        let url = self.prefs.url_for_input(&text);
        if let Some(v) = self.view() {
            self.load_chosen(v, &url);
        }
        self.address.clear();
        self.address.push_str(&url);
    }
}
