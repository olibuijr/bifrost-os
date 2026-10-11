//! Browser engine benchmark: parse, style and layout time per corpus page
//! (tests/pages/*.html), median of RUNS, printed as JSON.
//! Run with `cargo run --release --example browser_bench`.
use springboard::shell::browser::budget::MemStats;
use springboard::shell::browser::{css, html, layout};
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

const RUNS: usize = 15;
const WIDTH: f32 = 1200.0;

/// Fixed-pitch text metrics: isolates engine cost from font rasterisation.
struct Mono;
impl layout::Metrics for Mono {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        text.chars().count() as f32 * size * if bold { 0.58 } else { 0.52 }
    }
}

fn inline_css(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(a) = rest.find("<style>") {
        let b = rest[a..].find("</style>").map_or(rest.len(), |i| a + i);
        out.push_str(&rest[a + 7..b]);
        rest = &rest[b.min(rest.len())..];
        rest = rest.get(8..).unwrap_or("");
    }
    out
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pages");
    let mut files: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "html")).collect();
    files.sort();
    let ua = css::parse_stylesheet(css::USER_AGENT);
    let sizes = HashMap::new();
    let mut rows = vec![];
    for f in files {
        let src = fs::read_to_string(&f).unwrap();
        let sheet = inline_css(&src);
        let (mut p, mut s, mut l) = (vec![], vec![], vec![]);
        let mut items = 0;
        let mut height = 0.0;
        for _ in 0..RUNS {
            let t = Instant::now();
            let dom = html::parse(&src);
            p.push(t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            let mut rules = ua.clone();
            rules.extend(css::parse_stylesheet(&sheet));
            let tree = css::style_tree(&dom, &rules);
            s.push(t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            let page = layout::layout(&tree, &Mono, WIDTH, &sizes);
            l.push(t.elapsed().as_secs_f64() * 1e3);
            items = page.items.len();
            height = page.height;
        }
        let mem = MemStats::read();
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        rows.push(format!(
            "  {{\"page\":\"{name}\",\"bytes\":{},\"parse_ms\":{:.2},\"style_ms\":{:.2},\"layout_ms\":{:.2},\"items\":{items},\"height\":{height:.0},\"rss_kb\":{},\"peak_rss_kb\":{}}}",
            src.len(),
            median(&mut p),
            median(&mut s),
            median(&mut l),
            mem.rss / 1024,
            mem.hwm / 1024
        ));
    }
    println!("[\n{}\n]", rows.join(",\n"));
    let end = MemStats::read();
    eprintln!("total: rss {} kB, peak rss (VmHWM) {} kB", end.rss / 1024, end.hwm / 1024);
}
