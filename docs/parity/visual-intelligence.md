# Visual search (Visual Intelligence framework)

Status: partial. The model, the app-facing contract and the shell's delivery exist, and the person can start a search:
Look Up in the Markup editor (the screenshot thumbnail's result, and Preview's Markup) and "Look Up in Visual Search" in the
Photos viewer's Info sheet send the picture to the apps that answer visual searches and show what they found in a panel.

## Wired (wave 14)

- `springboard/src/shell/visual_search_panel.rs`: the panel and its glue. `Shell::visual_search_start` (a picture already
  in memory: Markup passes the picture as marked) and `Shell::visual_search_photo` (a file: read and decoded on a worker)
  build a `Scene` (`scene_from_pixels`), and a worker thread delivers it through `intents::visual_search` to every
  installed app that declares `[[intent]] id = "visual-search"` and is already running. The merged matches (best first,
  at most 20) are listed with the app's name; a tap on a match opens that app and runs the intent the match names, as a
  shortcut does. Escape or a tap outside closes the panel, which takes every touch while it is up.
- Markup (`shell/markup.rs`): a Look Up button in the bar, left of Done. The editor stays open under the panel.
- Photos (`shell/photos/mod.rs`): the Info sheet has a "Look Up in Visual Search" row above Revert to Original;
  `Photos::take_visual_search` hands the open photo's path to the shell once.
- Privacy: nothing is sent until the person taps Look Up. The picture goes only to apps that declare the intent and are
  already open (an app is never started for a search), shrunk to 192 px on its longest side, over the app's own control
  socket; the panel says so.

Tests: `shell::visual_search_panel::tests`, `shell::markup::tests`, `shell::photos::tests::the_info_sheet_look_up_row_hands_the_open_photo_to_the_shell_once`.

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

- Camera frames are not searched: the camera has QR decoding only (`springboard/src/shell/camera/qr.rs`), and the camera
  view has no Look Up button. `SceneKind::Stream` has no producer.
- Object selection: the editor sends the whole picture as marked. `Scene::crop` has no selection gesture that feeds it;
  circling an object with the pen is the only way to point at it, and apps see the marks as pixels.
- Labels come from no classifier, so `Scene::labels` is empty and `label_score` is not used by the shell. An app matches
  on the pixels.
- No Shortcuts or assistant integration; the Apple Intelligence schema side stays n/a.
