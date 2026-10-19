#!/usr/bin/env python3
"""Licence guard: a BifrOSt package carries only BifrOSt's own and openly licensed files.

usage: tools/check-package-licenses.py [--allow FILE] [--no-fonttools] [-q] TARGET [TARGET...]

TARGET is a built package (.pkg.tar.zst, or any tar), an app package (.aap) or a directory tree (a package root,
an ISO root, the packed assets). The exit status is 0 when every target is clean, 1 when a target holds an offending
file (the paths are printed to stderr), 2 when a target or the allowlist cannot be read.

A target fails when
  (a) a path is under usr/share/bifrost/ref (or a folder is named bifrost-ref): reference assets are never packaged;
  (b) a font file (.ttf .otf .ttc .otc .woff .woff2) does not name an open licence. Every face of a collection is
      read: the OpenType name table, IDs 13 (licence description), 14 (licence URL) and 0 (copyright). Accepted:
      SIL Open Font License, Apache License, Ubuntu Font Licence, GPL (or AGPL) with font exception, LPPL and the
      GUST Font License, Bitstream Vera and DejaVu, MIT, CC0 and public domain, the IPA Font License. A font that
      says "all rights reserved" and names none of them fails, so does one with no licence text at all.
The allowlist (tools/check-package-licenses-allow.txt) excuses a path or a family, with a mandatory reason:
    path usr/share/fonts/foo/*.ttf -- reason
    family Some Family -- reason
fontTools reads the fonts when it is installed; without it a built-in reader (struct, zlib) does. WOFF2 needs
fontTools with brotli, else the file counts as unreadable (and fails). Archives nested inside a package are not
opened. Tests: tools/test_check_package_licenses.py.
"""
import argparse
import collections
import contextlib
import fnmatch
import io
import os
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_ALLOW = os.path.join(ROOT, "tools", "check-package-licenses-allow.txt")
FONT_EXTS = (".ttf", ".otf", ".ttc", ".otc", ".woff", ".woff2")
REF_PREFIX = "usr/share/bifrost/ref"
ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"

# path, link target (symlinks, hard links), font bytes (regular font files), read error (a font that could not be read)
Entry = collections.namedtuple("Entry", "path link data err", defaults=(None, None, None))


class ScanError(Exception):
    """A target that cannot be read at all."""


class FontError(Exception):
    """A font file that cannot be parsed."""


class AllowError(ValueError):
    pass


# ---------------------------------------------------------------- paths

def norm(path):
    """`./usr//bin/x` and `/usr/bin/x` both become `usr/bin/x`."""
    return "/".join(part for part in path.split("/") if part not in ("", "."))


def is_font(path):
    return path.lower().endswith(FONT_EXTS)


def path_problem(entry):
    """Why the path (or the target of a link) may not be packaged, or None."""
    for what, value in (("path", entry.path), ("link target", entry.link or "")):
        value = norm(value)
        if value == REF_PREFIX or value.startswith(REF_PREFIX + "/"):
            return f"{what} is under /{REF_PREFIX}: reference assets are never packaged"
        if "bifrost-ref" in value.split("/"):
            return f"{what} goes through a bifrost-ref folder: reference assets are never packaged"
    return None


# ---------------------------------------------------------------- font names

def _unpack(fmt, data, offset):
    try:
        return struct.unpack_from(fmt, data, offset)
    except struct.error:
        raise FontError("truncated font file") from None


def _decode_name(platform, raw):
    if platform in (0, 3):
        return raw.decode("utf-16-be", "replace")
    if platform == 1:
        return raw.decode("mac_roman", "replace")
    return raw.decode("latin-1")


def _parse_name_table(blob):
    """{name ID: [text, ...]} (every language) from a raw `name` table."""
    _format, count, string_offset = _unpack(">HHH", blob, 0)
    names = {}
    for i in range(count):
        platform, _encoding, _language, name_id, length, offset = _unpack(">6H", blob, 6 + 12 * i)
        raw = blob[string_offset + offset:string_offset + offset + length]
        names.setdefault(name_id, []).append(_decode_name(platform, raw).strip("\x00 \t\r\n"))
    return names


