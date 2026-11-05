//! Parity tests (sdk-tests-09) for three shell models: the accessibility tree (`shell::atspi::model`), the NV12 picture
//! conversion (`media::convert`) and the PDF reflow of the Books reader (`shell::books::engine::pdf`). Headless host
//! tests: no D-Bus bus, no display, no device and no clock. Every input is built in memory, and each test drives the
//! public API of the module it names.

use bifrost_proto::{AxMeta, AxText, Node, NodeExt, Role, UiRect as Rect};
use bifrost_render::cg::pdf::extract::Piece;
use bifrost_render::cg::pdf::writer::{write, Page as PdfPage};
use bifrost_render::gfx::Image;
use springboard::media::convert::{fit, matrix_for, nv12_to_rgba, rgba_scale, Nv12, BT601, BT709};
use springboard::shell::atspi::model::{
    action_names, app_path, build, build_ax, children, chrome_snap, key_path, node_path, numeric, parse_path, role, role_name,
    state, states, text_edits, window_states, atspi_role, Key, SNode, TextEdit, CHROME_SLOT, ROOT_PATH,
};
use springboard::shell::books::engine::model::Block;
use springboard::shell::books::engine::pdf::{assemble, load, probe};

fn rect() -> Rect {
    Rect { x: 1., y: 2., w: 30., h: 40. }
}

fn node(id: u64, role: Role, label: &str, value: &str) -> Node {
    Node { id, role, label: label.into(), value: value.into(), bounds: rect(), enabled: true }
}

fn ext(id: u64, parent: Option<u64>) -> NodeExt {
    NodeExt { id, parent, focused: false, checked: None }
}

fn has(words: [u32; 2], bit: u32) -> bool {
    words[(bit / 32) as usize] >> (bit % 32) & 1 == 1
}

fn snode(id: u64, role: Role, enabled: bool, checked: Option<bool>) -> SNode {
    SNode {
        id,
        parent: None,
        role,
        name: format!("n{id}"),
        value: String::new(),
        bounds: rect(),
        enabled,
        focused: false,
        checked,
        title: String::new(),
        help: String::new(),
        text: None,
        actions: Vec::new(),
    }
}

// --- accessibility tree -------------------------------------------------------------------------------------------

#[test]
fn headless_each_role_maps_to_its_atspi_role_and_name() {
    assert_eq!(atspi_role(Role::Button), role::PUSH_BUTTON);
    assert_eq!(role_name(Role::Button), "push button");
    assert_eq!(atspi_role(Role::TextField), role::ENTRY);
    assert_eq!(role_name(Role::TextField), "entry");
    assert_eq!(atspi_role(Role::Window), role::FRAME);
    assert_eq!(role_name(Role::SegmentedControl), "page tab list");
    assert_eq!(atspi_role(Role::ProgressIndicator), role::PROGRESS_BAR);
    assert_eq!(role_name(Role::MenuItem), "menu item");
}

#[test]
fn headless_a_secure_field_is_a_password_and_shows_only_bullets() {
    let text = AxText { id: 1, secure: true, len: 7, ..Default::default() };
    let nodes = build_ax(&[node(1, Role::TextField, "PIN", "hunter2")], &[], &[text], &[]);
    assert_eq!(nodes[0].atspi_role(), role::PASSWORD_TEXT);
    assert_eq!(nodes[0].value, "\u{2022}".repeat(7), "one bullet per character");
    assert!(!nodes[0].value.contains("hunter"), "no character of the content is exposed");
}

#[test]
fn headless_a_plain_field_keeps_its_value_and_entry_role() {
    let text = AxText { id: 1, secure: false, len: 5, caret: 5, ..Default::default() };
    let nodes = build_ax(&[node(1, Role::TextField, "Name", "Hello")], &[], &[text], &[]);
    assert_eq!(nodes[0].atspi_role(), role::ENTRY);
    assert_eq!(nodes[0].value, "Hello");
    assert_eq!(nodes[0].text.map(|t| t.caret), Some(5));
}

#[test]
fn headless_the_title_stands_in_for_a_missing_label() {
    let metas = [AxMeta { id: 1, title: "Save".into(), help: "Writes the file".into() }];
    let nodes = build_ax(&[node(1, Role::Button, "", "")], &[], &[], &metas);
    assert_eq!(nodes[0].accessible_name(), "Save");
    assert_eq!(nodes[0].help, "Writes the file");
    let labelled = build_ax(&[node(2, Role::Button, "Open", "")], &[], &[], &[AxMeta { id: 2, title: "Title".into(), help: String::new() }]);
    assert_eq!(labelled[0].accessible_name(), "Open", "the label wins over the title");
}

