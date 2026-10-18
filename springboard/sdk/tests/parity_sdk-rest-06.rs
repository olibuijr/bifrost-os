//! Parity sdk-rest-06: headless tests for the GSS memory model (`gss_buffer`, `gss_oid`, `gss_context`) and the typed
//! file path (`typed_path`). The tests use values in memory only. They never open a file, a socket or a security
//! mechanism.

use std::path::{Path, PathBuf};

use bifrost_app::gss_buffer::{Buffer, BufferSet};
use bifrost_app::gss_context::{Context, Flags};
use bifrost_app::gss_oid::{Oid, OidSet};
use bifrost_app::typed_path::{Part, PathError, TypedPath};

#[test]
fn a_buffer_set_keeps_copies_after_the_caller_reuses_its_storage() {
    let mut storage = vec![1u8, 2, 3];
    let mut set = BufferSet::create_empty();
    set.add_member(&storage);
    storage.fill(0);
    assert_eq!(set.get(0).map(Buffer::as_slice), Some(&[1u8, 2, 3][..]));
    assert_eq!(set.total_len(), 3);
}

#[test]
fn an_owned_buffer_reports_its_length_and_its_emptiness() {
    let owned = Buffer::new(vec![0x47, 0x53]);
    assert_eq!(owned.len(), 2);
    assert!(!owned.is_empty());
    assert!(Buffer::from_slice(&[]).is_empty());
}

#[test]
fn an_object_identifier_round_trips_through_dotted_text_and_der_content() {
    let oid = Oid::parse("1.2.840.113549").expect("a valid dotted identifier");
    assert_eq!(oid.to_dotted(), "1.2.840.113549");
    assert_eq!(oid.to_der_content(), vec![0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d]);
    assert_eq!(Oid::from_der_content(&oid.to_der_content()), Ok(oid));
}

#[test]
fn an_identifier_set_answers_membership_for_the_identifiers_it_holds() {
    let kerberos = Oid::parse("1.2.840.113554.1.2.2").expect("a valid dotted identifier");
    let other = Oid::parse("1.2.3").expect("a valid dotted identifier");
    let mut set = OidSet::new();
    assert!(set.is_empty());
    set.add(kerberos.clone());
    assert!(set.contains(&kerberos));
    assert!(!set.contains(&other));
    assert_eq!(set.len(), 1);
}

#[test]
fn an_initiating_context_sends_its_first_token_once() {
    let mech = Oid::parse("1.2.840.113554.1.2.2").expect("a valid dotted identifier");
    let mut context = Context::initiate(mech.clone(), Flags::none(), None);
    assert_eq!(context.mech(), &mech);
    assert!(!context.is_established());
    let token = context.init_token().expect("the first token");
    assert!(!token.is_empty());
    assert!(context.init_token().is_err(), "a second initial token is refused");
}

#[test]
fn a_parsed_absolute_path_keeps_its_root_and_its_names() {
    let path = TypedPath::parse("/usr/lib/").expect("a valid path");
    assert!(path.is_absolute());
    assert_eq!(path.parts(), &[Part::Name("usr".into()), Part::Name("lib".into())]);
    assert_eq!(path.to_string(), "/usr/lib");
}

#[test]
fn repeated_separators_add_no_components() {
    assert_eq!(TypedPath::parse("a//b///c").unwrap().to_string(), "a/b/c");
    assert_eq!(TypedPath::parse("a//b///c").unwrap().parts().len(), 3);
}

#[test]
fn an_empty_path_is_refused() {
    assert_eq!(TypedPath::parse(""), Err(PathError::Empty));
    assert_eq!(Part::from_name(""), Err(PathError::Empty));
}

#[test]
fn a_nul_byte_is_refused_with_its_offset() {
    assert_eq!(TypedPath::parse("ab\0c"), Err(PathError::NulByte(2)));
    assert_eq!(Part::from_name("x\0"), Err(PathError::NulByte(1)));
}

