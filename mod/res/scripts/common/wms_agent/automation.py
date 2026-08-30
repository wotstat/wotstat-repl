"""Virtual game input and screenshot capture for the desktop MCP adapter.

All BigWorld/GUI/game calls happen through handlers marked as main-thread
operations by ``loop.py``. The desktop and game share a filesystem, so the
agent only starts a capture; the Rust desktop reads the resulting file.
"""

import math
import traceback

from .runner import DeferredMainResult


try:
    _STRING_TYPES = (basestring,)
except NameError:
    _STRING_TYPES = (str,)


def _error(req, kind, message, **extra):
    result = {'id': req.get('id'), 'type': kind, 'ok': False,
              'error': str(message)}
    result.update(extra)
    return result


def handle_screenshot(req):
    image_format = str(req.get('format') or 'jpg').lower()
    if image_format == 'jpeg':
        image_format = 'jpg'
    if image_format not in ('jpg', 'png'):
        return _error(req, 'screenshot_started',
                      'format must be jpg or png')
    capture_id = str(req.get('capture_id') or '').lower()
    if (len(capture_id) != 32
            or any(char not in '0123456789abcdef' for char in capture_id)):
        return _error(req, 'screenshot_started', 'invalid capture_id')

    try:
        import BigWorld
        game_mask = u'./../screenshots/wotstat-repl-' + capture_id
        width, height = BigWorld.windowSize()
        BigWorld.screenShot(image_format, game_mask)
        return {'id': req.get('id'), 'type': 'screenshot_started',
                'ok': True, 'width': int(width), 'height': int(height),
                'error': None}
    except BaseException as error:
        return _error(req, 'screenshot_started', error)


def _number(value, name):
    if isinstance(value, bool):
        raise ValueError('%s must be a finite number' % name)
    try:
        result = float(value)
    except (TypeError, ValueError):
        raise ValueError('%s must be a finite number' % name)
    if math.isnan(result) or math.isinf(result):
        raise ValueError('%s must be a finite number' % name)
    return result


def _modules():
    import BigWorld
    import GUI
    import Keys
    import Math
    import game
    return BigWorld, GUI, Keys, Math, game


def _modifiers(Keys, values):
    result = 0
    aliases = {
        'shift': Keys.MODIFIER_SHIFT,
        'control': Keys.MODIFIER_CTRL,
        'ctrl': Keys.MODIFIER_CTRL,
        'alt': Keys.MODIFIER_ALT,
    }
    for value in values or ():
        name = str(value).strip().lower()
        if name not in aliases:
            raise ValueError('unknown modifier: %s' % value)
        result |= aliases[name]
    return result


def _window(BigWorld):
    width, height = BigWorld.windowSize()
    width = float(width)
    height = float(height)
    if width <= 0 or height <= 0:
        raise ValueError('game window has an invalid size')
    return width, height


def _pixel_position(cursor, width, height):
    position = cursor.position
    return ((float(position[0]) + 1.0) * width / 2.0,
            (1.0 - float(position[1])) * height / 2.0)


def _vector2(Math, position):
    return Math.Vector2(float(position[0]), float(position[1]))


def _move_cursor(BigWorld, GUI, Math, game, x, y):
    width, height = _window(BigWorld)
    if x < 0 or x > width or y < 0 or y > height:
        raise ValueError('cursor coordinates are outside the game window')
    cursor = GUI.mcursor()
    old_x, old_y = _pixel_position(cursor, width, height)
    normalized = Math.Vector2(2.0 * x / width - 1.0,
                              1.0 - 2.0 * y / height)
    cursor.position = normalized
    event = BigWorld.MouseEvent(int(round(x - old_x)),
                                int(round(y - old_y)), 0, normalized)
    game.handleMouseEvent(event)
    return width, height, normalized


def _mouse_button(Keys, name):
    buttons = {
        'left': Keys.KEY_LEFTMOUSE,
        'right': Keys.KEY_RIGHTMOUSE,
        'middle': Keys.KEY_MIDDLEMOUSE,
    }
    key = buttons.get(str(name or '').strip().lower())
    if key is None:
        raise ValueError('button must be left, right, or middle')
    return key


def _deferred_click(BigWorld, game, down, up, response):
    def start(completed):
        def finish_error(error):
            try:
                game.handleKeyEvent(up)
            except BaseException:
                error += ('\nmouse release after failure also failed:\n' +
                          traceback.format_exc())
            completed(None, error)

        def release():
            try:
                game.handleKeyEvent(up)
            except BaseException:
                completed(None, traceback.format_exc())
            else:
                completed(response, None)

        def press():
            try:
                game.handleKeyEvent(down)
            except BaseException:
                finish_error(traceback.format_exc())
                return
            try:
                BigWorld.callback(0.0, release)
            except BaseException:
                finish_error(traceback.format_exc())

        BigWorld.callback(0.0, press)

    return DeferredMainResult(start)


