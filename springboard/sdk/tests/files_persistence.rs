//! Contract tests: Foundation "Files and Data Persistence" parity. Binary property lists against hand-made
//! fixtures, keyed archives (layout and secure coding), file wrappers, bookmarks, `replace_item_at`, the directory
//! enumerator with prefetched keys, backup exclusion and the file metadata query.

use bifrost_app::binary_plist::{self, BValue};
use bifrost_app::bookmark::{
    bookmark_data, resolve_bookmark, resource_values_from_bookmark, BookmarkCreationOptions, BookmarkError,
    BookmarkResolutionOptions,
};
use bifrost_app::coding::{Date, PropertyListEncoder};
use bifrost_app::file_manager::{DirectoryEnumerationOptions, NativeFileManager, ReplaceOptions, ResourceKey};
use bifrost_app::file_metadata::{
    backup_items, is_excluded_from_backup, remove_xattr, set_excluded_from_backup, set_xattr, xattr, BACKUP_EXCLUDED_XATTR,
};
use bifrost_app::file_wrapper::{FileWrapper, WritingOptions};
use bifrost_app::keyed_archive::{Archive, ArchiveError, Field, KeyedArchiver, KeyedUnarchiver, Uid};
use bifrost_app::metadata_query::{
    MetadataAttribute, MetadataNotification, MetadataOp, MetadataPredicate, MetadataQuery, MetadataValue,
};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---- binary property lists ----

fn trailer(offset_size: u8, ref_size: u8, count: u64, top: u64, table: u64) -> Vec<u8> {
    let mut t = vec![0u8; 6];
    t.push(offset_size);
    t.push(ref_size);
    t.extend(count.to_be_bytes());
    t.extend(top.to_be_bytes());
    t.extend(table.to_be_bytes());
    t
}

