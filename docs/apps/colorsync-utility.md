# Colour Utility

Colour Utility (`bifrost-colour-utility`, app id `is.olibuijr.colour-utility.desktop`) lists the ICC colour profiles on
the system and inspects them. It is a full-size window with a list of profiles on the left and four panels on the right.

## Where profiles come from

- System: `/usr/share/color/icc` and `/usr/local/share/color/icc`.
- User: `~/.local/share/color/icc` and `~/.color/icc`.

Files with an `.icc` or `.icm` extension are listed, searched four folder levels deep. The list reads directory entries
only; a profile's bytes are read when it is selected. A file larger than 64 MB is refused.

## Panels

- Header: the size, version, device class, colour space, connection space, creation date, CMM, platform, flags,
  manufacturer, model, creator, rendering intent, PCS illuminant, profile ID and description.
- Tags: each entry of the tag table with its name, data type, offset and size. A tag whose data lies outside the file is
  shown with its signature in red.
- Verify: findings from the structural checks, each as an error or a warning. The checks cover the header signature
  and version, the device class, the colour space and connection space, the tag table and each tag's bounds, alignment
  and overlap with the table, duplicate tags, the tags a matrix, greyscale or LUT profile must carry, and the data types
  of colorant, white point and curve tags.
- Gamut: for an RGB matrix profile, the red, green and blue primaries and the white point, plotted on the CIE 1931 xy
  chart with the spectral locus and the sRGB primaries for reference. The readout gives the area of the profile's
  triangle and its ratio to sRGB. LUT-based, CMYK and greyscale profiles have no colorant tags and show a note instead.

## Keys and menus

- File > Refresh Profiles (Command-R) searches the folders again. The selection stays on the same file if it is still
  there.
- View > Header, Tags, Verify, Gamut (Command-1 to Command-4).
- Up and Down move the selection in the list.

Touch: tap a profile to select it, tap a tab to change the panel, drag the list or the panel body to scroll.

## Limits

- The gamut is drawn from colorant tags only. A LUT-based profile (A2B0 and its relatives) is checked but has no gamut
  here.
- The colorants of a profile are D50-adapted, so the sRGB ratio is an estimate, a few per cent off for a display profile.
- Verification is structural. It does not check that a transform built from the profile is correct, and it does not
  read the curve data beyond its type.
- Long messages in the verification list are cut with an ellipsis rather than wrapped.
- The app does not write or install profiles.

## Code

- `src/model.rs`: the profile model, the checks, the text tags and the gamut. Pure, with unit tests that build profile
  files in memory.
- `src/library.rs`: finds the profile files in the folders above.
- `src/window.rs`: state, layout and hit testing.
- `src/draw.rs`: painting.
- `src/main.rs`: the `App` implementation (touch, keys, menus, drawing).
