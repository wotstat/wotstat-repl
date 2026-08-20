import { afterEach, describe, expect, test } from 'bun:test'
import type { ServerEvent } from '@/shared/api/dto'
import { webReplRuntime } from './web'

const originalFetch = globalThis.fetch

function jsonResponse(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    headers: { 'Content-Type': 'application/json' },
  })
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 1000
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error('timed out waiting for web events')
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

afterEach(async () => {
  await webReplRuntime.disconnect()
  globalThis.fetch = originalFetch
  Reflect.deleteProperty(globalThis, 'window')
})

describe('web REPL reconnect', () => {
  test('continues from the last event cursor instead of replaying python.log', async () => {
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: { location: { origin: 'http://127.0.0.1:8768' }, setTimeout },
    })
    let sessionReads = 0
    globalThis.fetch = ((input: string | URL | Request, init?: RequestInit) => {
      const path = String(input)
      if (path === '/api/session') {
        sessionReads += 1
        return Promise.resolve(jsonResponse({
          version: 'test',
          pid: 42,
          session: sessionReads >= 3 ? 'next-game-session' : 'game-session',
        }))
      }
      if (path.includes('/api/events?cursor=0')) {
        const text = sessionReads >= 3
          ? 'from next game\n'
          : 'from game startup\n'
        return Promise.resolve(jsonResponse({
          events: [{
            type: 'stdout',
            stream: 'python_log',
            text,
          }],
          nextCursor: 1,
          truncated: false,
        }))
      }
      if (sessionReads === 2 && path.includes('/api/events?cursor=1')) {
        return Promise.resolve(jsonResponse({
          events: [{
            type: 'stdout',
            stream: 'python_log',
            text: 'after reconnect\n',
          }],
          nextCursor: 2,
          truncated: false,
        }))
      }
      return new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new Error('aborted')), {
          once: true,
        })
      })
    }) as typeof fetch

    const received: ServerEvent[] = []
    await webReplRuntime.connect((event) => received.push(event))
    await waitFor(() => received.some(
      (event) => event.kind === 'log'
        && event.lines.some((line) => line.text === 'from game startup\n'),
    ))
    await webReplRuntime.disconnect()

    await webReplRuntime.connect((event) => received.push(event))
    await waitFor(() => received.filter((event) => event.kind === 'log').length >= 2)
    await webReplRuntime.disconnect()

    await webReplRuntime.connect((event) => received.push(event))
    await waitFor(() => received.filter((event) => event.kind === 'log').length >= 3)

    const text = received
      .filter((event): event is Extract<ServerEvent, { kind: 'log' }> => event.kind === 'log')
      .flatMap((event) => event.lines.map((line) => line.text))
    expect(text).toEqual([
      'from game startup\n',
      'after reconnect\n',
      'from next game\n',
    ])
  })
})
