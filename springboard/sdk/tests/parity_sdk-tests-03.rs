//! Headless host tests for the print dialog plug-in model (`bifrost_app::print_panel`). They run with no display,
//! printer or network: the plug-in registry is pure data, and every assertion checks a concrete value.
use bifrost_app::print_panel::{PanelError, PanelOption, Placement, PlugIn, PlugIns, PrintPanel, Value, RESERVED_KEYS};

fn sides_and_collate() -> PlugIn {
    let panel = PrintPanel::new("finishing", "Finishing", Placement::PrintDialog)
        .option(PanelOption::choice("sides", "Sides", &["one-sided", "two-sided-long-edge"], 0).unwrap())
        .option(PanelOption::toggle("collate", "Collate", false).unwrap());
    PlugIn::new("finishing-plugin", "Finishing").panel(panel)
}

fn page_gap_plugin() -> PlugIn {
    let panel = PrintPanel::new("layout", "Layout", Placement::PageLayout)
        .option(PanelOption::number("page-gap", "Page gap", 0, 10, 3).unwrap());
    PlugIn::new("layout-plugin", "Layout").panel(panel)
}

#[test]
fn choice_option_starts_at_its_default_index() {
    let o = PanelOption::choice("sides", "Sides", &["one-sided", "two-sided-long-edge"], 1).unwrap();
    assert_eq!(o.value, Value::Choice(1));
    assert_eq!(o.lp_value(), "two-sided-long-edge");
    assert_eq!(o.display(), "two-sided-long-edge");
}

#[test]
fn toggle_option_is_written_as_true_or_false_and_shown_as_on_or_off() {
    let on = PanelOption::toggle("collate", "Collate", true).unwrap();
    assert_eq!(on.lp_value(), "true");
    assert_eq!(on.display(), "On");
    let off = PanelOption::toggle("collate", "Collate", false).unwrap();
    assert_eq!(off.lp_value(), "false");
    assert_eq!(off.display(), "Off");
}

#[test]
fn number_option_accepts_its_bounds_and_refuses_values_outside_them() {
    let top = PanelOption::number("copy-count", "Copies", 1, 10, 10).unwrap();
    assert_eq!(top.lp_value(), "10");
    assert_eq!(top.display(), "10");
    assert_eq!(
        PanelOption::number("copy-count", "Copies", 1, 10, 11),
        Err(PanelError::OutOfRange("copy-count".into()))
    );
    assert_eq!(PanelOption::number("copy-count", "Copies", 5, 1, 3), Err(PanelError::OutOfRange("copy-count".into())));
}

#[test]
fn keys_must_be_short_and_use_only_letters_digits_dash_and_underscore() {
    assert_eq!(PanelOption::toggle("bad key", "x", true), Err(PanelError::BadKey("bad key".into())));
    assert_eq!(PanelOption::toggle("", "x", true), Err(PanelError::BadKey(String::new())));
    let long = "k".repeat(65);
    assert_eq!(PanelOption::toggle(&long, "x", true), Err(PanelError::BadKey(long.clone())));
    let max = "k".repeat(64);
    assert!(PanelOption::toggle(&max, "x", true).is_ok());
    assert!(PanelOption::toggle("page_gap-2", "x", true).is_ok());
}

#[test]
fn choice_refuses_empty_lists_empty_labels_and_out_of_range_defaults() {
    assert_eq!(PanelOption::choice("k", "K", &[], 0), Err(PanelError::Empty("k".into())));
    assert_eq!(PanelOption::choice("k", "K", &["a", ""], 0), Err(PanelError::Empty("k".into())));
    assert_eq!(PanelOption::choice("k", "K", &["a"], 1), Err(PanelError::OutOfRange("k".into())));
}

#[test]
fn reserved_print_info_keys_are_refused_at_registration() {
    assert_eq!(RESERVED_KEYS, ["media", "orientation-requested", "copies", "page-ranges"]);
    for key in RESERVED_KEYS {
        let panel = PrintPanel::new("reserved-panel", "Reserved", Placement::PrintDialog)
            .option(PanelOption::toggle(key, "Reserved", false).unwrap());
        let plugin = PlugIn::new("reserved-plugin", "Reserved").panel(panel);
        assert_eq!(PlugIns::new().register(plugin), Err(PanelError::ReservedKey(key.to_string())));
    }
}

