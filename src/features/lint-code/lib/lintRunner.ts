import type * as monaco from 'monaco-editor'
import { api, type Diagnostic } from '@/shared/api'
import { extractArray } from '@/shared/lib'
import { toMonacoMarker } from '@/entities/diagnostic'

const OWNER = 'wms-lint'
const DEBOUNCE_MS = 400

// Authoritative py2.7 compile() in the game when connected; otherwise the jedi
// static worker (compile + pyflakes). Either way, markers land on the model.
async function collect(code: string): Promise<Diagnostic[]> {
  try {
    const frame = await api.lintCode(code)
    if (frame.type === 'lint') return frame.diagnostics
  } catch {
    // not connected; fall through to static
  }
  try {
    return extractArray<Diagnostic>(await api.jediLint(code), 'diagnostics')
  } catch {
    return []
  }
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