/// A one-object file: `object` bytes at offset 8.
fn single(object: &[u8]) -> Vec<u8> {
    let mut b = b"bplist00".to_vec();
    b.extend_from_slice(object);
    b.push(8); // the offset table: object 0 is at offset 8
    b.extend(trailer(1, 1, 1, 0, 8 + object.len() as u64));
    b
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn bplist_reads_and_writes_a_hand_made_dictionary() {
    let mut fixture = b"bplist00".to_vec();
    fixture.extend_from_slice(&[0xD1, 0x01, 0x02, 0x51, b'a', 0x09]); // {"a": true}
    fixture.extend_from_slice(&[0x08, 0x0B, 0x0D]); // offsets of the three objects
    fixture.extend(trailer(1, 1, 3, 0, 14));
    let value = BValue::Dict(vec![("a".to_string(), BValue::Bool(true))]);
    assert_eq!(binary_plist::read(&fixture).unwrap(), value);
    assert_eq!(binary_plist::write(&value), fixture, "the writer lays a small dictionary out like the reference writer");
}

#[test]
fn bplist_integers_use_the_smallest_width_and_negatives_take_eight_bytes() {
    assert_eq!(binary_plist::write(&BValue::Int(5)), single(&[0x10, 0x05]));
    assert_eq!(binary_plist::write(&BValue::Int(300)), single(&[0x11, 0x01, 0x2C]));
    assert_eq!(binary_plist::write(&BValue::Int(70_000)), single(&[0x12, 0x00, 0x01, 0x11, 0x70]));
    assert_eq!(binary_plist::write(&BValue::Int(5_000_000_000)), single(&[0x13, 0, 0, 0, 1, 0x2A, 0x05, 0xF2, 0x00]));
    assert_eq!(binary_plist::write(&BValue::Int(-2)), single(&[0x13, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE]));
    let all = BValue::Array(
        [0, 255, 256, 65_535, 65_536, 4_294_967_295, 4_294_967_296, -1, i64::MIN, i64::MAX].iter().map(|i| BValue::Int(*i)).collect(),
    );
    assert_eq!(binary_plist::read(&binary_plist::write(&all)).unwrap(), all);
}

#[test]
fn bplist_reals_dates_data_and_uids() {
    assert_eq!(binary_plist::read(&single(&[0x23, 0x3F, 0xF8, 0, 0, 0, 0, 0, 0])).unwrap(), BValue::Real(1.5));
    assert_eq!(binary_plist::read(&single(&[0x22, 0x3F, 0xC0, 0, 0])).unwrap(), BValue::Real(1.5), "a 4-byte real reads too");
    assert_eq!(binary_plist::write(&BValue::Real(1.5)), single(&[0x23, 0x3F, 0xF8, 0, 0, 0, 0, 0, 0]));
    assert_eq!(binary_plist::write(&BValue::Date(0.0)), single(&[0x33, 0, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(BValue::date_from_unix(978_307_200.0), BValue::Date(0.0), "2001-01-01 is the reference date");
    assert_eq!(BValue::Date(86_400.0).unix_seconds(), Some(978_307_200.0 + 86_400.0));
    assert_eq!(binary_plist::write(&BValue::Data(vec![1, 2, 3])), single(&[0x43, 1, 2, 3]));
    assert!(contains(&binary_plist::write(&BValue::Data(vec![7; 20])), &[0x4F, 0x10, 0x14]), "a long length follows 0xF");
    assert_eq!(binary_plist::write(&BValue::Uid(5)), single(&[0x80, 0x05]));
    assert_eq!(binary_plist::write(&BValue::Uid(300)), single(&[0x81, 0x01, 0x2C]));
    assert_eq!(binary_plist::read(&single(&[0x81, 0x01, 0x2C])).unwrap(), BValue::Uid(300));
    assert_eq!(binary_plist::read(&single(&[0x00])).unwrap(), BValue::Null);
    assert_eq!(binary_plist::read(&single(&[0x08])).unwrap(), BValue::Bool(false));
}

#[test]
fn bplist_strings_are_ascii_or_utf16() {
    assert_eq!(binary_plist::write(&BValue::Str("hello".into())), single(&[0x55, b'h', b'e', b'l', b'l', b'o']));
    assert_eq!(binary_plist::write(&BValue::Str("\u{e9}".into())), single(&[0x61, 0x00, 0xE9]));
    assert_eq!(binary_plist::read(&single(&[0x61, 0x00, 0xE9])).unwrap(), BValue::Str("\u{e9}".into()));
    let long = "x".repeat(20);
    assert!(contains(&binary_plist::write(&BValue::Str(long.clone())), &[0x5F, 0x10, 0x14]));
    for s in [long.as_str(), "h\u{e9}llo \u{1F389}", "", "\u{65e5}\u{672c}\u{8a9e}"] {
        let v = BValue::Str(s.to_string());
        assert_eq!(binary_plist::read(&binary_plist::write(&v)).unwrap(), v, "{s:?}");
    }
}

#[test]
fn bplist_round_trips_nested_values_and_wide_references() {
    let nested = BValue::Dict(vec![
        ("list".to_string(), BValue::Array(vec![BValue::Int(1), BValue::Null, BValue::Str("two".into())])),
        ("inner".to_string(), BValue::Dict(vec![("k".to_string(), BValue::Real(-0.25))])),
        ("when".to_string(), BValue::Date(12.5)),
        ("blob".to_string(), BValue::Data(vec![0, 255])),
    ]);
    assert_eq!(binary_plist::read(&binary_plist::write(&nested)).unwrap(), nested);
    let big = BValue::Array((0..300).map(BValue::Int).collect());
    let bytes = binary_plist::write(&big);
    assert_eq!(bytes[bytes.len() - 25], 2, "300 objects need two-byte references");
    assert_eq!(binary_plist::read(&bytes).unwrap(), big);
}

#[test]
fn bplist_refuses_damaged_input() {
    assert!(binary_plist::read(b"bplist00").is_err(), "no trailer");
    assert!(binary_plist::read(&[0u8; 64]).is_err(), "no magic");
    let mut cycle = b"bplist00".to_vec();
    cycle.extend_from_slice(&[0xA1, 0x00, 0x08]); // an array that holds itself
    cycle.extend(trailer(1, 1, 1, 0, 10));
    assert!(binary_plist::read(&cycle).is_err(), "a cycle is refused, not followed forever");
    let mut dangling = b"bplist00".to_vec();
    dangling.extend_from_slice(&[0xA1, 0x05, 0x08]);
    dangling.extend(trailer(1, 1, 1, 0, 10));
    assert!(binary_plist::read(&dangling).is_err(), "a reference outside the table");
    let mut cut = binary_plist::write(&BValue::Str("a string long enough".into()));
    let n = cut.len();
    cut.drain(12..n - 32); // lose the middle of the file
    assert!(binary_plist::read(&cut).is_err());
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    name: String,
    count: u32,
    when: Date,
}
bifrost_app::codable!(Entry { name: String, count: u32, when: Date });

#[test]
fn property_list_coders_round_trip_through_the_binary_format() {
    let entry = Entry { name: "caf\u{e9}".into(), count: 9, when: Date(1_700_000_000.25) };
    let bytes = PropertyListEncoder::new().encode_binary(&entry);
    assert!(bytes.starts_with(b"bplist00"));
    let back: Entry = bifrost_app::coding::PropertyListDecoder::new().decode_binary(&bytes).unwrap();
    assert_eq!(back, entry);
    let back2: Entry = bifrost_app::coding::PropertyListDecoder::new().decode_bytes(&bytes).unwrap();
    assert_eq!(back2.name, entry.name);
}

// ---- keyed archives ----

fn sample_archive() -> (Archive, Uid) {
    let mut a = KeyedArchiver::new();
    let tag = a.encode_object(&["Tag", "Base"], vec![("name".to_string(), Field::Str("tax".to_string()))]).unwrap();
    let doc = a
        .encode_object(
            &["Document", "Base"],
            vec![
                ("title".to_string(), Field::Str("Invoice".to_string())),
                ("tag".to_string(), Field::Ref(tag)),
                ("count".to_string(), Field::Int(3)),
                ("ratio".to_string(), Field::Num(0.5)),
                ("blob".to_string(), Field::Bytes(vec![1, 2, 3])),
                ("flags".to_string(), Field::List(vec![Field::Bool(true), Field::Ref(tag)])),
                ("none".to_string(), Field::Null),
            ],
        )
        .unwrap();
    (a.finish(doc).unwrap(), doc)
}

#[test]
fn keyed_archive_binary_layout_has_top_objects_and_class_descriptions() {
    let (archive, doc) = sample_archive();
    let bytes = archive.to_binary();
    let root = binary_plist::read(&bytes).unwrap();
    assert_eq!(root.get("$archiver"), Some(&BValue::Str("BifrostKeyedArchiver".into())));
    assert_eq!(root.get("$version"), Some(&BValue::Int(100_000)));
    assert_eq!(root.get("$top").and_then(|t| t.get("root")), Some(&BValue::Uid(doc.0)));
    let Some(BValue::Array(objects)) = root.get("$objects") else { panic!("no $objects") };
    assert_eq!(objects[0], BValue::Str("$null".into()));
    // two objects, then one class description per distinct chain
    assert_eq!(objects.len(), 1 + 2 + 2);
    let Some(BValue::Uid(desc)) = objects[doc.0 as usize].get("$class") else { panic!("no $class") };
    assert_eq!(objects[*desc as usize].get("$classname"), Some(&BValue::Str("Document".into())));
    assert_eq!(
        objects[*desc as usize].get("$classes"),
        Some(&BValue::Array(vec![BValue::Str("Document".into()), BValue::Str("Base".into())]))
    );
    assert_eq!(Archive::from_binary(&bytes).unwrap(), archive, "the archive survives the binary form");
}

#[test]
fn keyed_archive_reads_values_stored_in_their_own_slots() {
    let doc = BValue::Dict(vec![
        ("$archiver".to_string(), BValue::Str("BifrostKeyedArchiver".into())),
        ("$version".to_string(), BValue::Int(100_000)),
        ("$top".to_string(), BValue::Dict(vec![("root".to_string(), BValue::Uid(1))])),
        (
            "$objects".to_string(),
            BValue::Array(vec![
                BValue::Str("$null".into()),
                BValue::Dict(vec![
                    ("$class".to_string(), BValue::Uid(3)),
                    ("title".to_string(), BValue::Uid(2)),
                    ("missing".to_string(), BValue::Uid(0)),
                ]),
                BValue::Str("Invoice".into()),
                BValue::Dict(vec![
                    ("$classname".to_string(), BValue::Str("Doc".into())),
                    ("$classes".to_string(), BValue::Array(vec![BValue::Str("Doc".into())])),
                ]),
            ]),
        ),
    ]);
    let archive = Archive::from_binary(&binary_plist::write(&doc)).unwrap();
    let top = archive.object(archive.top()).unwrap();
    assert_eq!(top.class(), "Doc");
    assert_eq!(top.field("title"), Some(&Field::Str("Invoice".into())));
    assert_eq!(top.field("missing"), Some(&Field::Ref(Uid::NIL)));
    assert_eq!(archive.object_count(), 1, "plain value slots and class descriptions are not objects");
}

#[test]
fn keyed_archive_binary_refuses_a_foreign_or_broken_archive() {
    let wrong = binary_plist::write(&BValue::Dict(vec![("$archiver".to_string(), BValue::Str("Other".into()))]));
    assert!(matches!(Archive::from_binary(&wrong), Err(ArchiveError::Malformed(_))));
    let version = binary_plist::write(&BValue::Dict(vec![
        ("$archiver".to_string(), BValue::Str("BifrostKeyedArchiver".into())),
        ("$version".to_string(), BValue::Int(7)),
    ]));
    assert_eq!(Archive::from_binary(&version).unwrap_err(), ArchiveError::UnsupportedVersion(7));
    assert!(Archive::from_binary(b"not a plist at all, not even close to one").is_err());
}

#[test]
fn secure_coding_checks_the_class_allow_list_per_field() {
    let (archive, _) = sample_archive();
    let secure = KeyedUnarchiver::new(&["Document", "Tag"]);
    assert!(secure.requires_secure_coding());
    let doc = secure.decode_top(&archive).unwrap().unwrap();
    let tag = secure.decode_object_of(&archive, doc, &["Tag"], "tag").unwrap().unwrap();
    assert_eq!(tag.field("name"), Some(&Field::Str("tax".into())));
    assert!(secure.decode_object_of(&archive, doc, &["Base"], "tag").unwrap().is_some(), "a superclass is accepted");
    assert_eq!(
        secure.decode_object_of(&archive, doc, &["Other"], "tag").unwrap_err(),
        ArchiveError::ClassNotAllowed("Tag".into())
    );
    assert!(secure.decode_object_of(&archive, doc, &["Tag"], "absent").unwrap().is_none());
    assert!(secure.decode_object_of(&archive, doc, &["Tag"], "none").unwrap().is_none());
    assert_eq!(
        secure.decode_object_of(&archive, doc, &["Tag"], "title").unwrap_err(),
        ArchiveError::TypeMismatch("an object reference")
    );
    // the top object's class has to be on the unarchiver's own list too
    assert_eq!(
        KeyedUnarchiver::new(&["Tag"]).decode_top(&archive).unwrap_err(),
        ArchiveError::ClassNotAllowed("Document".into())
    );
    let lax = KeyedUnarchiver::new(&[]).without_secure_coding();
    assert!(!lax.requires_secure_coding());
    let doc = lax.decode_top(&archive).unwrap().unwrap();
    assert!(lax.decode_object_of(&archive, doc, &["Other"], "tag").unwrap().is_some());
}

// ---- file wrappers ----

fn package() -> FileWrapper {
    let mut sub = FileWrapper::directory_with_file_wrappers(Vec::new());
    sub.add_regular_file(vec![9, 9, 9], "b.bin").unwrap();
    let mut root = FileWrapper::directory_with_file_wrappers(Vec::new());
    root.add_regular_file(b"alpha".to_vec(), "a.txt").unwrap();
    let mut data = FileWrapper::directory_with_file_wrappers(Vec::new());
    data.preferred_filename = Some("data".into());
    data.add_file_wrapper(sub).unwrap_err(); // a child needs a preferred file name
    let mut sub2 = FileWrapper::directory_with_file_wrappers(Vec::new());
    sub2.add_regular_file(vec![9, 9, 9], "b.bin").unwrap();
    sub2.preferred_filename = Some("sub".into());
    data.add_file_wrapper(sub2).unwrap();
    root.add_file_wrapper(data).unwrap();
    root.add_file_wrapper(FileWrapper::symbolic_link_with_destination(PathBuf::from("a.txt"))).unwrap_err();
    let mut link = FileWrapper::symbolic_link_with_destination(PathBuf::from("a.txt"));
    link.preferred_filename = Some("alias".into());
    root.add_file_wrapper(link).unwrap();
    root
}

#[test]
fn file_wrapper_writes_and_reads_a_package() {
    let t = tempfile::tempdir().unwrap();
    let pkg = t.path().join("doc.pkg");
    let mut w = package();
    w.write_to(&pkg, WritingOptions::default(), None).unwrap();
    assert_eq!(fs::read(pkg.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(pkg.join("data/sub/b.bin")).unwrap(), vec![9, 9, 9]);
    assert_eq!(fs::read_link(pkg.join("alias")).unwrap(), PathBuf::from("a.txt"));

    let read = FileWrapper::read_from(&pkg).unwrap();
    assert!(read.is_directory());
    assert_eq!(read.filename.as_deref(), Some("doc.pkg"));
    assert_eq!(read.file_wrapper("a.txt").unwrap().regular_file_contents(), Some(&b"alpha"[..]));
    assert!(read.file_wrapper("alias").unwrap().is_symbolic_link());
    assert_eq!(read.file_wrapper("alias").unwrap().symbolic_link_destination(), Some(Path::new("a.txt")));
    let sub = read.file_wrapper("data").unwrap().file_wrapper("sub").unwrap();
    assert_eq!(sub.file_wrapper("b.bin").unwrap().regular_file_contents(), Some(&[9u8, 9, 9][..]));
    assert!(read.file_wrapper("missing").is_none());
}

#[test]
fn file_wrapper_matches_contents_notices_changes() {
    let t = tempfile::tempdir().unwrap();
    let pkg = t.path().join("doc.pkg");
    let mut w = package();
    w.write_to(&pkg, WritingOptions::default(), None).unwrap();
    let read = FileWrapper::read_from(&pkg).unwrap();
    assert!(read.matches_contents_of(&pkg));
    assert!(w.matches_contents_of(&pkg), "the tree that was written matches what it wrote");

    fs::write(pkg.join("a.txt"), b"alphA").unwrap(); // same size, other bytes
    assert!(!read.matches_contents_of(&pkg), "changed bytes");
    fs::write(pkg.join("a.txt"), b"alpha").unwrap();
    assert!(read.matches_contents_of(&pkg));
    fs::write(pkg.join("extra"), b"").unwrap();
    assert!(!read.matches_contents_of(&pkg), "an added file");
    fs::remove_file(pkg.join("extra")).unwrap();
    fs::remove_file(pkg.join("alias")).unwrap();
    assert!(!read.matches_contents_of(&pkg), "a removed link");
    assert!(!read.matches_contents_of(&t.path().join("nowhere")));
}

#[test]
fn file_wrapper_names_children_uniquely_and_removes_them() {
    let mut dir = FileWrapper::directory_with_file_wrappers(Vec::new());
    assert_eq!(dir.add_regular_file(vec![1], "note.txt").unwrap(), "note.txt");
    assert_eq!(dir.add_regular_file(vec![2], "note.txt").unwrap(), "note 2.txt");
    assert_eq!(dir.add_regular_file(vec![3], "note.txt").unwrap(), "note 3.txt");
    assert_eq!(dir.add_regular_file(vec![4], "plain").unwrap(), "plain");
    assert_eq!(dir.add_regular_file(vec![5], "plain").unwrap(), "plain 2");
    assert_eq!(dir.file_wrapper("note 2.txt").unwrap().filename.as_deref(), Some("note 2.txt"));
    assert!(dir.remove_file_wrapper("note 2.txt"));
    assert!(!dir.remove_file_wrapper("note 2.txt"));
    assert_eq!(dir.file_wrappers().unwrap().len(), 4);
    let mut file = FileWrapper::regular_file_with_contents(vec![]);
    assert!(file.add_regular_file(vec![], "x").is_err(), "only a directory takes children");
    assert!(file.file_wrappers().is_none());
}

#[test]
fn atomic_write_keeps_original_contents_and_leaves_no_temporary() {
    let t = tempfile::tempdir().unwrap();
    let pkg = t.path().join("doc.pkg");
    let mut first = FileWrapper::directory_with_file_wrappers(Vec::new());
    first.add_regular_file(vec![1u8; 4096], "big.bin").unwrap();
    first.add_regular_file(b"v1".to_vec(), "small.txt").unwrap();
    first.write_to(&pkg, WritingOptions::default(), None).unwrap();
    let big_before = fs::metadata(pkg.join("big.bin")).unwrap().ino();

    let mut doc = FileWrapper::read_from(&pkg).unwrap();
    doc.add_regular_file(b"new".to_vec(), "added.txt").unwrap();
    doc.remove_file_wrapper("small.txt");
    let options = WritingOptions { atomic: true, with_name_updating: true };
    doc.write_to(&pkg, options, Some(&pkg)).unwrap();

    assert_eq!(fs::metadata(pkg.join("big.bin")).unwrap().ino(), big_before, "the unchanged member was linked, not rewritten");
    assert_eq!(fs::read(pkg.join("added.txt")).unwrap(), b"new");
    assert!(!pkg.join("small.txt").exists());
    let names: Vec<_> = fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("doc.pkg")], "no staging item is left behind");
    assert_eq!(doc.filename.as_deref(), Some("doc.pkg"));
}

#[test]
fn a_failed_atomic_write_leaves_the_old_package_whole() {
    let t = tempfile::tempdir().unwrap();
    let pkg = t.path().join("doc.pkg");
    let mut good = FileWrapper::directory_with_file_wrappers(Vec::new());
    good.add_regular_file(b"keep".to_vec(), "keep.txt").unwrap();
    good.write_to(&pkg, WritingOptions::default(), None).unwrap();

    // a child whose name points into a directory that does not exist cannot be written
    let broken = FileWrapper::regular_file_with_contents(vec![1]);
    let mut bad = FileWrapper::directory_with_file_wrappers(vec![("nodir/file".to_string(), broken)]);
    let options = WritingOptions { atomic: true, with_name_updating: false };
    assert!(bad.write_to(&pkg, options, None).is_err());
    assert_eq!(fs::read(pkg.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1, "the staging item was removed");
}

// ---- bookmarks ----

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_bookmark_resolves_unchanged_after_a_rename_and_after_a_move() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("a/report.txt");
    write_file(&file, "q3");
    let data = bookmark_data(&file, BookmarkCreationOptions::default(), &[], None).unwrap();
    let fresh = resolve_bookmark(&data, BookmarkResolutionOptions::default(), None).unwrap();
    assert_eq!((fresh.path.as_path(), fresh.is_stale), (file.as_path(), false));

    let renamed = t.path().join("a/final.txt");
    fs::rename(&file, &renamed).unwrap();
    let r = resolve_bookmark(&data, BookmarkResolutionOptions::default(), None).unwrap();
    assert_eq!(r.path, renamed, "found by inode in the directory it was in");
    assert!(r.is_stale);

    let moved = t.path().join("b/c/final.txt");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&renamed, &moved).unwrap();
    let r = resolve_bookmark(&data, BookmarkResolutionOptions::default(), None).unwrap();
    assert_eq!(r.path, moved, "found by inode below an ancestor");
    assert!(r.is_stale);

    let again = bookmark_data(&r.path, BookmarkCreationOptions::default(), &[], None).unwrap();
    assert!(!resolve_bookmark(&again, BookmarkResolutionOptions::default(), None).unwrap().is_stale, "a new bookmark is fresh");
}

#[test]
fn a_bookmark_fails_when_the_item_is_gone_and_refuses_other_data() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("gone.txt");
    write_file(&file, "x");
    let data = bookmark_data(&file, BookmarkCreationOptions::default(), &[], None).unwrap();
    fs::remove_file(&file).unwrap();
    assert!(matches!(
        resolve_bookmark(&data, BookmarkResolutionOptions::default(), None),
        Err(BookmarkError::NotFound(p)) if p == file
    ));
    assert!(matches!(resolve_bookmark(b"junk", BookmarkResolutionOptions::default(), None), Err(BookmarkError::Malformed(_))));
    let other = binary_plist::write(&BValue::Dict(vec![("format".to_string(), BValue::Str("other".into()))]));
    assert!(matches!(resolve_bookmark(&other, BookmarkResolutionOptions::default(), None), Err(BookmarkError::Malformed(_))));
    assert!(bookmark_data(&t.path().join("never"), BookmarkCreationOptions::default(), &[], None).is_err());
}

#[test]
fn a_relative_bookmark_follows_its_base_directory() {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().join("project");
    let file = base.join("src/lib.txt");
    write_file(&file, "code");
    let data = bookmark_data(&file, BookmarkCreationOptions::default(), &[], Some(&base)).unwrap();
    assert!(matches!(
        resolve_bookmark(&data, BookmarkResolutionOptions::default(), None),
        Err(BookmarkError::MissingBase)
    ));
    let moved_base = t.path().join("renamed");
    fs::rename(&base, &moved_base).unwrap();
    let r = resolve_bookmark(&data, BookmarkResolutionOptions::default(), Some(&moved_base)).unwrap();
    assert_eq!(r.path, moved_base.join("src/lib.txt"));
    assert!(!r.is_stale, "the whole folder moved and the relative path still names the same item");
}

#[test]
fn a_bookmark_caches_resource_values_for_use_without_the_file() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("sized.txt");
    write_file(&file, "12345");
    let data = bookmark_data(&file, BookmarkCreationOptions::default(), &[ResourceKey::IsHidden], None).unwrap();
    fs::remove_file(&file).unwrap();
    let v = resource_values_from_bookmark(&data, &[ResourceKey::FileSize, ResourceKey::Name, ResourceKey::IsHidden, ResourceKey::IsDirectory])
        .unwrap();
    assert_eq!(v.file_size, Some(5));
    assert_eq!(v.name.as_deref(), Some("sized.txt"));
    assert_eq!(v.is_hidden, Some(false));
    assert_eq!(v.is_directory, Some(false));
    assert_eq!(v.is_symbolic_link, None, "a key that was not asked for stays empty");

    write_file(&file, "12345");
    let minimal = BookmarkCreationOptions { minimal_bookmark: true, ..Default::default() };
    let data = bookmark_data(&file, minimal, &[], None).unwrap();
    assert_eq!(resource_values_from_bookmark(&data, &[ResourceKey::FileSize]).unwrap().file_size, None);
}

