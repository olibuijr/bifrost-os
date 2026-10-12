# Wire 3: content-area fill in Dark (CONTENT_BG)

## What is wired

Dark appearance consistency. Several apps filled their main content area with `DETAIL_BG`, which is pure black in
Dark. Stock dark content views use the dark content grey, which is `CONTENT_BG` in `springboard/render/src/ui.rs`
(`pack(30, 30, 30)` in Dark, white in Light). The main content and detail fills in these apps now use `CONTENT_BG`:

| App | Patch bump | Fills changed |
|---|---|---|
| assistant-desktop | 1.2.0 to 1.2.1 | window fill and the composer strip under the transcript (`src/view.rs`) |
| console-desktop | 1.2.0 to 1.2.1 | window fill, table header band, table footer band (`src/draw.rs`) |
| daw | 0.3.0 to 0.3.1 | window fill (`src/main.rs`) |
| games-desktop | 1.4.0 to 1.4.1 | window fill (`src/games.rs`); open-cell tiles keep `DETAIL_BG` |
| music-desktop | 1.4.0 to 1.4.1 | window fill and the top and bottom bands (`src/music.rs`) |
| podcasts-desktop | 1.5.0 to 1.5.1 | window fill and the detail column band (`src/podcasts.rs`) |
| shortcuts-desktop | 1.4.0 to 1.4.1 | window fill (`src/widget.rs`) |
| voicememos-desktop | 1.4.0 to 1.4.1 | window fill (`src/voicememos.rs`); the record button keeps `DETAIL_BG` |
| weather-desktop | 1.3.0 to 1.3.1 | window fill and the header bands (`src/weather.rs`) |

`freeform-desktop` has no `DETAIL_BG` fill (its canvas uses other colours), so it is unchanged.
Sidebars, grouped cards and cells keep `SIDEBAR_BG`, `CARD_BG` and their own colours. Apps owned by other agents
(news, calendar, reminders, clock, books, notes, files, browser, calculator, contacts) are not touched.

## How a user reaches it

Switch the PC to Dark (or Auto, which is Dark after 19:00), then open any app listed above. The content area is
dark grey, not black, and it matches the Files app.

## How to verify on Midget

1. `bifrost pc midget appearance dark`
2. `bifrost pc midget open-app <id>` for each app (for example `daw`, `weather-desktop`, `podcasts-desktop`).
3. `bifrost pc midget screen <scratch>/<app>.png` and check that the main area is dark grey (about RGB 30,30,30)
   and that the sidebar and cards keep their own tones.
4. Repeat with `bifrost pc midget appearance light`: the content area must be white.

Host preview: `bifrost pc smoke APP` and `bifrost pc smoke APP --light`.

## REA and official evidence

- `python3 -I tools/parity/docc.py appkit/nscolor/controlbackgroundcolor` returned the official description: "The color
  to use for the background of large controls, such as scroll views or table views."
- Existing matrix evidence for appkit-user-interface: `ev_f82ebd8719732e253617f0fa4a235954eb9b61fb56c2c20aeaeb04b009140b94`.
- No new REA page capture was made in this round (the change is a token swap, not a new reference behaviour).

## Remaining gaps

- The window-content fill in the other Dark apps (news, calendar, reminders, clock, books, notes, files, browser,
  calculator, contacts) is owned by other agents and is not changed here.
- Cargo.lock is not edited by hand. The version bumps in `Cargo.toml` change the lock on the next build.
- No new models were wired in this round. The `wire3` scope is the colour fix only.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path |
|---|---|---|
| appkit-user-interface | Appearance Customization | springboard/render/src/ui.rs (`CONTENT_BG`), applied in springboard/apps/{assistant-desktop,console-desktop,daw,games-desktop,music-desktop,podcasts-desktop,shortcuts-desktop,voicememos-desktop,weather-desktop}/src |
