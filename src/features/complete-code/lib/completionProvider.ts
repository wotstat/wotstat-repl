import type * as monaco from 'monaco-editor'
import { api, type Candidate } from '@/shared/api'
import { extractArray } from '@/shared/lib'
import { toMonacoCompletion } from '@/entities/completion-item'

const COMPLETION_REFRESH_DELAY = 100
const VISIBLE_HYDRATION_DELAY = 50

type SuggestItem = { completion: monaco.languages.CompletionItem }

type SuggestList = {
  length: number
  firstVisibleIndex: number
  scrollTop: number
  renderHeight: number
  indexAt(position: number): number
  element(index: number): SuggestItem
  splice(start: number, deleteCount: number, elements: SuggestItem[]): void
  getFocus(): number[]
  setFocus(indexes: number[]): void
  onDidScroll(listener: () => void): monaco.IDisposable
}

type SuggestController = monaco.editor.IEditorContribution & {
  model: {
    state: number
    trigger(options: { auto: boolean; retrigger: boolean }): void
  }
  widget: {
    value: {
      _list: SuggestList
      _ignoreFocusEvents: boolean
      onDidFocus(listener: () => void): monaco.IDisposable
    }
  }
}

type ResolutionBatch = {
  model: monaco.editor.ITextModel
  version: number
}

type ResolutionState = {
  prefix: string
  batch: ResolutionBatch
  pending?: Promise<void>
}

function cancellationError(): Error {
  const error = new Error('Canceled')
  error.name = error.message
  return error
}

function waitForStableCompletion(token: monaco.CancellationToken): Promise<void> {
  return new Promise((resolve, reject) => {
    let subscription: monaco.IDisposable | undefined
    const timer = setTimeout(() => {
      subscription?.dispose()
      resolve()
    }, VISIBLE_HYDRATION_DELAY)
    subscription = token.onCancellationRequested(() => {
      clearTimeout(timer)
      subscription?.dispose()
      reject(cancellationError())
    })
  })
}

function completionRequest(model: monaco.editor.ITextModel, position: monaco.Position) {
  const word = model.getWordUntilPosition(position)
  const prefixLine = model.getValueInRange({
    startLineNumber: position.lineNumber,
    startColumn: 1,
    endLineNumber: position.lineNumber,
    endColumn: position.column,
  })
  return {
    word,
    prefixLine,
    fullCode: model.getValue(),
    line: position.lineNumber,
    column: position.column - 1,
    version: model.getVersionId(),
  }
}

// Two-layer completion: live runtime (agent) merged over static (jedi). Live wins
// on name collisions; both failures degrade to an empty list.
async function gather(prefixLine: string, fullCode: string, line: number, column: number): Promise<Candidate[]> {
  const [live, statc] = await Promise.allSettled([
    api.complete(prefixLine),
    api.jediComplete(fullCode, line, column),
  ])

  const merged = new Map<string, Candidate>()
  if (statc.status === 'fulfilled') {
    for (const c of extractArray<Candidate>(statc.value, 'candidates')) merged.set(c.name, { ...c, source: 'static' })
  }
  if (live.status === 'fulfilled' && live.value.type === 'complete') {
    for (const c of live.value.candidates) merged.set(c.name, { ...c, source: 'live' })
  }
  return [...merged.values()]
}

