//! Parity sdk-tests-19: tests of the render crate's purgeable cache, font manager and RAW processing parameters through
//! their public API. No test decodes RAW data, draws, or waits for the memory pressure watcher.

use bifrost_render::fontobj::{AutoActivation, FontError, FontEvent, FontManager, FontSpec, Scope};
use bifrost_render::purgeable::{Cache, MemoryPressure};
use bifrost_render::raw_params::{
    RawError, RawParameter, RawProcessingParameters, RawRange, RawValue,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

const SYSTEM_FONT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf");
const ASSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets");

fn system_font_bytes() -> Vec<u8> {
    std::fs::read(SYSTEM_FONT).expect("assets/SFUI.ttf")
}

fn key(s: &str) -> String {
    s.to_string()
}

// ---- purgeable caches ----

#[test]
fn a_count_limit_evicts_the_least_recently_used_entry() {
    let c: Cache<String, u32> = Cache::new(2, 0);
    c.set(key("a"), 1, 1);
    c.set(key("b"), 2, 1);
    assert_eq!(c.get(&key("a")), Some(1));
    c.set(key("c"), 3, 1);
    assert_eq!(c.get(&key("b")), None);
    assert_eq!(c.get(&key("a")), Some(1));
    assert_eq!(c.get(&key("c")), Some(3));
    assert_eq!(c.len(), 2);
}

#[test]
fn a_cost_limit_evicts_oldest_entries_until_the_total_fits() {
    let c: Cache<String, u32> = Cache::new(0, 10);
    c.set(key("a"), 1, 6);
    c.set(key("b"), 2, 6);
    assert_eq!(c.get(&key("a")), None);
    assert_eq!(c.get(&key("b")), Some(2));
    assert_eq!(c.total_cost(), 6);
}

#[test]
fn a_value_costing_more_than_the_whole_limit_is_not_kept() {
    let c: Cache<String, u32> = Cache::new(0, 10);
    c.set(key("big"), 1, 100);
    assert_eq!(c.get(&key("big")), None);
    assert!(c.is_empty());
    assert_eq!(c.total_cost(), 0);
}

#[test]
fn replacing_a_key_keeps_the_cost_total_exact() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    c.set(key("a"), 1, 3);
    c.set(key("a"), 2, 5);
    assert_eq!(c.total_cost(), 5);
    assert_eq!(c.len(), 1);
    assert_eq!(c.get(&key("a")), Some(2));
}

#[test]
fn removing_entries_updates_the_count_and_the_total() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    c.set(key("a"), 1, 3);
    c.set(key("b"), 2, 4);
    c.remove(&key("a"));
    assert_eq!(c.total_cost(), 4);
    assert_eq!(c.len(), 1);
    c.remove(&key("missing"));
    assert_eq!(c.len(), 1);
    c.remove_all();
    assert!(c.is_empty());
    assert_eq!(c.total_cost(), 0);
}

#[test]
fn a_purge_drops_the_oldest_share_of_the_cost() {
    // Three entries of cost 4 make 12. Half of that leaves a target of 6, so the two oldest go and 8 is freed.
    let c: Cache<String, u32> = Cache::new(0, 0);
    c.set(key("a"), 1, 4);
    c.set(key("b"), 2, 4);
    c.set(key("c"), 3, 4);
    assert_eq!(c.purge(0.5), 8);
    assert_eq!(c.get(&key("a")), None);
    assert_eq!(c.get(&key("b")), None);
    assert_eq!(c.get(&key("c")), Some(3));
    assert_eq!(c.total_cost(), 4);
}

#[test]
fn a_full_purge_empties_the_cache_and_reports_the_cost_freed() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    c.set(key("a"), 1, 7);
    c.set(key("b"), 2, 5);
    assert_eq!(c.purge(1.0), 12);
    assert!(c.is_empty());
}

#[test]
fn lowering_a_limit_trims_the_cache_at_once() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    for i in 0..4u32 {
        c.set(format!("k{i}"), i, 1);
    }
    c.set_count_limit(2);
    assert_eq!(c.count_limit(), 2);
    assert_eq!(c.len(), 2);
    c.set_total_cost_limit(1);
    assert_eq!(c.total_cost_limit(), 1);
    assert_eq!(c.total_cost(), 1);
    assert_eq!(c.get(&key("k3")), Some(3));
}

