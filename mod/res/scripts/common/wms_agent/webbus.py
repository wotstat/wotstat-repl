"""Loopback HTTP transport and static UI host for the web-enabled mod.

Python 2.7's standard library is intentionally sufficient here. HTTP request
threads enqueue REPL frames; the existing agent poll thread dispatches them and
delivers correlated replies through ``send``. Async stdout frames are retained
in a bounded sequence log consumed by long polling.
"""

import collections
import json
import os
import threading
import time
import uuid

try:
    from BaseHTTPServer import BaseHTTPRequestHandler, HTTPServer
    from SocketServer import ThreadingMixIn
    from urlparse import parse_qs, urlparse
    from urllib import unquote
except ImportError:  # pragma: no cover - Python 3 test compatibility
    from http.server import BaseHTTPRequestHandler, HTTPServer
    from socketserver import ThreadingMixIn
    from urllib.parse import parse_qs, unquote, urlparse


DEFAULT_WEB_PORT = 8768
MAX_REQUEST_BYTES = 2 * 1024 * 1024
MAX_EVENT_BYTES = 8 * 1024 * 1024
MAX_EVENT_LIMIT = 1000
MAX_WAIT_SECONDS = 20.0
REQUEST_TIMEOUT = 30.0
RESOURCE_ROOT = 'wotstat_repl/web'
ALLOWED_OPERATIONS = frozenset(['exec', 'complete', 'inspect', 'lint'])

_MIME_TYPES = {
    '.css': 'text/css; charset=utf-8',
    '.html': 'text/html; charset=utf-8',
    '.ico': 'image/x-icon',
    '.js': 'application/javascript; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.map': 'application/json; charset=utf-8',
    '.png': 'image/png',
    '.svg': 'image/svg+xml',
    '.ttf': 'font/ttf',
    '.woff': 'font/woff',
    '.woff2': 'font/woff2',
}


def _json_bytes(value):
    body = json.dumps(value, ensure_ascii=True, separators=(',', ':'))
    if not isinstance(body, bytes):
        body = body.encode('utf-8')
    return body


class WebBusError(Exception):
    pass


class _Pending(object):
    def __init__(self):
        self.ready = threading.Event()
        self.response = None
        self.error = None


class _AssetStore(object):
    def __init__(self, filesystem_root=None):
        self._filesystem_root = (
            os.path.abspath(filesystem_root) if filesystem_root else None)

    def read(self, path):
        if self._filesystem_root is not None:
            target = os.path.abspath(os.path.join(self._filesystem_root, path))
            root_prefix = self._filesystem_root + os.sep
            if target != self._filesystem_root and not target.startswith(root_prefix):
                return None
            if not os.path.isfile(target):
                return None
            handle = open(target, 'rb')
            try:
                return handle.read()
            finally:
                handle.close()

        try:
            import ResMgr
            section = ResMgr.openSection('%s/%s' % (RESOURCE_ROOT, path))
            return None if section is None else section.asBinary
        except (ImportError, AttributeError):
            return None


class _ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, _request, _client_address):
        # Browsers routinely abandon a long-poll socket when a tab reloads or
        # closes. Python 2.7 otherwise prints a Broken pipe traceback.
        pass


