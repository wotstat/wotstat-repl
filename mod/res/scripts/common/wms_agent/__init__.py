"""WotStat REPL in-game agent (Python 2.7 / BigWorld).

Loaded inside the running WoT client by the bw_site loader. Exposes a tiny
RPC interface that streams captured stdout/log output and answers
exec/complete/inspect/lint requests on the game main thread. The universal
release serves the embedded web UI while also connecting to the desktop over
the existing TCP transport.

Public API:
    start(config_dir)  -> begin serving and return an optional web URL
    stop()             -> restore stdout and stop the loop
"""

__version__ = '{{VERSION}}'
__web_enabled__ = '{{WEB_ENABLED}}' == '1'

from .loop import start, stop

__all__ = ['start', 'stop']
