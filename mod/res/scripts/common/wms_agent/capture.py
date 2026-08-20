"""Capture sys.stdout / sys.stderr and the BigWorld.log* family.

Output is mirrored to the original streams (so python.log keeps working) and
pushed to a fast sink (a deque append) that the agent loop drains and ships as
stdout frames. The sink must not block the game thread, hence no file I/O here.
"""

import datetime
import sys

_BW_LOG_FUNCS = (
    'logTrace', 'logDebug', 'logInfo', 'logNotice',
    'logWarning', 'logError', 'logCritical', 'logHack',
)


def _text(value):
    try:
        text_type = unicode
    except NameError:
        text_type = str
    if isinstance(value, text_type):
        return value
    if isinstance(value, bytes):
        return value.decode('utf-8', 'replace')
    return text_type(value)


def _format_bigworld_log(prefix, message, args):
    """Mirror the category formatting performed by BigWorld's log writer."""
    category = _text(prefix or '').strip()
    rendered = _text(message)
    # Extra positional values are BigWorld metadata, not printf arguments.
    # Python logging has already formatted ``message`` before it reaches here.
    if category:
        if not category.startswith('['):
            category = '[%s]' % category
        if not category[-1:].isspace():
            category += ' '
    return '%s%s\n' % (category, rendered)


def _contains_stream(root, target, depth=4, seen=None):
    """Return whether a Python stream wrapper already contains our proxy."""
    if root is target:
        return True
    if root is None or depth <= 0:
        return False
    if seen is None:
        seen = set()
    marker = id(root)
    if marker in seen:
        return False
    seen.add(marker)
    try:
        values = vars(root).values()
    except (TypeError, AttributeError):
        return False
    for value in values:
        if value is target:
            return True
        if isinstance(value, (bool, int, float, bytes)):
            continue
        try:
            if isinstance(value, basestring):
                continue
        except NameError:
            if isinstance(value, str):
                continue
        if _contains_stream(value, target, depth - 1, seen):
            return True
    return False


class _StreamProxy(object):
    def __init__(self, original, sink, stream_name, level, now):
        self._original = original
        self._sink = sink
        self._stream = stream_name
        self._level = level
        self._now = now
        self._pending = u''
        self._pending_timestamp = None

    def set_original(self, original):
        self._original = original

    def _timestamp(self):
        return self._now().strftime('%Y-%m-%d %H:%M:%S.%f')[:-3]

    def _emit(self, text, timestamp):
        self._sink({
            'type': 'stdout',
            'stream': self._stream,
            'level': self._level,
            'timestamp': timestamp,
            'source': 'Main',
            'text': text,
        })

    def write(self, text):
        if self._original is not None:
            try:
                self._original.write(text)
            except Exception:
                pass
        if text:
            rendered = _text(text)
            current_timestamp = self._timestamp()
            if not self._pending:
                self._pending_timestamp = current_timestamp
            self._pending += rendered
            while '\n' in self._pending:
                line, self._pending = self._pending.split('\n', 1)
                self._emit(line + '\n', self._pending_timestamp)
                self._pending_timestamp = (
                    current_timestamp if self._pending else None)

    def drain(self):
        if self._pending:
            self._emit(self._pending, self._pending_timestamp)
            self._pending = u''
            self._pending_timestamp = None

    def flush(self):
        self.drain()
        if self._original is not None:
            try:
                self._original.flush()
            except Exception:
                pass

    def __getattr__(self, name):
        return getattr(self._original, name)


class Capture(object):
    def __init__(self, sink, now=None):
        self._sink = sink
        self._now = now or datetime.datetime.now
        self._saved_out = None
        self._saved_err = None
        self._saved_bw = {}
        self._bw = None
        self._out_proxy = None
        self._err_proxy = None

    def install(self):
        self._saved_out = sys.stdout
        self._saved_err = sys.stderr
        self._out_proxy = _StreamProxy(
            self._saved_out, self._sink, 'stdout', 'INFO', self._now)
        self._err_proxy = _StreamProxy(
            self._saved_err, self._sink, 'stderr', 'ERROR', self._now)
        sys.stdout = self._out_proxy
        sys.stderr = self._err_proxy
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
        now = self._now

        def hook(prefix, msg, *args):
            try:
                original(prefix, msg, *args)
            finally:
                try:
                    sink({'type': 'stdout', 'stream': 'log', 'level': level,
                          'timestamp': now().strftime(
                              '%Y-%m-%d %H:%M:%S.%f')[:-3],
                          'source': 'Main',
                          'text': _format_bigworld_log(prefix, msg, args)})
                except Exception:
                    pass
        return hook

    def maintain(self):
        """Restore capture if a later-loaded mod replaced a global stream.

        Wrappers which already delegate to this proxy are left alone. This is
        important for the REPL's temporary tee and for well-behaved third-party
        wrappers: putting the proxy in front of them would create a write cycle.
        """
        self._maintain_stream('stdout', self._out_proxy)
        self._maintain_stream('stderr', self._err_proxy)

    def _maintain_stream(self, name, proxy):
        if proxy is None:
            return
        current = getattr(sys, name)
        if current is proxy or _contains_stream(current, proxy):
            return
        proxy.drain()
        proxy.set_original(current)
        if name == 'stdout':
            self._saved_out = current
        else:
            self._saved_err = current
        setattr(sys, name, proxy)

    def uninstall(self):
        if self._out_proxy is not None:
            self._out_proxy.drain()
        if self._err_proxy is not None:
            self._err_proxy.drain()
        if sys.stdout is self._out_proxy:
            sys.stdout = self._saved_out
        if sys.stderr is self._err_proxy:
            sys.stderr = self._saved_err
        if self._bw is not None:
            for name, original in self._saved_bw.items():
                setattr(self._bw, name, original)
        self._saved_bw = {}
        self._out_proxy = None
        self._err_proxy = None
