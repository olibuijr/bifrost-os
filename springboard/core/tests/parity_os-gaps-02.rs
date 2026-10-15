//! Parity os-gaps-02: headless tests for the document link index, the versioned data migration chain, the cache purge
//! plan, the swap growth rule and the ambient light sample. No test touches a file, a device or the clock.

use bifrost_core::ambient_light::{IlluminanceSample, SampleError};
use bifrost_core::cache_purge::{plan_purge, CacheEntry};
use bifrost_core::link_index::{wiki_links, LinkIndex};
use bifrost_core::migrate::{MigrationError, Plan};
use bifrost_core::swap_plan::{growth_bytes, SwapState};

const GIB: u64 = 1 << 30;

#[test]
fn a_document_lists_the_documents_that_link_to_it() {
    let mut index = LinkIndex::new();
    index.set_links("a", ["b", "c"]);
    index.set_links("d", ["b"]);
    assert_eq!(index.backlinks("b"), vec!["a", "d"]);
    assert_eq!(index.backlinks("c"), vec!["a"]);
    assert!(index.backlinks("a").is_empty());
}

#[test]
fn a_document_never_links_to_itself() {
    let mut index = LinkIndex::new();
    index.set_links("a", ["a", "b"]);
    assert_eq!(index.links_from("a"), vec!["b"]);
    assert!(index.backlinks("a").is_empty());
}

#[test]
fn replacing_the_links_of_a_document_drops_the_old_targets() {
    let mut index = LinkIndex::new();
    index.set_links("a", ["b"]);
    index.set_links("a", ["c"]);
    assert!(index.backlinks("b").is_empty());
    assert_eq!(index.backlinks("c"), vec!["a"]);
}

#[test]
fn removing_a_document_removes_its_outgoing_links() {
    let mut index = LinkIndex::new();
    index.set_links("a", ["b"]);
    assert!(index.remove("a"));
    assert!(!index.remove("a"));
    assert!(index.is_empty());
    assert!(index.backlinks("b").is_empty());
}

#[test]
fn wiki_links_are_found_in_order_without_duplicates_or_empty_targets() {
    let text = "See [[Notes]] and [[ Plans ]], again [[Notes]], then [[]] and an open [[Unclosed";
    assert_eq!(wiki_links(text), vec!["Notes".to_string(), "Plans".to_string()]);
}

#[test]
fn a_chain_with_a_step_for_every_older_version_passes_its_check() {
    let plan: Plan<Vec<u32>> = Plan::new(2)
        .step(0, "to one", |mut d: Vec<u32>| {
            d.push(1);
            Ok(d)
        })
        .step(1, "to two", |mut d: Vec<u32>| {
            d.push(2);
            Ok(d)
        });
    assert_eq!(plan.check(), Ok(()));
    assert_eq!(plan.migrate(0, vec![]), Ok(vec![1, 2]));
    assert_eq!(plan.migrate(1, vec![9]), Ok(vec![9, 2]));
}

#[test]
fn data_written_by_a_newer_build_is_refused_rather_than_read() {
    let plan: Plan<Vec<u32>> = Plan::new(2).step(0, "to one", |d| Ok(d)).step(1, "to two", |d| Ok(d));
    assert_eq!(plan.migrate(3, vec![]), Err(MigrationError::NewerThanBuild { found: 3, current: 2 }));
}

fn entry(app: &str, bytes: u64, last_used: u64, running: bool) -> CacheEntry {
    CacheEntry { app: app.to_string(), bytes, last_used, running }
}

#[test]
fn the_least_recently_used_caches_are_purged_first_until_the_target_is_met() {
    let entries = vec![entry("a", 100, 10, false), entry("b", 50, 5, false), entry("c", 200, 1, false)];
    let plan = plan_purge(&entries, 10, 120);
    assert_eq!(plan.victims, vec!["c".to_string()]);
    assert_eq!((plan.reclaimed, plan.shortfall), (200, 0));
}

#[test]
fn a_purge_stops_once_the_shortfall_is_covered() {
    let entries = vec![entry("a", 100, 10, false), entry("b", 50, 5, false)];
    let plan = plan_purge(&entries, 10, 120);
    assert_eq!(plan.victims, vec!["b".to_string(), "a".to_string()]);
    assert_eq!((plan.reclaimed, plan.shortfall), (150, 0));
}

#[test]
fn a_purge_reports_the_shortfall_when_every_cache_is_not_enough() {
    let entries = vec![entry("a", 100, 10, false), entry("b", 50, 5, false)];
    let plan = plan_purge(&entries, 0, 1000);
    assert_eq!(plan.victims.len(), 2);
    assert_eq!((plan.reclaimed, plan.shortfall), (150, 850));
}

