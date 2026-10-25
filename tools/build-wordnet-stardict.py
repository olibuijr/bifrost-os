#!/usr/bin/env python3
"""Builds the system English dictionary: WordNet 3.0 converted to a StarDict dictionary.

usage: tools/build-wordnet-stardict.py [--tarball FILE] OUT_DIR

Writes OUT_DIR/wordnet.ifo, wordnet.idx and wordnet.dict (plain-text definitions, `sametypesequence=m`) and
OUT_DIR/LICENSE-WordNet (the Princeton notice the licence asks every copy to carry). The package installs the three
dictionary files in /usr/share/stardict, where the Dictionary app and the shell's Look Up read them
(bifrost_ui::stardict::SYSTEM_DICTIONARIES), and the notice in /usr/share/licenses/springboard-shell/.

The source is the WordNet 3.0 release tarball named in packaging/arch/dictionaries/wordnet.json (URL and SHA-256),
downloaded once into out/upstream/ and checked; `--tarball` reads a local copy instead. The StarDict files are built
once into out/dictionaries/wordnet/ (with a STAMP of the tarball's SHA-256 and this script's) and only rebuilt when
the cache is missing or either changes; a needed download that fails stops with an error. WordNet 3.0 is licensed by
Princeton University under the WordNet licence, which permits use, copying, modification and distribution for any
purpose without fee as long as the copyright notice and disclaimer appear on all copies.

Every lemma of the four index files is a headword; its definition lists the senses per part of speech, each with its
gloss and the other words of the synset. Inflected forms of the exception lists (geese, ran, better) are headwords
too and point at the definition of their base form. Only the standard library is used.
Tests: tools/test_build_wordnet_stardict.py.
"""
import argparse
import hashlib
import io
import json
import os
import re
import shutil
import struct
import sys
import tarfile
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "packaging", "arch", "dictionaries", "wordnet.json")
STEM = "wordnet"
BOOKNAME = "WordNet 3.0 (English)"

# WordNet's file suffixes, in the order the senses are shown, and the names shown for them
POS = (("noun", "noun"), ("verb", "verb"), ("adj", "adjective"), ("adv", "adverb"))


def load_source(path=SOURCE):
    with open(path, encoding="utf-8") as f:
        src = json.load(f)
    for key in ("url", "sha256", "license"):
        if not src.get(key):
            raise ValueError(f"{path}: {key} is missing")
    return src


def fetch(src, cache_dir=os.path.join(ROOT, "out", "upstream")):
    """The pinned tarball, downloaded once into `cache_dir` and checked against its SHA-256."""
    cache = os.path.join(cache_dir, f"wordnet-{src['sha256'][:12]}.tar.gz")

    def ok():
        if not os.path.isfile(cache):
            return False
        with open(cache, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest() == src["sha256"]

    if not ok():
        os.makedirs(cache_dir, exist_ok=True)
        print(f"Fetching {src['url']}", flush=True)
        part = cache + ".part"
        with urllib.request.urlopen(src["url"], timeout=120) as r, open(part, "wb") as f:
            shutil.copyfileobj(r, f)
        os.replace(part, cache)
        if not ok():
            os.unlink(cache)
            raise ValueError("the WordNet download does not match its SHA-256")
    return cache


def read_tarball(path):
    """The files of the database folder (`dict/<name>`) and the LICENSE file, by name."""
    files = {}
    with tarfile.open(path) as tf:
        for m in tf.getmembers():
            name = os.path.basename(m.name)
            in_dict = os.path.basename(os.path.dirname(m.name)) == "dict"
            if m.isfile() and (in_dict or name == "LICENSE"):
                files[name] = tf.extractfile(m).read().decode("utf-8", "replace")
    return files


def lemma_text(word):
    """`ice_cream` -> `ice cream`; an adjective's syntactic marker (`(a)`, `(p)`, `(ip)`) is dropped."""
    return re.sub(r"\((a|p|ip)\)$", "", word).replace("_", " ")


def is_header(line):
    """The licence lines at the top of every WordNet file start with two spaces."""
    return line.startswith("  ")


def parse_data(text):
    """data.<pos>: synset offset -> (words, gloss)."""
    out = {}
    for line in text.splitlines():
        if not line or is_header(line):
            continue
        head, _, gloss = line.partition(" | ")
        f = head.split()
        if len(f) < 4:
            continue
        try:
            count = int(f[3], 16)
        except ValueError:
            continue
        words = [lemma_text(f[4 + 2 * i]) for i in range(count) if 4 + 2 * i < len(f)]
        out[f[0]] = (words, gloss.strip())
    return out


def parse_index(text):
    """index.<pos>: (lemma, [synset offsets]) in sense order."""
    out = []
    for line in text.splitlines():
        if not line or is_header(line):
            continue
        f = line.split()
        if len(f) < 4:
            continue
        try:
            synsets = int(f[2])
        except ValueError:
            continue
        if synsets <= 0 or synsets > len(f):
            continue
        out.append((f[0], f[-synsets:]))
    return out


def parse_exceptions(text):
    """<pos>.exc: (inflected form, [base forms])."""
    out = []
    for line in text.splitlines():
        f = line.split()
        if len(f) >= 2:
            out.append((f[0], f[1:]))
    return out


def definition(lemma, senses):
    """The plain text of one headword: per part of speech its numbered senses, each with the synset's other words."""
    blocks = []
    for label, synsets in senses:
        lines = [label]
        for n, (words, gloss) in enumerate(synsets, 1):
            lines.append(f"{n}. {gloss}")
            others = [w for w in words if w.lower() != lemma.lower()]
            if others:
                lines.append("   Synonyms: " + ", ".join(dict.fromkeys(others)))
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks)


