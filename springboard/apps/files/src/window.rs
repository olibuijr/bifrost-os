//! The browser window controller: layout of sidebar, toolbar, content and status bar, focus, selection state and navigation
//! (Files's FileManagerWindowController).
use crate::*;

pub(crate) const HEADER_H: f32 = 60.;

/// How far a press must move before it becomes a drag.
pub(crate) const DRAG_SLOP: f32 = 12.;

impl Files {
    pub(crate) fn navigate(&mut self, path: PathBuf, remember: bool) {
        if remember {
            self.history.truncate(if self.history.is_empty() {
                0
            } else {
                self.at + 1
            });
            self.history.push(path.clone());
            self.at = self.history.len() - 1;
        }
        self.revision += 1;
        self.path = path.clone();
        // the extensions that watch the folder hear that it is shown
        self.ext.borrow_mut().show(&path);
        self.loading = true;
        self.error = None;
        self.notice = None;
        self.selection.clear();
        self.scroll = 0.;
        self.query.clear();
        self.preview = None;
        self.naming = None;
        self.stop_search();
        self.listing.clear();
        self.refresh_rows();
        self.rewatch();
        let (tx, rx) = feed();
        self.job = Some(rx);
        let roots = vec![self.home.clone()];
        let tagged = tag_filter(&path).map(|only| tagged_entries(&self.tag_index, only));
        let smart = smart::folder_index(&path).and_then(|i| self.smart.get(i)).map(|f| (f.clone(), self.tag_index.clone()));
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        // the stand-in listings have no folder above them to show
        let in_provider = provider::is_path(&path);
        let virtual_folder = path == Path::new(fs::RECENTS) || path.starts_with(TAGGED) || smart::folder_index(&path).is_some() || path == self.bin() || in_provider;
        // a provider folder is listed by its extension, which the host starts if it is not running
        let providers = in_provider.then(provider::shared);
        let (sort, bin) = (self.sort, self.bin());
        let chain = if virtual_folder { vec![] } else { ancestor_chain(&path) };
        let work = move || {
            let batch_path = path.clone();
            let result = if let Some(list) = tagged {
                Ok(list)
            } else if let Some((folder, index)) = smart {
                Ok(folder.results(&index, now))
            } else if path == Path::new(fs::RECENTS) {
                Ok(fs::recent_in(&roots, fs::RECENT_LIMIT))
            } else if let Some(providers) = providers {
                providers.list(&path)
            } else {
                // a folder's rows go to the UI as they are read, so a large folder shows its first rows at once
                fs::list_batched(&path, |batch| {
                    let _ = tx.send(Listing { path: batch_path.clone(), result: Ok(batch.to_vec()), ancestors: vec![], partial: true });
                })
                .map_err(|e| e.to_string())
            };
            let (in_bin, keep_order) = (path == bin, path == Path::new(fs::RECENTS) || path.starts_with(TAGGED));
            let result = result.map(|mut rows| {
                settle(&mut rows, sort, in_bin, keep_order);
                rows
            });
            let ancestors = chain
                .into_iter()
                .filter_map(|folder| {
                    fs::list_batched(&folder, |_| {}).ok().map(|mut list| {
                        order(&mut list, sort);
                        (folder, list)
                    })
                })
                .collect();
            let _ = tx.send(Listing { path, result, ancestors, partial: false });
        };
        // Under the test pin the listing is read at once, so a snapshot never catches "Loading…" on a busy machine.
        if bifrost_app::testing::pinned() {
            work();
        } else {
            DispatchQueue::global(Qos::Utility).async_(work);
        }
    }
    pub(crate) fn in_bin(&self) -> bool {
        self.path == self.bin()
    }
    /// Recently Deleted, the hidden trash folder under the home folder.
    pub(crate) fn bin(&self) -> PathBuf {
        trash::dir(&self.home)
    }
    /// The sidebar column's width: 0 while hidden.
    pub(crate) fn side(&self) -> f32 {
        if self.sidebar_hidden { 0. } else { SIDEBAR_W }
    }
    pub(crate) fn toggle_sidebar(&mut self) {
        self.sidebar_hidden = !self.sidebar_hidden;
        self.revision += 1;
    }
    pub(crate) fn toggle_status_bar(&mut self) {
        self.status_bar = !self.status_bar;
        self.revision += 1;
    }
    /// The footer's height: the status bar when it is on, or while a job, notice or drag needs its line.
    pub(crate) fn footer_h(&self) -> f32 {
        if self.status_bar || self.work.is_some() || self.notice.is_some() || self.dragging { FOOTER_H } else { 0. }
    }
    /// The bottom of the content area.
    pub(crate) fn bottom(&self) -> f32 {
        self.h - self.footer_h()
    }
    /// The focused row: the last one picked.
    pub(crate) fn focus(&self) -> Option<usize> {
        self.selection.last().copied()
    }
    pub(crate) fn focus_entry(&self) -> Option<&Entry> {
        self.focus().and_then(|i| self.rows.get(i))
    }
    pub(crate) fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection.iter().filter_map(|&i| self.rows.get(i)).map(|e| e.path.clone()).collect()
    }
    pub(crate) fn select_only(&mut self, i: usize) {
        self.selection = vec![i];
    }
    pub(crate) fn select_all(&mut self) {
        self.selection = self.visible();
    }
    pub(crate) fn toggle_selected(&mut self, i: usize) {
        if let Some(at) = self.selection.iter().position(|&s| s == i) {
            self.selection.remove(at);
        } else {
            self.selection.push(i);
        }
    }
    pub(crate) fn open(&mut self, i: usize) {
        let Some(e) = self.rows.get(i).cloned() else {
            return;
        };
        if provider::is_path(&e.path) {
            self.open_provider_item(e);
            return;
        }
        self.index_opened(&e);
        if e.is_dir {
            self.navigate(e.path, true)
        } else if Self::is_archive(&e) && !self.in_bin() {
            // opening an archive expands it next to itself
            let dest = e.path.parent().map(Path::to_path_buf).unwrap_or_else(|| self.path.clone());
            self.start_job("Expanding", JobKind::Extract, vec![e.path], dest);
        } else if Self::is_disk_image(&e) {
            // a disk image mounts read-only (`hdiutil attach` prints the loop device and the folder), then the folder opens here
            let path = e.path;
            DispatchQueue::global(Qos::Utility).async_(move || {
                let Ok(out) = std::process::Command::new("hdiutil").arg("attach").arg(&path).output() else {
                    return;
                };
                if !out.status.success() {
                    return;
                }
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                if let Some((_, point)) = text.trim().split_once(' ') {
                    let _ = std::process::Command::new("/usr/bin/bifrost-apps").arg("--open").arg(point).status();
                }
            });
        } else {
            let path = e.path;
            DispatchQueue::global(Qos::Utility).async_(move || {
                let _ = std::process::Command::new("/usr/bin/bifrost-apps")
                    .arg("--open")
                    .arg(path)
                    .status();
            });
        }
    }
    pub(crate) fn go_back(&mut self) {
        if self.at > 0 {
            self.at -= 1;
            self.navigate(self.history[self.at].clone(), false)
        }
    }
    pub(crate) fn go_forward(&mut self) {
        if self.at + 1 < self.history.len() {
            self.at += 1;
            self.navigate(self.history[self.at].clone(), false)
        }
    }
    pub(crate) fn go_up(&mut self) {
        if let Some(parent) = self.path.parent().map(Path::to_path_buf) {
            self.navigate(parent, true)
        }
    }
    pub(crate) fn move_focus(&mut self, down: bool) {
        let v = self.visible();
        let at = self.focus().and_then(|f| v.iter().position(|&j| j == f)).unwrap_or(0);
        let next = if down { (at + 1).min(v.len().saturating_sub(1)) } else { at.saturating_sub(1) };
        if let Some(&i) = v.get(next) {
            self.select_only(i);
        }
    }
    pub(crate) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.selection.clear();
        self.scroll = 0.;
    }
    pub(crate) fn set_view(&mut self, grid: bool, columns: bool) {
        self.grid = grid;
        self.columns = columns;
        self.vo.gallery = false;
        self.scroll = 0.;
        self.refresh_rows(); // inline folders only exist in the list view
        self.revision += 1;
    }
    pub(crate) fn update_controls(&mut self) {
        let make=|id,role,label:&str,bounds,enabled|Node{id,role,label:label.into(),value:if id==BAR_SEARCH as u64{self.query.to_string()}else{String::new()},bounds,enabled};
        let bar = self.bar();
        let placed = self.bar_layout();
        let mut nodes = vec![];
        for (id, r) in &placed.visible {
            if let Some(item) = bar.items.iter().find(|i| i.id == *id) {
                match *id {
                    BAR_TITLE => {}
                    BAR_SEARCH => nodes.push(make(*id as u64, Role::TextField, "Search", self.search_rect().unwrap_or(*r), true)),
                    _ => nodes.push(make(*id as u64, Role::Button, &item.label, *r, item.enabled)),
                }
            }
        }
        self.controls.update(nodes);
    }
    pub(crate) fn click(&mut self, x: f32, y: f32, t: f64) {
        if self.tag_panel {
            // a colour row toggles that colour on the selection; anywhere else closes the panel
            let row = ((y - TAG_ROWS_TOP) / TAG_ROW).floor();
            if x >= (self.side() + 16.) && x < self.w - 16. && (0. ..Tag::ALL.len() as f32).contains(&row) && y >= TAG_ROWS_TOP {
                self.toggle_tag(Tag::ALL[row as usize]);
            } else {
                self.tag_panel = false;
                self.revision += 1;
            }
            return;
        }
        if self.work.as_ref().is_some_and(|w| w.conflict.is_some()) {
            if (PROMPT_TOP..PROMPT_TOP + 30.).contains(&y) {
                if let Some((_, choice, _, _)) = PROMPT_BUTTONS.iter().find(|(_, _, left, width)| (*left..*left + *width).contains(&x)) {
                    self.answer_conflict(*choice);
                } else if x >= PROMPT_ALL_X && x < PROMPT_ALL_X + 130. {
                    if let Some(work) = self.work.as_mut() {
                        work.apply_all = !work.apply_all;
                    }
                    self.revision += 1;
                }
            }
            return;
        }
        if self.naming.is_some() {
            self.commit_rename();
        }
        self.update_controls();self.controls.down(x,y);let _=self.controls.up(x,y);
        self.searching=self.controls.focused().is_some_and(|n|n.id==BAR_SEARCH as u64);
        // a pointer click does not leave a keyboard focus ring on a button (Golden Gate: only Tab moves the ring there)
        if self.controls.focused().is_some_and(|n|n.role==Role::Button) { self.controls.focus(None); }
        if self.info.is_some() && self.info_click(x, y) {
            return;
        }
        if self.preview.is_some() || self.info.is_some() || self.open_with.is_some() {
            self.preview = None;
            self.info = None;
            self.open_with = None;
            self.revision += 1;
            return;
        }
        if x < self.side() {
            self.sidebar_click(x, y);
            return;
        }
        if y < HEADER_H {
            match self.bar_layout().hit(x, y) {
                Some(BAR_BACK) => self.go_back(),
                Some(BAR_FORWARD) => self.go_forward(),
                Some(BAR_ICONS) => self.set_view(true, false),
                Some(BAR_LIST) => self.set_view(false, false),
                Some(BAR_COLUMNS) => self.set_view(false, true),
                Some(BAR_GALLERY) => self.set_gallery(),
                // the pull-down, share, tags and action buttons queue their menu or sheet (barmenu.rs)
                Some(id @ (BAR_SORT | BAR_SHARE | BAR_TAGS | BAR_MORE)) => self.bar_action(id),
                Some(BAR_SEARCH) => {
                    if self.search_rect().is_some_and(|r| bifrost_app::controls::search::hits_clear(r, !self.query.is_empty(), x, y)) {
                        self.query.clear();
                        self.query_changed();
                    }
                    self.searching = true
                }
                _ => {}
            }
            return;
        }
        if let Some(hit) = self.tab_hit(x, y) {
            match hit {
                TabHit::Select(i) => self.select_tab(i),
                TabHit::Close(i) => self.close_tab_at(i),
            }
            return;
        }
        if let Some(scope) = self.scope_hit(x, y) {
            // the scope plates do not take the typing focus from the search field
            self.pick_scope(scope);
            self.searching = true;
            return;
        }
        if y >= self.bottom() {
            if let Some(i) = bifrost_app::controls::pathbar::hit(&self.path_segs, x, y) {
                if let Some((_, _, p)) = self.path_items().into_iter().nth(i) {
                    if p != self.path {
                        self.navigate(p, true);
                    }
                }
            }
            return;
        }
        if self.options_click(x, y) {
            return;
        }
        if self.is_list() && y >= self.body_top() && y < self.rows_top() {
            if let Some((by, _)) = self.list_columns().into_iter().rev().find(|(_, cx)| x >= *cx - 8.) {
                self.sort_by(by);
            }
            return;
        }
        if self.is_list() {
            // the disclosure triangle opens or closes a folder inline
            if let Some(i) = self.item_at(x, y).filter(|i| self.rows[*i].is_dir) {
                let tx = self.side() + 4. + self.depth_of(i) as f32 * 16.;
                if x >= tx - 2. && x < tx + 12. {
                    self.toggle_expand(i);
                    return;
                }
            }
        }
        self.searching = false;
        self.revision += 1; // an empty search field collapses back to its button
        if let Some(folder) = self.ancestor_hit(x, y) {
            self.navigate(folder, true);
            return;
        }
        if let Some(i) = self.item_at(x, y) {
            if self.select_mode {
                self.toggle_selected(i);
                return;
            }
            if self
                .last_click
                .is_some_and(|(j, prev)| j == i && t - prev < 400.)
            {
                self.open(i);
                self.last_click = None
            } else {
                self.select_only(i);
                self.last_click = Some((i, t))
            }
        } else {
            self.selection.clear()
        }
    }
    /// The folder or sidebar place under a point, when a drop there would move the selection.
    pub(crate) fn drop_target_at(&self, x: f32, y: f32) -> Option<PathBuf> {
        let target = if x < self.side() {
            self.sidebar().hit(self.sidebar_bounds(), SIDEBAR_SCROLL, x, y).and_then(|id| self.sidebar_path(id))
        } else {
            self.item_at(x, y)
                .filter(|i| !self.selection.contains(i) && self.rows[*i].is_dir && !self.fx.drag_paths.contains(&self.rows[*i].path))
                .map(|i| self.rows[i].path.clone())
        }?;
        // never into the bin (use Delete), the stand-in listings, or the folder already open
        let blocked = target == self.bin() || target == self.path || target == Path::new(fs::RECENTS) || target.starts_with(TAGGED) || smart::folder_index(&target).is_some();
        (!blocked).then_some(target)
    }
    pub(crate) fn render(&mut self, ui: &Ui, s: &mut Surface) -> Damage {
        let state = format!("{}:{}:{}:{:?}:{}:{}:{:?}:{:?}:{}:{}:{}:{}:{}:{:?}:{:?}:{:?}:{:?}:{}",
            self.revision, self.grid, self.scroll, self.sort, format!("{}:{}:{:?}:{:?}",self.query,self.query.caret(),self.query.selection(),self.controls.focused().map(|n|n.id)), self.searching,
            self.preview, self.error, self.loading, self.w, self.h, ui.s, self.show_hidden,
            self.naming.as_ref().map(|(p,f)|format!("{}:{}:{:?}:{:?}",p.display(),f,f.caret(),f.selection())), self.clip, self.notice,
            self.work.as_ref().map(|w| (&w.title, &w.progress)), self.select_mode);
        let state = format!("{state}:{}:{}", self.status_bar, self.dragging);
        let state = format!("{state}:{:?}", self.scope_bar.scope);
        let state = format!("{state}:{}", self.info.as_ref().map_or(0, |i| i.rows.len() * 10000 + i.mode.unwrap_or(0) as usize));
        let state = format!("{state}:{}", self.open_with.as_ref().map_or(0, |o| o.state()));
        let state = format!("{state}:{:?}:{}", self.fx.goto.as_ref().map(|f| (f.to_string(), f.caret())), self.fx.copy_drag);
        let vo = &self.vo;
        let state = format!("{state}:{}:{}:{:?}:{}:{}:{}:{:?}:{}", vo.gallery, vo.asc, vo.group, vo.icon, vo.text, vo.col_w, vo.show, vo.expanded.len());
        let mut damage = Damage::new(s.w,s.h);
        if state != self.painted_state {
            damage.all();
        } else if self.selection != self.painted_selection && (self.columns || self.vo.gallery) {
            // the column rows are not on the grid the partial repaint assumes
            damage.all();
        } else if self.selection != self.painted_selection {
            let visible = self.visible();
            let cols = ((self.w - (self.side() + 16.))/self.grid_w()).floor().max(1.) as usize;
            // every row whose selection state changed
            let changed = self.selection.iter().chain(self.painted_selection.iter()).filter(|i| self.selection.contains(i) != self.painted_selection.contains(i));
            for &selection in changed {
                if let Some(n) = visible.iter().position(|i| *i == selection) {
                    let first = self.rows_top();
                    let (y,height) = if self.grid {(first + 10. + (n/cols) as f32*self.grid_h() - self.scroll,self.grid_h())}
                        else {(first + n as f32*32. - self.scroll,32.)};
                    let top = ui.pt(y).floor() as i32;
                    let bottom = ui.pt(y+height).ceil() as i32;
                    damage.add(0,top,s.w as i32,bottom-top);
                }
            }
            damage.add(0,ui.pt(self.bottom()).floor() as i32,s.w as i32,ui.pt(self.footer_h()).ceil() as i32);
            // Share, Tags and the action menu enable with the selection
            damage.add(0,0,s.w as i32,ui.pt(self.body_top()).ceil() as i32);
        }
        let mut spans = Vec::new(); damage.rows(&mut spans);
        for (top,bottom) in spans { self.paint(ui,&mut s.rows(top,bottom)); }
        self.painted_state = state;
        self.painted_selection = self.selection.clone();
        damage
    }
    pub(crate) fn paint(&mut self, ui: &Ui, s: &mut Surface) {
        let (w, h) = (self.w, self.h);
        // Reset every dirty pixel opaquely, including fractional-scale edge rows. The content area is the window's
        // white in Light and the dark content grey in Dark (CONTENT_BG), not the pure black of DETAIL_BG.
        s.fill_rect(0,0,s.w,s.h,CONTENT_BG.now());
        // Golden Gate full-size window: the sidebar runs to the top edge under the traffic lights (the shell draws them)
        if !self.sidebar_hidden {
            let pressed = if self.dragging { self.drop_target.as_ref().and_then(|t| self.sidebar_id(t)) } else { None };
            widgets::source_list(s, ui, self.sidebar_bounds(), &self.sidebar(), SIDEBAR_SCROLL, pressed);
            rect(s, ui, self.side() - 1., 0., 1., h, SEPARATOR_C.now());
        }
        // The toolbar shares the content colour and its button groups float over it as glass capsules (Golden Gate's
        // Files): no `toolbar_band` behind them. Icons that have scrolled under the toolbar get a hairline at its edge.
        if self.grid && self.scroll > 0. {
            rect(s, ui, self.side(), self.body_top() - 1., w - self.side(), 1., SEPARATOR_C.now());
        }
        self.paint_tab_strip(ui, s);
        self.paint_scope_bar(ui, s);
        let title = self.title();
        self.title_w = (ui.measure(&title, MAC_TITLE3_PT, Weight::Bold) / ui.pt(1.) + 4.).clamp(40., self.title_room());
        let placed = widgets::toolbar(s, ui, self.bar_bounds(), &self.bar(), None);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let im = ui.text(&ui.fit(&title, MAC_TITLE3_PT, ui.pt(r.w)), MAC_TITLE3_PT, Weight::Bold, LABEL.now());
            s.blit(&im, ui.pt(r.x) as i32, (ui.pt(r.y + r.h / 2.) as i32) - im.h as i32 / 2, 255, 255);
        }
        if let Some(r) = self.search_rect() {
            if self.search_open() {
                widgets::search_field(s, ui, &self.query, r, "Search", self.searching);
            } else {
                widgets::search_button(s, ui, (r.x, r.y), false);
            }
        }
        if let Some(node)=self.controls.focused().filter(|n|n.role==Role::Button){let r=node.bounds;rect(s,ui,r.x,r.y+r.h-2.,r.w,SPACE_XXS,TINT);}
        if self.is_list() {
            self.paint_list_header(ui, s);
        }
        let visible = self.visible();
        let cols = ((w - (self.side() + 16.)) / self.grid_w()).floor().max(1.) as usize;
        let bottom = self.bottom();
        if self.columns {
            self.paint_columns(ui, s, &visible);
        } else if self.vo.gallery {
            self.paint_gallery(ui, s, &visible);
        } else {
            // only the rows or icon lines that can reach the viewport are drawn
            let first = self.rows_top();
            let (top, step, per) = if self.grid { (first + 16., self.grid_h(), cols) } else { (first, 32., 1) };
            let span = row_span(self.scroll, top, step, self.body_top() + 4., bottom, visible.len().div_ceil(per));
            for n in span.start * per..(span.end * per).min(visible.len()) {
                let i = visible[n];
                if self.grid {
                    self.paint_icon_cell(ui, s, n, i, cols, bottom);
                } else {
                    self.paint_list_row(ui, s, n, i, n.checked_sub(1).map(|p| visible[p]), bottom);
                }
            }
        }
        if self.loading || self.error.is_some() || visible.is_empty() {
            let msg = if self.loading {
                "Loading…"
            } else if self.scope_bar_shown() && self.error.is_none() {
                // a query searching a tree: nothing found yet, or nothing at all
                if self.search.is_some() { "Searching…" } else { "No results" }
            } else {
                self.error.as_deref().unwrap_or(if self.in_bin() { "The Trash is empty" } else { "This folder is empty" })
            };
            text(s, ui, msg, 15., LABEL_2.now(), self.side() + 32., self.rows_top() + 16., w - 270.);
        }
        let fh = self.footer_h();
        if fh > 0. {
            self.paint_footer(ui, s, &visible);
        } else {
            self.path_segs.clear();
        }
        self.paint_options(ui, s);
        self.paint_prompt(ui, s);
        self.paint_tag_panel(ui, s);
        self.paint_info(ui, s);
        self.paint_open_with(ui, s);
        self.paint_preview(ui, s);
        self.paint_goto(ui, s);
        self.paint_server(ui, s);
        self.paint_empty_trash(ui, s);
    }
    /// Recents, Tags and Smart Folders list files from several places; nothing can be created or pasted into them.
    pub(crate) fn in_recents(&self) -> bool {
        self.path == Path::new(fs::RECENTS) || self.path.starts_with(TAGGED) || smart::folder_index(&self.path).is_some() || provider::is_path(&self.path)
    }
}
