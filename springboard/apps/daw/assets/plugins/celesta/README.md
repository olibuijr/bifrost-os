# Celesta

Celesta (keyboard glockenspiel, hammered steel plates): soft, sweet music-box-like bell tone, 2 velocity layers. Range F#3-C8 (MIDI 54-108, best C4-C7). Lo-fi/hip-hop hooks, lullabies, fairy-tale and film cues, Christmas, doubling piano melodies.

- Source: FluidR3_GM.sf2 "Celesta" (bank 0, program 8), Debian package fluid-soundfont-gm 3.1-6 (http://deb.debian.org/debian/pool/main/f/fluid-soundfont/)
- License: MIT (see LICENSE.txt)
- Attribution: Celesta samples from the Fluid R3 GM SoundFont by Frank Wen (2000-2002, 2008) and Toby Smithe (2008), MIT license.
- Zones: 16 (2 velocity layer(s)), range MIDI 48-108

Built by `tools/build-instruments-keys2.py celesta`: onset trimmed (zero-crossing, 1 ms fade-in), partial per-note level balance, one global normalization (layer dynamics kept; `gain` per velocity layer sets the level curve), natural decay without loops, cosine fade-out, Ogg Vorbis q4 44.1 kHz stereo. `tune` (cents) where the source deviates > 12 cents.
