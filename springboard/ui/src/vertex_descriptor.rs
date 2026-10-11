//! Vertex layout: attributes, buffer strides and the packing rules that tie them together.
//! This is the pure model behind a mesh's vertex description. It owns no vertex data: `mesh.rs` holds the byte
//! ranges, and a renderer reads this layout to find each attribute inside a vertex buffer.

/// Scalar type of one vertex component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarKind {
    Float32,
    Float16,
    Int32,
    UInt32,
    Int16,
    UInt16,
    Int8,
    UInt8,
}

impl ScalarKind {
    /// Size of one component in bytes.
    pub fn size(self) -> usize {
        match self {
            ScalarKind::Float32 | ScalarKind::Int32 | ScalarKind::UInt32 => 4,
            ScalarKind::Float16 | ScalarKind::Int16 | ScalarKind::UInt16 => 2,
            ScalarKind::Int8 | ScalarKind::UInt8 => 1,
        }
    }
}

/// Format of one attribute: a scalar kind, 1 to 4 components, and (for integers) whether the value is read
/// as a normalized float in `0..1` or `-1..1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexFormat {
    pub kind: ScalarKind,
    pub components: u8,
    pub normalized: bool,
}

impl VertexFormat {
    /// Builds a format, or `None` when the component count is outside 1..=4 or `normalized` is set on a float.
    pub fn new(kind: ScalarKind, components: u8, normalized: bool) -> Option<Self> {
        if !(1..=4).contains(&components) {
            return None;
        }
        if normalized && matches!(kind, ScalarKind::Float32 | ScalarKind::Float16) {
            return None;
        }
        Some(VertexFormat { kind, components, normalized })
    }

    /// Bytes one value of this format occupies.
    pub fn size(self) -> usize {
        self.kind.size() * self.components as usize
    }
}

/// One named attribute: where it sits inside the vertex (`offset`) and which buffer slot carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexAttribute {
    pub name: String,
    pub format: VertexFormat,
    pub offset: usize,
    pub buffer_index: usize,
}

/// Stride of one vertex buffer slot: bytes from one vertex to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexBufferLayout {
    pub stride: usize,
}

/// Why a descriptor does not describe a usable vertex buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VertexDescriptorError {
    /// An attribute uses a buffer slot whose stride is zero.
    ZeroStride { buffer_index: usize },
    /// An attribute ends past the stride of its slot, so it would read into the next vertex.
    AttributeOutsideStride { name: String, end: usize, stride: usize },
}

/// The vertex description: attributes in insertion order plus one layout per buffer slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VertexDescriptor {
    attributes: Vec<VertexAttribute>,
    layouts: Vec<VertexBufferLayout>,
}

impl VertexDescriptor {
    /// An empty descriptor with no attributes and no buffer slots.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attributes in the order they were added.
    pub fn attributes(&self) -> &[VertexAttribute] {
        &self.attributes
    }

    /// The attribute with this name, if any.
    pub fn attribute_named(&self, name: &str) -> Option<&VertexAttribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// Adds an attribute, or replaces the existing one with the same name in place. The buffer slot it names is
    /// created with stride zero if it does not exist yet.
    pub fn add_or_replace_attribute(&mut self, attribute: VertexAttribute) {
        self.ensure_layout(attribute.buffer_index);
        match self.attributes.iter().position(|a| a.name == attribute.name) {
            Some(i) => self.attributes[i] = attribute,
            None => self.attributes.push(attribute),
        }
    }

    /// Removes the attribute with this name. Returns `false` when there was none.
    pub fn remove_attribute_named(&mut self, name: &str) -> bool {
        match self.attributes.iter().position(|a| a.name == name) {
            Some(i) => {
                self.attributes.remove(i);
                true
            }
            None => false,
        }
    }

    /// One layout per buffer slot, indexed by slot number.
    pub fn layouts(&self) -> &[VertexBufferLayout] {
        &self.layouts
    }

    /// Sets the stride of a slot, creating intermediate slots with stride zero as needed.
    pub fn set_stride(&mut self, buffer_index: usize, stride: usize) {
        self.ensure_layout(buffer_index);
        self.layouts[buffer_index].stride = stride;
    }

    /// Reassigns attribute offsets so that, within each slot, attributes are packed back to back in insertion order
    /// starting at zero. Attributes in other slots are unaffected.
    pub fn set_packed_offsets(&mut self) {
        let mut cursors = vec![0usize; self.layouts.len()];
        for attribute in self.attributes.iter_mut() {
            let cursor = &mut cursors[attribute.buffer_index];
            attribute.offset = *cursor;
            *cursor += attribute.format.size();
        }
    }

    /// Sets each slot's stride to the sum of its attribute sizes, so the slot holds no padding.
    pub fn set_packed_strides(&mut self) {
        for index in 0..self.layouts.len() {
            let stride: usize = self
                .attributes
                .iter()
                .filter(|a| a.buffer_index == index)
                .map(|a| a.format.size())
                .sum();
            self.layouts[index].stride = stride;
        }
    }