def _sfnt_name_table(data, base=0):
    """The raw `name` table of the font whose sfnt header starts at `base`, or None."""
    if data[base:base + 4] not in (b"\x00\x01\x00\x00", b"OTTO", b"true", b"typ1"):
        raise FontError("not a TrueType or OpenType font")
    (count,) = _unpack(">H", data, base + 4)
    for i in range(count):
        tag, _checksum, offset, length = _unpack(">4sIII", data, base + 12 + 16 * i)
        if tag == b"name":
            blob = data[offset:offset + length]
            if len(blob) < length:
                raise FontError("name table runs past the end of the file")
            return blob
    return None


def _woff_name_table(data):
    (count,) = _unpack(">H", data, 12)
    for i in range(count):
        tag, offset, comp_length, orig_length, _checksum = _unpack(">4sIIII", data, 44 + 20 * i)
        if tag == b"name":
            blob = data[offset:offset + comp_length]
            if len(blob) < comp_length:
                raise FontError("name table runs past the end of the file")
            if comp_length < orig_length:
                try:
                    blob = zlib.decompress(blob)
                except zlib.error as exc:
                    raise FontError(f"name table does not decompress: {exc}") from None
            return blob
    return None


def _builtin_faces(data):
    """One {name ID: [text]} per face, read without fontTools."""
    head = data[:4]
    if head == b"ttcf":
        (count,) = _unpack(">I", data, 8)
        if count > 4096:
            raise FontError(f"collection claims {count} faces")
        blobs = [_sfnt_name_table(data, _unpack(">I", data, 12 + 4 * i)[0]) for i in range(count)]
    elif head == b"wOFF":
        blobs = [_woff_name_table(data)]
    elif head == b"wOF2":
        raise FontError("WOFF2 can only be read with fontTools and brotli")
    else:
        blobs = [_sfnt_name_table(data)]
    return [_parse_name_table(blob) if blob is not None else {} for blob in blobs]


def _fonttools_faces(data):
    from fontTools.ttLib import TTCollection, TTFont
    stream = io.BytesIO(data)
    fonts = TTCollection(stream, lazy=True).fonts if data[:4] == b"ttcf" else [TTFont(stream, lazy=True)]
    faces = []
    for font in fonts:
        names = {}
        if "name" in font:
            for record in font["name"].names:
                try:
                    text = record.toUnicode()
                except Exception:  # an undecodable record carries no usable text
                    text = ""
                names.setdefault(record.nameID, []).append(text.strip("\x00 \t\r\n"))
        faces.append(names)
    return faces


def have_fonttools():
    try:
        import fontTools.ttLib  # noqa: F401
    except Exception:
        return False
    return True


def font_faces(data, use_fonttools=True):
    """One {name ID: [text]} per face of the font file `data` (a collection has several)."""
    if use_fonttools and have_fonttools():
        try:
            return _fonttools_faces(data)
        except Exception:  # fall back to the built-in reader, which gives the error text
            pass
    return _builtin_faces(data)


# ---------------------------------------------------------------- licences

_OPEN = [
    ("SIL Open Font License", [r"open\s+font\s+licen[cs]e", r"openfontlicense\.org", r"scripts\.sil\.org/ofl", r"\bOFL\b"]),
    ("Apache License", [r"apache\s+licen[cs]e", r"apache\.org/licenses"]),
    ("Ubuntu Font Licence", [r"ubuntu\s+font\s+licen[cs]e", r"ubuntu\.com/(?:legal/)?font-?licen[cs]e"]),
    ("LPPL / GUST Font License", [r"\bLPPL\b", r"latex\s+project\s+public\s+licen[cs]e", r"gust\s+font\s+licen[cs]e"]),
    ("Bitstream Vera / DejaVu", [r"bitstream\s+vera", r"dejavu", r"fonts\s+accompanying\s+this\s+licen[cs]e"]),
    ("MIT", [r"\bMIT\s+licen[cs]e", r"opensource\.org/licenses/mit", r"permission\s+is\s+hereby\s+granted,?\s+free\s+of\s+charge"]),
    ("CC0 / public domain", [r"\bCC0\b", r"creativecommons\.org/publicdomain/zero", r"(?<!not )(?<!not in the )public\s+domain"]),
    ("IPA Font License", [r"\bIPA\s+font\s+licen[cs]e", r"ipafont"]),
]
OPEN_LICENCES = [(name, [re.compile(rx, re.I) for rx in rxs]) for name, rxs in _OPEN]
_GPL = re.compile(r"\bA?GPL\b|general\s+public\s+licen[cs]e", re.I)
_EXCEPTION = re.compile(r"exception", re.I)
_RESERVED = re.compile(r"all\s+rights\s+reserved", re.I)