#[test]
fn clones_of_a_cache_share_one_store() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    let other = c.clone();
    other.set(key("a"), 1, 1);
    assert_eq!(c.get(&key("a")), Some(1));
    assert_eq!(c.len(), 1);
}

#[test]
fn a_zero_limit_keeps_every_entry() {
    let c: Cache<String, u32> = Cache::new(0, 0);
    for i in 0..200u32 {
        c.set(format!("k{i}"), i, 1);
    }
    assert_eq!(c.len(), 200);
    assert_eq!(c.total_cost(), 200);
}

#[test]
fn the_memory_pressure_levels_are_distinct_copies() {
    let level = MemoryPressure::Critical;
    let copy = level;
    assert_eq!(level, copy);
    assert_ne!(MemoryPressure::Normal, MemoryPressure::Warning);
    assert_ne!(MemoryPressure::Warning, MemoryPressure::Critical);
}

// ---- font manager ----

#[test]
fn a_face_is_registered_once_by_its_postscript_name() {
    let mut m = FontManager::new();
    let ps = m
        .register_data(system_font_bytes(), Scope::Process)
        .unwrap();
    assert_eq!(
        m.register_data(system_font_bytes(), Scope::Process),
        Err(FontError::AlreadyRegistered)
    );
    assert_eq!(m.available_postscript_names(), vec![ps.clone()]);
    assert_eq!(m.scope_of(&ps), Some(Scope::Process));
    assert_eq!(m.scope_of("No-Such-Face"), None);
}

#[test]
fn bytes_that_are_not_a_font_are_refused_and_leave_no_face() {
    let mut m = FontManager::new();
    assert_eq!(
        m.register_data(vec![1, 2, 3], Scope::Process),
        Err(FontError::Invalid)
    );
    assert!(m.faces().is_empty());
    assert_eq!(m.generation(), 0);
}

#[test]
fn registration_and_removal_report_events_in_order_and_notify_listeners() {
    let mut m = FontManager::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    m.on_change(move |e| sink.lock().unwrap().push(e.clone()));
    let ps = m
        .register_data(system_font_bytes(), Scope::Process)
        .unwrap();
    m.unregister(&ps).unwrap();
    assert_eq!(
        m.drain_events(),
        vec![
            FontEvent::Registered(ps.clone()),
            FontEvent::Unregistered(ps.clone())
        ]
    );
    assert!(m.drain_events().is_empty());
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(m.generation(), 2);
}

#[test]
fn a_system_face_cannot_be_removed_and_an_unknown_face_is_not_found() {
    let mut m = FontManager::new();
    let ps = m.register_data(system_font_bytes(), Scope::System).unwrap();
    assert_eq!(m.unregister(&ps), Err(FontError::InUse));
    assert_eq!(m.unregister("No-Such-Face"), Err(FontError::NotRegistered));
    assert_eq!(m.faces().len(), 1);
}

#[test]
fn a_file_registered_by_url_is_removed_by_the_same_url() {
    let mut m = FontManager::new();
    let url = format!("file://{SYSTEM_FONT}");
    m.register_url(&url, Scope::User).unwrap();
    assert_eq!(m.faces()[0].source.path(), Some(SYSTEM_FONT));
    assert!(m.bytes(0).is_some());
    assert_eq!(m.unregister_url(&url), 1);
    assert!(m.faces().is_empty());
    assert_eq!(m.unregister_url("/nowhere/none.ttf"), 0);
}

#[test]
fn a_missing_font_file_is_an_io_error() {
    let mut m = FontManager::new();
    assert!(matches!(
        m.register_url("/no/such/font-file.ttf", Scope::User),
        Err(FontError::Io(_))
    ));
}

