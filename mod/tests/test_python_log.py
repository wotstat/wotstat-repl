"""Incremental python.log reader tests (Python 2.7 and 3.x)."""

import os
import shutil
import sys
import tempfile
import time


sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.pythonlog import PythonLogTail


_WEEKDAYS = ('Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun')
_MONTHS = ('Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
           'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec')
_START_SEPARATOR = '/' + '-' * 88 + '\\'


def _write(path, body, mode='wb'):
    handle = open(path, mode)
    try:
        handle.write(body)
        handle.flush()
    finally:
        handle.close()


def _game_header(epoch, product=u'WorldOfTanks(x64)'):
    value = time.localtime(epoch)
    return (
        u'%s 2.3.1.10157 #2597749 starting on ' % product +
        '%s %s %02d %02d:%02d:%02d %04d' % (
            _WEEKDAYS[value.tm_wday], _MONTHS[value.tm_mon - 1],
            value.tm_mday, value.tm_hour, value.tm_min, value.tm_sec,
            value.tm_year))


def _log_timestamp(epoch):
    return time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(epoch)) + '.000'


def main():
    work = tempfile.mkdtemp(prefix='wms_python_log_')
    path = os.path.join(work, 'python.log')
    try:
        now = time.time()
        old_header = _game_header(now - 86400)
        current_header = _game_header(
            now - 30,
            u'\u041c\u0438\u0440 \u0442\u0430\u043d\u043a\u043e\u0432(x64)',
        )
        current_timestamp = _log_timestamp(now - 29)
        _write(path, (
            (_START_SEPARATOR + '\n' + old_header + '\n\n'
             '2026-08-19 01:39:35.767: INFO: Main: previous game\n'
             + _START_SEPARATOR + '\r\n' + current_header + '\r\n\r\n'
             + current_timestamp + ': INFO: Main: current game\n')
            .encode('utf-8')))

        current_session = PythonLogTail(path, interval=0).poll()
        assert [frame.get('text') for frame in current_session] == [
            _START_SEPARATOR + '\n',
            current_header + '\n',
            '\n',
            'current game\n',
        ], current_session
        assert all('previous game' not in frame.get('text', '')
                   for frame in current_session), current_session

        _write(path, (
            b'\xef\xbb\xbf2026-08-20 04:31:08.295: INFO: Main: '
            b'[web.cache.web_cache] WebDownloader created\r\n'
            b'2026-08-20 04:31:09.776: WARNING: Main: partial'))

        tail = PythonLogTail(path, interval=0)
        assert tail.poll() == []

        _write(path, (
            b' warning\n'
            b'2026-08-20 04:31:10.445: DEBUG: Main: ready\n'), 'ab')
        assert tail.poll() == [{
            'type': 'stdout',
            'stream': 'python_log',
            'timestamp': '2026-08-20 04:31:10.445',
            'level': 'DEBUG',
            'source': 'Main',
            'text': 'ready\n',
        }]
        assert tail.poll() == []

        _write(path, (
            b'2026-08-20 04:32:00.000: ERROR: Main: after truncate\n'))
        assert tail.poll() == [{
            'type': 'stdout',
            'stream': 'python_log',
            'timestamp': '2026-08-20 04:32:00.000',
            'level': 'ERROR',
            'source': 'Main',
            'text': 'after truncate\n',
        }]

        replacement = os.path.join(work, 'replacement.log')
        _write(replacement, (
            b'2026-08-20 04:33:00.000: NOTICE: Render: after rotation\n'))
        os.remove(path)
        os.rename(replacement, path)
        assert tail.poll() == [{
            'type': 'stdout',
            'stream': 'python_log',
            'timestamp': '2026-08-20 04:33:00.000',
            'level': 'NOTICE',
            'source': 'Render',
            'text': 'after rotation\n',
        }]

        print('PYTHON LOG OK -- game-start cutoff, partials, truncate, rotation')
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
