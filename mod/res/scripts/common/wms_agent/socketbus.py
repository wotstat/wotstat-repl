"""Reconnecting TCP transport for the in-game Python 2.7 agent.

The bus owns no threads. ``send`` only appends to a bounded in-memory backlog;
the agent poll thread calls ``poll`` to discover/connect, exchange frames, and
return desktop requests. Frames stay in memory until acknowledged, so a UI
started late or restarted receives the retained startup log without disk IPC.
When no config file exists, the bus uses the default ports and anonymous UDP
discovery; a desktop with Secure connection enabled rejects that connection.
"""

import collections
import errno
import hashlib
import hmac
import json
import os
import socket
import time
import uuid


PROTOCOL_VERSION = 1
DEFAULT_TCP_PORT = 8766
DEFAULT_DISCOVERY_PORT = 8767
MAX_FRAME_BYTES = 2 * 1024 * 1024
MAX_BACKLOG_BYTES = 8 * 1024 * 1024
CONNECT_INTERVAL = 1.0
DISCOVERY_INTERVAL = 1.0


def _proof(token, parts):
    message = '|'.join(str(part) for part in parts)
    if not isinstance(token, bytes):
        token = token.encode('utf-8')
    if not isinstance(message, bytes):
        message = message.encode('utf-8')
    return hmac.new(token, message, hashlib.sha256).hexdigest()


def _constant_time_equal(left, right):
    try:
        string_types = (basestring,)
    except NameError:
        string_types = (str,)
    if not isinstance(left, string_types) or not isinstance(right, string_types):
        return False
    compare = getattr(hmac, 'compare_digest', None)
    if compare is not None:
        try:
            return compare(left, right)
        except TypeError:
            pass
    if len(left) != len(right):
        return False
    result = 0
    for a, b in zip(bytearray(left.encode('ascii')), bytearray(right.encode('ascii'))):
        result |= a ^ b
    return result == 0


def _encode(frame):
    line = json.dumps(frame, ensure_ascii=True, separators=(',', ':')) + '\n'
    if not isinstance(line, bytes):
        line = line.encode('utf-8')
    if len(line) > MAX_FRAME_BYTES:
        raise ValueError('agent frame exceeds size limit')
    return line


def _load_config(directory):
    path = os.path.join(directory, 'agent-network.json')
    if not os.path.isfile(path):
        return None, 'auto', DEFAULT_TCP_PORT, DEFAULT_DISCOVERY_PORT
    handle = open(path, 'r')
    try:
        config = json.load(handle)
    finally:
        handle.close()
    token = config.get('token')
    if token is not None and not isinstance(token, str):
        try:
            string_types = (basestring,)
        except NameError:
            string_types = (str,)
        if not isinstance(token, string_types):
            raise ValueError('agent-network.json has an invalid token')
    if token == '':
        raise ValueError('agent-network.json has an empty token')
    host = config.get('host', 'auto')
    tcp_port = int(config.get('tcp_port', DEFAULT_TCP_PORT))
    discovery_port = int(config.get('discovery_port', DEFAULT_DISCOVERY_PORT))
    if not host or not 0 < tcp_port < 65536 or not 0 < discovery_port < 65536:
        raise ValueError('invalid agent-network.json endpoint')
    return token, host, tcp_port, discovery_port


