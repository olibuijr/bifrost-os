//! Surface materials for 3D objects (`NativeMaterial`, `NativeMaterialProperty`, `NativeMaterialSemantic`,
//! `NativeShadingModel`). A material is an ordered list of named properties. Each property says what its value is used
//! for (its semantic) and holds one value. Lookups by name or by semantic fall back to a base material, so a material
//! can override a few properties of another. This module stores the values only; a texture value is a path that the
//! caller resolves. Evaluating light on a surface is in `shading.rs`.

use crate::linalg::Vector3;

/// What a property's value is used for when a surface is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialSemantic {
    None,
    BaseColor,
    Subsurface,
    Metallic,
    Specular,
    SpecularExponent,
    SpecularTint,
    Roughness,
    Anisotropic,
    AnisotropicRotation,
    Sheen,
    SheenTint,
    Clearcoat,
    ClearcoatGloss,
    Emission,
    Bump,
    Opacity,
    InterfaceIndexOfRefraction,
    MaterialIndexOfRefraction,
    ObjectSpaceNormal,
    TangentSpaceNormal,
    Displacement,
    DisplacementScale,
    AmbientOcclusion,
    AmbientOcclusionScale,
    UserDefined,
}

/// The value a property holds. A texture is referenced by its path.
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialValue {
    Text(String),
    Path(String),
    Float(f32),
    Float2([f32; 2]),
    Float3(Vector3),
    Float4([f32; 4]),
    /// Red, green, blue and alpha, each 0..=1.
    Color([f32; 4]),
}

/// One named property of a material (`NativeMaterialProperty`).
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialProperty {
    pub name: String,
    pub semantic: MaterialSemantic,
    pub value: MaterialValue,
}

impl MaterialProperty {
    pub fn new(name: &str, semantic: MaterialSemantic, value: MaterialValue) -> Self {
        MaterialProperty { name: name.to_string(), semantic, value }
    }
}

/// The lighting model a material is written for: the basic one, or the physically based one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingModel {
    Basic,
    PhysicallyPlausible,
}

/// A named material with its own properties and an optional base (`NativeMaterial`).
#[derive(Clone, Debug, PartialEq)]
pub struct AssetMaterial {
    pub name: String,
    pub shading: ShadingModel,
    /// A material whose properties this one inherits when it has no property of the same name.
    pub base: Option<Box<AssetMaterial>>,
    properties: Vec<MaterialProperty>,
}

impl AssetMaterial {
    pub fn new(name: &str, shading: ShadingModel) -> Self {
        AssetMaterial { name: name.to_string(), shading, base: None, properties: Vec::new() }
    }

    /// The properties set on this material itself, in the order they were first added.
    pub fn properties(&self) -> &[MaterialProperty] {
        &self.properties
    }

    /// Number of properties set on this material itself.
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Adds the property, or replaces the one with the same name in place.
    pub fn set_property(&mut self, property: MaterialProperty) {
        match self.properties.iter().position(|p| p.name == property.name) {
            Some(i) => self.properties[i] = property,
            None => self.properties.push(property),
        }
    }

    /// Removes the property with this name from this material and returns it. A base property is not touched.
    pub fn remove(&mut self, name: &str) -> Option<MaterialProperty> {
        let i = self.properties.iter().position(|p| p.name == name)?;
        Some(self.properties.remove(i))
    }

    pub fn remove_all(&mut self) {
        self.properties.clear();
    }

    /// The property with this name: this material's own first, then the base's.
    pub fn property_named(&self, name: &str) -> Option<&MaterialProperty> {
        self.properties
            .iter()
            .find(|p| p.name == name)
            .or_else(|| self.base.as_ref().and_then(|b| b.property_named(name)))
    }

    /// The first property with this semantic: this material's own, then the base's.
    pub fn property_with(&self, semantic: MaterialSemantic) -> Option<&MaterialProperty> {
        self.properties_with(semantic).into_iter().next()
    }

