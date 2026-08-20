"""Regression tests for BigWorld log capture formatting."""

import os
import sys
import types
import datetime
try:
    from cStringIO import StringIO
except ImportError:
    from io import StringIO


sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.capture import Capture


def main():
    frames = []
    calls = []
    fake_bigworld = types.ModuleType('BigWorld')

    def log_info(prefix, message, *args):
        calls.append((prefix, message, args))

    fake_bigworld.logInfo = log_info
    saved_bigworld = sys.modules.get('BigWorld')
    sys.modules['BigWorld'] = fake_bigworld
    capture = Capture(
        frames.append,
        now=lambda: datetime.datetime(2026, 8, 19, 21, 48, 32, 33000),
    )
    process_out = sys.stdout
    try:
        capture.install()
        print('Renou_EU')
        fake_bigworld.logInfo(
            'web.cache.web_cache', 'WebDownloader destroyed')
        fake_bigworld.logInfo('json', 'value=%s', None)

        # A mod can replace the process-global stream after our early loader.
        # The periodic maintenance pass must put capture back in front while
        # continuing to mirror output to that mod's replacement stream.
        replacement = StringIO()
        sys.stdout = replacement
        capture.maintain()
        print('other mod')
        long_json = '{"payload":"%s"}' % ('x' * 12000)
        print(long_json)
    finally:
        capture.uninstall()
        sys.stdout = process_out
        if saved_bigworld is None:
            sys.modules.pop('BigWorld', None)
        else:
            sys.modules['BigWorld'] = saved_bigworld

    assert calls == [
        ('web.cache.web_cache', 'WebDownloader destroyed', ()),
        ('json', 'value=%s', (None,)),
    ], calls
    log_frames = [frame for frame in frames if frame.get('stream') == 'log']
    assert log_frames == [{
        'type': 'stdout',
        'stream': 'log',
        'level': 'logInfo',
        'timestamp': '2026-08-19 21:48:32.033',
        'source': 'Main',
        'text': '[web.cache.web_cache] WebDownloader destroyed\n',
    }, {
        'type': 'stdout',
        'stream': 'log',
        'level': 'logInfo',
        'timestamp': '2026-08-19 21:48:32.033',
        'source': 'Main',
        'text': '[json] value=%s\n',
    }], log_frames
    stdout_frames = [
        frame for frame in frames if frame.get('stream') == 'stdout']
    assert stdout_frames[:2] == [{
        'type': 'stdout',
        'stream': 'stdout',
        'level': 'INFO',
        'timestamp': '2026-08-19 21:48:32.033',
        'source': 'Main',
        'text': 'Renou_EU\n',
    }, {
        'type': 'stdout',
        'stream': 'stdout',
        'level': 'INFO',
        'timestamp': '2026-08-19 21:48:32.033',
        'source': 'Main',
        'text': 'other mod\n',
    }], stdout_frames
    assert stdout_frames[2]['text'] == long_json + '\n', len(
        stdout_frames[2]['text'])
    assert replacement.getvalue() == 'other mod\n' + long_json + '\n'

    print('CAPTURE OK -- global print, long lines and BigWorld formatting')
    return 0


if __name__ == '__main__':
    sys.exit(main())