def entries(files):
    """(headword, definition) for every lemma, then (inflected form, base form) for every exception form that is not a
    headword itself."""
    defs = {}
    for suffix, label in POS:
        data = parse_data(files.get(f"data.{suffix}", ""))
        for raw, offsets in parse_index(files.get(f"index.{suffix}", "")):
            lemma = lemma_text(raw)
            synsets = [data[o] for o in offsets if o in data]
            if synsets:
                defs.setdefault(lemma, []).append((label, synsets))
    words = {k: definition(k, v) for k, v in defs.items()}
    aliases = {}
    for suffix, _ in POS:
        for form, bases in parse_exceptions(files.get(f"{suffix}.exc", "")):
            form = lemma_text(form)
            if form in words or form in aliases:
                continue
            base = next((lemma_text(b) for b in bases if lemma_text(b) in words), None)
            if base:
                aliases[form] = base
    return words, aliases


def stardict_key(word):
    """StarDict's index order: case-insensitive ASCII compare, then byte compare."""
    b = word.encode("utf-8")
    return (b.lower(), b)


def write_stardict(words, aliases, out_dir, stem=STEM, bookname=BOOKNAME, source=""):
    """Writes `stem`.ifo/.idx/.dict. An alias shares its base's definition block. Returns the number of headwords."""
    os.makedirs(out_dir, exist_ok=True)
    data = io.BytesIO()
    where = {}
    for word in sorted(words, key=stardict_key):
        block = words[word].encode("utf-8")
        where[word] = (data.tell(), len(block))
        data.write(block)
    rows = dict(where)
    for form, base in aliases.items():
        rows[form] = where[base]
    idx = io.BytesIO()
    for word in sorted(rows, key=stardict_key):
        off, size = rows[word]
        idx.write(word.encode("utf-8") + b"\0" + struct.pack(">II", off, size))
    with open(os.path.join(out_dir, f"{stem}.dict"), "wb") as f:
        f.write(data.getvalue())
    with open(os.path.join(out_dir, f"{stem}.idx"), "wb") as f:
        f.write(idx.getvalue())
    ifo = [
        "StarDict's dict ifo file",
        "version=2.4.2",
        f"bookname={bookname}",
        f"wordcount={len(rows)}",
        f"idxfilesize={len(idx.getvalue())}",
        "sametypesequence=m",
        "author=Princeton University",
        f"website={source}" if source else "",
        "description=WordNet 3.0, a lexical database of English. Licensed under the WordNet 3.0 licence (see "
        "/usr/share/licenses/springboard-shell/LICENSE-WordNet).",
    ]
    with open(os.path.join(out_dir, f"{stem}.ifo"), "w", encoding="utf-8") as f:
        f.write("\n".join(line for line in ifo if line) + "\n")
    return len(rows)


