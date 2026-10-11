#!/usr/bin/env python3
"""Measure Midget's WayVNC frame source: RFB 3.8, no auth, 32 bpp true colour.

usage: tools/midget-vnc-probe.py [HOST] [PORT] [SECONDS] [--encoding raw|zrle] [--full]
  --full       full frame every request (FramebufferUpdateRequest incremental=0); default waits for damage
  --encoding   what to ask WayVNC for; zrle is checked pixel-for-pixel against raw before timing
It is the source the wowstream encoder will read from; it does not encode anything.
"""
import socket, struct, sys, time, zlib

import numpy as np

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
HOST = ARGS[0] if len(ARGS) > 0 else "192.168.20.140"
PORT = int(ARGS[1]) if len(ARGS) > 1 else 5900
SECONDS = float(ARGS[2]) if len(ARGS) > 2 else 5.0
FULL = 0 if "--full" in sys.argv else 1
ENC = sys.argv[sys.argv.index("--encoding") + 1] if "--encoding" in sys.argv else "raw"
ENC_CODE = {"raw": 0, "zrle": 16}[ENC]


def recv_exact(s, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("closed")
        buf += chunk
    return bytes(buf)


def read_runlen(data, i):
    """ZRLE run length: 1 + sum of bytes, continuing while a byte is 255."""
    n = 1
    while True:
        b = data[i]
        i += 1
        n += b
        if b != 255:
            return n, i


def decode_zrle(data, w, h):
    """One ZRLE rectangle -> (h, w, 3) uint8 of colour bytes (B, G, R) per pixel."""
    out = np.zeros((h, w, 3), np.uint8)
    i = 0
    for ty in range(0, h, 64):
        for tx in range(0, w, 64):
            tw, th = min(64, w - tx), min(64, h - ty)
            sub = data[i]
            i += 1
            if sub == 0:  # raw tile
                out[ty:ty + th, tx:tx + tw] = np.frombuffer(data, np.uint8, tw * th * 3, i).reshape(th, tw, 3)
                i += tw * th * 3
            elif sub == 1:  # solid
                out[ty:ty + th, tx:tx + tw] = np.frombuffer(data, np.uint8, 3, i)
                i += 3
            elif 2 <= sub <= 16:  # packed palette
                pal = np.frombuffer(data, np.uint8, sub * 3, i).reshape(sub, 3)
                i += sub * 3
                bits = 1 if sub == 2 else 2 if sub <= 4 else 4
                per_row = (tw * bits + 7) // 8
                shifts = np.arange(8 - bits, -1, -bits, dtype=np.uint8)
                for r in range(th):
                    row = np.frombuffer(data, np.uint8, per_row, i)
                    i += per_row
                    idx = ((row[:, None] >> shifts[None, :]) & ((1 << bits) - 1)).reshape(-1)[:tw]
                    out[ty + r, tx:tx + tw] = pal[idx]
            elif sub == 128:  # plain RLE
                pos, total = 0, tw * th
                while pos < total:
                    px = np.frombuffer(data, np.uint8, 3, i)
                    i += 3
                    n, i = read_runlen(data, i)
                    for k in range(pos, min(pos + n, total)):
                        out[ty + k // tw, tx + k % tw] = px
                    pos += n
            elif 130 <= sub <= 255:  # palette RLE
                size = sub - 128
                pal = np.frombuffer(data, np.uint8, size * 3, i).reshape(size, 3)
                i += size * 3
                pos, total = 0, tw * th
                while pos < total:
                    b = data[i]
                    i += 1
                    n = 1
                    if b & 0x80:
                        n, i = read_runlen(data, i)
                    for k in range(pos, min(pos + n, total)):
                        out[ty + k // tw, tx + k % tw] = pal[b & 0x7F]
                    pos += n
            else:
                raise ValueError(f"ZRLE subencoding {sub} not supported")
    return out


def connect():
    s = socket.create_connection((HOST, PORT), timeout=10)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    recv_exact(s, 12)
    s.sendall(b"RFB 003.008\n")
    types = recv_exact(s, recv_exact(s, 1)[0])
    if 1 not in types:
        raise SystemExit(f"no 'None' security type: {list(types)}")
    s.sendall(bytes([1]))
    if struct.unpack(">I", recv_exact(s, 4))[0] != 0:
        raise SystemExit("security failed")
    s.sendall(bytes([1]))  # ClientInit: shared
    w, h = struct.unpack(">HH", recv_exact(s, 4))
    recv_exact(s, 16)  # server pixel format; we set our own below
    recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0])  # desktop name
    # SetPixelFormat: 32 bpp, depth 24, little-endian, true colour, R 16, G 8, B 0
    s.sendall(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
    return s, w, h


def grab(s, w, h, enc, full):
    """Request one update; returns (wire_seconds, bytes, [(x, y, rw, rh, pixels)]) with pixels (h, w, 3) B,G,R."""
    s.sendall(struct.pack(">BxH", 2, 1) + struct.pack(">i", enc))
    t0 = time.time()
    s.sendall(struct.pack(">BBHHHH", 3, 0 if full else 1, 0, 0, w, h))
    if recv_exact(s, 1)[0] != 0:
        raise SystemExit("unexpected server message")
    recv_exact(s, 1)
    nrects = struct.unpack(">H", recv_exact(s, 2))[0]
    size, rects = 0, []
    zdec = grab.zdec
    for _ in range(nrects):
        x, y, rw, rh, e = struct.unpack(">HHHHi", recv_exact(s, 12))
        if e == 0:
            px = np.frombuffer(recv_exact(s, rw * rh * 4), np.uint8).reshape(rh, rw, 4)[:, :, :3]
            size += rw * rh * 4
        elif e == 16:
            n = struct.unpack(">I", recv_exact(s, 4))[0]
            blob = recv_exact(s, n)
            size += n
            px = decode_zrle(zdec.decompress(blob), rw, rh)
        else:
            raise SystemExit(f"encoding {e} not supported")
        rects.append((x, y, rw, rh, px))
    return time.time() - t0, size, rects


grab.zdec = zlib.decompressobj()


def main():
    s, w, h = connect()
    print(f"WayVNC {w}x{h}, encoding={ENC}, {'full' if FULL == 0 else 'incremental'} updates")
    if ENC == "zrle":
        # Check: raw, ZRLE, raw. Pixels that did not change across the three grabs must decode identically.
        _, _, a = grab(s, w, h, 0, True)
        _, _, z = grab(s, w, h, 16, True)
        _, _, b = grab(s, w, h, 0, True)
        still = (a[0][4] == b[0][4]).all(axis=2)
        bad = int((still & (z[0][4] != a[0][4]).any(axis=2)).sum())
        cover = still.mean()
        ok = bad == 0 and cover > 0.9 and len(a) == len(z) == 1
        print(f"ZRLE check: {bad} mismatching still pixels, {cover:.1%} of the screen still" + ("" if ok else " -> not timing it"))
        if not ok:
            raise SystemExit(1)
    end = time.time() + SECONDS
    times, sizes, frames = [], [], 0
    while time.time() < end:
        t, size, _ = grab(s, w, h, ENC_CODE, FULL == 0)
        times.append(t)
        sizes.append(size)
        frames += 1
    s.close()
    if not times:
        raise SystemExit("no frames")
    times.sort()
    print(f"{frames} updates in {SECONDS:.1f}s, {sum(sizes) / frames / 1e6:.2f} MB/update on the wire")
    print(f"update time ms: median {times[len(times) // 2] * 1000:.1f}, p90 {times[int(len(times) * 0.9)] * 1000:.1f}")


if __name__ == "__main__":
    main()
