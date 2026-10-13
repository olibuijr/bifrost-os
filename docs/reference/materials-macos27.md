# Material recipes of the macOS 27 reference (build 26A434)

Measured numbers behind BifrOSt's materials. Everything here is a number, a name or a behaviour in our own words; no
reference file, XML, artwork or shader text is copied. The values feed `springboard/wm/src/material_kind.rs`,
`springboard/wm/src/material_filter.rs`, `springboard/render/src/glass.rs` and the `MATERIAL_*` pairs in
`springboard/render/src/ui.rs`.

Status of each statement is marked: **measured** (read from a recipe file), **derived** (arithmetic on measured
values, shown), **inferred** (a judgement from names or behaviour; could be wrong), **designed** (ours, no reference).

## Evidence and its limits

- Source: the extracted system payload (`~/Projects/macos-ref/extract/ref27/`, `ref27-index.tsv`). The recipe and
  style-set files are property lists; 54 recipe files and 28 style-set files exist in the payload (tables below).
- **Evidence ids.** The REA tools could not give a per-value evidence id for these files in this session:
  `inspect_plist` needs a macOS host (`capability_unavailable`, "Native macOS utilities require macOS"), and
  `open_binary` refuses a `.materialrecipe` (unsupported format). `open_binary` did open the framework's `Info.plist`
  (sha256 prefix `cb5837c1592e`, bundle `CoreMaterial`, executable `CoreMaterial`). So the evidence id used below is the
  **sha256 prefix (12 hex) of each file**, which pins the exact bytes read; the values were read with a property list
  parser. When a macOS-capable REA host is available, run `inspect_plist` on the same files and replace these ids.
- **No binaries.** The payload extraction holds resources only; the dyld shared cache is not extracted (the `dyld/`
  directory is empty). The framework executables, the AppKit visual-effect view and the SwiftUI material code
  therefore could not be disassembled, and the string references that would prove which public material uses which
  recipe could not be read. Every public-name to recipe mapping below is **inferred** from recipe names, value
  ordering and known behaviour, and is marked so.
- Framework version file: product "Materials", build version 1140 (`version.plist`).

## Vocabulary of a recipe

A recipe has a `baseMaterial` with up to three parts, plus references to style sets.

| Key | Meaning (our reading) | Our model |
|---|---|---|
| blurRadius | Gaussian-equivalent blur radius in points | `Recipe::blur` |
| backdropScale | Scale the backdrop is captured and blurred at, as a fraction of the output (0.25 means a quarter-size copy) | `Recipe::backdrop_scale`; GLES path `glass::DIV` 4 |
| saturation | Colour saturation applied to the blurred backdrop; absent means 1 | `Recipe::saturation_dark/light` |
| brightness | Offset added to every channel after the other filters | `Tone::brightness` |
| luminanceAmount | How far the pixel's luma is pulled towards the luminance curve (0 none, 1 fully) | `Tone::luminance_amount` |
| luminanceValues | Four control values of the luminance curve; they may leave 0..1 (negative values occur) | `Tone::luminance_values` |
| blurAtEnd | Order of the blur relative to the colour stages: yes means the colour stages run first and the blur last | not modelled (see below) |
| averageColorEnabled | The backdrop is replaced by its average colour (no blur): the Reduce Transparency form | the opaque plate (`Recipe::plate_*`) |
| colorMatrix | A 5x4 matrix (offsets in the fifth column) applied to the backdrop instead of the saturation and luma stages | decomposed, see "Glass" |
| curvesValues | Per-channel tone curves (four values each) | not used by any desktop material |
| zoom | Slight scale-up of the backdrop | not used by any desktop material |
| tinting (tintColor, tintAlpha) | A flat veil over the blur | only in the tvOS-flavoured variants (suffix `~appletv`); no desktop recipe has one |
| styles.fill / styles.stroke | Names of the style sets used for content drawn on the material | see "Style sets" |

