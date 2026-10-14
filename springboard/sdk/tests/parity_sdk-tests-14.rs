//! Parity sdk-tests-14: headless tests of the app-facing materials in `bifrost_app::widgets`. The material model is checked
//! through the public API only: the material list, its mapping to the shared window-server kinds, and the tints that
//! selection emphasis and window activity change. No test draws a frame or reads a display.

use bifrost_app::widgets::Material;
use bifrost_ui::visual_effect::MaterialKind;

#[test]
fn the_material_list_names_each_window_server_kind_once() {
    assert_eq!(Material::ALL.len(), 14);
    let kinds: Vec<MaterialKind> = Material::ALL.iter().map(|m| m.model_kind()).collect();
    for (i, k) in kinds.iter().enumerate() {
        assert!(!kinds[i + 1..].contains(k), "{k:?} is mapped by two materials");
    }
}

#[test]
fn every_material_maps_back_from_its_window_server_kind() {
    for m in Material::ALL {
        assert_eq!(Material::from_model(m.model_kind()), m, "{m:?} survives a round trip through its kind");
    }
}

#[test]
fn the_heads_up_material_is_the_hud_window_kind() {
    assert_eq!(Material::Hud.model_kind(), MaterialKind::HudWindow);
    assert_eq!(Material::from_model(MaterialKind::HudWindow), Material::Hud);
}

#[test]
fn a_selection_tint_follows_emphasis_and_window_activity() {
    let emphasized = Material::Selection.tint_for(true, true);
    let plain = Material::Selection.tint_for(false, true);
    let inactive = Material::Selection.tint_for(false, false);
    assert_ne!(emphasized, plain, "an emphasized selection uses the full accent");
    assert_ne!(plain, inactive, "an inactive selection uses the neutral grey");
}

#[test]
fn a_surface_material_ignores_emphasis_and_window_activity() {
    for m in [Material::Sidebar, Material::Titlebar, Material::Popover, Material::Hud, Material::Menu, Material::Sheet] {
        assert_eq!(m.tint_for(true, true), m.tint_for(false, true), "{m:?} does not change with emphasis");
        assert_eq!(m.tint_for(false, false), m.tint_for(false, true), "{m:?} does not change with activity");
    }
}

#[test]
fn the_default_tint_is_the_active_unemphasized_tint() {
    for m in Material::ALL {
        assert_eq!(m.tint(), m.tint_for(false, true), "{m:?}");
    }
}
