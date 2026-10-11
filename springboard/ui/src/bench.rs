//! Benchmarks for the layout and animation models. Run through `tools/bench.py` (they are `#[ignore]`d so a normal
//! `cargo test` stays fast). Each prints `BENCH <name> <ms> ms`: the time of one call of the measured workload.
use crate::anim::{Animation, Curve};
use crate::geometry::EdgeInsets;
use crate::nav::{Toolbar, ToolbarItem};
use crate::sidebar::{SidebarItem, SourceList};
use crate::split::{place_popover, visible_rows, SplitView};
use crate::stack::{adaptive_columns, grid, stack, Axis, StackItem};
use crate::Rect;
use std::hint::black_box;
use std::time::Instant;

/// Median of `runs` timings of `f`, in milliseconds per call of the closure.
fn bench(name: &str, runs: usize, mut f: impl FnMut()) {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("BENCH {name} {:.4} ms", t[t.len() / 2]);
}

const B: Rect = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };

#[test]
#[ignore]
fn bench_ui_stack_200_items() {
    let items: Vec<StackItem> = (0..200).map(|i| if i % 3 == 0 { StackItem::flexible(10.0, 1.0) } else { StackItem::fixed(24.0) }).collect();
    bench("ui_stack_200_items", 101, || {
        black_box(stack(B, Axis::Vertical, 4.0, EdgeInsets::all(8.0), black_box(&items)));
    });
}

#[test]
#[ignore]
fn bench_ui_grid_10k_cells() {
    bench("ui_grid_10k_cells", 51, || {
        let cols = adaptive_columns(B.w, 120.0, 8.0);
        black_box(grid(B, cols, 8.0, 8.0, 90.0, black_box(10_000)));
    });
}

#[test]
#[ignore]
fn bench_ui_visible_rows_scroll() {
    // One frame of scrolling a million-row table: the cost of deciding which rows exist, 1000 times.
    bench("ui_visible_rows_x1000", 101, || {
        let mut n = 0;
        for i in 0..1000 {
            n += visible_rows(black_box(i as f32 * 37.0), 1080.0, 28.0, 1_000_000, 4).len();
        }
        black_box(n);
    });
}

#[test]
#[ignore]
fn bench_ui_split_and_popover() {
    let mut s = SplitView::new(1920.0, 260.0, 180.0, 300.0);
    bench("ui_split_drag_x1000", 101, || {
        for i in 0..1000 {
            s.drag_to(black_box(100.0 + i as f32));
            black_box(s.rects(B, 1.0));
        }
    });
    bench("ui_popover_x1000", 101, || {
        for i in 0..1000 {
            black_box(place_popover(Rect { x: i as f32, y: 10.0, w: 30.0, h: 20.0 }, (240.0, 160.0), B, crate::geometry::Edge::Top, 6.0));
        }
    });
}

#[test]
#[ignore]
fn bench_ui_toolbar_layout() {
    let mut items = vec![ToolbarItem::button(1, "Back"), ToolbarItem::flexible_space()];
    items.extend((2..40).map(|i| ToolbarItem::button(i, "Item")));
    let bar = Toolbar { items };
    bench("ui_toolbar_layout_40_items_x100", 101, || {
        for w in 0..100 {
            black_box(bar.layout(Rect { x: 0.0, y: 0.0, w: 200.0 + w as f32 * 10.0, h: 38.0 }, 32.0, 6.0));
        }
    });
}

#[test]
#[ignore]
fn bench_ui_animation_curves() {
    let a = Animation::new(0.0, 100.0, 0.0, 1.0, Curve::EaseInOut);
    let s = Animation::spring(0.0, 100.0, 0.0, 0.7, 0.4);
    bench("ui_bezier_x10k", 101, || {
        let mut v = 0.0;
        for i in 0..10_000 {
            v += a.value(black_box(i as f64 / 10_000.0));
        }
        black_box(v);
    });
    bench("ui_spring_x10k", 101, || {
        let mut v = 0.0;
        for i in 0..10_000 {
            v += s.value(black_box(i as f64 / 10_000.0 * s.duration as f64));
        }
        black_box(v);
    });
}

#[test]
#[ignore]
fn bench_ui_source_list() {
    // A 2000-row sidebar: hit-testing every row position of a 1000-pixel column, and 1000 arrow-key steps.
    let items: Vec<SidebarItem> = (0..2000u32).map(|i| if i % 50 == 0 { SidebarItem::header("Section") } else { SidebarItem::row(i, "Row") }).collect();
    let mut list = SourceList::new(items, 28.0, 22.0);
    let col = Rect { x: 0.0, y: 0.0, w: 240.0, h: 1000.0 };
    bench("ui_sidebar_hit_x1000", 21, || {
        let mut n = 0u32;
        for i in 0..1000 {
            n += list.hit(col, black_box(i as f32 * 40.0), 10.0, black_box(i as f32)).unwrap_or(0);
        }
        black_box(n);
    });
    bench("ui_sidebar_step_x1000", 21, || {
        for _ in 0..1000 {
            list.step(black_box(1));
            black_box(list.scroll_to_selected(1000.0, 0.0));
        }
    });
}

