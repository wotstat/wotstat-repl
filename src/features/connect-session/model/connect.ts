import { api, createServerChannel } from '@/shared/api'
import { useSession } from '@/entities/session'
import { consoleBus } from '@/entities/console'

let connectGeneration = 0

function waitForConnected(generation: number): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false
    let unsub: () => void = () => undefined

    const finish = (connected: boolean) => {
      if (settled) return
      settled = true
      unsub()
      resolve(connected)
    }

    unsub = useSession.subscribe((state) => {
      if (state.status === 'connected') finish(true)
      else if (generation !== connectGeneration || state.status === 'disconnected') finish(false)
    })

    const status = useSession.getState().status
    if (status === 'connected') finish(true)
    else if (generation !== connectGeneration || status === 'disconnected') finish(false)
  })
}

export async function connect(bufferDir?: string): Promise<void> {
  const dir = bufferDir ?? (await api.defaultBufferDir())
  const generation = ++connectGeneration
  const session = useSession.getState()
  session.setBufferDir(dir)
  session.setStatus('connecting')

  const channel = createServerChannel((event) => {
    if (generation !== connectGeneration) return
    if (event.kind === 'log') {
      consoleBus.append(event.lines)
    } else if (event.kind === 'hello') {
      const s = useSession.getState()
      s.setStatus('connected')
      s.setHello(event)
      consoleBus.system(`agent online (pid ${event.pid ?? '?'}, v${event.version ?? '?'})\n`)
    } else if (event.kind === 'disconnected') {
      const s = useSession.getState()
      if (s.status === 'connected') {
        s.setStatus('disconnected')
        consoleBus.system('game disconnected\n')
      }
    }
  })

  try {
    await api.connect(dir, channel)
    consoleBus.system(`listening on ${dir} — waiting for the game\n`)
    await waitForConnected(generation)
  } catch (error) {
    if (generation !== connectGeneration) return
    useSession.getState().setStatus('disconnected')
    consoleBus.system(`connect failed: ${String(error)}\n`)
  }
}

export async function disconnect(): Promise<void> {
  connectGeneration += 1
  await api.disconnect().catch(() => undefined)
  useSession.getState().setStatus('disconnected')
  consoleBus.system('disconnected\n')
}
