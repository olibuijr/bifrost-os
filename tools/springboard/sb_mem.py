#!/usr/bin/env python3
"""Peak memory of the springboard unit tests: all at once, then the heaviest modules one at a time.

usage: tools/springboard/sb_mem.py TEST_BIN

TEST_BIN is the springboard test executable, e.g. the one `cargo test --no-run` prints
(springboard/target/debug/deps/springboard-<hash>).
"""
import collections
import os
import subprocess
import sys


def peak(binary, args):
    pid = os.fork()
    if pid == 0:
        os.nice(15)
        fd = os.open(os.devnull, os.O_WRONLY)
        os.dup2(fd, 1)
        os.dup2(fd, 2)
        os.execv(binary, [binary, *args])
    _, _, ru = os.wait4(pid, 0)
    return ru.ru_maxrss // 1024


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    binary = argv[1]
    print("all tests, 16 threads:", peak(binary, ["--test-threads=16", "-q"]), "MB")
    listing = subprocess.run([binary, "--list"], capture_output=True, text=True).stdout.splitlines()
    names = [line.split(": ")[0] for line in listing if line.endswith(": test")]
    mods = collections.Counter(n.split("::")[0] for n in names)
    res = sorted(((peak(binary, [m + "::", "--test-threads=1", "-q"]), m, c) for m, c in mods.items()), reverse=True)
    for mb, m, c in res[:8]:
        print(f"{mb:6} MB  {m} ({c} tests, 1 thread)")


if __name__ == "__main__":
    main(sys.argv)
