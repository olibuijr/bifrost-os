//! Mesh containers: vertex layout, vertex buffers, indexed submeshes and bounds, plus a
//! CPU-side buffer allocator with a byte budget.
//!
//! This is the data half of model handling. A mesh owns vertex buffers described by a
//! [`VertexLayout`], and each [`Submesh`] draws its own index buffer against those vertices.
//! Nothing here touches a device: a GPU upload takes the bytes as they are.

/// Element format of one vertex attribute. Little-endian, tightly packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Float,
    Float2,
    Float3,
    Float4,
    /// Four unsigned bytes read as values in 0..=1 (colours, packed normals).
    UnormByte4,
    /// One unsigned 16-bit value.
    UShort,
}

impl Format {
    pub fn size(self) -> usize {
        match self {
            Format::Float => 4,
            Format::Float2 => 8,
            Format::Float3 => 12,
            Format::Float4 => 16,
            Format::UnormByte4 => 4,
            Format::UShort => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    Layout(&'static str),
    /// A buffer is shorter than the vertices or indices it must hold.
    BufferTooSmall,
    /// The index count does not make whole primitives.
    IndexCount,
    /// An index names a vertex past the end of the mesh.
    IndexOutOfRange,
    /// The allocator's byte budget would be exceeded.
    OutOfBudget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub format: Format,
    /// Index of the vertex buffer (and stride) the attribute lives in.
    pub buffer: usize,
    pub offset: usize,
}

/// Attributes and one stride per vertex buffer. Validated on construction: names are unique,
/// every attribute fits its stride and no two attributes in one buffer overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexLayout {
    attributes: Vec<Attribute>,
    strides: Vec<usize>,
}

impl VertexLayout {
    pub fn new(attributes: Vec<Attribute>, strides: Vec<usize>) -> Result<Self, MeshError> {
        for (i, a) in attributes.iter().enumerate() {
            let stride = *strides.get(a.buffer).ok_or(MeshError::Layout("attribute names a missing buffer"))?;
            let end = a.offset + a.format.size();
            if end > stride {
                return Err(MeshError::Layout("attribute runs past its stride"));
            }
            for (j, b) in attributes.iter().enumerate() {
                if j == i {
                    continue;
                }
                if b.name == a.name {
                    return Err(MeshError::Layout("duplicate attribute name"));
                }
                let overlaps = b.buffer == a.buffer && a.offset < b.offset + b.format.size() && b.offset < end;
                if overlaps {
                    return Err(MeshError::Layout("attributes overlap in one buffer"));
                }
            }
        }
        Ok(VertexLayout { attributes, strides })
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub fn strides(&self) -> &[usize] {
        &self.strides
    }

    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }
}

/// Bytes owned by an allocator, with an id that is unique for the allocator's lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buffer {
    pub id: usize,
    pub bytes: Vec<u8>,
}

/// Hands out buffers until the total live bytes would pass the budget.
#[derive(Debug)]
pub struct BufferAllocator {
    next_id: usize,
    budget: usize,
    used: usize,
}

impl BufferAllocator {
    pub fn new(budget: usize) -> Self {
        BufferAllocator { next_id: 0, budget, used: 0 }
    }

    /// Bytes held by buffers that have not been released.
    pub fn used(&self) -> usize {
        self.used
    }

    pub fn zeroed(&mut self, len: usize) -> Result<Buffer, MeshError> {
        self.with_bytes(vec![0; len])
    }

    pub fn with_bytes(&mut self, bytes: Vec<u8>) -> Result<Buffer, MeshError> {
        let total = self.used.checked_add(bytes.len()).filter(|t| *t <= self.budget).ok_or(MeshError::OutOfBudget)?;
        self.used = total;
        let id = self.next_id;
        self.next_id += 1;
        Ok(Buffer { id, bytes })
    }

    /// Returns a buffer's bytes to the budget.
    pub fn release(&mut self, buffer: Buffer) {
        self.used = self.used.saturating_sub(buffer.bytes.len());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Points,
    Lines,
    LineStrip,
    Triangles,
    TriangleStrip,
}

impl Primitive {
    /// Whether `count` indices make whole primitives of this kind.
    pub fn accepts_count(self, count: usize) -> bool {
        match self {
            Primitive::Points | Primitive::LineStrip | Primitive::TriangleStrip => count > 0,
            Primitive::Lines => count > 0 && count % 2 == 0,
            Primitive::Triangles => count > 0 && count % 3 == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexType {
    UInt16,
    UInt32,
}

impl IndexType {
    pub fn size(self) -> usize {
        match self {
            IndexType::UInt16 => 2,
            IndexType::UInt32 => 4,
        }
    }

    fn read(self, bytes: &[u8], at: usize) -> Option<u32> {
        match self {
            IndexType::UInt16 => Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32),
            IndexType::UInt32 => Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)),
        }
    }
}

/// One drawable part of a mesh: a primitive kind and the index buffer that selects vertices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submesh {
    pub name: String,
    pub primitive: Primitive,
    pub index_type: IndexType,
    pub index_count: usize,
    pub index_buffer: Buffer,
}

impl Submesh {
    pub fn new(
        name: &str,
        primitive: Primitive,
        index_type: IndexType,
        index_count: usize,
        index_buffer: Buffer,
    ) -> Result<Self, MeshError> {
        if !primitive.accepts_count(index_count) {
            return Err(MeshError::IndexCount);
        }
        if index_buffer.bytes.len() < index_count * index_type.size() {
            return Err(MeshError::BufferTooSmall);
        }
        Ok(Submesh { name: name.to_string(), primitive, index_type, index_count, index_buffer })
    }