#[test]
fn headless_a_parent_that_is_missing_or_the_node_itself_is_dropped() {
    let nodes = [node(1, Role::Row, "a", ""), node(2, Role::Row, "b", "")];
    let ext_list = [NodeExt { id: 1, parent: Some(9), focused: false, checked: None }, NodeExt { id: 2, parent: Some(2), focused: false, checked: None }];
    let out = build(&nodes, &ext_list);
    assert_eq!(out[0].parent, None, "a parent that is not in the packet");
    assert_eq!(out[1].parent, None, "a node that names itself as its parent");
}

#[test]
fn headless_a_cycle_in_the_parent_links_makes_both_nodes_window_children() {
    let nodes = [node(1, Role::Row, "a", ""), node(2, Role::Row, "b", "")];
    let links = [ext(1, Some(2)), ext(2, Some(1))];
    let out = build(&nodes, &links);
    assert!(out.iter().all(|n| n.parent.is_none()), "{out:?}");
}

#[test]
fn headless_children_lists_the_rows_under_their_list_in_packet_order() {
    let nodes = [
        node(10, Role::List, "Files", ""),
        node(11, Role::Row, "one", ""),
        node(12, Role::Row, "two", ""),
        node(13, Role::Button, "Close", ""),
    ];
    let links = [ext(11, Some(10)), ext(12, Some(10))];
    let out = build(&nodes, &links);
    assert_eq!(children(&out, None), vec![10, 13]);
    assert_eq!(children(&out, Some(10)), vec![11, 12]);
}

#[test]
fn headless_a_toggle_reads_its_checked_state_from_its_value_text() {
    let nodes = [node(1, Role::Toggle, "Wi-Fi", "on"), node(2, Role::Checkbox, "Mute", "off")];
    let out = build(&nodes, &[]);
    assert_eq!(out[0].checked, Some(true));
    assert_eq!(out[1].checked, Some(false));
    let told = build(&nodes, &[NodeExt { id: 1, parent: None, focused: false, checked: Some(false) }]);
    assert_eq!(told[0].checked, Some(false), "the reported state beats the guess from the text");
}

#[test]
fn headless_an_enabled_focused_checked_control_sets_its_states() {
    let mut n = snode(1, Role::Checkbox, true, Some(true));
    n.focused = true;
    let w = states(&n);
    assert!(has(w, state::VISIBLE) && has(w, state::SHOWING));
    assert!(has(w, state::ENABLED) && has(w, state::SENSITIVE));
    assert!(has(w, state::FOCUSABLE) && has(w, state::FOCUSED));
    assert!(has(w, state::CHECKABLE) && has(w, state::CHECKED));
}

#[test]
fn headless_a_disabled_static_text_is_visible_but_not_focusable_or_sensitive() {
    let n = snode(2, Role::StaticText, false, None);
    let w = states(&n);
    assert!(has(w, state::VISIBLE));
    assert!(!has(w, state::ENABLED) && !has(w, state::SENSITIVE));
    assert!(!has(w, state::FOCUSABLE));
    assert!(!has(w, state::CHECKABLE));
}

#[test]
fn headless_a_text_field_is_editable_and_single_line() {
    let w = states(&snode(3, Role::TextField, true, None));
    assert!(has(w, state::EDITABLE) && has(w, state::SINGLE_LINE));
    assert!(!has(states(&snode(4, Role::Button, true, None)), state::EDITABLE));
}

#[test]
fn headless_the_window_frame_is_active_and_shown() {
    let w = window_states();
    assert!(has(w, state::ACTIVE) && has(w, state::ENABLED) && has(w, state::SHOWING));
    assert!(!has(w, state::FOCUSED));
}

#[test]
fn headless_object_paths_round_trip_for_apps_and_nodes() {
    assert_eq!(app_path(3), "/org/a11y/atspi/accessible/root/app3");
    assert_eq!(parse_path(&node_path(3, 42)), Some(Key::Node(3, 42)));
    assert_eq!(parse_path(&app_path(3)), Some(Key::App(3)));
    assert_eq!(parse_path(ROOT_PATH), Some(Key::Root));
    assert_eq!(key_path(Key::Node(3, 42)), node_path(3, 42));
    assert_eq!(parse_path("/org/elsewhere"), None);
}

#[test]
fn headless_the_shell_chrome_is_a_tree_of_menus_and_dock_items() {
    let menus = [("File".to_string(), vec![("New".to_string(), true), ("Close".to_string(), false)])];
    let dock = [("Notes".to_string(), true)];
    let snap = chrome_snap(&menus, &dock);
    assert_eq!(snap.slot, CHROME_SLOT);
    assert_eq!(snap.nodes.len(), 7, "bar, one menu, two rows, dock, one dock app and the search field");
    assert_eq!(children(&snap.nodes, Some(100)), vec![101, 102], "the menu nests its rows");
    assert!(!snap.nodes.iter().find(|n| n.id == 102).unwrap().enabled, "a disabled row stays disabled");
    assert_eq!(children(&snap.nodes, None), vec![1, 2, 3]);
    assert_eq!(snap.nodes.iter().find(|n| n.id == 1000).unwrap().value, "open");
}

