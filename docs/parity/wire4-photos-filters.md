# Wire 4: Photos editing (adjust, effects, blur, vignette)

Branch `wire4/photos-filters`, from `main` (0.5.40). Wires the Core Image style models from the renderer into the
Photos app's edit mode. Nothing was built or run in this round (the round forbids builds); the tests are written with
hand-worked expected values but have not been executed. The lead builds once.

## What is now wired

The edit mode (`springboard/src/shell/photos/edit.rs`, `mod.rs`) now has three tool pages under a tab row:

| Page | Sliders | Model behind it |
|---|---|---|
| Adjust | Brightness, Contrast (existing), Exposure, Saturation, Warmth (new) | `effect_chain` filters `exposure` (ev = slider x 2 stops), `saturation` (amount = 1 + slider), `temperatureTint` (temperature = slider); brightness and contrast stay in `edit::adjust` |
| Effects | Blur (new), Vignette (new) | `blur_kernels::disc_blur` with radius = round(slider x 3) px; `effect_chain` filter `vignette` (strength = slider) |
| Crop | Straighten, Crop (existing) | unchanged |

The Filters row gains a sixth chip, **Sepia**, backed by the `sepia` filter of `effect_chain` (amount 1). The existing
Original, Vivid, Mono, Noir and Warm looks are unchanged.

**Revert to Original** already existed (`EditRevert` resets `Edit::default()`). It now also resets the new sliders,
because `is_identity` checks every new field. The bar shows "Edit" when the picture is untouched and "Revert" otherwise.

Order of the pixel work (`Edit::apply`): rotate, straighten, crop, filter, blur, brightness and contrast, then the
chain of exposure, saturation, warmth and vignette. Each chain step is skipped when its slider is at rest, so an
untouched slider costs nothing. Saving writes "NAME edited.png" beside the original, as before.

## How a user reaches it

1. Open Photos, pick a picture, and tap **Edit** in the viewer's bar.
2. The panel has four rows: Rotate and aspect presets; the filter chips (Original to Sepia); the tab row
   **Adjust | Effects | Crop**; then the sliders of the chosen tab (up to five).
3. Drag a slider; the preview updates live. **Revert** in the top bar clears every change. **Save** writes the copy.

## How to verify on the test PC (Midget)

1. Deploy the lead's build to Midget (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget open-app photos`, then open a picture and tap Edit (`bifrost pc midget tap-window Photos X Y`
   with window-local pixels from a `state` read).
3. Tap the **Effects** tab, drag **Blur** to the right: the picture softens. Drag **Vignette** right: the corners darken.
4. Tap the **Adjust** tab: drag **Exposure** right (brighter), **Saturation** left (greyer), **Warmth** right (warmer).
5. Tap **Sepia** in the filter row: the picture takes a brown tone. Tap **Revert**: the bar reads "Edit" again.
6. Save and check that "IMG edited.png" appears beside the original.
7. Screenshot with `bifrost pc midget screen PATH` and check the panel against the design rules (CONTENT_BG, MAC_* sizes).

## Host tests added (not run)

In `edit.rs`:
- `exposure_saturation_and_warmth_use_the_catalogue_maths`: 64 with +1 stop gives 128; red at saturation -1 gives
  grey 76 (0.299 x 255); warmth +1 on grey 100 gives (125, 100, 75).
- `blur_radius_steps_and_the_disc_average`: radius 0, 2 (for 0.5), 3 (for 1.0 and clamped 3.0); a 3x3 image with one
  white centre gives 51 at the centre (255 over the five pixels of radius 1, rounded).
- `vignette_darkens_the_corners_not_the_centre`: strength 1 on a 4x4 white picture gives a black corner and an
  untouched centre, alpha kept.
- `sepia_turns_white_into_the_warm_tone`: white gives (255, 255, 239) (blue row 0.937).
- The existing `Filter::ALL.len()` assertion moves from 5 to 6.

In `mod.rs`, `the_editor_previews_edits_and_saves_a_copy` now also taps the Effects tab and drags Blur to its
right end, and asserts the pane and the blur value. Brightness stays in slot 0 of the Adjust page, so the existing
brightness tap is unchanged.

## REA evidence

No new REA capture was made in this round. The rows below rely on the existing evidence in `docs/parity/w4-core-image.md`:
`ev_3f6a2488755d9e326cf62f04ba77ddb6e1cdfeaa73283049d55fd0711cbcd517` (rendered Core Image landing page: chaining
filters; Filter Catalog with Blur Filters and Composite Operations).

## Remaining gaps

- **Blend modes and composite operations**: not wired. `blend.rs` has the twelve modes but the Photos edit has no blend
  control. Still missing: a layer or second-image blend.
- **Gaussian and motion/zoom blur**: only the disc blur is used. Motion and zoom blur (`blur_kernels`) are not exposed.
  No sigma-based gaussian in the edit.
- **Hue, tone curve, temperature range**: the catalogue's `temperatureTint` range is -100..100, but the colour function
  reads its input as -1..1 (x0.25 per unit). Photos uses the -1..1 reading directly, so `set_clamped` with the catalogue
  range would give a much stronger effect. The catalogue range should be corrected in `effect_chain.rs` by its owner.
- **Core Graphics colour spaces**: the picture is treated as straight sRGB. No colour-managed input or colour space
  conversion is used (`imgdec` decodes JPEG and PNG only).
- **Filter chips**: Vivid, Mono, Noir and Warm are still the shell's own per-pixel code, not catalogue filters.
- **SDK**: no app can build or chain these filters yet. `springboard/sdk/src/` has no filter type.
- **Not built, not tested on Midget**: all code is unverified until the lead builds and runs the Photos tests.

## Matrix rows touched (for the lead to import; `docs/parity-matrix/data/core-image.json`)

| Slug | Feature (as written in the matrix) | Status | BifrOSt path |
|---|---|---|---|
| core-image | Colour adjustment (exposure, brightness, contrast, saturation, hue, temperature, tone curve) | partial | springboard/src/shell/photos/edit.rs |
| core-image | Photo and colour effects (sepia, mono, noir, vignette, false colour, posterize) | partial | springboard/src/shell/photos/edit.rs |
| core-image | Blur filters (gaussianBlur, boxBlur, motionBlur, zoomBlur, discBlur) | partial | springboard/src/shell/photos/edit.rs (disc blur via springboard/render/src/blur_kernels.rs) |
| core-image | Filter objects and chaining (CIFilter) | partial | springboard/render/src/effect_chain.rs (run by springboard/src/shell/photos/edit.rs) |
