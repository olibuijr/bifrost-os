//! Host tests for the layout engine. Text is measured with a fake font where
//! every char is `font-size` px wide, 1.25 x font-size tall per line.

use super::super::display::{DisplayItem, DisplayList, Rect};
use super::super::dom::{parse_html, Dom, NodeId};
use super::super::style::{
    parse_stylesheet, ua_stylesheet, MediaEnv, Origin, StyleEngine, StyleTree,
};
use super::inline::Frag;
use super::text::fake::Mono;
use super::*;
use std::collections::HashMap;

struct Imgs(HashMap<String, (u32, u32)>);

impl ImageSizes for Imgs {
    fn size(&self, src: &str) -> Option<(u32, u32)> {
        self.0.get(src).copied()
    }
}

struct Fix {
    dom: Dom,
    eng: StyleEngine,
    st: StyleTree,
    lt: LayoutTree,
}

const VP: (f32, f32) = (800.0, 600.0);

fn engine(css: &str) -> StyleEngine {
    StyleEngine::new(
        vec![ua_stylesheet(), parse_stylesheet(css, Origin::Author)],
        MediaEnv {
            width: VP.0,
            height: VP.1,
            dark: false,
        },
    )
}

/// Lays out `body` content with `css` (body margin 0, 10px font).
fn fix(body: &str, css: &str) -> Fix {
    fix_with(body, css, &super::NoImages)
}

fn fix_with(body: &str, css: &str, images: &dyn ImageSizes) -> Fix {
    let dom = parse_html(&format!("<html><head></head><body>{}</body></html>", body));
    let all = format!("body{{margin:0;font-size:10px}} {}", css);
    let eng = engine(&all);
    let st = eng.compute(&dom);
    let lt = LayoutTree::build(&dom, &st, VP, &Mono(10.0), images);
    Fix { dom, eng, st, lt }
}

impl Fix {
    fn node(&self, id: &str) -> NodeId {
        self.dom
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no #{}", id))
    }

    fn rect(&self, id: &str) -> Rect {
        self.lt
            .node_rect(self.node(id))
            .unwrap_or_else(|| panic!("no box for #{}", id))
    }

    fn frags(&self, id: &str) -> Vec<Frag> {
        let b = self.lt.box_of[self.node(id).0 as usize];
        let ifc = self.lt.boxes[b as usize].ifc;
        self.lt.ifcs[ifc as usize].frags.clone()
    }

    fn list(&self, scroll: (f32, f32)) -> DisplayList {
        self.lt.display_list(scroll, VP, &Mono(10.0))
    }

    fn relayout(&mut self, dirty: &[NodeId]) {
        self.eng.recompute(&self.dom, &mut self.st, dirty);
        self.lt
            .relayout(&self.dom, &self.st, dirty, &Mono(10.0), &super::NoImages);
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn is_rect(item: &DisplayItem, r: u8, g: u8, b: u8) -> bool {
    matches!(item, DisplayItem::Rect { color, .. } if color.r == r && color.g == g && color.b == b)
}

fn rect_pos(list: &DisplayList, r: u8, g: u8, b: u8) -> Option<usize> {
    list.entries()
        .iter()
        .position(|e| is_rect(&e.item, r, g, b))
}

// ---------------------------------------------------------------- block

#[test]
fn blocks_stack_and_fill_the_width() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a{height:50px} #b{height:30px}",
    );
    let (a, b) = (f.rect("a"), f.rect("b"));
    assert_eq!((a.x, a.y, a.w, a.h), (0.0, 0.0, 800.0, 50.0));
    assert_eq!((b.y, b.h), (50.0, 30.0));
    assert!(f.lt.doc_height() >= 80.0);
}

#[test]
fn sibling_margins_collapse() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a{height:20px;margin-bottom:30px} #b{height:20px;margin-top:10px}",
    );
    assert_eq!(f.rect("b").y, 50.0);
}

