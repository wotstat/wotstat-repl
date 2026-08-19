"""Small request-handler self-test (no BigWorld required, py2.7 or 3.x)."""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent import handlers
from wms_agent.socketbus import (
    DEFAULT_DISCOVERY_PORT,
    DEFAULT_TCP_PORT,
    _load_config,
)


def main():
    config_dir = tempfile.mkdtemp(prefix='wms_configless_')
    try:
        token, host, tcp_port, discovery_port = _load_config(config_dir)
        assert token is None
        assert host == 'auto'
        assert tcp_port == DEFAULT_TCP_PORT
        assert discovery_port == DEFAULT_DISCOVERY_PORT
    finally:
        shutil.rmtree(config_dir, ignore_errors=True)

    requests = [
        {"id": "1", "type": "exec", "code": "1 + 2"},
        {"id": "2", "type": "lint", "code": "print x\n"},
        {"id": "3", "type": "complete", "prefix": "in"},
    ]
    replies = {}
    for request in requests:
        reply = handlers.DISPATCH[request["type"]](request)
        replies[reply["id"]] = reply

    assert replies["1"]["repr"] == "3", replies["1"]
    assert replies["2"]["type"] == "lint", replies["2"]
    assert any(c["name"].startswith("in") for c in replies["3"]["candidates"]), replies["3"]

    print("OK  exec->%s  lint->%d diag  complete->%d cand" % (
        replies["1"]["repr"],
        len(replies["2"]["diagnostics"]),
        len(replies["3"]["candidates"]),
    ))
    return 0


if __name__ == "__main__":
    sys.exit(main())
