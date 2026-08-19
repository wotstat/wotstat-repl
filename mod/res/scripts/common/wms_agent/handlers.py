"""Request handlers. Each takes a request dict and returns a response dict.

Game-touching handlers (exec/complete/inspect) are marshaled to the main
thread by the loop; lint is pure and runs anywhere.
"""

import os
import sys
import inspect
import time
import traceback
from collections import OrderedDict

from . import __version__

_DEFAULT_COMPLETION_BUDGET = 120
_MAX_COMPLETION_BUDGET = 10000
_CACHE_LIMIT = 2048
_DIR_CACHE_TTL = 1.0
_CACHE_MISS = object()
_DIR_CACHE = OrderedDict()
_SIGNATURE_CACHE = OrderedDict()


def _ns():
    return sys.modules['__main__'].__dict__


def seed_namespace():
    ns = _ns()
    for name in ('BigWorld', 'ResMgr', 'Math', 'Account'):
        try:
            ns[name] = __import__(name)
        except BaseException:
            pass


def handle_hello(req):
    return {'type': 'hello', 'version': __version__, 'pid': os.getpid()}


class _ExecTee(object):
    """Best-effort capture; sys streams are process-global, not thread-local."""

    def __init__(self, original):
        self._original = original
        self._parts = []

    def write(self, text):
        try:
            string_types = (basestring,)
        except NameError:
            string_types = (str,)
        if not isinstance(text, string_types):
            raise TypeError('write() argument must be a string')
        result = None
        if self._original is not None:
            result = self._original.write(text)
        self._parts.append(text)
        return result

    def flush(self):
        if self._original is not None:
            return self._original.flush()

    def getvalue(self):
        try:
            return ''.join(self._parts)
        except (TypeError, UnicodeError):
            try:
                text_type = unicode
            except NameError:
                text_type = str
            parts = []
            for part in self._parts:
                if isinstance(part, text_type):
                    parts.append(part)
                else:
                    parts.append(part.decode('utf-8', 'replace'))
            return u''.join(parts)

    def __getattr__(self, name):
        return getattr(self._original, name)


def handle_exec(req):
    code = req.get('code', '')
    ns = _ns()
    out = {'id': req.get('id'), 'type': 'result', 'ok': True,
           'repr': None, 'exc': None, 'stdout': '', 'stderr': ''}
    saved_out = sys.stdout
    saved_err = sys.stderr
    captured_out = _ExecTee(saved_out)
    captured_err = _ExecTee(saved_err)
    sys.stdout = captured_out
    sys.stderr = captured_err
    try:
        try:
            compiled = compile(code, '<repl>', 'eval')
            value = eval(compiled, ns)
            if value is not None:
                out['repr'] = repr(value)
        except SyntaxError:
            exec(compile(code, '<repl>', 'exec'), ns)
    except BaseException:
        out['ok'] = False
        out['exc'] = traceback.format_exc()
    finally:
        sys.stdout = saved_out
        sys.stderr = saved_err
        out['stdout'] = captured_out.getvalue()
        out['stderr'] = captured_err.getvalue()
        # Executed code may add or remove attributes from live objects.
        _DIR_CACHE.clear()
    return out


def _split_prefix(line):
    """Split a line-prefix into (base_expr, partial_attr).

    'print BigWorld.player().g' -> ('BigWorld.player()', 'g')
    'BigWorld.'                 -> ('BigWorld', '')
    'spaceL'                    -> (None, 'spaceL')   # bare/global completion

    Walks back over a balanced primary expression so CALLS and subscripts work
    (rlcompleter's regex can't handle '()' and gives nothing after a call).
    """
    i = len(line)
    while i > 0 and (line[i - 1].isalnum() or line[i - 1] == '_'):
        i -= 1
    partial = line[i:]
    if i == 0 or line[i - 1] != '.':
        return None, partial
    end = i - 1  # index of the separating '.'
    depth = 0
    k = end - 1
    while k >= 0:
        ch = line[k]
        if ch in ')]}':
            depth += 1
        elif ch in '([{':
            if depth == 0:
                break
            depth -= 1
        elif depth == 0 and not (ch.isalnum() or ch == '_' or ch == '.'):
            break
        k -= 1
    base = line[k + 1:end].strip()
    return (base or None), partial


def _describe_into(cand, name, obj):
    if inspect.isclass(obj):
        cand['kind'] = 'class'
    elif callable(obj):
        cand['kind'] = 'function'
    else:
        cand['kind'] = type(obj).__name__
    if inspect.isclass(obj) or callable(obj):
        signature = _signature(name, obj)
        if signature is not None:
            cand['signature'] = signature
    doc = getattr(obj, '__doc__', None)
    if doc:
        cand['doc'] = _short(doc, 200)


