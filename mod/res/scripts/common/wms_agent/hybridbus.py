"""Route one REPL core across the desktop TCP and embedded web transports.

Async frames such as captured stdout are broadcast to every available
transport. Request IDs are replaced internally so even identical IDs arriving
from different transports cannot collide; correlated replies are restored and
sent only to the transport that originated the request.
"""

import uuid


class HybridBus(object):
    def __init__(self, tcp_bus, web_bus=None, web_error=None):
        self._transports = [tcp_bus]
        if web_bus is not None:
            self._transports.append(web_bus)
        self._routes = {}
        self.endpoint = getattr(web_bus, 'endpoint', None)
        self.web_error = web_error

    def poll(self):
        requests = []
        for transport in self._transports:
            for incoming in transport.poll():
                frame = dict(incoming)
                wire_id = frame.get('id')
                if wire_id is not None:
                    internal_id = uuid.uuid4().hex
                    self._routes[internal_id] = (transport, wire_id)
                    frame['id'] = internal_id
                requests.append(frame)
        return requests

    def send(self, frame):
        correlation_id = frame.get('id')
        if correlation_id is not None:
            route = self._routes.pop(correlation_id, None)
            if route is None:
                return False
            transport, wire_id = route
            outgoing = dict(frame)
            outgoing['id'] = wire_id
            return transport.send(outgoing)

        delivered = False
        for transport in self._transports:
            try:
                delivered = transport.send(frame) or delivered
            except Exception:
                pass
        return delivered

    def close(self, grace=0.2):
        self._routes.clear()
        for transport in self._transports:
            try:
                transport.close(grace)
            except Exception:
                pass
