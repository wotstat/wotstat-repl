"""Request handlers. Each takes a request dict and returns a response dict.

Game-touching handlers (exec/complete/inspect/dump) are marshaled to the main
thread by the loop; lint is pure and runs anywhere.
"""

import os
import sys
import inspect
import traceback

from . import __version__

_DEFAULT_COMPLETION_BUDGET = 120
_MAX_COMPLETION_BUDGET = 10000


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
        sig = _doc_signature(name, obj)
        if sig is not None:
            cand['signature'] = (sig[0] + sig[1]).strip()
    else:
        cand['kind'] = type(obj).__name__
    doc = getattr(obj, '__doc__', None)
    if doc:
        cand['doc'] = _short(doc, 200)


def _builtins_ns():
    try:
        import __builtin__ as b  # py2
    except ImportError:
        import builtins as b  # py3
    return b


def _iter_public_attrs(obj, keep_dunder=None):
    """Yield attribute names from sorted(dir(obj)), skipping dunders unless in keep_dunder."""
    try:
        attrs = sorted(dir(obj))
    except Exception:
        return
    for attr in attrs:
        if attr.startswith('__'):
            if keep_dunder is None or attr not in keep_dunder:
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
        cand = {'name': name, 'source': 'live'}
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
            cand = {'name': attr, 'source': 'live'}
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
    import inspect
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
    try:
        if inspect.isfunction(obj) or inspect.ismethod(obj):
            spec = inspect.getargspec(obj)
            out['signature'] = req.get('expr', '') + inspect.formatargspec(*spec)
    except Exception:
        pass
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


_PRIMITIVES = (type(None), bool, int, float, complex, str)
_STRING_TYPES = (str,)
try:
    _PRIMITIVES = _PRIMITIVES + (long, unicode)  # noqa: F821  py2.7
    _STRING_TYPES = (str, unicode)  # noqa: F821  json.loads yields unicode on py2.7
except NameError:
    pass

_KEEP_DUNDER = frozenset(['__init__', '__call__', '__getitem__', '__len__', '__iter__'])


def _short(text, limit=160):
    try:
        text = text if isinstance(text, str) else str(text)
    except Exception:
        return '<unrepr>'
    text = text.replace('\n', ' ')
    return text if len(text) <= limit else text[:limit] + '...'


def _safe_repr(obj):
    try:
        return _short(repr(obj))
    except Exception:
        return '<repr failed>'


def _kind_of(obj):
    if inspect.isclass(obj):
        return 'class'
    if inspect.isroutine(obj) or callable(obj):
        return 'function'
    if isinstance(obj, _PRIMITIVES):
        return 'value'
    return 'instance'


def _introspect(name, obj, budget, depth, seen):
    """Recursively describe a LIVE object: its type, members, fields, values.

    Captures what static analysis can't: entitydef-injected attributes, attached
    components, C-extension instance fields, and real runtime types. Bounded by
    budget (node count + depth + per-object breadth) and cycle-safe via id() set.
    """
    if budget['nodes'] <= 0:
        return None
    budget['nodes'] -= 1

    kind = _kind_of(obj)
    node = {'name': name, 'kind': kind}
    try:
        # For a class, its own name is the useful type (not its metaclass 'type').
        node['type'] = obj.__name__ if kind == 'class' else type(obj).__name__
    except Exception:
        node['type'] = '?'

    if kind in ('class', 'function'):
        sig = _doc_signature(name, obj)
        if sig is not None:
            node['signature'] = (name + sig[0] + sig[1]).strip()
        doc = getattr(obj, '__doc__', None)
        if doc:
            node['doc'] = _short(doc, 200)

    if kind == 'value':
        node['repr'] = _safe_repr(obj)
        return node

    # Functions/methods are leaves: never walk their internals (func_globals,
    # im_func, __call__, ...) -- that explodes and leaks the module dict.
    if kind == 'function':
        return node

    # Recurse into classes and instances only; stop at depth/cycle limits.
    if depth <= 0:
        return node
    oid = id(obj)
    if oid in seen:
        node['cyclic'] = True
        return node
    seen = seen | set([oid])

    target = obj
    members = []
    count = 0
    for attr in _iter_public_attrs(target, keep_dunder=_KEEP_DUNDER):
        if count >= budget['max_attrs'] or budget['nodes'] <= 0:
            node['truncated'] = True
            break
        try:
            value = getattr(target, attr)  # may fire a property getter
        except Exception as exc:
            members.append({'name': attr, 'kind': 'error', 'error': _short(str(exc))})
            count += 1
            continue
        child = _introspect(attr, value, budget, depth - 1, seen)
        if child is not None:
            members.append(child)
            count += 1
    node['members'] = members
    return node