Descendant recipes (`.descendantrecipe`) override single keys of an ancestor: a Reduce Transparency layer-one
override lowers the brightness of the reduced dark chrome to -0.04, and a shadowed thick-light override raises the last
luminance value from 0.905 to 0.955.

The appearance pairing is by file name: `...Dark` and `...Light` are the two appearances of one material. The only
state variants found are the Reduce Transparency pair of the chrome (the other regular materials ship no reduced
variant) and the dedicated override for a shadowed thick-light platter. No per-window-state (active/inactive) variants
exist in the recipe files: **inactive-window behaviour is therefore not in these files**; our opaque inactive plates
are derived (below).

## All recipes (measured)

Values are the plist values; "-" means the key is absent (the recipe relies on the default). `blur at end` yes means
`blurAtEnd` is true.

| Recipe file | sha256 (12) | blur | backdrop scale | saturation | brightness | luma amount | luma values | blur at end | other |
|---|---|---|---|---|---|---|---|---|---|
| coaching-material.materialrecipe | c42efd5a8460 | 20 | - | 1 | 0 | - | - | yes | - |
| weather-material-alternate.materialrecipe | 4d2045d87da5 | 0 | - | 2 | -0.02 | 0.35 | [0.25, 0.1, 0.666667, 0.33] | no | - |
| weather-material.materialrecipe | 44070ff56c53 | 15 | - | 2 | 0 | 0.4 | [0.25, 0.1, 0.666667, 0.33] | yes | - |
| timerDark.materialrecipe | 0abdc5ecc25f | 30 | 0.25 | - | - | - | - | yes | 5x4 colour matrix |
| timerYellow.materialrecipe | fd58448e1f3f | 30 | 0.25 | - | - | - | - | yes | 5x4 colour matrix |
| MultimodalNavigationChrome.materialrecipe | 97901805e88d | 25 | 0.125 | 1 | 0 | 0 | - | yes | - |
| ambientCompact.materialrecipe | b5681b783e4b | 40 | 0.25 | - | - | - | - | yes | RGB curves [0, 0.7, 0.88, 0.88] |
| carPlayPlatters.materialrecipe | 3fa1ef899f2f | 15 | - | - | - | - | - | - | tint white 0.95 alpha 0.65 |
| carPlayPlattersDark.materialrecipe | ae82f41e70b9 | 15 | - | - | - | - | - | - | tint white 0.1 alpha 0.65 |
| dockDark.materialrecipe | a9626a1417f8 | 30 | 0.25 | 1.6 | - | 0.5 | [0.29, -0.2, 0.375, 0.65] | yes | - |
| dockLight.materialrecipe | 34ac4791aea2 | 30 | 0.25 | 1.8 | 0.08 | 0.5 | [0.3, 0.5, 1, 0.77] | yes | - |
| knowledgeBackground.materialrecipe | bd5d8125c22a | 29.5 | 0.45 | 1.35 | - | 0.47 | [0, 0.13, 0.13, 0] | yes | fill platterFillLight, stroke platterStrokeLight |
| knowledgeBackgroundDark.materialrecipe | 4376f89a3224 | 29.5 | 0.45 | 1.45 | - | 0.57 | [-0.1, 0.03, 0.03, -0.1] | yes | - |
| knowledgePlatters.materialrecipe | 41f13e68d27c | 15 | 0.25 | - | - | - | - | yes | 5x4 colour matrix; fill platterFillLight, stroke platterStrokeLight |
| knowledgePlattersDark.materialrecipe | bd7e4c9b5357 | 15 | 0.25 | - | - | - | - | yes | 5x4 colour matrix; fill platterFillDark, stroke platterStrokeDark |
| knowledgePlattersSheer.materialrecipe | cac4f7686c6e | 30 | 0.25 | 2.4 | - | 0.6 | [0.775, 0.85, 1.05, 0.94] | yes | fill platterFillLight, stroke platterStrokeLight |
| knowledgePlattersSheerDark.materialrecipe | 166ccbad9f62 | 30 | 0.25 | 1.4 | -0.03 | 0.4 | [0.41, -0.4, 0.3, 0] | yes | fill platterFillDark, stroke platterStrokeDark |
| modules.materialrecipe | d1286a8c20f5 | - | - | - | - | - | - | - | 5x4 colour matrix; fill moduleFill, stroke moduleStroke |
| modulesBackground.materialrecipe | 94842da02b87 | 25 | - | 1.6 | - | 0.4 | [0.38, 0, 1, 0.76] | - | zoom 0.04; fill moduleFill, stroke moduleStroke |
| platformChromeDark.materialrecipe | 483edae9b3fb | 22.5 | 0.25 | 2 | -0.1 | 0.75 | [0.23, 0.52, 0.27, 0.255] | yes | fill platformChromeFillDark, stroke platformStrokeDark |
| platformChromeDarkReduceTransparency.materialrecipe | 85d9b490116b | - | 0.25 | - | -0.1 | 0.85 | [0.225, 0.31, 0.25, 0.2] | - | average colour; fill platformChromeFillDark, stroke platformStrokeDark |
| platformChromeLight.materialrecipe | b007fa3d0d39 | 22.5 | 0.25 | 1.1 | 0.1 | 0.75 | [0.8, 0.9, 1.1, 0.825] | yes | fill platformChromeFillLight, stroke platformStrokeLight |
| platformChromeLightReduceTransparency.materialrecipe | 88b1f5f59e19 | - | 0.25 | 0.8 | 0.1 | 0.75 | [0.95, 0.9, 1.1, 0.825] | - | average colour; fill platformChromeFillLight, stroke platformStrokeLight |
| platformContentDark.materialrecipe | 1488cdf66d56 | 30 | 0.25 | 1.5 | 0 | 0.75 | [0.16, 0.26, 0.1, 0.1] | yes | fill platformFillDark, stroke platformStrokeDark |
| platformContentDark~appletv.materialrecipe | 7094decd7e47 | 50 | 0.625 | 1.6 | - | - | - | - | tint white 0 alpha 0.5; fill platformFillDark, stroke platformStrokeDark |
| platformContentGlass.materialrecipe | 589fc5c5795c | 45 | - | - | - | - | - | - | 5x4 colour matrix |
| platformContentLight.materialrecipe | 685572626bdb | 30 | 0.25 | 1.5 | 0.1 | 0.75 | [0.9, 0.83, 0.925, 0.815] | yes | fill platformFillLight, stroke platformStrokeLight |
| platformContentLight~appletv.materialrecipe | f0334a12caf6 | 50 | 0.625 | 1.6 | - | - | - | - | tint white 1 alpha 0.4; fill platformFillLight, stroke platformStrokeLight |
| platformContentThickDark.materialrecipe | a5106facecc5 | 45 | 0.25 | 1.5 | 0 | 0.88 | [0.14, 0.16, 0.1, 0.03] | yes | fill platformFillDark, stroke platformStrokeDark |
| platformContentThickDark~appletv.materialrecipe | d9ff21b38a96 | 50 | 0.625 | 1 | - | - | - | - | tint white 0 alpha 0.8; fill platformFillDark, stroke platformStrokeDark |
| platformContentThickLight.materialrecipe | fb6ac1347061 | 45 | 0.25 | 1.5 | 0.045 | 0.88 | [0.99, 0.95, 0.98, 0.905] | yes | fill platformFillLight, stroke platformStrokeLight |
| platformContentThickLight~appletv.materialrecipe | 662d6091708e | 50 | 0.625 | 1 | - | - | - | - | tint white 1 alpha 0.6; fill platformFillLight, stroke platformStrokeLight |
| platformContentThinDark.materialrecipe | 51ee21d6b416 | 30 | 0.25 | 1.35 | 0 | 0.6 | [0.2, 0.21, 0.1, 0.15] | yes | fill platformFillDark, stroke platformStrokeThinDark |
| platformContentThinDark~appletv.materialrecipe | 6b8ba0b09a72 | 50 | 0.625 | 1.4 | - | - | - | - | tint white 0 alpha 0.32; fill platformFillDark, stroke platformStrokeThinDark |
| platformContentThinLight.materialrecipe | ccf9d9d851af | 30 | 0.25 | 1.35 | 0.12 | 0.6 | [0.725, 0.825, 0.76, 0.73] | yes | fill platformFillLight, stroke platformStrokeThinLight |
| platformContentThinLight~appletv.materialrecipe | c6af3c40fead | 50 | 0.625 | 1.8 | - | - | - | - | tint white 1 alpha 0.3; fill platformFillLight, stroke platformStrokeThinLight |
| platformContentUltraThinDark.materialrecipe | 9728f4dd6f6e | 22.5 | 0.25 | 1.1 | 0 | 0.5 | [0.24, 0.24, 0.3, 0.39] | yes | fill platformFillDark, stroke platformStrokeThinDark |
| platformContentUltraThinDark~appletv.materialrecipe | 0f6887413ca9 | 50 | 0.625 | 1.3 | - | - | - | - | tint white 0 alpha 0.08; fill platformFillDark, stroke platformStrokeThinDark |
| platformContentUltraThinLight.materialrecipe | 190ed5ac0d27 | 22.5 | 0.25 | 1.1 | 0.12 | 0.5 | [0.45, 0.55, 0.65, 0.68] | yes | fill platformFillLight, stroke platformStrokeThinLight |
| platformContentUltraThinLight~appletv.materialrecipe | 7e7bda0deeeb | 50 | 0.625 | 1.3 | - | - | - | - | tint white 1 alpha 0.06; fill platformFillLight, stroke platformStrokeThinLight |
| previewBackground.materialrecipe | e0ececeacb18 | 11.2 | - | - | - | - | - | - | tint white 0 alpha 0.088; zoom 0.024 |
| tintablePlatters.materialrecipe | 272752bf1133 | 30 | 0.25 | 1.8 | - | - | - | yes | fill platterFillLight, stroke platterStrokeLight |
| toolbarButtonBackground.materialrecipe | 00590a57c2bc | 15 | 0.25 | 1.1 | - | 0.5 | [0.24, 0.24, 0.3, 0.39] | yes | - |
| BubbleBackgroundDarkMaterial.materialrecipe | cd40c4aff88c | 30 | - | 3 | 0.4 | 0.3 | [0.3, 0.6, 0.6, 1] | yes | - |
| BubbleBackgroundLightMaterial.materialrecipe | fc662b79585d | 30 | - | 3 | - | 0.3 | [0.3, 0.6, 0.6, 1] | yes | - |
| platterGlass.materialrecipe | 078b237e21e1 | 45 | - | - | - | - | - | yes | 5x4 colour matrix |
| editingButtonDark.materialrecipe | 415ca25e20f4 | 30 | 0.125 | 1.6 | -0.02 | 0.5 | [0.29, -0.2, 0.375, 0.65] | yes | - |
| editingButtonLight.materialrecipe | 3e871358784f | 30 | 0.125 | 1.8 | 0.12 | 0.5 | [0.3, 0.5, 1, 0.77] | yes | - |
| fullScreenBannerBackground.materialrecipe | 8264583b61f4 | 30 | 0.25 | - | - | - | - | - | tint white 0 alpha 0.25; zoom 0.05 |
| homeScreenLegibility.materialrecipe | 459f534c4aee | 25 | 0.25 | - | - | - | - | yes | tint white 0 alpha 0.05 |
| modulesBackgroundSheer.descendantrecipe (override of modulesBackground) | 357dd791f57d | - | - | - | - | - | - | - | - |
| modulesSheer.descendantrecipe (override of modules) | 1098c429aa99 | - | - | - | - | - | - | - | - |
| platformChromeDarkReduceTransparencyLayer1.descendantrecipe (override of platformChromeDarkReduceTransparency) | 99f0e7ae274b | - | - | - | -0.04 | - | - | - | - |
| platformContentThickLightShadowed.descendantrecipe (override of platformContentThickLight) | f6b512c05419 | - | - | - | - | - | [0.99, 0.95, 0.98, 0.955] | - | - |

