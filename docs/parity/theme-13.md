# Theme 13: colour lookup and conversion

Colour cube, colour map, Lab conversion and palettize, built as pure Rust in `springboard/render`. The
modules are pure per-colour models with no hardware, no GPU and no image objects.

## What was built

- `springboard/render/src/colorcube.rs`: `ColorCube::lookup_extrapolated`. The existing `lookup` clamps
  channels to 0..=1. The new method extends the cube linearly from its edge samples, which mirrors the
  `extrapolate` option of a colour cube filter. Shared sampling moved into a private `sample`.
- `springboard/render/src/colorlab.rs` (new): `Lab` with `from_rgb` and `to_rgb`, and `delta_e` (CIE 1976).
  sRGB decodes through `colorspace::Space::Srgb`. The white point is D65. The reverse path clamps
  out-of-gamut values.
- `springboard/render/src/colormap.rs` (new):
  - `ColorMap`: a one-dimensional gradient. A colour's Rec. 601 luma (from `colorfx::Rgb::luma`) selects a
    position along it, and the entries are interpolated.
  - `Palette`: snaps each colour to its nearest entry, in RGB or, when `perceptual` is set, in L*a*b*.
  - `palettize_pixel` and `map_pixel` work on packed 0xAARRGGBB pixels and keep alpha.

## Official Apple API names mirrored

Taken from the official DocC pages (`python3 -I tools/parity/docc.py coreimage/<name>`):

| Official name | Mirrored by |
|---|---|
| `CIColorCube` (`cubeData`, `cubeDimension`, `extrapolate`) | `ColorCube::new`, `identity`, `lookup`, `lookup_extrapolated` |
| `CIColorMap` (`gradientImage`) | `ColorMap` (gradient entries replace the gradient image) |
| `CIPalettize` (`paletteImage`, `perceptual`) | `Palette::new(colours, perceptual)` |
| `CIConvertLab` | `Lab::from_rgb`, `Lab::to_rgb` (no image filter wrapper; the `normalize` input is not modelled) |

The `paletteImage` input of `CIPalettize` is documented as the output of a k-means clustering filter. This
theme takes the palette colours directly and does not generate them.

## REA evidence

Each row is a rendered official page, captured with `tools/parity/rea-page.sh`:

- Core Image landing: `ev_c9dfa40eac54cd8200e3cae23e8f60be04542acfbce4cf3b49d8d253651c5161`
- CIColorCube: `ev_3cb537594d25dc92c0a619a16da984393b7127acb9ba00a8aa901e07d6bfd891`
- CIColorMap: `ev_d16d6d5fe6e84fd54aa0349b11db05a0a3a19972374bb8f0acc0ccc2d43d44df`
- CIPalettize: `ev_be53c3e5b10c5bd44f18242fc20f1075fdec52dd3b8d8d7dabfc6d6aafec8420`
- CIConvertLab: `ev_8df960e7aa1d5e4031e10d907647e1f0925fd1a179d4a2a7e4179492a8ef9e88`

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| core-image | Colour lookup and conversion (colour cube, colour map, Lab, palettize) | partial | `springboard/render/src/colorcube.rs`, `springboard/render/src/colorlab.rs`, `springboard/render/src/colormap.rs` |

## Still missing

- No `CIImage` or `CIFilter` object: the colour cube, colour map and palette work on values and packed
  pixels, not on a lazy image graph.
- No palette generation (the k-means step that produces `paletteImage`).
- No `CIColorCubeWithColorSpace` (the cube's input colour space) and no `CIConvertRGBtoLab`
  (Lab as image channel values).
- No `normalize` option for Lab output, no wide-gamut Lab, and no ICC-profile conversion.
- No GPU or Metal kernel path.

## Tests

`cargo` was not run, per the wave rule. Each new or changed module has its own `#[cfg(test)] mod tests`:
`colorlab` (6 tests), `colormap` (7 tests), and `colorcube` (one new extrapolation test, plus the six
existing ones).
