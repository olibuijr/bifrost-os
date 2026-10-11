//! Quadtree: objects placed at points of a two-dimensional region, kept in a tree of quadrants so a region query only
//! visits the quadrants it touches. A leaf holds up to `capacity` objects; the next object that does not fit splits it
//! into four equal quadrants and the objects move down into them. Splitting stops at `max_depth`, so many objects at
//! one spot cannot make the tree grow without limit: past that depth a leaf simply keeps them all.
//!
//! The region is an axis-aligned `Quad` with both edges included. A point on a shared edge belongs to the first
//! quadrant that contains it, in the order lower-left, lower-right, upper-left, upper-right. Queries use the same
//! inclusive test. Removal is not provided here; a caller that moves objects rebuilds the tree.

/// An axis-aligned rectangle with both edges included. `min` must not be above `max` on either axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Quad {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Quad { min_x, min_y, max_x, max_y }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// True when the two rectangles share at least one point, edges included.
    pub fn intersects(&self, other: &Quad) -> bool {
        self.min_x <= other.max_x && other.min_x <= self.max_x && self.min_y <= other.max_y && other.min_y <= self.max_y
    }

    /// The four equal quadrants, in the order lower-left, lower-right, upper-left, upper-right.
    fn split(&self) -> [Quad; 4] {
        let cx = (self.min_x + self.max_x) / 2.0;
        let cy = (self.min_y + self.max_y) / 2.0;
        [
            Quad::new(self.min_x, self.min_y, cx, cy),
            Quad::new(cx, self.min_y, self.max_x, cy),
            Quad::new(self.min_x, cy, cx, self.max_y),
            Quad::new(cx, cy, self.max_x, self.max_y),
        ]
    }
}

struct Node<T> {
    bounds: Quad,
    depth: u32,
    items: Vec<(f64, f64, T)>,
    children: Option<Box<[Node<T>; 4]>>,
}

impl<T> Node<T> {
    fn new(bounds: Quad, depth: u32) -> Self {
        Node { bounds, depth, items: Vec::new(), children: None }
    }

    fn insert(&mut self, x: f64, y: f64, item: T, capacity: usize, max_depth: u32) {
        if let Some(children) = self.children.as_mut() {
            for child in children.iter_mut() {
                if child.bounds.contains(x, y) {
                    child.insert(x, y, item, capacity, max_depth);
                    return;
                }
            }
            // Unreachable for a point inside this node: the four quadrants cover the node. Keep the item here.
            self.items.push((x, y, item));
            return;
        }

        if self.items.len() < capacity || self.depth >= max_depth {
            self.items.push((x, y, item));
            return;
        }

        // The leaf is full: split it and move its items, then place the new item the same way.
        let quads = self.bounds.split();
        let depth = self.depth + 1;
        let mut children: Box<[Node<T>; 4]> = Box::new([
            Node::new(quads[0], depth),
            Node::new(quads[1], depth),
            Node::new(quads[2], depth),
            Node::new(quads[3], depth),
        ]);
        let moved: Vec<(f64, f64, T)> = self.items.drain(..).collect();
        for (ix, iy, it) in moved {
            let child = children.iter_mut().find(|c| c.bounds.contains(ix, iy));
            match child {
                Some(c) => c.items.push((ix, iy, it)),
                None => self.items.push((ix, iy, it)),
            }
        }
        self.children = Some(children);
        self.insert(x, y, item, capacity, max_depth);
    }

    fn count(&self) -> usize {
        let own = self.items.len();
        match &self.children {
            Some(children) => own + children.iter().map(|c| c.count()).sum::<usize>(),
            None => own,
        }
    }

    fn query<'a>(&'a self, region: &Quad, out: &mut Vec<&'a T>) {
        if !self.bounds.intersects(region) {
            return;
        }
        for (x, y, item) in &self.items {
            if region.contains(*x, *y) {
                out.push(item);
            }
        }
        if let Some(children) = &self.children {
            for child in children.iter() {
                child.query(region, out);
            }
        }
    }
}

/// A quadtree of objects of type `T`, each stored at a point.
pub struct Quadtree<T> {
    root: Node<T>,
    capacity: usize,
    max_depth: u32,
    len: usize,
}

impl<T> Quadtree<T> {
    /// An empty tree over `bounds`. A leaf splits when it would hold more than `capacity` objects (at least 1), and
    /// splitting stops at `max_depth` levels below the root.
    pub fn new(bounds: Quad, capacity: usize, max_depth: u32) -> Self {
        Quadtree { root: Node::new(bounds, 0), capacity: capacity.max(1), max_depth, len: 0 }
    }

    pub fn bounds(&self) -> Quad {
        self.root.bounds
    }

