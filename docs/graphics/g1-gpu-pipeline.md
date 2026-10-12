# G1: GPU pipelines, shaders, samplers (`gpu-pipeline`)

Owner: `gpu-pipeline`. Contract: `docs/graphics-architecture.md`, "bifrost-gpu". Files: `springboard/gpu/src/pipeline.rs`,
`springboard/gpu/src/shader.rs`, `springboard/gpu/build.rs`, and the bodies of `Device::new_render_pipeline`,
`Device::new_compute_pipeline` and `Device::new_sampler` in `device.rs`.

## What is implemented

- **Shaders.** `build.rs` compiles every `.vert`, `.frag` and `.comp` in `springboard/gpu/shaders/` with
  `glslc --target-env=vulkan1.3 -O`. Failures never stop the build. A missing `glslc` gives one warning. A failing
  shader gets a warning that names the file, followed by up to 8 compiler lines, and is left out of the table. Stale
  SPIR-V from an earlier build is deleted before each compile, so it cannot be embedded. Every file in `shaders/`
  (including include files such as `.glsl`) is a `rerun-if-changed` input. `shader::words_from_bytes` checks the SPIR-V
  magic and the length. `Stage::of_name` maps the suffix to a stage.
- **Render pipelines** (`new_render`). Vertex and fragment modules from `shader()`. Dynamic rendering with
  `VkPipelineRenderingCreateInfo` naming the target format (no render pass objects). Dynamic viewport and scissor.
  Blend: `PremultipliedOver` is `ONE / ONE_MINUS_SRC_ALPHA` for colour and alpha, `Add` is `ONE / ONE`, `Replace`
  disables blending. Write mask RGBA. Triangle strip or list. Optional instance or vertex layout on binding 0. Cull
  none, counter-clockwise front face, 1 sample. Push constants up to 128 bytes, a multiple of 4, visible to both stages.
- **Compute pipelines** (`new_compute`). One compute module, a set 0 layout, push constants for the compute stage.
- **Samplers** (`new_sampler`). Nearest or linear, clamp to edge on U, V and W, no mips, no anisotropy.
- **Pipeline cache.** One `VkPipelineCache` per device, made on first use and kept in a process registry. Creation is
  serialised under the registry lock because a cache needs external synchronisation.
- **Validation before any Vulkan call.** Stage suffix per slot, at most 32 textures and at most 32 samplers per kind,
  push constant size, vertex stride (multiple of 4, at most 2048) and attribute offsets inside the stride. Errors are
  `Error::Invalid`. A missing shader is `Error::Unsupported` and names the shader.
- **Drop.** `RenderInner`, `ComputeInner` and `SamplerInner` destroy their Vulkan objects. Each failure path in
  creation destroys what it made.

### Binding map (set 0, shared with `command.rs`)

| Pipeline | Resource | Binding |
|---|---|---|
| render | sampled texture `i` | `pipeline::texture_binding(i)` = `i` |
| render | sampler `j` | `pipeline::sampler_binding(textures, j)` = `textures + j` |
| compute | sampled texture `i` | `i` |
| compute | storage image `k` | `pipeline::compute_storage_binding(sampled, k)` = `sampled + k` |
| compute | sampler `j` | `pipeline::compute_sampler_binding(sampled, storage, j)` = `sampled + storage + j` |

Textures and samplers are separate descriptors (`SAMPLED_IMAGE`, `SAMPLER`), so one sampler serves many textures.
Each binding has `descriptor_count` 1.

## Contract changes

None to public signatures. Additions in `pipeline.rs`: `MAX_PUSH_CONSTANT_BYTES`, `MAX_DESCRIPTORS_PER_KIND`,
`MAX_VERTEX_STRIDE`, the binding helpers above, `RenderPipeline::descriptor()` and `ComputePipeline::descriptor()`.
In `shader.rs`: `SPIRV_MAGIC`, `Stage`, `words_from_bytes`.

## Tests

- `pipeline.rs`: blend factors, attachment state, topology, vertex input (instance and vertex rate), push constant
  validation, push ranges, descriptor validation, binding layouts (including a contiguity check over counts 0 to 4),
  sampler state, format mapping. Values are worked out by hand from the Vulkan spec.
- `shader.rs`: stage suffixes, SPIR-V word decoding (endianness, length, magic), every built shader decodes.
- `gpu-tests` feature (`pipeline::tests::gpu_render_and_compute_pipelines_build_on_the_device`): needs a real device;
  it accepts `Unsupported` when no shaders are built.

Checks run by the agent: `CARGO_TARGET_DIR=.../target-agents cargo check --offline -j 2 -p bifrost-gpu --tests` passes;
the only warnings are dead-code warnings for fields that the other owners' code does not read yet. `python3
tools/check-names.py` passes. The unit tests were not run in this round (the agent rules allow only `check`); run them
with `cargo test --offline -p bifrost-gpu` on a machine with `glslc`.

## What is left (todo_left)

- Pipeline cache persistence: the cache is in memory only. Save `vkGetPipelineCacheData` to
  `$XDG_CACHE_HOME/bifrost/gpu/` and seed the cache from it at creation, keyed by the driver UUID.
- The registry keeps each device alive for the process lifetime, because the cache must be destroyed before its
  device. A `pipeline_cache` field in `device::Inner`, destroyed in its `Drop`, would remove that; it belongs to
  `gpu-device`.
- Push descriptors (`VK_KHR_push_descriptor`) are not used: the layouts are plain set layouts. The push flag is a
  one-line change when `DeviceInfo::push_descriptors` is set.
- Blend modes beyond the three in the contract, and per-attachment blend for multiple targets (BifrOSt draws one target).
- `Keep` bookkeeping for pipelines and samplers belongs to the command owner (`command.rs`).
- Shader hot reload and specialization constants are not supported.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. On Midget, `glslc --version` must print, and the build output must show no `glslc failed` line for
   `fullscreen.vert`.
2. `cargo test --offline -p bifrost-gpu --features gpu-tests` on Midget: the pipeline test must pass. With the device
   open, `new_render_pipeline` on `quad.vert`/`quad.frag` returns `Ok`. An `Unsupported` result names the missing shader.
3. Validation layer (`VK_LAYER_KHRONOS_validation`, if installed): no errors for pipeline creation or destruction.
4. Once `gpu-command` lands: draw a fullscreen triangle with `fullscreen.vert` and a solid-colour fragment shader into
   a `Bgra8Unorm` target, read it back, and check the pixel values match the colour.
