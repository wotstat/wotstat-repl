"""Verify deep runtime introspection captures dynamic fields static can't see.

Mimics BigWorld: an instance whose attributes are injected at runtime (entitydef
style) and not present in the class source, plus a reference cycle.
Runs on py2.7 or 3.x.
"""

import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent import handlers


class Vector3(object):
    def __init__(self, x, y, z):
        self.x, self.y, self.z = x, y, z


class Avatar(object):
    """Static class body has NO 'position'/'health'; injected at runtime."""

    def stop(self):
        pass
    stop.__doc__ = 'stop() -> None'


def build_live_player():
    p = Avatar()
    # Runtime-injected fields (entitydef / components) invisible to static analysis:
    p.position = Vector3(1.0, 2.0, 3.0)
    p.health = 1500
    p.name = 'malder011'
    p.cell = None
    return p


def find(node, name):
    for m in node.get('members', []):
        if m['name'] == name:
            return m
    return None


def main():
    player = build_live_player()
    # cycle: player.cell -> back to player
    player.cell = player

    ns = sys.modules['__main__'].__dict__
    ns['player'] = player

    # Use a unicode expr: json.loads yields unicode on py2.7, which must NOT be
    # iterated char-by-char (regression for the "name 'B' is not defined" bug).
    out = handlers.handle_dump({
        'id': 'd', 'type': 'dump', 'expr': u'player', 'depth': 3, 'max_attrs': 50,
    })

    assert not out['errors'], out['errors']
    assert len(out['roots']) == 1, 'unicode expr must yield ONE root, not per-char: %r' % out
    root = out['roots'][0]
    assert root['type'] == 'Avatar', root

    # Dynamic fields are captured with their REAL runtime types:
    pos = find(root, 'position')
    assert pos and pos['type'] == 'Vector3', pos
    assert find(pos, 'x')['kind'] == 'value', pos        # nested field
    health = find(root, 'health')
    assert health and health['kind'] == 'value', health
    name = find(root, 'name')
    assert name and 'malder011' in name['repr'], name

    # Method keeps its typed signature from __doc__:
    stop = find(root, 'stop')
    assert stop and stop.get('signature') == 'stop() -> None', stop

    # Cycle is detected, not infinite-looped:
    cell = find(root, 'cell')
    assert cell and cell.get('cyclic'), cell

    import json
    json.dumps(out)  # must be JSON-serializable for the wire

    # Runtime-informed stub captures the injected fields (static can't):
    stub = out['stubs'].get('Avatar', '')
    assert 'class Avatar(object):' in stub, stub
    assert 'position = ...  # Vector3' in stub, stub
    assert 'health = ...  # int' in stub, stub
    assert 'def stop(' in stub, stub

    # "Dump all" collects a stub for EVERY distinct type, incl. the nested one:
    assert 'Vector3' in out['stubs'], list(out['stubs'])
    assert 'x = ...' in out['stubs']['Vector3'], out['stubs']['Vector3']

    if sys.version_info[0] >= 3:
        import ast
        ast.parse(stub, '<Avatar.pyi>')
        ast.parse(out['stubs']['Vector3'], '<Vector3.pyi>')

    print('DUMP OK -- position:%s health:%s cyclic-cell:%s; stub has injected fields' % (
        pos['type'], health['type'], cell.get('cyclic')))
    return 0


if __name__ == '__main__':
    sys.exit(main())
