"""Schedule a callable on the game main thread without blocking the caller.

BigWorld is single threaded; touching entities/GUI from the agent's poll thread
can crash the client (this is the bug PJOrion has). We schedule work with
BigWorld.callback(0, ...) so it runs on the next game tick. Completion is handed
back through a callback so the agent's network thread remains free to service
heartbeats while the game is loading.

Outside the game (no BigWorld) we complete inline so the agent stays testable.
"""

import traceback

try:
    import BigWorld
    _HAS_BW = True
except ImportError:
    _HAS_BW = False


class DeferredMainResult(object):
    """A handler result that completes after additional main-thread ticks."""

    def __init__(self, start):
        self._start = start

    def start(self, completed):
        self._start(completed)


def schedule_on_main(fn, completed):
    def wrapper():
        try:
            result = fn()
            if isinstance(result, DeferredMainResult):
                result.start(completed)
            else:
                completed(result, None)
        except BaseException:
            completed(None, traceback.format_exc())

    if _HAS_BW:
        BigWorld.callback(0.0, wrapper)
    else:
        wrapper()
