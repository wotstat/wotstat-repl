# -*- coding: utf-8 -*-
"""Compile the Python 2.7 game agent and package it as a universal .mod archive."""

import argparse
import os
import py_compile
import shutil
import sys
import tempfile
import zipfile


HERE = os.path.dirname(os.path.abspath(__file__))
RES_ROOT = os.path.join(HERE, 'res')
META_PATH = os.path.join(HERE, 'meta.xml')


def build(version, output):
    if sys.version_info[:2] != (2, 7):
        raise SystemExit('Python 2.7 is required (got %s)' % sys.version.split()[0])

    work = tempfile.mkdtemp(prefix='fuflo-wotrepl-mod-')
    try:
        staged_res = os.path.join(work, 'res')
        shutil.copytree(
            RES_ROOT,
            staged_res,
            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'),
        )

        version_path = os.path.join(
            staged_res, 'scripts', 'common', 'wms_agent', '__init__.py')
        with open(version_path, 'r') as handle:
            version_source = handle.read()
        if version_source.count('{{VERSION}}') != 1:
            raise RuntimeError('agent __version__ must contain exactly one {{VERSION}} placeholder')
        with open(version_path, 'w') as handle:
            handle.write(version_source.replace('{{VERSION}}', version))

        compiled = 0
        for root, _dirs, files in os.walk(staged_res):
            for name in files:
                if not name.endswith('.py'):
                    continue
                source = os.path.join(root, name)
                py_compile.compile(source, cfile=source + 'c', doraise=True)
                os.remove(source)
                compiled += 1
        if not compiled:
            raise RuntimeError('no Python sources found under mod/res')

        with open(META_PATH, 'r') as handle:
            meta = handle.read()
        if meta.count('{{VERSION}}') != 1:
            raise RuntimeError('meta.xml must contain exactly one {{VERSION}} placeholder')
        with open(os.path.join(work, 'meta.xml'), 'w') as handle:
            handle.write(meta.replace('{{VERSION}}', version))

        output = os.path.abspath(output)
        output_dir = os.path.dirname(output)
        if not os.path.isdir(output_dir):
            os.makedirs(output_dir)
        if os.path.exists(output):
            os.remove(output)

        archive = zipfile.ZipFile(output, 'w', zipfile.ZIP_STORED)
        try:
            for root, _dirs, files in os.walk(work):
                for name in sorted(files):
                    path = os.path.join(root, name)
                    archive.write(path, os.path.relpath(path, work).replace(os.sep, '/'))
        finally:
            archive.close()

        _verify(output, version)
        print 'built:', output
    finally:
        shutil.rmtree(work, ignore_errors=True)


def _verify(output, version):
    archive = zipfile.ZipFile(output, 'r')
    try:
        names = set(archive.namelist())
        required = set([
            'meta.xml',
            'res/scripts/common/bw_site.pyc',
            'res/scripts/common/wms_agent/__init__.pyc',
        ])
        missing = required - names
        if missing:
            raise RuntimeError('archive is missing: %s' % ', '.join(sorted(missing)))
        if any(name.endswith('.py') for name in names):
            raise RuntimeError('archive contains uncompiled Python sources')
        if any('__pycache__' in name.split('/') for name in names):
            raise RuntimeError('archive contains Python cache directories')
        agent_pyc = archive.read('res/scripts/common/wms_agent/__init__.pyc')
        if version not in agent_pyc or '{{VERSION}}' in agent_pyc:
            raise RuntimeError('agent version was not injected')
        corrupt = archive.testzip()
        if corrupt is not None:
            raise RuntimeError('corrupt archive member: %s' % corrupt)
    finally:
        archive.close()


def main():
    parser = argparse.ArgumentParser(description='Build the universal Fuflo WoT REPL .mod')
    parser.add_argument('--version', required=True)
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    build(args.version, args.out)


if __name__ == '__main__':
    main()
