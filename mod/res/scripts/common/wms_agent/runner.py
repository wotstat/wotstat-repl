"""Run a callable on the game main thread and return its result.

BigWorld is single threaded; touching entities/GUI from the agent's poll thread
can crash the client (this is the bug PJOrion has). We schedule work with
BigWorld.callback(0, ...) so it runs on the next game tick, then block the poll
thread on an Event until the result is ready.

Outside the game (no BigWorld) we just call inline so the agent stays testable.
"""

import threading

try:
    import BigWorld
    _HAS_BW = True
except ImportError:
    _HAS_BW = False


def run_on_main(fn, timeout=30.0):
    if not _HAS_BW:
        return fn()

    box = {}
    done = threading.Event()

    def wrapper():
        try:
            box['value'] = fn()
        except BaseException as exc:
            box['error'] = exc
        finally:
            done.set()

    BigWorld.callback(0.0, wrapper)
    if not done.wait(timeout):
        raise RuntimeError('main-thread execution timed out after %ss' % timeout)
    if 'error' in box:
        raise box['error']
    return box.get('value')