Observations (measured): the desktop-relevant family is `platformChrome*` and `platformContent*`; every one of them
uses backdrop scale 0.25 and blurs at the end. The `~appletv` variants are the only recipes with a flat tint and use a
different recipe shape (blur 50, backdrop scale 0.625), so they are not used for desktop materials. The others (weather,
timer, photos editing, car, home-screen legibility, banner) belong to single applications.

## Style sets (content drawn on the material)

A style set gives the colours of the primary, secondary, tertiary and quaternary content (labels, glyphs, strokes) on a
material. On the desktop (no suffix) they are **vibrant colour matrices**, not flat colours: each style is a 5x4 matrix
that remaps the backdrop's colour so text stays legible and picks up the material's colour. The `~appletv` sets are the
flat versions: white or black tint with alpha steps (stroke 1, 0.5, 0.3, 0.2 dark; 1, 0.6, 0.4, 0.2 light) blended with
a plus-lighter (dark) or plus-darker (light) compositing filter.

| Style set file | sha256 (12) | entries | tint alpha per entry | compositing filter | vibrant matrix entries |
|---|---|---|---|---|---|
| baseRingDark.visualstyleset | 510f9fd3b96e | 1 | 1 | plusL | 0 |
| baseRingLight.visualstyleset | 37d115de1f4d | 1 | 1 | plusD | 0 |
| highContrastStyle.visualstyleset | ab194e1fffd4 | 2 | 1, 1 | - | 0 |
| moduleFill.visualstyleset | 50b423e2ba1e | 7 | 1, 1, 1, 0.6, 0.12, 0.12, 0.05 | plusD, plusL | 2 |
| moduleStroke.visualstyleset | 7b5031777c78 | 5 | 1, 1, 1, 1, 1 | - | 2 |
| platformColorsLight.visualstyleset | 2198f94c0ede | 5 | 1, 1, 1, 1, 1 | - | 5 |
| platformFillDark.visualstyleset | c8e95263e515 | 4 | 1, 1, 1, 1 | - | 4 |
| platformFillDark~appletv.visualstyleset | 10f6d91d22a8 | 4 | 1, 1, 1, 1 | plusL | 0 |
| platformFillLight.visualstyleset | f92637a691b2 | 4 | 1, 1, 1, 1 | - | 4 |
| platformFillLight~appletv.visualstyleset | 6fb5295955bf | 4 | 1, 1, 1, 1 | plusD | 0 |
| platformStrokeDark.visualstyleset | 0ff306669750 | 4 | 0.96, 1, 1, 1 | - | 3 |
| platformStrokeDark~appletv.visualstyleset | 4ccd78bca9e3 | 4 | 1, 0.5, 0.3, 0.2 | plusL | 0 |
| platformStrokeLight.visualstyleset | 924acd4b502d | 4 | 0.96, 1, 1, 1 | - | 3 |
| platformStrokeLight~appletv.visualstyleset | 95a8a7904c93 | 4 | 1, 0.6, 0.4, 0.2 | plusD | 0 |
| platterClientStyle.visualstyleset | db20bfe499d6 | 6 | 0.6, 0.5, 0.13, 0.05, 0.5, 0.42 | plusD | 0 |
| platterFillDark.visualstyleset | 5a78dc4f47e4 | 2 | 1, 1 | - | 2 |
| platterFillLight.visualstyleset | b3bdb3aed37c | 2 | 1, 1 | - | 2 |
| platterStrokeDark.visualstyleset | 204d76556138 | 4 | 0.9, 1, 1, 1 | - | 3 |
| platterStrokeLight.visualstyleset | 41c64299fb39 | 4 | 0.9, 1, 1, 1 | - | 3 |
| platterVibrantShadowDark.visualstyleset | 2fdc5d821c06 | 1 | - | - | 1 |
| platterVibrantShadowLight.visualstyleset | d901985a9660 | 1 | - | - | 1 |
| widgetGalleryPlatterDark.visualstyleset | 71646a207692 | 1 | 1 | plusD | 0 |
| widgetGalleryPlatterLight.visualstyleset | 4b5b7530f8a2 | 1 | 1 | plusD | 0 |
| platformChromeFillDark.descendantstyleset | 48aa382dade4 | 1 | 1 | - | 1 |
| platformChromeFillLight.descendantstyleset | bcee674f2c90 | 1 | 1 | - | 1 |
| platformColorsDark.descendantstyleset | 71e83eae60be | 4 | 1, 1, 1, 1 | - | 4 |
| platformStrokeThinDark.descendantstyleset | 6d1efd3b749b | 2 | 1, 1 | - | 2 |
| platformStrokeThinLight.descendantstyleset | 5241487f265b | 2 | 1, 1 | - | 2 |

