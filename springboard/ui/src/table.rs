//! `NativeTableView` column model: sort descriptors, clickable headers with a sort indicator, column resize and reorder.
//! Pure model (no drawing): the app draws headers from [`TableColumns::header_frames`] and feeds clicks and drags back.
use crate::Rect;
use std::cmp::Ordering;

/// `NativeSortDescriptor`: a column key and a direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortDescriptor {
    pub key: String,
    pub ascending: bool,
}

impl SortDescriptor {
    pub fn new(key: &str, ascending: bool) -> Self {
        SortDescriptor { key: key.into(), ascending }
    }
    pub fn reversed(&self) -> Self {
        SortDescriptor { key: self.key.clone(), ascending: !self.ascending }
    }
}

/// Sorts `rows` by `descriptors` (first is primary); `cmp(a, b, key)` compares two rows on a key ascending.
/// Stable, like `sortedArray(using:)`.
pub fn sort_rows<T>(rows: &mut [T], descriptors: &[SortDescriptor], cmp: impl Fn(&T, &T, &str) -> Ordering) {
    rows.sort_by(|a, b| {
        for d in descriptors {
            let o = cmp(a, b, &d.key);
            let o = if d.ascending { o } else { o.reverse() };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
}

/// `NativeTableColumn`.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: String,
    pub title: String,
    pub width: f32,
    pub min_width: f32,
    pub max_width: f32,
    /// `sortDescriptorPrototype`: clicking the header sorts by this key; `None` = not sortable.
    pub sort_key: Option<String>,
    pub resizable: bool,
}

impl Column {
    pub fn new(id: &str, title: &str, width: f32) -> Self {
        Column { id: id.into(), title: title.into(), width, min_width: 40.0, max_width: 10_000.0, sort_key: Some(id.into()), resizable: true }
    }
    pub fn unsortable(mut self) -> Self {
        self.sort_key = None;
        self
    }
}

/// Width of the grab zone either side of a column divider, in points.
pub const RESIZE_GRAB: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeaderHit {
    /// Title area of column `i` (click sorts, drag reorders).
    Title(usize),
    /// Right divider of column `i` (drag resizes).
    Divider(usize),
}

#[derive(Clone, Debug, Default)]
pub struct TableColumns {
    pub columns: Vec<Column>,
    /// Active sort, primary first. Clicking a header makes its key primary (`tableView(_:sortDescriptorsDidChange:)`).
    pub sort: Vec<SortDescriptor>,
}

impl TableColumns {
    pub fn new(columns: Vec<Column>) -> Self {
        TableColumns { columns, sort: Vec::new() }
    }
    /// Column frames across `header`, left to right.
    pub fn header_frames(&self, header: Rect) -> Vec<Rect> {
        let mut x = header.x;
        self.columns
            .iter()
            .map(|c| {
                let r = Rect { x, y: header.y, w: c.width, h: header.h };
                x += c.width;
                r
            })
            .collect()
    }
    pub fn total_width(&self) -> f32 {
        self.columns.iter().map(|c| c.width).sum()
    }
    pub fn hit(&self, header: Rect, px: f32, py: f32) -> Option<HeaderHit> {
        if py < header.y || py >= header.y + header.h {
            return None;
        }
        let frames = self.header_frames(header);
        for (i, f) in frames.iter().enumerate() {
            if self.columns[i].resizable && (px - (f.x + f.w)).abs() <= RESIZE_GRAB {
                return Some(HeaderHit::Divider(i));
            }
        }
        frames.iter().position(|f| px >= f.x && px < f.x + f.w).map(HeaderHit::Title)
    }
    /// Header click: toggles direction when `i` is already primary, else sorts ascending by it. Returns the new primary
    /// descriptor, or `None` for a non-sortable column.
    pub fn click_header(&mut self, i: usize) -> Option<SortDescriptor> {
        let key = self.columns.get(i)?.sort_key.clone()?;
        let next = match self.sort.first() {
            Some(d) if d.key == key => d.reversed(),
            _ => SortDescriptor { key: key.clone(), ascending: true },
        };
        self.sort.retain(|d| d.key != key);
        self.sort.insert(0, next.clone());
        Some(next)
    }
    /// The sort indicator of column `i`: `Some(ascending)` when it is the primary sort key.
    pub fn indicator(&self, i: usize) -> Option<bool> {
        let key = self.columns.get(i)?.sort_key.as_ref()?;
        self.sort.first().filter(|d| &d.key == key).map(|d| d.ascending)
    }
    /// Resizes column `i` to `width` (clamped to its min and max); returns the applied width.
    pub fn resize(&mut self, i: usize, width: f32) -> Option<f32> {
        let c = self.columns.get_mut(i)?;
        if !c.resizable {
            return Some(c.width);
        }
        c.width = width.clamp(c.min_width, c.max_width.max(c.min_width));
        Some(c.width)
    }
    /// `moveColumn(_:toColumn:)`.
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.columns.len() || to >= self.columns.len() {
            return false;
        }
        let c = self.columns.remove(from);
        self.columns.insert(to, c);
        true
    }
    /// Destination index when a title drag is released at `px`.
    pub fn drop_index(&self, header: Rect, px: f32) -> usize {
        let frames = self.header_frames(header);
        frames.iter().position(|f| px < f.x + f.w).unwrap_or(self.columns.len().saturating_sub(1))
    }
    /// `autosaveName`-style persistence: `id:width` pairs in order, comma separated.
    pub fn save(&self) -> String {
        self.columns.iter().map(|c| format!("{}:{}", c.id, c.width)).collect::<Vec<_>>().join(",")
    }
    pub fn restore(&mut self, s: &str) {
        let mut out = Vec::new();
        for part in s.split(',') {
            if let Some((id, w)) = part.split_once(':') {
                if let (Some(pos), Ok(w)) = (self.columns.iter().position(|c| c.id == id), w.parse::<f32>()) {
                    let mut c = self.columns.remove(pos);
                    c.width = w.clamp(c.min_width, c.max_width.max(c.min_width));
                    out.push(c);
                }
            }
        }
        out.append(&mut self.columns);
        self.columns = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: Rect = Rect { x: 0.0, y: 0.0, w: 400.0, h: 22.0 };
    fn t() -> TableColumns {
        TableColumns::new(vec![Column::new("name", "Name", 200.0), Column::new("size", "Size", 80.0), Column::new("kind", "Kind", 100.0).unsortable()])
    }
    #[test]
    fn header_click_sorts_then_toggles() {
        let mut t = t();
        assert_eq!(t.click_header(0), Some(SortDescriptor::new("name", true)));
        assert_eq!(t.click_header(0), Some(SortDescriptor::new("name", false)));
        assert_eq!(t.indicator(0), Some(false));
        t.click_header(1);
        assert_eq!(t.sort.len(), 2);
        assert_eq!(t.indicator(0), None);
        assert_eq!(t.click_header(2), None);
    }
    #[test]
    fn sort_rows_uses_secondary_keys_and_direction() {
        let mut rows = vec![(2, "b"), (1, "b"), (3, "a")];
        let d = [SortDescriptor::new("s", false), SortDescriptor::new("n", true)];
        sort_rows(&mut rows, &d, |a, b, k| if k == "s" { a.1.cmp(b.1) } else { a.0.cmp(&b.0) });
        assert_eq!(rows, vec![(1, "b"), (2, "b"), (3, "a")]);
    }
    #[test]
    fn hit_finds_divider_before_title() {
        let t = t();
        assert_eq!(t.hit(H, 100.0, 5.0), Some(HeaderHit::Title(0)));
        assert_eq!(t.hit(H, 199.0, 5.0), Some(HeaderHit::Divider(0)));
        assert_eq!(t.hit(H, 100.0, 40.0), None);
    }
    #[test]
    fn resize_clamps_and_reorder_moves() {
        let mut t = t();
        assert_eq!(t.resize(1, 5.0), Some(40.0));
        assert!(t.reorder(2, 0));
        assert_eq!(t.columns[0].id, "kind");
        assert!(!t.reorder(9, 0));
        assert_eq!(t.drop_index(H, 10.0), 0);
    }
    #[test]
    fn save_restore_round_trips_order_and_widths() {
        let mut a = t();
        a.reorder(2, 0);
        a.resize(1, 150.0);
        let s = a.save();
        let mut b = t();
        b.restore(&s);
        assert_eq!(b.save(), s);
    }
}