    /// Bytes needed to hold `vertex_count` vertices in one slot.
    pub fn buffer_size(&self, buffer_index: usize, vertex_count: usize) -> usize {
        let stride = self.layouts.get(buffer_index).map_or(0, |l| l.stride);
        stride * vertex_count
    }

    /// Checks that every attribute has a usable slot and fits inside its stride.
    pub fn validate(&self) -> Result<(), VertexDescriptorError> {
        for attribute in &self.attributes {
            let stride = self.layouts.get(attribute.buffer_index).map_or(0, |l| l.stride);
            if stride == 0 {
                return Err(VertexDescriptorError::ZeroStride { buffer_index: attribute.buffer_index });
            }
            let end = attribute.offset + attribute.format.size();
            if end > stride {
                return Err(VertexDescriptorError::AttributeOutsideStride {
                    name: attribute.name.clone(),
                    end,
                    stride,
                });
            }
        }
        Ok(())
    }

    fn ensure_layout(&mut self, buffer_index: usize) {
        if self.layouts.len() <= buffer_index {
            self.layouts.resize(buffer_index + 1, VertexBufferLayout { stride: 0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(name: &str, kind: ScalarKind, components: u8, buffer_index: usize) -> VertexAttribute {
        VertexAttribute {
            name: name.into(),
            format: VertexFormat::new(kind, components, false).unwrap(),
            offset: 0,
            buffer_index,
        }
    }

    #[test]
    fn formats_reject_bad_component_counts_and_normalized_floats() {
        assert!(VertexFormat::new(ScalarKind::Float32, 0, false).is_none());
        assert!(VertexFormat::new(ScalarKind::Float32, 5, false).is_none());
        assert!(VertexFormat::new(ScalarKind::Float32, 3, true).is_none());
        assert!(VertexFormat::new(ScalarKind::UInt8, 4, true).is_some());
        assert_eq!(VertexFormat::new(ScalarKind::Float16, 3, false).unwrap().size(), 6);
    }

    #[test]
    fn packed_offsets_and_strides_follow_insertion_order_per_slot() {
        let mut d = VertexDescriptor::new();
        d.add_or_replace_attribute(attr("position", ScalarKind::Float32, 3, 0));
        d.add_or_replace_attribute(attr("uv", ScalarKind::Float32, 2, 0));
        d.add_or_replace_attribute(attr("color", ScalarKind::UInt8, 4, 1));
        d.set_packed_offsets();
        d.set_packed_strides();

        assert_eq!(d.attribute_named("position").unwrap().offset, 0);
        assert_eq!(d.attribute_named("uv").unwrap().offset, 12);
        assert_eq!(d.attribute_named("color").unwrap().offset, 0);
        assert_eq!(d.layouts()[0].stride, 20);
        assert_eq!(d.layouts()[1].stride, 4);
        assert!(d.validate().is_ok());
        assert_eq!(d.buffer_size(0, 10), 200);
    }

    #[test]
    fn replacing_by_name_keeps_position_and_creates_the_slot() {
        let mut d = VertexDescriptor::new();
        d.add_or_replace_attribute(attr("a", ScalarKind::Float32, 1, 0));
        d.add_or_replace_attribute(attr("b", ScalarKind::Float32, 1, 0));
        d.add_or_replace_attribute(attr("a", ScalarKind::Int16, 2, 2));

        let names: Vec<&str> = d.attributes().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(d.attribute_named("a").unwrap().buffer_index, 2);
        assert_eq!(d.layouts().len(), 3);
        assert_eq!(d.layouts()[1].stride, 0);
    }

    #[test]
    fn remove_reports_whether_the_name_existed() {
        let mut d = VertexDescriptor::new();
        d.add_or_replace_attribute(attr("normal", ScalarKind::Float32, 3, 0));
        assert!(d.remove_attribute_named("normal"));
        assert!(!d.remove_attribute_named("normal"));
        assert!(d.attributes().is_empty());
    }

    #[test]
    fn validation_rejects_zero_strides_and_attributes_past_the_stride() {
        let mut d = VertexDescriptor::new();
        d.add_or_replace_attribute(attr("position", ScalarKind::Float32, 3, 0));
        assert_eq!(d.validate(), Err(VertexDescriptorError::ZeroStride { buffer_index: 0 }));

        d.set_stride(0, 8);
        assert_eq!(
            d.validate(),
            Err(VertexDescriptorError::AttributeOutsideStride { name: "position".into(), end: 12, stride: 8 })
        );

        d.set_stride(0, 12);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn set_stride_grows_the_slot_list_without_touching_other_slots() {
        let mut d = VertexDescriptor::new();
        d.set_stride(2, 16);
        assert_eq!(d.layouts().len(), 3);
        assert_eq!(d.layouts()[0].stride, 0);
        assert_eq!(d.layouts()[2], VertexBufferLayout { stride: 16 });
        assert_eq!(d.buffer_size(5, 4), 0);
    }
}
