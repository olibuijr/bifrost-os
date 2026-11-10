#!/usr/bin/env python3
"""Exercise real Servo embedder contracts against an isolated local fixture server.

Usage: python tests/engine_contract.py /path/to/bifrost-webview
No public sites, existing profiles, desktop input, or installed OS services are used.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import selectors
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib


def unescape(value):
    out = []
    iterator = iter(value)
    for char in iterator:
        if char != '\\':
            out.append(char)
        else:
            char = next(iterator, '\\')
            out.append({'n': '\n', 'r': '\r', '\\': '\\'}.get(char, '\\' + char))
    return ''.join(out)


def icon():
    def chunk(kind, value):
        return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(b'\x00\xff\x00\x00\xff')) + chunk(b'IEND', b''))


def pixels(png):
    """Rows of (r, g, b, a) from an 8-bit RGBA, non-interlaced PNG (what the engine's `dev shot` writes)."""
    position, data = 8, b''
    while position < len(png):
        length, kind = struct.unpack('>I4s', png[position:position + 8])
        if kind == b'IHDR':
            width, height, depth, colour, _, _, interlace = struct.unpack('>IIBBBBB', png[position + 8:position + 21])
            assert (depth, colour, interlace) == (8, 6, 0), (depth, colour, interlace)
        elif kind == b'IDAT':
            data += png[position + 8:position + 8 + length]
        position += length + 12
    raw, stride, rows, previous = zlib.decompress(data), width * 4, [], bytes(width * 4)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            left, up, corner = line[x - 4] if x >= 4 else 0, previous[x], previous[x - 4] if x >= 4 else 0
            if kind == 1:
                line[x] = (line[x] + left) & 255
            elif kind == 2:
                line[x] = (line[x] + up) & 255
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 255
            elif kind == 4:
                estimate = left + up - corner
                nearest = min((abs(estimate - left), 0, left), (abs(estimate - up), 1, up), (abs(estimate - corner), 2, corner))[2]
                line[x] = (line[x] + nearest) & 255
        rows.append([tuple(line[x:x + 4]) for x in range(0, stride, 4)])
        previous = bytes(line)
    return rows


# Colour scheme fixtures: short pages with no colours of their own, except where the name says so.
SCHEME_PAGES = {
    '/scheme/none': '',
    '/scheme/css-dark': '<style>:root{color-scheme:dark}</style>',
    '/scheme/css-both': '<style>:root{color-scheme:light dark}</style>',
    '/scheme/meta-dark': '<meta name="color-scheme" content="dark">',
    '/scheme/meta-both': '<meta name="color-scheme" content="light dark">',
    '/scheme/meta-dark-css-light': '<meta name="color-scheme" content="dark"><style>:root{color-scheme:light}</style>',
    '/scheme/css-dark-body-background': '<style>:root{color-scheme:dark}body{background:rgb(0,128,0)}</style>',
}
# Default text and canvas of a scheme: stylo 0.22 device/servo.rs `system_color` (CanvasText, Canvas); the light
# canvas is Servo's white shell background.
LIGHT = ('rgb(0, 0, 0)', (255, 255, 255))
DARK = ('rgb(232, 232, 232)', (30, 30, 30))


REQUESTS = []
class Fixture(BaseHTTPRequestHandler):
    def do_GET(self):
        REQUESTS.append((self.path, self.headers.get('User-Agent')))
        self.send_response(200)
        if self.path == '/favicon.png':
            body = icon()
            self.send_header('Content-Type', 'image/png')
        elif self.path in SCHEME_PAGES:
            body = ('<!doctype html><title>Colour scheme fixture</title>' + SCHEME_PAGES[self.path] + '<p>Words</p>').encode()
            self.send_header('Content-Type', 'text/html')
        else:
            body = b'''<!doctype html><title>Native embedder fixture</title>
<link rel="icon" href="/favicon.png"><style>body{margin:0}p{font:24px sans-serif;margin:0;height:40px}#space{height:2000px}</style>
<p id="words">Hello native world. Hello native world.</p><input id="editor" value="start"><div id="space"></div>
<script>window.inputEvents=[];for(const name of ['mousedown','mouseup','mousemove','touchstart'])document.addEventListener(name,()=>inputEvents.push(name));</script>'''
            self.send_header('Content-Type', 'text/html')
            if self.path == '/first':
                self.send_header('Set-Cookie', 'secret=fixture; HttpOnly; Path=/')
                self.send_header('Set-Cookie', 'visible=fixture; Path=/')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass


class Engine:
    def __init__(self, binary, profile):
        self.fd = os.memfd_create('bifrost-engine-contract')
        os.ftruncate(self.fd, 800 * 600 * 4)
        self.log = open(Path(profile) / 'engine.log', 'w')
        self.process = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
            start_new_session=True, pass_fds=(self.fd,), env=dict(os.environ, BIFROST_WEBVIEW_FD=str(self.fd), BIFROST_WEBVIEW_PROFILE=profile))
        os.set_blocking(self.process.stdout.fileno(), False)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b''
        self.lines = []
        self.token = 100
    def send(self, line):
        self.process.stdin.write((line + '\n').encode())
        self.process.stdin.flush()
    def wait(self, predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for index, line in enumerate(self.lines):
                if predicate(line):
                    return self.lines.pop(index)
            for key, events in self.selector.select(timeout=.1):
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError('engine exited: ' + str(self.log.name))
                self.buffer += data
                while b'\n' in self.buffer:
                    raw, self.buffer = self.buffer.split(b'\n', 1)
                    line = raw.decode(errors='replace')
                    if line.startswith('frame '):
                        self.send('ack')
                    elif line.startswith(('fatal ', 'crashed ')):
                        raise RuntimeError(line)
                    self.lines.append(line)
        raise TimeoutError('engine response timed out; tail: ' + repr(self.lines[-8:]))
    def evaluate(self, javascript):
        self.token += 1
        prefix = f'dev eval {self.token} '
        escaped = javascript.replace('\\', '\\\\').replace('\n', '\\n').replace('\r', '\\r')
        self.send(f'dev eval {self.token} 1 {escaped}')
        line = self.wait(lambda line: line.startswith(prefix))
        parts = line.split(' ', 5)
        if parts[3] != 'ok':
            raise AssertionError(line)
        return json.loads(unescape(parts[5]))
    def eventually(self, javascript, expected=True):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            value = self.evaluate(javascript)
            if value == expected:
                return value
            time.sleep(.05)
        raise AssertionError(f'{javascript}: expected {expected!r}, received {value!r}')
    def canvas(self, path):
        """The colour of the page canvas: a 4x4 screenshot of the viewport's lower right, below the short fixture body."""
        self.token += 1
        prefix = f'dev shot {self.token} '
        self.send(f'dev shot {self.token} 1 rect 700 500 4 4 1000000 {path}')
        line = self.wait(lambda line: line.startswith(prefix))
        if line.split(' ')[3] != 'ok':
            raise AssertionError(line)
        colours = {pixel[:3] for row in pixels(Path(path).read_bytes()) for pixel in row}
        return colours.pop() if len(colours) == 1 else colours
    def scheme(self, shot, expected, canvas=None):
        """Waits until the page's default text colour and its canvas are the ones of the expected scheme."""
        text, default_canvas = expected
        self.eventually('getComputedStyle(document.body).color', text)
        canvas, deadline = canvas or default_canvas, time.monotonic() + 10
        while (seen := self.canvas(shot)) != canvas:
            if time.monotonic() > deadline:
                raise AssertionError(f'canvas: expected {canvas!r}, received {seen!r}')
            time.sleep(.05)
    def navigate(self, url, first=False):
        self.lines = [line for line in self.lines if not line.startswith('load 1 ')]
        self.send(f'new 1 {url}' if first else f'load 1 {url}')
        if first:
            self.send('show 1')
        self.wait(lambda line: line == 'load 1 complete')
    def close(self):
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        self.process.wait(timeout=10)
        os.close(self.fd)
        self.selector.close()
        self.log.close()


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    results = []
    def passed(name):
        results.append(name)
        print('PASS ' + name, flush=True)
    with tempfile.TemporaryDirectory(prefix='bifrost-engine-contract-') as profile:
        engine = Engine(sys.argv[1], profile)
        try:
            engine.send('size 800 600 1')
            engine.navigate(base + '/first', first=True)
            engine.eventually('typeof IntersectionObserver === "function" && typeof ResizeObserver === "function" && CSS.supports("display","grid")')
            engine.eventually('IntersectionObserver.toString().includes("[native code]") && ResizeObserver.toString().includes("[native code]")')
            assert engine.evaluate('(function(){try{new IntersectionObserver(()=>{}, {threshold:2});return false}catch(e){return e.name === "RangeError"}})()')
            engine.evaluate('window.observerFired=false;window.observer=new IntersectionObserver(()=>observerFired=true);observer.observe(document.querySelector("#words"));true')
            engine.eventually('observerFired')
            passed('native observers and CSS Grid')
            favicon = engine.wait(lambda line: line.startswith('favicon 1 '))
            assert len(favicon.split(' ', 2)[2]) == 32 * 32 * 8
            passed('native favicon callback')
            engine.send('mouse d 2 12')
            engine.eventually('inputEvents.includes("mousedown")')
            engine.send('mouse m 180 12')
            engine.eventually('getSelection().toString().length > 0')
            engine.send('mouse u 180 12')
            engine.eventually('getSelection().toString().length > 0 && inputEvents.includes("mousedown") && !inputEvents.includes("touchstart")')
            engine.send('chord 1 c')
            copied = engine.wait(lambda line: line.startswith('clipboard '))
            assert len(unescape(copied[len('clipboard '):])) > 0
            passed('native mouse selection and clipboard copy')
            engine.send('wheel 0 -200 200 200')
            engine.eventually('scrollY > 0')
            engine.evaluate('window.wheelSeen=false;window.preventWheel=event=>{wheelSeen=true;event.preventDefault()};document.addEventListener("wheel",preventWheel,{passive:false});window.scrollBeforeWheel=scrollY;true')
            engine.send('wheel 0 -200 200 200')
            engine.eventually('wheelSeen')
            engine.eventually('scrollY === scrollBeforeWheel')
            engine.evaluate('document.removeEventListener("wheel",preventWheel);true')
            passed('native wheel scrolling and preventDefault')
            engine.send('find 1 n native')
            assert engine.wait(lambda line: line.startswith('found 1 ')) == 'found 1 2'
            engine.eventually('getSelection().toString() === "native"')
            passed('DOM Range and Selection find')
            assert 'secret=' not in engine.evaluate('document.cookie')
            assert 'secret=' not in engine.evaluate("'bifrost-ck:' + document.cookie")
            engine.send('cookies 1 41')
            privileged = engine.wait(lambda line: line.startswith('cookies 1 41 '))
            assert 'secret' in privileged
            passed('HttpOnly stays hidden from JavaScript; native cookie API reads it')
            engine.send('ua NativeEngineTest/1.0')
            engine.navigate(base + '/second')
            assert (('/second', 'NativeEngineTest/1.0')) in REQUESTS
            engine.eventually('navigator.userAgent', 'NativeEngineTest/1.0')
            passed('native user-agent preference updates requests and navigator')
            engine.send('clear-history 1')
            engine.wait(lambda line: line == 'history 1 0 0')
            assert engine.evaluate('location.pathname') == '/second'
            passed('native history clearing keeps the current document')
            engine.evaluate('localStorage.setItem("local","value");sessionStorage.setItem("session","value");true')
            engine.send('clear-site-data')
            engine.eventually('localStorage.getItem("local") === null && sessionStorage.getItem("session") === null && document.cookie === ""')
            passed('native cookie and Web Storage clearing')
            engine.send('theme dark')
            engine.eventually('matchMedia("(prefers-color-scheme: dark)").matches')
            engine.eventually('getComputedStyle(document.body).color', 'rgb(0, 0, 0)')
            engine.evaluate('document.documentElement.setAttribute("style","color-scheme:dark");true')
            engine.eventually('getComputedStyle(document.body).color !== "rgb(0, 0, 0)"')
            engine.evaluate('document.documentElement.removeAttribute("style");true')
            engine.eventually('getComputedStyle(document.body).color', 'rgb(0, 0, 0)')
            engine.send('theme light')
            engine.eventually('matchMedia("(prefers-color-scheme: light)").matches')
            passed('a page without a colour scheme keeps dark default text in the dark appearance; a declared one wins')
            shot = str(Path(profile) / 'canvas.png')
            for appearance in ('dark', 'light'):
                engine.send('theme ' + appearance)
                engine.navigate(base + '/scheme/none')
                engine.eventually(f'matchMedia("(prefers-color-scheme: {appearance})").matches')
                engine.scheme(shot, LIGHT)
            passed('no colour scheme declared: dark text on a white canvas in both appearances')
            for page in ('/scheme/css-dark', '/scheme/meta-dark'):
                for appearance in ('dark', 'light'):
                    engine.send('theme ' + appearance)
                    engine.navigate(base + page)
                    engine.scheme(shot, DARK)
            passed('a dark colour scheme, from CSS or from the meta element alone: light text on a dark canvas in both appearances')
            for page in ('/scheme/css-both', '/scheme/meta-both'):
                engine.send('theme dark')
                engine.navigate(base + page)
                engine.scheme(shot, DARK)
                engine.send('theme light')
                engine.scheme(shot, LIGHT)
                engine.send('theme dark')
                engine.scheme(shot, DARK)
            passed('light and dark both supported, from CSS or from the meta element alone: the page follows the appearance, live')
            engine.navigate(base + '/scheme/meta-dark-css-light')
            engine.scheme(shot, LIGHT)
            engine.evaluate('document.querySelector("style").remove();true')
            engine.scheme(shot, DARK)
            engine.evaluate('document.querySelector("meta[name=color-scheme]").remove();true')
            engine.scheme(shot, LIGHT)
            passed('the CSS colour scheme of a page wins over its meta element; removing the meta element returns to light')
            engine.navigate(base + '/scheme/css-dark-body-background')
            engine.scheme(shot, DARK, canvas=(0, 128, 0))
            engine.send('theme light')
            engine.eventually('matchMedia("(prefers-color-scheme: light)").matches')
            passed('a background on the body still reaches the canvas of a dark page')
            engine.send('suspend')
            engine.send('ping 501')
            engine.wait(lambda line: line == 'pong 501')
            engine.lines = [line for line in engine.lines if not line.startswith('frame ')]
            engine.evaluate('document.body.style.backgroundColor="red";true')
            try:
                engine.wait(lambda line: line.startswith('frame '), timeout=.3)
            except TimeoutError:
                pass
            else:
                raise AssertionError('a suspended engine presented a frame')
            engine.send('resume')
            engine.wait(lambda line: line.startswith('frame '))
            passed('suspended presentation stops frames and resume presents again')
        except Exception:
            engine.log.flush()
            print('Engine stderr:\n' + (Path(profile) / 'engine.log').read_text(errors='replace')[-8000:], file=sys.stderr)
            raise
        finally:
            engine.close()
            server.shutdown()
    print(json.dumps({'passed': len(results), 'checks': results}), flush=True)

if __name__ == '__main__':
    main()