def license_text(files):
    """The Princeton notice: the tarball's LICENSE file, else the header of data.noun without its line numbers."""
    if "Princeton University" in files.get("LICENSE", ""):
        return files["LICENSE"].strip() + "\n"
    out = []
    for line in files.get("data.noun", "").splitlines():
        if not is_header(line):
            break
        out.append(re.sub(r"^\s*\d+ ?", "", line).rstrip())
    text = "\n".join(out).strip()
    if "Princeton University" not in text:
        raise ValueError("the WordNet licence notice was not found in data.noun")
    return text + "\n"


def build(tarball, out_dir, source=""):
    files = read_tarball(tarball)
    words, aliases = entries(files)
    if not words:
        raise ValueError(f"{tarball}: no WordNet entries found")
    count = write_stardict(words, aliases, out_dir, source=source)
    with open(os.path.join(out_dir, "LICENSE-WordNet"), "w", encoding="utf-8") as f:
        f.write(license_text(files))
    return count


# what a built dictionary consists of; the build cache holds all of them or is rebuilt
OUTPUTS = (f"{STEM}.ifo", f"{STEM}.idx", f"{STEM}.dict", "LICENSE-WordNet")
STAMP = "STAMP"


def build_stamp(src):
    """What the cached build was made from: the pinned tarball's SHA-256 and this converter's own source, so a new
    pin or a change to the conversion rebuilds it."""
    with open(os.path.abspath(__file__), "rb") as f:
        me = hashlib.sha256(f.read()).hexdigest()
    return f"{src['sha256']} {me}\n"


def cached_build(src, tarball_for, cache_dir):
    """The built dictionary in `cache_dir`, built from `tarball_for()` only when the cache is missing, incomplete or
    was made from another tarball or converter (`build_stamp`). Returns the headword count, or None when the cache
    was used as it was."""
    stamp = build_stamp(src)
    stamp_path = os.path.join(cache_dir, STAMP)
    try:
        with open(stamp_path, encoding="utf-8") as f:
            fresh = f.read() == stamp
    except OSError:
        fresh = False
    if fresh and all(os.path.isfile(os.path.join(cache_dir, n)) for n in OUTPUTS):
        return None
    tarball = tarball_for()
    shutil.rmtree(cache_dir, ignore_errors=True)
    part = cache_dir + ".part"
    shutil.rmtree(part, ignore_errors=True)
    os.makedirs(part)
    count = build(tarball, part, source=src.get("homepage", ""))
    with open(os.path.join(part, STAMP), "w", encoding="utf-8") as f:
        f.write(stamp)
    os.replace(part, cache_dir)
    return count


def copy_outputs(cache_dir, out_dir):
    """The built dictionary and its licence notice, copied from the cache into `out_dir`."""
    os.makedirs(out_dir, exist_ok=True)
    for n in OUTPUTS:
        shutil.copyfile(os.path.join(cache_dir, n), os.path.join(out_dir, n))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out_dir")
    ap.add_argument("--tarball", help="a local copy of the pinned tarball (its SHA-256 is still checked)")
    ap.add_argument("--cache", default=os.path.join(ROOT, "out", "dictionaries", STEM), help="the build cache (default out/dictionaries/wordnet)")
    a = ap.parse_args()
    src = load_source()

    def tarball_for():
        if a.tarball:
            with open(a.tarball, "rb") as f:
                if hashlib.sha256(f.read()).hexdigest() != src["sha256"]:
                    sys.exit(f"{a.tarball} does not match the SHA-256 in {SOURCE}")
            return a.tarball
        try:
            return fetch(src)
        except (OSError, ValueError) as e:
            sys.exit(f"build-wordnet-stardict: the WordNet tarball is not cached and could not be downloaded from {src['url']}: {e}")

    count = cached_build(src, tarball_for, a.cache)
    copy_outputs(a.cache, a.out_dir)
    print(f"{a.out_dir}: {'cached build' if count is None else f'{count} headwords'}")


if __name__ == "__main__":
    main()
