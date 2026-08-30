"""Agent loop: drain captured output to frames, poll inbound requests, dispatch.

A single daemon thread owns all network I/O. The game thread only ever appends
to a deque (via Capture), so it is never blocked by the channel.
"""

import os
import time
import threading
import collections

from . import __version__, __web_enabled__
from .capture import Capture
from .pythonlog import PythonLogTail
from .runner import schedule_on_main
from .handlers import (
    DISPATCH,
    MAIN_THREAD_OPS,
    seed_namespace,
)

_state = {'agent': None, 'running': False}
_LIVE_MIRROR_STREAMS = frozenset(('stdout', 'stderr', 'log'))
_LIVE_MIRROR_CACHE = 4096
_MIRROR_TIME_WINDOW_MS = 250
_MIRROR_PREFIX_MIN = 64
_DAYS_BEFORE_MONTH = (
    0, 31, 59, 90, 120, 151,
    181, 212, 243, 273, 304, 334,
)


def _normalized_level(value):
    level = str(value or '')
    if level.lower().startswith('log'):
        level = level[3:]
    return level.upper()


def _timestamp_millis(value):
    try:
        if not value or len(value) < 19:
            return None
        if (value[4] != '-' or value[7] != '-' or value[10] != ' ' or
                value[13] != ':' or value[16] != ':'):
            return None
        year = int(value[0:4])
        month = int(value[5:7])
        day = int(value[8:10])
        hour = int(value[11:13])
        minute = int(value[14:16])
        second = int(value[17:19])
        if year < 1 or year > 9999 or month < 1 or month > 12:
            return None
        leap_year = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
        month_days = (
            31, 29 if leap_year else 28, 31, 30, 31, 30,
            31, 31, 30, 31, 30, 31,
        )
        if (day < 1 or day > month_days[month - 1] or
                hour < 0 or hour > 23 or minute < 0 or minute > 59 or
                second < 0 or second > 59):
            return None
        previous_year = year - 1
        ordinal = (
            365 * previous_year
            + previous_year // 4
            - previous_year // 100
            + previous_year // 400
            + _DAYS_BEFORE_MONTH[month - 1]
            + day
        )
        if month > 2 and leap_year:
            ordinal += 1
        seconds = (((ordinal * 24 + hour) * 60 + minute) * 60 + second)
        fraction = value[20:] if len(value) > 19 and value[19] == '.' else ''
        return seconds * 1000 + int((fraction + '000')[:3])
    except (IndexError, TypeError, ValueError, OverflowError):
        return None


def _mirror_text(value):
    if value is None:
        return u''
    try:
        text_type = unicode
    except NameError:
        text_type = str
    if isinstance(value, text_type):
        return value.rstrip('\r\n')
    if isinstance(value, bytes):
        return value.decode('utf-8', 'replace').rstrip('\r\n')
    return text_type(value).rstrip('\r\n')


def _category_and_body(value):
    text = _mirror_text(value)
    position = 0
    categories = []
    while position < len(text) and text[position:position + 1] == '[':
        end = text.find(']', position + 1)
        if end < 0:
            break
        categories.append(text[position:end + 1])
        position = end + 1
        while position < len(text) and text[position:position + 1].isspace():
            position += 1
    return ''.join(categories), text[position:] if categories else text


def _live_mirror(frame):
    timestamp = _timestamp_millis(frame.get('timestamp'))
    level = _normalized_level(frame.get('level'))
    text = _mirror_text(frame.get('text'))
    if timestamp is None or not level or not text:
        return None
    category, body = _category_and_body(text)
    return {
        'timestamp': timestamp,
        'level': level,
        'source': frame.get('source'),
        'category': category,
        'body': body,
        'offset': 0,
    }


def _mirror_match(record, frame):
    timestamp = _timestamp_millis(frame.get('timestamp'))
    if timestamp is None:
        return None
    delta = abs(record['timestamp'] - timestamp)
    if delta > _MIRROR_TIME_WINDOW_MS:
        return None
    if record['level'] != _normalized_level(frame.get('level')):
        return None
    source = frame.get('source')
    if record['source'] and source and record['source'] != source:
        return None
    category, body = _category_and_body(frame.get('text'))
    if record['category'] != category or not body:
        return None
    remaining = record['body'][record['offset']:]
    if body == remaining:
        return delta, len(record['body']), True
    if len(body) >= _MIRROR_PREFIX_MIN and remaining.startswith(body):
        return delta, record['offset'] + len(body), False
    return None


