#!/usr/bin/env python3
"""Tests for tools/build-wordnet-stardict.py. The WordNet files are a few generated lines in the database layout; the
real tarball is never read.

Run: python3 tools/test_build_wordnet_stardict.py
"""
import importlib.util
import io
import os
import shutil
import struct
import tarfile
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("wordnet", os.path.join(HERE, "build-wordnet-stardict.py"))
wn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wn)

HEADER = "  1 This software and database is being provided to you, the LICENSEE, by  \n" \
         "  2 Princeton University under the following license.  \n"
DATA_NOUN = HEADER + (
    "00000001 05 n 02 goose 0 Anser 0 000 | web-footed long-necked birds  \n"
    "00000002 05 n 01 ice_cream 0 000 | frozen dessert containing cream and sugar  \n"
)
INDEX_NOUN = HEADER + "goose n 1 0 1 0 00000001  \nice_cream n 1 0 1 0 00000002  \n"
DATA_VERB = HEADER + "00000003 30 v 01 goose 0 000 00 | pinch in the buttocks  \n"
INDEX_VERB = HEADER + "goose v 1 0 1 0 00000003  \n"
DATA_ADJ = HEADER + "00000004 00 a 01 able(a) 0 000 | having the necessary means  \n"
INDEX_ADJ = HEADER + "able a 1 0 1 0 00000004  \n"
NOUN_EXC = "geese goose\nmice mouse\n"


def tarball(path, license_file=True):
    files = {
        "WordNet-3.0/dict/data.noun": DATA_NOUN,
        "WordNet-3.0/dict/index.noun": INDEX_NOUN,
        "WordNet-3.0/dict/data.verb": DATA_VERB,
        "WordNet-3.0/dict/index.verb": INDEX_VERB,
        "WordNet-3.0/dict/data.adj": DATA_ADJ,
        "WordNet-3.0/dict/index.adj": INDEX_ADJ,
        "WordNet-3.0/dict/noun.exc": NOUN_EXC,
        "WordNet-3.0/dict/Makefile": "ignored",
    }
    if license_file:
        files["WordNet-3.0/LICENSE"] = "WordNet Release 3.0\n\nCopyright 2006 by Princeton University.\n"
    with tarfile.open(path, "w:gz") as tf:
        for name, text in files.items():
            data = text.encode()
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))


def read_stardict(out):
    """headword -> definition text, read the way bifrost_ui::stardict does."""
    idx = open(os.path.join(out, "wordnet.idx"), "rb").read()
    data = open(os.path.join(out, "wordnet.dict"), "rb").read()
    words, order, i = {}, [], 0
    while i < len(idx):
        nul = idx.index(0, i)
        word = idx[i:nul].decode()
        off, size = struct.unpack(">II", idx[nul + 1:nul + 9])
        words[word] = data[off:off + size].decode()
        order.append(word)
        i = nul + 9
    return words, order


class Build(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.tar = os.path.join(self.tmp, "wn.tar.gz")
        self.out = os.path.join(self.tmp, "out")

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_every_lemma_is_a_headword_with_its_senses_per_part_of_speech(self):
        tarball(self.tar)
        self.assertEqual(wn.build(self.tar, self.out), 4)  # goose, ice cream, able, geese (mice has no base)
        words, _ = read_stardict(self.out)
        self.assertEqual(words["goose"], "noun\n1. web-footed long-necked birds\n   Synonyms: Anser\n\n"
                                         "verb\n1. pinch in the buttocks")
        self.assertIn("ice cream", words)  # underscores become spaces
        self.assertEqual(words["able"], "adjective\n1. having the necessary means")  # marker dropped, no self-synonym

    def test_an_inflected_form_points_at_its_base_definition(self):
        tarball(self.tar)
        wn.build(self.tar, self.out)
        words, _ = read_stardict(self.out)
        self.assertEqual(words["geese"], words["goose"])
        self.assertNotIn("mice", words)
        # the alias shares the block: the data file holds each definition once
        size = os.path.getsize(os.path.join(self.out, "wordnet.dict"))
        self.assertEqual(size, sum(len(words[w].encode()) for w in ("goose", "ice cream", "able")))

    def test_ifo_names_the_book_and_counts_match(self):
        tarball(self.tar)
        wn.build(self.tar, self.out)
        ifo = open(os.path.join(self.out, "wordnet.ifo")).read().splitlines()
        self.assertEqual(ifo[0], "StarDict's dict ifo file")
        self.assertIn("wordcount=4", ifo)
        self.assertIn("sametypesequence=m", ifo)
        self.assertIn(f"idxfilesize={os.path.getsize(os.path.join(self.out, 'wordnet.idx'))}", ifo)
        self.assertTrue(any(line.startswith("bookname=") for line in ifo))

    def test_index_is_in_stardict_order(self):
        tarball(self.tar)
        wn.build(self.tar, self.out)
        _, order = read_stardict(self.out)
        self.assertEqual(order, sorted(order, key=wn.stardict_key))
        self.assertEqual(order, ["able", "geese", "goose", "ice cream"])

    def test_the_licence_notice_ships_from_license_or_the_data_header(self):
        tarball(self.tar)
        wn.build(self.tar, self.out)
        self.assertIn("Princeton University", open(os.path.join(self.out, "LICENSE-WordNet")).read())
        os.unlink(self.tar)
        tarball(self.tar, license_file=False)
        shutil.rmtree(self.out)
        wn.build(self.tar, self.out)
        text = open(os.path.join(self.out, "LICENSE-WordNet")).read()
        self.assertTrue(text.startswith("This software and database"))

    def test_the_pinned_source_names_a_url_checksum_and_licence(self):
        src = wn.load_source()
        self.assertTrue(src["url"].startswith("https://"))
        self.assertEqual(len(src["sha256"]), 64)
        self.assertIn("WordNet", src["license"])


if __name__ == "__main__":
    unittest.main()