def handle_mouse(req):
    try:
        BigWorld, GUI, Keys, Math, game = _modules()
        action = str(req.get('action') or '').strip().lower()
        if action not in ('move', 'down', 'up', 'click', 'wheel'):
            raise ValueError('action must be move, down, up, click, or wheel')
        modifiers = _modifiers(Keys, req.get('modifiers'))
        width, height = _window(BigWorld)
        x_value = req.get('x')
        y_value = req.get('y')
        if (x_value is None) != (y_value is None):
            raise ValueError('x and y must be provided together')
        if x_value is not None:
            x = _number(x_value, 'x')
            y = _number(y_value, 'y')
            width, height, position = _move_cursor(
                BigWorld, GUI, Math, game, x, y)
        else:
            x, y = _pixel_position(GUI.mcursor(), width, height)
            position = _vector2(Math, GUI.mcursor().position)
        if action == 'move':
            if x_value is None:
                raise ValueError('move requires x and y')
        elif action == 'wheel':
            delta = int(req.get('wheel_delta') or 0)
            if delta == 0:
                raise ValueError('wheel requires a non-zero wheel_delta')
            event = BigWorld.MouseEvent(0, 0, delta, position)
            game.handleMouseEvent(event)
        else:
            key = _mouse_button(Keys, req.get('button'))
            if action == 'down':
                event = BigWorld.KeyEvent(key, 0, modifiers, None, position)
                game.handleKeyEvent(event)
            elif action == 'up':
                event = BigWorld.KeyEvent(key, -1, modifiers, None, position)
                game.handleKeyEvent(event)
            else:
                down = BigWorld.KeyEvent(key, 0, modifiers, None, position)
                up = BigWorld.KeyEvent(key, -1, modifiers, None, position)
                x, y = _pixel_position(GUI.mcursor(), width, height)
                response = {
                    'id': req.get('id'), 'type': 'input', 'ok': True,
                    'x': x, 'y': y, 'width': width, 'height': height,
                    'key': None, 'error': None,
                }
                return _deferred_click(BigWorld, game, down, up, response)
        x, y = _pixel_position(GUI.mcursor(), width, height)
        return {'id': req.get('id'), 'type': 'input', 'ok': True,
                'x': x, 'y': y, 'width': width, 'height': height,
                'key': None, 'error': None}
    except BaseException as error:
        return _error(req, 'input', error)


_KEY_ALIASES = {
    'escape': 'KEY_ESCAPE',
    'enter': 'KEY_RETURN',
    'return': 'KEY_RETURN',
    'tab': 'KEY_TAB',
    'backspace': 'KEY_BACKSPACE',
    'space': 'KEY_SPACE',
    'delete': 'KEY_DELETE',
    'insert': 'KEY_INSERT',
    'home': 'KEY_HOME',
    'end': 'KEY_END',
    'pageup': 'KEY_PGUP',
    'pagedown': 'KEY_PGDN',
    'arrowup': 'KEY_UPARROW',
    'arrowdown': 'KEY_DOWNARROW',
    'arrowleft': 'KEY_LEFTARROW',
    'arrowright': 'KEY_RIGHTARROW',
    'shift': 'KEY_LSHIFT',
    'control': 'KEY_LCONTROL',
    'ctrl': 'KEY_LCONTROL',
    'alt': 'KEY_LALT',
}


def _resolve_key(Keys, value):
    if not isinstance(value, _STRING_TYPES) or not value.strip():
        raise ValueError('key must be a non-empty string')
    raw = value.strip()
    compact = raw.replace('_', '').replace('-', '').replace(' ', '').lower()
    name = _KEY_ALIASES.get(compact)
    if name is None and len(raw) == 1 and raw.isalnum():
        name = 'KEY_' + raw.upper()
    if name is None and compact.startswith('f') and compact[1:].isdigit():
        number = int(compact[1:])
        if 1 <= number <= 15:
            name = 'KEY_F%d' % number
    if name is None and raw.upper().startswith('KEY_'):
        name = raw.upper()
    if name is None or not hasattr(Keys, name):
        raise ValueError('unknown key: %s' % value)
    return name, getattr(Keys, name)


def handle_keyboard(req):
    try:
        BigWorld, GUI, Keys, Math, game = _modules()
        action = str(req.get('action') or '').strip().lower()
        if action not in ('down', 'up', 'press'):
            raise ValueError('action must be down, up, or press')
        key_name, key = _resolve_key(Keys, req.get('key'))
        modifiers = _modifiers(Keys, req.get('modifiers'))
        character = req.get('character')
        if character is not None:
            if not isinstance(character, _STRING_TYPES) or len(character) != 1:
                raise ValueError('character must contain exactly one character')
        width, height = _window(BigWorld)
        position = _vector2(Math, GUI.mcursor().position)
        if action == 'down':
            game.handleKeyEvent(
                BigWorld.KeyEvent(key, 0, modifiers, character, position))
        elif action == 'up':
            game.handleKeyEvent(
                BigWorld.KeyEvent(key, -1, modifiers, character, position))
        else:
            down = BigWorld.KeyEvent(key, 0, modifiers, character, position)
            try:
                game.handleKeyEvent(down)
            finally:
                up = BigWorld.KeyEvent(key, -1, modifiers, character, position)
                game.handleKeyEvent(up)
        x, y = _pixel_position(GUI.mcursor(), width, height)
        return {'id': req.get('id'), 'type': 'input', 'ok': True,
                'x': x, 'y': y, 'width': width, 'height': height,
                'key': key_name, 'error': None}
    except BaseException as error:
        return _error(req, 'input', error)
