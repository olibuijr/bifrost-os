# Wave 5: AppKit (Text) parity: font descriptors and the user's font choice

Framework: AppKit (Text), `docs/parity-matrix/data/appkit-text.json`.

## What was built

`springboard/sdk/src/font_descriptor.rs` (`bifrost_app::font_descriptor`), std only, no unsafe:

- `FontDescriptor`: mandatory attributes for a font lookup (family name, point size, fixed pitch), with a builder
  (`new`, `with_family`, `with_size`, `with_fixed_pitch`), accessors, `accepts`, `matching` and `first_match` over a
  list of installed faces. The family compares without case; faces come back in library order. A size that is not
  finite or not above zero is rejected. An empty family name clears the requirement.
- `UserFonts`: the app-facing side of Settings > Fonts. `system_path` and `mono_path` return the file the role draws
  with (the Settings override when the file exists, else the packed asset). `is_replaced` reports whether the user
  changed a role. `choices` lists BifrOSt's font library (one face per family). `resolve` applies a descriptor to
  the choices.

It extends the existing `springboard/render/src/fontprefs.rs` (`font_path`, `installed`, `library`) and the
`springboard/render/src/font.rs` renderer. No module was duplicated; `fontprefs` is not changed.

Limits: the renderer draws whole files and `fontprefs::installed` keeps one face per family, so a descriptor cannot
select a weight, a slant or a variation axis. The reference descriptor's symbolic traits, matrix, design and
feature dictionaries are not mirrored. Requirements are matched on family and pitch only.

## Official API mirrored

- `NSFontDescriptor` (https://developer.apple.com/documentation/appkit/nsfontdescriptor): init(name:size:),
  withFamily(_:), withSize(_:), matchingFontDescriptors(withMandatoryKeys:), matchingFontDescriptor(withMandatoryKeys:),
  pointSize, and the family/size attributes of fontAttributes.
- Fonts topic (https://developer.apple.com/documentation/appkit/fonts): the user font choice that the system applies
  to every app.

## REA evidence

- AppKit landing page: `ev_d2b38efcddb295029b850be747154f028564f9f41459a4923a2af8da26dabb4f`
  (https://developer.apple.com/documentation/appkit)
- NSFontDescriptor page: `ev_ecb6ea1d6467cbcb049af617ae45374de4d1a2e2da7ce0c0540855293fe0b7e6`
  (https://developer.apple.com/documentation/appkit/nsfontdescriptor)

## Matrix rows

| Framework slug | Feature (as in data/appkit-text.json) | New status | BifrOSt path |
|---|---|---|---|
| appkit-text | Fonts: user font choice | partial (was os) | springboard/sdk/src/font_descriptor.rs (UserFonts) |
| appkit-text | Fonts: font rendering engine | partial (descriptor matching added; still partial) | springboard/sdk/src/font_descriptor.rs (FontDescriptor) |

Remaining gaps in the appkit-text matrix that this wave does not touch: the multi-line text view widget, the
TextKit layout manager and containers, the Writing Tools integration, the text finder wiring, and font collections.
