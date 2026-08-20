"""Regression checks for rediscovery while the agent is unauthenticated."""

import errno
import json
import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.socketbus import SocketBus


class _UnreadableUdp(object):
    def recvfrom(self, _size):
        raise IOError(errno.EWOULDBLOCK, 'would block')


class _OfferingUdp(object):
    def __init__(self, offer):
        self._offer = offer

    def recvfrom(self, _size):
        if self._offer is None:
            raise IOError(errno.EWOULDBLOCK, 'would block')
        offer = self._offer
        self._offer = None
        return json.dumps(offer).encode('utf-8'), ('192.168.0.30', 8767)


class _OpenConnection(object):
    def __init__(self):
        self.closed = False

    def close(self):
        self.closed = True


def main():
    bus = SocketBus(os.path.dirname(__file__), 'test', 1)
    discoveries = []
    bus._udp = _UnreadableUdp()
    bus._socket = object()
    bus._authenticated = False
    bus._next_discovery_at = 0.0
    bus._send_discovery = lambda now: discoveries.append(now)

    bus._poll_discovery(42.0)

    assert discoveries == [42.0], (
        'an unauthenticated connection must not prevent periodic rediscovery',
        discoveries,
    )

    bus._token = 'configured-for-another-desktop'
    bus._hello_nonce = 'insecure-desktop'
    bus._accept_welcome({
        'type': 'welcome',
        'protocol': 1,
        'agent_id': bus._agent_id,
        'session': bus._session,
        'nonce': 'insecure-desktop',
        'server_id': 'insecure-desktop',
        'secure': False,
        'proof': '',
    })
    assert bus.authenticated, (
        'a token-configured agent must accept an explicitly insecure desktop')

    connection = _OpenConnection()
    bus._send_discovery = lambda _now: None
    bus._socket = connection
    bus._authenticated = False
    bus._discovered_endpoint = ('198.18.0.1', 8766)
    bus._discovery_nonce = 'new-endpoint'
    bus._udp = _OfferingUdp({
        'type': 'offer',
        'protocol': 1,
        'agent_id': bus._agent_id,
        'nonce': 'new-endpoint',
        'tcp_port': 8766,
        'secure': False,
    })
    bus._poll_discovery(43.0)

    assert connection.closed, 'stale unauthenticated TCP connection stayed open'
    assert bus._socket is None, bus._socket
    assert bus._discovered_endpoint == ('192.168.0.30', 8766), (
        bus._discovered_endpoint)
    assert bus._next_connect_at == 0.0, bus._next_connect_at
    print('SOCKETBUS RECONNECT OK')
    return 0


if __name__ == '__main__':
    sys.exit(main())
