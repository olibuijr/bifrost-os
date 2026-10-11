#!/usr/bin/env python3
"""Generate springboard/src/media/sbr_tables.rs from the SBR tables pickle.

usage: tools/sbr/emit_sbr_tables.py TABLES_PKL [OUT_RS]

TABLES_PKL is the pickle of (qmf_window, noise_table, huffman_tables) extracted from FFmpeg's
libavcodec tables; it is not in the repository. OUT_RS defaults to springboard/src/media/sbr_tables.rs.
"""
import os
import pickle
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DEFAULT_OUT = os.path.join(ROOT, "springboard", "src", "media", "sbr_tables.rs")

HEADER = '''//! Constant tables of the SBR tool (ISO/IEC 14496-3 4.A.6): the 640-tap QMF
//! prototype window, the 512-entry complex noise table and the canonical
//! Huffman codes of the envelope and noise-floor scalefactors. These are
//! numeric data defined by the standard; they were transcribed from the
//! tables in FFmpeg's libavcodec (aacsbrdata.h, sbrdsp_template.c,
//! aac/aacdec_tab.c) after checking sizes and symmetry.
//!
//! The Huffman tables list `(symbol, code length)` in code order; codes are
//! canonical (each code is the previous one plus one, shifted left when the
//! length grows), and the delta is `symbol + HUFF_OFFSET[table]`.

'''


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    out_path = argv[2] if len(argv) > 2 else DEFAULT_OUT
    with open(argv[1], "rb") as f:
        us, nz, tabs = pickle.load(f)
    nz = nz[:1024]
    o = [HEADER]
    o.append('pub const QMF_WINDOW: [f32; 640] = [\n')
    for i in range(0, 640, 8):
        o.append('    ' + ', '.join(repr(v) for v in us[i:i + 8]) + ',\n')
    o.append('];\n\npub const NOISE: [[f32; 2]; 512] = [\n')
    for i in range(512):
        o.append('    [%r, %r],\n' % (nz[2 * i], nz[2 * i + 1]))
    o.append('];\n\n')
    o.append('pub const HUFF_OFFSET: [i32; 10] = [-60, -60, -24, -24, -31, -31, -12, -12, -31, -12];\n\n')
    o.append('pub const T_ENV_1_5: usize = 0;\npub const F_ENV_1_5: usize = 1;\npub const T_ENV_BAL_1_5: usize = 2;\n'
             'pub const F_ENV_BAL_1_5: usize = 3;\npub const T_ENV_3_0: usize = 4;\npub const F_ENV_3_0: usize = 5;\n'
             'pub const T_ENV_BAL_3_0: usize = 6;\npub const F_ENV_BAL_3_0: usize = 7;\npub const T_NOISE_3_0: usize = 8;\n'
             'pub const T_NOISE_BAL_3_0: usize = 9;\n\n')
    o.append('/// `(symbol, code length)` per table, in code order.\npub const HUFF: [&[(u8, u8)]; 10] = [\n')
    for _name, _code, lengths in tabs:
        o.append('    &[\n')
        for i in range(0, len(lengths), 8):
            o.append('        ' + ' '.join('(%d, %d),' % p for p in lengths[i:i + 8]) + '\n')
        o.append('    ],\n')
    o.append('];\n')
    with open(out_path, "w") as f:
        f.write(''.join(o))
    # sanity, as in the original generator
    print(us[320], us[319], us[321], max(us), len(nz), nz[:2])
    print(sum(1 for x in nz if abs(x) > 1))
    print("wrote", out_path)


if __name__ == "__main__":
    main(sys.argv)