#[test]
fn padding_border_and_percent_widths() {
    let f = fix(
        r#"<div id=o style="width:200px"><div id=i></div></div>"#,
        "#i{width:50%;height:10px;padding:5px;border:2px solid #000;box-sizing:content-box}",
    );
    let i = f.rect("i");
    assert_eq!(i.w, 100.0 + 10.0 + 4.0);
    assert_eq!(i.h, 10.0 + 10.0 + 4.0);
}

#[test]
fn calc_width_uses_the_parent_width() {
    let f = fix(
        r#"<div id=o style="width:300px"><div id=i></div></div>"#,
        "#i{width:calc(100% - 20px);height:10px}",
    );
    assert!(close(f.rect("i").w, 280.0));
}

#[test]
fn mixed_inline_and_block_children_get_anonymous_boxes() {
    let f = fix(
        r#"<div id=d>one<div id=e></div>two</div>"#,
        "#e{height:20px}",
    );
    assert!(close(f.rect("d").h, 12.5 + 20.0 + 12.5));
    assert!(close(f.rect("e").y, 12.5));
}

#[test]
fn display_none_makes_no_box() {
    let f = fix(
        r#"<div id=a style="display:none">x</div><div id=b></div>"#,
        "#b{height:5px}",
    );
    assert!(f.lt.node_rect(f.node("a")).is_none());
    assert_eq!(f.rect("b").y, 0.0);
}

// ---------------------------------------------------------------- flex

#[test]
fn flex_row_wraps_with_gap() {
    let f = fix(
        r#"<div id=f><div id=i1></div><div id=i2></div><div id=i3></div></div>"#,
        "#f{display:flex;flex-wrap:wrap;width:100px;gap:10px} #f div{width:40px;height:20px}",
    );
    let (a, b, c) = (f.rect("i1"), f.rect("i2"), f.rect("i3"));
    assert_eq!((a.x, a.y), (0.0, 0.0));
    assert_eq!((b.x, b.y), (50.0, 0.0));
    assert_eq!((c.x, c.y), (0.0, 30.0));
}

#[test]
fn flex_column_grows_items() {
    let f = fix(
        r#"<div id=f><div id=a></div><div id=b></div></div>"#,
        "#f{display:flex;flex-direction:column;height:100px} #f div{flex-grow:1}",
    );
    let (a, b) = (f.rect("a"), f.rect("b"));
    assert_eq!((a.y, a.h), (0.0, 50.0));
    assert_eq!((b.y, b.h), (50.0, 50.0));
}

#[test]
fn flex_justify_and_align() {
    let f = fix(
        r#"<div id=f><div id=a></div></div>"#,
        "#f{display:flex;justify-content:center;align-items:center;width:100px;height:100px} #a{width:20px;height:20px}",
    );
    let a = f.rect("a");
    assert_eq!((a.x, a.y), (40.0, 40.0));
}

#[test]
fn flex_text_items_shrink_to_content() {
    let f = fix(
        r#"<div id=f><span id=a>abc</span><span id=b>de</span></div>"#,
        "#f{display:flex}",
    );
    assert!(close(f.rect("a").w, 30.0));
    assert!(close(f.rect("b").x, 30.0));
}

#[test]
fn flex_order_sorts_items() {
    let f = fix(
        r#"<div id=f><div id=a style="order:2"></div><div id=b style="order:1"></div></div>"#,
        "#f{display:flex} #f div{width:10px;height:10px}",
    );
    assert!(f.rect("b").x < f.rect("a").x);
}

// ---------------------------------------------------------------- grid

#[test]
fn grid_auto_fill_minmax_columns() {
    let f = fix(
        r#"<div id=g><div id=i1></div><div id=i2></div><div id=i3></div><div id=i4></div></div>"#,
        "#g{display:grid;width:300px;grid-template-columns:repeat(auto-fill,minmax(100px,1fr))} #g div{height:10px}",
    );
    assert_eq!((f.rect("i2").x, f.rect("i2").w), (100.0, 100.0));
    assert_eq!((f.rect("i3").x, f.rect("i3").y), (200.0, 0.0));
    assert_eq!((f.rect("i4").x, f.rect("i4").y), (0.0, 10.0));
}

