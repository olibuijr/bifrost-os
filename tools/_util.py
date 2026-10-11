"""Helpers shared by tools/bifrost.py and springboard/tools/aap.py."""
import os
import re


def atomic_write(path, data, suffix=".part"):
    """Writes `data` (bytes or str) to `path` through `<path><suffix>` and a rename."""
    tmp = path + suffix
    mode, enc = ("wb", None) if isinstance(data, (bytes, bytearray)) else ("w", "utf-8")
    with open(tmp, mode, encoding=enc) as f:
        f.write(data)
    os.replace(tmp, path)


def changelog_sections(text):
    """[(version, body)] for each `## <version> ...` heading of a CHANGELOG, newest first as written."""
    out = []
    for part in re.split(r"^## ", text, flags=re.M)[1:]:
        head, _, body = part.partition("\n")
        out.append((head.split()[0] if head.split() else "", body))
    return out


def changelog_newest(text):
    """The version of the first `## x.y.z` heading, or None."""
    m = re.search(r"^## (\d+\.\d+\.\d+)", text, re.M)
    return m[1] if m else None


def changelog_has(text, version):
    """True when the CHANGELOG has a `## <version>` section with at least one `- ` note."""
    return any(v == version and re.search(r"^- \S", body, flags=re.M) for v, body in changelog_sections(text))
