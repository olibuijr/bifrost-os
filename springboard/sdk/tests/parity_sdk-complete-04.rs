//! Parity sdk-complete-04: sort descriptors of the file metadata query. The tests gather real files in a temporary
//! directory, so they need no device and no clock; the ordering is checked on the results the query returns.

use bifrost_app::metadata_query::{
    MetadataAttribute, MetadataOp, MetadataPredicate, MetadataQuery, MetadataSortDescriptor, MetadataValue,
};
use std::fs;
use std::path::{Path, PathBuf};

fn write_bytes(path: &Path, size: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, vec![b'x'; size]).unwrap();
}

fn names(query: &MetadataQuery) -> Vec<String> {
    query.results().iter().map(|item| item.path.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

/// Three files of different sizes and a folder, gathered with no predicate.
fn scope() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_path_buf();
    write_bytes(&root.join("bravo.txt"), 30);
    write_bytes(&root.join("alpha.txt"), 10);
    write_bytes(&root.join("charlie.txt"), 20);
    write_bytes(&root.join("folder/inside.txt"), 5);
    (t, root)
}

/// Matches the plain files only: a folder's size is the file system's, not a fixed value.
fn files_only() -> MetadataPredicate {
    MetadataPredicate::compare(MetadataAttribute::IsDirectory, MetadataOp::Equal, MetadataValue::Bool(false))
}

fn started(root: &Path) -> MetadataQuery {
    let mut q = MetadataQuery::new();
    q.search_scopes = vec![root.to_path_buf()];
    assert!(q.start_query());
    q
}

#[test]
fn a_query_without_sort_descriptors_lists_results_in_path_order() {
    let (_t, root) = scope();
    let q = started(&root);
    assert!(q.sort_descriptors().is_empty());
    assert_eq!(names(&q), vec!["alpha.txt", "bravo.txt", "charlie.txt", "folder", "inside.txt"]);
}

#[test]
fn sorting_by_name_descending_reverses_the_names() {
    let (_t, root) = scope();
    let mut q = started(&root);
    q.set_sort_descriptors(vec![MetadataSortDescriptor::descending(MetadataAttribute::FsName)]);
    assert_eq!(names(&q), vec!["inside.txt", "folder", "charlie.txt", "bravo.txt", "alpha.txt"]);
}

#[test]
fn sorting_by_size_ascending_puts_the_smallest_file_first() {
    let (_t, root) = scope();
    let mut q = MetadataQuery::new();
    q.search_scopes = vec![root];
    q.predicate = Some(files_only());
    assert!(q.start_query());
    q.set_sort_descriptors(vec![MetadataSortDescriptor::ascending(MetadataAttribute::FsSize)]);
    let sizes: Vec<i64> = q
        .results()
        .iter()
        .map(|item| match item.value(MetadataAttribute::FsSize) {
            Some(MetadataValue::Int(n)) => *n,
            other => panic!("no size for {:?}: {other:?}", item.path),
        })
        .collect();
    assert_eq!(sizes, vec![5, 10, 20, 30]);
    assert_eq!(names(&q), vec!["inside.txt", "alpha.txt", "charlie.txt", "bravo.txt"]);
}

#[test]
fn a_later_key_breaks_ties_left_by_the_first() {
    let (_t, root) = scope();
    let mut q = started(&root);
    q.set_sort_descriptors(vec![
        MetadataSortDescriptor::ascending(MetadataAttribute::IsDirectory),
        MetadataSortDescriptor::descending(MetadataAttribute::FsName),
    ]);
    assert_eq!(names(&q), vec!["inside.txt", "charlie.txt", "bravo.txt", "alpha.txt", "folder"]);
}

#[test]
fn sort_descriptors_keep_the_items_and_only_change_their_order() {
    let (_t, root) = scope();
    let mut q = started(&root);
    let mut by_path: Vec<PathBuf> = q.results().iter().map(|item| item.path.clone()).collect();
    q.set_sort_descriptors(vec![MetadataSortDescriptor::descending(MetadataAttribute::FsSize)]);
    let mut sorted: Vec<PathBuf> = q.results().iter().map(|item| item.path.clone()).collect();
    assert_eq!(q.result_count(), 5);
    by_path.sort();
    sorted.sort();
    assert_eq!(sorted, by_path);
}

#[test]
fn clearing_the_sort_descriptors_returns_to_path_order() {
    let (_t, root) = scope();
    let mut q = started(&root);
    q.set_sort_descriptors(vec![MetadataSortDescriptor::descending(MetadataAttribute::FsName)]);
    assert_eq!(q.sort_descriptors().len(), 1);
    q.set_sort_descriptors(Vec::new());
    assert!(q.sort_descriptors().is_empty());
    assert_eq!(names(&q), vec!["alpha.txt", "bravo.txt", "charlie.txt", "folder", "inside.txt"]);
}

#[test]
fn a_descriptor_set_before_the_gather_orders_the_first_results() {
    let (_t, root) = scope();
    let mut q = MetadataQuery::new();
    q.search_scopes = vec![root.clone()];
    q.set_sort_descriptors(vec![MetadataSortDescriptor::descending(MetadataAttribute::FsName)]);
    assert!(q.start_query());
    assert_eq!(names(&q), vec!["inside.txt", "folder", "charlie.txt", "bravo.txt", "alpha.txt"]);
}

#[test]
fn a_predicate_and_sort_descriptors_work_together() {
    let (_t, root) = scope();
    let mut q = MetadataQuery::new();
    q.search_scopes = vec![root];
    q.predicate = Some(MetadataPredicate::compare(
        MetadataAttribute::FsName,
        MetadataOp::EndsWith,
        MetadataValue::Str(".txt".into()),
    ));
    q.set_sort_descriptors(vec![MetadataSortDescriptor::descending(MetadataAttribute::FsSize)]);
    assert!(q.start_query());
    assert_eq!(names(&q), vec!["bravo.txt", "charlie.txt", "alpha.txt", "inside.txt"]);
}