#[test]
fn security_scoped_bookmarks_gate_access_until_started() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("secret.txt");
    write_file(&file, "data");
    let options = BookmarkCreationOptions { with_security_scope: true, security_scope_allows_only_read_access: true, ..Default::default() };
    let data = bookmark_data(&file, options, &[], None).unwrap();
    let scoped = BookmarkResolutionOptions { with_security_scope: true, ..Default::default() };

    let mut r = resolve_bookmark(&data, scoped, None).unwrap();
    assert_eq!(r.read().unwrap_err().kind(), std::io::ErrorKind::PermissionDenied, "not started yet");
    assert!(r.scope.as_mut().unwrap().start_accessing());
    assert_eq!(r.read().unwrap(), b"data");
    assert_eq!(r.write(b"x").unwrap_err().kind(), std::io::ErrorKind::PermissionDenied, "read-only scope");
    r.scope.as_mut().unwrap().start_accessing();
    r.scope.as_mut().unwrap().stop_accessing();
    assert!(r.scope.as_ref().unwrap().is_accessing(), "starts nest");
    r.scope.as_mut().unwrap().stop_accessing();
    assert!(r.read().is_err());

    let plain = resolve_bookmark(&data, BookmarkResolutionOptions::default(), None).unwrap();
    assert!(plain.scope.is_none(), "no scope unless it is asked for at resolution");
    assert_eq!(plain.read().unwrap(), b"data");

    let rw = bookmark_data(&file, BookmarkCreationOptions { with_security_scope: true, ..Default::default() }, &[], None).unwrap();
    let mut r = resolve_bookmark(&rw, scoped, None).unwrap();
    r.scope.as_mut().unwrap().start_accessing();
    r.write(b"changed").unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"changed");
}

