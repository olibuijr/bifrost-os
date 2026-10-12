# Custom kernels parity (wave 4, theme-04)

Theme: custom kernels (CIKernel, CIColorKernel, CIWarpKernel, CIBlendKernel), 34 gap rows across 13 frameworks.
This round builds one pure CPU model that covers the four kernel roles and their sampler. It closes one matrix row
honestly, as partial. The other theme rows (Metal shader libraries, MetalFX, Metal Performance Shaders, Model I/O
and the rest) are untouched.

## What was built

- `springboard/render/src/kernel.rs` (`pub mod kernel;` in `springboard/render/src/lib.rs`). It works on the renderer's
  premultiplied `u32` pixels (`gfx`, red in the low byte) and takes kernels as Rust closures.
  - `Sampler` (`new`, `fetch`, `sample`) reads an image at continuous positions. `Wrap::Edge` repeats the border pixel,
    `Wrap::Clear` makes positions outside the image transparent. `Filter::Nearest` takes the containing pixel and
    `Filter::Linear` interpolates the four nearest centres in premultiplied space, so transparent pixels do not bleed colour.
  - `color_kernel` maps every pixel's straight colour through a closure (one pixel in, one pixel out).
  - `warp_kernel` moves each destination pixel centre (x + 0.5, y + 0.5) to a source position through a closure and
    samples it. The output size is given by the caller.
  - `blend_kernel` composites two same-size buffers with `Blend`: Normal, Multiply, Screen, Overlay, HardLight, Darken,
    Lighten, Difference and Exclusion. Alpha follows source-over and colour follows the separable compositing rule.
  - Unit tests: 8 in `kernel::tests`, each with hand-worked expected values (sampler sizes and wrap rules, nearest and
    linear weights, colour inversion, mirror and shift warps, multiply and difference, half-alpha normal blend, size checks).

## Official API names mirrored

Each name is taken from its official rendered page (DocC JSON via `tools/parity/docc.py`, rendered page via REA).

- `CIKernel`: https://developer.apple.com/documentation/coreimage/cikernel. Mirrored role: a general kernel that computes
  a pixel colour. The kernel-language and Metal parts are not mirrored.
- `CIColorKernel`: https://developer.apple.com/documentation/coreimage/cicolorkernel. Mirrored by `color_kernel`.
- `CIWarpKernel`: https://developer.apple.com/documentation/coreimage/ciwarpkernel. Mirrored by `warp_kernel`.
- `CIBlendKernel`: https://developer.apple.com/documentation/coreimage/ciblendkernel. Mirrored by `blend_kernel`, with
  nine separable modes. The official built-in set is much larger (for example `source`, `destinationOver`, `hue`), so
  it is not matched.
- `CISampler`: https://developer.apple.com/documentation/coreimage/cisampler. Mirrored by `Sampler`.

## REA evidence

- Core Image landing page: `ev_87e0892fe79387ef9e3601cba99b459262a52ab2db1cbe468c750b19d3729f32`
- CIKernel: `ev_25cc9e02565e979c9bb033ed7b93b31752931cae94da0db1709e9cde84568a49`
- CIColorKernel: `ev_6a0efac380c7b4de64671dce968a88db1e3e3fb784edf475ea6f61a725c3ff8c`
- CIWarpKernel: `ev_d11f6d4d68e25a4c823685ab3acd61c97c750685f78a392a8472d1334297c9d2`
- CIBlendKernel: `ev_0bb2d37243372b9a9ccd286f6d6b57c6a9671e21d8cf7d9256d7f26117baa0df`
- CISampler: `ev_5b7d145c64586f6f84540a3b48ecf3b47c05d1d57283e394340545173524b73e` (the rendered text was empty in the
  REA output, so the DocC JSON above is the reference for its members.)

## Matrix rows closed

| Framework slug | Feature (exact, from data/core-image.json) | New status | BifrOSt path |
|---|---|---|---|
| core-image | Custom kernels (CIKernel, CIColorKernel, CIWarpKernel, CIBlendKernel) | partial | springboard/render/src/kernel.rs |

## Gaps left open

- No kernel language and no Metal kernel source: a kernel is a Rust closure, not a string that is compiled.
- No GPU path: everything runs on the CPU, and the shell compositor shaders are not reachable from apps.
- No app-facing API in `springboard/sdk/src`; the module is not wired into apps or the shell.
- No region-of-interest callbacks, no output pixel format choice, and no extent handling beyond the explicit size.
- Blend modes cover the separable set only. The non-separable modes (Hue, Saturation, Color, Luminosity) and the
  Porter-Duff compositing operators are not in the module.
- No build or test run was made in this round. The tests have been worked out by hand and are unverified until the
  next build.