Consequence for `MATERIAL_*`: the desktop style sets do not contain flat fill or stroke colours to copy, so the
`MATERIAL_*` pairs could not be set from them. They are set from the recipes instead (see "Flat stand-ins"), and the
vibrancy matrices are recorded here as data for the vibrancy work (`render/src/vibrancy.rs`).

## Recipes per material kind

The shared model kinds are `springboard/ui/src/visual_effect.rs` `MaterialKind`; window-server kinds are
`wm/src/material_kind.rs` `MaterialKind`. Saturation and tone are Dark / Light. All use backdrop scale 0.25.

| BifrOSt kind | Recipe pair (Dark / Light) | Blur | Saturation | Brightness | Luma amount | Luma values Dark | Luma values Light | Mapping status |
|---|---|---|---|---|---|---|---|---|
| titlebar, headerView | platformChromeDark / platformChromeLight | 22.5 | 2.0 / 1.1 | -0.1 / +0.1 | 0.75 | [0.23, 0.52, 0.27, 0.255] | [0.8, 0.9, 1.1, 0.825] | inferred (the chrome recipe is the only one that also has a Reduce Transparency pair, as the bars do) |
| menu, popover, sheet | platformContentDark / platformContentLight | 30 | 1.5 | 0 / +0.1 | 0.75 | [0.16, 0.26, 0.1, 0.1] | [0.9, 0.83, 0.925, 0.815] | inferred (the regular thickness) |
| windowBackground, contentBackground | platformContentThick* | 45 | 1.5 | 0 / +0.045 | 0.88 | [0.14, 0.16, 0.1, 0.03] | [0.99, 0.95, 0.98, 0.905] | inferred |
| sidebar, underPageBackground | platformContentThin* | 30 | 1.35 | 0 / +0.12 | 0.6 | [0.2, 0.21, 0.1, 0.15] | [0.725, 0.825, 0.76, 0.73] | inferred |
| underWindowBackground | platformContentUltraThin* | 22.5 | 1.1 | 0 / +0.12 | 0.5 | [0.24, 0.24, 0.3, 0.39] | [0.45, 0.55, 0.65, 0.68] | inferred |
| hudWindow, toolTip, fullScreenUI | platformContentThickDark in both appearances | 45 | 1.5 | 0 | 0.88 | [0.14, 0.16, 0.1, 0.03] | same | inferred (these materials are dark whatever the appearance) |
| glass regular | platformContentGlass | 45 | decomposed: 1.5885 | veil, see below | - | - | - | derived from the colour matrix; the name matches |
| glass clear | none | 10 (designed) | 1.4 (designed) | - | - | - | - | designed, unmeasured |
| selection | none (accent colour, not a blur) | 12 (designed) | 1.6 (designed) | - | - | - | - | designed, unmeasured |
| shell dock (`ShellSurface::Dock`) | dockDark / dockLight | 30 | 1.6 / 1.8 | 0 / +0.08 | 0.5 | [0.29, -0.2, 0.375, 0.65] | [0.3, 0.5, 1.0, 0.77] | name match; the dock keeps kind `underWindowBackground` in the shared model, the window server reads `dock_recipe()` |

