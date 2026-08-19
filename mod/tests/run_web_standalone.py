"""Run the embedded web transport outside the game for manual UI testing.

    python run_web_standalone.py <web_dist> [port]
"""

import os
import sys
import time

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

import wms_agent


web_root = sys.argv[1] if len(sys.argv) > 1 else 'dist-web'
port = int(sys.argv[2]) if len(sys.argv) > 2 else 8768
endpoint = wms_agent.start(
    '.', web_enabled=True, web_root=web_root, web_port=port)
print('Web UI: %s' % endpoint)

try:
    while True:
        time.sleep(0.2)
except KeyboardInterrupt:
    pass
finally:
    wms_agent.stop()
