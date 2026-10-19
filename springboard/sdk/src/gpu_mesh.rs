//! Meshes for GPU views: the app-facing counterpart of a mesh built from an asset. A built-in shape (box, plane, sphere) or a
//! mesh file (Wavefront OBJ, STL, ASCII PLY) becomes a [`Mesh`] of `bifrost_render::mesh`: one interleaved vertex buffer
//! (position, normal, texture coordinate, 32 bytes per vertex) and one triangle submesh whose indices are 16-bit when the
//! vertices fit and 32-bit otherwise. Buffers come from a [`BufferAllocator`] with a byte budget, so an app (or a file
//! it was given) cannot take more memory than it asked for.
//!
//! The vertex data stays on the CPU until a view uploads it. Vertex layouts of other formats and per-vertex tangents are
//! not built. See docs/bifrost-ui.md "GPU kits" for the decisions and the gaps.
use std::path::Path;

use bifrost_render::mesh::{Attribute, BufferAllocator, Format, IndexType, MeshError, Primitive, Submesh, VertexLayout};
use bifrost_ui::mesh_file::{read_obj, read_ply, read_stl, MeshFileError};
use bifrost_ui::mesh_primitives::{box_mesh, plane_mesh, sphere_mesh, PrimitiveMesh};

pub use bifrost_render::mesh::{Aabb, Buffer, Mesh};

/// The largest mesh file `load_mesh_file` reads.
pub const MAX_FILE_BYTES: usize = 64 << 20;
/// Bytes per vertex of the standard layout: position (3 floats), normal (3 floats), texture coordinate (2 floats).
pub const VERTEX_STRIDE: usize = 32;

/// Why a mesh could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshBuildError {
    /// The mesh container refused the data (an index past the last vertex, an index count that is not whole triangles, the
    /// allocator's budget).
    Mesh(MeshError),
    /// The shape has no vertices, or its normals or texture coordinates do not match its positions.
    Inconsistent,
    /// A shape size is negative or not finite.
    InvalidShape,
    /// The file could not be parsed.
    File(MeshFileError),
    /// The file extension is not `obj`, `stl` or `ply`.
    UnknownFormat,
    /// A text format file is not UTF-8.
    NotText,
    /// The file is larger than `MAX_FILE_BYTES`.
    FileTooLarge,
}

impl core::fmt::Display for MeshBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MeshBuildError::Mesh(e) => write!(f, "mesh: {e:?}"),
            MeshBuildError::Inconsistent => write!(f, "mesh: the vertex arrays do not match"),
            MeshBuildError::InvalidShape => write!(f, "mesh: a shape size is negative or not finite"),
            MeshBuildError::File(e) => write!(f, "mesh: {e}"),
            MeshBuildError::UnknownFormat => write!(f, "mesh: the file is not an obj, stl or ply file"),
            MeshBuildError::NotText => write!(f, "mesh: the text file is not UTF-8"),
            MeshBuildError::FileTooLarge => write!(f, "mesh: the file is larger than {MAX_FILE_BYTES} bytes"),
        }
    }
}

impl std::error::Error for MeshBuildError {}

impl From<MeshError> for MeshBuildError {
    fn from(e: MeshError) -> Self {
        MeshBuildError::Mesh(e)
    }
}

impl From<MeshFileError> for MeshBuildError {
    fn from(e: MeshFileError) -> Self {
        MeshBuildError::File(e)
    }
}

/// The layout every mesh of this module has: `position` (3 floats at 0), `normal` (3 floats at 12) and `uv` (2 floats at
/// 24) in one buffer of `VERTEX_STRIDE` bytes per vertex.
pub fn standard_layout() -> VertexLayout {
    let attr = |name: &str, format: Format, offset: usize| Attribute { name: name.to_string(), format, buffer: 0, offset };
    VertexLayout::new(
        vec![attr("position", Format::Float3, 0), attr("normal", Format::Float3, 12), attr("uv", Format::Float2, 24)],
        vec![VERTEX_STRIDE],
    )
    .expect("the standard layout is consistent")
}

