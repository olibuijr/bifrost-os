//! Host tests for the MIDI and device parameter wiring (src/midi.rs, src/params.rs): note recording from MIDI
//! messages with the sustain pedal, scheduled wire bytes, MIDI clock pulses, device parameters as plug-in
//! parameters, and presets saved, applied and written to JSON.
use bifrost_app::controls::midi_events::{MidiEvent, MidiParser};
use bifrost_app::controls::plugin_host::Unit;
use daw::devices::{effect_spec, instrument_spec, Params};
use daw::midi::{clock_pulses, schedule, Recorder, CLOCK, START, STOP};
use daw::model::{Extra, Note};
use daw::params::{apply_preset, new_presets, params_from_tree, presets_from_json, presets_to_json, save_preset, tree_for, unit_for};

fn note(t: f64, d: f64, p: f64, v: f64) -> Note {
    Note { t, d, p, v, prob: None, extra: Extra::new() }
}

#[test]
fn recorder_pairs_note_on_and_off_in_steps() {
    let mut r = Recorder::new();
    r.event(0.0, MidiEvent::NoteOn { channel: 0, key: 60, velocity: 127 });
    r.event(2.0, MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
    let notes = r.finish(4.0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].t, 0.0);
    assert_eq!(notes[0].d, 2.0);
    assert_eq!(notes[0].p, 60.0);
    assert_eq!(notes[0].v, 1.0);
}

#[test]
fn sustain_pedal_holds_a_note_until_it_is_released() {
    let mut r = Recorder::new();
    r.event(0.0, MidiEvent::ControlChange { channel: 0, controller: 64, value: 127 });
    r.event(0.0, MidiEvent::NoteOn { channel: 0, key: 60, velocity: 64 });
    r.event(1.0, MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
    r.event(3.0, MidiEvent::ControlChange { channel: 0, controller: 64, value: 0 });
    let notes = r.finish(10.0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].d, 3.0);
}

#[test]
fn finish_closes_notes_still_held() {
    let mut r = Recorder::new();
    r.event(1.0, MidiEvent::NoteOn { channel: 0, key: 64, velocity: 100 });
    let notes = r.finish(5.0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].t, 1.0);
    assert_eq!(notes[0].d, 4.0);
}

#[test]
fn raw_bytes_keep_running_status_across_calls() {
    let mut r = Recorder::new();
    r.bytes(0.0, &[0x90, 64, 90]);
    r.bytes(1.0, &[64, 0]);
    let notes = r.finish(2.0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].p, 64.0);
    assert_eq!(notes[0].d, 1.0);
}

#[test]
fn schedule_puts_note_off_before_note_on_at_the_same_step() {
    let notes = vec![note(0.0, 1.0, 60.0, 1.0), note(1.0, 1.0, 60.0, 0.5)];
    let s = schedule(&notes, 0);
    assert_eq!(s.len(), 4);
    let at_one: Vec<bool> = s.iter().filter(|x| x.step == 1.0).map(|x| x.on).collect();
    assert_eq!(at_one, vec![false, true]);
    assert_eq!(s[0].bytes, [0x90, 60, 127]);
    assert_eq!(s[0].len, 3);
}

#[test]
fn scheduled_bytes_parse_back_to_the_same_notes() {
    let notes = vec![note(0.0, 2.0, 67.0, 0.5)];
    let bytes: Vec<u8> = schedule(&notes, 2)
        .iter()
        .flat_map(|x| x.bytes[..x.len].to_vec())
        .collect();
    let events = MidiParser::parse_all(&bytes);
    assert_eq!(
        events,
        vec![
            MidiEvent::NoteOn { channel: 2, key: 67, velocity: 64 },
            MidiEvent::NoteOff { channel: 2, key: 67, velocity: 0 },
        ]
    );
}

#[test]
fn schedule_skips_notes_with_non_finite_fields() {
    let notes = vec![note(f64::NAN, 1.0, 60.0, 1.0), note(0.0, 1.0, 60.0, 1.0)];
    assert_eq!(schedule(&notes, 0).len(), 2);
}

#[test]
fn clock_pulses_are_six_per_step() {
    assert_eq!(clock_pulses(0.0, 4.0), 24);
    assert_eq!(clock_pulses(1.0, 2.0), 6);
    assert_eq!(clock_pulses(4.0, 0.0), 0);
    assert_eq!((CLOCK, START, STOP), (0xF8, 0xFA, 0xFC));
}

#[test]
fn every_effect_and_instrument_spec_becomes_a_tree() {
    for kind in ["filter", "delay", "reverb", "eq", "comp", "drive"] {
        let spec = effect_spec(kind).expect(kind);
        assert!(tree_for(spec).is_ok(), "{kind}");
    }
    let synth = instrument_spec("synth").expect("synth");
    assert!(tree_for(synth).is_ok());
}

#[test]
fn tree_clamps_writes_and_reports_units() {
    let spec = effect_spec("filter").expect("filter");
    let mut tree = tree_for(spec).expect("tree");
    assert_eq!(tree.value(1), Some(18000.0));
    assert_eq!(tree.set_value(1, 99999.0).expect("write"), 18000.0);
    assert_eq!(tree.parameter(1).map(|p| p.unit), Some(Unit::Hertz));
    assert_eq!(unit_for("dB"), Unit::Decibels);
    assert_eq!(unit_for(""), Unit::Generic);
}

#[test]
fn tree_values_reach_the_device_engine() {
    let spec = effect_spec("filter").expect("filter");
    let mut tree = tree_for(spec).expect("tree");
    tree.set_value(1, 1000.0).expect("write");
    let params = params_from_tree(spec, &tree);
    assert_eq!(params.get("freq"), 1000.0);
    assert_eq!(Params::new(spec, &Extra::new()).get("freq"), 18000.0);
}

#[test]
fn presets_save_apply_and_survive_json() {
    let spec = effect_spec("filter").expect("filter");
    let mut tree = tree_for(spec).expect("tree");
    let mut store = new_presets().expect("store");
    tree.set_value(1, 500.0).expect("write");
    let number = save_preset(&mut store, &tree, "Slapback").expect("save");
    tree.set_value(1, 900.0).expect("write");
    apply_preset(&mut store, &mut tree, number).expect("apply");
    assert_eq!(tree.value(1), Some(500.0));
    assert_eq!(store.current(), Some(number));

    let json = presets_to_json(&store);
    let mut reloaded = new_presets().expect("store");
    assert_eq!(presets_from_json(&mut reloaded, &json), Ok(1));
    assert_eq!(reloaded.presets()[0].name, "Slapback");
    assert_eq!(reloaded.presets()[0].state, store.presets()[0].state);
}

#[test]
fn bad_preset_json_is_an_error_and_adds_nothing() {
    let mut store = new_presets().expect("store");
    assert!(presets_from_json(&mut store, "not json").is_err());
    assert!(store.presets().is_empty());
}
