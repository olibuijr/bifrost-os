# Keyboard system package (`is.olibuijr.keyboard`)

`kind = "system"`, `exec = "bifrost-keyboard"` (crate `apps/keyboard`): the on-screen keyboard process. The shell spawns it on demand
and composites its frame; no home-screen icon. It shows in the
App Store (Apps > System, Updates) and is preinstalled and kept current by the shell's background
check. After an install or update the shell reloads what it `provides = ["keyboard"]`:

- `assets/NotoColorEmoji.ttf`: the CBDT color emoji font (tried before the `/system` paths; the
  tablet's own `/system/fonts/NotoColorEmoji.ttf` is COLRv1 and is ignored).
- `assets/words/<lang>.txt`: word lists overriding the compiled-in ones (`en`, `is`).
  `tools/aap.py pack` copies them from `apps/keyboard/src/words/` automatically.
- `assets/OFL.txt`: license notice of the font (SIL OFL 1.1).

The font (10.7 MB) is not committed. Before packing:

    cp /usr/share/fonts/noto/NotoColorEmoji.ttf system/keyboard/assets/   # must be the CBDT build
    tools/pc-app.py publish keyboard     # builds apps/keyboard, packs system/keyboard + the binary

`icon.png` comes from `cargo run --example keyboard_icon -- system/keyboard/icon.png`.
Bump `version` in `manifest.toml` for every release; the store's apps.json entry needs
`"kind":"system"` for the shell to treat it as a system component.