class _RequestHandler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    server_version = 'WotStatREPL/1'
    sys_version = ''

    def log_message(self, _format, *_args):
        # Request access logs would be captured as game output and feed back
        # into the browser console.
        pass

    def do_GET(self):
        if not self._valid_source():
            self._send_json(403, {'error': 'forbidden request origin'})
            return
        parsed = urlparse(self.path)
        if parsed.path == '/api/session':
            self._send_json(200, self.server.bus.session_info())
        elif parsed.path == '/api/events':
            self._read_events(parsed.query)
        elif parsed.path.startswith('/api/'):
            self._send_json(404, {'error': 'unknown endpoint'})
        else:
            self._serve_asset(parsed.path, False)

    def do_HEAD(self):
        if not self._valid_source():
            self._send_json(403, {'error': 'forbidden request origin'}, True)
            return
        parsed = urlparse(self.path)
        if parsed.path.startswith('/api/'):
            self._send_json(405, {'error': 'method not allowed'}, True)
        else:
            self._serve_asset(parsed.path, True)

    def do_POST(self):
        if not self._valid_source():
            self._send_json(403, {'error': 'forbidden request origin'})
            return
        parsed = urlparse(self.path)
        if parsed.path != '/api/repl':
            self._send_json(404, {'error': 'unknown endpoint'})
            return
        content_type = self.headers.get('Content-Type', '').split(';', 1)[0].strip().lower()
        if content_type != 'application/json':
            self._send_json(415, {'error': 'application/json is required'})
            return
        try:
            length = int(self.headers.get('Content-Length', ''))
        except (TypeError, ValueError):
            length = -1
        if length < 0 or length > MAX_REQUEST_BYTES:
            self._send_json(413, {'error': 'invalid request size'})
            return
        try:
            raw = self.rfile.read(length)
            if not isinstance(raw, str):
                raw = raw.decode('utf-8')
            frame = json.loads(raw)
        except (TypeError, ValueError, UnicodeError):
            self._send_json(400, {'error': 'invalid JSON request'})
            return
        if not isinstance(frame, dict) or frame.get('type') not in ALLOWED_OPERATIONS:
            self._send_json(400, {'error': 'invalid REPL operation'})
            return
        try:
            response = self.server.bus.submit(frame, REQUEST_TIMEOUT)
            self._send_json(200, response)
        except WebBusError as error:
            self._send_json(503, {'error': str(error)})

    def do_OPTIONS(self):
        # Cross-origin JSON requests trigger a preflight. Deliberately return no
        # CORS permission so another web page cannot drive the local REPL.
        self._send_json(405, {'error': 'cross-origin requests are not allowed'})

    def _valid_source(self):
        host = self.headers.get('Host', '').lower()
        if host not in self.server.bus.allowed_hosts:
            return False
        origin = self.headers.get('Origin')
        return origin is None or origin.lower() in self.server.bus.allowed_origins

    def _read_events(self, query):
        params = parse_qs(query)
        try:
            cursor = max(0, int(params.get('cursor', ['0'])[0]))
            limit = min(MAX_EVENT_LIMIT, max(1, int(params.get('limit', ['500'])[0])))
            wait_ms = min(
                int(MAX_WAIT_SECONDS * 1000),
                max(0, int(params.get('wait_ms', ['0'])[0])),
            )
        except (TypeError, ValueError):
            self._send_json(400, {'error': 'invalid event cursor'})
            return
        self._send_json(
            200,
            self.server.bus.read_events(cursor, limit, wait_ms / 1000.0),
        )

    def _serve_asset(self, request_path, head_only):
        try:
            path = unquote(request_path)
            if not isinstance(path, str):
                path = path.decode('utf-8')
        except (UnicodeError, ValueError):
            self._send_json(400, {'error': 'invalid asset path'}, head_only)
            return
        path = path.lstrip('/') or 'index.html'
        if any(part in ('', '.', '..') for part in path.split('/')):
            self._send_json(404, {'error': 'asset not found'}, head_only)
            return
        body = self.server.bus.assets.read(path)
        if body is None:
            self._send_json(404, {'error': 'asset not found'}, head_only)
            return
        extension = os.path.splitext(path)[1].lower()
        mime = _MIME_TYPES.get(extension, 'application/octet-stream')
        cache = 'no-store' if path == 'index.html' else 'public, max-age=31536000, immutable'
        self._send(200, body, mime, cache, head_only)

    def _send_json(self, status, value, head_only=False):
        self._send(
            status,
            _json_bytes(value),
            'application/json; charset=utf-8',
            'no-store',
            head_only,
        )

    def _send(self, status, body, content_type, cache_control, head_only):
        if not isinstance(body, bytes):
            body = body.encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', cache_control)
        self.send_header('Content-Security-Policy',
                         "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
                         "font-src 'self'; connect-src 'self'; worker-src 'self' blob:; "
                         "img-src 'self' data:; object-src 'none'; "
                         "base-uri 'none'; frame-ancestors 'none'")
        self.send_header('Cross-Origin-Resource-Policy', 'same-origin')
        self.send_header('Referrer-Policy', 'no-referrer')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.end_headers()
        if not head_only:
            try:
                self.wfile.write(body)
            except (IOError, OSError):
                pass