#[test]
#[ignore]
fn bench_ui_diff_10k_shuffled() {
    let old: Vec<u32> = (0..10_000).collect();
    let mut new = old.clone();
    for i in (1..new.len()).step_by(7) {
        new.swap(i, i - 1);
    }
    new.drain(100..200);
    new.extend(20_000..20_100);
    bench("ui_diff_10k_shuffled", 21, || {
        black_box(crate::diff::diff(black_box(&old), black_box(&new)));
    });
}

#[test]
#[ignore]
fn bench_ui_cache_churn_100k() {
    bench("ui_cache_churn_100k", 11, || {
        let mut c = crate::cache::Cache::new(1000);
        let mut hits = 0;
        for i in 0..100_000u32 {
            let k = (i.wrapping_mul(2654435761) >> 20) % 1500;
            if c.get(&k).is_some() {
                hits += 1;
            } else {
                c.put(k, i, 1);
            }
        }
        black_box(hits);
    });
}

#[test]
#[ignore]
fn bench_ui_fuzzy_quicksearch_5k() {
    let items: Vec<String> = (0..5000).map(|i| format!("Application Number {i} Settings Manager")).collect();
    bench("ui_fuzzy_quicksearch_5k", 11, || {
        black_box(crate::fuzzy::rank("setmgr", items.iter().map(|s| s.as_str())));
    });
}

#[test]
#[ignore]
fn bench_ui_textfit_wrap() {
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(200);
    let measure = |s: &str| s.chars().count() as f32 * 7.0;
    bench("ui_textfit_wrap_9k_chars", 21, || {
        black_box(crate::textfit::wrap(black_box(&text), 600.0, &measure));
    });
    bench("ui_textfit_truncate_middle", 101, || {
        black_box(crate::textfit::truncate(black_box(&text), 600.0, crate::textfit::Truncate::Middle, &measure));
    });
}

#[test]
#[ignore]
fn bench_ui_calendar_and_focus() {
    bench("ui_calendar_10k_dates", 51, || {
        let mut n = 0u32;
        for i in 0..10_000i64 {
            n += crate::calendar::weekday(crate::calendar::from_days(i * 3));
        }
        black_box(n);
    });
    let items: Vec<crate::focus::Focusable> = (0..500)
        .map(|i| crate::focus::Focusable { id: i, x: (i % 20) as f32 * 60.0, y: (i / 20) as f32 * 30.0, w: 50.0, h: 20.0, enabled: i % 9 != 0 })
        .collect();
    bench("ui_focus_next_500", 101, || {
        black_box(crate::focus::next(black_box(&items), Some(250), true));
    });
}

#[test]
#[ignore]
fn bench_ui_dock_drag_frame() {
    // One drag frame on a 24-icon dock: nearest slot, preview order, retarget all icons, read all offsets.
    let centres: Vec<(f32, f32)> = (0..24).map(|i| (i as f32 * 64.0 + 32.0, 40.0)).collect();
    let mut slide = crate::dockdrag::Slide::new();
    bench("ui_dock_drag_frame_x1000", 101, || {
        let mut sum = 0.0;
        for k in 0..1000 {
            let x = (k % 1500) as f32;
            let to = crate::dockdrag::slot_at(black_box(&centres), x, 40.0).unwrap();
            let order = crate::dockdrag::preview_order(24, 3, to);
            let targets: Vec<(f32, f32)> = (0..24).map(|i| (centres[order.iter().position(|&o| o == i).unwrap()].0 - centres[i].0, 0.0)).collect();
            slide.set_targets(k as f64 * 0.016, &targets);
            for i in 0..24 {
                sum += slide.offset(i, k as f64 * 0.016).0;
            }
        }
        black_box(sum);
    });
}

#[test]
#[ignore]
fn bench_ui_menu_500_items() {
    let items: Vec<crate::menu::MenuItem> = (0..500).map(|i| if i % 25 == 24 { crate::menu::MenuItem::separator() } else { crate::menu::MenuItem::new(i, &format!("Item number {i}")) }).collect();
    let mut m = crate::menu::Menu::new(items);
    bench("ui_menu_type_select_hit_x1000", 101, || {
        let mut n = 0;
        for k in 0..1000u32 {
            n += m.type_select('i', k as f64 * 2.0).unwrap_or(0);
            n += m.hit((k % 400) as f32 * 20.0, 22.0, 9.0, 5.0).unwrap_or(0);
        }
        black_box(n);
    });
}

