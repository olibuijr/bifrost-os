# Wire 5: launch services, default apps and URL schemes (wire5/url-schemes)

Scope: the Core Services and Bundle Resources rows about registering apps, URL schemes and the default app per type,
plus Uniform Type "open and share targets by type". The pure model (`bifrost_ui::launch::Registry`, `Handler`, `Rank`,
`Choice`) and the Default Apps page already existed (`docs/default-apps.md`). This round connects the remaining
pieces: a read-only service for apps, a link opener that routes every scheme through the registry, a Settings row
for `mailto:`, and the SDK client. Nothing was built or run (the lead builds once). Every expected value in the tests
was worked out by hand from the code.

## What is now wired

1. **`launch` system service** (`springboard/src/shell/services/launch_services.rs`, registered in `services::all()`).
   Three read-only methods, all answered at once:
   - `open_with` `{name}` answers `{apps: [{app, name, default}]}`: every app that can open the file's type, the
     default first, with display names. Built on `Registry::open_with` through `Shell::open_with_choices`.
   - `default_for_file` `{name}` answers `{app}` (null when nothing opens it), through `Shell::default_app_for_file`.
   - `default_for_scheme` `{scheme}` answers `{app}`, through `Shell::default_app_for_scheme`.
   An unknown method is an error. Apps cannot set defaults through the service; the user chooses them in Settings.
2. **SDK client** (`springboard/sdk/src/launch_services.rs`, `bifrost_app::launch_services`): `open_with`,
   `default_for_file`, `default_for_scheme` send the calls; `candidates` and `default_app` decode the replies
   (`Candidate { app, name, is_default }`). A candidate without a name is shown as its id and is never the default.
3. **Links go through the registry** (`Shell::desktop_open_link` in `defaultapps.rs`). The control socket's
   `open-url` request (used by `bifrost-open-url`) now accepts any URL with a scheme, not only http and https
   (`springboard/src/bare/control.rs`, `packaging/arch/wayland/open-url`). The shell decides: web links go to the
   browser, `mailto:` or `bifrost-app:` go to the app that declares the scheme (`Route::App`), local and script schemes
   (`file`, `javascript`, `data`, `blob`, `vbscript`, `about`) are refused. The existing `workspace::route_url` does the
   routing; the only change there is that `workspace_open_url` is now `pub(super)` so `defaultapps` can call it.
4. **Settings: Default Apps gets an Email group** (`CATEGORIES`, scheme `mailto`). It lists the apps that declare
   `schemes = ["mailto"]` and sets the scheme default with Change All. No app declares it yet, so the row reads
   "No app available", which is the honest state on this system.

## How a user reaches it

- Settings > General > Default Apps: one group per family (Web Browser, Images, Audio, Video, Text, PDF Documents,
  Archives, and now Email). Pick an app in a group to make it the default for the whole family.
- Links from a terminal or script: `bifrost-open-url https://example.org` opens the browser as before;
  `bifrost-open-url mailto:someone@example.org` goes to the mail app's handler once one is installed.
- Apps: `bifrost_app::launch_services::open_with(ctx, "notes.txt")` asks which apps open a file; the reply arrives in
  `App::service_reply`.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Settings: open General > Default Apps. Expect a group "Email" after "Archives" that reads "No app available".
   Expect the Images group to show Photos checked, as before.
3. Choose Files in the Images group. Expect Files checked, and Photos unchecked. Restart the shell: Files is still
   checked (the choice is kept in `default.apps`).
4. Links: from `ssh midget`, run `bifrost-open-url https://example.org`. Expect the browser to open the page.
   Run `bifrost-open-url file:///etc/passwd`. Expect `error: Unsupported link`.
   Run `bifrost-open-url gopher://example.org`. Expect `error: No app opens this link`.
   Run `bifrost-open-url mailto:someone@example.org`. Expect `error: No app opens this link` until a mail app declares
   `schemes = ["mailto"]`.
5. The service itself is covered by host tests (`cargo test` in `springboard`: `launch_services` and `defaultapps`).
   On Midget, an app that calls `launch_services::open_with` can show the reply in its own window.

## REA evidence

