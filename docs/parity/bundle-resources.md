# Bundle Resources: parity notes

Official reference: https://developer.apple.com/documentation/bundleresources (macOS 10.0 and later).
Matrix row: `docs/parity-matrix/data/bundle-resources.json` (status stays `partial`).

## What was built

`springboard/ui/src/privacy.rs` (`bifrost_ui::privacy`), a pure model of the privacy declarations an app ships:

- `PrivacyManifest::parse` reads a `privacy.toml` in the same `key = value` style as `manifest.toml`, with
  `[[collected]]` and `[[accessed_api]]` tables. Unknown keys, unknown tables and keys outside their table are errors.
- `PrivacyManifest::validate` checks that tracking domains are bare host names, that no data kind is declared twice,
  and that every required-reason API category gives at least one reason.
- `PrivacyManifest::tracks` is true when the app or one data kind is marked as tracking.
- `PrivacyManifest::undeclared_apis` returns the API categories the code uses that the manifest does not declare,
  in call order. A build step or the store review can use it to check a declaration against real use.

Seven unit tests in the module cover parsing, rejection of malformed input, host validation, duplicate and
reasonless declarations, tracking detection and the undeclared-API check.

## Official names it mirrors

From the Bundle Resources pages (`privacy-manifest-files`, `app-privacy-configuration`,
`describing-data-use-in-privacy-manifests`, `describing-use-of-required-reason-api`):

- Privacy manifest files and the App Privacy Configuration root object.
- Tracking (`NSPrivacyTracking`), tracking domains (`NSPrivacyTrackingDomains`), collected data types
  (`NSPrivacyCollectedDataTypes`) and accessed API types (`NSPrivacyAccessedAPITypes`).

The Rust code does not use those identifiers (owner rule); the mapping is by concept.
The format keys are `tracking`, `tracking_domains`, `[[collected]]` and `[[accessed_api]]`.

## REA evidence

- `ev_597463437fe308ebc974fa21ad6536470389e429debc8daa434b99d4470115b2`: rendered page
  `https://developer.apple.com/documentation/bundleresources/privacy-manifest-files` (title "Privacy manifest files").
- `ev_245e8b8557d7a64dd4d2b2b588dff39b3c6f7443c6959b6b4c47f2a11448de12`: the Bundle Resources framework row
  (from the parity matrix, unchanged).

## Remaining gaps

- The shell does not load `privacy.toml` from installed `.aap` packages. Nothing shows the declaration to people
  and the store does not check it.
- The official lists of collected data types and required-reason API categories are not mirrored. Kinds and
  categories are free strings, so `undeclared_apis` only compares names the caller supplies.
- The `Info.plist` identity keys (`CFBundleIdentifier`, `CFBundleShortVersionString`, `CFBundleVersion`) were not
  fetched from the official reference in this round, so no version or identifier format rules were added.
- Universal links (applinks), URL-scheme routing for non-web schemes, and documented placement rules for
  frameworks and plug-ins are still open (see the matrix gaps).

## Verification status

Not built or tested in this round by design; the build and test stage runs `cargo test` for `bifrost-ui`.

## Universal links: association file model

`springboard/sdk/src/bundle_resources.rs`, built on parity/team-a-06 (not yet verified on Midget).

- `AssociationFile::parse(text)` reads a domain association file (the well-known file a host serves over https). It takes the `applinks.details` entries: app ids (`appIDs` or
  `appID`), path rules (`components` with a `/` pattern and `exclude`), and the older `paths` list where a leading
  `NOT ` excludes. Details without an app id or a usable rule are dropped.
- `AssociationFile::route(path, installed)` returns the first installed bundle id that a detail names and whose path
  rules allow. Details are tried in file order; within a detail the first matching rule decides. A pattern uses `*`
  for any run of characters and `?` for one character. A component that also has a query or fragment rule is
  dropped when it includes and kept as a path rule when it excludes.
- App ids match a bare bundle id, or a ten-character team id followed by a dot and the bundle id.
- `split_https(url)` returns the lowercased host and the path of an https URL (port and user info dropped, parsing shared
  with `AppLinks` in `springboard/core/src/app_links.rs`), and None for other schemes. `AppLinks` is a separate registry of
  host and prefix claims and is not connected to this model.

Not done: fetching the file over https, checking that the host serves the file for the app, query-item rules, and
routing https links to apps in the shell. The browser still claims http and https (springboard/src/shell/defaultapps.rs).

## Package placement rules

`springboard/sdk/src/package_layout.rs` (`bifrost_app::package_layout`), built on parity/team-n-bundle-resources (not yet verified on Midget).

- `classify(name, exec)` places one archive entry: `manifest.toml`, `icon.png` and the `exec` binary at the top level;
  anything under `assets/`; flat `libs/<name>.so` shared libraries and `plugins/<name>.so` plug-ins.
- Nested library or plug-in folders, names without a `.so` stem, and unknown top-level files are refused. Names with an
  absolute path, an empty component, `.`, `..` or a backslash are refused as unsafe.
- `check_package(names, exec)` checks a whole listing, collects every problem, rejects duplicate names and a non-bare
  `exec`, and requires the manifest and the executable.

Not done: `install_archive` in `springboard/src/shell/store/install.rs` does not call the check yet. Wiring it in is a
follow-up, because existing packages must be checked first.

## Container folders

`springboard/sdk/src/container_layout.rs` (`bifrost_app::container_layout`), built on parity/team-n-bundle-resources (not
yet verified on Midget).

- `Folder` names the four fixed folders: `Documents`, `Library`, `Library/Caches` and `tmp`. `Documents` and `Library`
  are backed up and never purged; `Library/Caches` and `tmp` are purgeable and not backed up.
- `ContainerLayout::resolve(relative)` maps an app path under the container root and refuses empty, absolute and `..`
  paths.
- `ContainerLayout::ensure()` creates the folders in parent-first order and refuses a symlink or a file in place of a
  folder, so an app cannot redirect its container.

Not done: the shell does not create these folders for installed apps or move existing data into them.
`file_manager.rs` (`SearchPathDirectory`) has a home-directory mapping for the desktop file APIs; it is separate and was
not changed.

## Tests

Unit tests cover the classification and check rules in `package_layout.rs`, and the path rules, backup and purge policy,
and the symlink and file refusals in `container_layout.rs`. The filesystem tests use `tempfile`. None were run in this
round: the lead builds and runs `cargo test -p bifrost-app` later.
