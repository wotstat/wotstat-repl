"""Integration test for the Python 2.7 embedded HTTP REPL transport."""

import json
import os
import shutil
import sys
import tempfile

try:
    from urllib2 import HTTPError, Request, urlopen
except ImportError:  # pragma: no cover - Python 3 compatibility
    from urllib.error import HTTPError
    from urllib.request import Request, urlopen

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

import wms_agent


def read_json(url):
    response = urlopen(url, timeout=4.0)
    try:
        return json.loads(response.read())
    finally:
        response.close()


def post_json(url, origin, value):
    body = json.dumps(value, separators=(',', ':'))
    if not isinstance(body, bytes):
        body = body.encode('utf-8')
    request = Request(url, data=body, headers={
        'Content-Type': 'application/json',
        'Origin': origin,
    })
    response = urlopen(request, timeout=4.0)
    try:
        return json.loads(response.read())
    finally:
        response.close()


def main():
    work = tempfile.mkdtemp(prefix='wms_web_itest_')
    web_root = os.path.join(work, 'web')
    python_log = os.path.join(work, 'python.log')
    os.makedirs(os.path.join(web_root, 'assets'))
    with open(os.path.join(web_root, 'index.html'), 'wb') as handle:
        handle.write(b'<!doctype html><script src="/assets/app.js"></script>')
    with open(os.path.join(web_root, 'assets', 'app.js'), 'wb') as handle:
        handle.write(b'document.body.dataset.ready = "yes";')
    with open(python_log, 'wb') as handle:
        handle.write(
            b'2026-08-19 04:31:08.295: INFO: Main: '
            b'[web.cache.web_cache] from old native game log\n')

    endpoint = None
    try:
        endpoint = wms_agent.start(
            work,
            interval=0.01,
            web_enabled=True,
            web_root=web_root,
            web_port=0,
            python_log_path=python_log,
        )
        assert endpoint and endpoint.startswith('http://127.0.0.1:'), endpoint
        origin = endpoint.rstrip('/')

        with open(python_log, 'ab') as handle:
            handle.write(
                b'2026-08-20 04:31:08.295: INFO: Main: '
                b'[web.cache.web_cache] from current native game log\n')

        page = urlopen(endpoint, timeout=4.0)
        try:
            assert b'/assets/app.js' in page.read()
            assert page.headers.get('Content-Security-Policy')
        finally:
            page.close()

        session = read_json(endpoint + 'api/session')
        assert session.get('pid') == os.getpid(), session
        assert session.get('session'), session

        evaluated = post_json(endpoint + 'api/repl', origin, {
            'type': 'exec', 'code': '40 + 2',
        })
        assert evaluated.get('ok') and evaluated.get('repr') == '42', evaluated

        printed = post_json(endpoint + 'api/repl', origin, {
            'type': 'exec', 'code': "print('web-line')",
        })
        assert printed.get('ok'), printed
        cursor = 0
        collected = []
        events = None
        for _attempt in range(5):
            events = read_json(
                endpoint + 'api/events?cursor=%d&limit=500&wait_ms=1000' % cursor)
            collected.extend(events.get('events', []))
            cursor = events.get('nextCursor', cursor)
            if any(event.get('stream') == 'python_log' for event in collected):
                break
        output = ''.join(
            event.get('text', '') for event in collected
            if event.get('type') == 'stdout')
        assert 'web-line' in output, repr(output)
        assert 'old native game log' not in output, repr(output)
        file_events = [
            event for event in collected
            if event.get('stream') == 'python_log'
        ]
        assert file_events == [{
            'type': 'stdout',
            'stream': 'python_log',
            'timestamp': '2026-08-20 04:31:08.295',
            'level': 'INFO',
            'source': 'Main',
            'text': '[web.cache.web_cache] from current native game log\n',
        }], file_events
        assert cursor > 0, events

        try:
            post_json(endpoint + 'api/repl', 'http://example.invalid', {
                'type': 'exec', 'code': '1',
            })
            raise AssertionError('cross-origin request should be rejected')
        except HTTPError as error:
            assert error.code == 403, error.code

        print('WEB OK -- static/session/exec/python.log events/origin guard')
        return 0
    finally:
        try:
            wms_agent.stop()
        except Exception:
            pass
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
