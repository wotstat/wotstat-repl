import { repl } from '@/shared/repl'
import { consoleBus } from '@/entities/console'
import { pushHistory } from '@/entities/editor'

export async function runCode(code: string): Promise<void> {
  const trimmed = code.trim()
  if (!trimmed) return
  pushHistory(trimmed)
  consoleBus.append([{ stream: 'input', text: `>>> ${trimmed.replace(/\n/g, '\n... ')}\n` }])
  try {
    const frame = await repl.execCode(code)
    if (frame.type === 'result') {
      if (frame.exc) {
        consoleBus.append([{ stream: 'stderr', text: frame.exc.endsWith('\n') ? frame.exc : `${frame.exc}\n` }])
      } else if (frame.repr != null) {
        consoleBus.append([{ stream: 'result', text: `${frame.repr}\n` }])
      }
    }
  } catch (error) {
    consoleBus.append([{ stream: 'stderr', text: `${String(error)}\n` }])
  }
}