class _Agent(object):
    def __init__(self, config_dir, interval, web_enabled, web_root, web_port,
                 python_log_path=None):
        from .socketbus import SocketBus
        tcp_bus = SocketBus(config_dir, __version__, os.getpid())
        if web_enabled:
            from .hybridbus import HybridBus
            from .webbus import WebBus
            web_bus = None
            web_error = None
            try:
                web_bus = WebBus(
                    config_dir, __version__, os.getpid(), web_root, web_port)
            except Exception as error:
                web_error = str(error)
            self._bus = HybridBus(tcp_bus, web_bus, web_error)
        else:
            self._bus = tcp_bus
        self._interval = interval
        self._queue = collections.deque()
        self._completed = collections.deque()
        self._capture = Capture(self._queue.append)
        self._live_mirrors = collections.deque(maxlen=_LIVE_MIRROR_CACHE)
        self._python_log = (PythonLogTail(python_log_path)
                            if python_log_path else None)
        self._running = False
        self._thread = None

    def start(self):
        self._capture.install()
        web_error = getattr(self._bus, 'web_error', None)
        if web_error:
            print('WotStat REPL: web UI unavailable: %s' % web_error)
        # Seed inline, NOT via schedule_on_main: it only imports modules (no game-object
        # access), and start() may itself run on the game main thread -- scheduling
        # onto that same thread and blocking on it would deadlock.
        seed_namespace()
        self._running = True
        thread = threading.Thread(target=self._run)
        thread.setDaemon(True)
        thread.start()
        self._thread = thread
        return getattr(self._bus, 'endpoint', None)

    def stop(self):
        if not self._running:
            return
        self._running = False
        if self._thread is not None and threading.current_thread() is not self._thread:
            self._thread.join(0.25)
        try:
            self._capture.uninstall()
        finally:
            self._flush_output()
            self._bus.send({'type': 'disconnected'})
            self._bus.close(0.2)

    def _flush_output(self):
        pending = len(self._queue)
        if not pending:
            return
        merged = []
        last = None
        for _ in range(pending):
            try:
                frame = self._queue.popleft()
            except IndexError:
                break
            if (last is not None
                    and frame.get('stream') == last.get('stream')
                    and frame.get('level') == last.get('level')
                    and frame.get('timestamp') is None
                    and last.get('timestamp') is None):
                last['text'] += frame.get('text', '')
            else:
                last = dict(frame)
                merged.append(last)
        for frame in merged:
            if frame.get('stream') == 'python_log':
                if self._consume_python_log_mirror(frame):
                    continue
            delivered = self._bus.send(frame)
            if delivered and frame.get('stream') in _LIVE_MIRROR_STREAMS:
                mirror = _live_mirror(frame)
                if mirror is not None:
                    self._live_mirrors.append(mirror)

    def _consume_python_log_mirror(self, frame):
        best = None
        for record in self._live_mirrors:
            match = _mirror_match(record, frame)
            if match is None:
                continue
            delta, offset, complete = match
            if best is None or delta < best[0]:
                best = (delta, record, offset, complete)
        if best is None:
            return False
        _, record, offset, complete = best
        record['offset'] = offset
        if complete:
            try:
                self._live_mirrors.remove(record)
            except ValueError:
                pass
        return True

    def _dispatch(self, req):
        op = req.get('type')
        handler = DISPATCH.get(op)
        if handler is None:
            return
        if op in MAIN_THREAD_OPS:
            try:
                schedule_on_main(
                    lambda: handler(req),
                    lambda response, error: self._completed.append(
                        (req, response, error)))
            except BaseException:
                import traceback
                self._completed.append((req, None, traceback.format_exc()))
            self._flush_completed()
            return
        try:
            resp = handler(req)
        except BaseException:
            import traceback
            self._finish_dispatch(req, None, traceback.format_exc())
            return
        self._finish_dispatch(req, resp, None)

    def _finish_dispatch(self, req, resp, error):
        if error is not None:
            response_type = {
                'ready': 'ready',
                'screenshot': 'screenshot_started',
                'mouse': 'input',
                'keyboard': 'input',
            }.get(req.get('type'))
            if response_type is None:
                resp = {'id': req.get('id'), 'type': 'result', 'ok': False,
                        'exc': error}
            else:
                resp = {'id': req.get('id'), 'type': response_type,
                        'ok': False, 'error': error}
        # Ship any stdout produced while handling this request before its
        # response, so prints precede the result in the console.
        self._flush_output()
        if resp is not None:
            self._bus.send(resp)

    def _flush_completed(self):
        pending = len(self._completed)
        for _ in range(pending):
            try:
                req, response, error = self._completed.popleft()
            except IndexError:
                break
            self._finish_dispatch(req, response, error)

    def _run(self):
        while self._running:
            try:
                self._capture.maintain()
                if self._python_log is not None:
                    self._queue.extend(self._python_log.poll())
                self._flush_output()
                self._flush_completed()
                for req in self._bus.poll():
                    self._dispatch(req)
            except Exception:
                pass
            time.sleep(self._interval)


def start(config_dir, interval=0.05, web_enabled=None, web_root=None, web_port=None,
          python_log_path=None):
    if _state['running']:
        agent = _state.get('agent')
        return getattr(agent._bus, 'endpoint', None) if agent is not None else None
    if web_enabled is None:
        web_enabled = __web_enabled__
    try:
        if not os.path.isdir(config_dir):
            os.makedirs(config_dir)
    except OSError:
        pass
    if python_log_path is None:
        python_log_path = os.path.join(os.getcwd(), 'python.log')
    agent = _Agent(config_dir, interval, web_enabled, web_root, web_port,
                   python_log_path)
    endpoint = agent.start()
    _state['agent'] = agent
    _state['running'] = True
    return endpoint


def stop():
    agent = _state.get('agent')
    if agent is not None:
        agent.stop()
    _state['running'] = False
