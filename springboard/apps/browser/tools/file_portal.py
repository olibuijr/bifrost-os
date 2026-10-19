"""The local host's native open panel, using the SDK's granted-file descriptor protocol."""
import array
from collections import deque
import os
import socket
import stat
import struct

from gi.repository import Gtk, Gio, GLib


def wire_string(text):
    raw = text.encode('utf-8')
    return struct.pack('<I', len(raw)) + raw


class FilePortal:
    def __init__(self, window, sock):
        self.window, self.sock = window, sock
        self.active = None
        self.queue = deque()
        self.write_watch = None
        self.closed = False

    def reply(self, packet, descriptor=None):
        if self.closed:
            if descriptor is not None:
                os.close(descriptor)
            return
        self.queue.append((packet, descriptor))
        if self.write_watch is None:
            self.write_watch = GLib.io_add_watch(self.sock.fileno(), GLib.IO_OUT, self.flush)

    def flush(self, fd, condition):
        while self.queue:
            packet, descriptor = self.queue[0]
            ancillary = [] if descriptor is None else [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', [descriptor]))]
            try:
                self.sock.sendmsg([packet], ancillary)
            except BlockingIOError:
                return True
            except OSError:
                self.close()
                return False
            self.queue.popleft()
            if descriptor is not None:
                os.close(descriptor)
        self.write_watch = None
        return False

    def error(self, request, code):
        self.reply(struct.pack('<IIB', 17, request, code))

    def open(self, packet):
        request, mode = struct.unpack_from('<IB', packet, 4)
        offset = 9

        def string():
            nonlocal offset
            length, = struct.unpack_from('<I', packet, offset)
            offset += 4
            if length > 4096 or offset + length > len(packet):
                raise ValueError('invalid open panel string')
            value = packet[offset:offset + length].decode('utf-8')
            offset += length
            return value

        try:
            title = string()
            count, = struct.unpack_from('<I', packet, offset)
            offset += 4
            if count > 16:
                raise ValueError('too many filters')
            filters = [string() for _ in range(count)]
            tag, = struct.unpack_from('<I', packet)
            options = struct.unpack_from('<I', packet, offset)[0] if tag == 0x119 else 4
            if self.active is not None or mode != 0 or options & 2:
                self.error(request, 5)
                return
        except (ValueError, UnicodeError, struct.error):
            self.error(request, 1)
            return

        dialog = Gtk.FileDialog(title=title or 'Choose files', modal=True, accept_label='Choose')
        if filters:
            accepted = Gtk.FileFilter(name='Accepted file types')
            for extension in filters:
                if extension and not any(char in extension for char in '/\\\x00'):
                    accepted.add_suffix(extension.lstrip('.'))
            all_files = Gtk.FileFilter(name='All files')
            all_files.add_pattern('*')
            choices = Gio.ListStore.new(Gtk.FileFilter)
            choices.append(accepted)
            choices.append(all_files)
            dialog.set_filters(choices)
        cancellable = Gio.Cancellable()
        self.active = cancellable
        multiple = bool(options & 1)

        def chosen(source, result):
            self.active = None
            descriptors = []
            try:
                if multiple:
                    model = source.open_multiple_finish(result)
                    files = [model.get_item(index) for index in range(model.get_n_items())]
                else:
                    files = [source.open_finish(result)]
                if not files or len(files) > 64:
                    self.error(request, 1)
                    return
                for file in files:
                    path = file.get_path()
                    if path is None:
                        raise OSError('only local files can be granted')
                    descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK)
                    descriptors.append(descriptor)
                    info = os.fstat(descriptor)
                    if not stat.S_ISREG(info.st_mode):
                        raise OSError('not a regular file')
                if multiple:
                    self.reply(struct.pack('<III', 20, request, len(files)))
                for file, descriptor in zip(files, descriptors):
                    packet = struct.pack('<II', 16, request) + os.urandom(16) + bytes([0])
                    packet += wire_string(file.get_basename()[:255]) + struct.pack('<Q', os.fstat(descriptor).st_size)
                    self.reply(packet, descriptor)
                descriptors.clear()  # reply queue owns descriptors until sendmsg succeeds
            except GLib.Error:
                self.error(request, 0)
            except (OSError, ValueError):
                self.error(request, 1)
            finally:
                for descriptor in descriptors:
                    os.close(descriptor)

        if multiple:
            dialog.open_multiple(self.window, cancellable, chosen)
        else:
            dialog.open(self.window, cancellable, chosen)

    def close(self):
        self.closed = True
        if self.active is not None:
            self.active.cancel()
            self.active = None
        if self.write_watch is not None:
            GLib.source_remove(self.write_watch)
            self.write_watch = None
        while self.queue:
            _, descriptor = self.queue.popleft()
            if descriptor is not None:
                os.close(descriptor)
