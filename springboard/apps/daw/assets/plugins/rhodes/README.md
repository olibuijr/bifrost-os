# Rhodes

Fender Rhodes Mark II stage electric piano (tine EP), 3 velocity layers: soft = round and mellow, medium = classic bell-like tine, hard = barky/overdriven. Range E1-E7 (MIDI 28-100), best C2-C6. Neo-soul, R&B, jazz, lo-fi hip-hop, gospel and funk chords (7ths/9ths), soft ballads; natural decay.

- Source: https://freesound.org/people/tim.kahn/packs/3957/ (C_S Fender Rhodes Mark II); MIDI 65-84 from FluidR3_GM.sf2 "Rhodes EP" (http://deb.debian.org/debian/pool/main/f/fluid-soundfont/)
- License: CC-BY-4.0 (+ MIT for 3 gap-fill notes) (see LICENSE.txt)
- Attribution: "C_S Fender Rhodes Mark II" by tim.kahn (https://freesound.org/people/tim.kahn/packs/3957/), licensed CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/); modified. Notes G4-G6 (MIDI 67/72/79 roots) from Fluid R3 GM SoundFont by Frank Wen, MIT license.
- Zones: 63 (3 velocity layer(s)), range MIDI 28-100

Built by `tools/build-instruments-keys2.py rhodes`: onset trimmed (zero-crossing, 1 ms fade-in), partial per-note level balance, one global normalization (layer dynamics kept; `gain` per velocity layer sets the level curve), natural decay without loops, cosine fade-out, Ogg Vorbis q4 44.1 kHz mono. `tune` (cents) where the source deviates > 12 cents.