def _builtins_ns():
    try:
        import __builtin__ as b  # py2
    except ImportError:
        import builtins as b  # py3
    return b


def _cache_get(cache, key):
    try:
        value = cache.pop(key)
    except KeyError:
        return _CACHE_MISS
    cache[key] = value
    return value


def _cache_put(cache, key, value):
    try:
        cache.pop(key)
    except KeyError:
        pass
    cache[key] = value
    while len(cache) > _CACHE_LIMIT:
        cache.popitem(last=False)


def _cached_attrs(obj):
    """Cache the expensive dir() walk briefly without retaining the live object."""
    key = (id(obj), id(type(obj)))
    now = time.time()
    cached = _cache_get(_DIR_CACHE, key)
    if cached is not _CACHE_MISS and now - cached[0] <= _DIR_CACHE_TTL:
        return cached[1]
    try:
        attrs = tuple(sorted(dir(obj)))
    except Exception:
        attrs = ()
    _cache_put(_DIR_CACHE, key, (now, attrs))
    return attrs


def _iter_public_attrs(obj):
    """Yield public attribute names from a short-lived dir() cache."""
    attrs = _cached_attrs(obj)
    for attr in attrs:
        if attr.startswith('__'):
            continue
        yield attr


_MONACO_SEPARATORS = u'_-. /\\\'":$<>()[]{}'


def _strong_match_start(word, index):
    if index == 0:
        return True
    char = word[index]
    previous = word[index - 1]
    if char != char.lower() and previous == previous.lower():
        return True
    if char in _MONACO_SEPARATORS and previous not in _MONACO_SEPARATORS:
        return True
    return previous in _MONACO_SEPARATORS or previous in u' \t'


def _fuzzy_subsequence(pattern, word):
    """Mirror Monaco's default fuzzy candidate gate; Monaco still owns ranking."""
    pattern = pattern[:128]
    word_low = word[:128].lower()
    pattern_low = pattern.lower()
    if not pattern_low:
        return True
    start = 0
    while True:
        first = word_low.find(pattern_low[0], start)
        if first < 0:
            return False
        if _strong_match_start(word[:128], first):
            position = first + 1
            for char in pattern_low[1:]:
                position = word_low.find(char, position)
                if position < 0:
                    break
                position += 1
            else:
                return True
        start = first + 1


def _matches_completion(pattern, word):
    if _fuzzy_subsequence(pattern, word):
        return True
    if len(pattern) >= 3:
        for index in range(1, min(7, len(pattern) - 1)):
            swapped = pattern[:index] + pattern[index + 1] + pattern[index] + pattern[index + 2:]
            if _fuzzy_subsequence(swapped, word):
                return True
    return False


def _complete_names(names, partial, ns, budget):
    """Build candidate list from an iterable of names, consuming from budget[0]."""
    candidates = []
    for name in sorted(names, key=lambda value: (value != partial, value)):
        if name.startswith('__'):
            continue
        if not _matches_completion(partial, name):
            continue
        cand = {'name': name}
        if budget[0] > 0:
            budget[0] -= 1
            try:
                _describe_into(cand, name, eval(name, ns))
            except BaseException:
                pass
        candidates.append(cand)
    return candidates


def _completion_budget(req):
    try:
        value = int(req.get('budget', _DEFAULT_COMPLETION_BUDGET))
    except (TypeError, ValueError, OverflowError):
        value = _DEFAULT_COMPLETION_BUDGET
    return min(max(value, 0), _MAX_COMPLETION_BUDGET)


def handle_complete(req):
    out = {'id': req.get('id'), 'type': 'complete', 'candidates': []}
    ns = _ns()
    line = req.get('prefix', '')
    base, partial = _split_prefix(line)
    budget = [_completion_budget(req)]

    if base is not None:
        # Evaluate the base expression on the live object and dir() it. This
        # handles attribute access AFTER calls/subscripts (BigWorld.player().)
        # which rlcompleter cannot. NOTE: evaluating the base CALLS it (e.g.
        # player()) -- acceptable for a dev REPL on the game main thread.
        try:
            obj = eval(base, ns)
        except BaseException:
            return out
        for attr in sorted(_iter_public_attrs(obj),
                           key=lambda value: (value != partial, value)):
            if not _matches_completion(partial, attr):
                continue
            cand = {'name': attr}
            if budget[0] > 0:
                budget[0] -= 1
                try:
                    _describe_into(cand, attr, getattr(obj, attr))
                except BaseException:
                    pass
            out['candidates'].append(cand)
        return out

    # Bare / global completion: namespace names + builtins.
    names = set(ns.keys())
    names.update(dir(_builtins_ns()))
    out['candidates'] = _complete_names(names, partial, ns, budget)
    return out


