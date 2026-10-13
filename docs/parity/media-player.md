# Media Player: parity pass

Official documentation: https://developer.apple.com/documentation/mediaplayer

## What was built

`springboard/ui/src/media_picker.rs` is a pure model of a library picker, in the style of `contact_picker.rs`. The
app asks for the item kinds it accepts (songs, albums, playlists; an empty list accepts all) and a selection rule:
one item, or many with an optional limit. The picker lists items sorted by title, filters them with a fuzzy
query over title, artist and album, and refuses items that are not playable or not of an accepted kind.
`toggle` chooses or drops an item; `confirm` returns the chosen ids, and fails when nothing is chosen. The app
receives only the confirmed ids and never reads the library.

The model draws nothing. A sheet or SDK widget that draws the list still has to be built.

## Official API names mirrored

- `MPMediaPickerController`: the picker's kinds, selection rule and confirmed result.

Not covered: the picker view, external player items (`MPContentItem`, `MPPlayableContentManager`), and an app-facing
library query API. Built on parity/team-a-10; not yet verified on Midget.