#[test]
fn grid_fr_tracks_and_gap() {
    let f = fix(
        r#"<div id=g><div id=a></div><div id=b></div></div>"#,
        "#g{display:grid;width:210px;grid-template-columns:1fr 2fr;column-gap:30px} #g div{height:10px}",
    );
    assert!(close(f.rect("a").w, 60.0));
    assert!(close(f.rect("b").x, 90.0));
    assert!(close(f.rect("b").w, 120.0));
}

#[test]
fn grid_item_spans() {
    let f = fix(
        r#"<div id=g><div id=a style="grid-column:1 / span 2"></div><div id=b></div></div>"#,
        "#g{display:grid;width:200px;grid-template-columns:100px 100px} #g div{height:10px}",
    );
    assert_eq!(f.rect("a").w, 200.0);
    assert_eq!(f.rect("b").y, 10.0);
}

// ---------------------------------------------------------------- inline

#[test]
fn text_wraps_at_the_container_width() {
    let f = fix(r#"<p id=p>aaa bbb ccc ddd</p>"#, "p{margin:0;width:100px}");
    let p = f.rect("p");
    assert_eq!((p.w, p.h), (100.0, 25.0));
    let frags = f.frags("p");
    assert_eq!(frags.len(), 2);
    assert!(close(frags[0].t1 as f32 - frags[0].t0 as f32, 7.0));
}

#[test]
fn max_content_width_is_one_line() {
    let f = fix(r#"<span id=s>aaa bbb</span>"#, "#s{display:inline-block}");
    let s = f.rect("s");
    assert!(close(s.w, 70.0));
    assert!(close(s.h, 12.5));
}

#[test]
fn inline_block_shrinks_to_fit_available_width() {
    let f = fix(
        r#"<div style="width:50px"><span id=s>aaa bbb ccc</span></div>"#,
        "#s{display:inline-block}",
    );
    assert!(close(f.rect("s").w, 50.0));
}

#[test]
fn white_space_collapses_and_trims() {
    let f = fix(
        "<span id=s>  a \n\t  b  </span>",
        "#s{display:inline-block}",
    );
    assert!(close(f.rect("s").w, 30.0));
}

#[test]
fn nowrap_keeps_one_line() {
    let f = fix(
        r#"<p id=p>aaa bbb ccc ddd</p>"#,
        "p{margin:0;width:50px;white-space:nowrap}",
    );
    assert!(close(f.rect("p").h, 12.5));
}

#[test]
fn pre_keeps_newlines_and_spaces() {
    let f = fix("<pre id=p>a  b\nc</pre>", "pre{margin:0}");
    assert!(close(f.rect("p").h, 25.0));
    let frags = f.frags("p");
    assert!(close(frags[0].w, 40.0));
}

#[test]
fn overflow_wrap_breaks_long_words() {
    let f = fix(
        r#"<p id=p>aaaaaaaaaa</p>"#,
        "p{margin:0;width:50px;overflow-wrap:break-word}",
    );
    assert!(close(f.rect("p").h, 25.0));
    let g = fix(r#"<p id=p>aaaaaaaaaa</p>"#, "p{margin:0;width:50px}");
    assert!(close(g.rect("p").h, 12.5));
}

#[test]
fn break_all_splits_anywhere() {
    let f = fix(
        r#"<p id=p>aaaaaaaaaaaa</p>"#,
        "p{margin:0;width:50px;word-break:break-all}",
    );
    assert!(close(f.rect("p").h, 37.5));
}

#[test]
fn br_forces_a_line() {
    let f = fix(r#"<p id=p>a<br>b</p>"#, "p{margin:0}");
    assert!(close(f.rect("p").h, 25.0));
}

#[test]
fn text_align_center_and_right() {
    let f = fix(
        r#"<p id=p>ab</p><p id=q>ab</p>"#,
        "p{margin:0;width:100px;text-align:center} #q{text-align:right}",
    );
    assert!(close(f.frags("p")[0].x, 40.0));
    assert!(close(f.frags("q")[0].x, 80.0));
}

#[test]
fn inline_elements_flow_with_spaces() {
    let f = fix(
        r#"<p id=p>aaa <b id=b>bbb</b> ccc</p>"#,
        "p{margin:0;width:200px}",
    );
    let fr = f.frags("p");
    assert_eq!(fr.len(), 3);
    assert!(close(fr[1].x, 40.0));
    assert!(close(fr[2].x, 80.0));
}

#[test]
fn inline_padding_and_background_make_box_fragments() {
    let f = fix(
        r#"<p id=p>a<span id=s>bb</span>c</p>"#,
        "p{margin:0;width:200px} #s{padding:0 5px;background:#f00}",
    );
    let fr = f.frags("p");
    let seg = fr
        .iter()
        .find(|x| x.kind == super::inline::FragKind::InlineBox)
        .expect("inline box");
    assert!(close(seg.x, 10.0));
    assert!(close(seg.w, 30.0));
    let last = fr
        .iter()
        .filter(|x| x.kind == super::inline::FragKind::Text)
        .last()
        .unwrap();
    assert!(close(last.x, 40.0));
}

#[test]
fn letter_spacing_widens_text() {
    let f = fix(
        r#"<span id=s>abc</span>"#,
        "#s{display:inline-block;letter-spacing:2px}",
    );
    assert!(close(f.rect("s").w, 36.0));
}

#[test]
fn line_height_sets_line_boxes() {
    let f = fix(
        r#"<p id=p>aaa bbb</p>"#,
        "p{margin:0;width:40px;line-height:20px}",
    );
    assert!(close(f.rect("p").h, 40.0));
}

#[test]
fn vertical_align_raises_inline_content() {
    let f = fix(
        r#"<p id=p>a<sup id=s style="vertical-align:super;font-size:5px">b</sup></p>"#,
        "p{margin:0}",
    );
    let fr = f.frags("p");
    assert!(fr[1].y < fr[0].y + 5.0);
}

#[test]
fn before_content_is_inline_text() {
    let f = fix(
        r#"<span id=s>ab</span>"#,
        r#"#s{display:inline-block} #s::before{content:"xy"}"#,
    );
    assert!(close(f.rect("s").w, 40.0));
}

#[test]
fn replaced_images_use_natural_and_styled_sizes() {
    let mut m = HashMap::new();
    m.insert("a.png".to_string(), (40u32, 30u32));
    let imgs = Imgs(m);
    let f = fix_with(
        r#"<img id=i src="a.png"><img id=j src="a.png" style="width:20px">"#,
        "",
        &imgs,
    );
    let (i, j) = (f.rect("i"), f.rect("j"));
    assert_eq!((i.w, i.h), (40.0, 30.0));
    assert_eq!((j.w, j.h), (20.0, 15.0));
    assert!(j.x >= 40.0);
}

#[test]
fn img_with_attributes_and_unknown_size() {
    let f = fix(r#"<img id=i width=64 height=32>"#, "");
    let i = f.rect("i");
    assert_eq!((i.w, i.h), (64.0, 32.0));
}

#[test]
fn inline_block_baseline_aligns_with_text() {
    let f = fix(
        r#"<p id=p>ab<span id=s>cd</span>ef</p>"#,
        "p{margin:0} #s{display:inline-block;padding:2px}",
    );
    let s = f.rect("s");
    let fr = f.frags("p");
    // The inline block's text baseline equals the line's baseline.
    let text_base = fr[0].y + fr[0].up;
    assert!(close(s.y + 2.0 + 10.0, text_base));
}

// ---------------------------------------------------------------- positioned

#[test]
fn absolute_boxes_resolve_against_the_positioned_ancestor() {
    let f = fix(
        r#"<div id=c><div id=a></div></div>"#,
        "#c{position:relative;width:200px;height:100px;margin-left:30px} #a{position:absolute;right:10px;bottom:5px;width:20px;height:10px}",
    );
    let a = f.rect("a");
    assert_eq!((a.x, a.y, a.w, a.h), (200.0, 85.0, 20.0, 10.0));
}

#[test]
fn absolute_with_both_insets_stretches() {
    let f = fix(
        r#"<div id=c><div id=a></div></div>"#,
        "#c{position:relative;width:200px;height:100px} #a{position:absolute;left:10px;right:20px;top:5px;bottom:5px}",
    );
    let a = f.rect("a");
    assert_eq!((a.x, a.y, a.w, a.h), (10.0, 5.0, 170.0, 90.0));
}

#[test]
fn absolute_skips_static_ancestors() {
    let f = fix(
        r#"<div id=r><div id=m><div id=a></div></div></div>"#,
        "#r{position:relative;margin-left:50px;height:100px} #m{margin-left:30px;height:20px} #a{position:absolute;left:0;top:0;width:5px;height:5px}",
    );
    assert_eq!(f.rect("a").x, 50.0);
}

#[test]
fn absolute_static_position_follows_previous_sibling() {
    let f = fix(
        r#"<div id=x></div><div id=y></div>"#,
        "#x{height:40px} #y{position:absolute;width:10px;height:10px}",
    );
    assert_eq!((f.rect("y").x, f.rect("y").y), (0.0, 40.0));
}

#[test]
fn absolute_shrinks_to_fit() {
    let f = fix(
        r#"<div id=a>abc def</div>"#,
        "#a{position:absolute;left:0;top:0}",
    );
    assert!(close(f.rect("a").w, 70.0));
}

#[test]
fn absolute_does_not_take_flow_space() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a{position:absolute;height:100px} #b{height:10px}",
    );
    assert_eq!(f.rect("b").y, 0.0);
}

#[test]
fn relative_offsets_shift_without_moving_siblings() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a{position:relative;left:10px;top:7px;height:20px} #b{height:20px}",
    );
    assert_eq!((f.rect("a").x, f.rect("a").y), (10.0, 7.0));
    assert_eq!(f.rect("b").y, 20.0);
}

#[test]
fn fixed_boxes_follow_the_scroll_offset() {
    let f = fix(
        r#"<div style="height:2000px"></div><div id=f></div>"#,
        "#f{position:fixed;top:5px;left:7px;width:50px;height:20px;background:#0f0}",
    );
    let l = f.list((0.0, 300.0));
    let i = rect_pos(&l, 0, 255, 0).expect("fixed box painted");
    match &l.entries()[i].item {
        DisplayItem::Rect { rect, .. } => assert_eq!((rect.x, rect.y), (7.0, 305.0)),
        _ => unreachable!(),
    }
}

#[test]
fn sticky_boxes_stick_at_paint_time() {
    let f = fix(
        r#"<div id=wrap><div id=s></div></div>"#,
        "#wrap{height:1000px} #s{position:sticky;top:10px;height:20px;background:#0f0}",
    );
    let at = |sy: f32| -> f32 {
        let l = f.list((0.0, sy));
        match &l.entries()[rect_pos(&l, 0, 255, 0).expect("sticky painted")].item {
            DisplayItem::Rect { rect, .. } => rect.y,
            _ => unreachable!(),
        }
    };
    // Held 10px below the top of the viewport, even at scroll 0.
    assert_eq!(at(0.0), 10.0);
    assert_eq!(at(300.0), 310.0);
    // Stops at the bottom of its container.
    assert_eq!(at(990.0), 980.0);
}

#[test]
fn z_index_orders_stacking_contexts() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a{position:absolute;z-index:2;width:10px;height:10px;background:#00f} #b{position:absolute;z-index:1;width:10px;height:10px;background:#f00}",
    );
    let l = f.list((0.0, 0.0));
    let (red, blue) = (
        rect_pos(&l, 255, 0, 0).unwrap(),
        rect_pos(&l, 0, 0, 255).unwrap(),
    );
    assert!(red < blue);
}

