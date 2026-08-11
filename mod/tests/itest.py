"""Integration test: drive the real agent loop like the desktop would.

Exercises the daemon poll thread, main-thread dispatch (inline without BigWorld),
stdout capture, shutdown frames, and namespace persistence across requests. Runs
on py2.7 or 3.x.
"""

import os
import sys
import time
import shutil
import tempfile

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent.framebus import FrameBus
import wms_agent


def main():
    work = tempfile.mkdtemp(prefix="wms_itest_")
    desktop = FrameBus(work, out_name="d2c", in_name="c2d")
    try:
        wms_agent.start(work, interval=0.02)

        desktop.send({"id": "1", "type": "exec", "code": "print('hello'); x = 40 + 2"})
        desktop.send({"id": "2", "type": "exec", "code": "x * 2"})

        results = {}
        stdout_text = ""
        hello = None
        deadline = time.time() + 3.0
        while time.time() < deadline and not (
                "2" in results and "hello" in stdout_text and hello):
            for frame in desktop.drain():
                if frame.get("type") == "stdout":
                    stdout_text += frame.get("text", "")
                elif frame.get("type") == "hello":
                    hello = frame
                elif frame.get("id"):
                    results[frame["id"]] = frame
            time.sleep(0.02)

        desktop.send({"type": "hello"})
        rehello = None
        rehello_deadline = time.time() + 1.0
        while time.time() < rehello_deadline and rehello is None:
            for frame in desktop.drain():
                if frame.get("type") == "hello":
                    rehello = frame
                    break
            time.sleep(0.02)

        assert rehello is not None, "agent should repeat hello for a reconnecting desktop"

        wms_agent.stop()  # restores stdout before we assert/print
        disconnected = False
        disconnected_deadline = time.time() + 1.0
        while time.time() < disconnected_deadline and not disconnected:
            disconnected = any(
                frame.get("type") == "disconnected"
                for frame in desktop.drain())
            time.sleep(0.02)

        assert results.get("2", {}).get("repr") == "84", results
        assert "hello" in stdout_text, repr(stdout_text)
        assert hello.get("version") == wms_agent.__version__, hello
        assert disconnected, "agent should send disconnected on stop"

        print("ITEST OK  ns-persist x*2=%s  captured=%r"
              % (results["2"]["repr"], stdout_text.strip()))
        return 0
    finally:
        try:
            wms_agent.stop()
        except Exception:
            pass
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
