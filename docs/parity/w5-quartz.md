# Wave 5: Quartz (PDF widgets, filter catalogue)

Branch `parity/w5-quartz`. Two gap rows of the Quartz framework are closed as partial. Both are pure Rust models
with host unit tests. Nothing is wired into the shell or an app, and nothing was built or run in this round (the
round forbids builds), so the tests are written but not executed.

## What was built

1. **Filter catalogue and parameter ranges** (`springboard/render/src/effect_chain.rs`, extended).
   - `Filter::input_range(key)` returns an `InputRange` with `min`, `max` and `default` for an input.
   - `Filter::set_clamped(key, value)` clamps a value to its range and refuses a non-finite value.
   - `Filter::category()` returns a `Category` (`ColorAdjustment`, `ColorEffect`, `Stylize`) for the browser.
   - `filter_names()`, `filters_in(category)` and `search(query)` give the browser its list: a case-insensitive
     name match, and a blank query matches all.
   - The ranges are BifrOSt's own choices (for example exposure -10 to 10 stops, saturation 0 to 2). They are not
     taken from a framework default table.
2. **PDF form fields** (`springboard/ui/src/acroform.rs`, new, registered in `springboard/ui/src/lib.rs`).
   - `Field` with kinds `Text`, `Choice`, `Button` (check box, radio button, push button) and `Signature`.
   - Flags for read-only, required, multiline, password, comb, combo box and no-toggle-to-off, using the PDF field
     flag bit positions.
   - `set_value` enforces read-only, the length limit in characters (zero means none), and the option list of a
     choice field. `set_state` enforces read-only, the button kind and the radio no-toggle rule.
   - `FieldSet` keeps unique names, `missing_required`, `export` (text always sent, a button only when on) and
     `reset_all`.
   - The radio group rule (turning one button on clears the others) is left to the form owner. `FieldSet` does
     not do it.

## Official API names mirrored

Sources: `https://developer.apple.com/documentation/pdfkit/pdfannotation` (Configuring Widget Annotations and Text
Widget Annotations sections, read through `tools/parity/docc.py pdfkit/pdfannotation`), and
`https://developer.apple.com/documentation/pdfkit/pdfannotationwidgetsubtype` (cases button, choice, signature,
text). The Core Image attribute keys come from `https://developer.apple.com/documentation/coreimage/kciattributemin`,
`.../kciattributedefault` and `.../kciattributemax`.

| Official name | BifrOSt counterpart |
|---|---|
| `PDFAnnotation.widgetFieldType` / `PDFAnnotationWidgetSubtype` (button, choice, signature, text) | `acroform::Subtype` |
| `widgetControlType` (radio, push button, checkbox) | `acroform::ButtonControl` |
| `widgetStringValue`, `widgetDefaultStringValue` | `Field::value`, `Field::reset` |
| `isReadOnly` | `Field::is_read_only` |
| `maximumLength` | `Field::limit_length`, `Field::max_length` |
| `isMultiline`, `isPasswordField`, `hasComb` | `Field::is_multiline`, `is_password`, `has_comb` |
| `buttonWidgetState`, `allowsToggleToOff` | `Field::is_on`, `set_state`, `allows_toggle_to_off` |
| `fieldName` | `Field::name`, `FieldSet::get` |
| `CIFilter` inputs (input keys) | `Filter::input_keys`, `Filter::get`, `Filter::set` (existing) |
| `kCIAttributeMin`, `kCIAttributeMax`, `kCIAttributeDefault` | `InputRange::min`, `max`, `default` |

## REA evidence

- `ev_93dcaf14a52035b6a57b468ab63fa00d33627d1bea5e0160a833b87d722f518f`: rendered PDFAnnotation page (widget
  properties listed under Configuring Widget Annotations).
- `ev_98e2533bec3dc3ac8811e772c1aecb0dd2c4e7ba8d86875a5685e2547197b6ec`: rendered Quartz framework landing page.
- `ev_f5f8a1029670edf2e3f0dfc17c8f1ca47119bff5dff04e6a1adb7ab3c72e76a5`: rendered kCIAttributeMin page.

## Matrix rows closed

These are the rows for the lead to update centrally. Status is the new status, `partial`.

| Framework slug | Feature (as written in `docs/parity-matrix/data/quartz.json`) | Status | BifrOSt path |
|---|---|---|---|
| quartz | Core Image filters and filter browser (CIFilter, IKFilterBrowserView, IKFilterUIView) | partial | springboard/render/src/effect_chain.rs |
| quartz | PDF annotations and form widgets (PDFAnnotation, PDFAppearanceCharacteristics, PDFBorder, widgets) | partial | springboard/ui/src/acroform.rs |

## Still missing

- Neither model is connected to a PDF reader. The form model is not filled from `/AcroForm` or widget dictionaries
  in `springboard/src/shell/books/engine/pdf/`.
- No view draws a field, and no app can use the browser or the form model through `bifrost_app` yet.
- No appearance characteristics (`PDFAppearanceCharacteristics`, border style, colours) on fields.
- The filter catalogue has no GPU path and no filter browser view. The renderer of the Photos app still uses
  its own five filters.
- Tests have not been run (no builds were allowed in this round).
- Quartz Composer (`QCComposition`, `QCView`) was not attempted. Its status in the official documentation was not
  checked this round.