class SocketBus(object):
    def __init__(self, config_dir, version, pid):
        self._token, self._host, self._tcp_port, self._discovery_port = _load_config(config_dir)
        self._secure_required = self._token is not None
        self._version = version
        self._pid = pid
        self._agent_id = uuid.uuid4().hex
        self._session = uuid.uuid4().hex
        self._next_seq = 1
        self._acked_seq = 0
        self._dropped_through = 0
        self._backlog = collections.deque()
        self._backlog_bytes = 0
        self._socket = None
        self._udp = None
        self._authenticated = False
        self._hello_nonce = None
        self._discovery_nonce = None
        self._discovered_endpoint = None
        self._received = b''
        self._write_buffer = b''
        self._controls = collections.deque()
        self._next_send_seq = 1
        self._next_connect_at = 0.0
        self._next_discovery_at = 0.0

    @property
    def authenticated(self):
        return self._authenticated

    @property
    def backlog_size(self):
        return len(self._backlog)

    def send(self, frame):
        frame = dict(frame)
        frame['session'] = self._session
        frame['seq'] = self._next_seq
        self._next_seq += 1
        encoded = _encode(frame)
        self._backlog.append((frame['seq'], encoded))
        self._backlog_bytes += len(encoded)
        self._trim_backlog()
        return True

    def poll(self):
        now = time.time()
        self._poll_discovery(now)
        if self._socket is None and now >= self._next_connect_at:
            self._connect(now)
        if self._socket is None:
            return []
        try:
            self._pump_write()
            incoming = self._pump_read()
            self._pump_write()
            return incoming
        except (IOError, OSError, ValueError):
            self._disconnect()
            return []

    def close(self, grace=0.2):
        deadline = time.time() + max(0.0, grace)
        while self._socket is not None and self._backlog and time.time() < deadline:
            self.poll()
            time.sleep(0.005)
        self._disconnect()
        if self._udp is not None:
            try:
                self._udp.close()
            except Exception:
                pass
            self._udp = None

    def _trim_backlog(self):
        while self._backlog and self._backlog_bytes > MAX_BACKLOG_BYTES:
            seq, encoded = self._backlog.popleft()
            self._backlog_bytes -= len(encoded)
            self._dropped_through = max(self._dropped_through, seq)
            if self._next_send_seq <= seq:
                self._next_send_seq = seq + 1

    def _connect(self, now):
        self._next_connect_at = now + CONNECT_INTERVAL
        endpoint = self._discovered_endpoint
        if endpoint is None:
            host = '127.0.0.1' if self._host == 'auto' else self._host
            endpoint = (host, self._tcp_port)
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            sock.settimeout(0.35)
            sock.connect(endpoint)
            sock.setblocking(False)
        except (IOError, OSError):
            try:
                sock.close()
            except Exception:
                pass
            if self._host == 'auto':
                self._send_discovery(now)
            return
        self._socket = sock
        self._authenticated = False
        self._received = b''
        self._write_buffer = b''
        self._controls.clear()
        self._next_send_seq = self._backlog[0][0] if self._backlog else self._next_seq
        self._hello_nonce = uuid.uuid4().hex
        hello = {
            'type': 'hello',
            'protocol': PROTOCOL_VERSION,
            'agent_id': self._agent_id,
            'session': self._session,
            'nonce': self._hello_nonce,
            'version': self._version,
            'pid': self._pid,
            'acked_seq': self._acked_seq,
            'dropped_through': self._dropped_through,
            'proof': (_proof(self._token, [
                'hello', PROTOCOL_VERSION, self._agent_id, self._session,
                self._hello_nonce,
            ]) if self._token is not None else ''),
        }
        self._controls.append(_encode(hello))

    def _disconnect(self):
        if self._socket is not None:
            try:
                self._socket.close()
            except Exception:
                pass
        self._socket = None
        self._authenticated = False
        self._received = b''
        self._write_buffer = b''
        self._controls.clear()
        self._next_send_seq = self._backlog[0][0] if self._backlog else self._next_seq
        self._next_connect_at = time.time() + CONNECT_INTERVAL

    def _pump_write(self):
        while self._socket is not None:
            if not self._write_buffer:
                if self._controls:
                    self._write_buffer = self._controls.popleft()
                elif self._authenticated:
                    encoded = self._next_backlog_frame()
                    if encoded is None:
                        return
                    self._write_buffer = encoded
                else:
                    return
            try:
                sent = self._socket.send(self._write_buffer)
            except (IOError, OSError) as error:
                if _would_block(error):
                    return
                raise
            if sent <= 0:
                raise IOError('agent socket closed while writing')
            self._write_buffer = self._write_buffer[sent:]

    def _next_backlog_frame(self):
        for seq, encoded in self._backlog:
            if seq >= self._next_send_seq:
                self._next_send_seq = seq + 1
                return encoded
        return None

    def _pump_read(self):
        while True:
            try:
                chunk = self._socket.recv(16384)
            except (IOError, OSError) as error:
                if _would_block(error):
                    break
                raise
            if not chunk:
                raise IOError('agent socket closed')
            self._received += chunk
            if len(self._received) > MAX_FRAME_BYTES and b'\n' not in self._received:
                raise ValueError('desktop frame exceeds size limit')

        requests = []
        while b'\n' in self._received:
            raw, self._received = self._received.split(b'\n', 1)
            if len(raw) > MAX_FRAME_BYTES:
                raise ValueError('desktop frame exceeds size limit')
            if not raw.strip():
                continue
            try:
                if not isinstance(raw, str):
                    raw = raw.decode('utf-8')
                frame = json.loads(raw)
            except (TypeError, ValueError, UnicodeError):
                raise ValueError('invalid desktop frame')
            kind = frame.get('type')
            if kind == 'welcome':
                self._accept_welcome(frame)
            elif not self._authenticated:
                raise ValueError('desktop sent data before authentication')
            elif kind == 'ack':
                self._ack(frame)
            elif kind == 'ping':
                self._controls.append(_encode({'type': 'pong'}))
            else:
                requests.append(frame)
        return requests

    def _accept_welcome(self, frame):
        if self._authenticated:
            return
        if (frame.get('protocol') != PROTOCOL_VERSION
                or frame.get('agent_id') != self._agent_id
                or frame.get('session') != self._session
                or frame.get('nonce') != self._hello_nonce):
            raise ValueError('invalid desktop welcome')
        secure = bool(frame.get('secure', True))
        if self._secure_required and not secure:
            raise ValueError('desktop attempted to downgrade secure connection')
        if secure:
            if self._token is None:
                raise ValueError('desktop requires agent token')
            expected = _proof(self._token, [
                'welcome', PROTOCOL_VERSION, self._agent_id, self._session,
                self._hello_nonce, frame.get('server_id', ''),
            ])
            if not _constant_time_equal(expected, frame.get('proof', '')):
                raise ValueError('desktop authentication failed')
        self._authenticated = True

    def _ack(self, frame):
        if frame.get('session') != self._session:
            return
        try:
            acknowledged = int(frame.get('seq'))
        except (TypeError, ValueError):
            return
        while self._backlog and self._backlog[0][0] <= acknowledged:
            _seq, encoded = self._backlog.popleft()
            self._backlog_bytes -= len(encoded)
        self._acked_seq = max(self._acked_seq, acknowledged)
        if self._next_send_seq <= acknowledged:
            self._next_send_seq = acknowledged + 1

    def _ensure_udp(self):
        if self._udp is not None:
            return True
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
            sock.bind(('', 0))
            sock.setblocking(False)
            self._udp = sock
            return True
        except (IOError, OSError):
            return False

    def _send_discovery(self, now):
        if now < self._next_discovery_at or not self._ensure_udp():
            return
        self._next_discovery_at = now + DISCOVERY_INTERVAL
        self._discovery_nonce = uuid.uuid4().hex
        request = {
            'type': 'discover',
            'protocol': PROTOCOL_VERSION,
            'agent_id': self._agent_id,
            'nonce': self._discovery_nonce,
            'proof': (_proof(self._token, [
                'discover', self._agent_id, self._discovery_nonce,
            ]) if self._token is not None else ''),
        }
        try:
            self._udp.sendto(_encode(request).rstrip(b'\n'),
                             ('255.255.255.255', self._discovery_port))
        except (IOError, OSError):
            pass

    def _poll_discovery(self, now):
        if self._host != 'auto':
            return
        if self._udp is None:
            if self._socket is None and now >= self._next_discovery_at:
                self._send_discovery(now)
            return
        while True:
            try:
                body, peer = self._udp.recvfrom(4096)
            except (IOError, OSError) as error:
                if _would_block(error):
                    return
                return
            try:
                if not isinstance(body, str):
                    body = body.decode('utf-8')
                offer = json.loads(body)
                port = int(offer.get('tcp_port'))
            except (TypeError, ValueError, UnicodeError):
                continue
            if (offer.get('type') != 'offer'
                    or offer.get('protocol') != PROTOCOL_VERSION
                    or offer.get('agent_id') != self._agent_id
                    or offer.get('nonce') != self._discovery_nonce):
                continue
            secure = bool(offer.get('secure', True))
            if self._secure_required and not secure:
                continue
            if secure:
                if self._token is None:
                    continue
                expected = _proof(self._token, [
                    'offer', self._agent_id, self._discovery_nonce, port,
                    offer.get('server_id', ''),
                ])
                if not _constant_time_equal(expected, offer.get('proof', '')):
                    continue
            self._discovered_endpoint = (peer[0], port)
            self._next_connect_at = 0.0
            return


def _would_block(error):
    code = getattr(error, 'errno', None)
    return code in (
        errno.EAGAIN,
        errno.EWOULDBLOCK,
        getattr(errno, 'WSAEWOULDBLOCK', 10035),
    )
