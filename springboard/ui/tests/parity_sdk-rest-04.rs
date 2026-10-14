//! Parity sdk-rest-04: headless tests for the rendering-context requirement check (`gl_requirements`) and the
//! node-graph composition model (`patch_graph`). No test creates a GPU context or draws a frame.

use bifrost_ui::gl_requirements::{GlApi, GlRequirement, GlVersion, GlVersionError};
use bifrost_ui::patch_graph::{Endpoint, GraphError, PatchGraph, PatchSpec, Port, PortType, Value};

#[test]
fn a_desktop_core_version_string_parses_with_its_profile() {
    let v = GlVersion::parse("3.2 (Core Profile) Mesa 23.2.1").unwrap();
    assert_eq!(v.api, GlApi::Desktop);
    assert_eq!((v.major, v.minor), (3, 2));
    assert!(v.core_profile);
}

#[test]
fn an_embedded_version_string_parses_without_a_profile() {
    let v = GlVersion::parse("OpenGL ES 3.2 Mesa 23.2.1").unwrap();
    assert_eq!(v.api, GlApi::Es);
    assert_eq!((v.major, v.minor), (3, 2));
    assert!(!v.core_profile);
}

#[test]
fn a_vendor_string_with_a_patch_number_reads_major_and_minor_only() {
    let v = GlVersion::parse("4.6.0 NVIDIA 550.54.14").unwrap();
    assert_eq!((v.api, v.major, v.minor, v.core_profile), (GlApi::Desktop, 4, 6, false));
}

#[test]
fn surrounding_spaces_do_not_change_the_parsed_version() {
    assert_eq!(GlVersion::parse("  OpenGL ES 2.0 Mesa 22.0  ").unwrap(), GlVersion::parse("OpenGL ES 2.0 Mesa 22.0").unwrap());
}

#[test]
fn an_empty_version_string_is_reported_as_empty() {
    assert_eq!(GlVersion::parse("   "), Err(GlVersionError::Empty));
    assert_eq!(GlVersion::parse("").unwrap_err().to_string(), "the rendering context reported no GL version");
}

#[test]
fn a_version_without_a_minor_number_is_malformed_and_keeps_the_input() {
    assert_eq!(GlVersion::parse("OpenGL ES 3"), Err(GlVersionError::Malformed("OpenGL ES 3".into())));
    assert_eq!(GlVersion::parse("beta build").unwrap_err().to_string(), "unrecognised GL version string: beta build");
}

#[test]
fn a_core_profile_context_at_three_two_meets_the_desktop_floor() {
    let v = GlVersion::parse("3.2 (Core Profile) Mesa 23.2.1").unwrap();
    assert!(v.satisfies(GlRequirement::DesktopCore32));
    assert!(GlRequirement::DesktopCore32.check(&v).is_ok());
}

#[test]
fn a_compatibility_context_at_four_six_does_not_meet_the_core_floor() {
    let v = GlVersion::parse("4.6.0 NVIDIA 550.54.14").unwrap();
    assert!(!v.satisfies(GlRequirement::DesktopCore32), "the core profile is named in the string, not implied");
    assert_eq!(GlRequirement::DesktopCore32.check(&v), Err("the context does not meet OpenGL 3.2 core profile".to_string()));
}

#[test]
fn a_desktop_context_below_three_two_fails_the_desktop_floor() {
    let v = GlVersion::parse("3.1 (Core Profile) Mesa 20.0").unwrap();
    assert!(!v.satisfies(GlRequirement::DesktopCore32));
}

#[test]
fn an_embedded_context_never_meets_the_desktop_floor() {
    let v = GlVersion::parse("OpenGL ES 3.2 Mesa 23.2.1").unwrap();
    assert!(!v.satisfies(GlRequirement::DesktopCore32));
}

#[test]
fn embedded_floors_compare_the_minor_number_too() {
    let es30 = GlVersion::parse("OpenGL ES 3.0 Mesa 21.0").unwrap();
    let es20 = GlVersion::parse("OpenGL ES 2.0 Mesa 21.0").unwrap();
    assert!(es30.satisfies(GlRequirement::EmbeddedEs20));
    assert!(es30.satisfies(GlRequirement::EmbeddedEs30));
    assert!(es20.satisfies(GlRequirement::EmbeddedEs20));
    assert!(!es20.satisfies(GlRequirement::EmbeddedEs30));
    assert_eq!(GlRequirement::EmbeddedEs30.describe(), "OpenGL ES 3.0");
}

