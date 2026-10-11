# Glockenspiel

Orchestral glockenspiel (metal bars), 3 velocity layers soft/medium/loud, sparkling bell tone with natural ring. Sounds G5-C8 (MIDI 79-108, playable C5-C8). Doubling melodies an octave up, bell hooks, lo-fi/hip-hop and pop sparkle, Christmas/film/cinematic.

- Source: https://github.com/sgossner/VCSL/tree/master/Idiophones/Struck%20Idiophones/Glockenspiel
- License: CC0-1.0 (see LICENSE.txt)
- Attribution: Versilian Community Sample Library (VCSL) by Versilian Studios / Sam Gossner, CC0 1.0
- Zones: 18 (3 velocity layer(s)), range MIDI 72-108

Built by `tools/build-instruments-keys2.py glock`: onset trimmed (zero-crossing, 1 ms fade-in), partial per-note level balance, one global normalization (layer dynamics kept; `gain` per velocity layer sets the level curve), natural decay without loops, cosine fade-out, Ogg Vorbis q4 44.1 kHz stereo. `tune` (cents) where the source deviates > 8 cents.
