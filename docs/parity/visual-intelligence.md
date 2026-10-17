# Visual search (Visual Intelligence framework)

Status: partial. The model and the app-facing contract exist; the shell does not yet capture a camera frame or a
screenshot selection and pass it to apps, so nothing reaches an app from the desktop.

## What was built

- `springboard/ui/src/visual_search.rs` (`bifrost_ui::visual_search`), pure model:
  - `Scene`: a captured scene with kind (`Screenshot`, `Photo`, `Stream`), RGBA8 pixels, and scene labels.
    `Scene::new` refuses a zero or oversized side (`MAX_SIDE` 8192) and a pixel buffer of the wrong length.
  - `Scene::with_labels` normalises labels (trimmed, lowercase, no duplicates).
  - `Scene::crop(PixelRect)`: the object selection on a screenshot. Clamps to the scene and refuses an empty result.
  - `label_score`: 0 to 100, the share of an app's keywords that equal a scene label.
  - `merge`: drops untitled matches, keeps the best copy of a repeated (app, intent, title), sorts by score, limits to
    `MAX_MATCHES` (20).
- `springboard/sdk/src/visual_search.rs` (`bifrost_app::visual_search`), app-facing:
  - `VisualSearchProvider` trait: `app_id()` and `search(&Scene) -> Vec<Match>`.
  - `search(providers, scene, limit)`: asks every provider and stamps each match with the provider's own app id, so an
    app cannot answer for another app.

- The intent path (2026-10-17): a provider app declares `visual_search::intent()` (`visual-search`, parameters `kind`
  and `labels`) and answers it with `visual_search::perform(provider, params)`; the reply value is the match list
  (`matches_to_json`), which a caller reads with `matches_from_json` and merges across providers with `collect`. Pixels
  do not cross the intent socket. Tests: `visual_search::tests` in the SDK.

## Official API names mirrored

- Visual Intelligence framework: https://developer.apple.com/documentation/visualintelligence
  (search integration: scene captured by the camera or a screenshot, matches returned by the app).
- `SemanticContentDescriptor` (struct), https://developer.apple.com/documentation/visualintelligence/semanticcontentdescriptor:
  `labels: [String]` maps to `Scene::labels`; `pixelBuffer` maps to `Scene::pixels` (RGBA8 instead of a CVPixelBuffer).
- Object selection in a screenshot: the framework's screenshot selection maps to `Scene::crop`.

## REA evidence

- `ev_f8f9cb13fa5f3b9ff2cf18b0c591b5db5c5634d7a4e2af2e41811a37ccfffde4`: Visual Intelligence landing page.
- `ev_904fa3820ed30981688fc4476d45fd97f44bde474762f7383949023f52f63d8c`: SemanticContentDescriptor page.

## Matrix rows closed

| Framework | Feature (as in data/visual-intelligence.json) | New status | BifrOSt path |
|---|---|---|---|
| visual-intelligence | Visual search integration with the system camera and screenshots | partial | springboard/sdk/src/visual_search.rs; springboard/ui/src/visual_search.rs |
| visual-intelligence | SemanticContentDescriptor (scene labels, pixel buffer for apps) | partial | springboard/ui/src/visual_search.rs (`Scene`) |
| visual-intelligence | Camera or screenshot object selection feeding a search | partial | springboard/ui/src/visual_search.rs (`Scene::crop`, `PixelRect`) |

## Still missing

- The shell does not capture camera frames or screenshot selections and does not call apps (no control-socket method,
  no camera or screenshot UI). The camera has QR decoding only (`springboard/src/shell/camera/qr.rs`).
- Labels come from no classifier. A shell classifier would fill `Scene::labels`.
- No Shortcuts or assistant integration; the Apple Intelligence schema side stays n/a.