// ---- replaceItemAt ----

fn xattrs_supported(dir: &Path) -> bool {
    let probe = dir.join("xattr-probe");
    fs::write(&probe, b"").unwrap();
    let on_file = set_xattr(&probe, "user.probe", b"1").is_ok();
    let on_dir = set_xattr(dir, "user.probe", b"1").is_ok();
    let _ = remove_xattr(dir, "user.probe");
    fs::remove_file(&probe).unwrap();
    on_file && on_dir
}

#[test]
fn replace_item_at_swaps_content_and_keeps_the_originals_metadata() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("doc.txt");
    let staged = t.path().join("staged.txt");
    write_file(&original, "old");
    write_file(&staged, "new");
    fs::set_permissions(&original, fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    let has_xattrs = xattrs_supported(t.path());
    if has_xattrs {
        set_xattr(&original, "user.keep", b"tag").unwrap();
    }
    let fm = NativeFileManager;
    let result = fm.replace_item_at(&original, &staged, None, ReplaceOptions::default()).unwrap();
    assert_eq!(result, original);
    assert_eq!(fs::read(&original).unwrap(), b"new");
    assert!(!staged.exists(), "the replacement was moved in");
    assert_eq!(fs::metadata(&original).unwrap().permissions().mode() & 0o777, 0o640);
    if has_xattrs {
        assert_eq!(xattr(&original, "user.keep").unwrap(), Some(b"tag".to_vec()));
    }
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}

