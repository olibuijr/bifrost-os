//! Spatial partitioning: a point quadtree that holds values at 2D positions and answers rectangle queries.
//! A node holds up to `capacity` points. When it holds more, it splits into four equal quadrants and passes
//! its points down. Rectangles are half-open: the minimum edge is inside and the maximum edge is outside, so
//! a point on a shared edge belongs to exactly one quadrant. Points outside the root bounds are rejected.

/// A point in 2D space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An axis-aligned rectangle from its minimum corner to its maximum corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub min: Point,
    pub max: Point,
}

impl Area {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min: Point { x: min_x, y: min_y },
            max: Point { x: max_x, y: max_y },
        }
    }

    /// Whether `p` lies inside, with the maximum edges excluded.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// Whether the two half-open rectangles share any interior. Rectangles that only touch along an edge do not.
    pub fn intersects(&self, other: &Area) -> bool {
        self.min.x < other.max.x && other.min.x < self.max.x && self.min.y < other.max.y && other.min.y < self.max.y
    }

    /// Quadrant `i`: 0 is the lower-left, 1 lower-right, 2 upper-left, 3 upper-right.
    fn quadrant(&self, i: usize) -> Area {
        let mx = (self.min.x + self.max.x) / 2.0;
        let my = (self.min.y + self.max.y) / 2.0;
        match i {
            0 => Area::new(self.min.x, self.min.y, mx, my),
            1 => Area::new(mx, self.min.y, self.max.x, my),
            2 => Area::new(self.min.x, my, mx, self.max.y),
            _ => Area::new(mx, my, self.max.x, self.max.y),
        }
    }
}

/// A point quadtree over values of type `T`. `T` is copied in and out, so use small ids.
pub struct QuadTree<T: Copy + PartialEq> {
    bounds: Area,
    capacity: usize,
    max_depth: u32,
    root: Node<T>,
}

struct Node<T> {
    bounds: Area,
    depth: u32,
    /// Points held here. Empty once the node has children.
    items: Vec<(Point, T)>,
    children: Option<Box<[Node<T>; 4]>>,
}

impl<T: Copy + PartialEq> QuadTree<T> {
    /// An empty tree over `bounds`. A node splits once it holds more than `capacity` points (at least 1),
    /// and never below `max_depth` levels under the root.
    pub fn new(bounds: Area, capacity: usize, max_depth: u32) -> Self {
        let capacity = capacity.max(1);
        Self {
            bounds,
            capacity,
            max_depth,
            root: Node::new(bounds, 0),
        }
    }

    pub fn bounds(&self) -> Area {
        self.bounds
    }

    /// Adds `value` at `p`. Returns false, and stores nothing, when `p` is outside the bounds.
    pub fn insert(&mut self, p: Point, value: T) -> bool {
        self.root.insert(p, value, self.capacity, self.max_depth)
    }

    /// Removes the first entry equal to `value` at `p`. Returns false when there is none.
    pub fn remove(&mut self, p: Point, value: T) -> bool {
        self.root.remove(p, value)
    }

    /// Every value whose point lies inside `area`, in tree order (not insertion order).
    pub fn query(&self, area: Area) -> Vec<T> {
        let mut out = Vec::new();
        self.root.query(&area, &mut out);
        out
    }

    /// Number of stored points.
    pub fn len(&self) -> usize {
        self.root.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes every point and keeps the bounds.
    pub fn clear(&mut self) {
        self.root = Node::new(self.bounds, 0);
    }
}

impl<T: Copy + PartialEq> Node<T> {
    fn new(bounds: Area, depth: u32) -> Self {
        Self {
            bounds,
            depth,
            items: Vec::new(),
            children: None,
        }
    }

    fn insert(&mut self, p: Point, value: T, capacity: usize, max_depth: u32) -> bool {
        if !self.bounds.contains(p) {
            return false;
        }
        if let Some(kids) = self.children.as_mut() {
            return kids.iter_mut().any(|k| k.insert(p, value, capacity, max_depth));
        }
        self.items.push((p, value));
        if self.items.len() > capacity && self.depth < max_depth {
            self.split(capacity, max_depth);
        }
        true
    }

    fn split(&mut self, capacity: usize, max_depth: u32) {
        let bounds = self.bounds;
        let depth = self.depth + 1;
        let kids: [Node<T>; 4] = [0, 1, 2, 3].map(|i| Node::new(bounds.quadrant(i), depth));
        self.children = Some(Box::new(kids));
        let items = std::mem::take(&mut self.items);
        for (p, value) in items {
            // The four quadrants tile this node, so one of them always accepts the point.
            let _ = self.insert(p, value, capacity, max_depth);
        }
    }

    fn remove(&mut self, p: Point, value: T) -> bool {
        if !self.bounds.contains(p) {
            return false;
        }
        if let Some(kids) = self.children.as_mut() {
            return kids.iter_mut().any(|k| k.remove(p, value));
        }
        match self.items.iter().position(|(q, v)| *q == p && *v == value) {
            Some(i) => {
                self.items.remove(i);
                true
            }
            None => false,
        }
    }

