# Theme 35: custom file type thumbnails (QLThumbnailProvider)

## What was built

A pure model in `springboard/ui/src/thumbnail.rs` (`bifrost_ui::thumbnail`), re-exported to apps from
`springboard/sdk/src/peek.rs`. It has four parts:

- `FileThumbnailRequest`: the file, the minimum and maximum accepted size in points, and the scale. `fit(aspect)`
  picks the largest drawing size inside the maximum that keeps the aspect, or `None` when the minimum cannot be met.
- `ThumbnailReply`: a drawing of a given size or an image file, with an optional short extension badge.
  `accepted_by(request)` checks a drawing against the size range the request allowed.
- `ThumbnailError`: the six generator codes, each with a short English message and `is_cancellation()`.
- `Progressive`: collects the representations of one request as they are produced. `offer` returns an update only
  when the new representation is better than the best so far. `cancel` and `finish` report the final outcome.

The representation types (`Representation`: thumbnail, low quality, icon) and the request type
(`ThumbnailRequest`) were already in `springboard/ui/src/preview.rs` and are reused unchanged.

## Official API names mirrored

- `QLThumbnailProvider`, `QLFileThumbnailRequest` (`maximumSize`, `minimumSize`, `scale`, `fileURL`)
  https://developer.apple.com/documentation/quicklookthumbnailing/qlfilethumbnailrequest
- `QLThumbnailReply` (`init(contextSize:drawing:)`, `init(imageFileURL:)`, `extensionBadge`)
  https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailreply
- `QLThumbnailError.Code`: `generationFailed`, `noCachedThumbnail`, `noCloudThumbnail`, `requestCancelled`,
  `requestInvalid`, `savingToURLFailed` (read from the rendered page; DocC returned 404 for this path)
  https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailerror-swift.struct/code
- `QLThumbnailRepresentation.RepresentationType` (`icon`, `lowQualityThumbnail`, `thumbnail`)
  https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailrepresentation/representationtype
- `QLThumbnailGenerator.generateRepresentations(for:update:)`
  https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator/generaterepresentations(for:update:)

## REA evidence

- Framework landing page: `ev_b8d41abf24da3dcdc11073bf1944b6671a345423254bb63f7a2e4e8539087ab9`
- `QLThumbnailError.Code` page: `ev_0ae5e0e8cc1d328b994fb22e12f07a6b1820b23f5f6dfbf8cec399f60ed62e74`

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| quick-look-thumbnailing | Custom file type thumbnail extension (QLThumbnailProvider) | partial | springboard/ui/src/thumbnail.rs (FileThumbnailRequest, ThumbnailReply); springboard/sdk/src/peek.rs (re-export) |
| quick-look-thumbnailing | Error reporting for thumbnail generation | partial | springboard/ui/src/thumbnail.rs (ThumbnailError) |
| quick-look-thumbnailing | Thumbnail representation types (icon, low quality, thumbnail) | partial | springboard/ui/src/preview.rs (Representation), springboard/ui/src/thumbnail.rs (Progressive selects by it) |
| quick-look-thumbnailing | Progressive or updated thumbnails | partial | springboard/ui/src/thumbnail.rs (Progressive) |

## Not closed (still gaps)

- No extension host: the system does not call an app's thumbnail provider for files of other apps, so the
  "system-wide thumbnails" row stays a gap.
- No asynchronous generator and no update handler. `Progressive` is a state machine that the caller drives. Nothing
  produces thumbnails for PDF, text, audio, or video files.
- No disk writes of thumbnails and no drawing. The models describe the reply; the renderer does not execute it.
- The `QLThumbnailError.Code` raw integer values are not recorded, so the codes are not decodable from an integer.
- Quartz Composer rows and the `QLPreviewView` embedded preview are outside this theme.