def handle_inspect(req):
    out = {'id': req.get('id'), 'type': 'inspect',
           'signature': None, 'doc': None}
    try:
        obj = eval(req.get('expr', ''), _ns())
    except BaseException:
        return out
    try:
        out['doc'] = inspect.getdoc(obj)
    except Exception:
        pass
    expr = req.get('expr', '')
    name = expr.rsplit('.', 1)[-1]
    signature = _signature(name, obj)
    if signature is not None:
        out['signature'] = expr + signature
    return out


def handle_lint(req):
    out = {'id': req.get('id'), 'type': 'lint', 'diagnostics': []}
    try:
        compile(req.get('code', ''), '<repl>', 'exec')
    except SyntaxError as exc:
        out['diagnostics'].append({
            'line': exc.lineno or 1,
            'col': exc.offset or 1,
            'severity': 'error',
            'message': exc.msg,
        })
    return out


def _doc_signature(name, obj):
    """Recover a typed signature from the object's docstring.

    pybind11 and BigWorld's PY_AUTO wrappers emit the full typed signature as the
    first line of __doc__, e.g. "spaceLoadStatus(distance: float = -1.0) -> float".
    Returns the parenthesized params + optional return, or None.
    """
    doc = getattr(obj, '__doc__', None)
    if not doc:
        return None
    for line in doc.split('\n'):
        line = line.strip()
        if not line:
            continue
        head = line.split('(', 1)[0].strip()
        # First token before '(' must be this function's name (pybind11 style).
        if '(' in line and ')' in line and head in (name, '__init__', name.split('.')[-1]):
            params = line[line.index('('):line.rindex(')') + 1]
            ret = ''
            tail = line[line.rindex(')') + 1:].strip()
            if tail.startswith('->'):
                ret = ' -> ' + tail[2:].strip().rstrip(':')
            return params, ret
        return None  # first non-empty line isn't a signature -> give up
    return None


def _signature_target(obj):
    if inspect.isclass(obj):
        return obj
    target = getattr(obj, 'im_func', None) or getattr(obj, '__func__', None)
    if target is not None:
        return target
    if not inspect.isroutine(obj) and callable(obj):
        call = getattr(obj, '__call__', obj)
        return getattr(call, 'im_func', None) or getattr(call, '__func__', None) or call
    return obj


def _inspect_signature(obj):
    """Best-effort callable signature, compatible with Python 2.7 and modern Python."""
    try:
        signature = getattr(inspect, 'signature', None)
        if signature is not None:
            return str(signature(obj))
    except Exception:
        pass

    target = obj
    strip_first = False
    if inspect.isclass(obj):
        target = getattr(obj, '__init__', None)
        strip_first = target is not None
    elif inspect.ismethod(obj):
        bound_to = getattr(obj, 'im_self', None) or getattr(obj, '__self__', None)
        target = getattr(obj, 'im_func', None) or getattr(obj, '__func__', None) or obj
        strip_first = bound_to is not None
    elif not inspect.isfunction(obj) and callable(obj):
        target = getattr(obj, '__call__', None)
        if target is not None:
            bound_to = getattr(target, 'im_self', None) or getattr(target, '__self__', None)
            target = getattr(target, 'im_func', None) or getattr(target, '__func__', None) or target
            strip_first = bound_to is not None
    if target is None:
        return None

    try:
        spec = inspect.getargspec(target)
        args = list(spec.args)
        if strip_first and args:
            args.pop(0)
        return inspect.formatargspec(args, spec.varargs, spec.keywords, spec.defaults)
    except Exception:
        return None


def _signature(name, obj):
    target = _signature_target(obj)
    key = (name, id(target), id(type(target)))
    cached = _cache_get(_SIGNATURE_CACHE, key)
    if cached is not _CACHE_MISS:
        return cached

    documented = _doc_signature(name, obj)
    if documented is not None:
        value = (documented[0] + documented[1]).strip()
    else:
        value = _inspect_signature(obj)
    _cache_put(_SIGNATURE_CACHE, key, value)
    return value


def _short(text, limit=160):
    try:
        text = text if isinstance(text, str) else str(text)
    except Exception:
        return '<unrepr>'
    text = text.replace('\n', ' ')
    return text if len(text) <= limit else text[:limit] + '...'


DISPATCH = {
    'hello': handle_hello,
    'exec': handle_exec,
    'complete': handle_complete,
    'inspect': handle_inspect,
    'lint': handle_lint,
}

# Ops that touch live game objects and must run on the main thread.
MAIN_THREAD_OPS = frozenset(['exec', 'complete', 'inspect'])