Why the thickness ladder is inferred: the recipe names order the materials by thickness (ultra thin, thin, regular,
thick) and their numbers agree (luminance amount 0.5, 0.6, 0.75, 0.88 and blur 22.5, 30, 30, 45 rise with thickness).
Which public name sits on which rung is not in the files. The mapping above follows the platform's documented
thickness order for the public materials (sidebar and under-page thin, popover and menu regular, window and content
backgrounds thick, under-window ultra thin). It is an estimate until the code that selects a recipe by name can be
read (needs the dyld cache; see "What could not be measured").

### Shell surfaces (`ShellSurface` in `ui/src/material_request.rs`)

| Shell surface | Model kind | Recipe used |
|---|---|---|
| Spotlight, ControlCentre, NotificationPanel, OverviewPill | hudWindow | thick, dark |
| MenuBar, SettingsToolbar | headerView | chrome |
| MenuBarMenu, ContextMenu | menu | regular |
| Dock | underWindowBackground | dock recipes (dedicated pair) |
| NotificationBanner, Popover | popover | regular |
| Sheet | sheet | regular |
| Tooltip | toolTip | thick, dark |
| SettingsSidebar | sidebar | thin |
| OverviewVeil, AppGridVeil | fullScreenUI | thick, dark |

## Filter math (derived)

