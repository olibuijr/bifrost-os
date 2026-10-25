//! Navigation and presentation models: a push/pop stack (`NativeNavigationController`), a value path that drives it
//! (`NavigationPath`), a toolbar with validation and overflow (`NativeToolbar`), and an alert (`NativeAlert`,
//! `NativeAlertController`).
use crate::Rect;

/// A push/pop stack of screens. The back button is titled after the screen beneath the top one.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationStack<T> {
    screens: Vec<(String, T)>,
}

impl<T> NavigationStack<T> {
    pub fn new(root_title: impl Into<String>, root: T) -> Self {
        NavigationStack { screens: vec![(root_title.into(), root)] }
    }
    pub fn depth(&self) -> usize {
        self.screens.len()
    }
    pub fn can_go_back(&self) -> bool {
        self.screens.len() > 1
    }
    pub fn push(&mut self, title: impl Into<String>, screen: T) {
        self.screens.push((title.into(), screen));
    }
    /// Pops the top screen and returns it; the root can never be popped.
    pub fn pop(&mut self) -> Option<(String, T)> {
        if self.can_go_back() { self.screens.pop() } else { None }
    }
    /// Pops down to the root; returns how many screens went.
    pub fn pop_to_root(&mut self) -> usize {
        let n = self.screens.len() - 1;
        self.screens.truncate(1);
        n
    }
    /// Pops until the screen at `index` (0 = root) is on top; returns how many went.
    pub fn pop_to(&mut self, index: usize) -> usize {
        let keep = (index + 1).clamp(1, self.screens.len());
        let n = self.screens.len() - keep;
        self.screens.truncate(keep);
        n
    }
    pub fn top(&self) -> &T {
        &self.screens[self.screens.len() - 1].1
    }
    pub fn top_mut(&mut self) -> &mut T {
        let i = self.screens.len() - 1;
        &mut self.screens[i].1
    }
    pub fn title(&self) -> &str {
        &self.screens[self.screens.len() - 1].0
    }
    /// The back button label: the previous screen's title, or `None` at the root.
    pub fn back_title(&self) -> Option<&str> {
        let n = self.screens.len();
        (n > 1).then(|| self.screens[n - 2].0.as_str())
    }
    /// Titles from the root to the top (a breadcrumb or the long-press back menu).
    pub fn path(&self) -> Vec<&str> {
        self.screens.iter().map(|(t, _)| t.as_str()).collect()
    }
    /// Shows the destinations of `path` above the root: pops to the root, then pushes one screen per destination, with
    /// the title and screen `build` returns for it. Returns how many screens were pushed.
    pub fn apply_path<D>(&mut self, path: &NavigationPath<D>, mut build: impl FnMut(&D) -> (String, T)) -> usize {
        self.pop_to_root();
        for destination in path.iter() {
            let (title, screen) = build(destination);
            self.push(title, screen);
        }
        path.count()
    }
}

/// The destinations of a navigation stack as plain values (`NavigationPath`): an app pushes a value per screen, keeps
/// the path, and rebuilds the stack from it with `NavigationStack::apply_path`. A path of one kind of value has no
/// type erasure; the app chooses the enum that holds its destinations.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationPath<D> {
    items: Vec<D>,
}

impl<D> Default for NavigationPath<D> {
    fn default() -> Self {
        NavigationPath { items: Vec::new() }
    }
}

