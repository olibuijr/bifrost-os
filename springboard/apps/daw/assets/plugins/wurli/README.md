# Wurlitzer

Wurlitzer EP200 reed electric piano, 4 velocity layers pp/mp/f/ff (soft = sweet and hollow, hard = gritty/honky bark). Range A1-C7 (MIDI 33-96), best C2-C6. Soul, 60s-70s pop/rock, indie, lo-fi, Motown and gospel comping; short natural decay.

- Source: https://github.com/sfzinstruments/GregSullivan.E-Pianos (Wurlitzer EP200 v1.1, http://www.sullivang.net/)
- License: CC-BY-3.0 (see LICENSE.txt)
- Attribution: Wurlitzer EP200 samples by Greg Sullivan (http://www.sullivang.net/), SFZ mapping by kinwie, licensed CC BY 3.0 (https://creativecommons.org/licenses/by/3.0/); modified.
- Zones: 47 (4 velocity layer(s)), range MIDI 33-96

Built by `tools/build-instruments-keys2.py wurli`: onset trimmed (zero-crossing, 1 ms fade-in), partial per-note level balance, one global normalization (layer dynamics kept; `gain` per velocity layer sets the level curve), natural decay without loops, cosine fade-out, Ogg Vorbis q4 44.1 kHz mono. `tune` (cents) where the source deviates > 12 cents.
