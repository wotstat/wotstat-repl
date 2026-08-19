# jedi static worker

A long-lived CPython **2.7** process that answers completion/infer/lint requests
over JSON stdio. The Rust backend spawns and supervises it.

## Why 2.7

The decompiled WoT source is Python 2.7 (`print` statements, `except X, e`). Only
`jedi==0.17.2` (with `parso<0.8`) still parses that grammar; `jedi>=0.18` is
py3-only. The worker binds jedi's environment to its own `sys.executable`, so as
long as it runs under 2.7 the analyzed grammar is 2.7.

You can reuse the `python27.dll` + stdlib bundled with PJOrion as the interpreter.

## Install

```
py -2.7 -m pip install -r requirements.txt
```

## Configure (first request)

```json
{"id":"0","op":"configure","root":"U:/Programs/wot-src/sources/res/scripts",
 "sys_path":["U:/Programs/wot-src/sources/res/scripts/common",
             "U:/Programs/wot-src/sources/res/scripts/client",
             "U:/Programs/wot-src/sources/res/scripts/client_common",
             "%LOCALAPPDATA%/WotStatWoTREPL/stubs"]}
```

The last root is the canonical stubs dir (Rust `stubs_dir` command / `install::stubs_dir_path`).
The agent's `dump` op emits **typed** `.pyi` for every live type it walks (signatures parsed from each object's
`__doc__` first line — pybind11/BigWorld embed the full typed signature there, e.g.
`spaceLoadStatus(distance: float = -1.0) -> float`). The Rust `write_stubs` command persists
them as `<module>.pyi` into this dir, so jedi resolves native C-extension modules
(`BigWorld`, `Math`, `ResMgr`, `Account`) that have no `.py` source. `.pyi` files are parsed
with a py3 grammar even for the py2.7 project target, so annotations are fine.

`lint` works even without jedi installed (authoritative `compile()` + optional
pyflakes), which lets the protocol and supervisor be tested first.
