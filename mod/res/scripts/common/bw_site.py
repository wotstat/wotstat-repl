# Fuflo WoT REPL early loader (Python 2.7 / BigWorld).
#
# Installed as scripts/common/bw_site.py via the .mtmod res overlay, so it loads
# at site-init time (the earliest point, like PJOrion) and its stdout/BigWorld.log
# hooks catch ALL logs from the first line. It then defers to the game's original
# bw_site so client startup is unaffected.

import os

try:
    import wms_agent
    _base = os.environ.get('LOCALAPPDATA') or os.environ.get('APPDATA') or os.getcwd()
    wms_agent.start(os.path.join(_base, 'FufloWoTREPL', 'buffer'))
    print 'Fuflo WoT REPL: agent started'
except Exception:
    import traceback
    print 'Fuflo WoT REPL: agent failed to start'
    print traceback.format_exc()

# Defer to the original bw_site.pyc shipped inside the game's script package.
try:
    from zipfile import ZipFile
    from marshal import loads as _loads
    _z = ZipFile('./res/packages/scripts.pkg', 'r')
    try:
        _f = _z.open('scripts/common/bw_site.pyc')
        try:
            _original = _loads(_f.read()[8:])
        finally:
            _f.close()
    finally:
        _z.close()
    exec _original in globals()
except Exception:
    import traceback
    print 'Fuflo WoT REPL: original bw_site load failed'
    print traceback.format_exc()

# Preserve the game's shutdown hook while letting the agent tell the desktop
# about a normal client exit before the original cleanup runs.
_agent = globals().get('wms_agent')
_original_fini = globals().get('fini')

def fini():
    try:
        if _agent is not None:
            _agent.stop()
    finally:
        if _original_fini is not None:
            _original_fini()
