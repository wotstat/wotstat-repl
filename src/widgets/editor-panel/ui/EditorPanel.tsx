import { useEffect, useRef } from 'react'
import { monaco, loadState, saveState } from '@/shared/lib'
import { Panel, HeaderButton } from '@/shared/ui'
import { registerPythonCompletion } from '@/features/complete-code'
import { attachLinter } from '@/features/lint-code'
import { runCode } from '@/features/run-code'
import { useEditorCursor, getHistory } from '@/entities/editor'
import { useSession } from '@/entities/session'

const SAMPLE = [
  '# Ctrl/Cmd+Enter runs the selection (or the whole buffer) in the live game.',
  'import BigWorld',
  'print BigWorld.player()',
  '',
].join('\n')

function runEditor(editor: monaco.editor.IStandaloneCodeEditor | null) {
  if (!editor) return
  const selection = editor.getSelection()
  const code =
    selection && !selection.isEmpty()
      ? (editor.getModel()?.getValueInRange(selection) ?? '')
      : editor.getValue()
  void runCode(code)
}

export function EditorPanel() {
  const container = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const setCursor = useEditorCursor((s) => s.setCursor)
  const connected = useSession((s) => s.status === 'connected')
  const historyIndexRef = useRef<number>(-1)

  useEffect(() => {
    const host = container.current
    if (!host) return

    const editor = monaco.editor.create(host, {
      value: loadState<string>('editor.buffer', SAMPLE),
      language: 'python',
      theme: 'wms-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontFamily: 'JetBrains Mono, ui-monospace, monospace',
      fontSize: 13,
      scrollBeyondLastLine: false,
      renderLineHighlight: 'line',
      padding: { top: 8 },
      // Only show OUR providers (live agent + jedi). Monaco's word-based
      // suggestions otherwise pollute the list with words from the buffer text.
      wordBasedSuggestions: 'off',
      suggest: { showWords: false },
      // Render suggest/hover widgets at document-body level so the editor panel's
      // overflow:auto doesn't clip them (and Monaco flips them to fit the window).
      fixedOverflowWidgets: true,
    })
    editorRef.current = editor
    const completionDisposable = registerPythonCompletion(monaco, editor)

    const model = editor.getModel()
    const detachLint = model ? attachLinter(monaco, model) : () => undefined

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => runEditor(editor))

    // Alt+Arrow for history cycling — using Alt instead of bare arrows so normal
    // cursor movement and the suggest-widget navigation stay unaffected.
    editor.addCommand(monaco.KeyMod.Alt | monaco.KeyCode.UpArrow, () => {
      const history = getHistory()
      if (history.length === 0) return
      const next = historyIndexRef.current === -1 ? history.length - 1 : Math.max(0, historyIndexRef.current - 1)
      historyIndexRef.current = next
      editor.setValue(history[next])
      const lineCount = editor.getModel()?.getLineCount() ?? 1
      editor.setPosition({ lineNumber: lineCount, column: editor.getModel()?.getLineMaxColumn(lineCount) ?? 1 })
    })

    editor.addCommand(monaco.KeyMod.Alt | monaco.KeyCode.DownArrow, () => {
      const history = getHistory()
      if (historyIndexRef.current === -1) return
      if (historyIndexRef.current >= history.length - 1) {
        historyIndexRef.current = -1
        editor.setValue('')
        editor.setPosition({ lineNumber: 1, column: 1 })
        return
      }
      const next = historyIndexRef.current + 1
      historyIndexRef.current = next
      editor.setValue(history[next])
      const lineCount = editor.getModel()?.getLineCount() ?? 1
      editor.setPosition({ lineNumber: lineCount, column: editor.getModel()?.getLineMaxColumn(lineCount) ?? 1 })
    })

    const cursorSub = editor.onDidChangeCursorPosition((e) =>
      setCursor(e.position.lineNumber, e.position.column),
    )

    let saveTimer: ReturnType<typeof setTimeout> | null = null
    const contentSub = editor.onDidChangeModelContent(() => {
      if (saveTimer !== null) clearTimeout(saveTimer)
      saveTimer = setTimeout(() => {
        const value = editor.getValue()
        // Skip persisting unreasonably large buffers (a big paste) to avoid
        // thrashing localStorage quota on every keystroke.
        if (value.length <= 256 * 1024) saveState('editor.buffer', value)
        saveTimer = null
      }, 400)
    })

    return () => {
      if (saveTimer !== null) clearTimeout(saveTimer)
      contentSub.dispose()
      cursorSub.dispose()
      detachLint()
      completionDisposable.dispose()
      editor.dispose()
      editorRef.current = null
    }
  }, [setCursor])

  return (
    <Panel
      title="Editor"
      className="w-full"
      actions={
        <HeaderButton
          onClick={() => runEditor(editorRef.current)}
          disabled={!connected}
          title="Run selection or buffer (Ctrl/Cmd+Enter)"
          className="border-live/40 text-fg"
        >
          ▶ Run
        </HeaderButton>
      }
    >
      <div ref={container} className="h-full w-full" />
    </Panel>
  )
}
