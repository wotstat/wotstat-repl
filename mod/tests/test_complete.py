"""Verify live completion attaches typed signatures (py2.7 or 3.x)."""

import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent import handlers


def main():
    ns = sys.modules['__main__'].__dict__

    def spaceLoadStatus(*a, **k):
        pass
    spaceLoadStatus.__doc__ = 'spaceLoadStatus(distance: float = -1.0) -> float'

    class Avatar(object):
        pass

    ns['spaceLoadStatus'] = spaceLoadStatus

    def pythonOnly(required, optional=3, *args, **kwargs):
        pass
    ns['pythonOnly'] = pythonOnly

    class LiveAvatar(object):
        def move(self, distance, speed=1):
            pass
    ns['liveAvatar'] = LiveAvatar()

    ns['Avatar'] = Avatar
    ns['spaceConst'] = 42

    def player():
        pass
    player.__doc__ = 'player() -> Avatar'

    def playRenderer():
        pass
    playRenderer.__doc__ = 'playRenderer() -> object'

    class BigWorld(object):
        pass
    BigWorld.player = staticmethod(player)
    BigWorld.playRenderer = staticmethod(playRenderer)
    for i in range(300):
        setattr(BigWorld, 'a%03d' % i, i)
    ns['BigWorld'] = BigWorld
    for i in range(300):
        ns['global%03d' % i] = i
    ns['zzLastGlobal'] = True

    out = handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'space'})
    by_name = dict((c['name'], c) for c in out['candidates'])

    fn = by_name.get('spaceLoadStatus')
    assert fn, by_name
    assert fn.get('signature') == '(distance: float = -1.0) -> float', fn
    assert fn.get('kind') == 'function', fn

    python_out = handlers.handle_complete({
        'id': 'c', 'type': 'complete', 'prefix': u'pythonO',
    })
    python_fn = dict((c['name'], c) for c in python_out['candidates'])['pythonOnly']
    assert python_fn.get('signature') == '(required, optional=3, *args, **kwargs)', python_fn

    method_out = handlers.handle_complete({
        'id': 'c', 'type': 'complete', 'prefix': u'liveAvatar.mo',
    })
    method = dict((c['name'], c) for c in method_out['candidates'])['move']
    assert method.get('signature') == '(distance, speed=1)', method

    # Repeated requests reuse both expensive runtime operations.
    handlers._DIR_CACHE.clear()
    builtin_dir = dir
    dir_calls = []
    handlers.dir = lambda obj: (dir_calls.append(obj), builtin_dir(obj))[1]
    try:
        handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'liveAvatar.mo'})
        handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'liveAvatar.mo'})
    finally:
        del handlers.dir
    assert len(dir_calls) == 1, dir_calls

    def cacheOnly(value):
        pass
    ns['cacheOnly'] = cacheOnly
    handlers._SIGNATURE_CACHE.clear()
    original_inspect_signature = handlers._inspect_signature
    signature_calls = []
    handlers._inspect_signature = lambda obj: (
        signature_calls.append(obj), original_inspect_signature(obj)
    )[1]
    try:
        handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'cacheOnly'})
        handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'cacheOnly'})
    finally:
        handlers._inspect_signature = original_inspect_signature
    assert len(signature_calls) == 1, signature_calls

    const = by_name.get('spaceConst')
    assert const and const.get('kind') == 'int', const

    out2 = handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u'Avat'})
    cls = dict((c['name'], c) for c in out2['candidates']).get('Avatar')
    assert cls and cls.get('kind') == 'class', cls

    out3 = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                     'prefix': u'BigWorld.'})
    members = dict((c['name'], c) for c in out3['candidates'])
    assert 'player' in members, 'BigWorld.player missing from %d candidates' % len(members)

    fuzzy = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                      'prefix': u'BigWorld.pr'})
    fuzzy_names = set(c['name'] for c in fuzzy['candidates'])
    assert 'player' in fuzzy_names, fuzzy_names

    typo = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                     'prefix': u'BigWorld.pal'})
    assert 'player' in set(c['name'] for c in typo['candidates']), typo

    weak = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                     'prefix': u'BigWorld.la'})
    assert 'player' not in set(c['name'] for c in weak['candidates']), weak

    resolved = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                         'prefix': u'BigWorld.player'})
    player_candidate = dict((c['name'], c) for c in resolved['candidates'])['player']
    assert player_candidate.get('signature') == '() -> Avatar', player_candidate

    resolved_one = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                              'prefix': u'BigWorld.player', 'budget': 1})
    player_one = dict((c['name'], c) for c in resolved_one['candidates'])['player']
    assert player_one.get('signature') == '() -> Avatar', resolved_one

    out4 = handlers.handle_complete({'id': 'c', 'type': 'complete', 'prefix': u''})
    globals_ = dict((c['name'], c) for c in out4['candidates'])
    assert 'zzLastGlobal' in globals_, 'last global missing from %d candidates' % len(globals_)

    limited = handlers.handle_complete({'id': 'c', 'type': 'complete',
                                        'prefix': u'global', 'budget': 1})
    described = [c for c in limited['candidates'] if c.get('kind')]
    assert len(described) == 1, described

    print('COMPLETE OK -- native=%r python=%r method=%r caches=dir:%d/sig:%d kind(const)=%s kind(cls)=%s' % (
        fn['signature'], python_fn['signature'], method['signature'], len(dir_calls),
        len(signature_calls), const['kind'], cls['kind']))
    return 0


if __name__ == '__main__':
    sys.exit(main())