#[test]
fn a_duplicate_key_in_another_plugin_is_refused_and_the_registry_is_unchanged() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    let again = PlugIn::new("second-plugin", "Second").panel(
        PrintPanel::new("second", "Second", Placement::PrintDialog)
            .option(PanelOption::toggle("collate", "Collate again", true).unwrap()),
    );
    assert_eq!(reg.register(again), Err(PanelError::DuplicateKey("collate".into())));
    assert_eq!(reg.plugins().len(), 1);
    assert_eq!(reg.get("collate").unwrap().value, Value::Toggle(false));
}

#[test]
fn duplicate_plugin_ids_and_duplicate_panel_ids_are_refused() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    assert_eq!(reg.register(sides_and_collate()), Err(PanelError::DuplicateId("finishing-plugin".into())));
    let clash = PlugIn::new("other-plugin", "Other").panel(
        PrintPanel::new("finishing", "Clash", Placement::PrintDialog)
            .option(PanelOption::toggle("staple", "Staple", false).unwrap()),
    );
    assert_eq!(reg.register(clash), Err(PanelError::DuplicateId("finishing".into())));
    assert_eq!(reg.plugins().len(), 1);
}

#[test]
fn panels_are_filtered_by_the_dialog_that_hosts_them() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    reg.register(page_gap_plugin()).unwrap();
    let print: Vec<&str> = reg.panels(Placement::PrintDialog).iter().map(|p| p.id.as_str()).collect();
    let layout: Vec<&str> = reg.panels(Placement::PageLayout).iter().map(|p| p.id.as_str()).collect();
    assert_eq!(print, ["finishing"]);
    assert_eq!(layout, ["layout"]);
}

#[test]
fn set_checks_the_kind_and_range_and_changes_nothing_on_refusal() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    reg.register(page_gap_plugin()).unwrap();
    assert_eq!(reg.set("collate", Value::Number(1)), Err(PanelError::KindMismatch("collate".into())));
    assert_eq!(reg.set("sides", Value::Choice(5)), Err(PanelError::OutOfRange("sides".into())));
    assert_eq!(reg.set("page-gap", Value::Number(11)), Err(PanelError::OutOfRange("page-gap".into())));
    assert_eq!(reg.set("nope", Value::Toggle(true)), Err(PanelError::UnknownKey("nope".into())));
    assert_eq!(reg.get("sides").unwrap().value, Value::Choice(0));
    assert_eq!(reg.get("page-gap").unwrap().value, Value::Number(3));
    assert_eq!(reg.get("collate").unwrap().value, Value::Toggle(false));
}

#[test]
fn set_stores_an_accepted_value_in_the_panel_that_owns_the_key() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    reg.register(page_gap_plugin()).unwrap();
    reg.set("collate", Value::Toggle(true)).unwrap();
    reg.set("sides", Value::Choice(1)).unwrap();
    reg.set("page-gap", Value::Number(10)).unwrap();
    assert_eq!(reg.get("collate").unwrap().display(), "On");
    assert_eq!(reg.get("sides").unwrap().lp_value(), "two-sided-long-edge");
    assert_eq!(reg.get("page-gap").unwrap().lp_value(), "10");
}

#[test]
fn job_options_follow_registration_order_and_carry_the_current_values() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    reg.register(page_gap_plugin()).unwrap();
    let pairs = reg.job_options();
    let got: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(got, [("sides", "one-sided"), ("collate", "false"), ("page-gap", "3")]);
    reg.set("sides", Value::Choice(1)).unwrap();
    let pairs = reg.job_options();
    assert_eq!(pairs[0], ("sides".to_string(), "two-sided-long-edge".to_string()));
}

#[test]
fn panels_keep_their_display_order_and_plugins_keep_registration_order() {
    let mut reg = PlugIns::new();
    reg.register(sides_and_collate()).unwrap();
    reg.register(page_gap_plugin()).unwrap();
    let ids: Vec<&str> = reg.plugins().iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["finishing-plugin", "layout-plugin"]);
    let panel = reg.panels(Placement::PrintDialog)[0];
    let keys: Vec<&str> = panel.options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["sides", "collate"]);
    assert!(panel.get("collate").is_some());
    assert!(panel.get("page-gap").is_none());
}
