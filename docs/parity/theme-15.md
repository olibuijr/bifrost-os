# Theme 15: caption language selection and profiles

## What was built

`springboard/ui/src/captions.rs` (bifrost-ui, pure model, no I/O) holds the caption preferences the system keeps for a
person:

- **Language stack**: ordered caption languages, normalised tags (`en_us` becomes `en-US`), newest first, duplicates move
  to the top, invalid tags refused.
- **Display type**: `ForcedOnly`, `Automatic`, `AlwaysOn`.
- **Appearance**: relative text size, text colour, opacity and edge style, highlight colour and opacity, window colour,
  opacity and corner radius. Opacities are clamped to 0 to 1; non-finite or non-positive sizes are refused.
- **Profiles**: named copies of a full settings set. The active profile is what the `User` domain reads and writes;
  removing the active profile falls back to the person's own settings.
- **Domains**: `Default` is read-only, `User` is the person's choice.
- **Change notice**: `take_changes()` returns each kind of change once; `report_displayed()` records the tracks shown and
  reports only a real difference.
- **Track choice**: `choose_track()` picks a caption track from the audio language, the system language and the media's
  track tags, following the display type and the language stack (exact tag, then same primary language).

The module is wired with one `pub mod captions;` line in `springboard/ui/src/lib.rs`. Its
`#[cfg(test)] mod tests` holds 7 behaviour tests.

## Official API mirrored

Media Accessibility, Captions collection (https://developer.apple.com/documentation/mediaaccessibility/captions):

- Language settings: `MACaptionAppearanceAddSelectedLanguage`, `MACaptionAppearanceCopySelectedLanguages`
- Profile settings: `MACaptionAppearanceCopyProfileIDs`, `MACaptionAppearanceSetActiveProfileID`,
  `MACaptionAppearanceCopyActiveProfileID`, `MACaptionAppearanceCopyProfileName`,
  `MACaptionAppearanceExecuteBlockForProfileID`
- General settings: `MACaptionAppearanceSetDisplayType`, `MACaptionAppearanceGetDisplayType`,
  `kMACaptionAppearanceSettingsChangedNotification`, `MACaptionAppearanceDidDisplayCaptions`
- Text, highlight and window settings: the `CopyForegroundColor`, `GetForegroundOpacity`, `GetRelativeCharacterSize`,
  `GetTextEdgeStyle`, `CopyBackgroundColor`, `GetBackgroundOpacity`, `CopyWindowColor`, `GetWindowOpacity` and
  `GetWindowRoundedCornerRadius` getters
- Enumerations: `MACaptionAppearanceDisplayType` (`forcedOnly`, `automatic`, `alwaysOn`),
  `MACaptionAppearanceDomain` (`default`, `user`), `MACaptionAppearanceTextEdgeStyle` (`undefined`, `none`, `raised`,
  `depressed`, `uniform`, `dropShadow`)

The Rust names are neutral (`CaptionAppearance`, `Domain`, `DisplayType`, `EdgeStyle`, `Settings`, `Change`); the
official names appear only in this document.

## REA evidence

- `ev_25b8a0ae7efdd9b954b0abc8803b1bc82d6b7ef2f07ecd215e6ac5717851cd22`: rendered Captions page (language, display,
  text, highlight and window groups).
- `ev_43d4941553afe442f3531e82d037c6e9c932d937d252f93a6d06bd706534e009`: rendered `MACaptionAppearanceDisplayType` page
  (three cases and their meaning).

The profile functions have no summary text on the official pages, so their semantics here come from the names and the
collection listing only. The `MACaptionAppearanceBehavior` distinction (use value, or use content if available) is not
modelled.

## Matrix rows closed

| Framework slug | Feature (as in data file) | Status | BifrOSt path |
|---|---|---|---|
| media-accessibility | Caption language selection and profiles | partial | springboard/ui/src/captions.rs |
| media-accessibility | Caption appearance settings (font, colours, opacity, edge style, window, display type) | partial | springboard/ui/src/captions.rs |
| media-accessibility | Caption change notification and display reporting | partial | springboard/ui/src/captions.rs |

Status is partial because the model has no system preference store, no font descriptor (font choice stays in
`render/src/fontprefs.rs`), no Settings page wiring and no player integration yet.

Not closed: flashing lights, image caption metadata, localized media names, and the other theme rows.
