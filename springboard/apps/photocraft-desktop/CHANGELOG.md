## 0.6.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.10.0

- Upstream photocraft v0.10.0.
  ## Scorecard since v0.9.0
  From [`docs/scorecard.md`](https://github.com/storytold/photocraft/blob/v0.10.0/docs/scorecard.md) (done / partial / missing):
  - **Tools:** 29 / 13 / 1 → **33 / 11 / 0**. Done: per-control pressure curves, image files as brush tips, Pen Shift 45° constraints, arrow-key nudges bundled into one History state.
  - **File compatibility:** 7 / 13 / 7 → **7 / 14 / 6**; PDF save went from missing to partial (one flattened page with ICC).
  - **Corpus floors raised:** Photoshop oracles 145 → **188** / 256, psd-tools 236 → **249** / 309, io corpus 146 → **148** / 170 (round-trip floors unchanged).

## 0.9.1

- Upstream photocraft v0.9.0.
  ## What's Changed
  * Use craft-print for system printing by @skylartaylor in https://github.com/storytold/photocraft/pull/3872
  * Tests: allow-list language-invariant bit-depth labels, add missing tr/hu 'Enable animations' (#4359) by @vangel76 in https://github.com/storytold/photocraft/pull/4362
  * feat(automation): allow Crop and Straighten Photos through automation (#1052) by @buivanvu94 in https://github.com/storytold/photocraft/pull/3917
  * Pencil: no 8-bit dither on aliased strokes, one flat colour at partial opacity by @cloudalister in https://github.com/storytold/photocraft/pull/3990

## 0.9.0

- Upstream photocraft v0.9.0.
  ## What's Changed
  * Use craft-print for system printing by @skylartaylor in https://github.com/storytold/photocraft/pull/3872
  * Tests: allow-list language-invariant bit-depth labels, add missing tr/hu 'Enable animations' (#4359) by @vangel76 in https://github.com/storytold/photocraft/pull/4362
  * feat(automation): allow Crop and Straighten Photos through automation (#1052) by @buivanvu94 in https://github.com/storytold/photocraft/pull/3917
  * Pencil: no 8-bit dither on aliased strokes, one flat colour at partial opacity by @cloudalister in https://github.com/storytold/photocraft/pull/3990

## 0.8.3

- Upstream photocraft v0.8.2.
  ## What's Changed
  * Adopt shared docking while preserving panel controls and workspaces by @skylartaylor in https://github.com/storytold/photocraft/pull/3097
  * GPU composite: reuse unchanged lower-stack chunks for property edits by @Elvis33LE in https://github.com/storytold/photocraft/pull/1169
  * Design: bounded GPU stack prefix reuse for property edits by @Elvis33LE in https://github.com/storytold/photocraft/pull/1164
  * feat(ui): allow disabling Studio canvas background dots by @Ni-zav in https://github.com/storytold/photocraft/pull/2317

## 0.8.2

- Upstream photocraft v0.8.2.
  ## What's Changed
  * Adopt shared docking while preserving panel controls and workspaces by @skylartaylor in https://github.com/storytold/photocraft/pull/3097
  * GPU composite: reuse unchanged lower-stack chunks for property edits by @Elvis33LE in https://github.com/storytold/photocraft/pull/1169
  * Design: bounded GPU stack prefix reuse for property edits by @Elvis33LE in https://github.com/storytold/photocraft/pull/1164
  * feat(ui): allow disabling Studio canvas background dots by @Ni-zav in https://github.com/storytold/photocraft/pull/2317

## 0.7.0

- Upstream photocraft v0.7.0.
  ## What's Changed
  * docs: register the MCP server from an installed release by @sawizzle in https://github.com/storytold/photocraft/pull/2185
  * docs(adjust): align Levels and Curves comments with evaluation order by @coygeek in https://github.com/storytold/photocraft/pull/2212
  * Preserve unchanged XMP translations in File Info by @Benziza in https://github.com/storytold/photocraft/pull/2152
  * Smudge: use the Brush Settings spacing for retouch strokes (#2140) by @davidmann4 in https://github.com/storytold/photocraft/pull/2171

## 0.6.0

- Initial BifrOSt App Store package of upstream photocraft v0.6.0.