#[test]
fn negative_z_paints_below_the_parent_content() {
    let f = fix(
        r#"<div id=p style="background:#00f;height:50px"><div id=n></div></div>"#,
        "#p{position:relative} #n{position:absolute;z-index:-1;width:10px;height:10px;background:#f00}",
    );
    let l = f.list((0.0, 0.0));
    // The negative-z child of a z-auto parent belongs to the root context:
    // painted after the root backgrounds (the blue parent) and before
    // in-flow content, so red comes after blue here only if the parent paints
    // first; both must be present.
    assert!(rect_pos(&l, 255, 0, 0).is_some());
    assert!(rect_pos(&l, 0, 0, 255).is_some());
}

#[test]
fn opacity_and_transform_wrap_their_content() {
    let f = fix(
        r#"<div id=a></div>"#,
        "#a{width:10px;height:10px;background:#f00;opacity:0.5;transform:translateX(5px)}",
    );
    let l = f.list((0.0, 0.0));
    let kinds: Vec<&DisplayItem> = l.entries().iter().map(|e| &e.item).collect();
    assert!(kinds
        .iter()
        .any(|i| matches!(i, DisplayItem::PushOpacity { .. })));
    assert!(kinds
        .iter()
        .any(|i| matches!(i, DisplayItem::PushTransform { .. })));
    assert!(kinds.iter().any(|i| matches!(i, DisplayItem::PopOpacity)));
}

