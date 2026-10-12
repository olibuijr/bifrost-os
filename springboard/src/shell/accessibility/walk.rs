//! The screen reader's vocabulary: how a walked element is spoken, which rotor it belongs to, and how the menu bar's elements
//! are built. Pure and std-only, so it is tested on the host; the `Shell` glue that walks the desktop lives in `reader`.
//!
//! The rotors are the shared model `bifrost_ui::ax::STANDARD_ROTORS` (Controls, Fields, Lists, Menu items, Text, Progress),
//! with "All" in front: rotor 0 walks every enabled element, rotor `i > 0` walks the roles of `STANDARD_ROTORS[i - 1]`.
//! A heading or link rotor needs roles the tree does not carry yet (`bifrost_ui::ax::Role` has no heading or link), so it is
//! not offered.

use bifrost_ui::ax::{Element, STANDARD_ROTORS};
use bifrost_ui::Rect;

/// The handle of a Settings row: the reader presses it with a tap at its centre.
pub const SETTINGS: &str = "settings";
/// The handle of a menu bar title: pressing it opens the menu.
pub const BAR: &str = "bar";
/// The handle of a row of the open menu: pressing it runs the row's action.
pub const BAR_ROW: &str = "bar-row";

/// The word a role is spoken with, before the name: `button, Close`. An empty word means the name alone is spoken.
pub fn spoken_kind(role: &str) -> &'static str {
    match role {
        "page" | "button" => "button",
        "toggle" => "switch",
        "slider" => "slider",
        "choice" => "choice",
        "checkbox" => "checkbox",
        "field" => "text field",
        "segmented-control" => "segmented control",
        "list" => "list",
        "menu" => "menu",
        "item" | "menu-item" => "menu item",
        "window" => "window",
        "app" => "app",
        "progress-indicator" => "progress indicator",
        "alert" => "alert",
        _ => "",
    }
}

/// What a listener hears for an element: its kind, its name, its value, then "unavailable" when it cannot be used.
/// `button, Close`, `switch, Bold Text, on`, `Message, Hello` (a text row has no kind word).
pub fn speech(e: &Element) -> String {
    let mut parts: Vec<String> = Vec::new();
    let kind = spoken_kind(&e.role);
    if !kind.is_empty() {
        parts.push(kind.to_string());
    }
    parts.push(e.name.clone());
    if !e.value.is_empty() {
        parts.push(e.value.clone());
    }
    if !e.enabled {
        parts.push("unavailable".to_string());
    }
    parts.join(", ")
}

/// The role a Settings or menu element is matched with by a rotor. Settings rows use their own role words (`page`, `choice`),
/// which the standard rotor roles do not name; a Settings button is a button, a choice row is a list row.
pub fn rotor_role(role: &str) -> &str {
    match role {
        "page" => "button",
        "choice" => "row",
        "item" => "menu-item",
        "menu" => "menu-item",
        other => other,
    }
}

/// How many rotors a user cycles through: "All" and each standard rotor.
pub fn rotor_count() -> usize {
    STANDARD_ROTORS.len() + 1
}

/// The name a rotor is announced with when the user picks it. Rotor indices wrap.
pub fn rotor_name(rotor: usize) -> &'static str {
    let i = rotor % rotor_count();
    if i == 0 {
        "All"
    } else {
        STANDARD_ROTORS[i - 1].name
    }
}

/// Whether an element is one the rotor walks. Rotor 0 walks every element.
pub fn in_rotor(e: &Element, rotor: usize) -> bool {
    let i = rotor % rotor_count();
    if i == 0 {
        return true;
    }
    let role = rotor_role(&e.role);
    STANDARD_ROTORS[i - 1].roles.iter().any(|r| *r == role)
}

/// A menu bar title as an element. `slug` is the title's address part (`file`).
pub fn menu_title(slug: &str, name: &str, bounds: Rect) -> Element {
    Element {
        address: format!("os/menubar/{slug}"),
        name: name.to_string(),
        role: "menu".to_string(),
        value: String::new(),
        enabled: true,
        bounds: Some(bounds),
        handle: BAR.to_string(),
    }
}

/// A row of the open menu as an element. `index` counts the rows without separators.
pub fn menu_row(index: usize, text: &str, enabled: bool, bounds: Rect) -> Element {
    Element {
        address: format!("os/menubar-row/{index}"),
        name: text.to_string(),
        role: "menu-item".to_string(),
        value: String::new(),
        enabled,
        bounds: Some(bounds),
        handle: BAR_ROW.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(role: &str, name: &str, value: &str, enabled: bool) -> Element {
        Element { address: format!("a/{name}"), name: name.into(), role: role.into(), value: value.into(), enabled, bounds: None, handle: String::new() }
    }

    #[test]
    fn speech_puts_the_kind_before_the_name() {
        assert_eq!(speech(&el("button", "Close", "", true)), "button, Close");
        assert_eq!(speech(&el("page", "Display Zoom", "Standard", true)), "button, Display Zoom, Standard");
        assert_eq!(speech(&el("toggle", "Bold Text", "on", true)), "switch, Bold Text, on");
        assert_eq!(speech(&el("text", "Message", "Hello", true)), "Message, Hello");
        assert_eq!(speech(&el("menu", "File", "", true)), "menu, File");
        assert_eq!(speech(&el("item", "Save", "", false)), "menu item, Save, unavailable");
    }

    #[test]
    fn rotors_wrap_and_start_with_all() {
        assert_eq!(rotor_count(), 7);
        assert_eq!(rotor_name(0), "All");
        assert_eq!(rotor_name(1), "Controls");
        assert_eq!(rotor_name(5), "Text");
        assert_eq!(rotor_name(6), "Progress");
        assert_eq!(rotor_name(7), "All", "the index wraps after the last rotor");
    }

    #[test]
    fn a_rotor_walks_only_its_roles() {
        let button = el("page", "Wi-Fi", "", true);
        let text = el("text", "Message", "", true);
        assert!(in_rotor(&button, 0) && in_rotor(&text, 0), "All walks everything");
        assert!(in_rotor(&button, 1), "a Settings button is in Controls");
        assert!(!in_rotor(&text, 1), "static text is not in Controls");
        assert!(in_rotor(&text, 5), "static text is in Text");
        assert!(!in_rotor(&button, 5));
        assert!(!in_rotor(&el("choice", "Off", "", true), 3), "a choice row is a list row, not a menu item");
        assert!(in_rotor(&el("choice", "Off", "", true), 2), "a choice row is in Lists");
    }

    #[test]
    fn rotor_roles_map_settings_words() {
        assert_eq!(rotor_role("page"), "button");
        assert_eq!(rotor_role("choice"), "row");
        assert_eq!(rotor_role("menu"), "menu-item");
        assert_eq!(rotor_role("slider"), "slider");
    }

    #[test]
    fn menu_elements_carry_their_handles() {
        let b = Rect { x: 10.0, y: 0.0, w: 40.0, h: 28.0 };
        let title = menu_title("file", "File", b);
        assert_eq!((title.address.as_str(), title.handle.as_str(), title.role.as_str()), ("os/menubar/file", BAR, "menu"));
        let row = menu_row(2, "Save", false, b);
        assert_eq!((row.address.as_str(), row.handle.as_str(), row.enabled), ("os/menubar-row/2", BAR_ROW, false));
        assert_eq!(speech(&title), "menu, File");
    }
}
