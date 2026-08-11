"""Cross-process advisory lock via exclusive file creation.

Works identically on Windows and POSIX, and must match the desktop (Rust) side:
    acquire = create <path> with O_CREAT | O_EXCL
    release = remove <path>

A lock older than STALE_SECONDS is force-broken so a peer that crashed
mid-write cannot deadlock the channel.
"""

import os
import time

STALE_SECONDS = 5.0


class FileLock(object):
    def __init__(self, path):
        self._path = path
        self._fd = None

    def acquire(self, timeout=2.0, poll=0.002):
        deadline = time.time() + timeout
        while True:
            try:
                self._fd = os.open(
                    self._path, os.O_CREAT | os.O_EXCL | os.O_WRONLY
                )
                return True
            except OSError:
                if self._is_stale():
                    self._break()
                    continue
                if time.time() >= deadline:
                    return False
                time.sleep(poll)

    def _is_stale(self):
        try:
            return (time.time() - os.path.getmtime(self._path)) > STALE_SECONDS
        except OSError:
            return False

    def _break(self):
        try:
            os.remove(self._path)
        except OSError:
            pass

    def release(self):
        if self._fd is not None:
            try:
                os.close(self._fd)
            except OSError:
                pass
            self._fd = None
        try:
            os.remove(self._path)
        except OSError:
            pass

    def __enter__(self):
        self.acquire()
        return self

    def __exit__(self, exc_type, exc_value, tb):
        self.release()
