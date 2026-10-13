# Default apps (Launch Services)

## How macOS does it

Sources: Apple Support, "Choose an app to open a file on Mac"; AppKit `NSWorkspace` (skill `apple-appkit`,
`documents-files-writing-tools.md`, section 2).

- **Declaration.** An app lists the document types it opens in its Info.plist (`CFBundleDocumentTypes`, each with
  `LSItemContentTypes` as uniform type identifiers and an `LSHandlerRank`: Owner, Default, Alternate, None) and the URL
  schemes it opens (`CFBundleURLTypes`). Launch Services collects them into one database when an app is installed.
- **Resolution.** The default for a type is the user's explicit choice, else the best declaration: the most specific type
  (PNG before "image"), then the rank. `None` means it can open the type but is never chosen by itself.
- **API.** `NSWorkspace.urlForApplication(toOpen:)` (a file or a content type), `urlsForApplications(toOpen:)` (every
  candidate, for an Open With menu), `setDefaultApplication(at:toOpenContentType:)` and `...toOpenURLsWithScheme:`.
- **UI.** There is no single page. Per type: Finder > Get Info > "Open with" > pick an app > "Change All" (changing only
  that type, not its siblings: JPEG and PNG are separate). Per file once: File > Open With, or the context menu, with the
  default marked. Web browser: System Settings > Desktop & Dock; email: Mail > Settings. Third-party panes (RCDefaultApps,
  SwiftDefaultApps) exist because the system has no overview.

## How BifrOSt does it

- Model: `bifrost_ui::launch` (`Registry`, `Handler`, `Rank`), the same shape and names as above, pure and cached
  (`ui_launch_default_for_file_10k`: 0.39 ms for 10k lookups with 30 apps). Types come from `uttype`.
- Declarations: installed manifests (`handles = ["public.png:owner", "public.text"]`, `schemes = ["https"]`; rank is
  owner, default, alternate or none, default is `default`), built-in apps in `shell/defaultapps.rs`, and a table for the
  first-party desktop apps until their manifests carry the keys.
- Settings: **General > Default Apps** (desktop). One group per family (Web Browser, Images, Audio, Video, Text, PDF,
  Archives), the candidates as checked rows, the default first. Choosing an app is "Change All" for the family: every
  type of it the app can open, and the scheme. This is one overview page where macOS has none, which is a deliberate
  difference; the per-type model underneath is the same, so Get Info > Open with can use it unchanged.
- Storage: one preference, `default.apps` (`type public.png <app>;scheme https <app>`); choices for an app that is not
  installed are kept and apply again if it returns.
- Shell service: `launch` (`shell/services/launch_services.rs`, SDK `bifrost_app::launch_services`) answers
  `default_for_file`, the Open With candidates (default first, `Shell::open_with_choices`) and the URL-scheme lookups
  (`default_app_for_scheme`, `apps_for_scheme`). Apps reach it without touching the registry.
- Links: `open-url` on the control port (and `bifrost-open-url`) goes through `Shell::desktop_open_link`: web links to the
  browser, any other declared scheme (`mailto:`, `bifrost-app:`) to the app the registry names; local and script schemes
  are refused.
- Files: the Open With panel (`springboard/apps/files/src/openwith.rs`) lists the candidates with the default marked.
  It is read-only: no shell call opens a file in a chosen app yet, so a row cannot open the file.
- Not done yet: opening a file in a chosen app (the missing shell call), Get Info > Open with in Files, and Open With in the
  context menu.
