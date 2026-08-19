import type * as monaco from 'monaco-editor'
import type { Diagnostic } from '@/shared/api'
import { repl } from '@/shared/repl'
import { toMonacoMarker } from '@/entities/diagnostic'

const OWNER = 'wms-lint'
const DEBOUNCE_MS = 400

// The running game's Python 2.7 compiler is the syntax authority.
async function collect(code: string): Promise<Diagnostic[]> {
  try {
    const frame = await repl.lintCode(code)
    if (frame.type === 'lint') return frame.diagnostics
  } catch {
    // No live client: there is no compatible Python 2.7 parser to consult.
  }
  return []
}

export function attachLinter(
  m: Pick<typeof monaco, 'editor'>,
  model: monaco.editor.ITextModel,
): () => void {
  let timer: number | undefined

  const run = async () => {
    const diagnostics = await collect(model.getValue())
    if (model.isDisposed()) return
    m.editor.setModelMarkers(model, OWNER, diagnostics.map(toMonacoMarker))
  }

  const sub = model.onDidChangeContent(() => {
    window.clearTimeout(timer)
    timer = window.setTimeout(() => void run(), DEBOUNCE_MS)
  })

  return () => {
    window.clearTimeout(timer)
    sub.dispose()
  }
}
