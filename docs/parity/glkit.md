# GLKit parity: pure Rust models

Status: partial (was partial). GLKit is the OpenGL and OpenGL ES helper layer. Apple deprecated most of it on macOS
from 10.14 and points new work at the Metal path. This round adds the parts that need no GPU and no shader supply, as
pure `std` Rust models. The matrix row is `docs/parity-matrix/data/glkit.json`.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/linalg.rs` | GLKVector3, GLKVector4, GLKMatrix4, GLKQuaternion, GLKit Math Utilities | Column-major 4x4 matrices (identity, translation, scaling, axis rotation, perspective, look-at, transform, transform point, transpose, inverse), vectors, Hamilton-product quaternions with axis-angle, rotate, slerp and matrix conversion, and degree/radian helpers. |
| `springboard/ui/src/mesh.rs` | GLKMesh, GLKMeshBuffer, GLKMeshBufferAllocator, GLKSubmesh | Byte-range layout inside one allocation, submeshes with a primitive mode and index width, primitive counts, whole-primitive checks, and a 4-byte aligned bump allocator. No GPU memory is touched. |
| `springboard/ui/src/shading.rs` | GLKBaseEffect lighting, GLKEffectPropertyLight, GLKEffectPropertyMaterial, GLKEffectPropertyFog | Blinn-Phong shading per point (ambient, Lambert diffuse, highlight, clamp) and linear or exponential fog. |

Each module has its own `#[cfg(test)] mod tests` with behaviour tests (linalg 8, mesh 8, shading 6). Each module
is registered with one `pub mod` line in `springboard/ui/src/lib.rs`.

## Apple API names mirrored

Taken from the official GLKit documentation (`developer.apple.com/documentation/glkit`), read with the DocC
tool. In the Rust source the framework prefix is written as `Native` in comments, following the repo convention;
the names below are the framework's own.

- Math: GLKVector3, GLKVector4, GLKMatrix4, GLKQuaternion, GLKMatrix4Identity, GLKMathDegreesToRadians,
  GLKMathRadiansToDegrees, GLKMatrix4MultiplyVector4, GLKQuaternionMakeWithAngleAndAxis, GLKQuaternionSlerp.
  Function-level signatures for the rest of the matrix and quaternion family were not all retrievable from the
  DocC endpoint; their names follow the documented type members and are not confirmed signature by signature.
- Mesh: GLKMesh (`init(mesh:)`, `vertexBuffers`, `vertexCount`, `submeshes`, `vertexDescriptor`),
  GLKMeshBuffer (`allocator`, `glBufferName`, `length`, `offset`, `type`), GLKMeshBufferAllocator,
  GLKSubmesh (`elementBuffer`, `elementCount`, `mode`, `name`, `type`). `vertexDescriptor` and `glBufferName`
  are not modelled.
- Effects: GLKBaseEffect (lighting), GLKEffectPropertyLight, GLKEffectPropertyMaterial, GLKEffectPropertyFog.

## REA evidence

- Landing page, rendered: `ev_2dea8330752a36acabb14ce26422118cccfeccc1eef2a1724b27913e3d95c67d` (from the existing matrix row).
- Landing page, re-captured for this round: `ev_feef1a9fdda2fe16c4f519399af127b519c4e93f390ed83ca6218b3f2a7463c4`
  (title "GLKit | Apple Developer Documentation"; states that on macOS GLKit requires an OpenGL context supporting
  the OpenGL 3.2 Core Profile).

## Not built, and why

- No GL view, drawable or context for apps. The presenter in `springboard/src/gpu/` stays internal.
- No app-supplied shaders, reflection-map or skybox effects, and no effect object that binds lighting to a drawable.
- No texture property in the effect model and no background GPU texture loader. Image decode is CPU only.
- No vertex descriptor, GPU buffer upload or mesh file loader.
- No 3x3 matrix type or matrix stack.
- The 3D math is not yet re-exported through `bifrost_app`. The models live in `bifrost-ui`, the same crate as the
  other pure models.
- The render loop controller (preferred frame rate, pause, activation) is unchanged.

## Verification

Not built or run in this round, by instruction. `python3 tools/check-names.py` exits 0 on the new files after the
Apple framework prefix was replaced in comments with the repo's `Native` convention. The next stage must run
`cargo test -p bifrost-ui` before this is published.