#[test]
fn a_running_app_and_an_empty_cache_are_never_purged() {
    let entries = vec![entry("running", 500, 0, true), entry("empty", 0, 0, false), entry("idle", 40, 9, false)];
    let plan = plan_purge(&entries, 0, 100);
    assert_eq!(plan.victims, vec!["idle".to_string()]);
    assert_eq!(plan.shortfall, 60);
}

#[test]
fn no_purge_is_planned_when_free_space_already_meets_the_target() {
    let entries = vec![entry("a", 100, 1, false)];
    let plan = plan_purge(&entries, 200, 200);
    assert!(plan.victims.is_empty());
    assert_eq!((plan.reclaimed, plan.shortfall), (0, 0));
}

#[test]
fn caches_with_equal_last_use_are_purged_by_app_name() {
    let entries = vec![entry("x", 10, 1, false), entry("w", 10, 1, false)];
    assert_eq!(plan_purge(&entries, 0, 10).victims, vec!["w".to_string()]);
}

#[test]
fn swap_grows_by_one_chunk_when_ram_and_swap_are_both_short() {
    let state = SwapState { ram_total: 16 * GIB, ram_available: GIB, swap_total: 0, swap_free: 0 };
    assert_eq!(growth_bytes(state, GIB, 8 * GIB), GIB);
}

#[test]
fn swap_does_not_grow_while_ram_is_comfortably_available() {
    let state = SwapState { ram_total: 16 * GIB, ram_available: 8 * GIB, swap_total: 0, swap_free: 0 };
    assert_eq!(growth_bytes(state, GIB, 8 * GIB), 0);
}

#[test]
fn swap_does_not_grow_while_more_than_a_quarter_of_it_is_free() {
    let state = SwapState { ram_total: 16 * GIB, ram_available: GIB, swap_total: 4 * GIB, swap_free: 2 * GIB };
    assert_eq!(growth_bytes(state, GIB, 8 * GIB), 0);
}

#[test]
fn swap_growth_stops_at_the_cap() {
    let full = SwapState { ram_total: 16 * GIB, ram_available: GIB, swap_total: 8 * GIB, swap_free: 0 };
    assert_eq!(growth_bytes(full, GIB, 8 * GIB), 0);
    let nearly = SwapState { ram_total: 16 * GIB, ram_available: GIB, swap_total: 7 * GIB + GIB / 2, swap_free: 0 };
    assert_eq!(growth_bytes(nearly, GIB, 8 * GIB), GIB / 2);
}

#[test]
fn a_raw_count_is_converted_to_lux_with_its_scale() {
    let sample = IlluminanceSample::from_raw(1000, 250, 0.5).unwrap();
    assert_eq!(sample.timestamp_ms, 1000);
    assert_eq!(sample.lux, 125.0);
}

#[test]
fn sysfs_text_with_a_trailing_newline_is_converted_to_lux() {
    let sample = IlluminanceSample::from_sysfs(5, "120\n", "0.25\n").unwrap();
    assert_eq!(sample.lux, 30.0);
}

#[test]
fn sysfs_text_that_is_not_a_number_is_unreadable() {
    assert_eq!(IlluminanceSample::from_sysfs(5, "abc", "1"), Err(SampleError::Unreadable));
    assert_eq!(IlluminanceSample::from_sysfs(5, "10", "x"), Err(SampleError::Unreadable));
}

#[test]
fn a_negative_or_non_finite_lux_value_is_refused() {
    assert_eq!(IlluminanceSample::new(0, -1.0), Err(SampleError::Negative));
    assert_eq!(IlluminanceSample::new(0, f64::NAN), Err(SampleError::NotFinite));
    assert_eq!(IlluminanceSample::new(0, f64::INFINITY), Err(SampleError::NotFinite));
    assert_eq!(IlluminanceSample::from_raw(0, -3, 1.0), Err(SampleError::Negative));
}

#[test]
fn a_nonpositive_or_non_finite_sensor_scale_is_refused() {
    assert_eq!(IlluminanceSample::from_raw(0, 1, 0.0), Err(SampleError::InvalidScale));
    assert_eq!(IlluminanceSample::from_raw(0, 1, -0.5), Err(SampleError::InvalidScale));
    assert_eq!(IlluminanceSample::from_raw(0, 1, f64::INFINITY), Err(SampleError::InvalidScale));
}