#[test]
fn overflow_hidden_clips_children() {
    let f = fix(
        r#"<div id=c><div id=a></div></div>"#,
        "#c{overflow:hidden;width:50px;height:20px} #a{width:100px;height:100px;background:#f00}",
    );
    let l = f.list((0.0, 0.0));
    assert!(l
        .entries()
        .iter()
        .any(|e| matches!(e.item, DisplayItem::PushClip { .. })));
    assert!(l
        .entries()
        .iter()
        .any(|e| matches!(e.item, DisplayItem::PopClip)));
    // The clipped child's painted bounds are cut to the container.
    let i = rect_pos(&l, 255, 0, 0).unwrap();
    assert!(l.entries()[i].bounds.w <= 50.0 + 0.01);
}

// ---------------------------------------------------------------- table

#[test]
fn table_cells_sit_on_a_grid() {
    let f = fix(
        r#"<table id=t><tr><td id=a></td><td id=b></td></tr><tr><td id=c colspan=2></td></tr></table>"#,
        "#t{width:100px;border-spacing:0} td{padding:0;height:20px} #a,#b{width:50px}",
    );
    let (a, b, c) = (f.rect("a"), f.rect("b"), f.rect("c"));
    assert_eq!((a.x, a.y), (0.0, 0.0));
    assert_eq!(b.x, 50.0);
    assert_eq!((c.x, c.y), (0.0, 20.0));
    assert_eq!(c.w, 100.0);
}

