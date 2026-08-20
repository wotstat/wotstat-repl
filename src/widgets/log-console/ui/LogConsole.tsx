import { useEffect, useRef, useState } from 'react'
import { Panel, HeaderButton } from '@/shared/ui'
import { monaco } from '@/shared/lib'
import { loadState, saveState } from '@/shared/lib/storage'
import type { LogLine } from '@/shared/api'
import { consoleBus } from '@/entities/console'
import {
  projectLogLines,
  type LogDecorationSpan,
  type LogDisplayOptions,
} from '../lib/logDocument'
import { visibleLogLines } from '../lib/logDisplay'
import { LOG_LANGUAGE_ID, registerLogLanguage } from '../lib/logLanguage'
import { SEVERITIES, type Severity } from '../lib/severity'

const HISTORY_REPLAY_INTERVAL = 1000
const DISPLAY_SETTINGS_KEY = 'console.display'

interface ConsoleDisplaySettings extends LogDisplayOptions {
  showInput: boolean
}

const DEFAULT_CONSOLE_DISPLAY_SETTINGS: ConsoleDisplaySettings = {
  showTimestamp: false,
  showLevel: true,
  showSource: true,
  showInput: true,
}

function hasCustomDisplaySettings(display: ConsoleDisplaySettings): boolean {
  return (Object.keys(DEFAULT_CONSOLE_DISPLAY_SETTINGS) as Array<keyof ConsoleDisplaySettings>)
    .some((setting) => display[setting] !== DEFAULT_CONSOLE_DISPLAY_SETTINGS[setting])
}

interface ConsoleView {
  editor: monaco.editor.IStandaloneCodeEditor
  model: monaco.editor.ITextModel
  decorations: monaco.editor.IEditorDecorationsCollection
  hidden: ReadonlySet<Severity>
  filter: string
  display: ConsoleDisplaySettings
  appendedSinceReplay: number
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    try {
      document.execCommand('copy')
    } catch {
      // no clipboard access available; nothing more we can do
    }
    document.body.removeChild(ta)
  }
}

function toModelDecorations(
  model: monaco.editor.ITextModel,
  spans: readonly LogDecorationSpan[],
  offset = 0,
): monaco.editor.IModelDeltaDecoration[] {
  return spans.map((span) => {
    const start = model.getPositionAt(offset + span.start)
    const end = model.getPositionAt(offset + span.end)
    return {
      range: new monaco.Range(start.lineNumber, start.column, end.lineNumber, end.column),
      options: {
        inlineClassName: span.className,
        stickiness: monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges,
      },
    }
  })
}

function isAtBottom(editor: monaco.editor.IStandaloneCodeEditor): boolean {
  return editor.getScrollTop() + editor.getLayoutInfo().height >= editor.getScrollHeight() - 2
}

function scrollToBottom(editor: monaco.editor.IStandaloneCodeEditor): void {
  editor.setScrollTop(editor.getScrollHeight(), monaco.editor.ScrollType.Immediate)
}

function rebuildView(view: ConsoleView, lines: readonly LogLine[]): void {
  const stick = isAtBottom(view.editor)
  const scrollTop = view.editor.getScrollTop()
  const document = projectLogLines(lines, view.display)
  view.model.setValue(document.text)
  view.decorations.set(toModelDecorations(view.model, document.decorations))
  view.appendedSinceReplay = 0
  if (stick) scrollToBottom(view.editor)
  else view.editor.setScrollTop(scrollTop, monaco.editor.ScrollType.Immediate)
}

