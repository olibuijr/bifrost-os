# Changelog

## 0.2.13

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 0.2.12

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.2.11

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.2.10

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.2.9

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.2.8 — 2026-10-23

- Declares the movie types so Files opens movies in it.

## 0.2.7 — 2026-10-20

- Sound plays on the default output, and the app waits before reconnecting to the media service instead of retrying in a tight loop.

## 0.2.6 — 2026-10-20

- Playback reaches the system's media service again (the app's audio connection was never opened, so nothing played).

## 0.2.5

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## Unreleased

- Playback menu: a speed picker (0.5x to 3x, the sound time-stretched so its pitch holds) and a subtitle picker.
- Subtitle files per language (`name.en.srt`) are all loaded and offered in the picker, not only the first sidecar.

## 0.2.4 — 2026-10-14

- Size notice and subtitle text margins from shared spacing tokens so captions remain readable across window scales.
- Empty-state and caption HUD surfaces use shared appearance-aware material and background colours.

## 0.2.3 — 2026-10-14

- Size notices to the available window width, use shared corner-radius tokens, and position captions relative to the viewport.

## 0.2.2 — 2026-10-14

- Add the generated app compliance harness.

## 0.2.1 — 2026-10-14

- Add the Video Player store icon.
## 0.2.0 — 2026-10-13

- Open Movie intent (`open-movie`, parameter `path`): the shell hands the Video Player a movie file by path. The shell uses it to open a finished screen recording, which the player plays like a movie from the open panel.

## 0.1.0 — 2026-10-13

- New app: Video Player plays a movie file in the shell's fullscreen player view over the shared media stack. Movies open from Movie > Open (the document panel), from a file handed to the app, or from the command line.
- Captions: a sidecar `.srt` or `.vtt` next to the movie shows as subtitles (Movie > Subtitles), parsed by `springboard::media::subtitles`.
- Trim: Set Trim Start and Set Trim End at the playhead (shared `Trim` model); playback loops inside the trim.
- Recording: New Audio Recording writes a WAV take through the audio service into the app's data folder (shared `Recorder` model). New Screen Recording is listed and stays off until the shell has a screen capture service.
