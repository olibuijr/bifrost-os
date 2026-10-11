//! Mesh layout for 3D content (`NativeMesh`, `NativeMeshBuffer`, `NativeSubmesh`, `NativeMeshBufferAllocator`).
//! This module holds only the layout: byte ranges inside one allocation, primitive counts and validation.
//! It owns no GPU memory; a renderer maps each range onto its own buffers.

/// How consecutive elements form primitives (the OpenGL-style primitive modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveMode {
    Points,
    Lines,
    LineStrip,
    Triangles,
    TriangleStrip,
    TriangleFan,
}

/// Width of one element index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexType {
    U8,
    U16,
    U32,
}

impl IndexType {
    /// Size of one index in bytes.
    pub fn size(self) -> usize {
        match self {
            IndexType::U8 => 1,
            IndexType::U16 => 2,
            IndexType::U32 => 4,
        }
    }
}

/// What a buffer holds: vertex attributes or element indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    Vertex,
    Element,
}

/// A byte range inside an allocation (`NativeMeshBuffer`: length, offset and type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshBuffer {
    pub kind: BufferKind,
    pub offset: usize,
    pub length: usize,
}

impl MeshBuffer {
    /// One past the last byte of the range.
    pub fn end(self) -> usize {
        self.offset + self.length
    }
}

/// A run of primitives drawn from one element buffer (`NativeSubmesh`).
#[derive(Clone, Debug, PartialEq)]
pub struct Submesh {
    pub name: String,
    pub mode: PrimitiveMode,
    pub element_buffer: MeshBuffer,
    pub element_count: usize,
    pub index_type: IndexType,
}

impl Submesh {
    /// Number of primitives the elements form under this submesh's mode.
    pub fn primitive_count(&self) -> usize {
        let n = self.element_count;
        match self.mode {
            PrimitiveMode::Points => n,
            PrimitiveMode::Lines => n / 2,
            PrimitiveMode::LineStrip => n.saturating_sub(1),
            PrimitiveMode::Triangles => n / 3,
            PrimitiveMode::TriangleStrip | PrimitiveMode::TriangleFan => n.saturating_sub(2),
        }
    }

    /// Whether the element count holds only whole primitives (an empty run is always whole).
    pub fn is_whole(&self) -> bool {
        let n = self.element_count;
        match self.mode {
            PrimitiveMode::Points => true,
            PrimitiveMode::Lines => n % 2 == 0,
            PrimitiveMode::LineStrip => n != 1,
            PrimitiveMode::Triangles => n % 3 == 0,
            PrimitiveMode::TriangleStrip | PrimitiveMode::TriangleFan => n != 1 && n != 2,
        }
    }
}

/// Why a mesh failed validation. Indices name the offending buffer or submesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// A vertex buffer has the wrong kind or runs past the allocation.
    VertexBuffer { index: usize },
    /// A submesh's element buffer has the wrong kind or runs past the allocation.
    ElementBuffer { submesh: usize },
    /// A submesh's element count does not fit in its element buffer.
    ElementRange { submesh: usize },
    /// A submesh's element count does not form whole primitives for its mode.
    Primitive { submesh: usize },
}

/// Shared vertex buffers plus the submeshes that index into element buffers (`NativeMesh`).
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub vertex_count: usize,
    pub vertex_buffers: Vec<MeshBuffer>,
    pub submeshes: Vec<Submesh>,
}

impl Mesh {
    pub fn new(name: &str, vertex_count: usize) -> Self {
        Mesh { name: name.to_string(), vertex_count, vertex_buffers: Vec::new(), submeshes: Vec::new() }
    }

    /// Checks every range against an allocation of `allocation_length` bytes and every submesh for whole primitives.
    pub fn validate(&self, allocation_length: usize) -> Result<(), MeshError> {
        for (index, b) in self.vertex_buffers.iter().enumerate() {
            if b.kind != BufferKind::Vertex || b.end() > allocation_length {
                return Err(MeshError::VertexBuffer { index });
            }
        }
        for (submesh, s) in self.submeshes.iter().enumerate() {
            let eb = s.element_buffer;
            if eb.kind != BufferKind::Element || eb.end() > allocation_length {
                return Err(MeshError::ElementBuffer { submesh });
            }
            if s.element_count > eb.length / s.index_type.size() {
                return Err(MeshError::ElementRange { submesh });
            }
            if !s.is_whole() {
                return Err(MeshError::Primitive { submesh });
            }
        }
        Ok(())
    }
}

/// Hands out byte ranges from one fixed-size allocation (`NativeMeshBufferAllocator`).
/// Ranges are never freed; each is aligned to `ALIGN` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferAllocator {
    capacity: usize,
    used: usize,
}

impl BufferAllocator {
    /// Byte alignment of every range the allocator returns.
    pub const ALIGN: usize = 4;

    pub fn new(capacity: usize) -> Self {
        BufferAllocator { capacity, used: 0 }
    }

    /// Reserves `length` bytes, or `None` when they do not fit in the remaining capacity.
    pub fn allocate(&mut self, kind: BufferKind, length: usize) -> Option<MeshBuffer> {
        let offset = (self.used + Self::ALIGN - 1) / Self::ALIGN * Self::ALIGN;
        let end = offset.checked_add(length)?;
        if end > self.capacity {
            return None;
        }
        self.used = end;
        Some(MeshBuffer { kind, offset, length })
    }