Order of stages on the blurred backdrop pixel (straight RGB), every stage clamped to 0..1. Pure reference in
`wm/src/material_filter.rs`; the shaders match it (`gpu/shaders/compose.frag`, `wm/src/glass.rs`, `src/gpu/wl.rs`).

1. Saturation `s`: `luma + (rgb - luma) * s`, luma weights (0.2126, 0.7152, 0.0722). **Derived:** the platform's glass
   colour matrix has off-diagonal terms in the ratio 0.7152 : 0.2126 (-0.265 : -0.079 = 3.35, the weight ratio is 3.36)
   and 0.0722 : 0.7152, so the Rec.709 weights are what the filters use.
2. Luminance mapping: `rgb += amount * (curve(luma) - luma)`, where `curve` is the cubic Bezier through the four
   luminance values, `(1-x)^3 v0 + 3(1-x)^2 x v1 + 3(1-x) x^2 v2 + x^3 v3`. **Inferred.** What is measured: four values
   and an amount per recipe, and a 256 x 1 RGBA lookup image (`luminanceColorMap.png`, sha256 prefix `ac977ed4a0e1`)
   shipped beside them, which suggests the curve is baked to a 256-entry table at run time. What is inferred: that the
   four values are Bezier control values on an equally spaced luma axis, and that the amount mixes the curve with the
   identity additively per channel. The inference fits the data: the light recipes (values near 0.8 to 1.1) lift dark
   backdrops towards light and the dark recipes (0.1 to 0.5) pull everything towards dark, the negative dock values
   overshoot mid-tones as a Bezier with a negative control value does.
