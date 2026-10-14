//! Parity sdk-complete-42: headless tests of the nested outline tree built from the flattened PDF outline. The tree is
//! built from entries the tests make themselves, so no PDF file is read.

use bifrost_render::cg::pdf::outline_tree::{entry_count, outline_tree, titles};
use bifrost_render::cg::pdf::page::{Dest, OutlineItem};

fn entry(title: &str, page: u32, level: u8) -> OutlineItem {
    OutlineItem { title: title.into(), dest: Dest { page, left: None, top: None, zoom: None }, level }
}

#[test]
fn nested_entries_become_children_of_the_entry_above_them() {
    let items = vec![entry("Part", 1, 0), entry("Chapter 1", 2, 1), entry("Section 1.1", 3, 2), entry("Chapter 2", 4, 1), entry("Appendix", 9, 0)];
    let tree = outline_tree(&items);
    assert_eq!(tree.len(), 2, "two top-level entries");
    assert_eq!(tree[0].item.title, "Part");
    assert_eq!(tree[0].children.len(), 2);
    assert_eq!(tree[0].children[0].item.title, "Chapter 1");
    assert_eq!(tree[0].children[0].children[0].item.title, "Section 1.1");
    assert_eq!(tree[0].children[1].item.title, "Chapter 2");
    assert!(tree[0].children[1].children.is_empty());
    assert_eq!(tree[1].item.title, "Appendix");
    assert!(tree[1].children.is_empty());
}

#[test]
fn the_tree_keeps_the_reading_order_and_every_entry() {
    let items = vec![entry("A", 1, 0), entry("B", 2, 1), entry("C", 3, 2), entry("D", 4, 1), entry("E", 5, 0)];
    let tree = outline_tree(&items);
    assert_eq!(titles(&tree), vec!["A", "B", "C", "D", "E"]);
    assert_eq!(entry_count(&tree), items.len());
}

#[test]
fn a_level_that_jumps_two_steps_is_placed_under_the_nearest_open_entry() {
    let items = vec![entry("Top", 1, 0), entry("Deep", 2, 2), entry("Next", 3, 1)];
    let tree = outline_tree(&items);
    assert_eq!(tree.len(), 1);
    let names: Vec<&str> = tree[0].children.iter().map(|n| n.item.title.as_str()).collect();
    assert_eq!(names, vec!["Deep", "Next"], "the deep entry is a child of Top, and nothing is dropped");
    assert_eq!(entry_count(&tree), 3);
}

#[test]
fn an_outline_that_starts_below_the_top_level_keeps_all_of_its_entries() {
    let items = vec![entry("Indented", 1, 1), entry("Top", 2, 0)];
    let tree = outline_tree(&items);
    assert_eq!(tree.len(), 2);
    assert_eq!(titles(&tree), vec!["Indented", "Top"]);
}

#[test]
fn each_node_keeps_its_destination_page() {
    let items = vec![entry("Front", 1, 0), entry("Body", 17, 1)];
    let tree = outline_tree(&items);
    assert_eq!(tree[0].item.dest.page, 1);
    assert_eq!(tree[0].children[0].item.dest.page, 17);
}

#[test]
fn an_empty_outline_has_an_empty_tree() {
    let tree = outline_tree(&[]);
    assert!(tree.is_empty());
    assert_eq!(entry_count(&tree), 0);
    assert!(titles(&tree).is_empty());
}
