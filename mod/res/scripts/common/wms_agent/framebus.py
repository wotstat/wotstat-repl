"""Newline-delimited JSON frame transport over two append files.

Shared directory layout (see docs/PROTOCOL.md):
    c2d         game -> desktop frames (one JSON object per line)
    d2c         desktop -> game frames
    c2d.lock    exclusive-create lock guarding c2d
    d2c.lock    exclusive-create lock guarding d2c

The agent writes c2d and reads d2c; the desktop does the mirror. Each side holds
the relevant lock for the whole read+truncate or append, so frames are never
torn or lost.
"""

import os
import json

from .lockfile import FileLock


class FrameBus(object):
    def __init__(self, directory, out_name='c2d', in_name='d2c'):
        self._out = os.path.join(directory, out_name)
        self._in = os.path.join(directory, in_name)
        self._out_lock = FileLock(self._out + '.lock')
        self._in_lock = FileLock(self._in + '.lock')

    # If the desktop is not draining, the outbound file would grow without bound
    # (it carries all captured game stdout). Cap it: past this size we drop the
    # backlog on the next write rather than accumulate forever. Sized to hold a
    # full client startup (~1.6MB) so early logs survive until the desktop attaches.
    MAX_OUT_BYTES = 8 << 20

    def send(self, frame):
        line = json.dumps(frame, ensure_ascii=True) + '\n'
        if not self._out_lock.acquire():
            return False
        try:
            mode = 'a'
            try:
                if os.path.getsize(self._out) > self.MAX_OUT_BYTES:
                    mode = 'w'
            except OSError:
                pass
            handle = open(self._out, mode)
            try:
                handle.write(line)
            finally:
                handle.close()
        finally:
            self._out_lock.release()
        return True

    def drain(self):
        if not os.path.exists(self._in):
            return []
        if not self._in_lock.acquire():
            return []
        try:
            handle = open(self._in, 'r')
            try:
                data = handle.read()
            finally:
                handle.close()
            open(self._in, 'w').close()
        finally:
            self._in_lock.release()
        frames = []
        for line in data.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                frames.append(json.loads(line))
            except ValueError:
                pass
        return frames
