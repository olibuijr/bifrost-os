# GLKit parity: pure Rust models

Status: partial (was partial; a second round on branch parity/team-n-glkit adds shader-free effects, a texture request model, a drawable view model and a render loop row). GLKit is the OpenGL and OpenGL ES helper layer. Apple deprecated most of it on macOS
from 10.14 and points new work at the Metal path. This round adds the parts that need no GPU and no shader supply, as
pure `std` Rust models. The matrix row is `docs/parity-matrix/data/glkit.json`.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/linalg.rs` | GLKVector3, GLKVector4, GLKMatrix4, GLKQuaternion, GLKit Math Utilities | Column-major 4x4 matrices (identity, translation, scaling, axis rotation, perspective, look-at, transform, transform point, transpose, inverse), vectors, Hamilton-product quaternions with axis-angle, rotate, slerp and matrix conversion, and degree/radian helpers. |
| `springboard/ui/src/mesh.rs` | GLKMesh, GLKMeshBuffer, GLKMeshBufferAllocator, GLKSubmesh | Byte-range layout inside one allocation, submeshes with a primitive mode and index width, primitive counts, whole-primitive checks, and a 4-byte aligned bump allocator. No GPU memory is touched. |
| `springboard/ui/src/shader_effect.rs` | GLKBaseEffect (transform, lights, fog), GLKReflectionMapEffect and GLKSkyboxEffect (reflection and cube-map lookup) | A base effect that holds a projection, a model-view matrix, three light slots, a material and fog, and shades a vertex in eye space. Also the GL `reflect` rule, the cube-map face and coordinate lookup, and the world-space ray a skybox samples. No shader source and no GPU program. |
| `springboard/ui/src/texture_loader.rs` | GLKTextureLoader, GLKTextureInfo, GLKTextureLoaderOptions | Load options (origin, sRGB, premultiplied alpha, mipmaps), the texture description of a decoded image with its mip count and alpha state, and a request table with pending, loaded and failed states. Decodes nothing and uploads nothing. |
| `springboard/ui/src/drawable_view.rs` | GLKView (display pass, needs display, snapshot) | A view that owns a drawable, runs a delegate draw inside a display pass only when the drawable is valid and sized, counts passes, and reports the padded layout of the last pass as a snapshot. |
| `springboard/ui/src/render_loop.rs` (existing) | GLKViewController (preferred frames per second, paused, pause and resume with app activity) | Named here for the matrix row; the model was built in an earlier round and is not changed by this one. |
| `springboard/ui/src/shading.rs` | GLKBaseEffect lighting, GLKEffectPropertyLight, GLKEffectPropertyMaterial, GLKEffectPropertyFog | Blinn-Phong shading per point (ambient, Lambert diffuse, highlight, clamp) and linear or exponential fog. |

Each module has its own `#[cfg(test)] mod tests` with behaviour tests (linalg 8, mesh 8, shading 6). Each module
is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. The second round adds `shader_effect` (14 tests),
`texture_loader` (8 tests) and `drawable_view` (7 tests), registered in the same way.

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
- Effects: GLKBaseEffect (lighting, transform, fog), GLKEffectPropertyLight, GLKEffectPropertyMaterial, GLKEffectPropertyFog,
  GLKReflectionMapEffect and GLKSkyboxEffect (the reflection and cube-map parts only).
- Textures: GLKTextureLoader, GLKTextureInfo, GLKTextureLoaderOptions (the request and description model only).
- View: GLKView (`display`, `needsDisplay`, `drawableColorFormat`, `snapshot`) as a pure model, with the matching
  drawable rules from `drawable.rs`.

## REA evidence

- Landing page, rendered: `ev_2dea8330752a36acabb14ce26422118cccfeccc1eef2a1724b27913e3d95c67d` (from the existing matrix row).
- Landing page, re-captured for this round: `ev_feef1a9fdda2fe16c4f519399af127b519c4e93f390ed83ca6218b3f2a7463c4`
  (title "GLKit | Apple Developer Documentation"; states that on macOS GLKit requires an OpenGL context supporting
  the OpenGL 3.2 Core Profile).

## Not built, and why

- No GL view, drawable or context for apps. The presenter in `springboard/src/gpu/` stays internal, and `drawable_view`
  has no context behind it.
- No app-supplied shaders and no GPU program for the effect model. The effect and reflection code computes on the CPU.
- No GPU texture upload. The texture loader models requests only; image decode is CPU only.
- No vertex descriptor, GPU buffer upload or mesh file loader.
- No 3x3 matrix type or matrix stack.
- The 3D math is not yet re-exported through `bifrost_app`. The models live in `bifrost-ui`, the same crate as the
  other pure models.
- The render loop model (`render_loop.rs`) is not yet used by the SDK's timed draw path.

## Verification

Not built or run in this round, by instruction. `python3 tools/check-names.py` exits 0 on the new files after the
Apple framework prefix was replaced in comments with the repo's `Native` convention. The next stage must run
`cargo test -p bifrost-ui` before this is published. The second round is the same: it was written without a build or test run,
and the lead runs `cargo test -p bifrost-ui` and `python3 tools/check-names.py` before merging.