def licence_text(names):
    """The name-table text that can state a licence: IDs 13, 14 and 0."""
    return " ".join(t for name_id in (13, 14, 0) for t in names.get(name_id, []) if t)


def open_licence(text):
    """The name of the open licence the text states, or None."""
    for name, patterns in OPEN_LICENCES:
        if any(p.search(text) for p in patterns):
            return name
    if _GPL.search(text) and _EXCEPTION.search(text):
        return "GPL with font exception"
    return None


def licence_problem(names):
    """Why a face has no open licence, or None when it names one."""
    text = licence_text(names)
    if open_licence(text):
        return None
    description = " ".join(t for name_id in (13, 14) for t in names.get(name_id, []) if t).strip()
    if description:
        problem = f"licence is not a known open licence: {description[:90]!r}"
    else:
        problem = "no licence description (name ID 13 or 14) in the font"
    if _RESERVED.search(text):
        problem += "; it says all rights reserved"
    return problem


def families(names):
    return {t.lower() for name_id in (1, 4, 16) for t in names.get(name_id, []) if t}


def face_label(names):
    for name_id in (16, 1, 4):
        for t in names.get(name_id, []):
            if t:
                return t
    return "unnamed"


# ---------------------------------------------------------------- allowlist

class Allow:
    def __init__(self):
        self.paths = []      # (glob, reason)
        self.families = {}   # lower-case family -> reason

    def path_reason(self, path):
        for glob, reason in self.paths:
            target = path if "/" in glob else path.rsplit("/", 1)[-1]
            if fnmatch.fnmatchcase(target, glob):
                return reason
        return None

    def family_reason(self, names):
        for family in families(names):
            if family in self.families:
                return self.families[family]
        return None


def parse_allowlist(text, source="allowlist"):
    allow = Allow()
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        head, separator, reason = line.partition(" -- ")
        kind, _, value = head.strip().partition(" ")
        value, reason = value.strip(), reason.strip()
        if not separator or not reason or not value or kind not in ("path", "family"):
            raise AllowError(f"{source}:{number}: expected 'path GLOB -- reason' or 'family NAME -- reason'")
        if kind == "path":
            if not re.search(r"[^*?/\[\]]", value):
                raise AllowError(f"{source}:{number}: a blanket path pattern excuses everything: {value}")
            allow.paths.append((norm(value), reason))
        else:
            allow.families[value.lower()] = reason
    return allow


def load_allowlist(path=DEFAULT_ALLOW):
    with open(path, encoding="utf-8") as f:
        return parse_allowlist(f.read(), path)


# ---------------------------------------------------------------- targets

def walk_tree(root):
    """Entries of a directory tree. Symbolic links are reported, never followed."""
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames.sort()
        for name in dirnames:
            full = os.path.join(dirpath, name)
            yield Entry(norm(os.path.relpath(full, root)), os.readlink(full) if os.path.islink(full) else None)
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            rel = norm(os.path.relpath(full, root))
            if os.path.islink(full):
                yield Entry(rel, os.readlink(full))
            elif is_font(rel):
                try:
                    with open(full, "rb") as f:
                        yield Entry(rel, None, f.read())
                except OSError as exc:
                    yield Entry(rel, None, None, str(exc))
            else:
                yield Entry(rel)