    /// Bytes reserved so far, including alignment padding.
    pub fn used(&self) -> usize {
        self.used
    }

    pub fn remaining(&self) -> usize {
        self.capacity - self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element_submesh(mode: PrimitiveMode, count: usize, buffer: MeshBuffer) -> Submesh {
        Submesh { name: "body".into(), mode, element_buffer: buffer, element_count: count, index_type: IndexType::U16 }
    }

    #[test]
    fn primitive_counts_follow_the_mode() {
        let buf = MeshBuffer { kind: BufferKind::Element, offset: 0, length: 0 };
        let count = |mode, n| element_submesh(mode, n, buf).primitive_count();
        assert_eq!(count(PrimitiveMode::Points, 5), 5);
        assert_eq!(count(PrimitiveMode::Lines, 6), 3);
        assert_eq!(count(PrimitiveMode::LineStrip, 6), 5);
        assert_eq!(count(PrimitiveMode::Triangles, 9), 3);
        assert_eq!(count(PrimitiveMode::TriangleStrip, 6), 4);
        assert_eq!(count(PrimitiveMode::TriangleFan, 1), 0);
    }

    #[test]
    fn incomplete_primitives_are_detected() {
        let buf = MeshBuffer { kind: BufferKind::Element, offset: 0, length: 0 };
        assert!(!element_submesh(PrimitiveMode::Triangles, 4, buf).is_whole());
        assert!(!element_submesh(PrimitiveMode::Lines, 3, buf).is_whole());
        assert!(!element_submesh(PrimitiveMode::TriangleStrip, 2, buf).is_whole());
        assert!(element_submesh(PrimitiveMode::TriangleStrip, 0, buf).is_whole());
        assert!(element_submesh(PrimitiveMode::TriangleFan, 3, buf).is_whole());
    }

    #[test]
    fn allocator_aligns_offsets_and_refuses_overflow() {
        let mut alloc = BufferAllocator::new(16);
        let a = alloc.allocate(BufferKind::Vertex, 6).unwrap();
        assert_eq!((a.offset, a.length), (0, 6));
        let b = alloc.allocate(BufferKind::Element, 6).unwrap();
        assert_eq!(b.offset, 8, "the next range starts on the 4-byte boundary after 6");
        assert_eq!(alloc.used(), 14);
        assert_eq!(alloc.remaining(), 2);
        assert!(alloc.allocate(BufferKind::Element, 4).is_none(), "16..20 is past the capacity");
        assert_eq!(alloc.used(), 14, "a refused request reserves nothing");
    }

    #[test]
    fn valid_mesh_passes_validation() {
        let mut alloc = BufferAllocator::new(100);
        let mut mesh = Mesh::new("cube", 8);
        mesh.vertex_buffers.push(alloc.allocate(BufferKind::Vertex, 48).unwrap());
        let elements = alloc.allocate(BufferKind::Element, 12).unwrap();
        mesh.submeshes.push(element_submesh(PrimitiveMode::Triangles, 6, elements));
        assert_eq!(mesh.validate(100), Ok(()));
    }

    #[test]
    fn element_count_past_the_buffer_is_rejected() {
        let elements = MeshBuffer { kind: BufferKind::Element, offset: 48, length: 12 };
        let mut mesh = Mesh::new("cube", 8);
        // Six U16 indices need 12 bytes; seven need 14.
        mesh.submeshes.push(element_submesh(PrimitiveMode::Triangles, 7, elements));
        assert_eq!(mesh.validate(100), Err(MeshError::ElementRange { submesh: 0 }));
    }

    #[test]
    fn incomplete_triangles_are_rejected() {
        let elements = MeshBuffer { kind: BufferKind::Element, offset: 0, length: 64 };
        let mut mesh = Mesh::new("strip", 8);
        mesh.submeshes.push(element_submesh(PrimitiveMode::Triangles, 4, elements));
        assert_eq!(mesh.validate(64), Err(MeshError::Primitive { submesh: 0 }));
    }

    #[test]
    fn wrong_buffer_kinds_and_ranges_past_the_allocation_are_rejected() {
        let mut mesh = Mesh::new("cube", 8);
        mesh.vertex_buffers.push(MeshBuffer { kind: BufferKind::Element, offset: 0, length: 16 });
        assert_eq!(mesh.validate(100), Err(MeshError::VertexBuffer { index: 0 }));

        let mut mesh = Mesh::new("cube", 8);
        mesh.vertex_buffers.push(MeshBuffer { kind: BufferKind::Vertex, offset: 0, length: 48 });
        assert_eq!(mesh.validate(40), Err(MeshError::VertexBuffer { index: 0 }));

        let mut mesh = Mesh::new("cube", 8);
        let vertex_kind = MeshBuffer { kind: BufferKind::Vertex, offset: 0, length: 12 };
        mesh.submeshes.push(element_submesh(PrimitiveMode::Points, 1, vertex_kind));
        assert_eq!(mesh.validate(100), Err(MeshError::ElementBuffer { submesh: 0 }));
    }

    #[test]
    fn index_sizes_match_their_widths() {
        assert_eq!(IndexType::U8.size(), 1);
        assert_eq!(IndexType::U16.size(), 2);
        assert_eq!(IndexType::U32.size(), 4);
    }
}
