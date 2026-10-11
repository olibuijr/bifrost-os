# Choir Aahs

Mixed choir singing "aah", sustained and looped, stereo, MIDI 36-84. Pads, hymns, cinematic swells, soft vocal harmony under melodies; hold long notes.

- Source: FluidR3_GM.sf2 "Ahh Choir" (bank 0, program 52), Debian package fluid-soundfont-gm 3.1-6 (http://deb.debian.org/debian/pool/main/f/fluid-soundfont/)
- License: MIT (see LICENSE.txt)
- Attribution: Choir samples from the Fluid R3 GM SoundFont by Frank Wen (2000-2002, 2008) and Toby Smithe (2008), MIT license.
- Zones: 15 (15 roots x 1 velocity layer(s)), range MIDI 36-84

Built by `tools/build-instruments-keys.py choir`: onset trimmed to the attack (2 ms pre-roll, zero-crossing, 1 ms fade-in), one global normalization per instrument (layer dynamics preserved in the files; `gain` per velocity layer sets the pp..f level curve), sustain loop on zero crossings with baked equal-power crossfade, Ogg Vorbis q4, 44.1 kHz stereo. `tune` (cents) only where the source deviates > 30 cents.