#[test]
fn a_name_cannot_hide_a_separator() {
    assert_eq!(Part::from_name("a/b"), Err(PathError::SeparatorInName));
    let mut path = TypedPath::parse("docs").unwrap();
    assert_eq!(path.push("x/y"), Err(PathError::SeparatorInName));
    assert_eq!(path.to_string(), "docs", "a refused component leaves the path unchanged");
}

#[test]
fn a_dot_and_a_double_dot_keep_their_own_parts() {
    assert_eq!(Part::from_name("."), Ok(Part::CurrentDirectory));
    assert_eq!(Part::from_name(".."), Ok(Part::ParentDirectory));
    assert_eq!(Part::from_name(".hidden"), Ok(Part::Name(".hidden".into())));
}

#[test]
fn normalising_cancels_a_parent_against_the_name_before_it() {
    let path = TypedPath::parse("/a/b/../c/./d").unwrap();
    assert_eq!(path.lexically_normalized().to_string(), "/a/c/d");
}

#[test]
fn a_parent_at_the_root_has_no_effect() {
    assert_eq!(TypedPath::parse("/../a").unwrap().lexically_normalized().to_string(), "/a");
}

#[test]
fn a_relative_path_keeps_the_parents_it_cannot_cancel() {
    assert_eq!(TypedPath::parse("../../a/../b").unwrap().lexically_normalized().to_string(), "../../b");
}

#[test]
fn remove_last_pops_one_component_and_reports_an_empty_path() {
    let mut path = TypedPath::parse("/a").unwrap();
    assert!(path.remove_last());
    assert_eq!(path.to_string(), "/");
    assert!(!path.remove_last(), "the root has no component to remove");
}

#[test]
fn the_file_name_is_the_last_component_only_when_it_is_a_name() {
    assert_eq!(TypedPath::parse("/x/report.pdf").unwrap().file_name(), Some("report.pdf"));
    assert_eq!(TypedPath::parse("/").unwrap().file_name(), None);
    assert_eq!(TypedPath::parse("a/..").unwrap().file_name(), None);
}

#[test]
fn a_typed_path_converts_to_the_language_path_type_with_the_same_text() {
    let path = TypedPath::parse("/var/cache/app").unwrap();
    let buf: PathBuf = path.to_path_buf();
    assert_eq!(buf.as_path(), Path::new("/var/cache/app"));
    assert_eq!(TypedPath::default().to_path_buf(), PathBuf::from("."));
}

#[test]
fn parsing_through_from_str_matches_parse() {
    let parsed: TypedPath = "x/y".parse().expect("a valid path");
    assert!(!parsed.is_absolute());
    assert_eq!(parsed, TypedPath::parse("x/y").unwrap());
}

#[test]
fn pushing_a_name_extends_a_relative_path() {
    let mut path = TypedPath::parse("docs").unwrap();
    path.push("a.txt").expect("a plain name");
    assert_eq!(path.to_string(), "docs/a.txt");
    assert_eq!(path.file_name(), Some("a.txt"));
}


#[test]
fn a_path_reports_its_root_and_walks_its_components() {
    use bifrost_app::typed_path::{Part, Root, TypedPath};
    let abs = TypedPath::parse("/a/./b/..").expect("parses");
    assert_eq!(abs.root(), Root::Absolute);
    assert_eq!(abs.components().len(), 4);
    assert_eq!(abs.components().iter().next(), Some(&Part::Name("a".into())));
    let rel = TypedPath::parse("docs/x").expect("parses");
    assert_eq!(rel.root(), Root::Relative);
    assert!(!rel.components().is_empty());
}

#[test]
fn the_root_path_has_no_components() {
    use bifrost_app::typed_path::{Root, TypedPath};
    let root = TypedPath::parse("/").expect("parses");
    assert_eq!(root.root(), Root::Absolute);
    assert!(root.components().is_empty());
}

#[test]
fn a_path_converts_to_a_nul_terminated_platform_string() {
    use bifrost_app::typed_path::TypedPath;
    let p = TypedPath::parse("/tmp/report.pdf").expect("parses");
    assert_eq!(p.to_c_string().expect("no NUL byte").as_bytes(), b"/tmp/report.pdf");
}
