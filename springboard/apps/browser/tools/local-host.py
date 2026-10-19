#!/usr/bin/env python3
"""Local desktop host for the browser's existing SDK protocol."""
import array
import mmap
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import time

import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gtk, Gdk, GLib, Gio
from file_portal import FilePortal

ROOT = Path(os.environ.get('BIFROST_BROWSER_REPO', Path(__file__).resolve().parents[4]))
WORK = Path(os.environ.get('BIFROST_BROWSER_WORK', Path.home() / 'Work/bifrost-browser'))

class Host(Gtk.Application):
    def __init__(self):
        super().__init__(application_id='is.bifrost.browser.local', flags=Gio.ApplicationFlags.NON_UNIQUE if '--private' in sys.argv else Gio.ApplicationFlags.DEFAULT_FLAGS)
        WORK.mkdir(parents=True, exist_ok=True)
        self.maps = []
        self.size = (0, 0)
        self.generation = 0
        self.pointer = (0., 0.)
        self.down = False
        self.pending_texture = None
        self.connect('activate', self.activate)

    def send(self, fmt, *args):
        try:
            self.sock.send(struct.pack('<' + fmt, *args))
        except (BlockingIOError, BrokenPipeError, OSError):
            pass

    def activate(self, app):
        self.win = Gtk.ApplicationWindow(application=self, title='BifrOSt Browser')
        self.win.set_default_size(1200, 800)
        self.picture = Gtk.Picture()
        self.picture.set_can_shrink(True)
        self.picture.set_hexpand(True)
        self.picture.set_vexpand(True)
        self.win.set_child(self.picture)
        self.sock, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        self.file_portal = FilePortal(self.win, self.sock)
        data = WORK / 'data'
        data.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, BIFROST_SOCK=str(child.fileno()), BIFROST_DATA=str(data),
                   BIFROST_SHELL_DIR=str(ROOT / 'springboard/assets'),
                   BIFROST_CONTROL=os.environ.get('BIFROST_CONTROL', str(WORK / 'browser.sock')),
                   BIFROST_BROWSER_LAUNCHER=str(Path(__file__).resolve().with_name('run-local')),
                   SPRINGBOARD_PLATFORM='desktop', BIFROST_LAYOUT='desktop',
                   BIFROST_HARDWARE_KEYBOARD='1')
        self.log = (WORK / 'browser.log').open('ab')
        binary = Path(os.environ.get('BIFROST_LOCAL_BINARY', ROOT / 'springboard/target/fast/bifrost-browser'))
        self.proc = subprocess.Popen([binary, *sys.argv[1:]],
                                     env=env, cwd=data, pass_fds=(child.fileno(),),
                                     stdout=self.log, stderr=self.log)
        child.close()
        self.sock.setblocking(False)
        GLib.io_add_watch(self.sock.fileno(), GLib.IO_IN | GLib.IO_HUP, self.receive)
        self.picture.add_tick_callback(self.resize)
        click = Gtk.GestureClick()
        click.set_button(0)
        click.connect('pressed', self.press)
        click.connect('released', self.release)
        self.picture.add_controller(click)
        motion = Gtk.EventControllerMotion()
        motion.connect('motion', self.motion)
        self.picture.add_controller(motion)
        scroll = Gtk.EventControllerScroll(flags=Gtk.EventControllerScrollFlags.BOTH_AXES)
        scroll.connect('scroll', self.scroll)
        self.picture.add_controller(scroll)
        keys = Gtk.EventControllerKey()
        keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        keys.connect('key-pressed', self.key)
        self.win.add_controller(keys)
        self.win.connect('close-request', self.close)
        self.win.present()

    def resize(self, widget, clock):
        # Present the newest complete texture on GTK's display clock.
        if self.pending_texture is not None:
            self.picture.set_paintable(self.pending_texture)
            self.pending_texture = None
        size = (widget.get_width(), widget.get_height())
        if min(size) > 0 and size != self.size:
            self.size = size
            self.send('IIIfffBfB', 1, *size, 1., 0., 0., 1, 1., 0)
        return True

    def receive(self, fd, condition):
        if condition & GLib.IO_HUP:
            self.file_portal.close()
            self.quit()
            return False
        try:
            packet, ancillary, flags, addr = self.sock.recvmsg(16384, socket.CMSG_SPACE(32))
        except BlockingIOError:
            return True
        if not packet:
            self.file_portal.close()
            self.quit()
            return False
        fds = []
        for level, kind, raw in ancillary:
            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                values = array.array('i')
                values.frombytes(raw[:len(raw) - len(raw) % values.itemsize])
                fds.extend(values)
        try:
            tag, = struct.unpack_from('<I', packet)
            if tag == 0x102:
                self.width, self.height, self.generation = struct.unpack_from('<III', packet, 4)
                for old in self.maps:
                    old.close()
                self.maps = [mmap.mmap(f, self.width * self.height * 4, access=mmap.ACCESS_READ) for f in fds]
            elif tag == 0x103 and self.maps:
                index, = struct.unpack_from('<I', packet, 4)
                pixels = GLib.Bytes.new(self.maps[index][:])
                texture = Gdk.MemoryTexture.new(self.width, self.height, Gdk.MemoryFormat.R8G8B8A8,
                                               pixels, self.width * 4)
                self.pending_texture = texture
                self.send('III', 4, index, self.generation)
            elif tag == 0x109:
                length, = struct.unpack_from('<I', packet, 4)
                self.win.get_clipboard().set(packet[8:8 + length].decode())
            elif tag in (0x110, 0x119):
                self.file_portal.open(packet)
        finally:
            for descriptor in fds:
                os.close(descriptor)
        return True

    def touch(self, kind, x, y):
        self.send('IBIffI', 2, kind, 0, x, y, int(time.monotonic() * 1000) & 0xffffffff)

    def press(self, gesture, count, x, y):
        if gesture.get_current_button() == 3:
            self.send('Iff', 12, x, y)
        else:
            self.down = True
            self.touch(0, x, y)

    def release(self, gesture, count, x, y):
        if self.down:
            self.touch(2, x, y)
            self.down = False

    def motion(self, controller, x, y):
        self.pointer = (x, y)
        if self.down:
            self.touch(1, x, y)
        else:
            self.send('IBff', 0x1f1, 1, x, y)

    def scroll(self, controller, dx, dy):
        # Forward only actual input. No generated momentum or frames after the gesture ends.
        # Wheel conversion and touchpad sensitivity are independent, applied exactly once.
        scale = 40. if controller.get_unit() == Gdk.ScrollUnit.WHEEL else 2.
        self.send('IBffffBBB', 0x1f1, 0, *self.pointer, -dx * scale, -dy * scale, 1, 0, 0)
        return True

    def key(self, controller, keyval, keycode, state):
        mods = ((1 if state & Gdk.ModifierType.CONTROL_MASK else 0)
                | (2 if state & Gdk.ModifierType.SHIFT_MASK else 0)
                | (4 if state & Gdk.ModifierType.ALT_MASK else 0)
                | (8 if state & Gdk.ModifierType.SUPER_MASK else 0))
        codes = {Gdk.KEY_BackSpace: 8, Gdk.KEY_Tab: 9, Gdk.KEY_Return: 13, Gdk.KEY_Escape: 1,
                 Gdk.KEY_Home: 36, Gdk.KEY_Left: 37, Gdk.KEY_Up: 38, Gdk.KEY_Right: 39,
                 Gdk.KEY_Down: 40, Gdk.KEY_End: 35, Gdk.KEY_Page_Up: 33,
                 Gdk.KEY_Page_Down: 34, Gdk.KEY_Delete: 46, Gdk.KEY_F10: 121}
        if mods == 4 and keyval in (Gdk.KEY_f, Gdk.KEY_F):
            self.send('IBI', 3, 1, 121)
            return True
        if mods & 13:
            code = 0x10000 | mods << 8 | (keycode - 8) & 0xff
            if keyval in (Gdk.KEY_v, Gdk.KEY_V) and mods & 9:
                def pasted(clipboard, result):
                    try:
                        text = (clipboard.read_text_finish(result) or '').encode()[:12000]
                        self.sock.send(struct.pack('<II', 9, len(text)) + text)
                        self.send('IBI', 3, 1, code)
                    except GLib.Error:
                        pass
                self.win.get_clipboard().read_text_async(None, pasted)
            else:
                self.send('IBI', 3, 1, code)
        elif keyval in codes:
            self.send('IBI', 3, 1, codes[keyval])
        else:
            char = Gdk.keyval_to_unicode(keyval)
            if char:
                raw = chr(char).encode()
                self.sock.send(struct.pack('<IBI', 3, 0, len(raw)) + raw)
        return True

    def close(self, window):
        self.file_portal.close()
        self.send('I', 7)
        try:
            self.proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.proc.terminate()
        return False

Host().run([sys.argv[0]])
