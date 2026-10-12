# Wave 4: Model I/O gap rows (materials, lights, voxels)

Pure Rust models for three gap rows of the Model I/O framework. They sit in `springboard/ui/src` (`bifrost_ui`), hold
data only, and draw nothing. None is wired into apps, the shell or the renderer yet, and none has run on the VM or
on Midget. Their tests are written and have not been run, because this wave does not build.

## What was built

- `springboard/ui/src/scene_material.rs`: `AssetMaterial` is an ordered list of named `MaterialProperty` values. Each
  property has a `MaterialSemantic` (what its value is for) and a `MaterialValue` (float, vector, colour, text or
  texture path). Setting a name again replaces the property in place. Lookups by name or by semantic fall back to an
  optional base material, and a property with the same name shadows the base. `ShadingModel` records basic or
  physically plausible.
- `springboard/ui/src/scene_light.rs`: `AssetLight` with `AssetLightType` (the twelve types of the official enum).
  `irradiance_at(point)` computes ambient, directional, point and spot lights. Point and spot lights fade linearly
  between `attenuation_start` and `attenuation_end`. A spot light is also scaled by a linear cone between
  `inner_cone_deg` and `outer_cone_deg`. The other eight types return `None`.
- `springboard/ui/src/voxel_array.rs`: `VoxelArray` is a set of cubic voxels on an integer grid over `VoxelBounds`,
  with `VoxelIndex` and `VoxelIndexExtent`. It supports set and query, index and spatial-location conversion, and
  union, intersection and difference between arrays on the same grid. Operations on different grids return `false`
  and change nothing.

Both `scene_material.rs` and `scene_light.rs` are new files. They do not reuse `springboard/ui/src/shading.rs`, which
models the GLKit effect lighting (`Light` and `Material` there are per-point shading inputs). The new types are asset
descriptions. A later step can turn an `AssetMaterial` into a `shading::Material`.

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`.

## Official API mirrored

Official pages read with `python3 -I tools/parity/docc.py`:

- `https://developer.apple.com/documentation/modelio/mdlmaterial`: `init(name:scatteringFunction:)`,
  `propertyNamed(_:)`, `property(with:)`, `properties(with:)`, `setProperty(_:)`, `remove(_:)`, `removeAllProperties()`,
  `base`, `count`.
- `https://developer.apple.com/documentation/modelio/mdlmaterialsemantic`: the semantic cases in
  `MaterialSemantic` (`baseColor` through `userDefined`).
- `https://developer.apple.com/documentation/modelio/mdlmaterialproperty`: `init(name:semantic:float:)` and the other
  value initialisers, as `MaterialValue`.
- `https://developer.apple.com/documentation/modelio/mdlscatteringfunction`: `ShadingModel` (basic model only; the
  property set is not mirrored).
- `https://developer.apple.com/documentation/modelio/mdllight`: `irradiance(atPoint:)`, `lightType`.
- `https://developer.apple.com/documentation/modelio/mdllighttype`: the cases of `AssetLightType`.
- `https://developer.apple.com/documentation/modelio/mdlphysicallyplausiblelight`: `innerConeAngle`,
  `outerConeAngle`, `attenuationStartDistance`, `attenuationEndDistance`, `lumens`, `color`.
- `https://developer.apple.com/documentation/modelio/mdlvoxelarray`: `init(data:boundingBox:voxelExtent:)`, `count`,
  `voxelIndexExtent`, `voxelExists(atIndex:)`, `voxels(within:)`, `voxelIndices()`, `setVoxelAtIndex(_:)`,
  `union(with:)`, `intersect(with:)`, `difference(with:)`, `boundingBox`, `index(ofSpatialLocation:)`,
  `spatialLocation(ofIndex:)`, `voxelBoundingBox(atIndex:)`.
- `https://developer.apple.com/documentation/modelio/mdlvoxelindex` and `MDLVoxelIndexExtent`: `VoxelIndex` and
  `VoxelIndexExtent`.

## REA evidence

Rendered official pages, read with `tools/parity/rea-page.sh`:

- `ev_0fa74b2631e4eacab53a04f19f040388dfd57758d4c83dbb47c9dfdd5e8d4a12`: framework landing page
  (`https://developer.apple.com/documentation/modelio`).
- `ev_9d2c3e9045dfc0ff5ba08c4ef78aa43e876607ad77e4835f5c069c4515f7ed12`: MDLMaterial.
- `ev_76519afbc6c0abcbefac7ee68cc755c3a36fe6751002cf4db9c197d05bab3a4a`: MDLLight.
- `ev_d05917159376967a346c3d01d5d2685093967afc6c249eba1977f7e781b5d38c`: MDLPhysicallyPlausibleLight.
- `ev_b0d3cdfa56754a2175992f11d0d1fcbe211859a296c1cbb8df8ccebf59b90d8b`: MDLVoxelArray.

No REA page was recorded for MDLScatteringFunction, MDLLightType or MDLMaterialSemantic. Their DocC pages were read
with `docc.py` only.

## Matrix rows closed

These rows go to `partial`. The matrix itself is not edited here.

| Framework | Feature (as written in `docs/parity-matrix/data/model-i-o.json`) | New status | BifrOSt path |
|---|---|---|---|
| Model I/O | Materials and shading models | partial | `springboard/ui/src/scene_material.rs` |
| Model I/O | Lights | partial | `springboard/ui/src/scene_light.rs` |
| Model I/O | Volumetric representation | partial | `springboard/ui/src/voxel_array.rs` |

## Not built, and why

- Texture samplers, procedural textures and the normal map generator (row "Textures as 3D material inputs"): the
  models store a texture as a path only, with no sampler, no filter and no pixel data.
- Mesh buffer allocation (row "Mesh buffer allocation and mapping"): `springboard/render/src/mesh.rs` and
  `springboard/ui/src/mesh.rs` already hold the layout pieces from waves 1 and 2, so this row is left for them.
- Colour temperature (`setColorByTemperature`), photometric profiles, probe irradiance and the environment light
  types: no formula is written for them here, so their types return `None`.
- Voxel shells, signed distance fields, the mesh built from voxels, and reducing the bounds on intersect or
  difference. The bounds are kept on those two operations, which is a known difference from the official
  description.
- The per-material `MDLScatteringFunction` property set. Only the basic and physically plausible labels are kept.

## Verification

Not run. This wave does not build (no `cargo`, `rustc` or tests). Each module has its own `#[cfg(test)] mod tests` with
hand-worked expected values: 6 tests in `scene_material.rs`, 7 in `scene_light.rs`, 10 in `voxel_array.rs`.
`python3 tools/check-names.py` passes with exit 0.
