# Changelog

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
