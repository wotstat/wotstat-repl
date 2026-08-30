"""The game main thread must never block the agent network loop (py2.7/3.x)."""

import os
import shutil
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

import wms_agent.loop as agent_loop
from wms_agent.loop import _Agent


class DeferredMainThread(object):
    def __init__(self):
        self.pending = []

    def schedule(self, fn, completed):
        self.pending.append((fn, completed))

    def finish_one(self):
        fn, completed = self.pending.pop(0)
        try:
            completed(fn(), None)
        except BaseException as error:
            completed(None, error)


class PollingBus(object):
    def __init__(self):
        self.poll_count = 0
        self.sent = []

    def poll(self):
        self.poll_count += 1
        if self.poll_count == 1:
            return [{'id': 'slow-ready', 'type': 'ready'}]
        return []

    def send(self, frame):
        self.sent.append(dict(frame))
        return True

    def close(self, _grace=0.2):
        pass


def wait_until(predicate, timeout=1.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.005)
    return predicate()


def main():
    work = tempfile.mkdtemp(prefix='wms_main_thread_')
    original_schedule = getattr(agent_loop, 'schedule_on_main', None)
    original_ready = agent_loop.DISPATCH['ready']
    agent = None
    try:
        deferred = DeferredMainThread()
        agent_loop.schedule_on_main = deferred.schedule
        agent_loop.DISPATCH['ready'] = lambda req: {
            'id': req.get('id'), 'type': 'ready', 'ok': True, 'error': None,
        }

        agent = _Agent(work, 0.001, False, None, None, python_log_path=None)
        agent._bus.close()
        agent._bus = PollingBus()
        agent._running = True
        thread = threading.Thread(target=agent._run)
        thread.daemon = True
        agent._thread = thread
        thread.start()

        assert wait_until(lambda: len(deferred.pending) == 1), deferred.pending
        assert wait_until(lambda: agent._bus.poll_count >= 3), agent._bus.poll_count
        assert not agent._bus.sent, agent._bus.sent

        deferred.finish_one()
        assert wait_until(lambda: len(agent._bus.sent) == 1), agent._bus.sent
        assert agent._bus.sent[0] == {
            'id': 'slow-ready', 'type': 'ready', 'ok': True, 'error': None,
        }, agent._bus.sent
    finally:
        if agent is not None:
            agent._running = False
            if getattr(agent, '_thread', None) is not None:
                agent._thread.join(0.25)
        agent_loop.DISPATCH['ready'] = original_ready
        if original_schedule is None:
            try:
                del agent_loop.schedule_on_main
            except AttributeError:
                pass
        else:
            agent_loop.schedule_on_main = original_schedule
        shutil.rmtree(work, ignore_errors=True)

    print('MAIN THREAD OK -- deferred game work does not block network polling')
    return 0


if __name__ == '__main__':
    sys.exit(main())
