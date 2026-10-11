# Space — impulse response attribution

Plugin code: MIT. The impulse responses (`*.ogg`) are derived from the
**OpenAIR — Open Acoustic Impulse Response Library**, Audiolab, University of York
(https://www.openair.hosted.york.ac.uk/, files from https://webfiles.york.ac.uk/OPENAIR/IRs/),
licensed **Creative Commons Attribution 4.0 International (CC BY 4.0)**
https://creativecommons.org/licenses/by/4.0/

Changes made: B-format files decoded to stereo (L = W + 0.7Y, R = W - 0.7Y), onset-aligned
(pre-delay silence removed), truncated with a fade-out, resampled to 44.1 kHz stereo,
peak-normalized to -1 dBFS, encoded as Ogg Vorbis (q4).

| File | Label | OpenAIR site | Source file | Length |
|---|---|---|---|---|
| hall.ogg | Hall | Jack Lyons Concert Hall, University of York | b-format/rir_jack_lyons_lp1_96k.wav | 3.5 s |
| cathedral.ogg | Cathedral | York Minster | stereo/minster1_000_ortf_48k.wav | 3.5 s |
| church.ogg | Church | St George's Episcopal Church | stereo/st_georges_far.wav | 3.0 s |
| chamber.ogg | Chamber | York Guildhall Council Chamber | b-format/councilchamber_s1_r1_ir_1_96000.wav | 2.5 s |
| room.ogg | Room | Terry's Typing Room (Terry's Chocolate Factory, York) | stereo/terrys_typing_ortf.wav | 2.0 s |
| studio.ogg | Studio | Genesis 6 Studio, live room (drum set) | b-format/snare_ir.wav | 1.2 s |

Credit line: "Impulse responses from the OpenAIR library (openair.hosted.york.ac.uk), Audiolab,
University of York, CC BY 4.0."
