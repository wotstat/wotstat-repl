import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { Panel, HeaderButton } from '@/shared/ui'
import { paintLine } from '@/shared/lib'
import { consoleBus } from '@/entities/console'
import { SEVERITIES, type Severity, matchesFilter, matchesSearch } from '../lib/severity'

function readBuffer(term: Terminal): string {
  const buf = term.buffer.active
  const out: string[] = []
  for (let i = 0; i < buf.length; i++) {
    out.push(buf.getLine(i)?.translateToString(true) ?? '')
  }
  return out.join('\n').replace(/\s+$/, '') + '\n'
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

interface LogConsoleProps {
  verticalLayout: boolean
  onToggleLayout: () => void
}

export function LogConsole({ verticalLayout, onToggleLayout }: LogConsoleProps) {
  const host = useRef<HTMLDivElement | null>(null)
  const termRef = useRef<Terminal | null>(null)
  const filterMenu = useRef<HTMLDetailsElement | null>(null)

  const [hidden, setHidden] = useState<ReadonlySet<Severity>>(new Set())
  const [search, setSearch] = useState('')
  const [appliedSearch, setAppliedSearch] = useState('')
  const [atBottom, setAtBottom] = useState(true)

  // The xterm subscribe callback is installed once; refs keep it reading the live
  // filter/search/scroll state without re-creating the terminal on every change.
  const hiddenRef = useRef(hidden)
  const searchRef = useRef(appliedSearch)
  const atBottomRef = useRef(atBottom)
  hiddenRef.current = hidden
  searchRef.current = appliedSearch
  atBottomRef.current = atBottom

  // Debounce the search box so each keystroke doesn't replay the whole scrollback.
  useEffect(() => {
    const t = setTimeout(() => setAppliedSearch(search), 150)
    return () => clearTimeout(t)
  }, [search])

  useEffect(() => {
    const closeFilterMenu = (event: PointerEvent) => {
      if (!filterMenu.current?.contains(event.target as Node)) filterMenu.current?.removeAttribute('open')
    }
    document.addEventListener('pointerdown', closeFilterMenu)
    return () => document.removeEventListener('pointerdown', closeFilterMenu)
  }, [])

  useEffect(() => {
    const node = host.current
    if (!node) return

    const term = new Terminal({
      fontFamily: 'JetBrains Mono, ui-monospace, monospace',
      fontSize: 13,
      convertEol: false,
      cursorBlink: false,
      scrollback: 20000,
      theme: { background: '#0E1116', foreground: '#C9D3DF', cursor: '#0E1116' },
    })
    termRef.current = term
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(node)
    const doFit = () => {
      try {
        fit.fit()
      } catch {
        // container not laid out yet; the observer will retry
      }
    }
    requestAnimationFrame(doFit)

    const observer = new ResizeObserver(doFit)
    observer.observe(node)

    const isAtBottom = () => {
      const buf = term.buffer.active
      return buf.viewportY >= buf.baseY
    }

    const unsubScroll = term.onScroll(() => setAtBottom(isAtBottom()))

    const unsub = consoleBus.subscribe((lines) => {
      const stick = isAtBottom()
      let wrote = false
      for (const line of lines) {
        if (!matchesFilter(line, hiddenRef.current) || !matchesSearch(line, searchRef.current)) continue
        term.write(paintLine(line))
        wrote = true
      }
      if (wrote && stick) term.scrollToBottom()
    })
    const unsubClear = consoleBus.subscribeClear(() => term.reset())

    return () => {
      unsubScroll.dispose()
      unsub()
      unsubClear()
      observer.disconnect()
      term.dispose()
      termRef.current = null
    }
  }, [])

  // Re-render retained scrollback whenever the filter or search changes.
  useEffect(() => {
    const term = termRef.current
    if (!term) return
    term.clear()
    for (const line of consoleBus.history()) {
      if (!matchesFilter(line, hidden) || !matchesSearch(line, appliedSearch)) continue
      term.write(paintLine(line))
    }
    if (atBottomRef.current) term.scrollToBottom()
  }, [hidden, appliedSearch])

  const onCopy = () => {
    const term = termRef.current
    if (term) void copyText(readBuffer(term))
  }

  const toggle = (sev: Severity) => {
    setHidden((prev) => {
      const next = new Set(prev)
      if (next.has(sev)) next.delete(sev)
      else next.add(sev)
      return next
    })
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
              {SEVERITIES.map((sev) => (
                <label
                  key={sev}
                  className="flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1.5 text-[11px] text-fg hover:bg-panel"
                >
                  <input
                    type="checkbox"
                    checked={!hidden.has(sev)}
                    onChange={() => toggle(sev)}
                    className="accent-live"
                  />
                  {sev}
                </label>
              ))}
            </div>
          </details>
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="search"
            className="h-6 w-28 rounded border border-edge bg-transparent px-2 text-[11px] text-fg placeholder:text-muted focus:border-live focus:outline-none"
          />
          <HeaderButton onClick={onCopy} title="Copy console to clipboard">
            Copy
          </HeaderButton>
          <HeaderButton onClick={() => consoleBus.clear()} title="Clear console">
            Clear
          </HeaderButton>
        </div>
      }
    >
      <div className="relative h-full w-full">
        <div ref={host} className="h-full w-full px-2 py-1" />
        {!atBottom && (
          <button
            type="button"
            onClick={() => termRef.current?.scrollToBottom()}
            title="Jump to bottom"
            className="absolute bottom-3 right-3 h-7 rounded border border-edge bg-[#0E1116] px-2 text-[11px] text-muted transition-colors hover:border-live hover:text-fg"
          >
            Jump to bottom
          </button>
        )}
      </div>
    </Panel>
  )
}
