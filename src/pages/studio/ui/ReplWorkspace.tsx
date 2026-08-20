import { useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'
import { EditorPanel } from '@/widgets/editor-panel'
import { LogConsole } from '@/widgets/log-console'
import { beginSeparatorResize } from './resizeSeparator'

export function ReplWorkspace() {
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
    beginSeparatorResize(event)
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
    const updateSize = (size: number) =>
      Math.min(80, Math.max(20, size + (event.key === decrease ? -2 : 2)))
    if (verticalLayout) setEditorHeight(updateSize)
    else setEditorWidth(updateSize)
  }

  return (
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
      <LogConsole
        verticalLayout={verticalLayout}
        onToggleLayout={() => setVerticalLayout((layout) => !layout)}
      />
    </main>
  )
}
