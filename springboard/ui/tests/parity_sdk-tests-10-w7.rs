//! Parity sdk-tests-10: headless tests of the widget kind registry (`bifrost_ui::widget_kind`) and of the OBJ scene
//! import (`bifrost_ui::scene_source`).
//!
//! Headless: both are pure models. Nothing is drawn, no file is read from disk, and no shell or extension process runs.
//! Every mesh is built from text held in memory.

use bifrost_ui::scene_source::{import_obj, ImportError};
use bifrost_ui::widget_kind::{Family, KindError, WidgetBundle, WidgetKind, MAX_ID_LEN};
use bifrost_ui::linalg::Vector3;

fn kind(id: &str, families: &[Family]) -> WidgetKind {
    WidgetKind::new(id, id, families).expect("a valid kind")
}

fn ids(list: &[&WidgetKind]) -> Vec<String> {
    list.iter().map(|k| k.id().to_string()).collect()
}

// --- widget kinds ----------------------------------------------------------------------------------------------------

#[test]
fn headless_family_keys_are_the_stored_names_smallest_first() {
    assert_eq!(Family::ALL, [Family::Small, Family::Medium, Family::Large]);
    let keys: Vec<&str> = Family::ALL.iter().map(|f| f.key()).collect();
    assert_eq!(keys, ["small", "medium", "large"]);
    assert!(Family::Small < Family::Medium && Family::Medium < Family::Large);
}

#[test]
fn headless_a_new_kind_keeps_its_name_and_sorts_its_sizes_without_repeats() {
    let k = WidgetKind::new("app.stats", "Stats", &[Family::Large, Family::Small, Family::Large]).unwrap();
    assert_eq!(k.id(), "app.stats");
    assert_eq!(k.display_name, "Stats");
    assert_eq!(k.families(), &[Family::Small, Family::Large]);
    assert!(k.supports(Family::Small));
    assert!(!k.supports(Family::Medium), "a size the kind did not list is not offered");
}

#[test]
fn headless_a_new_kind_has_no_description_and_takes_no_settings() {
    let mut k = kind("app.clock", &[Family::Medium]);
    assert_eq!(k.description, "");
    assert!(!k.configurable);
    k.description = "Shows the time".to_string();
    k.configurable = true;
    assert_eq!(k.description, "Shows the time");
    assert!(k.configurable, "the settings flag is a plain field the app sets");
}

#[test]
fn headless_an_identifier_is_refused_when_empty() {
    assert_eq!(WidgetKind::new("", "x", &[Family::Small]).unwrap_err(), KindError::EmptyId);
}

#[test]
fn headless_an_identifier_may_hold_letters_digits_dots_dashes_and_underscores() {
    for id in ["com.example-app_1.stats", "A9", "x-y_z.0"] {
        assert!(WidgetKind::new(id, "x", &[Family::Small]).is_ok(), "{id} is a valid identifier");
    }
}

#[test]
fn headless_an_identifier_with_a_space_or_slash_names_the_bad_character() {
    assert_eq!(WidgetKind::new("a b", "x", &[Family::Small]).unwrap_err(), KindError::BadChar(' '));
    assert_eq!(WidgetKind::new("a/b", "x", &[Family::Small]).unwrap_err(), KindError::BadChar('/'));
    assert_eq!(WidgetKind::new("caf\u{e9}", "x", &[Family::Small]).unwrap_err(), KindError::BadChar('\u{e9}'));
}

#[test]
fn headless_an_identifier_longer_than_the_limit_is_refused_and_the_limit_itself_is_accepted() {
    assert_eq!(MAX_ID_LEN, 128);
    let long = "a".repeat(MAX_ID_LEN + 1);
    assert_eq!(WidgetKind::new(&long, "x", &[Family::Small]).unwrap_err(), KindError::TooLong);
    let edge = "a".repeat(MAX_ID_LEN);
    assert_eq!(WidgetKind::new(&edge, "x", &[Family::Small]).unwrap().id().len(), MAX_ID_LEN);
}

