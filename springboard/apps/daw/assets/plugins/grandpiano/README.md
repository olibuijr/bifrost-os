# Grand Piano

Concert grand (Yamaha C5, Salamander), 4 velocity layers pp/mp/mf/f, full 88 keys A0-C8, stereo, natural decay. Classical/solo piano, sonatas, ballads, chords and melody; velocity shapes tone.

- Source: https://archive.org/details/SalamanderGrandPianoV3 (Salamander Grand Piano V3, 44.1 kHz 16-bit)
- License: CC-BY 3.0 (see LICENSE.txt)
- Attribution: Salamander Grand Piano V3 by Alexander Holm (https://archive.org/details/SalamanderGrandPianoV3), licensed CC-BY 3.0 (https://creativecommons.org/licenses/by/3.0/). Resampled/trimmed/encoded for DAW.
- Zones: 120 (30 roots x 4 velocity layer(s)), range MIDI 21-108

Built by `tools/build-instruments-keys.py grandpiano`: onset trimmed to the attack (2 ms pre-roll, zero-crossing, 1 ms fade-in), one global normalization per instrument (layer dynamics preserved in the files; `gain` per velocity layer sets the pp..f level curve), natural decay, no loops, shortened with a cosine fade-out, Ogg Vorbis q4, 44.1 kHz stereo. `tune` (cents) only where the source deviates > 30 cents.
