# Upright Bass

Acoustic upright/double bass, pizzicato (plucked), 2 velocity layers, E1-G4 (MIDI 28-67). Walking jazz bass, acoustic ballads, orchestral pizz bass lines.

- Source: https://github.com/sgossner/VSCO-2-CE/tree/master/Strings/Solo%20Contrabass/Pizz
- License: CC0 1.0 (see LICENSE.txt)
- Attribution: VS Chamber Orchestra: Community Edition (VSCO-2 CE) by Versilian Studios (Sam Gossner, Simon Dalzell; sample cutting by Elan Hickler/Soundemote). CC0 1.0.
- Zones: 28 (14 roots x 2 velocity layer(s)), range MIDI 26-67

Built by `tools/build-instruments-keys.py uprightbass`: onset trimmed to the attack (2 ms pre-roll, zero-crossing, 1 ms fade-in), one global normalization per instrument (layer dynamics preserved in the files; `gain` per velocity layer sets the pp..f level curve), natural decay, no loops, shortened with a cosine fade-out, Ogg Vorbis q4, 44.1 kHz stereo. `tune` (cents) only where the source deviates > 30 cents.