#[test]
fn headless_a_kind_with_no_size_is_refused_because_nothing_could_be_shown() {
    assert_eq!(WidgetKind::new("app.empty", "Empty", &[]).unwrap_err(), KindError::NoFamilies);
}

#[test]
fn headless_an_empty_bundle_offers_no_sizes_and_no_kinds() {
    let b = WidgetBundle::new();
    assert!(b.is_empty());
    assert_eq!(b.len(), 0);
    assert!(b.sizes().is_empty());
    assert!(b.offered_at(Family::Small).is_empty());
    assert_eq!(b, WidgetBundle::default());
}

#[test]
fn headless_a_bundle_refuses_a_second_kind_with_the_same_identifier_and_keeps_the_first() {
    let mut b = WidgetBundle::new();
    b.add(kind("app.one", &[Family::Small])).unwrap();
    let again = WidgetKind::new("app.one", "Other", &[Family::Medium]).unwrap();
    assert_eq!(b.add(again), Err(KindError::DuplicateId("app.one".to_string())));
    assert_eq!(b.len(), 1);
    assert_eq!(b.get("app.one").unwrap().display_name, "app.one");
    assert!(b.get("app.missing").is_none());
}

#[test]
fn headless_offered_at_lists_the_kinds_of_one_size_in_registration_order() {
    let mut b = WidgetBundle::new();
    b.add(kind("a", &[Family::Small])).unwrap();
    b.add(kind("b", &[Family::Medium, Family::Large])).unwrap();
    b.add(kind("c", &[Family::Small, Family::Medium])).unwrap();
    assert_eq!(ids(&b.offered_at(Family::Medium)), ["b", "c"]);
    assert_eq!(ids(&b.offered_at(Family::Small)), ["a", "c"]);
    assert_eq!(ids(&b.offered_at(Family::Large)), ["b"]);
}

#[test]
fn headless_the_bundle_sizes_are_the_union_of_its_kind_sizes_smallest_first() {
    let mut b = WidgetBundle::new();
    b.add(kind("a", &[Family::Large])).unwrap();
    b.add(kind("b", &[Family::Small, Family::Large])).unwrap();
    assert_eq!(b.sizes(), vec![Family::Small, Family::Large]);
    assert_eq!(b.kinds().len(), 2);
    assert_eq!(b.kinds()[0].id(), "a", "kinds() keeps registration order");
}

// --- OBJ scene import ------------------------------------------------------------------------------------------------

fn close(a: Vector3, b: Vector3) -> bool {
    (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5 && (a.z - b.z).abs() < 1e-5
}

#[test]
fn headless_one_triangle_imports_as_one_mesh_with_a_flat_normal_from_its_winding() {
    let meshes = import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    assert_eq!(meshes.len(), 1);
    let mesh = &meshes[0];
    assert_eq!(mesh.name, "");
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.triangle_count(), 1);
    assert!(close(mesh.normals[0], Vector3::new(0.0, 0.0, 1.0)), "counter-clockwise from +z gives +z");
    assert_eq!(mesh.texture_coordinates[0], [0.0, 0.0], "no texture coordinate gives the origin");
}

#[test]
fn headless_a_quad_is_fan_triangulated_into_two_triangles() {
    let mesh = &import_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").unwrap()[0];
    assert_eq!(mesh.positions.len(), 4);
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.triangle_count(), 2);
}

#[test]
fn headless_a_pentagon_is_fan_triangulated_into_three_triangles() {
    let text = "v 0 0 0\nv 1 0 0\nv 1.5 1 0\nv 0.5 1.5 0\nv -0.5 1 0\nf 1 2 3 4 5\n";
    let mesh = &import_obj(text).unwrap()[0];
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 0, 3, 4]);
    assert_eq!(mesh.triangle_count(), 3);
}

#[test]
fn headless_negative_indices_count_back_from_the_last_vertex_defined() {
    let mesh = &import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n").unwrap()[0];
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.positions[1], Vector3::new(1.0, 0.0, 0.0));
}

