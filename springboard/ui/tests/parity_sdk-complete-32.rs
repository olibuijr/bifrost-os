//! Parity sdk-complete-32: headless tests for the view environment (`bifrost_ui::environment`): typed reads, scoped
//! settings and the modifier that applies a value to one subtree. Pure host tests; no view is laid out.

use bifrost_ui::environment::{EnvValue, Environment};

#[test]
fn typed_reads_return_the_value_only_for_its_own_kind() {
    let mut env = Environment::new();
    env.set("columns", EnvValue::Int(3));
    env.set("scale", EnvValue::Float(1.5));
    env.set("title", EnvValue::Text("Inbox".into()));
    assert_eq!(env.int("columns"), Some(3));
    assert_eq!(env.float("scale"), Some(1.5));
    assert_eq!(env.text("title"), Some("Inbox"));
    assert_eq!(env.int("scale"), None, "a float is not an integer");
    assert_eq!(env.text("columns"), None);
    assert_eq!(env.float("missing"), None);
}

#[test]
fn scoped_settings_are_gone_when_the_subtree_returns() {
    let mut env = Environment::new();
    env.set("tint", EnvValue::Text("blue".into()));
    let inside = env.scoped(|env| {
        env.set("tint", EnvValue::Text("red".into()));
        env.text("tint").map(str::to_string)
    });
    assert_eq!(inside.as_deref(), Some("red"));
    assert_eq!(env.text("tint"), Some("blue"), "the parent sees its own value again");
    assert_eq!(env.depth(), 1, "the scope was closed");
}

#[test]
fn with_value_applies_to_the_body_and_returns_what_the_body_returns() {
    let mut env = Environment::new();
    assert!(!env.flag("reduce-motion"));
    let seen = env.with_value("reduce-motion", EnvValue::Bool(true), |env| env.flag("reduce-motion"));
    assert!(seen, "the body sees the modifier");
    assert!(!env.flag("reduce-motion"), "the modifier does not outlive the body");
}

#[test]
fn nested_modifiers_unwind_in_order() {
    let mut env = Environment::new();
    env.with_value("columns", EnvValue::Int(2), |env| {
        assert_eq!(env.int("columns"), Some(2));
        env.with_value("columns", EnvValue::Int(5), |env| assert_eq!(env.int("columns"), Some(5)));
        assert_eq!(env.int("columns"), Some(2), "the outer modifier shows again");
    });
    assert_eq!(env.int("columns"), None);
    assert_eq!(env.depth(), 1);
}
