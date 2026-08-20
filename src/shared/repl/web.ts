import type { OutFrame, ServerEvent } from '@/shared/api/dto'
import { completionBudget } from './completionBudget'
import type { ReplRuntime } from './runtime'

interface WebSession {
  version?: string | null
  pid?: number | null
  session: string
}

type WebEventFrame =
  | Extract<OutFrame, { type: 'stdout' }>
  | { type: 'disconnected' }

interface WebEventRead {
  events: WebEventFrame[]
  nextCursor: number
  truncated: boolean
}

let generation = 0
let activePoll: AbortController | null = null
let eventSession: string | null = null
let eventCursor = 0

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    cache: 'no-store',
    ...init,
  })
  if (!response.ok) {
    const message = await response.text().catch(() => '')
    throw new Error(message || `HTTP ${response.status}`)
  }
  return response.json() as Promise<T>
}

function postFrame(frame: Record<string, unknown>): Promise<OutFrame> {
  return readJson<OutFrame>('/api/repl', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(frame),
  })
}

function emitSession(onEvent: (event: ServerEvent) => void, session: WebSession): boolean {
  const changed = eventSession !== session.session
  if (changed) {
    eventSession = session.session
    eventCursor = 0
  }
  onEvent({
    kind: 'hello',
    version: session.version,
    pid: session.pid,
    remote: false,
  })
  return changed
}

async function pollEvents(
  currentGeneration: number,
  onEvent: (event: ServerEvent) => void,
): Promise<void> {
  let online = true

  while (currentGeneration === generation) {
    const controller = new AbortController()
    activePoll = controller
    try {
      const read = await readJson<WebEventRead>(
        `/api/events?cursor=${eventCursor}&limit=500&wait_ms=20000`,
        { signal: controller.signal },
      )
      if (currentGeneration !== generation) return
      if (!online) {
        const changed = emitSession(onEvent, await readJson<WebSession>('/api/session'))
        online = true
        if (changed) continue
      }
      eventCursor = read.nextCursor
      if (read.truncated) {
        onEvent({
          kind: 'log',
          lines: [{ stream: 'system', level: 'warn', text: 'older game log output was discarded\n' }],
        })
      }
      const lines = read.events
        .filter((event): event is Extract<WebEventFrame, { type: 'stdout' }> => event.type === 'stdout')
        .map(({ stream, level, timestamp, source, text }) => ({
          stream,
          level,
          timestamp,
          source,
          text,
        }))
      if (lines.length > 0) onEvent({ kind: 'log', lines })
      if (read.events.some((event) => event.type === 'disconnected')) {
        onEvent({ kind: 'disconnected' })
        online = false
      }
    } catch (error) {
      if (currentGeneration !== generation || controller.signal.aborted) return
      if (online) {
        onEvent({ kind: 'disconnected' })
        online = false
      }
      await new Promise((resolve) => window.setTimeout(resolve, 1000))
    } finally {
      if (activePoll === controller) activePoll = null
    }
  }
}

export const webReplRuntime: ReplRuntime = {
  async connect(onEvent) {
    const currentGeneration = ++generation
    activePoll?.abort()
    const session = await readJson<WebSession>('/api/session')
    if (currentGeneration !== generation) throw new Error('connection cancelled')
    emitSession(onEvent, session)
    void pollEvents(currentGeneration, onEvent)
    return { endpoint: window.location.origin, waitingForAgent: false }
  },

  async disconnect() {
    generation += 1
    activePoll?.abort()
    activePoll = null
  },

  execCode: (code) => postFrame({ type: 'exec', code }),
  complete: (prefix, budget = completionBudget()) =>
    postFrame({ type: 'complete', prefix, budget }),
  inspect: (expr) => postFrame({ type: 'inspect', expr }),
  lintCode: (code) => postFrame({ type: 'lint', code }),
}