#[test]
fn scanning_the_asset_folder_registers_each_readable_font_as_a_system_face() {
    let mut m = FontManager::new();
    let n = m.scan_dir(Path::new(ASSET_DIR), 0);
    assert!(n >= 1);
    assert_eq!(n, m.faces().len());
    assert!(m.faces().iter().all(|f| f.scope == Scope::System));
}

#[test]
fn family_names_are_listed_once_and_sorted_ignoring_case() {
    let mut m = FontManager::new();
    m.scan_dir(Path::new(ASSET_DIR), 0);
    let families = m.available_families();
    let mut sorted = families.clone();
    sorted.sort_by_key(|s| s.to_ascii_lowercase());
    assert_eq!(families, sorted);
    let unique: std::collections::HashSet<String> =
        families.iter().map(|s| s.to_ascii_lowercase()).collect();
    assert_eq!(unique.len(), families.len());
}

#[test]
fn a_family_is_matched_ignoring_case_and_an_unknown_one_is_not_matched() {
    let m = {
        let mut m = FontManager::new();
        m.register_data(system_font_bytes(), Scope::Process)
            .unwrap();
        m
    };
    let family = m.faces()[0].record.family.clone();
    assert_eq!(
        m.matching(&FontSpec::new().family(&family.to_uppercase())),
        vec![0]
    );
    assert!(m
        .matching(&FontSpec::new().family("No Such Family"))
        .is_empty());
}

#[test]
fn a_name_resolves_as_postscript_name_then_full_name() {
    let mut m = FontManager::new();
    m.register_data(system_font_bytes(), Scope::Process)
        .unwrap();
    let full = m.faces()[0].record.full_name.clone();
    assert_eq!(m.find_name(&full), Some(0));
    assert_eq!(m.find_name("No Such Name"), None);
}

#[test]
fn a_collection_keeps_only_members_that_are_still_registered() {
    let mut m = FontManager::new();
    let ps = m
        .register_data(system_font_bytes(), Scope::Process)
        .unwrap();
    m.define_collection("Mine", &[ps.as_str(), "Gone-Regular"]);
    assert_eq!(m.named_collection("MINE").unwrap().faces, vec![0]);
    assert_eq!(
        m.named_collection("Mine").unwrap().name.as_deref(),
        Some("Mine")
    );
    assert!(m.named_collection("other").is_none());
    m.define_collection("Mine", &[]);
    assert!(m.named_collection("Mine").unwrap().faces.is_empty());
}

#[test]
fn a_collection_built_from_repeated_specs_holds_each_face_once() {
    let mut m = FontManager::new();
    m.register_data(system_font_bytes(), Scope::Process)
        .unwrap();
    let specs = [FontSpec::new(), FontSpec::new()];
    assert_eq!(m.collection_from_specs(&specs).faces, vec![0]);
    assert_eq!(m.all_faces().faces, vec![0]);
    assert!(m
        .collection_matching(&FontSpec::new().family("zz"))
        .faces
        .is_empty());
    assert_eq!(m.all_faces().families(&m).len(), 1);
}

#[test]
fn auto_activation_starts_at_the_default_and_takes_a_new_setting() {
    let mut m = FontManager::new();
    assert_eq!(m.auto_activation(), AutoActivation::Default);
    m.set_auto_activation(AutoActivation::Disabled);
    assert_eq!(m.auto_activation(), AutoActivation::Disabled);
}

#[test]
fn a_font_spec_keeps_the_names_and_sizes_it_is_given() {
    let s = FontSpec::new().family("Foo").size(14.0).bold();
    assert_eq!(s.family.as_deref(), Some("Foo"));
    assert_eq!(s.size, Some(14.0));
    assert!(!s.is_empty());
    assert!(FontSpec::new().is_empty());
    assert_eq!(FontSpec::new().size(-1.0).size, None);
}

// ---- RAW processing parameters ----

fn exposure() -> RawParameter {
    RawParameter::new(
        "exposure",
        "Exposure",
        "Brightness in stops",
        RawRange::Float {
            minimum: -5.0,
            maximum: 5.0,
        },
        RawValue::Float(0.0),
        Some(RawValue::Float(0.0)),
        Some(RawValue::Float(0.5)),
    )
    .unwrap()
}

