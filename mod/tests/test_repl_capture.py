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

import wms_agent.loop as agent_loop
from wms_agent.loop import _Agent, _timestamp_millis
from wms_agent.pythonlog import _frame


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

        # Timestamp parsing runs on the REPL worker while the game is still
        # loading mods.  time.strptime() performs unsafe one-time locale/cache
        # initialization there, which deadlocks the Lesta client.
        original_strptime = agent_loop.time.strptime

        def forbidden_strptime(*args, **kwargs):
            raise RuntimeError('time.strptime must not run on the REPL worker')

        agent_loop.time.strptime = forbidden_strptime
        startup_frame = {
            'type': 'stdout',
            'stream': 'log',
            'level': 'logInfo',
            'timestamp': '2026-08-20 19:18:57.714',
            'source': 'Main',
            'text': '[startup] REPL ready\n',
        }
        try:
            agent._queue.append(startup_frame)
            agent._flush_output()
        finally:
            agent_loop.time.strptime = original_strptime
        assert startup_frame in agent._bus.sent, agent._bus.sent

        rollover = (
            _timestamp_millis('2026-08-20 19:59:59.900'),
            _timestamp_millis('2026-08-20 20:00:00.100'),
        )
        leap_day = (
            _timestamp_millis('2024-02-28 23:59:59.900'),
            _timestamp_millis('2024-02-29 00:00:00.100'),
        )
        assert rollover[1] - rollover[0] == 200, rollover
        assert leap_day[1] - leap_day[0] == 200, leap_day
        for invalid_timestamp in (
                None, '', '2026-13-20 19:18:57.714',
                '2026-02-29 19:18:57.714', '2026-08-20 24:18:57.714'):
            assert _timestamp_millis(invalid_timestamp) is None, invalid_timestamp

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

        # Lesta omits the source field written by WG (``Main:``). The file
        # record still needs to retain its metadata so it can be matched to
        # and suppressed as a mirror of the live capture.
        lesta_live = {
            'type': 'stdout',
            'stream': 'stdout',
            'level': 'INFO',
            'timestamp': '2026-08-20 18:49:02.415',
            'source': 'Main',
            'text': 'Renou\n',
        }
        lesta_mirror = _frame(
            b'2026-08-20 18:49:02.414: INFO: Renou')
        agent._queue.append(lesta_live)
        agent._flush_output()
        agent._queue.append(lesta_mirror)
        agent._flush_output()
        assert lesta_mirror not in agent._bus.sent, agent._bus.sent

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