def _node_to_stub(node):
    """Emit a .pyi class for a dumped class/instance, capturing runtime-injected
    fields (typed by their live type) and methods (signatures from __doc__)."""
    type_name = node.get('type') or node.get('name')
    if node.get('kind') not in ('instance', 'class') or not type_name:
        return None
    if not type_name.replace('_', '').isalnum():
        return None
    lines = ['class %s(object):' % type_name]
    body = []
    for m in node.get('members', []):
        mname = m.get('name', '')
        if not mname or mname.startswith('__'):
            continue
        mkind = m.get('kind')
        if mkind == 'function':
            sig = m.get('signature')
            if sig and '(' in sig:
                params = sig[sig.index('('):]
                body.append('    def %s%s: ...' % (mname, params if params.startswith('(') else '(self, *args, **kwargs)'))
            else:
                body.append('    def %s(self, *args, **kwargs): ...' % mname)
        elif mkind in ('value', 'instance'):
            body.append('    %s = ...  # %s' % (mname, m.get('type', '?')))
    if not body:
        body.append('    ...')
    lines.extend(body)
    return type_name, '\n'.join(lines) + '\n'


def _collect_stubs(node, stubs, seen):
    """Walk the dump tree and emit a .pyi class for EVERY distinct live type
    encountered (not just the root), so completion learns the whole live world.
    Keeps the richest stub per type (the node with the most members)."""
    if not node:
        return
    if node.get('kind') in ('instance', 'class'):
        res = _node_to_stub(node)
        if res is not None:
            tname, text = res
            count = len(node.get('members', []))
            if tname not in seen or count > seen[tname]:
                seen[tname] = count
                stubs[tname] = text
    for member in node.get('members', []):
        _collect_stubs(member, stubs, seen)


# Roots walked by "dump all" -- evaluated best-effort; missing ones are skipped
# silently (they are probes; not every build/state exposes every root).
DEFAULT_DUMP_ROOTS = [
    'BigWorld', 'Math', 'ResMgr', 'Account',
    'BigWorld.player()', 'BigWorld.entities', 'BigWorld.target',
    'BigWorld.camera',
]


def _resolve_root(expr, ns):
    """Eval an expr; if it's a bare module name not in the namespace, import it.
    Returns (obj, error_or_None)."""
    try:
        return eval(expr, ns), None
    except BaseException as exc:
        ident = expr.strip()
        if ident and all(c.isalnum() or c == '_' for c in ident):
            try:
                return __import__(ident), None
            except BaseException:
                pass
        return None, _short(str(exc))


def _normalize_dump_exprs(exprs):
    """Return (expr_list, is_dump_all) from the raw expr field of a dump request.

    CRITICAL py2.7: json.loads yields unicode, so a single string (str OR unicode,
    via _STRING_TYPES) must become a one-element list -- must NOT be iterated
    char-by-char.
    """
    if exprs in ('*', ['*'], None, ''):
        return DEFAULT_DUMP_ROOTS, True
    if isinstance(exprs, _STRING_TYPES):
        return [exprs], False
    return exprs, False


def handle_dump(req):
    """Deep runtime introspection of live game objects -> tree + per-type stubs.

    req: { expr: "BigWorld.player()" | ["a","b"] | "*" (= dump everything),
           depth?, max_attrs?, max_nodes?, stubs?: true }
    out['stubs'] maps EVERY distinct live type encountered to a runtime-informed
    .pyi class (capturing entitydef-injected fields static can't see).
    """
    out = {'id': req.get('id'), 'type': 'dump', 'roots': [], 'errors': [], 'stubs': {}}
    exprs, dump_all = _normalize_dump_exprs(req.get('expr'))
    budget = {
        'nodes': int(req.get('max_nodes', 30000 if dump_all else 4000)),
        'max_attrs': int(req.get('max_attrs', 80)),
    }
    depth = int(req.get('depth', 3 if dump_all else 2))
    seen_types = {}
    ns = _ns()
    for expr in exprs:
        obj, err = _resolve_root(expr, ns)
        if obj is None:
            # In dump-all mode the roots are best-effort probes -> skip silently.
            if not dump_all:
                out['errors'].append({'expr': expr, 'error': err or 'unresolved'})
            continue
        root = _introspect(expr, obj, budget, depth, set())
        out['roots'].append(root)
        if req.get('stubs', True):
            _collect_stubs(root, out['stubs'], seen_types)
        if budget['nodes'] <= 0:
            break
    return out


DISPATCH = {
    'hello': handle_hello,
    'exec': handle_exec,
    'complete': handle_complete,
    'inspect': handle_inspect,
    'lint': handle_lint,
    'dump': handle_dump,
}

# Ops that touch live game objects and must run on the main thread.
MAIN_THREAD_OPS = frozenset(['exec', 'complete', 'inspect', 'dump'])
