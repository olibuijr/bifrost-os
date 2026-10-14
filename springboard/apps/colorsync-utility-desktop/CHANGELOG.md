# Changelog

## 0.1.0

- New app: Colour Utility lists the ICC colour profiles in the system folders (`/usr/share/color/icc`,
  `/usr/local/share/color/icc`) and in the user's folders (`~/.local/share/color/icc`, `~/.color/icc`).
- Header: the profile size, version, device class, colour space, connection space, creation date, platform,
  manufacturer, model, creator, rendering intent and PCS illuminant.
- Tags: every entry of the tag table with its name, data type, offset and size. A tag that lies outside the file is
  marked.
- Verify: structural checks on the header, the tag table, the tag bounds and alignment, duplicate tags, required tags
  of matrix and greyscale profiles, and the data types of colorant and curve tags.
- Gamut: the red, green and blue primaries and the white point of a matrix RGB profile, drawn on the CIE 1931 xy
  chart with the spectral locus, and the area against sRGB. LUT-based profiles report that they carry no colorants.