/// Builds a mesh from vertex data. The vertex buffer and the index buffer are taken from `alloc`; when the budget does not
/// cover both, nothing stays allocated.
pub fn mesh_from_primitive(name: &str, src: &PrimitiveMesh, alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    let n = src.positions.len();
    if n == 0 || src.normals.len() != n || src.texture_coordinates.len() != n {
        return Err(MeshBuildError::Inconsistent);
    }
    if !Primitive::Triangles.accepts_count(src.indices.len()) {
        return Err(MeshError::IndexCount.into());
    }
    if src.indices.iter().any(|&i| i as usize >= n) {
        return Err(MeshError::IndexOutOfRange.into());
    }
    let mut vertices = Vec::with_capacity(n * VERTEX_STRIDE);
    for i in 0..n {
        let (p, nm, uv) = (src.positions[i], src.normals[i], src.texture_coordinates[i]);
        for v in [p.x, p.y, p.z, nm.x, nm.y, nm.z, uv[0], uv[1]] {
            vertices.extend_from_slice(&v.to_le_bytes());
        }
    }
    let index_type = if n <= u16::MAX as usize + 1 { IndexType::UInt16 } else { IndexType::UInt32 };
    let mut indices = Vec::with_capacity(src.indices.len() * index_type.size());
    for &i in &src.indices {
        match index_type {
            IndexType::UInt16 => indices.extend_from_slice(&(i as u16).to_le_bytes()),
            IndexType::UInt32 => indices.extend_from_slice(&i.to_le_bytes()),
        }
    }
    let vertex_buffer = alloc.with_bytes(vertices)?;
    let index_buffer = match alloc.with_bytes(indices) {
        Ok(b) => b,
        Err(e) => {
            alloc.release(vertex_buffer);
            return Err(e.into());
        }
    };
    let submesh = Submesh::new("triangles", Primitive::Triangles, index_type, src.indices.len(), index_buffer)?;
    Ok(Mesh::new(name, n, standard_layout(), vec![vertex_buffer], vec![submesh])?)
}

fn from_shape(name: &str, shape: Option<PrimitiveMesh>, alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    mesh_from_primitive(name, &shape.ok_or(MeshBuildError::InvalidShape)?, alloc)
}

/// A box centred on the origin (`width` along x, `height` along y, `length` along z), each face cut into the given
/// number of cells (at least one).
pub fn box_asset(width: f32, height: f32, length: f32, segments: [usize; 3], alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    from_shape("box", box_mesh(width, height, length, segments[0], segments[1], segments[2]), alloc)
}

/// A rectangle in the x-y plane facing +z.
pub fn plane_asset(width: f32, height: f32, segments: [usize; 2], alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    from_shape("plane", plane_mesh(width, height, segments[0], segments[1]), alloc)
}

/// A sphere with `segments` divisions around the equator (at least three).
pub fn sphere_asset(radius: f32, segments: usize, alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    from_shape("sphere", sphere_mesh(radius, segments), alloc)
}

/// Builds a mesh from the bytes of a mesh file. The format comes from the extension of `file_name` (`obj`, `stl`, `ply`,
/// any case) and the mesh is named after the file's stem. Hostile files are refused: a file above `MAX_FILE_BYTES`, a
/// face that names a vertex the file does not define, and anything the allocator's budget cannot hold.
pub fn load_mesh_file(file_name: &str, bytes: &[u8], alloc: &mut BufferAllocator) -> Result<Mesh, MeshBuildError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(MeshBuildError::FileTooLarge);
    }
    let path = Path::new(file_name);
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    let text = || std::str::from_utf8(bytes).map_err(|_| MeshBuildError::NotText);
    let shape = match ext.as_deref() {
        Some("obj") => read_obj(text()?)?,
        Some("stl") => read_stl(bytes)?,
        Some("ply") => read_ply(text()?)?,
        _ => return Err(MeshBuildError::UnknownFormat),
    };
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mesh");
    mesh_from_primitive(name, &shape, alloc)
}

