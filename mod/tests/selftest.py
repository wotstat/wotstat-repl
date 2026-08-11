"""Loopback self-test for the framebus + handlers (no BigWorld required).

Runs on Python 2.7 or 3.x. Simulates the desktop side writing requests and the
agent side draining, dispatching, and replying, all through the real FrameBus.
"""

import os
import sys
import shutil
import tempfile

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.framebus import FrameBus
from wms_agent import handlers


def main():
    work = tempfile.mkdtemp(prefix="wms_selftest_")
    try:
        agent = FrameBus(work, out_name="c2d", in_name="d2c")
        desktop = FrameBus(work, out_name="d2c", in_name="c2d")

        desktop.send({"id": "1", "type": "exec", "code": "1 + 2"})
        desktop.send({"id": "2", "type": "lint", "code": "print x\n"})
        desktop.send({"id": "3", "type": "complete", "prefix": "in"})

        for req in agent.drain():
            handler = handlers.DISPATCH[req["type"]]
            agent.send(handler(req))

        replies = {r["id"]: r for r in desktop.drain()}

        assert replies["1"]["repr"] == "3", replies["1"]
        assert replies["2"]["type"] == "lint", replies["2"]
        assert any(c["name"].startswith("in") for c in replies["3"]["candidates"]), replies["3"]

        # py2.7 SyntaxError on "print x" only when this runs under py2; under py3
        # the static lint still returns a well-formed (possibly empty) frame.
        print("OK  exec->%s  lint->%d diag  complete->%d cand" % (
            replies["1"]["repr"],
            len(replies["2"]["diagnostics"]),
            len(replies["3"]["candidates"]),
        ))
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