fn temperature() -> RawParameter {
    RawParameter::new(
        "temperature",
        "Temperature",
        "",
        RawRange::Integer {
            minimum: 2000,
            maximum: 50000,
        },
        RawValue::Integer(5500),
        None,
        None,
    )
    .unwrap()
}

#[test]
fn a_new_parameter_starts_at_its_initial_value_with_its_reference_values() {
    let p = exposure();
    assert_eq!(p.key(), "exposure");
    assert_eq!(p.name(), "Exposure");
    assert_eq!(p.description(), "Brightness in stops");
    assert_eq!(p.value(), RawValue::Float(0.0));
    assert_eq!(p.neutral(), Some(RawValue::Float(0.0)));
    assert_eq!(p.camera(), Some(RawValue::Float(0.5)));
    assert!(p.enabled());
}

#[test]
fn a_parameter_needs_a_key() {
    let r = RawParameter::new(
        "",
        "Name",
        "",
        RawRange::Boolean,
        RawValue::Boolean(false),
        None,
        None,
    );
    assert!(matches!(r, Err(RawError::EmptyKey)));
}

#[test]
fn an_inverted_or_non_finite_range_is_refused() {
    let make = |range| RawParameter::new("k", "K", "", range, RawValue::Float(0.0), None, None);
    assert!(matches!(
        make(RawRange::Integer {
            minimum: 5,
            maximum: 1
        }),
        Err(RawError::InvalidRange)
    ));
    assert!(matches!(
        make(RawRange::Float {
            minimum: f64::NAN,
            maximum: 1.0
        }),
        Err(RawError::InvalidRange)
    ));
    assert!(matches!(
        make(RawRange::Float {
            minimum: 0.0,
            maximum: f64::INFINITY
        }),
        Err(RawError::InvalidRange)
    ));
}

#[test]
fn reference_values_must_fit_the_range_and_its_kind() {
    let range = RawRange::Float {
        minimum: 0.0,
        maximum: 5.0,
    };
    let initial = RawValue::Float(1.0);
    assert!(matches!(
        RawParameter::new("k", "K", "", range, RawValue::Float(9.0), None, None),
        Err(RawError::OutOfRange)
    ));
    assert!(matches!(
        RawParameter::new(
            "k",
            "K",
            "",
            range,
            initial,
            Some(RawValue::Integer(1)),
            None
        ),
        Err(RawError::TypeMismatch)
    ));
    assert!(matches!(
        RawParameter::new(
            "k",
            "K",
            "",
            range,
            initial,
            None,
            Some(RawValue::Float(f64::NAN))
        ),
        Err(RawError::NotFinite)
    ));
    assert!(matches!(
        RawParameter::new(
            "k",
            "K",
            "",
            RawRange::Boolean,
            RawValue::Integer(1),
            None,
            None
        ),
        Err(RawError::TypeMismatch)
    ));
}

#[test]
fn a_set_value_is_clamped_into_the_range_and_the_stored_value_is_returned() {
    let mut p = exposure();
    assert_eq!(p.set(RawValue::Float(9.0)), Ok(RawValue::Float(5.0)));
    assert_eq!(p.value(), RawValue::Float(5.0));
    assert_eq!(p.set(RawValue::Float(-9.0)), Ok(RawValue::Float(-5.0)));
    let mut t = temperature();
    assert_eq!(t.set(RawValue::Integer(100)), Ok(RawValue::Integer(2000)));
    assert_eq!(
        t.set(RawValue::Integer(99999)),
        Ok(RawValue::Integer(50000))
    );
}

#[test]
fn a_non_finite_float_is_refused_and_the_value_is_kept() {
    let mut p = exposure();
    assert!(matches!(
        p.set(RawValue::Float(f64::NAN)),
        Err(RawError::NotFinite)
    ));
    assert_eq!(p.value(), RawValue::Float(0.0));
}

#[test]
fn a_value_of_the_wrong_kind_is_refused_and_the_value_is_kept() {
    let mut p = exposure();
    assert!(matches!(
        p.set(RawValue::Integer(1)),
        Err(RawError::TypeMismatch)
    ));
    assert_eq!(p.value(), RawValue::Float(0.0));
}