impl<D> NavigationPath<D> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn count(&self) -> usize {
        self.items.len()
    }
    /// Appends a destination to the end of the path.
    pub fn append(&mut self, destination: D) {
        self.items.push(destination);
    }
    /// Removes up to `n` destinations from the end and returns how many went.
    pub fn remove_last(&mut self, n: usize) -> usize {
        let n = n.min(self.items.len());
        self.items.truncate(self.items.len() - n);
        n
    }
    /// The last destination, the one on top of the stack.
    pub fn last(&self) -> Option<&D> {
        self.items.last()
    }
    /// The destinations from the root screen upwards.
    pub fn iter(&self) -> std::slice::Iter<'_, D> {
        self.items.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarKind {
    Button,
    Separator,
    /// Expands to take the spare width (`NativeToolbarFlexibleSpaceItem`).
    FlexibleSpace,
    /// A slot of a fixed `width` the app draws itself (a title, a search field, a pop-up button), as a custom
    /// `NativeToolbarItem` view. It is laid out and reported in `visible` like a button but drawn by the app.
    Custom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarItem {
    pub id: u32,
    pub label: String,
    pub kind: ToolbarKind,
    pub enabled: bool,
    /// An the system symbol set name (`NativeImage(systemSymbolName:)`); the button shows the glyph and `label` becomes its
    /// accessibility label and tooltip.
    pub symbol: Option<String>,
    /// Buttons with the same non-zero group share one capsule (`NativeToolbarItemGroup`), with no gap between them.
    pub group: u32,
    /// The chosen segment of a segmented group (the current view mode).
    pub selected: bool,
    /// The width of a `Custom` slot, in points.
    pub width: f32,
    /// Laid out in the middle of the bar, not in the leading-to-trailing flow (`NativeToolbar.centeredItemIdentifiers`).
    pub centered: bool,
    /// A pull-down button (`NativeMenuToolbarItem`): drawn with a trailing chevron; the app opens its menu under
    /// `ToolbarLayout::menu_anchor` when it is clicked.
    pub menu: bool,
}

impl ToolbarItem {
    fn of(id: u32, label: &str, kind: ToolbarKind) -> Self {
        ToolbarItem { id, label: label.into(), kind, enabled: true, symbol: None, group: 0, selected: false, width: 0.0, centered: false, menu: false }
    }
    pub fn button(id: u32, label: &str) -> Self {
        Self::of(id, label, ToolbarKind::Button)
    }
    /// A glyph button: `symbol` is drawn, `label` is what assistive technology reads.
    pub fn icon(id: u32, label: &str, symbol: &str) -> Self {
        ToolbarItem { symbol: Some(symbol.into()), ..Self::of(id, label, ToolbarKind::Button) }
    }
    /// A slot of `width` points the app draws itself.
    pub fn custom(id: u32, label: &str, width: f32) -> Self {
        ToolbarItem { width, ..Self::of(id, label, ToolbarKind::Custom) }
    }
    /// A pull-down menu button (`NativeMenuToolbarItem`): a titled or glyph (`symbol`) button with a chevron.
    pub fn menu(id: u32, label: &str, symbol: Option<&str>) -> Self {
        ToolbarItem { symbol: symbol.map(Into::into), menu: true, ..Self::of(id, label, ToolbarKind::Button) }
    }
    /// Lays the item out in the middle of the bar (`NativeToolbar.centeredItemIdentifiers`). Consecutive centred items
    /// form one run; a centred group stays one capsule.
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
    pub fn separator() -> Self {
        Self::of(0, "", ToolbarKind::Separator)
    }
    pub fn flexible_space() -> Self {
        Self::of(0, "", ToolbarKind::FlexibleSpace)
    }
    /// Puts the button in capsule `group` (non-zero).
    pub fn in_group(mut self, group: u32) -> Self {
        self.group = group;
        self
    }
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
}

/// Where each item went: on the bar, or into the overflow menu because the window is too narrow.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarLayout {
    /// Buttons and custom slots on the bar.
    pub visible: Vec<(u32, Rect)>,
    pub overflow: Vec<u32>,
    /// The `»` button, present only when something overflowed.
    pub overflow_button: Option<Rect>,
    /// One capsule per group on the bar: the group id and the rectangle around its visible buttons.
    pub groups: Vec<(u32, Rect)>,
}

