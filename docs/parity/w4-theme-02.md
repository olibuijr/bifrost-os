# Wave 4, theme-02: resource management, document types and shared lists

Theme: "Resource management (nib, storyboard, bundle)" (42 gap rows across 27 frameworks). This wave closes four
of the closest rows with pure Rust modules, std only. Nib and storyboard files are not built: they are a
declarative UI format, and the shell builds UI in Rust code.

## What was built

| Module | What it does | Mirrors (official API names) |
|---|---|---|
| `springboard/sdk/src/bundle_resources.rs` (`ResourceBundle`) | Finds a named resource in an app package. Localized copies in `<lang>.lproj/` come before package files. Language order: region, language, `Base`, `en`. Names cannot leave the package (no `..`, no absolute paths). | Bundle resource loading: `Bundle` URL lookup by resource name, extension and subdirectory, and the placement rules in "Placing content in a bundle". |
| `springboard/ui/src/utdecl.rs` (`Declared`, `Declaration`) | App-declared exported and imported types. Checks reverse-DNS identifiers, refuses the `public.` and `dyn.` namespaces, lowercase extensions, duplicate ids, unknown parents and cycles. Conformance runs through declared types and then the built-in tree in `uttype`. Exported declarations win over imported ones for an extension, which win over built-in types. | Declared exported and imported type identifiers, and the conformance query (`conforms(to:)`) over them. |
| `springboard/ui/src/shared_list.rs` (`SharedList`) | Recent items and favourites for a document list: most recent first, a cap on recents, favourites never dropped by the cap, favourites listed first. | Shared file list (recent items and favourites). Model only; no system-wide shared list. |
| `springboard/ui/src/accessible_look.rs` (`Preferences`) | Decisions from four display settings: reduced motion (movement becomes a cross-fade of at most 120 ms), increased contrast (text minimum 7.0 instead of 4.5), reduced transparency (fills become opaque), and differentiation without colour (a second mark is needed). WCAG 2 contrast ratio. | Accessibility display settings: reduce motion, increase contrast, reduce transparency, differentiate without colour. |

Unit tests: 6 in `utdecl.rs`, 5 in `bundle_resources.rs`, 6 in `shared_list.rs`, 6 in `accessible_look.rs`.
Expected values were worked out by hand from the rules the code implements (for example, #767676 on white is
about 4.54:1, #777777 on white is about 4.48:1, black on white is 21:1).

Not run: no build or test was run in this round (repo rule for this wave). The tests have not been compiled.

## Official evidence

- Bundle Resources collection page: `ev_672728ebdd595d73c3dc6dc80cf3405be2b13fd95097b3162ce71fd82a37390e`
  (https://developer.apple.com/documentation/bundleresources; sections Property Lists, Structure "Placing content in a bundle").
- Uniform Type Identifiers framework page: `ev_631e24c802bbd1e88eaa8598271401671014a6b811caa54d2325731c14bfae0b`
  (https://developer.apple.com/documentation/uniformtypeidentifiers; "Defining file and data types for your app", conformance by subtype).
- Accessibility landing page: `ev_b88470e871d8b1a9dcfbb8fe9cef1aea4cc7f3dc109ab115cce8218683f45518`
  (https://developer.apple.com/documentation/accessibility). Used for the accessibility row. The per-symbol pages
  (`accessibilityReduceMotion` and similar) were not fetched in this round.

The DocC JSON fetch (`tools/parity/docc.py`) worked for the `bundleresources` and `uniformtypeidentifiers` collection
pages. The `foundation/nsbundle` path returned an HTTP error in this round.

## Matrix rows closed

Status is `partial` on every row: the model exists, but the system-level counterpart (nib and storyboard files,
system-wide recent lists, the live accessibility settings feed) is not built.

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| appkit-app-structure | Resource management (nib, storyboard, bundle) | partial | springboard/sdk/src/bundle_resources.rs |
| core-services | Shared file lists (recent items, favourites) | partial | springboard/ui/src/shared_list.rs |
| uniform-type-identifiers | App-declared exported and imported types | partial | springboard/ui/src/utdecl.rs |
| swiftui-events-a11y-integration | Accessible appearance | partial | springboard/ui/src/accessible_look.rs |

## Gaps left open

- Nib and storyboard files: no declarative UI format; the shell builds UI in Rust.
- Shared file lists: the model is in-process only. The shell does not store or share the list across apps.
- Declared types: the shell does not yet read them from the app manifest or register them with the file manager.
- Accessibility: the decisions exist, but the renderer does not yet call them for every transition.
- Bundle resources: no asset-catalogue format and no on-demand resource requests.