#[test]
fn a_boolean_parameter_stores_its_value_and_resets() {
    let mut p = RawParameter::new(
        "nr",
        "Noise Reduction",
        "",
        RawRange::Boolean,
        RawValue::Boolean(false),
        None,
        None,
    )
    .unwrap();
    assert_eq!(p.set(RawValue::Boolean(true)), Ok(RawValue::Boolean(true)));
    assert_eq!(p.value(), RawValue::Boolean(true));
    p.reset();
    assert_eq!(p.value(), RawValue::Boolean(false));
}

#[test]
fn reset_returns_to_the_initial_value_and_keeps_the_reference_values() {
    let mut p = exposure();
    p.set(RawValue::Float(3.0)).unwrap();
    p.reset();
    assert_eq!(p.value(), RawValue::Float(0.0));
    assert_eq!(p.neutral(), Some(RawValue::Float(0.0)));
    assert_eq!(p.camera(), Some(RawValue::Float(0.5)));
}

#[test]
fn a_parameter_can_be_switched_off_and_on_again() {
    let mut p = exposure();
    p.set_enabled(false);
    assert!(!p.enabled());
    p.set_enabled(true);
    assert!(p.enabled());
}

#[test]
fn a_parameter_set_refuses_a_duplicate_key() {
    let mut set = RawProcessingParameters::new();
    assert!(set.is_empty());
    set.add(exposure()).unwrap();
    assert!(matches!(set.add(exposure()), Err(RawError::DuplicateKey(k)) if k == "exposure"));
    assert_eq!(set.len(), 1);
}

#[test]
fn a_parameter_set_keeps_keys_in_the_order_they_were_added() {
    let mut set = RawProcessingParameters::new();
    set.add(exposure()).unwrap();
    set.add(temperature()).unwrap();
    assert_eq!(set.keys(), vec!["exposure", "temperature"]);
    assert!(set.get("temperature").is_some());
    assert!(set.get("tint").is_none());
}

#[test]
fn a_parameter_set_routes_values_by_key_and_clamps_them() {
    let mut set = RawProcessingParameters::new();
    set.add(exposure()).unwrap();
    set.add(temperature()).unwrap();
    assert_eq!(
        set.set("exposure", RawValue::Float(2.5)),
        Ok(RawValue::Float(2.5))
    );
    assert_eq!(set.get("exposure").unwrap().value(), RawValue::Float(2.5));
    assert_eq!(
        set.set("temperature", RawValue::Integer(100000)),
        Ok(RawValue::Integer(50000))
    );
    assert!(
        matches!(set.set("nope", RawValue::Float(1.0)), Err(RawError::UnknownKey(k)) if k == "nope")
    );
}

#[test]
fn reset_all_returns_every_parameter_to_its_initial_value() {
    let mut set = RawProcessingParameters::new();
    set.add(exposure()).unwrap();
    set.add(temperature()).unwrap();
    set.set("exposure", RawValue::Float(2.0)).unwrap();
    set.set("temperature", RawValue::Integer(9000)).unwrap();
    set.reset_all();
    assert_eq!(set.get("exposure").unwrap().value(), RawValue::Float(0.0));
    assert_eq!(
        set.get("temperature").unwrap().value(),
        RawValue::Integer(5500)
    );
}

#[test]
fn enabling_a_parameter_routes_by_key() {
    let mut set = RawProcessingParameters::new();
    set.add(exposure()).unwrap();
    assert_eq!(set.set_enabled("exposure", false), Ok(()));
    assert!(!set.get("exposure").unwrap().enabled());
    assert!(matches!(
        set.set_enabled("nope", true),
        Err(RawError::UnknownKey(_))
    ));
}

#[test]
fn raw_errors_read_as_plain_sentences() {
    assert_eq!(
        RawError::UnknownKey("exposure".into()).to_string(),
        "no parameter exposure"
    );
    assert_eq!(
        RawError::InvalidRange.to_string(),
        "parameter range is inverted or not finite"
    );
}
