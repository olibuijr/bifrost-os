//! A lazy image graph with any number of inputs: the general form of `lazy_image`. A `GraphImage` is a node
//! that knows how to make its pixels: from a source image, from a generator, by mapping one input through an
//! operation, or by combining two inputs (a blend, a transition, a displacement map). Building a graph runs
//! nothing. A `GraphContext` renders a node: it evaluates the inputs the node needs, each node at most once
//! per render even when several nodes share it, and it keeps the results of nodes in a size-limited cache, so
//! rendering again, or rendering a node built on an earlier one, reuses the earlier work.
//!
//! The operations are plain closures over `gfx::Image`, so any CPU filter in the render crate or the SDK can be
//! a node. Nodes are immutable and cheap to clone (they share their inputs), so one node can feed many graphs.
//! Limits: the cache keeps whole images and evicts the oldest first once it holds more than its byte limit
//! (always keeping the newest); a graph deeper than `MAX_DEPTH` is refused at render time; evaluation is single
//! threaded and CPU only; a node has no extent until it is rendered.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::gfx::Image;

/// The deepest graph a context renders. Evaluation recurses once per level.
pub const MAX_DEPTH: usize = 256;

type MapFn = Arc<dyn Fn(&Image) -> Option<Image> + Send + Sync>;
type CombineFn = Arc<dyn Fn(&Image, &Image) -> Option<Image> + Send + Sync>;
type GenerateFn = Arc<dyn Fn() -> Option<Image> + Send + Sync>;

enum Op {
    Source(Arc<Image>),
    Generate(GenerateFn),
    Map(Arc<Node>, MapFn),
    Combine(Arc<Node>, Arc<Node>, CombineFn),
}

struct Node {
    id: u64,
    name: String,
    depth: usize,
    op: Op,
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// An image that has not been computed yet: a node of the graph.
#[derive(Clone)]
pub struct GraphImage {
    node: Arc<Node>,
}

impl fmt::Debug for GraphImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GraphImage({:?}, depth {})", self.node.name, self.node.depth)
    }
}

impl GraphImage {
    fn make(name: &str, depth: usize, op: Op) -> GraphImage {
        GraphImage { node: Arc::new(Node { id: next_id(), name: name.to_string(), depth, op }) }
    }

    /// A node that shows `image` as it is.
    pub fn from_image(image: Image) -> GraphImage {
        GraphImage::make("source", 0, Op::Source(Arc::new(image)))
    }

    /// A node whose pixels come from `f` when it is rendered (a generator: a gradient, a code, noise).
    /// `f` returning `None` makes the render fail.
    pub fn generate(name: &str, f: impl Fn() -> Option<Image> + Send + Sync + 'static) -> GraphImage {
        GraphImage::make(name, 0, Op::Generate(Arc::new(f)))
    }

    /// A node that applies `f` to this one. Nothing runs until a context renders it.
    pub fn map(&self, name: &str, f: impl Fn(&Image) -> Option<Image> + Send + Sync + 'static) -> GraphImage {
        GraphImage::make(name, self.node.depth + 1, Op::Map(self.node.clone(), Arc::new(f)))
    }

    /// A node that combines this image (the first argument of `f`) with `other` (the second).
    pub fn combine(&self, other: &GraphImage, name: &str, f: impl Fn(&Image, &Image) -> Option<Image> + Send + Sync + 'static) -> GraphImage {
        let depth = self.node.depth.max(other.node.depth) + 1;
        GraphImage::make(name, depth, Op::Combine(self.node.clone(), other.node.clone(), Arc::new(f)))
    }

    /// The name this node was given.
    pub fn name(&self) -> &str {
        &self.node.name
    }

    /// How many operations lie between this node and its deepest source.
    pub fn depth(&self) -> usize {
        self.node.depth
    }

    /// The names of the distinct nodes this one needs, inputs first, in the order they would be evaluated. A node
    /// shared by several others is listed once.
    pub fn describe(&self) -> Vec<String> {
        fn walk(node: &Arc<Node>, seen: &mut HashSet<u64>, out: &mut Vec<String>) {
            if !seen.insert(node.id) {
                return;
            }
            match &node.op {
                Op::Map(a, _) => walk(a, seen, out),
                Op::Combine(a, b, _) => {
                    walk(a, seen, out);
                    walk(b, seen, out);
                }
                Op::Source(_) | Op::Generate(_) => {}
            }
            out.push(node.name.clone());
        }
        let mut out = Vec::new();
        if self.node.depth <= MAX_DEPTH {
            walk(&self.node, &mut HashSet::new(), &mut out);
        }
        out
    }
}

/// Renders graph images and remembers what it computed.
#[derive(Debug)]
pub struct GraphContext {
    cache: HashMap<u64, Arc<Image>>,
    order: VecDeque<u64>,
    bytes: usize,
    max_bytes: usize,
    evaluations: usize,
}

impl Default for GraphContext {
    fn default() -> Self {
        GraphContext::new()
    }
}

fn image_bytes(img: &Image) -> usize {
    img.px.len() * 4
}

impl GraphContext {
    /// A context whose cache holds up to 64 MiB of images.
    pub fn new() -> GraphContext {
        GraphContext::with_cache_limit(64 << 20)
    }

    /// A context with its own cache limit in bytes. A limit of 0 keeps only the newest image.
    pub fn with_cache_limit(max_bytes: usize) -> GraphContext {
        GraphContext { cache: HashMap::new(), order: VecDeque::new(), bytes: 0, max_bytes, evaluations: 0 }
    }

    /// How many nodes this context has computed (cache hits do not count).
    pub fn evaluations(&self) -> usize {
        self.evaluations
    }

