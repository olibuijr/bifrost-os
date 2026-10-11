# Vibraphone

Vibraphone (motor off), 4 velocity layers: soft yarn mallets (pp/mp) and hard mallets (mf/f), long natural ring. Range F3-F6 (MIDI 53-89, playable C3-G6). Jazz, lounge, lo-fi hip-hop hooks, bossa, chillhop, film; comping chords and melodies.

- Source: https://github.com/sgossner/VCSL/tree/master/Idiophones/Struck%20Idiophones/Vibraphone
- License: CC0-1.0 (see LICENSE.txt)
- Attribution: Versilian Community Sample Library (VCSL) by Versilian Studios / Sam Gossner, CC0 1.0
- Zones: 44 (4 velocity layer(s)), range MIDI 48-91

Built by `tools/build-instruments-keys2.py vibes`: onset trimmed (zero-crossing, 1 ms fade-in), partial per-note level balance, one global normalization (layer dynamics kept; `gain` per velocity layer sets the level curve), natural decay without loops, cosine fade-out, Ogg Vorbis q4 44.1 kHz stereo. `tune` (cents) where the source deviates > 12 cents.