#[test]
fn replace_item_at_with_new_metadata_only_keeps_the_replacements_metadata() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("doc.txt");
    let staged = t.path().join("staged.txt");
    write_file(&original, "old");
    write_file(&staged, "new");
    fs::set_permissions(&original, fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    let options = ReplaceOptions { using_new_metadata_only: true, ..Default::default() };
    NativeFileManager.replace_item_at(&original, &staged, None, options).unwrap();
    assert_eq!(fs::metadata(&original).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn replace_item_at_backup_name_is_kept_or_deleted() {
    let t = tempfile::tempdir().unwrap();
    let fm = NativeFileManager;
    let original = t.path().join("doc.txt");
    let backup = t.path().join("doc.txt~");

    write_file(&original, "v1");
    write_file(&t.path().join("s1"), "v2");
    fm.replace_item_at(&original, &t.path().join("s1"), Some("doc.txt~"), ReplaceOptions::default()).unwrap();
    assert!(!backup.exists(), "the backup is deleted after a successful replace");
    assert_eq!(fs::read(&original).unwrap(), b"v2");

    write_file(&t.path().join("s2"), "v3");
    let keep = ReplaceOptions { without_deleting_backup_item: true, ..Default::default() };
    fm.replace_item_at(&original, &t.path().join("s2"), Some("doc.txt~"), keep).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), b"v2", "the backup holds the replaced version");
    assert_eq!(fs::read(&original).unwrap(), b"v3");

    write_file(&t.path().join("s3"), "v4");
    fm.replace_item_at(&original, &t.path().join("s3"), Some("doc.txt~"), keep).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), b"v3", "an older backup is replaced");

    write_file(&t.path().join("s4"), "v5");
    let bad = fm.replace_item_at(&original, &t.path().join("s4"), Some("../escape"), keep);
    assert_eq!(bad.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    assert!(fm.replace_item_at(&t.path().join("nope"), &original, None, ReplaceOptions::default()).is_err());
}