    /// How many node results the cache holds.
    pub fn cached_images(&self) -> usize {
        self.cache.len()
    }

    /// Drops every cached result.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.order.clear();
        self.bytes = 0;
    }

    /// Computes the pixels of `image`. `None` when an operation or generator on the way returned `None`, or the
    /// graph is deeper than `MAX_DEPTH`.
    pub fn render(&mut self, image: &GraphImage) -> Option<Image> {
        if image.node.depth > MAX_DEPTH {
            return None;
        }
        self.eval(&image.node).map(|a| (*a).clone())
    }

    fn eval(&mut self, node: &Arc<Node>) -> Option<Arc<Image>> {
        if let Some(hit) = self.cache.get(&node.id) {
            return Some(hit.clone());
        }
        let out = match &node.op {
            Op::Source(img) => return Some(img.clone()),
            Op::Generate(f) => f()?,
            Op::Map(input, f) => {
                let a = self.eval(input)?;
                f(&a)?
            }
            Op::Combine(left, right, f) => {
                let a = self.eval(left)?;
                let b = self.eval(right)?;
                f(&a, &b)?
            }
        };
        self.evaluations += 1;
        let out = Arc::new(out);
        self.bytes += image_bytes(&out);
        self.cache.insert(node.id, out.clone());
        self.order.push_back(node.id);
        while self.bytes > self.max_bytes && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front() {
                if let Some(gone) = self.cache.remove(&old) {
                    self.bytes -= image_bytes(&gone);
                }
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    /// An operation that adds 1 to the red byte of every pixel and counts its runs.
    fn add_red(counter: &Arc<AtomicUsize>) -> impl Fn(&Image) -> Option<Image> + Send + Sync + 'static {
        let counter = counter.clone();
        move |img: &Image| {
            counter.fetch_add(1, Ordering::SeqCst);
            Some(Image { w: img.w, h: img.h, px: img.px.iter().map(|&p| p + 1).collect() })
        }
    }

    #[test]
    fn building_a_graph_runs_nothing_and_rendering_applies_the_operations_in_order() {
        let runs = Arc::new(AtomicUsize::new(0));
        let node = GraphImage::from_image(solid(2, 1, 0xff00_0000)).map("a", add_red(&runs)).map("b", add_red(&runs));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(node.depth(), 2);
        let out = GraphContext::new().render(&node).unwrap();
        assert_eq!(out.px, vec![0xff00_0002; 2]);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_shared_node_is_computed_once_per_render() {
        let runs = Arc::new(AtomicUsize::new(0));
        let base = GraphImage::from_image(solid(1, 1, 0xff00_0000)).map("base", add_red(&runs));
        let left = base.map("left", |i| Some(i.clone()));
        let right = base.map("right", |i| Some(i.clone()));
        let both = left.combine(&right, "sum", |a, b| Some(Image { w: a.w, h: a.h, px: vec![a.px[0] + (b.px[0] & 0xff)] }));
        let mut ctx = GraphContext::new();
        assert_eq!(ctx.render(&both).unwrap().px, vec![0xff00_0002]);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(both.describe(), ["source", "base", "left", "right", "sum"]);
    }

    #[test]
    fn rendering_again_reuses_the_cache_and_clearing_it_recomputes() {
        let runs = Arc::new(AtomicUsize::new(0));
        let node = GraphImage::from_image(solid(1, 1, 0xff00_0000)).map("a", add_red(&runs));
        let mut ctx = GraphContext::new();
        ctx.render(&node).unwrap();
        ctx.render(&node).unwrap();
        assert_eq!((runs.load(Ordering::SeqCst), ctx.evaluations(), ctx.cached_images()), (1, 1, 1));
        // A node built on the first one reuses its result.
        let more = node.map("b", add_red(&runs));
        assert_eq!(ctx.render(&more).unwrap().px, vec![0xff00_0002]);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        ctx.clear_cache();
        assert_eq!(ctx.cached_images(), 0);
        ctx.render(&node).unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_cache_evicts_the_oldest_result_past_its_limit_but_keeps_the_newest() {
        let node = GraphImage::from_image(solid(4, 4, 0xff00_0000)).map("a", |i| Some(i.clone())).map("b", |i| Some(i.clone()));
        let mut ctx = GraphContext::with_cache_limit(0);
        assert!(ctx.render(&node).is_some());
        assert_eq!(ctx.cached_images(), 1);
        let mut roomy = GraphContext::with_cache_limit(1 << 20);
        roomy.render(&node).unwrap();
        assert_eq!(roomy.cached_images(), 2);
    }

    #[test]
    fn a_failing_operation_fails_the_render_and_generators_feed_graphs() {
        let bad = GraphImage::from_image(solid(1, 1, 0)).map("bad", |_| None);
        assert!(GraphContext::new().render(&bad).is_none());
        let generated = GraphImage::generate("flat", || Some(solid(3, 2, 0xff11_2233)));
        let out = GraphContext::new().render(&generated.map("same", |i| Some(i.clone()))).unwrap();
        assert_eq!((out.w, out.h, out.px[0]), (3, 2, 0xff11_2233));
        assert_eq!(generated.name(), "flat");
        assert!(GraphContext::new().render(&GraphImage::generate("none", || None)).is_none());
    }

    #[test]
    fn a_graph_deeper_than_the_limit_is_refused() {
        let mut node = GraphImage::from_image(solid(1, 1, 0));
        for _ in 0..=MAX_DEPTH {
            node = node.map("n", |i| Some(i.clone()));
        }
        assert_eq!(node.depth(), MAX_DEPTH + 1);
        assert!(GraphContext::new().render(&node).is_none());
        assert!(node.describe().is_empty());
    }
}
