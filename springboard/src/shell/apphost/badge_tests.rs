use super::clean_badge;

#[test]
fn badges_are_short_single_line_labels() {
    assert_eq!(clean_badge("12"), "12");
    assert_eq!(clean_badge("  7 \n"), "7");
    assert_eq!(clean_badge("99+"), "99+");
    assert_eq!(clean_badge("123456"), "1234");
    assert_eq!(clean_badge("a\u{0}b\tc"), "abc");
    assert_eq!(clean_badge(""), "");
    assert_eq!(clean_badge("éééééé"), "éééé", "cut by characters, not bytes");
}

#[test]
fn a_failed_launch_names_the_app_and_says_to_try_again() {
    let (title, body) = crate::shell::apphost::launch_failure_text("  Notes ");
    assert_eq!(title, "Notes could not be opened");
    assert!(body.contains("Try opening it again"));
    assert_eq!(crate::shell::apphost::launch_failure_text("").0, "The app could not be opened");
}

#[test]
fn a_refused_permission_is_named_in_the_alert() {
    let body = crate::shell::apphost::unsupported_permission_text("teleport");
    let (title, shown) = crate::shell::apphost::launch_failure_text_for("Notes", Some(&body));
    assert_eq!(title, "Notes could not be opened");
    assert!(shown.contains("\"teleport\"") && !shown.contains("Try opening it again"));
    assert!(crate::shell::apphost::launch_failure_text_for("Notes", Some("  ")).1.contains("Try opening it again"));
}
