"""Network integration test for the real agent loop (Python 2.7 or 3.x).

Starts the game-side agent before the fake desktop begins listening, verifies
that startup output is retained in RAM, authenticates the TCP session, executes
two correlated requests, and observes a clean disconnect.
"""

import json
import os
import shutil
import socket
import sys
import tempfile
import time

try:
    from urllib2 import Request, urlopen
except ImportError:  # pragma: no cover - Python 3 compatibility
    from urllib.request import Request, urlopen

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

import wms_agent
from wms_agent.socketbus import PROTOCOL_VERSION, _proof


TOKEN = '00000000-0000-4000-8000-000000000001'


def send_frame(sock, frame):
    body = json.dumps(frame, separators=(',', ':')) + '\n'
    if not isinstance(body, bytes):
        body = body.encode('utf-8')
    sock.sendall(body)


def post_json(url, origin, frame):
    body = json.dumps(frame, separators=(',', ':'))
    if not isinstance(body, bytes):
        body = body.encode('utf-8')
    request = Request(url, data=body, headers={
        'Content-Type': 'application/json',
        'Origin': origin,
    })
    response = urlopen(request, timeout=4.0)
    try:
        return json.loads(response.read())
    finally:
        response.close()


def receive_frame(sock, received, deadline):
    while time.time() < deadline:
        if b'\n' in received[0]:
            raw, received[0] = received[0].split(b'\n', 1)
            if not isinstance(raw, str):
                raw = raw.decode('utf-8')
            return json.loads(raw)
        try:
            chunk = sock.recv(16384)
        except socket.timeout:
            continue
        if not chunk:
            return None
        received[0] += chunk
    return None


def authenticate(client, received, server_id, deadline):
    hello = receive_frame(client, received, deadline)
    assert hello and hello.get('type') == 'hello', hello
    assert hello.get('proof') == _proof(TOKEN, [
        'hello', PROTOCOL_VERSION, hello['agent_id'], hello['session'],
        hello['nonce'],
    ]), hello
    send_frame(client, {
        'type': 'welcome',
        'protocol': PROTOCOL_VERSION,
        'agent_id': hello['agent_id'],
        'session': hello['session'],
        'nonce': hello['nonce'],
        'server_id': server_id,
        'proof': _proof(TOKEN, [
            'welcome', PROTOCOL_VERSION, hello['agent_id'], hello['session'],
            hello['nonce'], server_id,
        ]),
    })
    return hello


def main():
    work = tempfile.mkdtemp(prefix='wms_network_itest_')
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    client = None
    try:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
        with open(os.path.join(work, 'agent-network.json'), 'w') as handle:
            json.dump({
                'token': TOKEN,
                'host': '127.0.0.1',
                'tcp_port': port,
                'discovery_port': port,
            }, handle)

        # Game first: the initial connection is refused, but captured output
        # remains in the agent's bounded RAM backlog.
        web_endpoint = wms_agent.start(
            work, interval=0.01, web_enabled=True, web_root=work, web_port=0)
        assert web_endpoint and web_endpoint.startswith('http://127.0.0.1:'), web_endpoint
        print('early-startup-line')
        time.sleep(0.08)

        listener.listen(1)
        listener.settimeout(4.0)
        client, _peer = listener.accept()
        client.settimeout(0.2)
        received = [b'']
        deadline = time.time() + 4.0
        server_id = 'desktop-test'
        hello = authenticate(client, received, server_id, deadline)

        # Drop the desktop before acknowledging the startup frame. The same
        # session/sequence must be replayed after the agent reconnects.
        early = receive_frame(client, received, deadline)
        assert early and early.get('type') == 'stdout', early
        assert 'early-startup-line' in early.get('text', ''), early
        early_seq = early['seq']
        client.close()
        client = None

        client, _peer = listener.accept()
        client.settimeout(0.2)
        received = [b'']
        reconnect_deadline = time.time() + 4.0
        rehello = authenticate(client, received, server_id, reconnect_deadline)
        assert rehello['session'] == hello['session'], (hello, rehello)
        replayed = receive_frame(client, received, reconnect_deadline)
        assert replayed and replayed.get('seq') == early_seq, replayed
        assert 'early-startup-line' in replayed.get('text', ''), replayed
        send_frame(client, {
            'type': 'ack', 'session': hello['session'], 'seq': early_seq,
        })

        web_result = post_json(
            web_endpoint + 'api/repl', web_endpoint.rstrip('/'), {
                'type': 'exec', 'code': 'shared_from_web = 21',
            })
        assert web_result.get('ok'), web_result

        send_frame(client, {'id': '1', 'type': 'exec',
                            'code': "print('hello'); x = 40 + 2"})
        send_frame(client, {
            'id': '2', 'type': 'exec', 'code': 'shared_from_web * 4',
        })

        results = {}
        stdout_text = replayed.get('text', '')
        deadline = time.time() + 4.0
        while time.time() < deadline and not (
                results.get('2', {}).get('repr') == '84'
                and 'early-startup-line' in stdout_text
                and 'hello' in stdout_text):
            frame = receive_frame(client, received, deadline)
            if frame is None:
                break
            if frame.get('type') == 'pong':
                continue
            seq = frame.get('seq')
            assert frame.get('session') == hello['session'], frame
            assert isinstance(seq, int), frame
            send_frame(client, {
                'type': 'ack', 'session': hello['session'], 'seq': seq,
            })
            if frame.get('type') == 'stdout':
                stdout_text += frame.get('text', '')
            elif frame.get('id'):
                results[frame['id']] = frame

        assert results.get('2', {}).get('repr') == '84', results
        assert 'early-startup-line' in stdout_text, repr(stdout_text)
        assert 'hello' in stdout_text, repr(stdout_text)

        wms_agent.stop()
        disconnected = False
        disconnect_deadline = time.time() + 1.0
        while time.time() < disconnect_deadline:
            frame = receive_frame(client, received, disconnect_deadline)
            if frame is None:
                break
            if frame.get('type') == 'disconnected':
                disconnected = True
                break
        assert disconnected, 'agent should send disconnected before closing TCP'

        print("ITEST OK  late-ui backlog=%r  shared web->tcp=%s" % (
            'early-startup-line', results['2']['repr']))
        return 0
    finally:
        try:
            wms_agent.stop()
        except Exception:
            pass
        if client is not None:
            client.close()
        listener.close()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