/// The number of triangles in every submesh of `mesh` that draws triangles.
pub fn triangle_count(mesh: &Mesh) -> usize {
    mesh.submeshes().iter().filter(|s| s.primitive == Primitive::Triangles).map(|s| s.index_count / 3).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::linalg::Vector3;

    const ROOMY: usize = 64 << 20;

    #[test]
    fn a_box_becomes_one_interleaved_buffer_and_one_submesh() {
        let mut alloc = BufferAllocator::new(ROOMY);
        let mesh = box_asset(2.0, 4.0, 6.0, [1, 1, 1], &mut alloc).unwrap();
        assert_eq!(mesh.vertex_count(), 24);
        assert_eq!(triangle_count(&mesh), 12);
        assert_eq!(mesh.vertex_buffers().len(), 1);
        assert_eq!(mesh.vertex_buffers()[0].bytes.len(), 24 * VERTEX_STRIDE);
        assert_eq!(mesh.submeshes()[0].index_type, IndexType::UInt16);
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min, b.max), ([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]));
        assert_eq!(alloc.used(), 24 * VERTEX_STRIDE + 36 * 2);
        assert_eq!(mesh.layout().attribute("uv").unwrap().offset, 24);
    }

    #[test]
    fn indices_round_trip_and_stay_in_range() {
        let shape = sphere_mesh(1.0, 12).unwrap();
        let mut alloc = BufferAllocator::new(ROOMY);
        let mesh = mesh_from_primitive("ball", &shape, &mut alloc).unwrap();
        assert_eq!(mesh.name, "ball");
        assert_eq!(mesh.submeshes()[0].indices().unwrap(), shape.indices);
        let b = mesh.bounds().unwrap();
        assert!(b.max[0] <= 1.0001 && b.min[1] >= -1.0001);
        assert!((mesh.position(0).unwrap()[1] - 1.0).abs() < 1e-4, "the first ring is the pole");
    }

    #[test]
    fn a_large_grid_switches_to_32_bit_indices() {
        let mut alloc = BufferAllocator::new(ROOMY);
        let mesh = plane_asset(1.0, 1.0, [256, 256], &mut alloc).unwrap();
        assert_eq!(mesh.vertex_count(), 257 * 257);
        assert_eq!(mesh.submeshes()[0].index_type, IndexType::UInt32);
        assert_eq!(triangle_count(&mesh), 256 * 256 * 2);
        let last = mesh.submeshes()[0].indices().unwrap().into_iter().max().unwrap();
        assert!(last as usize >= 65536 && (last as usize) < mesh.vertex_count());
    }

    #[test]
    fn the_budget_is_enforced_and_nothing_leaks_on_failure() {
        let mut alloc = BufferAllocator::new(100);
        assert_eq!(box_asset(1.0, 1.0, 1.0, [1, 1, 1], &mut alloc), Err(MeshBuildError::Mesh(MeshError::OutOfBudget)));
        assert_eq!(alloc.used(), 0);
        // Room for the vertices (24 * 32 = 768) but not for the indices (72): the vertex buffer is given back.
        let mut tight = BufferAllocator::new(768 + 10);
        assert_eq!(box_asset(1.0, 1.0, 1.0, [1, 1, 1], &mut tight), Err(MeshBuildError::Mesh(MeshError::OutOfBudget)));
        assert_eq!(tight.used(), 0);
    }

    #[test]
    fn bad_shapes_and_inconsistent_data_are_refused() {
        let mut alloc = BufferAllocator::new(ROOMY);
        assert_eq!(box_asset(-1.0, 1.0, 1.0, [1, 1, 1], &mut alloc), Err(MeshBuildError::InvalidShape));
        assert_eq!(sphere_asset(f32::NAN, 8, &mut alloc), Err(MeshBuildError::InvalidShape));
        let one = PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0)],
            normals: vec![],
            texture_coordinates: vec![[0.0, 0.0]],
            indices: vec![],
        };
        assert_eq!(mesh_from_primitive("x", &one, &mut alloc), Err(MeshBuildError::Inconsistent));
        let tri = |indices: Vec<u32>| PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0), Vector3::new(0.0, 1.0, 0.0)],
            normals: vec![Vector3::new(0.0, 0.0, 1.0); 3],
            texture_coordinates: vec![[0.0, 0.0]; 3],
            indices,
        };
        assert_eq!(mesh_from_primitive("x", &tri(vec![0, 1]), &mut alloc), Err(MeshBuildError::Mesh(MeshError::IndexCount)));
        assert_eq!(mesh_from_primitive("x", &tri(vec![0, 1, 3]), &mut alloc), Err(MeshBuildError::Mesh(MeshError::IndexOutOfRange)));
        assert_eq!(alloc.used(), 0);
        assert!(mesh_from_primitive("x", &tri(vec![0, 1, 2]), &mut alloc).is_ok());
    }

    #[test]
    fn an_obj_file_becomes_a_mesh_named_after_the_file() {
        let mut alloc = BufferAllocator::new(ROOMY);
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let mesh = load_mesh_file("models/Triangle.OBJ", obj, &mut alloc).unwrap();
        assert_eq!(mesh.name, "Triangle");
        assert_eq!((mesh.vertex_count(), triangle_count(&mesh)), (3, 1));
        assert_eq!(mesh.position(1), Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn an_ascii_stl_and_a_ply_file_load() {
        let mut alloc = BufferAllocator::new(ROOMY);
        let stl = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let mesh = load_mesh_file("t.stl", stl, &mut alloc).unwrap();
        assert_eq!(triangle_count(&mesh), 1);
        let ply = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = load_mesh_file("t.ply", ply.as_bytes(), &mut alloc).unwrap();
        assert_eq!((mesh.vertex_count(), triangle_count(&mesh)), (3, 1));
    }

    #[test]
    fn hostile_files_are_refused() {
        let mut alloc = BufferAllocator::new(ROOMY);
        assert_eq!(load_mesh_file("a.fbx", b"x", &mut alloc), Err(MeshBuildError::UnknownFormat));
        assert_eq!(load_mesh_file("noextension", b"x", &mut alloc), Err(MeshBuildError::UnknownFormat));
        assert_eq!(load_mesh_file("a.obj", &[0xff, 0xfe, 0xfd], &mut alloc), Err(MeshBuildError::NotText));
        assert_eq!(
            load_mesh_file("a.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n", &mut alloc),
            Err(MeshBuildError::File(MeshFileError::IndexOutOfRange { line: 4 }))
        );
        assert_eq!(load_mesh_file("a.obj", b"# nothing\n", &mut alloc), Err(MeshBuildError::File(MeshFileError::Empty)));
        // A binary STL that promises a million triangles and brings none.
        let mut stl = vec![0u8; 84];
        stl[80..84].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(matches!(load_mesh_file("a.stl", &stl, &mut alloc), Err(MeshBuildError::File(_))));
        assert_eq!(alloc.used(), 0);
    }
}