#[test]
fn replace_item_at_replaces_a_directory_and_a_missing_replacement_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let fm = NativeFileManager;
    let original = t.path().join("pkg");
    write_file(&original.join("old.txt"), "old");
    write_file(&t.path().join("fresh/new.txt"), "new");
    fm.replace_item_at(&original, &t.path().join("fresh"), None, ReplaceOptions::default()).unwrap();
    assert!(original.join("new.txt").exists());
    assert!(!original.join("old.txt").exists());
    assert!(!t.path().join("fresh").exists());
    assert!(fm.replace_item_at(&original, &t.path().join("missing"), None, ReplaceOptions::default()).is_err());
    assert!(original.join("new.txt").exists(), "a failed replace leaves the original");
}

// ---- directory enumerator and resource keys ----

fn tree(root: &Path) {
    write_file(&root.join("a.txt"), "aa");
    write_file(&root.join("dir/inner.txt"), "inner!");
    write_file(&root.join("dir/deep/leaf.txt"), "l");
    write_file(&root.join(".hidden"), "h");
    write_file(&root.join("z.txt"), "");
}

#[test]
fn the_enumerator_walks_depth_first_with_prefetched_resource_values() {
    let t = tempfile::tempdir().unwrap();
    tree(t.path());
    let keys = [ResourceKey::Name, ResourceKey::IsDirectory, ResourceKey::FileSize, ResourceKey::IsHidden];
    let items: Vec<_> = NativeFileManager.enumerator(t.path(), &keys, DirectoryEnumerationOptions::default()).unwrap().collect();
    let names: Vec<_> = items.iter().map(|i| i.path.strip_prefix(t.path()).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec![".hidden", "a.txt", "dir", "dir/deep", "dir/deep/leaf.txt", "dir/inner.txt", "z.txt"]);
    let inner = items.iter().find(|i| i.path.ends_with("inner.txt")).unwrap();
    assert_eq!(inner.values.name.as_deref(), Some("inner.txt"));
    assert_eq!(inner.values.file_size, Some(6));
    assert_eq!(inner.values.is_directory, Some(false));
    assert_eq!(items[0].values.is_hidden, Some(true));
    let dir = items.iter().find(|i| i.path.ends_with("dir")).unwrap();
    assert_eq!(dir.values.is_directory, Some(true));
    assert_eq!(inner.values.content_modification_date, None, "only the keys asked for are filled");
}

