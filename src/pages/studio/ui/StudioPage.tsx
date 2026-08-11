import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'
import { useSession } from '@/entities/session'
import { ConnectControls } from '@/features/connect-session'
import { dumpLive } from '@/features/dump-object'
import { EditorPanel } from '@/widgets/editor-panel'
import { LogConsole } from '@/widgets/log-console'
import { StatusBar } from '@/widgets/status-bar'
import { CommandPalette } from '@/widgets/command-palette'

export function StudioPage() {
  const status = useSession((s) => s.status)
  const autoDumped = useRef(false)
  const workspace = useRef<HTMLElement>(null)
  const [editorWidth, setEditorWidth] = useState(58)
  const [editorHeight, setEditorHeight] = useState(58)
  const [verticalLayout, setVerticalLayout] = useState(false)

  const resize = (clientPosition: number) => {
    const bounds = workspace.current?.getBoundingClientRect()
    if (!bounds) return
    const size = verticalLayout
      ? ((clientPosition - bounds.top) / bounds.height) * 100
      : ((clientPosition - bounds.left) / bounds.width) * 100
    const nextSize = Math.min(80, Math.max(20, size))
    if (verticalLayout) setEditorHeight(nextSize)
    else setEditorWidth(nextSize)
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId)
    resize(verticalLayout ? event.clientY : event.clientX)
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      resize(verticalLayout ? event.clientY : event.clientX)
    }
  }

  const onSeparatorKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const decrease = verticalLayout ? 'ArrowUp' : 'ArrowLeft'
    const increase = verticalLayout ? 'ArrowDown' : 'ArrowRight'
    if (event.key !== decrease && event.key !== increase) return
    event.preventDefault()
    const updateSize = (size: number) => Math.min(80, Math.max(20, size + (event.key === decrease ? -2 : 2)))
    if (verticalLayout) setEditorHeight(updateSize)
    else setEditorWidth(updateSize)
  }

  // Auto-dump all live types once the agent is truly alive (hello -> connected),
  // so jedi stubs are ready without a manual Ctrl+K. Re-arms on disconnect.
  useEffect(() => {
    if (status === 'connected' && !autoDumped.current) {
      autoDumped.current = true
      const t = setTimeout(() => void dumpLive('*', 3), 500)
      return () => clearTimeout(t)
    }
    if (status === 'disconnected') autoDumped.current = false
  }, [status])

  return (
    <>
      <header className="flex h-10 shrink-0 select-none items-center justify-between border-b border-edge bg-panel px-3">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-semibold tracking-tight text-fg">Fuflo WoT REPL</span>
          <span className="text-[11px] text-faint">Ctrl/Cmd+K for commands</span>
        </div>
        <ConnectControls />
      </header>
      <main
        ref={workspace}
        className="grid min-h-0 flex-1"
        style={
          verticalLayout
            ? { gridTemplateRows: `${editorHeight}fr 5px ${100 - editorHeight}fr` }
            : { gridTemplateColumns: `${editorWidth}fr 5px ${100 - editorWidth}fr` }
        }
      >
        <EditorPanel />
        <div
          role="separator"
          aria-label={`Resize editor and console ${verticalLayout ? 'vertically' : 'horizontally'}`}
          aria-orientation={verticalLayout ? 'horizontal' : 'vertical'}
          aria-valuemin={20}
          aria-valuemax={80}
          aria-valuenow={Math.round(verticalLayout ? editorHeight : editorWidth)}
          tabIndex={0}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onKeyDown={onSeparatorKeyDown}
          className={`z-10 touch-none bg-edge transition-colors hover:bg-live focus-visible:bg-live focus-visible:outline-none ${verticalLayout ? 'cursor-row-resize' : 'cursor-col-resize'}`}
        />
        <LogConsole verticalLayout={verticalLayout} onToggleLayout={() => setVerticalLayout((layout) => !layout)} />
      </main>
      <StatusBar />
      <CommandPalette />
    </>
  )
}
