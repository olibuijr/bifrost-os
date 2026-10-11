# Nylon Guitar

Spanish classical nylon-string guitar, plucked, chromatic-ish samples E2-C6, 2 velocity layers (soft is darker), natural decay. Fingerstyle arpeggios, bossa, folk, classical guitar parts.

- Source: https://freepats.zenvoid.org/Guitar/acoustic-guitar.html (SpanishClassicalGuitar-SFZ+FLAC-20190618)
- License: CC0 1.0 (see LICENSE.txt)
- Attribution: Spanish Classical Guitar by roberto@zenvoid.org for the FreePats project, CC0 1.0 (credit appreciated).
- Zones: 96 (48 roots x 2 velocity layer(s)), range MIDI 28-88

Built by `tools/build-instruments-keys.py nylonguitar`: onset trimmed to the attack (2 ms pre-roll, zero-crossing, 1 ms fade-in), one global normalization per instrument (layer dynamics preserved in the files; `gain` per velocity layer sets the pp..f level curve), natural decay, no loops, shortened with a cosine fade-out, Ogg Vorbis q4, 44.1 kHz stereo. `tune` (cents) only where the source deviates > 30 cents.
