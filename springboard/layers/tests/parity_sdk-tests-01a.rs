//! Parity sdk-tests-01a: headless tests for the layer material model. A layer holds a material that the renderer blurs
//! behind it; the model keeps the blur radius, tint and saturation the app asked for. No test composes a frame.

use bifrost_layers::geometry::Color;
use bifrost_layers::layer::{Layer, LayerId, Material};

#[test]
fn a_new_layer_has_no_material() {
    let layer = Layer::new(LayerId(7));
    assert_eq!(layer.material, Material::None);
    assert_eq!(Material::default(), Material::None);
}

#[test]
fn a_glass_material_keeps_its_blur_tint_and_saturation() {
    let mut layer = Layer::new(LayerId(8));
    let tint = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.25 };
    layer.material = Material::Glass { blur: 20.0, tint, saturation: 1.8 };
    match layer.material {
        Material::Glass { blur, tint: kept, saturation } => {
            assert_eq!(blur, 20.0);
            assert_eq!(kept, tint);
            assert_eq!(saturation, 1.8);
        }
        Material::None => panic!("the glass material was not kept"),
    }
    assert_ne!(layer.material, Material::None);
}

#[test]
fn glass_materials_compare_by_all_their_parameters() {
    let clear = Color::CLEAR;
    let a = Material::Glass { blur: 10.0, tint: clear, saturation: 1.0 };
    let b = Material::Glass { blur: 12.0, tint: clear, saturation: 1.0 };
    assert_ne!(a, b, "a different blur radius is a different material");
    assert_eq!(a, Material::Glass { blur: 10.0, tint: clear, saturation: 1.0 });
}