    /// Adds an object at a point. Returns false, and stores nothing, when the point is outside the tree's bounds.
    pub fn insert(&mut self, x: f64, y: f64, item: T) -> bool {
        if !self.root.bounds.contains(x, y) {
            return false;
        }
        self.root.insert(x, y, item, self.capacity, self.max_depth);
        self.len += 1;
        true
    }

    /// Number of objects in the tree.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every object whose point lies inside `region`, edges included. The order is the tree's own, not insertion order.
    pub fn query(&self, region: &Quad) -> Vec<&T> {
        let mut out = Vec::new();
        self.root.query(region, &mut out);
        out
    }

    /// Number of quadrant levels that exist below the root, 0 while the root is still a leaf.
    pub fn depth(&self) -> u32 {
        fn walk<T>(node: &Node<T>) -> u32 {
            match &node.children {
                Some(children) => 1 + children.iter().map(walk).max().unwrap_or(0),
                None => 0,
            }
        }
        walk(&self.root)
    }

    /// Recounts the stored objects by walking the tree. It always equals `len`; tests use it to check splits keep
    /// every object.
    pub fn stored(&self) -> usize {
        self.root.count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Quad {
        Quad::new(0.0, 0.0, 100.0, 100.0)
    }

    #[test]
    fn insert_refuses_points_outside_the_bounds() {
        let mut t = Quadtree::new(square(), 4, 8);
        assert!(t.insert(0.0, 100.0, "edge"), "edges are inside");
        assert!(!t.insert(100.5, 50.0, "outside"));
        assert!(!t.insert(-1.0, 0.0, "outside"));
        assert_eq!(t.len(), 1);
        assert!(!t.is_empty());
    }

    #[test]
    fn splitting_keeps_every_object_and_adds_a_level() {
        let mut t = Quadtree::new(square(), 2, 8);
        let corners = [(10.0, 10.0), (90.0, 10.0), (10.0, 90.0), (90.0, 90.0), (60.0, 60.0)];
        for (i, &(x, y)) in corners.iter().enumerate() {
            assert!(t.insert(x, y, i));
        }
        assert_eq!(t.len(), 5);
        assert_eq!(t.stored(), 5, "no object lost in a split");
        // The third insert splits the root once; each quadrant then holds at most two, so one level is enough.
        assert_eq!(t.depth(), 1);
    }

    #[test]
    fn query_returns_exactly_the_objects_inside_the_region() {
        let mut t = Quadtree::new(square(), 2, 8);
        let points = [(10.0, 10.0, 'a'), (60.0, 10.0, 'b'), (10.0, 60.0, 'c'), (60.0, 60.0, 'd'), (50.0, 50.0, 'e')];
        for &(x, y, id) in &points {
            t.insert(x, y, id);
        }
        let mut found: Vec<char> = t.query(&Quad::new(0.0, 0.0, 50.0, 50.0)).into_iter().copied().collect();
        found.sort();
        assert_eq!(found, vec!['a', 'e'], "50,50 is on the edge of the region and counts");

        let all: Vec<char> = t.query(&square()).into_iter().copied().collect();
        assert_eq!(all.len(), 5);

        assert!(t.query(&Quad::new(70.0, 0.0, 90.0, 5.0)).is_empty());
    }

    #[test]
    fn a_region_that_misses_a_quadrant_does_not_return_its_objects() {
        let mut t = Quadtree::new(square(), 1, 8);
        t.insert(5.0, 5.0, 1);
        t.insert(95.0, 95.0, 2);
        t.insert(5.0, 95.0, 3);
        t.insert(95.0, 5.0, 4);
        let hits: Vec<i32> = t.query(&Quad::new(90.0, 90.0, 100.0, 100.0)).into_iter().copied().collect();
        assert_eq!(hits, vec![2]);
    }

    #[test]
    fn splitting_stops_at_max_depth_for_coincident_points() {
        let mut t = Quadtree::new(square(), 1, 3);
        for i in 0..10 {
            assert!(t.insert(42.0, 42.0, i), "a point at one spot is still accepted");
        }
        assert_eq!(t.len(), 10);
        assert_eq!(t.stored(), 10);
        assert_eq!(t.depth(), 3, "the tree stops splitting at the depth limit");
        assert_eq!(t.query(&Quad::new(42.0, 42.0, 42.0, 42.0)).len(), 10);
    }

    #[test]
    fn zero_capacity_is_treated_as_one() {
        let mut t = Quadtree::new(square(), 0, 4);
        t.insert(1.0, 1.0, 'x');
        t.insert(99.0, 99.0, 'y');
        assert_eq!(t.stored(), 2);
        assert_eq!(t.depth(), 1);
    }
}