function appendToView(view: ConsoleView, lines: readonly LogLine[]): void {
  if (lines.length === 0) return

  view.appendedSinceReplay += lines.length
  if (view.appendedSinceReplay >= HISTORY_REPLAY_INTERVAL) {
    rebuildView(
      view,
      visibleLogLines(consoleBus.history(), view.hidden, view.filter, view.display.showInput),
    )
    return
  }

  const document = projectLogLines(lines, view.display)
  if (!document.text) return

  const stick = isAtBottom(view.editor)
  const offset = view.model.getValueLength()
  const end = view.model.getPositionAt(offset)
  view.model.applyEdits([
    {
      range: new monaco.Range(end.lineNumber, end.column, end.lineNumber, end.column),
      text: document.text,
    },
  ])
  view.decorations.append(toModelDecorations(view.model, document.decorations, offset))
  if (stick) scrollToBottom(view.editor)
}

interface LogConsoleProps {
  verticalLayout: boolean
  onToggleLayout: () => void
}

export function LogConsole({ verticalLayout, onToggleLayout }: LogConsoleProps) {
  const host = useRef<HTMLDivElement | null>(null)
  const viewRef = useRef<ConsoleView | null>(null)
  const filterMenu = useRef<HTMLDetailsElement | null>(null)
  const displayMenu = useRef<HTMLDetailsElement | null>(null)

  const [hidden, setHidden] = useState<ReadonlySet<Severity>>(new Set())
  const [filter, setFilter] = useState('')
  const [appliedFilter, setAppliedFilter] = useState('')
  const [atBottom, setAtBottom] = useState(true)
  const [display, setDisplay] = useState<ConsoleDisplaySettings>(() => {
    const saved = loadState<Partial<ConsoleDisplaySettings>>(DISPLAY_SETTINGS_KEY, {})
    return {
      showTimestamp: saved.showTimestamp ?? DEFAULT_CONSOLE_DISPLAY_SETTINGS.showTimestamp,
      showLevel: saved.showLevel ?? DEFAULT_CONSOLE_DISPLAY_SETTINGS.showLevel,
      showSource: saved.showSource ?? DEFAULT_CONSOLE_DISPLAY_SETTINGS.showSource,
      showInput: saved.showInput ?? DEFAULT_CONSOLE_DISPLAY_SETTINGS.showInput,
    }
  })

  const hiddenRef = useRef(hidden)
  const filterRef = useRef(appliedFilter)
  const displayRef = useRef(display)
  hiddenRef.current = hidden
  filterRef.current = appliedFilter
  displayRef.current = display

  useEffect(() => {
    const timer = setTimeout(() => setAppliedFilter(filter), 150)
    return () => clearTimeout(timer)
  }, [filter])

  useEffect(() => {
    const closeMenus = (event: PointerEvent) => {
      const target = event.target as Node
      if (!filterMenu.current?.contains(target)) filterMenu.current?.removeAttribute('open')
      if (!displayMenu.current?.contains(target)) displayMenu.current?.removeAttribute('open')
    }
    document.addEventListener('pointerdown', closeMenus)
    return () => document.removeEventListener('pointerdown', closeMenus)
  }, [])

  useEffect(() => saveState(DISPLAY_SETTINGS_KEY, display), [display])

  useEffect(() => {
    const node = host.current
    if (!node) return

    registerLogLanguage(monaco)
    const editor = monaco.editor.create(node, {
      value: '',
      language: LOG_LANGUAGE_ID,
      theme: 'wms-dark',
      readOnly: true,
      domReadOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      lineNumbers: 'off',
      lineDecorationsWidth: 8,
      glyphMargin: false,
      folding: false,
      fontFamily: 'JetBrains Mono, ui-monospace, monospace',
      fontSize: 13,
      wordWrap: 'on',
      wrappingIndent: 'none',
      scrollBeyondLastLine: false,
      renderLineHighlight: 'line',
      renderWhitespace: 'none',
      stickyScroll: { enabled: false },
      padding: { top: 4, bottom: 4 },
    })
    const model = editor.getModel()
    if (!model) {
      editor.dispose()
      return
    }

    const view: ConsoleView = {
      editor,
      model,
      decorations: editor.createDecorationsCollection(),
      hidden: hiddenRef.current,
      filter: filterRef.current,
      display: displayRef.current,
      appendedSinceReplay: 0,
    }
    viewRef.current = view
    rebuildView(
      view,
      visibleLogLines(consoleBus.history(), view.hidden, view.filter, view.display.showInput),
    )

    const scrollSubscription = editor.onDidScrollChange(() => setAtBottom(isAtBottom(editor)))
    const unsubscribe = consoleBus.subscribe((lines) => {
      appendToView(
        view,
        visibleLogLines(
          lines,
          hiddenRef.current,
          filterRef.current,
          displayRef.current.showInput,
        ),
      )
    })
    const unsubscribeClear = consoleBus.subscribeClear(() => {
      model.setValue('')
      view.decorations.clear()
      view.appendedSinceReplay = 0
      setAtBottom(true)
    })

    return () => {
      scrollSubscription.dispose()
      unsubscribe()
      unsubscribeClear()
      editor.dispose()
      model.dispose()
      viewRef.current = null
    }
  }, [])

  useEffect(() => {
    const view = viewRef.current
    if (
      !view
      || (view.hidden === hidden && view.filter === appliedFilter && view.display === display)
    ) return
    view.hidden = hidden
    view.filter = appliedFilter
    view.display = display
    rebuildView(
      view,
      visibleLogLines(consoleBus.history(), hidden, appliedFilter, display.showInput),
    )
  }, [hidden, appliedFilter, display])

  const onCopy = () => {
    const text = viewRef.current?.model.getValue()
    if (text !== undefined) void copyText(text)
  }

  const toggle = (severity: Severity) => {
    setHidden((previous) => {
      const next = new Set(previous)
      if (next.has(severity)) next.delete(severity)
      else next.add(severity)
      return next
    })
  }

  const toggleDisplay = (setting: keyof ConsoleDisplaySettings) => {
    setDisplay((previous) => ({ ...previous, [setting]: !previous[setting] }))
  }

  return (
    <Panel
      title="Console"
      className="w-full"
      actions={
        <div className="flex items-center gap-1.5">
          <HeaderButton
            onClick={onToggleLayout}
            title={verticalLayout ? 'Switch to horizontal layout' : 'Switch to vertical layout'}
            aria-label={verticalLayout ? 'Switch to horizontal layout' : 'Switch to vertical layout'}
            className="inline-flex w-7 items-center justify-center p-0"
            style={{ padding: 0 }}
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="h-3.5 w-3.5">
              <rect x="2" y="2" width="12" height="12" rx="0.5" className="stroke-current" strokeWidth="1.2" />
              {verticalLayout ? (
                <path d="M3 8h10v5H3z" className="fill-current" />
              ) : (
                <path d="M8 3h5v10H8z" className="fill-current" />
              )}
            </svg>
          </HeaderButton>
          <details ref={displayMenu} className="relative z-20">
            <summary
              title="Console display settings"
              aria-label="Console display settings"
              className={`flex h-6 w-7 cursor-pointer list-none items-center justify-center rounded border text-muted transition-colors hover:border-live hover:text-fg [&::-webkit-details-marker]:hidden ${hasCustomDisplaySettings(display) ? 'border-live text-fg' : 'border-edge'}`}
            >
              <svg viewBox="0 0 16 16" aria-hidden="true" className="h-3.5 w-3.5 fill-current">
                <path d="M9.405 1.05c-.413-1.4-2.397-1.4-2.81 0l-.1.34a1.464 1.464 0 0 1-2.105.872l-.31-.17c-1.283-.698-2.686.705-1.987 1.987l.169.311c.446.82.023 1.841-.872 2.105l-.34.1c-1.4.413-1.4 2.397 0 2.81l.34.1c.895.264 1.318 1.285.872 2.105l-.17.31c-.698 1.283.705 2.686 1.987 1.987l.311-.169a1.464 1.464 0 0 1 2.105.872l.1.34c.413 1.4 2.397 1.4 2.81 0l.1-.34a1.464 1.464 0 0 1 2.105-.872l.31.17c1.283.698 2.686-.705 1.987-1.987l-.169-.311a1.464 1.464 0 0 1 .872-2.105l.34-.1c1.4-.413 1.4-2.397 0-2.81l-.34-.1a1.464 1.464 0 0 1-.872-2.105l.17-.31c.698-1.283-.705-2.686-1.987-1.987l-.311.169a1.464 1.464 0 0 1-2.105-.872zM8 10.93a2.929 2.929 0 1 1 0-5.858 2.929 2.929 0 0 1 0 5.858" />
              </svg>
            </summary>
            <div className="absolute top-7 left-0 w-44 rounded border border-edge bg-elevated p-1 shadow-lg">
              <label className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel">
                <input
                  type="checkbox"
                  checked={display.showTimestamp}
                  onChange={() => toggleDisplay('showTimestamp')}
                  className="accent-live"
                />
                Show log time
              </label>
              <label className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel">
                <input
                  type="checkbox"
                  checked={display.showLevel}
                  onChange={() => toggleDisplay('showLevel')}
                  className="accent-live"
                />
                Show log level
              </label>
              <label className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel">
                <input
                  type="checkbox"
                  checked={display.showSource}
                  onChange={() => toggleDisplay('showSource')}
                  className="accent-live"
                />
                Show log source
              </label>
              <div className="my-1 border-t border-edge" />
              <label className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel">
                <input
                  type="checkbox"
                  checked={display.showInput}
                  onChange={() => toggleDisplay('showInput')}
                  className="accent-live"
                />
                Show executed code
              </label>
            </div>
          </details>
          <details ref={filterMenu} className="relative z-20">
            <summary
              title="Filter log levels"
              aria-label="Filter log levels"
              className={`flex h-6 w-7 cursor-pointer list-none items-center justify-center rounded border text-muted transition-colors hover:border-live hover:text-fg [&::-webkit-details-marker]:hidden ${hidden.size ? 'border-live text-fg' : 'border-edge'}`}
            >
              <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="h-3.5 w-3.5 stroke-current">
                <path d="M2 3h12L9.5 8v4L6.5 14V8L2 3Z" strokeWidth="1.4" strokeLinejoin="round" />
              </svg>
            </summary>
            <div className="absolute top-7 left-0 w-36 rounded border border-edge bg-elevated p-1 shadow-lg">
              {SEVERITIES.map((severity) => (
                <label
                  key={severity}
                  className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel"
                >
                  <input
                    type="checkbox"
                    checked={!hidden.has(severity)}
                    onChange={() => toggle(severity)}
                    className="accent-live"
                  />
                  {severity}
                </label>
              ))}
            </div>
          </details>
          <input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="filter"
            title="Filter console output"
            aria-label="Filter console output"
            className="h-6 w-28 rounded border border-edge bg-transparent px-2 text-[11px] text-fg placeholder:text-muted focus:border-live focus:outline-none"
          />
          <HeaderButton onClick={onCopy} title="Copy filtered console to clipboard">
            Copy
          </HeaderButton>
          <HeaderButton onClick={() => consoleBus.clear()} title="Clear console">
            Clear
          </HeaderButton>
        </div>
      }
    >
      <div className="relative h-full w-full">
        <div ref={host} className="h-full w-full" />
        {!atBottom && (
          <button
            type="button"
            onClick={() => {
              const editor = viewRef.current?.editor
              if (editor) scrollToBottom(editor)
            }}
            title="Jump to bottom"
            className="absolute bottom-3 right-3 z-10 h-7 rounded border border-edge bg-[#0E1116] px-2 text-[11px] text-muted transition-colors hover:border-live hover:text-fg"
          >
            Jump to bottom
          </button>
        )}
      </div>
    </Panel>
  )
}
