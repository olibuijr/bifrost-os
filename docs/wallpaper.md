# Wallpaper: dynamic schedule and typed wallpapers

Code: `springboard/src/shell/wallpaper_types.rs` (kinds and app wallpapers), `wallpaper_schedule.rs` (the day schedule),
`wallpaper_page.rs` (Settings > Wallpaper).

## Kinds

- A *still* is one picture.
- A *day* is a set of pictures that changes through the day.
- An app declares its wallpapers in `manifest.toml`: `wallpaper_kind = "still"` or `"day"`, and `wallpapers = [...]` with
  package-relative paths. The Settings page lists installed app wallpapers.

## Day schedule (`wallpaper_schedule.rs`)

- The day (1440 minutes) is cut into equal slices, one per picture, in order. The first picture shows from midnight.
- The model is pure. The shell asks which picture is current and how many minutes until the next change, and sets a
  timer for it. A change lands on the first whole minute at or after the boundary.
- Settings > Wallpaper has the Dynamic Wallpaper switch, which drives the wallpaper from this schedule.

## Settings page

- A grid of the built-in stills and the photos. Tapping one opens a preview with Set Lock Screen and Set Wallpaper actions.
- Search lists a Set Wallpaper row for each picture in the catalogue.