    fn query(&self, area: &Area, out: &mut Vec<T>) {
        if !self.bounds.intersects(area) {
            return;
        }
        for (p, value) in &self.items {
            if area.contains(*p) {
                out.push(*value);
            }
        }
        if let Some(kids) = &self.children {
            for k in kids.iter() {
                k.query(area, out);
            }
        }
    }

    fn len(&self) -> usize {
        match &self.children {
            Some(kids) => kids.iter().map(|k| k.len()).sum::<usize>(),
            None => self.items.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    /// Root 0..8 with capacity 2. The six points below split the root into quadrants:
    /// lower-left {a, b}, lower-right {f}, upper-left {e}, upper-right {c, d}.
    fn sample_tree() -> QuadTree<u32> {
        let mut t = QuadTree::new(Area::new(0.0, 0.0, 8.0, 8.0), 2, 4);
        assert!(t.insert(pt(1.0, 1.0), 1)); // a
        assert!(t.insert(pt(3.0, 1.0), 2)); // b
        assert!(t.insert(pt(5.0, 5.0), 3)); // c, third point: the root splits
        assert!(t.insert(pt(7.0, 7.0), 4)); // d
        assert!(t.insert(pt(1.0, 6.0), 5)); // e
        assert!(t.insert(pt(4.0, 1.0), 6)); // f, on the x = 4 edge: lower-right, since the minimum edge is inside
        t
    }

    fn sorted(mut v: Vec<u32>) -> Vec<u32> {
        v.sort();
        v
    }

    #[test]
    fn insert_counts_points_and_rejects_the_maximum_edge() {
        let mut t = sample_tree();
        assert_eq!(t.len(), 6);
        assert!(!t.insert(pt(8.0, 8.0), 9)); // outside: maximum edges are excluded
        assert!(!t.insert(pt(-0.5, 2.0), 9));
        assert_eq!(t.len(), 6);
        assert!(!t.is_empty());
    }

    #[test]
    fn quadrant_query_returns_only_the_points_inside() {
        let t = sample_tree();
        assert_eq!(sorted(t.query(Area::new(0.0, 0.0, 4.0, 4.0))), vec![1, 2]);
        assert_eq!(sorted(t.query(Area::new(4.0, 0.0, 8.0, 4.0))), vec![6]);
        assert_eq!(sorted(t.query(Area::new(0.0, 4.0, 4.0, 8.0))), vec![5]);
        assert_eq!(sorted(t.query(Area::new(4.0, 4.0, 8.0, 8.0))), vec![3, 4]);
    }

    #[test]
    fn half_open_edges_decide_membership() {
        let t = sample_tree();
        // x in [0, 4) excludes f at x = 4.0.
        assert_eq!(sorted(t.query(Area::new(0.0, 0.0, 4.0, 8.0))), vec![1, 2, 5]);
        // x in [4, 8) includes f.
        assert_eq!(sorted(t.query(Area::new(4.0, 0.0, 8.0, 8.0))), vec![3, 4, 6]);
        // The whole root returns all six.
        assert_eq!(t.query(Area::new(0.0, 0.0, 8.0, 8.0)).len(), 6);
    }

    #[test]
    fn remove_takes_out_one_matching_entry() {
        let mut t = sample_tree();
        assert!(t.remove(pt(3.0, 1.0), 2)); // b
        assert!(!t.remove(pt(3.0, 1.0), 2)); // already gone
        assert!(!t.remove(pt(3.0, 1.0), 99)); // wrong value at that point
        assert_eq!(t.len(), 5);
        assert_eq!(sorted(t.query(Area::new(0.0, 0.0, 4.0, 4.0))), vec![1]);
    }

    #[test]
    fn max_depth_stops_splitting_and_keeps_every_point() {
        // Depth 0 is the root only, so the root never splits and all points stay in one node.
        let mut t = QuadTree::new(Area::new(0.0, 0.0, 8.0, 8.0), 1, 0);
        for i in 0..3u32 {
            assert!(t.insert(pt(2.0, 2.0), i));
        }
        assert_eq!(t.len(), 3);
        assert_eq!(sorted(t.query(Area::new(1.0, 1.0, 3.0, 3.0))), vec![0, 1, 2]);
        t.clear();
        assert!(t.is_empty());
    }

    #[test]
    fn touching_rectangles_do_not_intersect() {
        let left = Area::new(0.0, 0.0, 4.0, 4.0);
        let right = Area::new(4.0, 0.0, 8.0, 4.0);
        let overlap = Area::new(3.0, 3.0, 5.0, 5.0);
        assert!(!left.intersects(&right));
        assert!(left.intersects(&overlap));
        assert!(right.intersects(&overlap));
    }
}