#[test]
fn headless_shared_corners_collapse_to_one_vertex_and_keep_the_file_normal_and_texture() {
    let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
                vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
                f 1/1/1 2/2/1 3/3/1\nf 1/1/1 3/3/1 4/4/1\n";
    let mesh = &import_obj(text).unwrap()[0];
    assert_eq!(mesh.positions.len(), 4, "the two faces share the corners 1 and 3");
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.texture_coordinates[2], [1.0, 1.0]);
    assert!(close(mesh.normals[0], Vector3::new(0.0, 0.0, 1.0)));
}

#[test]
fn headless_a_position_and_normal_face_form_takes_the_file_normal_without_texture() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 -1\nf 1//1 2//1 3//1\n";
    let mesh = &import_obj(text).unwrap()[0];
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert!(close(mesh.normals[1], Vector3::new(0.0, 0.0, -1.0)), "the file normal wins over the winding");
    assert_eq!(mesh.texture_coordinates[1], [0.0, 0.0]);
}

#[test]
fn headless_corners_without_normals_are_flat_shaded_so_a_shared_corner_is_split_per_face() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nf 1 2 3\nf 2 4 3\n";
    let mesh = &import_obj(text).unwrap()[0];
    assert_eq!(mesh.positions.len(), 6, "each face keeps its own copies of the shared corners");
    assert_eq!(mesh.triangle_count(), 2);
    assert!(close(mesh.normals[5], Vector3::new(0.0, 0.0, 1.0)));
}

#[test]
fn headless_each_object_becomes_its_own_named_mesh_and_an_object_without_faces_is_dropped() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\no first\nf 1 2 3\no second\nf 3 2 1\no empty\n";
    let meshes = import_obj(text).unwrap();
    assert_eq!(meshes.len(), 2);
    assert_eq!(meshes[0].name, "first");
    assert_eq!(meshes[1].name, "second");
    assert_eq!(meshes[1].indices, vec![0, 1, 2]);
}

#[test]
fn headless_an_object_name_keeps_its_spaces() {
    let meshes = import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\no main body\nf 1 2 3\n").unwrap();
    assert_eq!(meshes[0].name, "main body");
}

#[test]
fn headless_comments_and_material_lines_are_read_and_ignored() {
    let text = "# exported scene\nmtllib scene.mtl\ng part\ns off\nusemtl red\nv 0 0 0 # origin\nv 1 0 0\nv 0 1 0\nf 1 2 3 # face\n";
    let meshes = import_obj(text).unwrap();
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].indices, vec![0, 1, 2]);
}

#[test]
fn headless_a_malformed_number_or_a_short_face_is_refused_with_its_line_number() {
    assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nf 1 2\n"), Err(ImportError::Syntax { line: 3 }));
    assert_eq!(import_obj("v 0 0 x\n"), Err(ImportError::Syntax { line: 1 }));
    assert_eq!(import_obj("v 0 0\n"), Err(ImportError::Syntax { line: 1 }));
}

#[test]
fn headless_a_non_finite_coordinate_is_refused() {
    assert_eq!(import_obj("v nan 0 0\n"), Err(ImportError::Syntax { line: 1 }));
    assert_eq!(import_obj("v 0 inf 0\n"), Err(ImportError::Syntax { line: 1 }));
}

#[test]
fn headless_a_face_index_past_the_defined_vertices_or_zero_is_refused() {
    assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n"), Err(ImportError::Index { line: 4 }));
    assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n"), Err(ImportError::Index { line: 4 }));
    assert_eq!(import_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nf 1/2 2 3\n"), Err(ImportError::Index { line: 5 }));
}

#[test]
fn headless_a_file_with_no_faces_has_nothing_to_import() {
    assert_eq!(import_obj("v 0 0 0\n# only a comment\n"), Err(ImportError::NoFaces));
    assert_eq!(import_obj(""), Err(ImportError::NoFaces));
}
