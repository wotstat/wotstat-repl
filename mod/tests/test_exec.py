"""Verify correlated exec output without bypassing the outer stdout stream."""

import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent import handlers


class OuterStream(object):
    def __init__(self):
        self.parts = []

    def write(self, text):
        self.parts.append(text)

    def flush(self):
        pass

    def getvalue(self):
        return ''.join(self.parts)


def main():
    saved_out = sys.stdout
    saved_err = sys.stderr
    outer_out = OuterStream()
    outer_err = OuterStream()
    try:
        sys.stdout = outer_out
        sys.stderr = outer_err

        evaluated = handlers.handle_exec({'id': '1', 'code': '21 * 2'})
        assert evaluated['ok'] and evaluated['repr'] == '42', evaluated

        printed = handlers.handle_exec({'id': '2', 'code': "print('hello')"})
        assert printed['stdout'] == 'hello\n', printed
        assert 'hello\n' in outer_out.getvalue(), outer_out.getvalue()

        errored = handlers.handle_exec({
            'id': '3', 'code': "import sys; sys.stderr.write('warning')",
        })
        assert errored['stderr'] == 'warning', errored
        assert 'warning' in outer_err.getvalue(), outer_err.getvalue()

        failed = handlers.handle_exec({
            'id': '4', 'code': "print('before boom')\nraise RuntimeError('boom')",
        })
        assert not failed['ok'] and 'RuntimeError' in failed['exc'], failed
        assert failed['stdout'] == 'before boom\n', failed
        assert sys.stdout is outer_out and sys.stderr is outer_err
        assert 'before boom\n' in outer_out.getvalue(), outer_out.getvalue()

        invalid_write = handlers.handle_exec({
            'id': '5', 'code': 'import sys; sys.stdout.write(None)',
        })
        assert not invalid_write['ok'], invalid_write
        assert 'TypeError' in invalid_write['exc'], invalid_write
        assert invalid_write['stdout'] == '', invalid_write
        assert sys.stdout is outer_out and sys.stderr is outer_err
    finally:
        sys.stdout = saved_out
        sys.stderr = saved_err

    print('EXEC OK -- repr/stdout/stderr/exception capture')
    return 0


if __name__ == '__main__':
    sys.exit(main())
