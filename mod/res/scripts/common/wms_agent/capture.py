"""Capture sys.stdout / sys.stderr and the BigWorld.log* family.

Output is mirrored to the original streams (so python.log keeps working) and
pushed to a fast sink (a deque append) that the agent loop drains and ships as
stdout frames. The sink must not block the game thread, hence no file I/O here.
"""

import sys

_BW_LOG_FUNCS = (
    'logTrace', 'logDebug', 'logInfo', 'logNotice',
    'logWarning', 'logError', 'logCritical', 'logHack',
)


class _StreamProxy(object):
    def __init__(self, original, sink, stream_name):
        self._original = original
        self._sink = sink
        self._stream = stream_name

    def write(self, text):
        if self._original is not None:
            try:
                self._original.write(text)
            except Exception:
                pass
        if text:
            self._sink({'type': 'stdout', 'stream': self._stream, 'text': text})

    def flush(self):
        if self._original is not None:
            try:
                self._original.flush()
            except Exception:
                pass

    def __getattr__(self, name):
        return getattr(self._original, name)


class Capture(object):
    def __init__(self, sink):
        self._sink = sink
        self._saved_out = None
        self._saved_err = None
        self._saved_bw = {}
        self._bw = None

    def install(self):
        self._saved_out = sys.stdout
        self._saved_err = sys.stderr
        sys.stdout = _StreamProxy(self._saved_out, self._sink, 'stdout')
        sys.stderr = _StreamProxy(self._saved_err, self._sink, 'stderr')
        try:
            import BigWorld
            self._bw = BigWorld
        except ImportError:
            self._bw = None
        if self._bw is not None:
            for name in _BW_LOG_FUNCS:
                original = getattr(self._bw, name, None)
                if original is None:
                    continue
                self._saved_bw[name] = original
                setattr(self._bw, name, self._make_hook(name, original))

    def _make_hook(self, level, original):
        sink = self._sink

        def hook(prefix, msg, *args):
            try:
                original(prefix, msg, *args)
            finally:
                try:
                    sink({'type': 'stdout', 'stream': 'log', 'level': level,
                          'text': '%s%s\n' % (prefix, msg)})
                except Exception:
                    pass
        return hook

    def uninstall(self):
        if self._saved_out is not None:
            sys.stdout = self._saved_out
        if self._saved_err is not None:
            sys.stderr = self._saved_err
        if self._bw is not None:
            for name, original in self._saved_bw.items():
                setattr(self._bw, name, original)
        self._saved_bw = {}