#[test]
fn a_minor_version_compares_as_a_number_so_three_ten_is_above_three_nine() {
    let v = GlVersion::parse("OpenGL ES 3.10 Mesa 25.0").unwrap();
    assert!(v.is_at_least(GlApi::Es, 3, 9));
    assert!(!GlVersion::parse("OpenGL ES 3.9 Mesa 25.0").unwrap().is_at_least(GlApi::Es, 3, 10));
}

#[test]
fn the_graph_starts_empty_and_patches_get_consecutive_indices() {
    let mut g = PatchGraph::new();
    assert_eq!(g.connections().len(), 0);
    assert_eq!(g.add_patch(PatchSpec::new("source", vec![], vec![Port::new("value", PortType::Number)])), 0);
    assert_eq!(g.add_patch(PatchSpec::new("sink", vec![Port::new("in", PortType::Number)], vec![])), 1);
    assert_eq!(g.patch(1).unwrap().name, "sink");
    assert!(g.patch(2).is_none());
}

fn chain() -> PatchGraph {
    let mut g = PatchGraph::new();
    g.add_patch(PatchSpec::new("source", vec![], vec![Port::new("value", PortType::Number)]));
    g.add_patch(PatchSpec::new("double", vec![Port::new("in", PortType::Number)], vec![Port::new("out", PortType::Number)]));
    g.add_patch(PatchSpec::new("show", vec![Port::new("in", PortType::Number)], vec![]));
    g
}

#[test]
fn a_typed_connection_is_recorded_and_its_source_is_found() {
    let mut g = chain();
    g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 0 }).unwrap();
    assert_eq!(g.connections().len(), 1);
    assert_eq!(g.source_of(Endpoint { patch: 1, port: 0 }), Some(Endpoint { patch: 0, port: 0 }));
    assert_eq!(g.source_of(Endpoint { patch: 2, port: 0 }), None);
}

#[test]
fn connecting_ports_of_different_types_is_refused() {
    let mut g = PatchGraph::new();
    g.add_patch(PatchSpec::new("flag", vec![], vec![Port::new("on", PortType::Boolean)]));
    g.add_patch(PatchSpec::new("label", vec![Port::new("text", PortType::Text)], vec![]));
    let err = g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 0 }).unwrap_err();
    assert_eq!(err, GraphError::TypeMismatch { from: PortType::Boolean, to: PortType::Text });
    assert_eq!(err.to_string(), "cannot connect a Boolean port to a Text port");
}

#[test]
fn a_port_index_past_the_end_is_refused_on_either_side() {
    let mut g = chain();
    assert_eq!(g.connect(Endpoint { patch: 0, port: 3 }, Endpoint { patch: 1, port: 0 }), Err(GraphError::NoOutputPort(Endpoint { patch: 0, port: 3 })));
    assert_eq!(g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 4 }), Err(GraphError::NoInputPort(Endpoint { patch: 1, port: 4 })));
}

#[test]
fn an_unknown_patch_is_refused() {
    let mut g = chain();
    assert_eq!(g.connect(Endpoint { patch: 9, port: 0 }, Endpoint { patch: 1, port: 0 }), Err(GraphError::UnknownPatch(9)));
    assert_eq!(g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 7, port: 0 }), Err(GraphError::UnknownPatch(7)));
}

#[test]
fn an_input_takes_only_one_source() {
    let mut g = chain();
    g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 2, port: 0 }).unwrap();
    let again = g.connect(Endpoint { patch: 1, port: 0 }, Endpoint { patch: 2, port: 0 });
    assert_eq!(again, Err(GraphError::InputInUse(Endpoint { patch: 2, port: 0 })));
}