    /// The vertex indices in order, or None if the buffer no longer holds them.
    pub fn indices(&self) -> Option<Vec<u32>> {
        let size = self.index_type.size();
        (0..self.index_count).map(|i| self.index_type.read(&self.index_buffer.bytes, i * size)).collect()
    }
}

/// Axis-aligned box around vertex positions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    fn including(self, p: [f32; 3]) -> Self {
        let min = [0, 1, 2].map(|i| self.min[i].min(p[i]));
        let max = [0, 1, 2].map(|i| self.max[i].max(p[i]));
        Aabb { min, max }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    vertex_count: usize,
    layout: VertexLayout,
    vertex_buffers: Vec<Buffer>,
    submeshes: Vec<Submesh>,
}

fn f32_le(bytes: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

impl Mesh {
    /// Checks that there is one buffer per stride, that each holds `vertex_count` vertices and
    /// that every submesh index names an existing vertex.
    pub fn new(
        name: &str,
        vertex_count: usize,
        layout: VertexLayout,
        vertex_buffers: Vec<Buffer>,
        submeshes: Vec<Submesh>,
    ) -> Result<Self, MeshError> {
        if vertex_buffers.len() != layout.strides().len() {
            return Err(MeshError::Layout("one vertex buffer per stride"));
        }
        for (buffer, stride) in vertex_buffers.iter().zip(layout.strides()) {
            if buffer.bytes.len() < vertex_count * stride {
                return Err(MeshError::BufferTooSmall);
            }
        }
        for sub in &submeshes {
            let indices = sub.indices().ok_or(MeshError::BufferTooSmall)?;
            if indices.iter().any(|&i| i as usize >= vertex_count) {
                return Err(MeshError::IndexOutOfRange);
            }
        }
        Ok(Mesh { name: name.to_string(), vertex_count, layout, vertex_buffers, submeshes })
    }

    pub fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    pub fn layout(&self) -> &VertexLayout {
        &self.layout
    }

    pub fn vertex_buffers(&self) -> &[Buffer] {
        &self.vertex_buffers
    }

    pub fn submeshes(&self) -> &[Submesh] {
        &self.submeshes
    }

    /// The `position` attribute of one vertex, when it is a three-float attribute.
    pub fn position(&self, vertex: usize) -> Option<[f32; 3]> {
        let a = self.layout.attribute("position")?;
        if a.format != Format::Float3 || vertex >= self.vertex_count {
            return None;
        }
        let stride = self.layout.strides()[a.buffer];
        let at = vertex * stride + a.offset;
        let bytes = self.vertex_buffers[a.buffer].bytes.get(at..at + 12)?;
        Some([f32_le(bytes)?, f32_le(&bytes[4..])?, f32_le(&bytes[8..])?])
    }

    /// The box around every vertex position; None without positions or vertices.
    pub fn bounds(&self) -> Option<Aabb> {
        let first = self.position(0)?;
        (1..self.vertex_count).try_fold(Aabb { min: first, max: first }, |b, v| Some(b.including(self.position(v)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(name: &str, format: Format, buffer: usize, offset: usize) -> Attribute {
        Attribute { name: name.to_string(), format, buffer, offset }
    }

    fn f32s(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn layout_accepts_interleaved_attributes_and_rejects_bad_ones() {
        let ok = VertexLayout::new(
            vec![attr("position", Format::Float3, 0, 0), attr("normal", Format::Float3, 0, 12), attr("uv", Format::Float2, 0, 24)],
            vec![32],
        );
        assert!(ok.is_ok());
        let overlap = VertexLayout::new(
            vec![attr("position", Format::Float3, 0, 0), attr("normal", Format::Float3, 0, 8)],
            vec![24],
        );
        assert_eq!(overlap, Err(MeshError::Layout("attributes overlap in one buffer")));
        let past_stride = VertexLayout::new(vec![attr("uv", Format::Float2, 0, 28)], vec![30]);
        assert_eq!(past_stride, Err(MeshError::Layout("attribute runs past its stride")));
        let dup = VertexLayout::new(vec![attr("p", Format::Float, 0, 0), attr("p", Format::Float, 0, 4)], vec![8]);
        assert_eq!(dup, Err(MeshError::Layout("duplicate attribute name")));
        let missing = VertexLayout::new(vec![attr("p", Format::Float, 1, 0)], vec![8]);
        assert_eq!(missing, Err(MeshError::Layout("attribute names a missing buffer")));
    }

    #[test]
    fn mesh_rejects_a_vertex_buffer_shorter_than_its_count() {
        let layout = VertexLayout::new(vec![attr("position", Format::Float3, 0, 0)], vec![12]).unwrap();
        let mut alloc = BufferAllocator::new(1024);
        let short = alloc.zeroed(40).unwrap();
        assert_eq!(Mesh::new("m", 4, layout.clone(), vec![short], vec![]), Err(MeshError::BufferTooSmall));
        let exact = alloc.zeroed(48).unwrap();
        assert!(Mesh::new("m", 4, layout, vec![exact], vec![]).is_ok());
    }

    #[test]
    fn submesh_checks_index_count_and_reads_back_u16_indices() {
        let mut alloc = BufferAllocator::new(1024);
        let triangles_of_four = alloc.zeroed(8).unwrap();
        assert_eq!(
            Submesh::new("s", Primitive::Triangles, IndexType::UInt16, 4, triangles_of_four),
            Err(MeshError::IndexCount)
        );
        let short = alloc.with_bytes(vec![0; 5]).unwrap();
        assert_eq!(
            Submesh::new("s", Primitive::Triangles, IndexType::UInt16, 3, short),
            Err(MeshError::BufferTooSmall)
        );
        let bytes: Vec<u8> = [1u16, 2, 300].iter().flat_map(|i| i.to_le_bytes()).collect();
        let buf = alloc.with_bytes(bytes).unwrap();
        let sub = Submesh::new("s", Primitive::Triangles, IndexType::UInt16, 3, buf).unwrap();
        assert_eq!(sub.indices(), Some(vec![1, 2, 300]));
    }

    #[test]
    fn mesh_rejects_indices_past_its_vertices() {
        let layout = VertexLayout::new(vec![attr("position", Format::Float3, 0, 0)], vec![12]).unwrap();
        let mut alloc = BufferAllocator::new(1024);
        let verts = alloc.zeroed(24).unwrap();
        let idx: Vec<u8> = [0u32, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
        let sub = Submesh::new("s", Primitive::Triangles, IndexType::UInt32, 3, alloc.with_bytes(idx).unwrap()).unwrap();
        assert_eq!(Mesh::new("m", 2, layout, vec![verts], vec![sub]), Err(MeshError::IndexOutOfRange));
    }

    #[test]
    fn bounds_follow_the_position_attribute_in_interleaved_data() {
        let layout = VertexLayout::new(
            vec![attr("position", Format::Float3, 0, 0), attr("normal", Format::Float3, 0, 12)],
            vec![24],
        )
        .unwrap();
        let verts = f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, -1.0, 5.0, 0.0, 0.0, 1.0]);
        let mut alloc = BufferAllocator::new(1024);
        let mesh = Mesh::new("m", 2, layout, vec![alloc.with_bytes(verts).unwrap()], vec![]).unwrap();
        assert_eq!(mesh.position(1), Some([2.0, -1.0, 5.0]));
        assert_eq!(mesh.bounds(), Some(Aabb { min: [0.0, -1.0, 0.0], max: [2.0, 0.0, 5.0] }));
    }

    #[test]
    fn bounds_are_none_without_a_position_attribute() {
        let layout = VertexLayout::new(vec![attr("uv", Format::Float2, 0, 0)], vec![8]).unwrap();
        let mut alloc = BufferAllocator::new(1024);
        let mesh = Mesh::new("m", 1, layout, vec![alloc.zeroed(8).unwrap()], vec![]).unwrap();
        assert_eq!(mesh.position(0), None);
        assert_eq!(mesh.bounds(), None);
    }

    #[test]
    fn allocator_enforces_its_budget_and_reuses_released_bytes() {
        let mut alloc = BufferAllocator::new(150);
        let first = alloc.zeroed(100).unwrap();
        assert_eq!(alloc.zeroed(100), Err(MeshError::OutOfBudget));
        assert_eq!(alloc.used(), 100, "a refused allocation takes no budget");
        alloc.release(first);
        let third = alloc.zeroed(100).unwrap();
        assert_eq!((third.id, alloc.used()), (1, 100), "ids are never reused");
    }
}