export function registerPythonCompletion(
  m: typeof monaco,
  editor: monaco.editor.IStandaloneCodeEditor,
): monaco.IDisposable {
  const resolutions = new WeakMap<monaco.languages.CompletionItem, ResolutionState>()
  const resolvedCandidates = new Map<string, Candidate>()
  let active: {
    model: monaco.editor.ITextModel
    line: number
    wordStartColumn: number
    resolvePrefix: string
  } | null = null
  let snapshot: {
    model: monaco.editor.ITextModel
    version: number
    line: number
    column: number
    candidates: Candidate[]
  } | null = null
  let refreshTimer: ReturnType<typeof setTimeout> | null = null
  let visibleHydrationTimer: ReturnType<typeof setTimeout> | null = null
  let refreshGeneration = 0

  const hydrate = (item: monaco.languages.CompletionItem): Promise<void> => {
    const state = resolutions.get(item)
    if (!state) return Promise.resolve()
    const key = state.prefix + item.insertText
    const cached = resolvedCandidates.get(key)
    if (cached) {
      Object.assign(item, toMonacoCompletion(cached, item.range as monaco.IRange))
      return Promise.resolve()
    }
    if (!state.pending) {
      state.pending = api.complete(key, 1).then((response) => {
        if (response.type !== 'complete') return
        const candidate = response.candidates.find((c) => c.name === item.insertText)
        if (candidate) {
          const resolved = { ...candidate, source: 'live' }
          resolvedCandidates.set(key, resolved)
          Object.assign(
            item,
            toMonacoCompletion(resolved, item.range as monaco.IRange),
          )
        }
      }).catch(() => undefined)
    }
    return state.pending
  }

  const provider = m.languages.registerCompletionItemProvider('python', {
    triggerCharacters: ['.'],
    async provideCompletionItems(model, position) {
      const request = completionRequest(model, position)
      const { word, prefixLine } = request
      const range: monaco.IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }
      const cached = snapshot
        && snapshot.model === model
        && snapshot.version === request.version
        && snapshot.line === request.line
        && snapshot.column === request.column
        ? snapshot.candidates
        : null
      const candidates = cached ?? await gather(
        prefixLine, request.fullCode, request.line, request.column,
      )
      const resolvePrefix = prefixLine.slice(0, prefixLine.length - word.word.length)
      snapshot = {
        model,
        version: request.version,
        line: request.line,
        column: request.column,
        candidates,
      }
      active = {
        model,
        line: position.lineNumber,
        wordStartColumn: word.startColumn,
        resolvePrefix,
      }
      const enrichedCandidates = candidates.map(
        (candidate) => resolvedCandidates.get(resolvePrefix + candidate.name) ?? candidate,
      )
      const suggestions = enrichedCandidates.map((c) => toMonacoCompletion(c, range))
      const batch: ResolutionBatch = { model, version: request.version }
      enrichedCandidates.forEach((c, index) => {
        if (
          !resolvedCandidates.has(resolvePrefix + c.name)
          && !c.signature
          && (c.kind == null || c.kind === 'function')
        ) {
          resolutions.set(suggestions[index], { prefix: resolvePrefix, batch })
        }
      })
      return {
        suggestions,
      }
    },
    async resolveCompletionItem(item, token) {
      if (token.isCancellationRequested) throw cancellationError()
      await waitForStableCompletion(token)
      if (token.isCancellationRequested) throw cancellationError()
      await hydrate(item)
      return item
    },
  })

  const suggestController = editor.getContribution<SuggestController>('editor.contrib.suggestController')
  const suggestWidget = suggestController?.widget.value
  const suggestList = suggestWidget?._list
  const hydrateVisible = () => {
    if (!suggestWidget || !suggestList || suggestList.length === 0 || suggestList.renderHeight <= 0) return
    const start = suggestList.firstVisibleIndex
    const end = Math.min(
      suggestList.length,
      suggestList.indexAt(suggestList.scrollTop + suggestList.renderHeight - 1) + 1,
    )
    const visibleItems: SuggestItem[] = []
    const prefetched: Promise<void>[] = []
    let batch: ResolutionBatch | undefined

    for (let index = start; index < end; index += 1) {
      const suggestItem = suggestList.element(index)
      visibleItems.push(suggestItem)
      const completion = suggestItem.completion
      const state = resolutions.get(completion)
      if (state) {
        batch ??= state.batch
        prefetched.push(hydrate(completion))
      }
    }

    if (!batch) return
    void Promise.all(prefetched).then(() => {
      if (
        editor.getModel() !== batch.model
        || batch.model.getVersionId() !== batch.version
      ) return
      if (start + visibleItems.length > suggestList.length) return
      for (let offset = 0; offset < visibleItems.length; offset += 1) {
        if (suggestList.element(start + offset) !== visibleItems[offset]) return
      }
      // Repaint only the hydrated viewport without rebuilding the model.
      const focused = suggestList.getFocus()
      suggestWidget._ignoreFocusEvents = true
      try {
        suggestList.splice(start, visibleItems.length, visibleItems)
        suggestList.setFocus(focused)
        for (const item of visibleItems) resolutions.delete(item.completion)
      } finally {
        suggestWidget._ignoreFocusEvents = false
      }
    })
  }
  const scheduleVisibleHydration = () => {
    if (visibleHydrationTimer !== null) clearTimeout(visibleHydrationTimer)
    visibleHydrationTimer = setTimeout(() => {
      visibleHydrationTimer = null
      hydrateVisible()
    }, VISIBLE_HYDRATION_DELAY)
  }
  const focus = suggestWidget?.onDidFocus(scheduleVisibleHydration)
  const scroll = suggestList?.onDidScroll(scheduleVisibleHydration)

  const content = editor.onDidChangeModelContent((event) => {
    refreshGeneration += 1
    if (refreshTimer !== null) clearTimeout(refreshTimer)
    refreshTimer = null
    const session = active
    if (!session || event.isFlush || event.changes.some((change) => change.text.length > 1)) return

    const generation = refreshGeneration
    refreshTimer = setTimeout(() => {
      refreshTimer = null
      const model = editor.getModel()
      const position = editor.getPosition()
      if (!model || !position || model !== session.model || position.lineNumber !== session.line) return
      const request = completionRequest(model, position)
      const resolvePrefix = request.prefixLine.slice(
        0,
        request.prefixLine.length - request.word.word.length,
      )
      if (request.word.startColumn !== session.wordStartColumn || resolvePrefix !== session.resolvePrefix) return

      void gather(request.prefixLine, request.fullCode, request.line, request.column).then((candidates) => {
        const currentModel = editor.getModel()
        const currentPosition = editor.getPosition()
        if (
          generation !== refreshGeneration
          || currentModel !== model
          || model.getVersionId() !== request.version
          || currentPosition?.lineNumber !== request.line
          || currentPosition.column - 1 !== request.column
        ) return

        snapshot = { ...request, model, candidates }
        // Monaco has no public "replace this open completion list" API. A
        // retrigger keeps the widget visible; the provider immediately serves
        // the prepared snapshot from memory.
        if (suggestController?.model.state) {
          suggestController.model.trigger({ auto: true, retrigger: true })
        }
      })
    }, COMPLETION_REFRESH_DELAY)
  })

  return {
    dispose() {
      refreshGeneration += 1
      if (refreshTimer !== null) clearTimeout(refreshTimer)
      if (visibleHydrationTimer !== null) clearTimeout(visibleHydrationTimer)
      focus?.dispose()
      scroll?.dispose()
      content.dispose()
      provider.dispose()
    },
  }
}
