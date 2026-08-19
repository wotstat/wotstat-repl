"""Routing self-test for the simultaneous TCP + web transport."""

import os
import shutil
import socket
import sys
import tempfile

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.hybridbus import HybridBus
from wms_agent.loop import _Agent


class FakeBus(object):
    def __init__(self, incoming=None, endpoint=None):
        self.incoming = list(incoming or [])
        self.sent = []
        self.closed = False
        self.endpoint = endpoint

    def poll(self):
        incoming = self.incoming
        self.incoming = []
        return incoming

    def send(self, frame):
        self.sent.append(dict(frame))
        return True

    def close(self, _grace=0.2):
        self.closed = True


def main():
    tcp = FakeBus([{'id': 'same', 'type': 'exec', 'code': '1'}])
    web = FakeBus(
        [{'id': 'same', 'type': 'exec', 'code': '2'}],
        'http://127.0.0.1:8768/',
    )
    bus = HybridBus(tcp, web)

    requests = bus.poll()
    assert len(requests) == 2, requests
    assert requests[0]['id'] != requests[1]['id'], requests
    assert bus.endpoint == 'http://127.0.0.1:8768/'

    bus.send({'id': requests[0]['id'], 'type': 'result', 'repr': '1'})
    assert tcp.sent == [{'id': 'same', 'type': 'result', 'repr': '1'}], tcp.sent
    assert web.sent == [], web.sent

    bus.send({'id': requests[1]['id'], 'type': 'result', 'repr': '2'})
    assert web.sent == [{'id': 'same', 'type': 'result', 'repr': '2'}], web.sent

    event = {'type': 'stdout', 'text': 'shared log'}
    bus.send(event)
    assert tcp.sent[-1] == event, tcp.sent
    assert web.sent[-1] == event, web.sent

    bus.close()
    assert tcp.closed and web.closed

    # A second game process can find 8768 occupied. The universal agent must
    # retain its TCP transport instead of failing startup altogether.
    work = tempfile.mkdtemp(prefix='wms_hybrid_fallback_')
    occupied = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        occupied.bind(('127.0.0.1', 0))
        occupied.listen(1)
        agent = _Agent(
            work, 0.01, True, work, occupied.getsockname()[1])
        assert agent._bus.endpoint is None
        assert agent._bus.web_error
        assert len(agent._bus._transports) == 1
        agent._bus.close()
    finally:
        occupied.close()
        shutil.rmtree(work, ignore_errors=True)

    print('HYBRID OK -- routed replies, shared events, TCP-only fallback')
    return 0


if __name__ == '__main__':
    sys.exit(main())