3. Brightness: `rgb += brightness`.

Hand-computed checks (also unit tests): dark chrome on black gives 0.0725, on mid grey 0.29266; light chrome on white
gives 0.96875, on black 0.7.

`blurAtEnd` yes means the colour stages run on the downsampled backdrop before the blur. For a linear stage this is the
same as running after; the saturation and luma stages are not linear (clamping, the curve), so the difference is
small but real. We run the stages after the blur (cheaper: one pass on the final pass); this is a known deviation.

### Reduce Transparency

Only the chrome has a reduced pair: `averageColorEnabled` replaces the blur with the backdrop's average colour, with the
dark pair's amount 0.85 and values [0.225, 0.31, 0.25, 0.2] (brightness -0.1), the light pair's amount 0.75, values
[0.95, 0.9, 1.1, 0.825], saturation 0.8, brightness +0.1. We do not average the backdrop (a flat opaque plate is
used instead): **plate = the tone applied to a neutral grey backdrop**, luma 0.15 under Dark and 0.85 under Light
(our choice of a typical wallpaper per appearance; the recipes have no fixed backdrop). Kinds without a reduced recipe
use their normal tone for the plate (**derived**, flagged "plate" below).

| Kind | Plate Dark (0..255) | Plate Light (0..255) | Recipe used for the plate |
|---|---|---|---|
| titlebar, headerView, glass regular | 35.3 | 255 | chrome reduced pair |
| menu, popover, sheet | 45.7 | 242.6 | regular (no reduced recipe) |
| windowBackground, contentBackground, hud, toolTip, fullScreenUI | 36.9 | 246.7 (Dark value in both for dark-only kinds) | thick |
| sidebar, underPageBackground | 45.5 | 231.3 | thin |
| underWindowBackground | 50.2 | 223.4 | ultra thin |
| dock | 36.6 | 234.3 | dock |

