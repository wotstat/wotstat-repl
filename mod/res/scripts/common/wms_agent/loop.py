"""Agent loop: drain captured output to frames, poll inbound requests, dispatch.

A single daemon thread owns all network I/O. The game thread only ever appends
to a deque (via Capture), so it is never blocked by the channel.
"""

import os
import time
import threading
import collections

from . import __version__
from .socketbus import SocketBus
from .capture import Capture
from .runner import run_on_main
from .handlers import DISPATCH, MAIN_THREAD_OPS, seed_namespace

_state = {'agent': None, 'running': False}


class _Agent(object):
    def __init__(self, config_dir, interval):
        self._bus = SocketBus(config_dir, __version__, os.getpid())
        self._interval = interval
        self._queue = collections.deque()
        self._capture = Capture(self._queue.append)
        self._running = False
        self._thread = None

    def start(self):
        self._capture.install()
        # Seed inline, NOT via run_on_main: it only imports modules (no game-object
        # access), and start() may itself run on the game main thread -- scheduling
        # onto that same thread and blocking on it would deadlock.
        seed_namespace()
        self._running = True
        thread = threading.Thread(target=self._run)
        thread.setDaemon(True)
        thread.start()
        self._thread = thread

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
                    and frame.get('level') == last.get('level')):
                last['text'] += frame.get('text', '')
            else:
                last = dict(frame)
                merged.append(last)
        for frame in merged:
            self._bus.send(frame)

    def _dispatch(self, req):
        op = req.get('type')
        handler = DISPATCH.get(op)
        if handler is None:
            return
        try:
            if op in MAIN_THREAD_OPS:
                resp = run_on_main(lambda: handler(req))
            else:
                resp = handler(req)
        except BaseException:
            import traceback
            resp = {'id': req.get('id'), 'type': 'result', 'ok': False,
                    'exc': traceback.format_exc()}
        # Ship any stdout produced while handling this request before its
        # response, so prints precede the result in the console.
        self._flush_output()
        if resp is not None:
            self._bus.send(resp)

    def _run(self):
        while self._running:
            try:
                self._flush_output()
                for req in self._bus.poll():
                    self._dispatch(req)
            except Exception:
                pass
            time.sleep(self._interval)


def start(config_dir, interval=0.05):
    if _state['running']:
        return
    try:
        if not os.path.isdir(config_dir):
            os.makedirs(config_dir)
    except OSError:
        pass
    agent = _Agent(config_dir, interval)
    agent.start()
    _state['agent'] = agent
    _state['running'] = True


def stop():
    agent = _state.get('agent')
    if agent is not None:
        agent.stop()
    _state['running'] = False
