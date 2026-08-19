"""WotStat REPL in-game agent (Python 2.7 / BigWorld).

Loaded inside the running WoT client by the bw_site loader. Exposes a tiny
authenticated TCP RPC the desktop app drives: it streams captured stdout/log
output and answers exec/complete/inspect/lint requests on the game main thread.

Public API:
    start(config_dir)  -> begin serving
    stop()             -> restore stdout and stop the loop
"""

__version__ = '{{VERSION}}'

from .loop import start, stop

__all__ = ['start', 'stop']
