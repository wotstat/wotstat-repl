import { describe, expect, test } from 'bun:test'
import {
  activityArguments,
  activityResponseError,
  activityResponseImage,
  activityResponseText,
  activityStructuredContent,
  getActivityNavigation,
  hasPrettyActivity,
} from './activityPresentation'

const request = {
  method: 'tools/call',
  params: { name: 'wot_exec', arguments: { code: 'print 42', timeout_ms: 1000 } },
}

const response = {
  result: {
    content: [{ type: 'text', text: 'Python execution succeeded.' }],
    structuredContent: { ok: true, repr: '42', stdout: '', stderr: '' },
    isError: false,
  },
}

describe('MCP activity presentation', () => {
  test('extracts the readable request and response fields', () => {
    expect(activityArguments(request)).toEqual({ code: 'print 42', timeout_ms: 1000 })
    expect(activityResponseText(response)).toBe('Python execution succeeded.')
    expect(activityStructuredContent(response)).toEqual({
      ok: true,
      repr: '42',
      stdout: '',
      stderr: '',
    })
    expect(activityResponseError(response)).toBeNull()
  })

  test('extracts tool and protocol errors', () => {
    expect(
      activityResponseError({
        result: { content: [{ type: 'text', text: 'No active client' }], isError: true },
      }),
    ).toBe('No active client')
    expect(activityResponseError({ error: { code: -32602, message: 'Invalid params' } })).toBe(
      'Invalid params',
    )
  })

  test('extracts a safe image preview from screenshot content', () => {
    expect(
      activityResponseImage({
        result: {
          content: [
            { type: 'text', text: 'Screenshot captured.' },
            { type: 'image', data: 'base64-payload', mimeType: 'image/png' },
          ],
        },
      }),
    ).toEqual({ data: 'base64-payload', mimeType: 'image/png' })
    expect(
      activityResponseImage({
        result: { content: [{ type: 'image', data: 'payload', mimeType: 'text/html' }] },
      }),
    ).toBeNull()
  })

  test('enables readable views only for known commands', () => {
    for (const command of [
      'wot_exec',
      'wot_screenshot',
      'wot_list_clients',
      'wot_read_log',
      'wot_start_client',
      'wot_close_client',
      'wot_kill_client',
      'wot_mouse',
      'wot_keyboard',
    ]) {
      expect(hasPrettyActivity(command)).toBe(true)
    }
    expect(hasPrettyActivity('future_tool')).toBe(false)
  })

  test('navigates a newest-first activity list in chronological order', () => {
    const entries = [{ id: 3 }, { id: 2 }, { id: 1 }]

    expect(getActivityNavigation(entries, 1)).toEqual({
      position: 1,
      total: 3,
      previousId: null,
      nextId: 2,
    })
    expect(getActivityNavigation(entries, 2)).toEqual({
      position: 2,
      total: 3,
      previousId: 1,
      nextId: 3,
    })
    expect(getActivityNavigation(entries, 3)).toEqual({
      position: 3,
      total: 3,
      previousId: 2,
      nextId: null,
    })
  })

  test('disables activity navigation when the selected event is unavailable', () => {
    expect(getActivityNavigation([{ id: 2 }, { id: 1 }], 9)).toEqual({
      position: 0,
      total: 2,
      previousId: null,
      nextId: null,
    })
  })
})