#[test]
#[ignore]
fn bench_ui_uttype_lookup_10k_names() {
    let names: Vec<String> = (0..10_000).map(|i| format!("file number {i}.{}", ["png", "RS", "mp3", "zip", "txt", "unknown", "pdf", "MOV"][i % 8])).collect();
    bench("ui_uttype_lookup_10k_names", 51, || {
        let mut n = 0;
        for name in &names {
            if let Some(id) = crate::uttype::type_for_extension(name) {
                n += crate::uttype::mime(id).len();
            }
        }
        black_box(n);
    });
}

#[test]
#[ignore]
fn bench_ui_launch_default_for_file_10k() {
    use crate::launch::{Handler, Rank, Registry};
    let mut r = Registry::new();
    for i in 0..30 {
        r.register(Handler::new(&format!("app{i}")).opens("public.image", Rank::Alternate).opens("public.text", Rank::Default).opens(["public.png", "public.mp3", "com.adobe.pdf"][i % 3], Rank::Owner));
    }
    let names: Vec<String> = (0..10_000).map(|i| format!("file {i}.{}", ["png", "rs", "mp3", "pdf", "txt", "zip"][i % 6])).collect();
    bench("ui_launch_default_for_file_10k", 21, || {
        let mut n = 0;
        for name in &names {
            n += r.default_for_file(name).map_or(0, str::len);
        }
        black_box(n);
    });
}

#[test]
#[ignore]
fn bench_ui_detect_scan_100kb() {
    use crate::detect::{find, Kind};
    let line = "Fundur 2026-10-09 kl. 10 hjá https://bifrost.olibuijr.com/docs, Laugavegur 12 og Main Street 5, hringdu í +354 555 1234 eða sendu á www.example.is/a. ";
    let text = line.repeat(100_000 / line.len());
    bench("ui_detect_scan_100kb", 21, || {
        black_box(find(&text, &[Kind::Link, Kind::Date, Kind::Address, Kind::Phone]).len());
    });
}

#[test]
#[ignore]
fn bench_ui_constraint_solve_200_views() {
    // A column of 200 views with equal heights and 4 pt gaps: build the system, then 100 window resizes.
    use crate::constraint::{Expr, Rel, Solver, Strength};
    bench("ui_constraint_solve_200_views", 11, || {
        let mut s = Solver::new();
        let win = s.var();
        let ys: Vec<_> = (0..200).map(|_| s.var()).collect();
        let hs: Vec<_> = (0..200).map(|_| s.var()).collect();
        s.add(Expr::var(ys[0]), Rel::Eq, Strength::REQUIRED).unwrap();
        for i in 0..199 {
            s.add(Expr::var(ys[i + 1]) - Expr::var(ys[i]) - Expr::var(hs[i]) - Expr::constant(4.0), Rel::Eq, Strength::REQUIRED).unwrap();
            s.add(Expr::var(hs[i + 1]) - Expr::var(hs[i]), Rel::Eq, Strength::REQUIRED).unwrap();
            s.add(Expr::var(hs[i]) - Expr::constant(20.0), Rel::Ge, Strength::REQUIRED).unwrap();
        }
        s.add(Expr::var(ys[199]) + Expr::var(hs[199]) - Expr::var(win), Rel::Eq, Strength::REQUIRED).unwrap();
        s.add_edit(win, Strength::STRONG).unwrap();
        for i in 0..100 {
            s.suggest(win, 1080.0 + i as f64 * 3.0).unwrap();
        }
        black_box(s.value(hs[0]));
    });
}

#[test]
#[ignore]
fn bench_ui_constraint_resize_x100() {
    // Only the incremental part: 100 suggest calls on 100 equal-width views.
    use crate::constraint::{Expr, Rel, Solver, Strength};
    let mut s = Solver::new();
    let win = s.var();
    let ws: Vec<_> = (0..100).map(|_| s.var()).collect();
    for i in 0..99 {
        s.add(Expr::var(ws[i + 1]) - Expr::var(ws[i]), Rel::Eq, Strength::REQUIRED).unwrap();
    }
    let sum = ws.iter().fold(Expr::constant(0.0), |a, &w| a + Expr::var(w));
    s.add(sum - Expr::var(win), Rel::Eq, Strength::REQUIRED).unwrap();
    s.add_edit(win, Strength::STRONG).unwrap();
    let mut n = 0.0;
    bench("ui_constraint_resize_x100", 51, || {
        for i in 0..100 {
            n += 1.0;
            s.suggest(win, 1000.0 + i as f64 + n % 7.0).unwrap();
        }
        black_box(s.value(ws[0]));
    });
}
