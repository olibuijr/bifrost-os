# Wave 3, theme-22: preview extensions and file-manager extensions

Branch `parity/w3-theme-22`, on top of `parity/integration` (waves 1 and 2).

## What was built

Pure models in `springboard/ui/src` (bifrost-ui). No SDK wiring, no shell drawing and no app uses yet. None of the
code has been compiled or run (the wave rules forbid builds); the tests are written and hand-checked only.

- **`preview.rs` (extended)**: the view-based preview subject, which the system calls with a file URL or with a
  searchable item identifier.
  - `Subject::file(url)` takes an absolute path or a `file://` URL that names a file; `Subject::searchable(identifier,
    query)` takes an indexed item identifier and drops a blank query.
  - `Registry::register_searchable(provider, domain)` maps a domain to a provider. An identifier matches a domain when it
    equals the domain or starts with the domain and a `/`. The longest matching domain wins.
  - `Registry::provider_for_subject(&Subject)` resolves a file subject through the existing type registry, so a
    `QLPreviewProvider` registration and a view-based preview share one lookup.
  - `ModelError::BadUrl` is new.
- **`fileaction.rs` (extended)**: `Placement` (`ItemsMenu`, `ContainerMenu`, `Toolbar`). An `ActionRegistry` can now place
  a registered action in a placement, unplace it, and list the actions shown in a placement that apply to a selection
  (`placed_for`, reusing the existing type rule). `toolbar_actions()` ignores the selection. Unregistering an action or an
  owner also drops its placements. The container menu takes the folder's type identifier as its item.
- **`filebadge.rs` (new)**: `BadgeSet`, the badge model of a file-manager extension. `set_image` registers or replaces a
  badge (identifier, image, label). `set_badge` marks an absolute path with a registered badge. `badge_for`, `clear` and
  `request` (the file manager's question for one path) complete the model. Paths are normalized: trailing slashes go, the
  root stays `/`.
- **`itemattrs.rs` (new)**: `Store`, the per-item tags and last-used date. `set_tags` trims, dedupes and orders labels,
  with a 64-character limit per label and 32 tags per item. `set_last_used` stores seconds since the Unix epoch. `values`
  returns only the attributes that are set, in the order asked. An item with neither is forgotten.

## Official API names mirrored

- QuickLook UI: `QLPreviewingController` (`preparePreviewOfFile(at:completionHandler:)`,
  `preparePreviewOfSearchableItem(identifier:queryString:completionHandler:)`, `providePreview(for:completionHandler:)`).
  Reference: https://developer.apple.com/documentation/quicklook/qlpreviewingcontroller
- Finder Sync: `FIFinderSync`, `FIFinderSyncController` (`setBadgeIdentifier(_:for:)`, `setBadgeImage(_:label:forBadgeIdentifier:)`,
  `selectedItemURLs()`, `targetedURL()`, `tagDataForItem(with:)`, `setTagData(_:forItemWith:completion:)`,
  `lastUsedDateForItem(with:)`, `setLastUsedDate(_:forItemWith:completion:)`), and `FIMenuKind`
  (`contextualMenuForItems`, `contextualMenuForContainer`) with `toolbarItemName`, `toolbarItemImage` and
  `toolbarItemToolTip` from `FIFinderSyncProtocol`.
  References: https://developer.apple.com/documentation/findersync and
  https://developer.apple.com/documentation/findersync/fifindersynccontroller

Official DocC pages read with `tools/parity/docc.py`: `quicklook/qlpreviewingcontroller`, `findersync`,
`findersync/fifindersynccontroller`. The `findersync/fifindersync` DocC path returned 404, so its abstract comes from the
rendered page of the framework landing.

## REA evidence

- `ev_0f4f2ffff42a58be09a91c2124e0c12e887fcb8c7ffa6ab9378876ac22c2f94f`: QLPreviewingController
- `ev_460c92b9c9a004aad7ab8f212af5dda6d278544a6dcccb95f0cdc39a8c091389`: Finder Sync framework landing
- `ev_408164e56b1f68b376bf29698a7939159bcdfe4ba834c07a95d85736ba49e5b7`: FIFinderSyncController

## Matrix rows closed (status partial)

| framework slug | feature | BifrOSt path |
|---|---|---|
| quicklook-ui | Preview extensions (QLPreviewingController) | springboard/ui/src/preview.rs (Subject, Registry::register_searchable, Registry::provider_for_subject) |
| finder-sync | Badges on files and folders | springboard/ui/src/filebadge.rs (BadgeSet) |
| finder-sync | Custom shortcut menus in the file manager | springboard/ui/src/fileaction.rs (Placement::ItemsMenu, Placement::ContainerMenu, placed_for) |
| finder-sync | Toolbar buttons in the file manager | springboard/ui/src/fileaction.rs (Placement::Toolbar, toolbar_actions) |
| finder-sync | Per-item tags and last-used data shared with the Finder | springboard/ui/src/itemattrs.rs (Store) |

All five are partial because the models are not wired: nothing in the SDK or the file manager calls them yet, no
extension host exists, and the toolbar item has no image or tooltip. The Quick Look panel and the thumbnail rows are not
changed by this wave.

## Not changed

- `quicklook-ui` "Data-based preview extensions (QLPreviewProvider)": the model already sits in `preview.rs` from an earlier
  wave, and this wave does not change it.
- `finder-sync` rows for selection and target folder, extension enablement and directory observation: no model added.
