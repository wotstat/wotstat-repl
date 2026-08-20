"""Incrementally tail the game's python.log from the agent background thread."""

import os
import re
import time


_STRUCTURED_LINE = re.compile(
    r'^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+):\s+'
    r'([A-Z]+):\s+(.*)$')
_SOURCE_PAYLOAD = re.compile(r'^([^:\r\n]+):\s+(.*)$')
_LESTA_EXECUTABLE = 'tanki.exe'
_GAME_START_BLOCK = re.compile(
    br'(?m)^/-{8,}\\[ \t]*\r?\n'
    br'[^\r\n]*\bstarting on\s+'
    br'(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun) '
    br'(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+'
    br'(\d{1,2}) (\d{2}):(\d{2}):(\d{2}) (\d{4})\r?$')
_MONTHS = {
    'Jan': 1, 'Feb': 2, 'Mar': 3, 'Apr': 4,
    'May': 5, 'Jun': 6, 'Jul': 7, 'Aug': 8,
    'Sep': 9, 'Oct': 10, 'Nov': 11, 'Dec': 12,
}
_STARTUP_WINDOW_SECONDS = 3 * 60
_STARTUP_SCAN_BYTES = 8 * 1024 * 1024
_STARTUP_SCAN_CHUNK = 64 * 1024
_HEADER_OVERLAP_BYTES = 1024


def _decode(raw):
    if not isinstance(raw, bytes):
        return raw
    return raw.decode('utf-8', 'replace')


def _has_source_field(path):
    """Lesta omits the source column which is present in WG python.log."""
    try:
        names = set(name.lower() for name in os.listdir(
            os.path.dirname(os.path.abspath(path))))
    except (IOError, OSError):
        return True
    return _LESTA_EXECUTABLE not in names


def _file_identity(stat):
    inode = getattr(stat, 'st_ino', 0)
    if inode:
        return (getattr(stat, 'st_dev', 0), inode)
    return (getattr(stat, 'st_ctime', 0),)


def _game_start_time(match):
    try:
        month = match.group(1).decode('ascii')
        return time.mktime((
            int(match.group(6)), _MONTHS[month], int(match.group(2)),
            int(match.group(3)), int(match.group(4)), int(match.group(5)),
            0, 0, -1,
        ))
    except (KeyError, TypeError, ValueError, OverflowError):
        return None


def _recent_game_start(handle, size, started_at):
    """Find the last game banner near mod startup without reading a huge log."""
    lower_bound = max(0, size - _STARTUP_SCAN_BYTES)
    end = size
    overlap = b''
    while end > lower_bound:
        start = max(lower_bound, end - _STARTUP_SCAN_CHUNK)
        handle.seek(start)
        chunk = handle.read(end - start)
        data = chunk + overlap
        matches = list(_GAME_START_BLOCK.finditer(data))
        if matches:
            match = matches[-1]
            marker_time = _game_start_time(match)
            if marker_time is None:
                return None
            age = started_at - marker_time
            if -5 <= age <= _STARTUP_WINDOW_SECONDS:
                return start + match.start()
            return None
        overlap = data[:_HEADER_OVERLAP_BYTES]
        end = start
    return None


def _frame(raw_line, has_source=True):
    text = _decode(raw_line.rstrip(b'\r'))
    if text.startswith(u'\ufeff'):
        text = text[1:]
    match = _STRUCTURED_LINE.match(text)
    if match is None:
        return {
            'type': 'stdout',
            'stream': 'python_log',
            'text': text + '\n',
        }
    source = None
    payload = match.group(3)
    if has_source:
        source_match = _SOURCE_PAYLOAD.match(payload)
        if source_match is not None:
            source = source_match.group(1)
            payload = source_match.group(2)
    return {
        'type': 'stdout',
        'stream': 'python_log',
        'timestamp': match.group(1),
        'level': match.group(2),
        'source': source,
        'text': payload + '\n',
    }


class PythonLogTail(object):
    """Read completed lines appended after construction, including rotation."""

    def __init__(self, path, interval=0.25, read_size=256 * 1024):
        self._path = path
        self._interval = max(0, interval)
        self._read_size = max(1, read_size)
        self._has_source = _has_source_field(path)
        self._next_poll_at = 0
        self._identity = None
        self._offset = 0
        self._pending = b''
        self._skip_until_newline = False
        self._snapshot_existing_file()

    def _snapshot_existing_file(self):
        """Start at the current game banner, or EOF when none is recent."""
        try:
            stat = os.stat(self._path)
        except (IOError, OSError):
            return

        self._identity = _file_identity(stat)
        self._offset = stat.st_size
        if not stat.st_size:
            return

        try:
            handle = open(self._path, 'rb')
            try:
                game_start = _recent_game_start(
                    handle, stat.st_size, time.time())
                if game_start is not None:
                    self._offset = game_start
                    return
                handle.seek(stat.st_size - 1)
                self._skip_until_newline = handle.read(1) not in (b'\n', b'\r')
            finally:
                handle.close()
        except (IOError, OSError):
            # Avoid emitting an arbitrary tail fragment if the last-byte probe
            # races with the game replacing the file.
            self._skip_until_newline = True

    def poll(self, now=None):
        current = time.time() if now is None else now
        if current < self._next_poll_at:
            return []
        self._next_poll_at = current + self._interval

        try:
            stat = os.stat(self._path)
        except (IOError, OSError):
            return []

        identity = _file_identity(stat)
        if self._identity is None:
            self._identity = identity
        elif identity != self._identity or stat.st_size < self._offset:
            self._identity = identity
            self._offset = 0
            self._pending = b''
            self._skip_until_newline = False

        try:
            handle = open(self._path, 'rb')
            try:
                handle.seek(self._offset)
                chunk = handle.read(self._read_size)
            finally:
                handle.close()
        except (IOError, OSError):
            return []

        if not chunk:
            return []
        self._offset += len(chunk)
        data = self._pending + chunk
        if self._skip_until_newline:
            newline = data.find(b'\n')
            if newline < 0:
                self._pending = b''
                return []
            data = data[newline + 1:]
            self._skip_until_newline = False

        parts = data.split(b'\n')
        self._pending = parts.pop()
        return [_frame(part, self._has_source) for part in parts]