#[test]
fn the_enumerator_honours_its_options_and_skip_descendants() {
    let t = tempfile::tempdir().unwrap();
    tree(t.path());
    let fm = NativeFileManager;
    let shallow = DirectoryEnumerationOptions { skips_subdirectory_descendants: true, skips_hidden_files: true };
    let names: Vec<_> = fm
        .enumerator(t.path(), &[], shallow)
        .unwrap()
        .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["a.txt", "dir", "z.txt"]);

    let mut e = fm.enumerator(t.path(), &[], DirectoryEnumerationOptions::default()).unwrap();
    let mut seen = Vec::new();
    while let Some(item) = e.next() {
        if item.path.ends_with("dir") {
            e.skip_descendants();
        }
        seen.push(item.path.file_name().unwrap().to_string_lossy().into_owned());
    }
    assert_eq!(seen, vec![".hidden", "a.txt", "dir", "z.txt"]);
    assert!(fm.enumerator(&t.path().join("a.txt"), &[], DirectoryEnumerationOptions::default()).is_err(), "a file is not a directory");
}

#[test]
fn resource_values_report_identity_dates_and_volume_capacity() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("f");
    write_file(&file, "abc");
    let keys = [
        ResourceKey::FileResourceIdentifier,
        ResourceKey::ContentModificationDate,
        ResourceKey::VolumeTotalCapacity,
        ResourceKey::VolumeAvailableCapacity,
        ResourceKey::VolumeAvailableCapacityForImportantUsage,
        ResourceKey::IsRegularFile,
        ResourceKey::IsSymbolicLink,
    ];
    let v = NativeFileManager.resource_values(&file, &keys).unwrap();
    let meta = fs::metadata(&file).unwrap();
    assert_eq!(v.file_resource_identifier, Some((meta.dev(), meta.ino())));
    assert!(v.content_modification_date.unwrap() > 1.0e9);
    let (total, free, important) =
        (v.volume_total_capacity.unwrap(), v.volume_available_capacity.unwrap(), v.volume_available_capacity_for_important_usage.unwrap());
    assert!(total > 0 && free <= total);
    assert!(important >= free, "important usage counts at least the free space");
    assert_eq!((v.is_regular_file, v.is_symbolic_link), (Some(true), Some(false)));
    assert!(NativeFileManager.resource_values(&t.path().join("missing"), &keys).is_err());
}

// ---- backup exclusion ----