impl ToolbarLayout {
    /// The visible item under `(x, y)`.
    pub fn hit(&self, x: f32, y: f32) -> Option<u32> {
        self.visible.iter().find(|(_, r)| r.contains(x, y)).map(|(id, _)| *id)
    }
    pub fn rect_of(&self, id: u32) -> Option<Rect> {
        self.visible.iter().find(|(i, _)| *i == id).map(|(_, r)| *r)
    }
    /// Where a pull-down item's menu opens (`NativeMenuToolbarItem`): its leading edge, `gap` points below the button.
    pub fn menu_anchor(&self, id: u32, gap: f32) -> Option<(f32, f32)> {
        self.rect_of(id).map(|r| (r.x, r.bottom() + gap))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolbar {
    pub items: Vec<ToolbarItem>,
}

impl Toolbar {
    pub fn new(items: Vec<ToolbarItem>) -> Self {
        Toolbar { items }
    }
    /// Asks `valid` whether each button should be enabled (`validateToolbarItem:`); true when anything changed.
    pub fn validate(&mut self, valid: impl Fn(u32) -> bool) -> bool {
        let mut changed = false;
        for it in self.items.iter_mut().filter(|i| i.kind == ToolbarKind::Button) {
            let on = valid(it.id);
            changed |= it.enabled != on;
            it.enabled = on;
        }
        changed
    }
    /// Lays buttons of `item_w` left to right, `gap` apart, in `bounds`. Flexible spaces share what is left. When the
    /// bar is too narrow, trailing buttons move to the overflow menu behind a `»` button of `item_w`.
    pub fn layout(&self, bounds: Rect, item_w: f32, gap: f32) -> ToolbarLayout {
        self.layout_with(bounds, |_| item_w, item_w, gap)
    }
    /// As `layout`, with each button's width from `button_w` (glyph buttons are narrower than titled ones) and
    /// custom slots at their own width. Buttons of one group touch; a group overflows as a whole.
    ///
    /// Centred items (`ToolbarItem::centered`) leave the flow: the rest is laid out around a hole at the first flexible
    /// space, and the centred run sits in the middle of `bounds`, pushed aside only as far as the neighbours need.
    pub fn layout_with(&self, bounds: Rect, button_w: impl Fn(&ToolbarItem) -> f32, overflow_w: f32, gap: f32) -> ToolbarLayout {
        if !self.items.iter().any(|i| i.centered) {
            return self.layout_flow(bounds, &button_w, overflow_w, gap);
        }
        let centre: Vec<ToolbarItem> = self.items.iter().filter(|i| i.centered).cloned().collect();
        let side = Toolbar { items: self.items.iter().filter(|i| !i.centered).cloned().collect() };
        let run = Toolbar { items: centre };
        let probe = run.layout_flow(Rect { x: 0.0, w: f32::MAX / 4.0, ..bounds }, &button_w, overflow_w, gap);
        let run_w = probe.visible.iter().map(|(_, r)| r.right()).fold(0.0, f32::max);
        let hole = run_w + 2.0 * gap;
        let mut out = side.layout_flow(Rect { w: (bounds.w - hole).max(0.0), ..bounds }, &button_w, overflow_w, gap);
        let flex = side.items.iter().position(|i| i.kind == ToolbarKind::FlexibleSpace);
        let trailing: Vec<u32> = flex.map_or(Vec::new(), |f| side.items[f..].iter().map(|i| i.id).collect());
        let shift = |r: &mut Rect| r.x += hole;
        if flex.is_some() {
            for (id, r) in out.visible.iter_mut() {
                if trailing.contains(id) {
                    shift(r);
                }
            }
            for (g, r) in out.groups.iter_mut() {
                if side.items.iter().skip(flex.unwrap()).any(|i| i.group == *g) {
                    shift(r);
                }
            }
            if let Some(r) = out.overflow_button.as_mut() {
                shift(r);
            }
        }
        let lead_end = out.visible.iter().filter(|(id, _)| !trailing.contains(id)).map(|(_, r)| r.right()).fold(bounds.x, f32::max);
        let trail_start = out.visible.iter().filter(|(id, _)| trailing.contains(id)).map(|(_, r)| r.x).chain(out.overflow_button.map(|r| r.x)).fold(bounds.right(), f32::min);
        let lo = if lead_end > bounds.x { lead_end + gap } else { bounds.x };
        let hi = if trail_start < bounds.right() { trail_start - gap - run_w } else { bounds.right() - run_w };
        let x0 = (bounds.x + (bounds.w - run_w) / 2.0).min(hi).max(lo);
        out.visible.extend(probe.visible.into_iter().map(|(id, r)| (id, Rect { x: r.x + x0, ..r })));
        out.groups.extend(probe.groups.into_iter().map(|(g, r)| (g, Rect { x: r.x + x0, ..r })));
        out
    }

    fn layout_flow(&self, bounds: Rect, button_w: &impl Fn(&ToolbarItem) -> f32, overflow_w: f32, gap: f32) -> ToolbarLayout {
        let width_of = |it: &ToolbarItem| match it.kind {
            ToolbarKind::Button => button_w(it),
            ToolbarKind::Custom => it.width,
            ToolbarKind::Separator => gap,
            ToolbarKind::FlexibleSpace => 0.0,
        };
        let joined = |k: usize| k > 0 && self.items[k].group != 0 && self.items[k].group == self.items[k - 1].group;
        let gap_before = |k: usize| if k == 0 || joined(k) { 0.0 } else { gap };
        let need = |n: usize| -> f32 { (0..n).map(|k| width_of(&self.items[k]) + gap_before(k)).sum() };
        // never split a group: a cut inside one moves back to its first member
        let whole = |mut n: usize| {
            while n > 0 && n < self.items.len() && joined(n) {
                n -= 1;
            }
            n
        };
        let mut shown = self.items.len();
        while shown > 0 && need(shown) > bounds.w {
            shown = whole(shown - 1);
        }
        let overflowing = shown < self.items.len();
        if overflowing {
            while shown > 0 && need(shown) + gap + overflow_w > bounds.w {
                shown = whole(shown - 1);
            }
        }
        let spaces = self.items[..shown].iter().filter(|i| i.kind == ToolbarKind::FlexibleSpace).count();
        let reserved = if overflowing { gap + overflow_w } else { 0.0 };
        let spare = (bounds.w - need(shown) - reserved).max(0.0);
        let each = if spaces > 0 { spare / spaces as f32 } else { 0.0 };
        let mut x = bounds.x;
        let mut visible = Vec::new();
        let mut groups: Vec<(u32, Rect)> = Vec::new();
        for (k, it) in self.items[..shown].iter().enumerate() {
            x += gap_before(k);
            let w = if it.kind == ToolbarKind::FlexibleSpace { each } else { width_of(it) };
            let r = Rect { x, y: bounds.y, w, h: bounds.h };
            if matches!(it.kind, ToolbarKind::Button | ToolbarKind::Custom) {
                visible.push((it.id, r));
            }
            if it.group != 0 && it.kind == ToolbarKind::Button {
                match groups.last_mut() {
                    Some((g, gr)) if joined(k) && *g == it.group => gr.w = r.right() - gr.x,
                    _ => groups.push((it.group, r)),
                }
            }
            x += w;
        }
        let overflow: Vec<u32> = self.items[shown..].iter().filter(|i| i.kind == ToolbarKind::Button).map(|i| i.id).collect();
        // never left of the bar: a bar narrower than the button (squeezed by the traffic-light inset) keeps it at its start
        let overflow_button = overflowing.then(|| Rect { x: (bounds.right() - overflow_w).max(bounds.x), y: bounds.y, w: overflow_w, h: bounds.h });
        ToolbarLayout { visible, overflow, overflow_button, groups }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertStyle {
    Default,
    Cancel,
    Destructive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AlertAction {
    pub id: u32,
    pub label: String,
    pub style: AlertStyle,
}

/// An alert: a title, a message and buttons. The first non-cancel, non-destructive action is the default (Return);
/// a cancel action answers Escape.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub title: String,
    pub message: String,
    actions: Vec<AlertAction>,
}

impl Alert {
    pub fn new(title: &str, message: &str) -> Self {
        Alert { title: title.into(), message: message.into(), actions: Vec::new() }
    }
    /// Adds a button. Only one cancel action is kept: a second replaces the first (the native toolkit allows only one).
    pub fn action(mut self, id: u32, label: &str, style: AlertStyle) -> Self {
        if style == AlertStyle::Cancel {
            self.actions.retain(|a| a.style != AlertStyle::Cancel);
        }
        self.actions.push(AlertAction { id, label: label.into(), style });
        self
    }
    pub fn actions(&self) -> &[AlertAction] {
        &self.actions
    }
    /// The action Return activates.
    pub fn default_action(&self) -> Option<&AlertAction> {
        self.actions.iter().find(|a| a.style == AlertStyle::Default).or_else(|| self.actions.iter().find(|a| a.style != AlertStyle::Cancel))
    }
    /// The action Escape activates.
    pub fn cancel_action(&self) -> Option<&AlertAction> {
        self.actions.iter().find(|a| a.style == AlertStyle::Cancel)
    }
    /// Indices into `actions` from left to right on Golden Gate: destructive and other actions first, then Cancel, with the
    /// default button rightmost.
    pub fn button_order(&self) -> Vec<usize> {
        let default = self.default_action().map(|d| self.actions.iter().position(|a| a.id == d.id).unwrap());
        let mut order: Vec<usize> = (0..self.actions.len()).filter(|&i| Some(i) != default && self.actions[i].style != AlertStyle::Cancel).collect();
        order.extend((0..self.actions.len()).filter(|&i| self.actions[i].style == AlertStyle::Cancel));
        order.extend(default);
        order
    }

    /// The alert as a document-modal sheet (`NativeAlert.beginSheetModal(for:)`): it hangs from its window's title bar
    /// (`sheet::Sheet::layout`), its title and message wrap instead of being cut, the default button is filled with the
    /// accent and answers Return, the cancel button answers Escape. Buttons keep [`Alert::button_order`] (the sheet lists
    /// them right to left). A destructive action stays destructive, so it is never the accent-filled Return button.
    pub fn sheet(&self) -> crate::sheet::Sheet {
        use crate::sheet::{ButtonRole, Sheet, SheetButton};
        let default = self.default_action().map(|a| a.id);
        let buttons = self
            .button_order()
            .into_iter()
            .rev()
            .map(|i| {
                let a = &self.actions[i];
                let role = match a.style {
                    AlertStyle::Cancel => ButtonRole::Cancel,
                    AlertStyle::Destructive => ButtonRole::Destructive,
                    AlertStyle::Default if Some(a.id) == default => ButtonRole::Default,
                    AlertStyle::Default => ButtonRole::Normal,
                };
                SheetButton::new(a.id, &a.label, role)
            })
            .collect();
        Sheet::alert(&self.title, &self.message, buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_back_button_is_titled_after_the_previous_screen() {
        let mut n = NavigationStack::new("Lists", 0);
        assert_eq!((n.back_title(), n.can_go_back()), (None, false));
        n.push("Groceries", 1);
        n.push("Milk", 2);
        assert_eq!((n.title(), n.back_title(), n.depth()), ("Milk", Some("Groceries"), 3));
        assert_eq!(n.path(), vec!["Lists", "Groceries", "Milk"]);
        *n.top_mut() = 5;
        assert_eq!(*n.top(), 5);
        assert_eq!(n.pop(), Some(("Milk".to_string(), 5)));
        assert_eq!(n.back_title(), Some("Lists"));
    }

    #[test]
    fn the_root_cannot_be_popped_and_pop_to_clamps() {
        let mut n = NavigationStack::new("A", ());
        assert_eq!(n.pop(), None);
        n.push("B", ());
        n.push("C", ());
        n.push("D", ());
        assert_eq!(n.pop_to(1), 2);
        assert_eq!(n.title(), "B");
        assert_eq!(n.pop_to(99), 0, "an index past the top changes nothing");
        assert_eq!(n.pop_to_root(), 1);
        assert_eq!((n.depth(), n.title()), (1, "A"));
        assert_eq!(n.pop_to_root(), 0);
    }

    fn bar() -> Toolbar {
        Toolbar { items: vec![ToolbarItem::button(1, "Back"), ToolbarItem::button(2, "Forward"), ToolbarItem::flexible_space(), ToolbarItem::button(3, "Share"), ToolbarItem::button(4, "Add")] }
    }

    #[test]
    fn validation_enables_items_and_reports_changes() {
        let mut t = bar();
        assert!(t.validate(|id| id != 2));
        assert!(!t.items[1].enabled && t.items[0].enabled);
        assert!(!t.validate(|id| id != 2), "no change the second time");
        assert!(t.items[2].enabled, "spaces are untouched");
    }

    #[test]
    fn a_wide_bar_shows_everything_and_flexible_space_pushes_the_trailing_items_right() {
        let l = bar().layout(Rect { x: 0.0, y: 0.0, w: 500.0, h: 30.0 }, 40.0, 8.0);
        assert!(l.overflow.is_empty() && l.overflow_button.is_none());
        assert_eq!(l.visible.len(), 4);
        assert_eq!(l.visible[0].1.x, 0.0);
        let last = l.visible[3].1;
        assert!((last.right() - 500.0).abs() < 0.01, "the last button sits at the trailing edge: {last:?}");
    }

    #[test]
    fn a_narrow_bar_moves_trailing_items_to_the_overflow_menu() {
        let b = Rect { x: 0.0, y: 0.0, w: 130.0, h: 30.0 };
        let l = bar().layout(b, 40.0, 8.0);
        assert!(!l.overflow.is_empty());
        let ob = l.overflow_button.expect("overflow button");
        assert!(l.visible.iter().all(|(_, r)| r.right() <= ob.x + 0.01), "visible buttons stay clear of the » button");
        assert!(ob.right() <= b.right() + 0.01);
        assert_eq!(l.visible.len() + l.overflow.len(), 4, "every button is either shown or in the menu");
        assert_eq!(l.overflow.last(), Some(&4));
    }

    #[test]
    fn the_overflow_button_never_leaves_a_bar_narrower_than_itself() {
        let b = Rect { x: 95.0, y: 0.0, w: 20.0, h: 30.0 };
        let l = bar().layout(b, 40.0, 8.0);
        let ob = l.overflow_button.expect("every button overflows");
        assert!(ob.x >= b.x, "the » button starts at the bar's left edge, not left of it: {ob:?}");
        assert!(l.visible.is_empty());
    }

    fn file_manager() -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::icon(1, "Back", "chevron.left").in_group(1),
            ToolbarItem::icon(2, "Forward", "chevron.right").in_group(1),
            ToolbarItem::custom(9, "Title", 120.0),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(3, "Icons", "square.grid.2x2").in_group(2).selected(true),
            ToolbarItem::icon(4, "List", "list.bullet").in_group(2),
            ToolbarItem::icon(5, "Columns", "rectangle.split.3x1").in_group(2),
            ToolbarItem::custom(10, "Search", 180.0),
        ])
    }

    #[test]
    fn buttons_of_a_group_touch_and_share_one_capsule() {
        let l = file_manager().layout_with(Rect { x: 0.0, y: 0.0, w: 800.0, h: 32.0 }, |_| 32.0, 32.0, 10.0);
        assert!(l.overflow.is_empty());
        let (back, fwd) = (l.rect_of(1).unwrap(), l.rect_of(2).unwrap());
        assert_eq!(back.right(), fwd.x, "no gap inside a group");
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0], (1, Rect { x: 0.0, y: 0.0, w: 64.0, h: 32.0 }));
        assert_eq!(l.groups[1].1.w, 96.0, "three segments");
        assert_eq!(l.rect_of(9).unwrap().x, 74.0, "the title slot follows the group after one gap");
        assert!((l.rect_of(10).unwrap().right() - 800.0).abs() < 0.01, "search sits at the trailing edge");
        assert_eq!(l.hit(80.0, 10.0), Some(9));
    }

    #[test]
    fn a_group_overflows_as_a_whole() {
        let l = file_manager().layout_with(Rect { x: 0.0, y: 0.0, w: 300.0, h: 32.0 }, |_| 32.0, 32.0, 10.0);
        let view: Vec<u32> = [3, 4, 5].into_iter().filter(|id| l.overflow.contains(id)).collect();
        assert!(view.is_empty() || view.len() == 3, "the view group is shown or hidden together: {:?}", l.overflow);
        assert!(l.overflow_button.is_some());
    }

    #[test]
    fn centred_items_sit_in_the_middle_between_the_leading_and_trailing_flow() {
        let t = Toolbar::new(vec![
            ToolbarItem::icon(1, "Sidebar", "sidebar.left"),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(2, "Day", "calendar").in_group(5).centered(),
            ToolbarItem::icon(3, "Week", "calendar").in_group(5).centered(),
            ToolbarItem::icon(4, "Add", "plus"),
        ]);
        let b = Rect { x: 0.0, y: 0.0, w: 600.0, h: 32.0 };
        let l = t.layout_with(b, |_| 32.0, 32.0, 10.0);
        let (day, week) = (l.rect_of(2).unwrap(), l.rect_of(3).unwrap());
        assert_eq!(day.right(), week.x, "the centred group stays joined");
        assert!(((day.x + week.right()) / 2.0 - 300.0).abs() < 0.01, "centred in the bar: {day:?} {week:?}");
        assert_eq!(l.rect_of(1).unwrap().x, 0.0);
        assert!((l.rect_of(4).unwrap().right() - 600.0).abs() < 0.01, "trailing item keeps the trailing edge");
        assert!(l.groups.iter().any(|(g, r)| *g == 5 && r.w == 64.0));
    }

    #[test]
    fn a_centred_run_is_pushed_aside_by_wide_leading_items() {
        let t = Toolbar::new(vec![ToolbarItem::custom(1, "Title", 280.0), ToolbarItem::flexible_space(), ToolbarItem::custom(2, "Mode", 100.0).centered()]);
        let l = t.layout_with(Rect { x: 0.0, y: 0.0, w: 500.0, h: 32.0 }, |_| 32.0, 32.0, 10.0);
        assert_eq!(l.rect_of(2).unwrap().x, 290.0, "one gap after the title, not overlapping it");
        let plain = Toolbar::new(vec![ToolbarItem::button(1, "A"), ToolbarItem::button(2, "B")]);
        assert_eq!(plain.layout_with(Rect { x: 0.0, y: 0.0, w: 200.0, h: 30.0 }, |_| 40.0, 40.0, 8.0), plain.layout(Rect { x: 0.0, y: 0.0, w: 200.0, h: 30.0 }, 40.0, 8.0));
    }

    #[test]
    fn a_menu_item_is_a_button_with_an_anchor_under_it() {
        let it = ToolbarItem::menu(7, "Sort", Some("arrow.up.arrow.down"));
        assert!(it.menu && it.kind == ToolbarKind::Button && it.symbol.is_some());
        let l = Toolbar::new(vec![it]).layout(Rect { x: 10.0, y: 5.0, w: 200.0, h: 30.0 }, 50.0, 8.0);
        assert_eq!(l.menu_anchor(7, 4.0), Some((10.0, 39.0)));
        assert_eq!(l.menu_anchor(8, 4.0), None);
    }

    /// E2E9 6: a document's close confirmation is a sheet whose Save is the accent-filled Return button, rightmost, with
    /// Cancel for Escape and Don't Save at the left.
    #[test]
    fn an_alert_becomes_a_sheet_with_the_default_rightmost() {
        use crate::keys::Key;
        use crate::sheet::ButtonRole;
        let a = Alert::new("Do you want to save the changes made to the document \u{201c}Untitled\u{201d}?", "Your changes will be lost if you don\u{2019}t save them.")
            .action(1, "Save", AlertStyle::Default)
            .action(2, "Cancel", AlertStyle::Cancel)
            .action(3, "Don\u{2019}t Save", AlertStyle::Destructive);
        let sheet = a.sheet();
        assert!(sheet.alert, "sized from its wrapped text");
        assert_eq!(sheet.title, a.title, "the whole title, never cut");
        let roles: Vec<(u32, ButtonRole)> = sheet.buttons.iter().map(|b| (b.id, b.role)).collect();
        assert_eq!(roles, vec![(1, ButtonRole::Default), (2, ButtonRole::Cancel), (3, ButtonRole::Destructive)], "first is rightmost");
        assert_eq!(sheet.key(Key::Return).0, Some(1));
        assert_eq!(sheet.key(Key::Escape).0, Some(2));
        // it hangs from the title bar of its window
        let parent = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        assert_eq!(sheet.layout(parent, 52.0, 1.0).panel.y, 52.0);
    }

    #[test]
    fn an_alert_has_a_default_for_return_and_a_cancel_for_escape() {
        let a = Alert::new("Delete?", "This cannot be undone.")
            .action(1, "Delete", AlertStyle::Destructive)
            .action(2, "Cancel", AlertStyle::Cancel)
            .action(3, "Keep", AlertStyle::Default);
        assert_eq!(a.default_action().map(|x| x.id), Some(3));
        assert_eq!(a.cancel_action().map(|x| x.id), Some(2));
        assert_eq!(a.button_order(), vec![0, 1, 2], "destructive, cancel, then the default rightmost");
    }

    #[test]
    fn a_second_cancel_replaces_the_first_and_without_a_default_the_first_other_action_is_used() {
        let a = Alert::new("t", "m").action(1, "No", AlertStyle::Cancel).action(2, "Nope", AlertStyle::Cancel).action(3, "Remove", AlertStyle::Destructive);
        assert_eq!(a.actions().len(), 2);
        assert_eq!(a.cancel_action().map(|x| x.id), Some(2));
        assert_eq!(a.default_action().map(|x| x.id), Some(3));
        assert_eq!(Alert::new("t", "m").default_action(), None);
        assert!(Alert::new("t", "m").button_order().is_empty());
    }

    #[test]
    fn a_navigation_path_holds_destinations_in_order_and_removes_from_the_end() {
        let mut p = NavigationPath::new();
        assert!(p.is_empty() && p.last().is_none());
        p.append("Groceries");
        p.append("Milk");
        assert_eq!((p.count(), p.last()), (2, Some(&"Milk")));
        assert_eq!(p.iter().copied().collect::<Vec<_>>(), vec!["Groceries", "Milk"]);
        assert_eq!(p.remove_last(1), 1);
        assert_eq!(p.last(), Some(&"Groceries"));
        assert_eq!(p.remove_last(9), 1, "removing more than the path holds stops at empty");
        assert!(p.is_empty());
    }

    #[test]
    fn apply_path_rebuilds_the_screens_above_the_root_from_values() {
        let mut stack = NavigationStack::new("Lists", 0usize);
        let mut path = NavigationPath::new();
        path.append(7usize);
        path.append(9usize);
        let pushed = stack.apply_path(&path, |&id| (format!("List {id}"), id));
        assert_eq!(pushed, 2);
        assert_eq!(stack.path(), vec!["Lists", "List 7", "List 9"]);
        assert_eq!((stack.back_title(), *stack.top()), (Some("List 7"), 9));
        path.remove_last(1);
        stack.apply_path(&path, |&id| (format!("List {id}"), id));
        assert_eq!(stack.path(), vec!["Lists", "List 7"], "a shorter path pops the screens it no longer holds");
    }

    #[test]
    fn apply_path_with_an_empty_path_returns_to_the_root_and_keeps_its_screen() {
        let mut stack = NavigationStack::new("Root", "root");
        stack.push("Detail", "detail");
        let empty: NavigationPath<&str> = NavigationPath::new();
        assert_eq!(stack.apply_path(&empty, |_| (String::new(), "")), 0);
        assert_eq!((stack.depth(), stack.title(), *stack.top()), (1, "Root", "root"));
    }

    #[test]
    fn navigation_paths_compare_by_their_destinations() {
        let mut a = NavigationPath::new();
        a.append(1u8);
        let mut b = NavigationPath::new();
        b.append(1u8);
        assert_eq!(a, b, "the same destinations make equal paths");
        b.append(2);
        assert_ne!(a, b);
        assert_eq!(NavigationPath::<u8>::default().count(), 0);
    }
}
