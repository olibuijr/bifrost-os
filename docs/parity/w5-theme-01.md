# Wave 5, theme-01: Core Image filters and filter browser

## What was built

`springboard/render/src/effect_chain.rs` (extended, no new module) now models the parts of the filter
catalogue that a filter browser needs, on top of the named filters from wave 4:

- `Category` (ColorAdjustment, ColorEffect) with the catalogue headings "Color Adjustment Filters" and
  "Color Effect Filters". `Filter::category()` places each filter in one of them.
- `filter_names(Option<Category>)` lists the names in a category, or all names with `None`, in catalogue order.
- `title_of(name)` and `Filter::title()` give the display name of a filter.
- `search(query)` returns the filters whose name or display name contains the query, ignoring case. An empty
  query lists everything. This is the lookup a browser's search field uses.
- `InputAttribute` (key, default, min, max, identity). `Filter::attribute(key)` and `Filter::attributes()`
  report the range each input slider covers and the value that leaves pixels unchanged (`None` for posterize,
  which has no such value).
- `Filter::set_clamped(key, value)` limits a value to its declared range and returns the stored value.
  `Filter::set` is unchanged and still stores any value.
- `Filter::reset_inputs()` restores the defaults, as a browser's Reset button does.

The ranges, display names and category placement are BifrOSt's own. They follow the catalogue's category
names, but the individual ranges are chosen here and are not copied from Apple. Vignette is placed under
Color Effect here; the official catalogue page does not settle that placement in the text checked.

## Apple APIs mirrored (official documentation)

- `CIFilter`: the catalogue category names, and the class method `filterNames(inCategories:)` that lists
  filter names by category. Official page: https://developer.apple.com/documentation/coreimage/cifilter-swift.class
- Core Image Filter Catalog: "Color Adjustment Filters" and "Color Effect Filters" headings.
  Official page: https://developer.apple.com/documentation/coreimage

Apple's per-input attribute dictionary (range, default, identity) is mirrored in shape only. I did not read the
attribute key names from the official reference, so none are claimed here.

## REA evidence

- `ev_0bda6d3fd105f51d595a696263db8e890c527dc5737ca99133648a018d4c2e68`: Core Image landing page, which lists
  the Filter Catalog categories and the CIFilter class.
- `ev_926fafefb4e42b8ddea65c8a1fc638575c81683bab61e07b14b39af01b552203`: CIFilter page, which shows the
  `filterNames(inCategories:)` class method.

## Matrix rows closed

| Framework slug | Feature (as written in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| quartz | Core Image filters and filter browser (CIFilter, IKFilterBrowserView, IKFilterUIView) | partial | springboard/render/src/effect_chain.rs |

## Not closed, and why

- No filter browser view or panel. IKFilterBrowserView, IKFilterBrowserPanel, IKFilterUIView and
  IKFilterCustomUIProvider need a UI, which this round did not build. The row stays partial.
- The module is pure and host-tested only. It is not wired into any app or the shell, and it has not run on the
  VM or Midget (the wave 4 note still applies).
- Render destinations, image accumulators, feature detection and colour profile rows are in other themes.
- No builds were run for this round, per the task rules. The tests are written, not executed.

## Tests

Ten tests in `effect_chain.rs` (the five from wave 4 plus five new ones): catalogue categories and order,
search by name and display name, attribute ranges and identity, clamped sets, and reset to defaults.