- `ev_cf47282d5c54fa94e4f8950b2db5f05fa544948d2f09f585695fc65eca2a7dcd`: rendered official page
  `https://developer.apple.com/documentation/coreservices/launch_services` (Locating an App: the default app for a
  URL or content type, the candidate list, `LSCanURLAcceptURL`; Registering an App: `LSRegisterURL`; Role handlers and
  `LSSetDefaultHandlerForURLScheme`). `docc.py coreservices/launch_services` lists the same symbols.
- `ev_ada821e13873e0992599418f926fc8eb085101ef47513bd56111510f0971e686`: rendered official page
  `https://developer.apple.com/documentation/appkit/nsworkspace` (the "Opening URLs" and "Managing Open Configurations"
  topics: `urlForApplication(toOpen:)`, `urlsForApplications(toOpen:)`, `open(_:withApplicationAt:...)`).
  `docc.py appkit/nsworkspace` returned the same topic list. The page
  `appkit/nsworkspace/urlsforapplications(toopen:)` is not in the public documentation index (404), so it is cited
  through the overview only.

## Matrix rows

The rows are not edited in this round (`docs/parity-matrix/` is shared). The statuses below are the proposal for the
database update.

| slug | feature (as in the matrix json) | BifrOSt path | proposed status |
|---|---|---|---|
| core-services | Registering apps and URL schemes | springboard/src/shell/defaultapps.rs (manifest_handler, registry, desktop_open_link); springboard/src/shell/services/launch_services.rs; springboard/src/bare/control.rs (open-url) | partial (no database API an app can call; registration still comes from manifests) |
| core-services | Locating the default app for a file or type | springboard/ui/src/launch.rs (Registry::default_for_type); springboard/src/shell/defaultapps.rs (default_app_for_file); springboard/src/shell/services/launch_services.rs (default_for_file) | parity |
| core-services | Listing every app that can open an item | springboard/ui/src/launch.rs (open_with); springboard/src/shell/services/launch_services.rs (open_with); springboard/sdk/src/launch_services.rs (open_with, candidates) | parity |
| core-services | Opening items in an app | springboard/src/shell/workspace.rs (route_url, workspace_open_url); springboard/src/shell/defaultapps.rs (desktop_open_link) | parity |
| bundle-resources | URL schemes and document types declared by the app | springboard/src/shell/defaultapps.rs (manifest_handler, Email category); springboard/ui/src/launch.rs (Handler::scheme) | partial (no Open With in Files yet, see gaps) |
| uniform-type-identifiers | Open and share targets by type | springboard/src/shell/defaultapps.rs (CATEGORIES); springboard/src/shell/sharing.rs (open_types) | partial (receiving works from the manifest list; no per-app rank API) |

## Remaining gaps

- **Open With in Files is not wired.** The candidate list is now available through the service, but Files' context
  menu (`springboard/apps/files/src/menus.rs`) takes flat items with no submenu, and the reply arrives after the menu
  opens. The next step is a Files submenu built from the reply, or a sheet, which is an app change outside this round.
- **Get Info > Open with in Files is not wired** for the same reason (the candidate list is asynchronous there too).
- **Files still opens its own previews** for the files it shows. `Shell::default_app_for_file` decides for Files'
  workspace requests (`OpenFile`) and for the shell's own dispatch; nothing else in the shell asks it yet.
- **No Email app declares `mailto`**, so the Email group is empty and `mailto:` links have no handler.
- **App-declared exported and imported types** (`springboard/ui/src/utdecl.rs`, Wave 4 model) are not read from manifests.
- **Maps URLs** (`springboard/ui/src/maps_link.rs`) are not routed: no maps scheme is declared by any app.
- **Universal links** have no domain association and no https routing to a non-browser app.
- **No acceptance check by bundle id** (`LSCanURLAcceptURL`): `apps_for_type` answers by declaration only.
- **Launch configuration** (activation, arguments) for `open_with` is not in the API.
- Not verified: no build or test run (the lead builds once). The Default Apps Settings page test in
  `defaultapps.rs` (`the_settings_page_lists_every_category_with_the_default_checked`) still indexes groups 0 and 1 and
  still holds with the appended Email group.
- `docs/default-apps.md` (a shared doc) still lists the service and link routing under "Not done yet"; the lead should
  update it with this round.
