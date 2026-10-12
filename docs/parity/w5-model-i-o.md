# Wave 5: Model I/O gap rows (pure Rust)

Framework: Model I/O (https://developer.apple.com/documentation/modelio). Official pages read through the DocC index
(`tools/parity/docc.py modelio`) and rendered in Chromium for REA
(`ev_9bfce4b8df9d8c12596a51db580ce862701578a1729a5705beb8b34800a873e2`, title "Model I/O | Apple Developer Documentation").

Three of the four gap rows were built as pure models, std only, extending the existing mesh and texture layers.
The fourth gap row (sharing buffers with GPU frameworks) is tied to Apple-only frameworks and is not built.
Nothing is wired into apps or the shell, and nothing has been built or run in this wave (no cargo, per the wave rules).

## What was built

| File | What it does | Official API it mirrors (DocC topic) |
|---|---|---|
| `springboard/ui/src/mesh_file.rs` | Reads Wavefront OBJ (v, vt, vn, f with `v/vt/vn`, negative indices, fan triangulation, shared corners emitted once), STL (ASCII, and binary when the length matches the 84 + 50 n rule), and ASCII PLY (x y z, optional normals and s/t or u/v, face lists). Writes OBJ. Returns `PrimitiveMesh`. | Import and export of mesh files in the "Extensible Asset Format Support" topic (`MDLAsset` import and export through `MDLComponent`); formats OBJ, PLY, STL. USD and Alembic are not built. |
| `springboard/ui/src/mesh_process.rs` | Area-weighted vertex normals (`smooth_normals`) and one level of midpoint subdivision (`subdivide_midpoints`) that keeps shared edges welded and winding intact. | "Mesh processing" in the framework overview: subdivision surfaces. Ambient occlusion baking, light probe placement and normal map generation are not built. |
| `springboard/ui/src/procedural_texture.rs` | RGBA8 checkerboard and horizontal two-colour gradient generators, and a sampler with nearest or bilinear filtering and repeat or clamp addressing. | "Textures" topic: `MDLCheckerboardTexture`, `MDLColorSwatchTexture`, `MDLTextureSampler`, `MDLTextureFilter`. `MDLURLTexture`, `MDLNoiseTexture`, `MDLNormalMapTexture` and `MDLSkyCubeTexture` are not built. |

Each module has a `#[cfg(test)] mod tests` with hand-computed expected values (OBJ quad triangulation and dedupe,
negative OBJ indices, binary and ASCII STL, PLY quad with uvs, area-weighted normals, midpoint subdivision area sum,
checkerboard cells, gradient rounding, repeat versus clamp, bilinear seam blend).

Reused existing types: `PrimitiveMesh` and `Vector3` (`mesh_primitives.rs`, `linalg.rs`). `lib.rs` gained three
`pub mod` lines after `pub mod mesh_primitives;`.

## Matrix rows closed

| Framework (slug) | Feature (as in `data/model-i-o.json`) | New status | BifrOSt path |
|---|---|---|---|
| model-i-o | File format import and export | partial | springboard/ui/src/mesh_file.rs |
| model-i-o | Mesh processing and baking | partial | springboard/ui/src/mesh_process.rs |
| model-i-o | Textures as 3D material inputs | partial | springboard/ui/src/procedural_texture.rs |

## Not done and why

- `uttype.rs` has no 3D type identifiers. Adding them needs a decision on identifiers that is not in the official
  pages read here, so the readers are not yet tied to Files or drag and drop.
- No mesh-buffer allocator or map is added (the "Mesh buffer allocation and mapping" gap row stays a gap).
- Output of these readers has not been compiled or run; the first build must run the three module test sets.