The `MATERIAL_*` pairs in `render/src/ui.rs` use these greys (rounded) with the earlier alpha.

### Glass (regular)

The glass recipe has blur 45 and a colour matrix with no saturation key. Measured matrix rows (R, G, B; offset): row
R = (0.921, -0.265, -0.027), G = (-0.079, 0.735, -0.027), B = (-0.079, -0.265, 0.973), offsets 0.235, alpha row
identity. **Derived decomposition:** the matrix equals `gain * saturation(s) + offset` with s = 1.5885, gain = 0.6295,
offset = 0.235 (all three rows, to the printed precision). A gain below one plus an offset is exactly "saturate, then
put a grey veil of alpha 1 - gain = 0.3705 and grey 0.235 / 0.3705 = 0.6343 over it", which the existing glass pipeline
already does (saturation plus a premultiplied tint). So no matrix stage is needed. Unit test:
`the_glass_colour_matrix_is_a_saturation_gain_and_offset`.

## Flat stand-ins in the shell (`MATERIAL_*`)

Where the shell cannot sample a blurred backdrop it paints a flat translucent colour. Those colours were "derived, not
yet measured". They are now the recipe applied to the neutral backdrop (table above), neutral grey (the recipes carry no
hue), alpha unchanged from the earlier design. Not changed: `MATERIAL_SELECTION*` (accent colours, no recipe).

## What could not be measured

- The mapping of public material names to recipe names (needs the framework and AppKit/SwiftUI executables, which are
  not in the extraction; see "Evidence and its limits"). Marked inferred above.
- The exact luminance curve definition (the four values' role) and the order relative to saturation. The runtime that
  consumes the values is not extracted.
- Active versus inactive window behaviour of window materials; our opaque plates are derived, not measured.
- The Gaussian-equivalent unit of `blurRadius` against our dual-Kawase reach; we use the numbers as logical px and let
  the frame-time budget (`BLUR_TEXEL_BUDGET`) bound them.
- Noise or grain of the materials (no recipe key); `Recipe::noise` stays a designed 0.02.
- The `blurAtEnd` ordering (deviation noted above).
- Clear glass, the selection plate, and the plain background-effect blur have no recipe.
- Per-channel curves and zoom exist in two single-app recipes only; no desktop material uses them.
- Pixel-level comparison against a reference capture was not done (no capture in this task).

## Parity rows this moves

Evidence is the file hashes above plus the unit tests named in the commits. Rows remain "built, not parity" until the
change is verified on Midget; nothing here was run (code and commit only).

- AppKit materials and vibrancy: per-kind blur, saturation, brightness and luminance mapping now measured; the vibrancy
  matrices of the style sets are recorded as data, not yet applied.
- Visual effect material catalogue: all fourteen kinds map to a recipe or are marked designed (clear glass, selection).
- SwiftUI Liquid Glass: the regular glass recipe is decomposed exactly; clear glass remains designed.
