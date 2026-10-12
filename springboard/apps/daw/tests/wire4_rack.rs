//! Host tests for the rack inspector model (src/rack.rs) and the tree helpers in src/params.rs: slider values and
//! positions on each parameter's curve, stream format text, preset file names, and presets saved and loaded from the
//! instrument's stored values. Expected values come from the device specs in src/devices (filter: type 0..2 integer
//! default 0, freq 40..18000 logarithmic default 18000, q 0..20 linear default 1).
use daw::devices::{effect_spec, instrument_spec};
use daw::model::Extra;
use daw::params;
use daw::rack::{self, Item, Owner};
use serde_json::Value;
use std::path::Path;

fn filter_item(index: usize) -> Item {
    Item { owner: Owner::Effect(0), spec: effect_spec("filter").expect("filter"), index }
}

fn number(extra: &Extra, name: &str) -> Option<f64> {
    extra.get(name).and_then(Value::as_f64)
}

#[test]
fn value_is_the_stored_value_or_the_spec_default() {
    let freq = filter_item(1);
    let mut extra = Extra::new();
    assert_eq!(rack::value(&freq, &extra), 18000.0);
    extra.insert("freq".to_string(), Value::from(500.0));
    assert_eq!(rack::value(&freq, &extra), 500.0);
}

#[test]
fn set_clamps_into_the_range_and_stores_the_clamped_value() {
    let freq = filter_item(1);
    let mut extra = Extra::new();
    assert_eq!(rack::set(&freq, &mut extra, 99999.0), 18000.0);
    assert_eq!(rack::set(&freq, &mut extra, 10.0), 40.0);
    assert_eq!(number(&extra, "freq"), Some(40.0));
}

#[test]
fn logarithmic_slider_spans_the_range_on_a_log_curve() {
    let freq = filter_item(1);
    // the top of the range is exactly the top of the slider, and the bottom is the bottom
    assert_eq!(rack::position(&freq, 18000.0), 1.0);
    assert_eq!(rack::position(&freq, 40.0), 0.0);
    // the slider's start maps back to the minimum, to within float error
    assert!((rack::value_at(&freq, 0.0) - 40.0).abs() < 0.001);
    assert!((rack::value_at(&freq, 1.0) - 18000.0).abs() < 0.01);
}

#[test]
fn linear_slider_is_proportional_and_clamps_its_position() {
    let q = filter_item(2);
    assert_eq!(rack::position(&q, 5.0), 0.25);
    assert_eq!(rack::position(&q, 99.0), 1.0);
    assert_eq!(rack::value_at(&q, 0.5), 10.0);
    assert_eq!(rack::value_at(&q, 2.0), 20.0);
}

#[test]
fn stream_format_reads_as_rate_layout_and_labels() {
    assert_eq!(rack::describe(48000, 2), "48000 Hz · Stereo · Left, Right");
    assert_eq!(rack::describe(44100, 1), "44100 Hz · Mono · Center");
    assert_eq!(rack::describe(48000, 3), "48000 Hz · 3 discrete channels");
}

#[test]
fn stream_format_the_render_model_refuses_reads_as_unavailable() {
    assert_eq!(rack::describe(48000, 0), "Stream format unavailable");
    assert_eq!(rack::describe(4000, 2), "Stream format unavailable");
}

#[test]
fn preset_files_are_named_by_the_kind_with_unsafe_characters_dropped() {
    let dir = Path::new("/s");
    assert_eq!(rack::preset_path(dir, "synth").to_str(), Some("/s/presets/synth.json"));
    assert_eq!(rack::preset_path(dir, "../x").to_str(), Some("/s/presets/x.json"));
}

#[test]
fn tree_helpers_read_stored_values_and_write_them_back() {
    let spec = effect_spec("filter").expect("filter");
    let mut extra = Extra::new();
    let tree = params::tree_from_extra(spec, &extra).expect("tree");
    assert_eq!(tree.value(1), Some(18000.0));

    extra.insert("freq".to_string(), Value::from(500.0));
    let mut tree = params::tree_from_extra(spec, &extra).expect("tree");
    assert_eq!(tree.value(1), Some(500.0));
    tree.set_value(1, 900.0).expect("write");

    let mut out = Extra::new();
    params::write_back(spec, &tree, &mut out);
    assert_eq!(number(&out, "freq"), Some(900.0));
    assert_eq!(number(&out, "type"), Some(0.0));
    assert_eq!(number(&out, "q"), Some(1.0));
}

#[test]
fn instrument_preset_saves_the_values_and_loads_them_back() {
    let spec = instrument_spec("synth").expect("synth");
    let first = spec[0];
    let mut store = params::new_presets().expect("store");
    let mut extra = Extra::new();

    extra.insert(first.name.to_string(), Value::from(first.min));
    let name = rack::add_preset(&mut store, "synth", &extra).expect("add");
    assert_eq!(name, "Preset 1");

    extra.insert(first.name.to_string(), Value::from(first.max));
    let number = store.presets()[0].number;
    let loaded = rack::load_preset(&mut store, "synth", &mut extra, number).expect("load");
    assert_eq!(loaded, "Preset 1");
    let back = number_of(&extra, first.name);
    assert!((back - first.min).abs() < 1e-4 * first.min.abs().max(1.0), "{back} != {}", first.min);
}

fn number_of(extra: &Extra, name: &str) -> f64 {
    number(extra, name).expect("stored number")
}