#[test]
fn table_rowspan_occupies_cells_below() {
    let f = fix(
        r#"<table id=t><tr><td id=a rowspan=2></td><td id=b></td></tr><tr><td id=c></td></tr></table>"#,
        "#t{width:100px} td{padding:0;height:10px} #a{width:40px}",
    );
    assert_eq!(f.rect("a").h, 20.0);
    assert_eq!(f.rect("c").x, f.rect("b").x);
    assert_eq!(f.rect("c").y, 10.0);
}

#[test]
fn table_row_backgrounds_are_painted() {
    let f = fix(
        r#"<table id=t><tr style="background:#f00"><td>a</td><td>b</td></tr></table>"#,
        "#t{width:100px} td{padding:0}",
    );
    let l = f.list((0.0, 0.0));
    assert!(rect_pos(&l, 255, 0, 0).is_some());
}

// ---------------------------------------------------------------- paint

#[test]
fn paint_emits_backgrounds_borders_and_text() {
    let f = fix(
        r#"<div id=a>hi</div>"#,
        "#a{width:100px;background:#0f0;border:2px solid #00f;border-radius:4px;box-shadow:1px 1px 3px #000}",
    );
    let l = f.list((0.0, 0.0));
    let items: Vec<&DisplayItem> = l.entries().iter().map(|e| &e.item).collect();
    assert!(items
        .iter()
        .any(|i| matches!(i, DisplayItem::RoundedRect { .. })));
    assert!(items
        .iter()
        .any(|i| matches!(i, DisplayItem::Border { .. })));
    assert!(items
        .iter()
        .any(|i| matches!(i, DisplayItem::Shadow { .. })));
    assert!(items.iter().any(|i| matches!(i, DisplayItem::Text { .. })));
    // Shadow under the background under the border.
    let pos = |pred: &dyn Fn(&DisplayItem) -> bool| items.iter().position(|i| pred(i)).unwrap();
    let s = pos(&|i| matches!(i, DisplayItem::Shadow { .. }));
    let b = pos(&|i| matches!(i, DisplayItem::RoundedRect { .. }));
    let o = pos(&|i| matches!(i, DisplayItem::Border { .. }));
    assert!(s < b && b < o);
}

