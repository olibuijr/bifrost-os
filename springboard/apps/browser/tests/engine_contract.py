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


REQUESTS = []
class Fixture(BaseHTTPRequestHandler):
    def do_GET(self):
        REQUESTS.append((self.path, self.headers.get('User-Agent')))
        self.send_response(200)
        if self.path == '/favicon.png':
            body = icon()
            self.send_header('Content-Type', 'image/png')
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