#[test]
fn backup_exclusion_is_an_extended_attribute_that_backups_honour() {
    let t = tempfile::tempdir().unwrap();
    if !xattrs_supported(t.path()) {
        eprintln!("skipped: this file system has no user extended attributes");
        return;
    }
    tree(t.path());
    let skipped = t.path().join("dir");
    let file = t.path().join("a.txt");
    assert!(!is_excluded_from_backup(&file));
    set_excluded_from_backup(&skipped, true).unwrap();
    assert_eq!(xattr(&skipped, BACKUP_EXCLUDED_XATTR).unwrap(), Some(b"1".to_vec()), "stored as an attribute");
    assert!(is_excluded_from_backup(&skipped));
    let v = NativeFileManager.resource_values(&skipped, &[ResourceKey::IsExcludedFromBackup]).unwrap();
    assert_eq!(v.is_excluded_from_backup, Some(true));

    let covered: Vec<String> =
        backup_items(t.path()).unwrap().iter().map(|p| p.strip_prefix(t.path()).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(covered, vec![".hidden", "a.txt", "z.txt"], "an excluded directory is left out with everything in it");

    set_excluded_from_backup(&skipped, false).unwrap();
    assert!(!is_excluded_from_backup(&skipped));
    set_excluded_from_backup(&skipped, false).unwrap(); // clearing twice is fine
    assert_eq!(backup_items(t.path()).unwrap().len(), 7, "everything is covered again");
    set_excluded_from_backup(t.path(), true).unwrap();
    assert!(backup_items(t.path()).unwrap().is_empty(), "an excluded root has nothing to back up");
}

// ---- metadata query ----

fn drain_until(query: &mut MetadataQuery, want: impl Fn(&MetadataNotification) -> bool) -> Vec<MetadataNotification> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while Instant::now() < deadline {
        let batch = query.poll(Instant::now());
        let hit = batch.iter().any(&want);
        got.extend(batch);
        if hit {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    got
}

fn name_like(pattern: &str) -> MetadataPredicate {
    MetadataPredicate::compare(MetadataAttribute::FsName, MetadataOp::Like, MetadataValue::Str(pattern.into()))
}

#[test]
fn a_metadata_query_gathers_matches_and_then_reports_file_changes() {
    let t = tempfile::tempdir().unwrap();
    write_file(&t.path().join("a.txt"), "one");
    write_file(&t.path().join("sub/b.txt"), "two");
    write_file(&t.path().join("c.md"), "three");

    let mut q = MetadataQuery::new();
    assert!(!q.start_query(), "no search scope");
    q.search_scopes = vec![t.path().to_path_buf()];
    q.predicate = Some(name_like("*.txt"));
    assert!(q.is_stopped());
    assert!(q.start_query());
    assert!(!q.start_query(), "already running");
    assert!(q.is_gathering());

    let first = q.poll(Instant::now());
    assert_eq!(
        &first[..3],
        &[MetadataNotification::DidStartGathering, MetadataNotification::GatheringProgress, MetadataNotification::DidFinishGathering]
    );
    assert!(!q.is_gathering() && q.is_started());
    let paths: Vec<_> = q.results().iter().map(|i| i.path.clone()).collect();
    assert_eq!(paths, vec![t.path().join("a.txt"), t.path().join("sub/b.txt")]);
    assert_eq!(q.result_count(), 2);
    let a = q.result(0).unwrap();
    assert_eq!(a.value(MetadataAttribute::FsName), Some(&MetadataValue::Str("a.txt".into())));
    assert_eq!(a.value(MetadataAttribute::FsSize), Some(&MetadataValue::Int(3)));
    assert_eq!(a.value(MetadataAttribute::ContentType), Some(&MetadataValue::Str("public.plain-text".into())));
    assert_eq!(a.value(MetadataAttribute::IsDirectory), Some(&MetadataValue::Bool(false)));
    assert!(matches!(a.value(MetadataAttribute::FsContentChangeDate), Some(MetadataValue::Date(d)) if *d > 1.0e9));
    assert!(a.attributes().contains(&MetadataAttribute::Path));
    assert!(q.fd().is_some(), "live updates wake on a descriptor");

    // a new match, a changed match and a removed match
    write_file(&t.path().join("d.txt"), "four");
    let got = drain_until(&mut q, |n| matches!(n, MetadataNotification::DidUpdate { added, .. } if !added.is_empty()));
    let Some(MetadataNotification::DidUpdate { added, changed, removed }) =
        got.into_iter().find(|n| matches!(n, MetadataNotification::DidUpdate { .. }))
    else {
        panic!("no update after a file was created");
    };
    assert_eq!(added, vec![t.path().join("d.txt")]);
    assert!(changed.is_empty() && removed.is_empty());
    assert_eq!(q.result_count(), 3);

    fs::write(t.path().join("a.txt"), "one, longer").unwrap();
    fs::remove_file(t.path().join("sub/b.txt")).unwrap();
    write_file(&t.path().join("ignored.bin"), "no match");
    let got = drain_until(&mut q, |n| matches!(n, MetadataNotification::DidUpdate { removed, .. } if !removed.is_empty()));
    let mut all_changed = Vec::new();
    let mut all_removed = Vec::new();
    for n in &got {
        if let MetadataNotification::DidUpdate { changed, removed, .. } = n {
            all_changed.extend(changed.iter().cloned());
            all_removed.extend(removed.iter().cloned());
        }
    }
    assert!(all_changed.contains(&t.path().join("a.txt")));
    assert_eq!(all_removed, vec![t.path().join("sub/b.txt")]);
    assert!(q.results().iter().all(|i| !i.path.ends_with("ignored.bin")));

    q.stop_query();
    assert!(q.is_stopped());
    assert!(q.fd().is_none());
    assert_eq!(q.result_count(), 3, "stopping keeps the results");
}

#[test]
fn metadata_predicates_combine_comparisons() {
    let t = tempfile::tempdir().unwrap();
    write_file(&t.path().join("small.txt"), "1");
    write_file(&t.path().join("large.txt"), &"x".repeat(100));
    write_file(&t.path().join("pic.png"), "p");
    fs::create_dir(t.path().join("folder.txt")).unwrap();

    let find = |predicate: Option<MetadataPredicate>| -> Vec<String> {
        let mut q = MetadataQuery::new();
        q.search_scopes = vec![t.path().to_path_buf()];
        q.predicate = predicate;
        assert!(q.start_query());
        let names = q.results().iter().map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        q.stop_query();
        names
    };
    let size = |op, n| MetadataPredicate::compare(MetadataAttribute::FsSize, op, MetadataValue::Int(n));
    let not_dir = || {
        MetadataPredicate::Not(Box::new(MetadataPredicate::compare(
            MetadataAttribute::IsDirectory,
            MetadataOp::Equal,
            MetadataValue::Bool(true),
        )))
    };
    assert_eq!(find(None).len(), 4, "no predicate matches everything");
    assert_eq!(find(Some(MetadataPredicate::And(vec![size(MetadataOp::Greater, 50), not_dir()]))), vec!["large.txt"]);
    assert_eq!(
        find(Some(MetadataPredicate::And(vec![name_like("*.txt"), size(MetadataOp::LessOrEqual, 50), not_dir()]))),
        vec!["small.txt"]
    );
    assert_eq!(find(Some(MetadataPredicate::Or(vec![name_like("pic.*"), name_like("l?rge.txt")]))), vec!["large.txt", "pic.png"]);
    assert_eq!(find(Some(MetadataPredicate::And(vec![name_like("*.txt"), not_dir()]))), vec!["large.txt", "small.txt"]);
    let starts = MetadataPredicate::compare(MetadataAttribute::FsName, MetadataOp::BeginsWith, MetadataValue::Str("s".into()));
    assert_eq!(find(Some(starts)), vec!["small.txt"]);
    let png = MetadataPredicate::compare(MetadataAttribute::ContentType, MetadataOp::Equal, MetadataValue::Str("public.png".into()));
    assert_eq!(find(Some(png)), vec!["pic.png"]);
    let mismatch = MetadataPredicate::compare(MetadataAttribute::FsSize, MetadataOp::Equal, MetadataValue::Str("1".into()));
    assert!(find(Some(mismatch)).is_empty(), "a value of the wrong kind never matches");
}

#[test]
fn disabled_updates_are_held_until_enabled() {
    let t = tempfile::tempdir().unwrap();
    write_file(&t.path().join("a.txt"), "x");
    let mut q = MetadataQuery::new();
    q.search_scopes = vec![t.path().to_path_buf()];
    assert!(q.start_query());
    q.poll(Instant::now());
    q.disable_updates();
    write_file(&t.path().join("late.txt"), "y");
    std::thread::sleep(Duration::from_millis(200));
    let held = q.poll(Instant::now());
    assert!(!held.iter().any(|n| matches!(n, MetadataNotification::DidUpdate { .. })));
    assert_eq!(q.result_count(), 1);
    q.enable_updates();
    let got = drain_until(&mut q, |n| matches!(n, MetadataNotification::DidUpdate { .. }));
    assert!(got.iter().any(|n| matches!(n, MetadataNotification::DidUpdate { added, .. } if added.contains(&t.path().join("late.txt")))));
    assert_eq!(q.result_count(), 2);
}