    /// Every property with this semantic. Own properties come first, then base properties whose name this material
    /// does not set itself.
    pub fn properties_with(&self, semantic: MaterialSemantic) -> Vec<&MaterialProperty> {
        let mut out: Vec<&MaterialProperty> = self.properties.iter().filter(|p| p.semantic == semantic).collect();
        if let Some(base) = &self.base {
            for p in base.properties_with(semantic) {
                if !self.properties.iter().any(|own| own.name == p.name) {
                    out.push(p);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, semantic: MaterialSemantic, v: f32) -> MaterialProperty {
        MaterialProperty::new(name, semantic, MaterialValue::Float(v))
    }

    #[test]
    fn setting_a_name_again_replaces_it_in_place() {
        let mut m = AssetMaterial::new("paint", ShadingModel::PhysicallyPlausible);
        m.set_property(f("roughness", MaterialSemantic::Roughness, 0.5));
        m.set_property(f("metal", MaterialSemantic::Metallic, 0.0));
        m.set_property(f("roughness", MaterialSemantic::Roughness, 0.8));
        assert_eq!(m.len(), 2);
        assert_eq!(m.properties()[0].value, MaterialValue::Float(0.8));
        assert_eq!(m.properties()[1].name, "metal");
    }

    #[test]
    fn lookup_by_name_and_by_semantic() {
        let mut m = AssetMaterial::new("wood", ShadingModel::Basic);
        m.set_property(MaterialProperty::new(
            "tint",
            MaterialSemantic::BaseColor,
            MaterialValue::Color([0.5, 0.3, 0.1, 1.0]),
        ));
        assert_eq!(m.property_named("tint").map(|p| p.semantic), Some(MaterialSemantic::BaseColor));
        assert!(m.property_named("missing").is_none());
        assert!(m.property_with(MaterialSemantic::BaseColor).is_some());
        assert!(m.property_with(MaterialSemantic::Emission).is_none());
    }

    #[test]
    fn semantic_lookup_falls_back_to_the_base() {
        let mut base = AssetMaterial::new("base", ShadingModel::Basic);
        base.set_property(f("gloss", MaterialSemantic::Roughness, 0.3));
        let mut child = AssetMaterial::new("child", ShadingModel::Basic);
        child.base = Some(Box::new(base));
        assert_eq!(child.property_with(MaterialSemantic::Roughness).map(|p| p.name.as_str()), Some("gloss"));
        assert!(child.is_empty());
    }

    #[test]
    fn own_property_shadows_the_base_of_the_same_name() {
        let mut base = AssetMaterial::new("base", ShadingModel::Basic);
        base.set_property(f("roughness", MaterialSemantic::Roughness, 0.3));
        base.set_property(f("opacity", MaterialSemantic::Opacity, 1.0));
        let mut child = AssetMaterial::new("child", ShadingModel::Basic);
        child.base = Some(Box::new(base));
        child.set_property(f("roughness", MaterialSemantic::Roughness, 0.9));
        assert_eq!(child.property_named("roughness").unwrap().value, MaterialValue::Float(0.9));
        assert_eq!(child.property_named("opacity").unwrap().value, MaterialValue::Float(1.0));
        // The base's roughness is shadowed, so only the child's roughness is returned.
        assert_eq!(child.properties_with(MaterialSemantic::Roughness).len(), 1);
        assert_eq!(child.len(), 1);
    }

    #[test]
    fn same_semantic_lists_own_properties_before_base_ones() {
        let mut base = AssetMaterial::new("base", ShadingModel::Basic);
        base.set_property(f("halo", MaterialSemantic::Emission, 0.2));
        let mut child = AssetMaterial::new("child", ShadingModel::Basic);
        child.base = Some(Box::new(base));
        child.set_property(f("glow", MaterialSemantic::Emission, 0.7));
        child.set_property(f("rim", MaterialSemantic::Emission, 0.4));
        let names: Vec<String> = child.properties_with(MaterialSemantic::Emission).iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["glow".to_string(), "rim".to_string(), "halo".to_string()]);
        assert_eq!(child.property_with(MaterialSemantic::Emission).unwrap().name, "glow");
    }

    #[test]
    fn remove_takes_only_the_own_property() {
        let mut m = AssetMaterial::new("m", ShadingModel::Basic);
        m.set_property(f("a", MaterialSemantic::Opacity, 0.5));
        m.set_property(f("b", MaterialSemantic::Bump, 0.1));
        let removed = m.remove("a");
        assert_eq!(removed.map(|p| p.name), Some("a".to_string()));
        assert_eq!(m.len(), 1);
        assert!(m.remove("a").is_none());
        m.remove_all();
        assert!(m.is_empty());
    }
}