#[test]
fn headless_numeric_elements_offer_increment_and_decrement() {
    assert_eq!(action_names(false), vec!["press"]);
    assert_eq!(action_names(true), vec!["press", "increment", "decrement"]);
}

#[test]
fn headless_numeric_values_read_fractions_percents_and_ratios() {
    assert_eq!(numeric("0.4"), Some((0.4, 1.0)));
    assert_eq!(numeric("40%"), Some((40.0, 100.0)));
    assert_eq!(numeric("3/10"), Some((3.0, 10.0)));
    assert_eq!(numeric("250"), Some((250.0, 250.0)));
    assert_eq!(numeric("3/0"), None, "a zero maximum is not a range");
    assert_eq!(numeric("loud"), None);
}

#[test]
fn headless_text_edits_report_a_replace_as_a_delete_then_an_insert() {
    let edits = text_edits("hello", "help");
    assert_eq!(
        edits,
        vec![
            TextEdit { insert: false, at: 3, len: 2, text: "lo".into() },
            TextEdit { insert: true, at: 3, len: 1, text: "p".into() },
        ]
    );
    assert!(text_edits("same", "same").is_empty());
    assert_eq!(text_edits("", "ab"), vec![TextEdit { insert: true, at: 0, len: 2, text: "ab".into() }]);
}

// --- picture conversion -------------------------------------------------------------------------------------------

/// A `w` x `h` NV12 picture with every luma sample `y` and every chroma sample `c`.
fn flat_nv12(w: usize, h: usize, y: u8, c: u8) -> (Vec<u8>, Vec<u8>) {
    (vec![y; w * h], vec![c; w * h / 2])
}

#[test]
fn headless_fit_scales_a_picture_to_the_viewport_width_when_it_is_wider() {
    assert_eq!(fit(1920, 1080, 300, 300), (300, 168));
    assert_eq!(fit(720, 1280, 300, 300), (168, 300), "a tall picture is limited by the height");
}

#[test]
fn headless_matrix_choice_follows_the_picture_height() {
    assert_eq!(matrix_for(1080).rv, BT709.rv);
    assert_eq!(matrix_for(720).bu, BT709.bu);
    assert_eq!(matrix_for(480).rv, BT601.rv);
}

#[test]
fn headless_a_mid_grey_nv12_picture_converts_to_opaque_grey_pixels() {
    let (y, uv) = flat_nv12(16, 8, 126, 128);
    let img = nv12_to_rgba(Nv12 { y: &y, uv: &uv, stride: 16, w: 16, h: 8 }, 16, 8, BT601, 2);
    assert_eq!((img.w, img.h, img.px.len()), (16, 8, 128));
    assert!(img.px.iter().all(|&p| p == 0xff80_8080), "{:08x}", img.px[0]);
}

#[test]
fn headless_neutral_chroma_gives_the_same_pixels_under_both_matrices() {
    let (y, uv) = flat_nv12(16, 8, 90, 128);
    let src = Nv12 { y: &y, uv: &uv, stride: 16, w: 16, h: 8 };
    let bt601 = nv12_to_rgba(src, 16, 8, BT601, 1);
    let bt709 = nv12_to_rgba(src, 16, 8, BT709, 1);
    assert_eq!(bt601.px, bt709.px, "no colour difference is encoded in the chroma");
}

#[test]
fn headless_the_output_of_a_scaled_conversion_has_the_requested_size() {
    let (y, uv) = flat_nv12(16, 8, 126, 128);
    let img = nv12_to_rgba(Nv12 { y: &y, uv: &uv, stride: 16, w: 16, h: 8 }, 5, 3, BT601, 1);
    assert_eq!((img.w, img.h, img.px.len()), (5, 3, 15));
    assert!(img.px.iter().all(|&p| p == 0xff80_8080));
}

#[test]
fn headless_the_pixels_do_not_depend_on_how_many_threads_convert_them() {
    let (w, h) = (16usize, 8usize);
    let y: Vec<u8> = (0..w * h).map(|i| ((i % w) * 15) as u8).collect();
    let uv = vec![128u8; w * h / 2];
    let src = Nv12 { y: &y, uv: &uv, stride: w, w, h };
    let one = nv12_to_rgba(src, 8, 4, BT601, 1);
    let three = nv12_to_rgba(src, 8, 4, BT601, 3);
    assert_eq!(one.px, three.px);
    assert!(one.px[0] & 0xff < one.px[7] & 0xff, "the luma ramp is kept across the picture");
}

