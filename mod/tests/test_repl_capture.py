"""Unified live capture and python.log mirror de-duplication (Python 2.7)."""

import os
import shutil
import sys
import tempfile
try:
    from cStringIO import StringIO
except ImportError:
    from io import StringIO


sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.loop import _Agent


class _RecordingBus(object):
    def __init__(self):
        self.sent = []

    def send(self, frame):
        self.sent.append(dict(frame))
        return True


def main():
    work = tempfile.mkdtemp(prefix='wms_repl_capture_')
    try:
        agent = _Agent(work, 0.01, False, None, None, python_log_path=None)
        agent._bus.close()
        agent._bus = _RecordingBus()

        process_out = sys.stdout
        sys.stdout = StringIO()
        agent._capture.install()
        try:
            print('other mod')
            agent._dispatch({
                'id': 'repl-print',
                'type': 'exec',
                'code': "print('Renou_EU')",
            })
            long_json = '{"payload":"%s"}' % ('x' * 12000)
            print(long_json)
            agent._flush_output()
        finally:
            agent._capture.uninstall()
            sys.stdout = process_out

        captured = [
            frame for frame in agent._bus.sent
            if frame.get('stream') == 'stdout'
        ]
        assert [frame.get('text') for frame in captured] == [
            'other mod\n',
            'Renou_EU\n',
            long_json + '\n',
        ], captured

        # python.log is written a moment before the proxy timestamps the same
        # print, and the file copy can be shorter than the full live payload.
        mirrors = [{
            'type': 'stdout',
            'stream': 'python_log',
            'level': 'INFO',
            'timestamp': '2026-08-20 16:52:33.304',
            'source': 'Main',
            'text': 'Renou_EU\n',
        }, {
            'type': 'stdout',
            'stream': 'python_log',
            'level': 'INFO',
            'timestamp': captured[2]['timestamp'],
            'source': 'Main',
            'text': long_json[:400] + '\n',
        }]
        captured[1]['timestamp'] = '2026-08-20 16:52:33.305'
        # Re-seed the live cache with the adjusted regression timestamp.
        agent._queue.append(captured[1])
        agent._flush_output()
        agent._queue.extend(mirrors)
        agent._flush_output()
        assert all(frame not in agent._bus.sent for frame in mirrors), agent._bus.sent

        # BigWorld's Python wrapper splits long log calls into 1792-character
        # file records and repeats the category on every chunk. One full live
        # frame must suppress the whole mirrored chunk sequence.
        log_body = 'y' * 4200
        live_log = {
            'type': 'stdout',
            'stream': 'log',
            'level': 'logInfo',
            'timestamp': '2026-08-20 16:52:35.100',
            'source': 'Main',
            'text': '[json] ' + log_body + '\n',
        }
        agent._queue.append(live_log)
        agent._flush_output()
        chunk_frames = []
        for index, start in enumerate(range(0, len(log_body), 1792)):
            chunk_frames.append({
                'type': 'stdout',
                'stream': 'python_log',
                'level': 'INFO',
                'timestamp': '2026-08-20 16:52:35.%03d' % (99 + index),
                'source': 'Main',
                'text': '[json] ' + log_body[start:start + 1792] + '\n',
            })
        agent._queue.extend(chunk_frames)
        agent._flush_output()
        assert all(frame not in agent._bus.sent for frame in chunk_frames), agent._bus.sent

        file_only = {
            'type': 'stdout',
            'stream': 'python_log',
            'level': 'WARNING',
            'timestamp': '2026-08-20 16:52:34.000',
            'source': 'Main',
            'text': 'not intercepted live\n',
        }
        agent._queue.append(file_only)
        agent._flush_output()
        assert file_only in agent._bus.sent, agent._bus.sent

        print('REPL CAPTURE OK -- all live output plus file-only fallback')
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
