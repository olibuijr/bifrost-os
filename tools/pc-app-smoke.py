#!/usr/bin/env python3
"""Exercise real desktop SDK binaries: identity, buffers, rendering and resize.

Uses isolated temporary app/home data and a socketpair, without a compositor or
device. Does not test installed sandbox policy, portal support or audio hardware.
No touches or playback/record commands are sent. Run after the fast app build.
"""
import argparse
import array
import json
import mmap
import os
from pathlib import Path
import re
import socket
import signal
import struct
import subprocess
import tempfile
import time
import tomllib
import zlib

ROOT = Path(__file__).resolve().parents[1]
SDK_VERSION_MATCH = re.search(
    r"^pub const SDK_VERSION: u32 = (\d+);$",
    (ROOT / 'springboard/proto/src/lib.rs').read_text(),
    re.MULTILINE,
)
if SDK_VERSION_MATCH is None:
    raise RuntimeError('could not read SDK_VERSION from springboard/proto/src/lib.rs')
SDK_VERSION = int(SDK_VERSION_MATCH.group(1))


SIZES = [(1000, 720), (900, 600)]
SCALE = 1.0
DARK = True


def configure(width, height):
    return struct.pack('<IIIfffBfB', 1, width, height, SCALE, 0.0, 0.0, 1 if DARK else 0, 1.0, 0)


def png(path, width, height, pixels):
    def chunk(kind, data):
        return (struct.pack('>I', len(data)) + kind + data
                + struct.pack('>I', zlib.crc32(kind + data)))
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        row = pixels[y * width * 4:(y + 1) * width * 4]
        for x in range(0, len(row), 4):
            rows.extend(row[x:x + 3])
    path.write_bytes(b'\x89PNG\r\n\x1a\n'
                     + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def smoke(app, output):
    manifest = tomllib.loads((app / 'manifest.toml').read_text())
    assert manifest['architecture'] == 'x86_64-linux'
    binary = ROOT / 'springboard/target/fast' / manifest['exec']
    with tempfile.TemporaryDirectory(prefix='bifrost-app-smoke-') as work:
        home = Path(work) / 'home'
        data = Path(work) / 'data'
        home.mkdir()
        data.mkdir()
        parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        # Deliberately do not inherit device or service access from the host.
        env = {'PATH': '/usr/bin', 'LANG': 'en_US.UTF-8', 'HOME': str(home),
               'BIFROST_DATA': str(data), 'BIFROST_SOCK': str(child.fileno()),
               'BIFROST_SHELL_DIR': str(ROOT / 'springboard/assets'),
               'BIFROST_SERVICE_ROOT': str(Path(work) / 'no-audio-service'),
               'SPRINGBOARD_PLATFORM': 'desktop', 'BIFROST_LAYOUT': 'desktop',
               'BIFROST_HARDWARE_KEYBOARD': '1'}
        maps = []
        mapped_size = None
        generation = 0
        hello = False
        sizes = []
        log = output / (app.name + '.log')
        with log.open('wb') as stderr:
            process = subprocess.Popen([binary], env=env, cwd=data, pass_fds=(child.fileno(),),
                                       stdin=subprocess.DEVNULL, stdout=stderr, stderr=stderr, start_new_session=True)
            child.close()
            try:
                for width, height in SIZES:
                    parent.send(configure(width, height))
                    deadline = time.monotonic() + 15
                    while True:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError(f'{app.name}: no {width}x{height} frame; see {log}')
                        parent.settimeout(remaining)
                        packet, ancillary, flags, _ = parent.recvmsg(16384, socket.CMSG_SPACE(8))
                        assert packet, f'{app.name}: process closed channel; see {log}'
                        assert not flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC)
                        descriptors = []
                        for level, kind, raw in ancillary:
                            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                                fds = array.array('i')
                                fds.frombytes(raw[:len(raw) - len(raw) % fds.itemsize])
                                descriptors.extend(fds)
                        tag, = struct.unpack_from('<I', packet)
                        try:
                            if tag == 0x101:
                                version, length = struct.unpack_from('<II', packet, 4)
                                identity = packet[12:12 + length].decode()
                                assert version == SDK_VERSION and identity == manifest['id'], (version, identity, manifest['id'])
                                hello = True
                            elif tag == 0x102:
                                actual = struct.unpack_from('<II', packet, 4)
                                generation, = struct.unpack_from('<I', packet, 12)
                                assert actual == (width, height) and len(descriptors) == 2
                                mapped_size = actual
                                for old in maps:
                                    old.close()
                                maps = [mmap.mmap(fd, width * height * 4, access=mmap.ACCESS_READ)
                                        for fd in descriptors]
                            elif tag == 0x103:
                                index, = struct.unpack_from('<I', packet, 4)
                                assert hello and len(maps) == 2 and index < 2
                                if mapped_size != (width, height):
                                    parent.send(struct.pack('<III', 4, index, generation))
                                    continue
                                pixels = maps[index][:]
                                assert len(set(pixels[i:i + 4] for i in range(0, len(pixels), 4))) > 1
                                png(output / f"{app.name}-{width}x{height}{'' if DARK else '-light'}.png", width, height, pixels)
                                sizes.append([width, height])
                                parent.send(struct.pack('<III', 4, index, generation))
                                break
                        finally:
                            for fd in descriptors:
                                os.close(fd)
                parent.send(struct.pack('<I', 6))  # save state on Suspend
                parent.send(struct.pack('<I', 5))  # Resume
                parent.send(struct.pack('<I', 7))  # Quit
                code = process.wait(timeout=5)
                assert code == 0, f'{app.name}: exit {code}; see {log}'
            finally:
                parent.close()
                for mapping in maps:
                    mapping.close()
                # Content workers can outlive the app itself. Each preview owns its process group.
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    pass
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
    return {'app': app.name, 'id': manifest['id'], 'version': manifest['version'], 'frames': sizes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('apps', nargs='*')
    parser.add_argument('--out', type=Path, default=ROOT / 'out/pc-app-smoke')
    parser.add_argument('--sizes', default='1000x720,900x600', help='comma list of WxH in pixels, e.g. 1200x769,640x480')
    parser.add_argument('--scale', type=float, default=1.0, help='pixels per point (Midget is 2.0)')
    parser.add_argument('--light', action='store_true', help='start the app in the Light scheme (Configure::dark = false); PNGs end in -light')
    args = parser.parse_args()
    global SIZES, SCALE, DARK
    DARK = not args.light
    SIZES = [tuple(int(v) for v in s.split('x')) for s in args.sizes.split(',')]
    SCALE = args.scale
    args.out.mkdir(parents=True, exist_ok=True)
    apps = args.apps or [
        p.name for p in sorted((ROOT / 'springboard/apps').glob('*-desktop'))
        if (p / 'manifest.toml').exists() and (p / 'src/main.rs').exists()
        and not (p / 'upstream.toml').exists()
    ]  # only manifest-backed SDK apps with a binary target use the frame protocol
    results = []
    for name in apps:
        assert Path(name).name == name
        result = smoke(ROOT / 'springboard/apps' / name, args.out)
        results.append(result)
        (args.out / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
        print(f"PASS {name}: identity, first frame, resize, suspend/resume, exit", flush=True)


if __name__ == '__main__':
    main()