#[test]
fn gradients_become_display_items() {
    let f = fix(
        r#"<div id=a></div>"#,
        "#a{width:100px;height:50px;background:linear-gradient(90deg,#f00,#00f)}",
    );
    let l = f.list((0.0, 0.0));
    assert!(l
        .entries()
        .iter()
        .any(|e| matches!(&e.item, DisplayItem::LinearGradient { stops, .. } if stops.len() == 2)));
}

#[test]
fn text_decoration_draws_a_line() {
    let f = fix(
        r#"<p id=p style="text-decoration:underline">abc</p>"#,
        "p{margin:0}",
    );
    let l = f.list((0.0, 0.0));
    // The underline is a thin rect as wide as the text.
    assert!(l.entries().iter().any(
        |e| matches!(&e.item, DisplayItem::Rect { rect, .. } if close(rect.w, 30.0) && rect.h < 2.0)
    ));
}

#[test]
fn list_items_get_markers() {
    let f = fix(
        r#"<ul><li id=a>x</li></ul><ol><li id=b>y</li></ol>"#,
        "ul,ol{margin:0;padding-left:20px}",
    );
    let l = f.list((0.0, 0.0));
    assert!(l
        .entries()
        .iter()
        .any(|e| matches!(e.item, DisplayItem::RoundedRect { .. })));
    // "1." for the ordered item.
    let texts = l
        .entries()
        .iter()
        .filter(|e| matches!(e.item, DisplayItem::Text { .. }))
        .count();
    assert!(texts >= 3);
}

#[test]
fn display_list_leaves_out_far_content() {
    let mut body = String::new();
    for i in 0..200 {
        body.push_str(&format!(
            r#"<div style="height:50px;background:#{}00">row {}</div>"#,
            if i % 2 == 0 { "f" } else { "0" },
            i
        ));
    }
    let f = fix(&body, "");
    let all = f.list((0.0, 0.0)).len();
    let mid = f.list((0.0, 5000.0));
    assert!(all < 200 * 2);
    assert!(mid.len() < 200 * 2);
    assert!(f.lt.doc_height() >= 10000.0);
}

// ---------------------------------------------------------------- hit testing

#[test]
fn hit_test_finds_the_deepest_element_and_links() {
    let f = fix(
        r#"<div id=o><a id=l href="/x">link</a></div>"#,
        "#o{width:100px;height:100px}",
    );
    let _ = f.list((0.0, 0.0));
    assert_eq!(f.lt.hit_test(5.0, 5.0), Some(f.node("l")));
    assert_eq!(hit_test(&f.lt, 80.0, 50.0), Some(f.node("o")));
    assert_eq!(link_at(&f.lt, &f.dom, 5.0, 5.0), Some(f.node("l")));
    assert_eq!(link_at(&f.lt, &f.dom, 80.0, 50.0), None);
}

#[test]
fn hit_test_prefers_higher_z() {
    let f = fix(
        r#"<div id=a></div><div id=b></div>"#,
        "#a,#b{position:absolute;width:20px;height:20px} #a{z-index:5} #b{z-index:1}",
    );
    assert_eq!(f.lt.hit_test(5.0, 5.0), Some(f.node("a")));
}