class WebBus(object):
    def __init__(self, _config_dir, version, pid, web_root=None, port=None):
        self._version = version
        self._pid = pid
        self._session = uuid.uuid4().hex
        self.assets = _AssetStore(web_root)
        self._running = True
        self._requests = collections.deque()
        self._request_lock = threading.Lock()
        self._pending = {}
        self._pending_lock = threading.Lock()
        self._events = collections.deque()
        self._event_bytes = 0
        self._next_sequence = 1
        self._dropped_through = 0
        self._event_condition = threading.Condition()

        bind_port = DEFAULT_WEB_PORT if port is None else int(port)
        self._server = _ThreadingHTTPServer(
            ('127.0.0.1', bind_port), _RequestHandler)
        self._server.bus = self
        self._port = self._server.server_address[1]
        self.endpoint = 'http://127.0.0.1:%d/' % self._port
        host_suffix = ':%d' % self._port
        self.allowed_hosts = frozenset([
            '127.0.0.1' + host_suffix,
            'localhost' + host_suffix,
        ])
        self.allowed_origins = frozenset([
            'http://127.0.0.1' + host_suffix,
            'http://localhost' + host_suffix,
        ])
        self._server_thread = threading.Thread(target=self._server.serve_forever)
        self._server_thread.setDaemon(True)
        self._server_thread.start()

    def session_info(self):
        return {
            'version': self._version,
            'pid': self._pid,
            'session': self._session,
        }

    def poll(self):
        requests = []
        with self._request_lock:
            while self._requests:
                requests.append(self._requests.popleft())
        return requests

    def submit(self, frame, timeout):
        if not self._running:
            raise WebBusError('web REPL is stopped')
        frame = dict(frame)
        request_id = uuid.uuid4().hex
        frame['id'] = request_id
        pending = _Pending()
        with self._pending_lock:
            self._pending[request_id] = pending
        with self._request_lock:
            self._requests.append(frame)
        if not pending.ready.wait(timeout):
            with self._pending_lock:
                if self._pending.get(request_id) is pending:
                    self._pending.pop(request_id, None)
            raise WebBusError('game did not answer the REPL request in time')
        if pending.error is not None:
            raise WebBusError(pending.error)
        return pending.response

    def send(self, frame):
        frame = dict(frame)
        request_id = frame.get('id')
        if request_id is not None:
            with self._pending_lock:
                pending = self._pending.pop(request_id, None)
            if pending is not None:
                pending.response = frame
                pending.ready.set()
            return True

        encoded_size = len(_json_bytes(frame))
        with self._event_condition:
            sequence = self._next_sequence
            self._next_sequence += 1
            self._events.append((sequence, encoded_size, frame))
            self._event_bytes += encoded_size
            while self._events and self._event_bytes > MAX_EVENT_BYTES:
                old_sequence, old_size, _old_frame = self._events.popleft()
                self._event_bytes -= old_size
                self._dropped_through = old_sequence
            self._event_condition.notify_all()
        return True

    def read_events(self, cursor, limit, wait_seconds):
        deadline = time.time() + max(0.0, wait_seconds)
        with self._event_condition:
            while self._running and not self._has_events_after(cursor):
                remaining = deadline - time.time()
                if remaining <= 0:
                    break
                self._event_condition.wait(remaining)

            truncated = cursor < self._dropped_through
            effective_cursor = max(cursor, self._dropped_through)
            selected = []
            for sequence, _size, frame in self._events:
                if sequence > effective_cursor:
                    selected.append((sequence, frame))
                    if len(selected) >= limit:
                        break
            next_cursor = selected[-1][0] if selected else effective_cursor
            return {
                'events': [frame for _sequence, frame in selected],
                'nextCursor': next_cursor,
                'truncated': truncated,
            }

    def _has_events_after(self, cursor):
        return bool(self._events and self._events[-1][0] > cursor)

    def close(self, _grace=0.2):
        if not self._running:
            return
        self._running = False
        with self._event_condition:
            self._event_condition.notify_all()
        with self._pending_lock:
            pending = list(self._pending.values())
            self._pending.clear()
        for request in pending:
            request.error = 'web REPL stopped before the game answered'
            request.ready.set()
        self._server.shutdown()
        self._server.server_close()
        if threading.current_thread() is not self._server_thread:
            self._server_thread.join(0.5)
