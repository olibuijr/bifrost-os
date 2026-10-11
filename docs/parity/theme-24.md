# Theme 24: extension-based UI in a host app (remote views)

## What was built

One pure module, `springboard/ui/src/linkmeta.rs` (registered in `springboard/ui/src/lib.rs`), for rich links:

- `validate_url` and `resolve`: an absolute http or https URL check, and resolution of page references (`//host`,
  `/path`, relative paths) against the final URL after redirects. `data:` and `javascript:` references are refused.
- `LinkMetadata::from_html`: reads the head of a page. Title from `og:title`, then `twitter:title`, then `<title>`,
  falling back to the host. Description, site name, image (`og:image` and variants), video (`og:video` variants)
  and icon (`<link rel>` naming an icon, non-touch icons first). Fails with `NoMetadata` when there is nothing to show.
- `LinkMetadata::subresources(policy)`: the icon, image and video addresses the caller should fetch, empty when the
  policy turns subresources off.
- `FetchPolicy` (timeout, `fetch_subresources`) with validation against `MAX_TIMEOUT_SECS`.
- `MetadataProvider`: a state machine (`Idle`, `Fetching`, `Done`, `Failed`) with `start`, `cancel`, `fail`, `tick`
  and `complete`. Time is passed in as seconds. A late answer after a cancel or a timeout is ignored.
- `LinkView` (placeholder or loaded) with `card()` returning a `Card` (title, subtitle, icon, image, has-video,
  placeholder flag) for the shared kit to draw.
- `LinkError`: a typed error (`InvalidUrl`, `UnsupportedScheme`, `BadPolicy`, `Timeout`, `Cancelled`, `NoMetadata`).

The module makes no network request and draws nothing. The caller fetches the bytes; the model reads them.
Tests: 9 behaviour tests in `linkmeta::tests` (head metadata and resolution, title fallbacks, no-metadata case, URL
validation, reference resolution, provider success path, cancel and timeout, bad input, subresources and card).

Not built in this round: the SDK wrapper in `springboard/sdk/src/` (no app-facing call yet), a fetch that uses the
network service, rendering of the card, the share-sheet rich preview, and the News app's own og:image scrape, which
still lives in `springboard/apps/news-desktop/src/pictures.rs` and was not switched over.

## Official API names mirrored

Official pages (DocC JSON via `tools/parity/docc.py`, rendered via REA):

- `LPMetadataProvider` (`startFetchingMetadata(for:completionHandler:)`, `cancel()`, `timeout`,
  `shouldFetchSubresources`): https://developer.apple.com/documentation/linkpresentation/lpmetadataprovider
- `LPLinkMetadata` (`url`, `originalURL`, `title`, `iconProvider`, `imageProvider`, `remoteVideoURL`):
  https://developer.apple.com/documentation/linkpresentation/lplinkmetadata
- `LPLinkView` (`init(metadata:)`, `init(url:)`, `metadata`): https://developer.apple.com/documentation/linkpresentation/lplinkview
- `LinkMetadata` (structure): https://developer.apple.com/documentation/linkpresentation (Structures section)
- `LinkPresentation Errors` and `LinkPresentation Macros`: https://developer.apple.com/documentation/linkpresentation

Mapping: `MetadataProvider` mirrors the provider, `FetchPolicy` the timeout and subresource flags, `LinkMetadata`
the metadata record (its `url` and `original_url` as `url` and `original_url`, and the image, icon and video as
addresses rather than lazy providers), `LinkView` the view with its placeholder and metadata initialisers, and
`LinkError` the errors. Icon, image and video are addresses here, so the caller does the download that a provider
would do.

## REA evidence

- `ev_4ecb82343635797988fd1e84b1d52dc3e1a752a163553ef25c33ca94cc11a967`: the rendered Link Presentation landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/linkpresentation`), headings for link
  metadata, rich links and the reference section, platforms macOS 10.15+.

## Matrix rows closed

Framework slug `link-presentation`. Status is `partial` for every row, because the model exists but no SDK call,
network fetch or view draws it yet.

| Framework slug | Feature (as in data/link-presentation.json) | New status | BifrOSt path |
|---|---|---|---|
| link-presentation | Fetching link metadata (LPMetadataProvider, LinkMetadata.init(fetching:)) | partial | springboard/ui/src/linkmeta.rs (MetadataProvider, FetchPolicy) |
| link-presentation | Rich link view (LPLinkView) | partial | springboard/ui/src/linkmeta.rs (LinkView, Card) |
| link-presentation | Representative title, icon and image of a URL | partial | springboard/ui/src/linkmeta.rs (LinkMetadata::from_html) |
| link-presentation | Video of a link | partial | springboard/ui/src/linkmeta.rs (LinkMetadata.video) |
| link-presentation | Errors and macros (LPError, LPErrorDomain, LPError.Code) | partial | springboard/ui/src/linkmeta.rs (LinkError) |

Rows not closed in this theme: the ExtensionKit, QuickLook UI, TipKit, WidgetKit, WebKit, Security Interface,
PaperKit, VisionKit and Local Authentication Embedded UI rows. The remote-view hosting that ExtensionKit needs
(host view controller, scene handles) is not a pure model that fits this crate, so it was left for a later round.

## Verification

- `python3 tools/check-names.py`: exit 0.
- No cargo or test runs were made in this worktree, as the task required. The module has not been compiled.