#[test]
fn a_patch_wired_to_its_own_output_is_refused() {
    let mut loopback = PatchGraph::new();
    loopback.add_patch(PatchSpec::new("self", vec![Port::new("in", PortType::Number)], vec![Port::new("out", PortType::Number)]));
    assert_eq!(loopback.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 0, port: 0 }), Err(GraphError::Cycle));
}

#[test]
fn a_longer_loop_through_several_patches_is_refused() {
    let mut g = PatchGraph::new();
    let pass = || PatchSpec::new("pass", vec![Port::new("in", PortType::Number)], vec![Port::new("out", PortType::Number)]);
    g.add_patch(pass());
    g.add_patch(pass());
    g.add_patch(pass());
    g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 0 }).unwrap();
    g.connect(Endpoint { patch: 1, port: 0 }, Endpoint { patch: 2, port: 0 }).unwrap();
    assert_eq!(g.connect(Endpoint { patch: 2, port: 0 }, Endpoint { patch: 0, port: 0 }), Err(GraphError::Cycle));
}

#[test]
fn evaluation_order_puts_each_source_before_its_consumers() {
    let mut g = chain();
    g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 0 }).unwrap();
    g.connect(Endpoint { patch: 1, port: 0 }, Endpoint { patch: 2, port: 0 }).unwrap();
    assert_eq!(g.evaluation_order(), vec![0, 1, 2]);
}

#[test]
fn evaluation_order_breaks_ties_by_the_lower_patch_index() {
    let mut g = PatchGraph::new();
    g.add_patch(PatchSpec::new("late", vec![], vec![Port::new("v", PortType::Number)]));
    g.add_patch(PatchSpec::new("early", vec![], vec![Port::new("v", PortType::Number)]));
    g.add_patch(PatchSpec::new("sink", vec![Port::new("a", PortType::Number)], vec![]));
    g.connect(Endpoint { patch: 1, port: 0 }, Endpoint { patch: 2, port: 0 }).unwrap();
    assert_eq!(g.evaluation_order(), vec![0, 1, 2]);
}

#[test]
fn evaluate_passes_each_output_to_the_input_it_is_wired_to() {
    let mut g = chain();
    g.connect(Endpoint { patch: 0, port: 0 }, Endpoint { patch: 1, port: 0 }).unwrap();
    g.connect(Endpoint { patch: 1, port: 0 }, Endpoint { patch: 2, port: 0 }).unwrap();
    let results = g
        .evaluate(|patch, inputs| match patch {
            0 => vec![Value::Number(21.0)],
            1 => match inputs[0] {
                Some(Value::Number(n)) => vec![Value::Number(n * 2.0)],
                _ => panic!("the double patch must receive the number"),
            },
            _ => vec![],
        })
        .unwrap();
    assert_eq!(results[1], vec![Value::Number(42.0)]);
    assert_eq!(results[2], Vec::<Value>::new());
}

#[test]
fn an_unconnected_input_arrives_as_none() {
    let g = chain();
    let results = g
        .evaluate(|patch, inputs| match patch {
            0 => vec![Value::Number(1.0)],
            1 => {
                assert_eq!(inputs[0], None);
                vec![Value::Number(0.0)]
            }
            _ => vec![],
        })
        .unwrap();
    assert_eq!(results[0], vec![Value::Number(1.0)]);
}

#[test]
fn a_patch_that_returns_the_wrong_number_of_values_stops_the_run() {
    let g = chain();
    let err = g.evaluate(|_, _| Vec::new()).unwrap_err();
    assert_eq!(err, GraphError::OutputCount { patch: 0, expected: 1, found: 0 });
}

#[test]
fn a_patch_that_returns_a_value_of_the_wrong_kind_stops_the_run() {
    let g = chain();
    let err = g.evaluate(|patch, _| if patch == 0 { vec![Value::Text("x".into())] } else { vec![] }).unwrap_err();
    assert_eq!(err, GraphError::ValueKind { patch: 0, port: 0 });
}

#[test]
fn a_value_reports_the_port_type_it_fills() {
    assert_eq!(Value::Number(1.5).kind(), PortType::Number);
    assert_eq!(Value::Boolean(false).kind(), PortType::Boolean);
    assert_eq!(Value::Text(String::new()).kind(), PortType::Text);
}