#[test]
fn headless_a_picture_with_a_short_plane_converts_to_a_blank_frame() {
    let (y, uv) = flat_nv12(16, 8, 200, 128);
    let img = nv12_to_rgba(Nv12 { y: &y[..10], uv: &uv, stride: 16, w: 16, h: 8 }, 8, 4, BT601, 1);
    assert_eq!(img.px.len(), 32);
    assert!(img.px.iter().all(|&p| p == 0), "no pixel is read from a truncated plane");
}

#[test]
fn headless_scaling_a_ramp_keeps_it_monotonic_and_opaque() {
    let src = Image { w: 2, h: 1, px: vec![0xff00_0000, 0xffff_ffff] };
    let out = rgba_scale(&src, 4, 1);
    let red: Vec<u32> = out.px.iter().map(|p| p & 0xff).collect();
    assert_eq!(red[0], 0, "the first output pixel samples the dark source pixel");
    assert_eq!(red[3], 255, "the last output pixel samples the bright source pixel");
    assert!(red.windows(2).all(|w| w[0] <= w[1]), "{red:?}");
    assert!(out.px.iter().all(|p| p >> 24 == 0xff));
}

// --- Books PDF reflow ----------------------------------------------------------------------------------------------

fn line(y: f32, size: f32, text: &str) -> Piece {
    Piece::Line { y, size, text: text.into() }
}

fn texts(blocks: &[Block]) -> Vec<String> {
    blocks.iter().filter_map(|b| b.text().map(str::to_string)).collect()
}

#[test]
fn headless_a_large_line_above_the_body_size_becomes_a_level_one_heading() {
    let pieces = vec![Piece::Page, line(740.0, 24.0, "Chapter"), line(700.0, 12.0, "Body text."), line(686.0, 12.0, "More.")];
    let (blocks, starts) = assemble(pieces);
    assert_eq!(texts(&blocks), ["Chapter", "Body text. More."]);
    assert!(matches!(blocks[0], Block::Heading(1, _)));
    assert_eq!(starts, vec![0]);
}

#[test]
fn headless_a_heading_of_one_and_a_half_times_the_body_is_level_two() {
    let pieces = vec![Piece::Page, line(740.0, 18.0, "Part Two"), line(700.0, 12.0, "Text one."), line(686.0, 12.0, "text two.")];
    let (blocks, _) = assemble(pieces);
    assert!(matches!(blocks[0], Block::Heading(2, _)), "{:?}", texts(&blocks));
}

#[test]
fn headless_a_hyphen_at_a_line_end_joins_the_next_word() {
    let (blocks, _) = assemble(vec![Piece::Page, line(700.0, 12.0, "Ex-"), line(686.0, 12.0, "ample.")]);
    assert_eq!(texts(&blocks), ["Example."]);
}

#[test]
fn headless_a_large_vertical_gap_starts_a_new_paragraph() {
    let (blocks, _) = assemble(vec![Piece::Page, line(700.0, 12.0, "First."), line(600.0, 12.0, "Second.")]);
    assert_eq!(texts(&blocks), ["First.", "Second."]);
}

#[test]
fn headless_bare_page_numbers_are_chrome_and_never_become_text() {
    let (blocks, _) = assemble(vec![Piece::Page, line(700.0, 12.0, "Only text."), line(60.0, 12.0, "42")]);
    assert_eq!(texts(&blocks), ["Only text."]);
}

#[test]
fn headless_an_image_is_its_own_block_between_the_paragraphs_around_it() {
    let picture = Image { w: 1, h: 1, px: vec![0xff00_0000] };
    let pieces = vec![
        Piece::Page,
        line(700.0, 12.0, "Before the picture."),
        Piece::Image(picture),
        line(700.0, 12.0, "After the picture."),
    ];
    let (blocks, _) = assemble(pieces);
    assert_eq!(blocks.len(), 3);
    assert!(blocks[1].text().is_none(), "the picture block carries no text");
    assert_eq!(texts(&blocks), ["Before the picture.", "After the picture."]);
}

#[test]
fn headless_a_file_that_is_not_a_pdf_is_not_probed_and_does_not_load() {
    assert!(probe(b"plain text".to_vec()).is_none());
    assert!(load(b"%PDF-1.4 garbage".to_vec()).is_err());
}

#[test]
fn headless_a_pdf_without_text_is_refused_by_the_reader() {
    let bytes = write(&[PdfPage { width: 200.0, height: 100.0, content: b"0 0 1 rg 10 10 50 20 re f".to_vec(), images: Vec::new() }], &[]);
    assert!(probe(bytes.clone()).is_some(), "the structure reads");
    match load(bytes) {
        Err(e) => assert!(e.contains("no readable text"), "{e}"),
        Ok(_) => panic!("a drawing without text is not a readable book"),
    }
}
