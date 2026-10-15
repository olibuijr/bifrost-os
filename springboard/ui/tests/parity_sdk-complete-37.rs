//! Parity sdk-complete-37: headless tests for the switch watch, the model compile step and the
//! model artifact cache. No test opens a device or a model file.

use bifrost_ui::input_switches::{edge_for, SwitchEdge, SwitchState, SwitchWatch, SW_HEADPHONE_INSERT, SW_LID, SW_TABLET_MODE};
use bifrost_ui::model_compile::{compile, output_width, prepare, CompileError, Graph, Op, Target, MAGIC, VERSION};
use bifrost_ui::model_specialization::{Policy, SpecializationCache};

fn classifier() -> Graph {
    Graph { name: "classifier".into(), input_width: 4, ops: vec![Op::Dense { outputs: 8 }, Op::Relu, Op::Dense { outputs: 3 }, Op::Softmax] }
}

#[test]
fn tablet_mode_is_read_from_its_own_bit() {
    assert!(SwitchState::from_bits(1 << SW_TABLET_MODE).tablet_mode());
    assert!(!SwitchState::from_bits(1 << SW_LID).tablet_mode());
    assert!(!SwitchState::default().tablet_mode());
}

#[test]
fn a_watch_receives_only_the_edges_of_its_switches() {
    let mut state = SwitchState::default();
    let edges = state.update((1 << SW_LID) | (1 << SW_HEADPHONE_INSERT));
    let headset = SwitchWatch::new(&[SW_HEADPHONE_INSERT]);
    assert_eq!(headset.select(&edges), vec![SwitchEdge { code: SW_HEADPHONE_INSERT, on: true }]);
    assert!(!headset.watches(SW_LID));
}

#[test]
fn a_watch_keeps_the_edge_order_of_the_update() {
    let mut state = SwitchState::from_bits(0b10001);
    let edges = state.update(0b00100);
    let watch = SwitchWatch::new(&[4, 0]);
    assert_eq!(watch.select(&edges), vec![SwitchEdge { code: 0, on: false }, SwitchEdge { code: 4, on: false }]);
    assert_eq!(edge_for(&watch.select(&edges), 2), None);
}

#[test]
fn a_watch_for_all_switches_passes_every_edge_and_codes_above_31_are_never_watched() {
    let edges = [SwitchEdge { code: 1, on: true }, SwitchEdge { code: 31, on: false }];
    assert_eq!(SwitchWatch::all().select(&edges), edges.to_vec());
    assert!(!SwitchWatch::new(&[32, 200]).watches(32));
    assert_eq!(SwitchWatch::new(&[]).select(&edges), Vec::new());
}

#[test]
fn the_compiled_artifact_starts_with_the_magic_version_and_target_tag() {
    let artifact = compile(&classifier(), Target::Cpu).unwrap();
    assert_eq!(&artifact[..4], MAGIC);
    assert_eq!(artifact[4], VERSION);
    assert_eq!(artifact[5], 0, "cpu target tag");
    let gpu = compile(&classifier(), Target::Gpu).unwrap();
    assert_eq!(gpu[5], 1, "gpu target tag");
    assert_ne!(artifact, gpu, "the same model compiles differently for each target");
}

#[test]
fn compiling_the_same_description_gives_the_same_bytes() {
    assert_eq!(compile(&classifier(), Target::Gpu).unwrap(), compile(&classifier(), Target::Gpu).unwrap());
}

#[test]
fn the_output_width_follows_the_last_dense_operator() {
    assert_eq!(output_width(&classifier()), Ok(3));
    let relu_only = Graph { name: "r".into(), input_width: 5, ops: vec![Op::Relu] };
    assert_eq!(output_width(&relu_only), Ok(5));
}

#[test]
fn the_artifact_ends_with_the_output_width_for_the_runtime_to_check() {
    let artifact = compile(&classifier(), Target::Cpu).unwrap();
    assert_eq!(&artifact[artifact.len() - 4..], &3u32.to_le_bytes());
}

#[test]
fn a_zero_width_is_refused_at_the_input_and_at_a_dense_operator() {
    let no_input = Graph { name: "m".into(), input_width: 0, ops: vec![Op::Relu] };
    assert_eq!(compile(&no_input, Target::Cpu), Err(CompileError::ZeroWidth));
    let no_outputs = Graph { name: "m".into(), input_width: 2, ops: vec![Op::Dense { outputs: 0 }] };
    assert_eq!(compile(&no_outputs, Target::Cpu), Err(CompileError::ZeroWidth));
}

#[test]
fn a_model_needs_a_name_and_at_least_one_operator() {
    let unnamed = Graph { name: String::new(), input_width: 2, ops: vec![Op::Relu] };
    assert_eq!(compile(&unnamed, Target::Cpu), Err(CompileError::EmptyName));
    let empty = Graph { name: "m".into(), input_width: 2, ops: vec![] };
    assert_eq!(compile(&empty, Target::Cpu), Err(CompileError::NoOperators));
}

#[test]
fn prepare_compiles_once_and_then_reuses_the_artifact() {
    let mut cache = SpecializationCache::new(1024);
    assert_eq!(prepare(&mut cache, &classifier(), Target::Cpu, Policy::Purgeable), Ok(true));
    assert_eq!(prepare(&mut cache, &classifier(), Target::Cpu, Policy::Purgeable), Ok(false));
    let held = cache.lookup("classifier", "cpu").unwrap().to_vec();
    assert_eq!(held, compile(&classifier(), Target::Cpu).unwrap());
}

#[test]
fn each_target_has_its_own_artifact_in_the_cache() {
    let mut cache = SpecializationCache::new(1024);
    assert_eq!(prepare(&mut cache, &classifier(), Target::Cpu, Policy::Purgeable), Ok(true));
    assert_eq!(prepare(&mut cache, &classifier(), Target::Gpu, Policy::Persistent), Ok(true));
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.policy_of("classifier", "gpu"), Some(Policy::Persistent));
}

#[test]
fn a_purge_drops_the_purgeable_artifact_and_a_persistent_one_stays() {
    let mut cache = SpecializationCache::new(1024);
    prepare(&mut cache, &classifier(), Target::Cpu, Policy::Purgeable).unwrap();
    prepare(&mut cache, &classifier(), Target::Gpu, Policy::Persistent).unwrap();
    assert_eq!(cache.purge(), 1);
    assert_eq!(prepare(&mut cache, &classifier(), Target::Cpu, Policy::Purgeable), Ok(true), "a purged artifact is compiled again");
    assert_eq!(prepare(&mut cache, &classifier(), Target::Gpu, Policy::Persistent), Ok(false));
}

#[test]
fn a_refused_description_stores_nothing() {
    let mut cache = SpecializationCache::new(1024);
    let bad = Graph { name: "bad".into(), input_width: 0, ops: vec![Op::Relu] };
    assert_eq!(prepare(&mut cache, &bad, Target::Cpu, Policy::Persistent), Err(CompileError::ZeroWidth));
    assert!(cache.is_empty());
}