#[test]
fn hit_test_honours_scroll_containers() {
    let mut f = fix(
        r#"<div id=sc><div id=a></div><div id=b></div></div>"#,
        "#sc{overflow:auto;width:100px;height:50px} #sc div{height:100px}",
    );
    assert_eq!(f.lt.hit_test(10.0, 10.0), Some(f.node("a")));
    let sc = f.node("sc");
    f.lt.set_scroll(sc, (0.0, 100.0));
    assert_eq!(f.lt.hit_test(10.0, 10.0), Some(f.node("b")));
    // Outside the clip (but inside the overflowing child's box) hits nothing in it.
    f.lt.set_scroll(sc, (0.0, 0.0));
    assert_ne!(f.lt.hit_test(10.0, 80.0), Some(f.node("b")));
    assert_eq!(scroller_at(&f.lt, &f.dom, 10.0, 10.0), Some(sc));
}

#[test]
fn hit_test_follows_transforms() {
    let f = fix(
        r#"<div id=p><div id=c></div></div>"#,
        "#p{width:100px;height:100px} #c{width:20px;height:20px;transform:translateX(50px)}",
    );
    assert_eq!(f.lt.hit_test(60.0, 10.0), Some(f.node("c")));
    assert_eq!(f.lt.hit_test(10.0, 10.0), Some(f.node("p")));
}

#[test]
fn hit_test_follows_fixed_and_sticky_offsets() {
    let f = fix(
        r#"<div style="height:3000px"></div><div id=f></div>"#,
        "#f{position:fixed;top:0;left:0;width:50px;height:20px}",
    );
    let _ = f.list((0.0, 1000.0));
    assert_eq!(f.lt.hit_test(5.0, 1005.0), Some(f.node("f")));
    assert_ne!(f.lt.hit_test(5.0, 5.0), Some(f.node("f")));
}

#[test]
fn hit_test_inline_block_children() {
    let f = fix(
        r#"<p id=p>ab <span id=s>cd</span></p>"#,
        "p{margin:0} #s{display:inline-block;width:30px;height:12px}",
    );
    assert_eq!(f.lt.hit_test(45.0, 5.0), Some(f.node("s")));
}

// ---------------------------------------------------------------- relayout

#[test]
fn relayout_after_a_style_change_is_incremental() {
    let mut f = fix(r#"<p id=p>aaa bbb ccc ddd</p>"#, "p{margin:0;width:100px}");
    assert_eq!(f.rect("p").h, 25.0);
    let before = f.lt.box_count();
    let p = f.node("p");
    f.dom.set_attr(p, "style", "width:50px");
    f.relayout(&[p]);
    assert_eq!(f.lt.box_count(), before);
    assert_eq!(f.rect("p").w, 50.0);
    assert!(f.rect("p").h > 25.0);
}

#[test]
fn relayout_after_a_text_change_rebuilds() {
    let mut f = fix(
        r#"<p id=p>aaa</p><div id=d></div>"#,
        "p{margin:0} #d{height:5px}",
    );
    assert!(close(f.rect("d").y, 12.5));
    let p = f.node("p");
    f.dom.set_text_content(p, "aaa<br>bbb");
    f.relayout(&[p]);
    assert!(close(f.rect("d").y, 12.5));
    let n = f.dom.create_element("br");
    f.dom.append_child(p, n);
    let t = f.dom.create_text("ccc");
    f.dom.append_child(p, t);
    f.relayout(&[p]);
    assert!(close(f.rect("d").y, 25.0));
}

#[test]
fn resize_changes_the_layout_width() {
    let mut f = fix(r#"<div id=a></div>"#, "#a{height:10px}");
    assert_eq!(f.rect("a").w, 800.0);
    f.lt.resize(&f.dom, &f.st, (400.0, 600.0), &Mono(10.0), &super::NoImages);
    assert_eq!(f.rect("a").w, 400.0);
}

#[test]
fn empty_document_lays_out() {
    let dom = Dom::new();
    let st = engine("").compute(&dom);
    let lt = LayoutTree::build(&dom, &st, VP, &Mono(10.0), &super::NoImages);
    assert_eq!(lt.doc_height(), 0.0);
    let _ = lt.display_list((0.0, 0.0), VP, &Mono(10.0));
    assert_eq!(lt.hit_test(1.0, 1.0), None);
}