@contextlib.contextmanager
def _tar_stream(path):
    """A readable binary stream of the tar archive in `path` (plain, or compressed with gzip, xz, bzip2 or zstd)."""
    with open(path, "rb") as probe:
        magic = probe.read(4)
    if magic != ZSTD_MAGIC:
        with open(path, "rb") as stream:
            yield stream
        return
    try:
        from compression import zstd  # Python 3.14 and later
    except ImportError:
        zstd = None
    if zstd is not None:
        with zstd.open(path, "rb") as stream:
            yield stream
        return
    exe = shutil.which("zstd") or shutil.which("unzstd")
    if exe is None:
        raise ScanError(f"{path}: zstd compressed, and neither Python's zstd module nor the zstd tool is available")
    proc = subprocess.Popen([exe, "-dc", "--", path], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        yield proc.stdout
        while proc.stdout.read(1 << 20):  # let the tool finish, so it does not die on a closed pipe
            pass
    finally:
        proc.stdout.close()
        status = proc.wait()
    if status:
        raise ScanError(f"{path}: zstd failed to decompress it (status {status})")


def walk_tar(path):
    """Entries of a package or app package, read as a stream."""
    try:
        with _tar_stream(path) as stream, tarfile.open(fileobj=stream, mode="r|*") as tar:
            for member in tar:
                name = norm(member.name)
                if not name:
                    continue
                if member.issym() or member.islnk():
                    yield Entry(name, member.linkname)
                elif member.isreg() and is_font(name):
                    try:
                        yield Entry(name, None, tar.extractfile(member).read())
                    except (tarfile.TarError, OSError, EOFError) as exc:
                        yield Entry(name, None, None, str(exc))
                else:
                    yield Entry(name)
    except (tarfile.TarError, OSError, EOFError, zlib.error) as exc:
        raise ScanError(f"{path}: not a readable package or tar archive ({exc})") from None


def walk_target(target):
    if os.path.isdir(target):
        return walk_tree(target)
    if os.path.isfile(target):
        return walk_tar(target)
    raise ScanError(f"{target}: no such package or directory")


# ---------------------------------------------------------------- the check

def check_entries(entries, allow, use_fonttools=True):
    """(findings, stats): findings are (path, problem); stats counts fonts checked and excused."""
    findings = []
    stats = {"fonts": 0, "excused": 0}
    for entry in entries:
        problem = path_problem(entry)
        if problem:
            findings.append((entry.path, problem))
        if entry.data is None and entry.err is None:
            continue
        stats["fonts"] += 1
        if allow.path_reason(entry.path):
            stats["excused"] += 1
            continue
        if entry.err is not None:
            findings.append((entry.path, f"font cannot be read: {entry.err}"))
            continue
        try:
            faces = font_faces(entry.data, use_fonttools)
        except FontError as exc:
            findings.append((entry.path, f"font cannot be parsed: {exc}"))
            continue
        for index, names in enumerate(faces):
            if allow.family_reason(names):
                stats["excused"] += 1
                continue
            problem = licence_problem(names)
            if problem:
                where = entry.path if len(faces) == 1 else f"{entry.path} (face {index})"
                findings.append((where, f"font '{face_label(names)}': {problem}"))
    return findings, stats


def check_target(target, allow, use_fonttools=True):
    return check_entries(walk_target(target), allow, use_fonttools)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("targets", nargs="+", metavar="TARGET", help="a package, an .aap or a directory")
    ap.add_argument("--allow", default=DEFAULT_ALLOW, help="allowlist file (default: tools/check-package-licenses-allow.txt)")
    ap.add_argument("--no-fonttools", action="store_true", help="always use the built-in name-table reader")
    ap.add_argument("-q", "--quiet", action="store_true", help="print only problems")
    args = ap.parse_args(argv)
    try:
        allow = load_allowlist(args.allow)
    except (OSError, AllowError) as exc:
        print(f"check-package-licenses: {exc}", file=sys.stderr)
        return 2
    failed = errored = False
    for target in args.targets:
        try:
            findings, stats = check_target(target, allow, not args.no_fonttools)
        except ScanError as exc:
            print(f"check-package-licenses: {exc}", file=sys.stderr)
            errored = True
            continue
        for where, problem in findings:
            print(f"FAIL {target}: {where}: {problem}", file=sys.stderr)
        if findings:
            failed = True
            print(f"check-package-licenses: {target}: {len(findings)} problem(s), {stats['fonts']} font file(s) checked", file=sys.stderr)
        elif not args.quiet:
            note = f", {stats['excused']} excused by the allowlist" if stats["excused"] else ""
            print(f"check-package-licenses: {target}: ok, {stats['fonts']} font file(s) checked{note}")
    return 1 if failed else 2 if errored else 0


if __name__ == "__main__":
    sys.exit(main())
