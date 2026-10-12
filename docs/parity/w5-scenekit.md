# Wave 5: SceneKit gap rows (scene asset import, physics simulation)

Branch `parity/w5-scenekit`, from `parity/integration` (waves 1 to 4). Two gap rows of the SceneKit framework are
now partial. Both are pure Rust with std only and host unit tests. Neither is wired into an app or the shell, and
neither has run on the VM or Midget yet.

## What was built

### Scene asset import: `springboard/ui/src/scene_source.rs`

A Wavefront OBJ text importer. `import_obj(text)` returns one `ImportedMesh` per object (`o`) that has faces.
It mirrors the reading side of the scene source (official: `SCNSceneSource`, `init(url:options:)`, `scene(options:)`,
`identifiersOfEntries(withClass:)`, `entries(passingTest:)`) for the one text-mesh format, not the scene graph.

- Lines: `v`, `vt`, `vn`, `f` (`v`, `v/vt`, `v//vn`, `v/vt/vn`), `o`. Other lines are ignored.
- Negative indices count back from the last vertex. Index 0 and out-of-range indices are refused.
- Faces with more than three corners are fan-triangulated.
- Corners without a normal take the face normal (Newell's method) and are flat-shaded.
- Each unique (position, texture, normal) combination is one output vertex.
- Errors carry the 1-based line: `Syntax`, `Index`, `NoFaces`.

Tests: `one_triangle_gets_a_flat_normal_from_its_winding`, `a_quad_is_fan_triangulated_into_two_triangles`,
`negative_indices_count_back_from_the_last_vertex`, `shared_corners_collapse_into_one_vertex_and_file_normals_are_kept`,
`each_object_becomes_its_own_named_mesh`, `malformed_and_out_of_range_input_is_refused_with_its_line`.

### Physics simulation: `springboard/ui/src/physics3d.rs`

A 3D rigid-body world of spheres under gravity, a ground plane at y = 0, and sphere-to-sphere contacts. It is the 3D
counterpart of `springboard/ui/src/physics.rs` (2D circles). It mirrors the physics world's gravity and speed
(official: `SCNPhysicsWorld`, `gravity`, `speed`, `timeStep`, `contactTest`) and the body's dynamic and static kinds
(official: `SCNPhysicsBody`).

- `World3::step(dt)` applies gravity, moves dynamic bodies (semi-implicit Euler), stops bodies below the ground with
  restitution, then resolves each overlapping pair once: push apart by inverse mass, reflect the closing speed by the
  smaller restitution.
- A zero step only resolves existing overlaps. A negative or non-finite step does nothing.
- The report lists the sphere contacts (`Contact3`, normal from `a` to `b`) and the dynamic bodies on the ground.

Tests: `a_falling_body_follows_semi_implicit_euler`, `speed_scales_the_time_step`,
`the_ground_stops_a_sinking_body_and_reflects_its_speed_by_restitution`,
`a_fixed_body_does_not_move_and_pushes_a_dynamic_one_out`,
`two_equal_elastic_spheres_swap_their_velocities_and_separate`, `a_non_finite_step_leaves_the_world_unchanged`.

Test values were worked out by hand from the rules above (for example a body falling from y = 100 for 0.5 s under
gravity -10 has velocity -5 and y = 97.5).

Both modules are registered in `springboard/ui/src/lib.rs` (one `pub mod` line each).

## Official API evidence

| Official page | REA evidence id |
|---|---|
| SceneKit (framework landing page) | `ev_8e75efc91449185d899c39041c8470671b07f03574c563d2730398ef64ad3af1` |
| SCNSceneSource | `ev_0456d016cd086d9c4c3f84e9a50f963cd6e164b8cdd5affbe0905b430cb81e1c` |
| SCNPhysicsWorld | `ev_4e9c2fb648d567737b4f8cb7c2c8049efb611506b6859efda95a51846e33822e` |

The official pages mark SceneKit and these classes as deprecated in favour of RealityKit. This wave mirrors the
behaviour, not the newer API.

## Matrix rows closed

| Framework slug | Feature (as in the matrix JSON) | New status | BifrOSt path |
|---|---|---|---|
| scenekit | Scene asset import | partial | springboard/ui/src/scene_source.rs |
| scenekit | Physics simulation | partial | springboard/ui/src/physics3d.rs |

## Not done, and why

- Views for 3D content (no GPU path for 3D; the presenter draws 2D surfaces only).
- Particle systems: `springboard/ui/src/emitter.rs` already exists (SpriteKit emitter row). A 3D emitter would reuse it,
  so it was left for a later round.
- Positional audio: `springboard/ui/src/sound_space.rs` already has listener, pan and occlusion gains. The row is still a
  gap in the matrix because no playback path uses it; that is an audio-service task.
- Custom shaders and DAE, USDZ or glTF import (only OBJ here).
- 3D joints, boxes, and contact callbacks as an app-facing delegate.

## Checks run

- `python3 tools/check-names.py`: exit 0.
- No cargo, rustc or tests were run in this wave (build is for the integration step).
