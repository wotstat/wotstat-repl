"""Interface tests for virtual input and local screenshot capture (py2.7/3.x)."""

import os
import shutil
import sys
import tempfile
import types

sys.path.insert(0, os.path.abspath(os.path.join(
    os.path.dirname(__file__), '..', 'res', 'scripts', 'common')))

from wms_agent import automation
from wms_agent.runner import schedule_on_main


class Cursor(object):
    def __init__(self):
        self.position = Vector2(0.0, 0.0)


class Vector2(object):
    def __init__(self, x, y):
        self.x = float(x)
        self.y = float(y)

    def __getitem__(self, index):
        return (self.x, self.y)[index]

    def __eq__(self, other):
        try:
            return self.x == float(other[0]) and self.y == float(other[1])
        except (IndexError, TypeError, ValueError):
            return False


class KeyEvent(object):
    def __init__(self, key, repeat_count, modifiers, character, position):
        self.key = key
        self.repeat_count = repeat_count
        self.modifiers = modifiers
        self.character = character
        self.position = position


class MouseEvent(object):
    def __init__(self, dx, dy, dz, position):
        assert type(dx) is int, type(dx)
        assert type(dy) is int, type(dy)
        assert type(dz) is int, type(dz)
        assert isinstance(position, Vector2), type(position)
        self.dx = dx
        self.dy = dy
        self.dz = dz
        self.position = position


def _install_fake_game(root):
    cursor = Cursor()
    key_events = []
    mouse_events = []
    screenshot_calls = []
    callbacks = []

    bigworld = types.ModuleType('BigWorld')
    bigworld.windowSize = lambda: (200.0, 100.0)
    bigworld.KeyEvent = KeyEvent
    bigworld.MouseEvent = MouseEvent

    def callback(delay, function):
        assert delay == 0.0, delay
        callbacks.append(function)
        return len(callbacks)

    bigworld.callback = callback

    def screenshot(image_format, mask):
        assert image_format in ('jpg', 'png')
        normalized = mask.replace('\\', '/')
        assert normalized.startswith('./../screenshots/wotstat-repl-'), mask
        screenshot_calls.append((image_format, normalized))

    bigworld.screenShot = screenshot

    gui_module = types.ModuleType('GUI')
    gui_module.mcursor = lambda: cursor

    math_module = types.ModuleType('Math')
    math_module.Vector2 = Vector2

    keys = types.ModuleType('Keys')
    keys.MODIFIER_SHIFT = 1
    keys.MODIFIER_CTRL = 2
    keys.MODIFIER_ALT = 4
    keys.KEY_ESCAPE = 1
    keys.KEY_A = 30
    keys.KEY_LSHIFT = 42
    keys.KEY_LCONTROL = 29
    keys.KEY_LALT = 56
    keys.KEY_LEFTMOUSE = 256
    keys.KEY_RIGHTMOUSE = 257
    keys.KEY_MIDDLEMOUSE = 258

    game = types.ModuleType('game')

    def handle_key(event):
        key_events.append(event)
        return True

    def handle_mouse(event):
        mouse_events.append(event)
        return False

    game.handleKeyEvent = handle_key
    game.handleMouseEvent = handle_mouse

    replacements = {
        'BigWorld': bigworld,
        'GUI': gui_module,
        'Keys': keys,
        'Math': math_module,
        'game': game,
    }
    previous = dict((name, sys.modules.get(name)) for name in replacements)
    sys.modules.update(replacements)
    return (previous, cursor, key_events, mouse_events, screenshot_calls,
            callbacks)


def _restore_modules(previous):
    for name, module in previous.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


def main():
    root = tempfile.mkdtemp(prefix='wms_automation_')
    old_cwd = os.getcwd()
    previous = None
    try:
        os.chdir(root)
        (previous, cursor, key_events, mouse_events, screenshot_calls,
         callbacks) = _install_fake_game(root)

        moved = automation.handle_mouse({
            'id': 'move', 'type': 'mouse', 'action': 'move',
            'x': 50, 'y': 25, 'modifiers': [],
        })
        assert moved['ok'], moved
        assert 'handled' not in moved, moved
        assert cursor.position == (-0.5, 0.5), cursor.position
        assert len(mouse_events) == 1
        assert mouse_events[0].dx == -50 and mouse_events[0].dy == -25

        completed = []
        schedule_on_main(
            lambda: automation.handle_mouse({
                'id': 'click', 'type': 'mouse', 'action': 'click',
                'button': 'left', 'modifiers': ['Control'],
            }),
            lambda response, error: completed.append((response, error)))
        assert not completed, completed
        assert not key_events, key_events
        assert len(callbacks) == 1, callbacks
        callbacks.pop(0)()
        assert [event.repeat_count for event in key_events] == [0], key_events
        assert not completed, completed
        assert len(callbacks) == 1, callbacks
        callbacks.pop(0)()
        assert [event.repeat_count for event in key_events] == [0, -1]
        assert all(event.modifiers == 2 for event in key_events)
        assert len(completed) == 1, completed
        clicked, click_error = completed[0]
        assert click_error is None, click_error
        assert clicked['ok'], clicked
        assert 'handled' not in clicked, clicked

        pressed = automation.handle_keyboard({
            'id': 'key', 'type': 'keyboard', 'action': 'press',
            'key': 'Escape', 'modifiers': [],
        })
        assert pressed['ok'] and pressed['key'] == 'KEY_ESCAPE', pressed
        assert 'handled' not in pressed, pressed
        assert [event.repeat_count for event in key_events[-2:]] == [0, -1]

        started = automation.handle_screenshot({
            'id': 'shot', 'type': 'screenshot', 'format': 'jpg',
            'capture_id': '0123456789abcdef0123456789abcdef',
        })
        assert started['ok'], started
        assert started['width'] == 200 and started['height'] == 100, started
        assert screenshot_calls == [(
            'jpg',
            './../screenshots/wotstat-repl-0123456789abcdef0123456789abcdef',
        )], screenshot_calls

        invalid = automation.handle_screenshot({
            'id': 'bad-shot', 'type': 'screenshot', 'format': 'jpg',
            'capture_id': '../outside',
        })
        assert not invalid['ok'] and 'capture_id' in invalid['error'], invalid
    finally:
        if previous is not None:
            _restore_modules(previous)
        os.chdir(old_cwd)
        shutil.rmtree(root, ignore_errors=True)

    print('AUTOMATION OK -- virtual mouse/keyboard and local screenshot capture')
    return 0


if __name__ == '__main__':
    sys.exit(main())
