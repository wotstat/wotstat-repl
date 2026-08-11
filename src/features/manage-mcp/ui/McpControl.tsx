import { useEffect, useRef, useState } from 'react'
import { api, type McpCliStatus, type McpConnectionInfo, type McpStatus } from '@/shared/api'

const DOT: Record<McpStatus, string> = {
  disabled: 'bg-faint',
  starting: 'bg-warn animate-pulse',
  listening: 'bg-ok',
  error: 'bg-error',
}

const LABEL: Record<McpStatus, string> = {
  disabled: 'Disabled',
  starting: 'Starting',
  listening: 'Listening',
  error: 'Error',
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function CopyButton({
  label,
  disabled,
  copied,
  onClick,
}: {
  label: string
  disabled: boolean
  copied: boolean
  onClick: () => void
}) {
  const accessibleLabel = copied ? `Copied ${label}` : `Copy ${label}`
  return (
    <button
      type="button"
      title={accessibleLabel}
      aria-label={accessibleLabel}
      disabled={disabled}
      onClick={onClick}
      className={`absolute right-1 top-1 flex size-6 items-center justify-center rounded transition-all duration-200 disabled:opacity-40 ${copied ? 'bg-ok/10 text-ok' : 'text-muted hover:bg-elevated hover:text-fg'}`}
    >
      <span className="relative size-3.5">
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className={`absolute inset-0 size-3.5 stroke-current transition-all duration-200 ${copied ? 'scale-75 opacity-0 blur-[2px]' : 'scale-100 opacity-100 blur-none'}`}
        >
          <rect x="5" y="3" width="8" height="10" rx="1" strokeWidth="1.4" />
          <path d="M3 11V4a1 1 0 0 1 1-1" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className={`absolute inset-0 size-3.5 stroke-current transition-all duration-200 ${copied ? 'scale-100 opacity-100 blur-none' : 'scale-75 opacity-0 blur-[2px]'}`}
        >
          <path d="m3 8.5 3 3L13 4.5" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
    </button>
  )
}

export function McpControl() {
  const root = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [info, setInfo] = useState<McpConnectionInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [requestError, setRequestError] = useState<string | null>(null)
  const [copied, setCopied] = useState<'local' | 'network' | null>(null)
  const [copyError, setCopyError] = useState<string | null>(null)
  const copyTimer = useRef<number | null>(null)
  const copyGeneration = useRef(0)
  const [cliStatus, setCliStatus] = useState<McpCliStatus | null>(null)
  const [cliBusy, setCliBusy] = useState<{
    cli: 'codex' | 'claude'
    action: 'add' | 'remove'
  } | null>(null)
  const [cliFeedback, setCliFeedback] = useState<{ ok: boolean; text: string } | null>(null)

  const refresh = async () => {
    try {
      setInfo(await api.mcpConnectionInfo())
      setRequestError(null)
    } catch (error) {
      setRequestError(errorMessage(error))
    }
  }

  const refreshCliStatus = async () => {
    try {
      setCliStatus(await api.mcpCliStatus())
      return true
    } catch (error) {
      setCliFeedback({ ok: false, text: `Could not refresh CLI status: ${errorMessage(error)}` })
      return false
    }
  }

  useEffect(() => {
    void refresh()
    void refreshCliStatus()
  }, [])

  useEffect(() => {
    if (!open) return
    void refresh()
    const interval = window.setInterval(() => void refresh(), 2000)
    return () => window.clearInterval(interval)
  }, [open])

  useEffect(() => {
    if (open) return
    copyGeneration.current += 1
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    copyTimer.current = null
    setCopied(null)
    setCopyError(null)
  }, [open])

  useEffect(
    () => () => {
      copyGeneration.current += 1
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    },
    [],
  )

  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', close)
    document.addEventListener('keydown', escape)
    return () => {
      document.removeEventListener('pointerdown', close)
      document.removeEventListener('keydown', escape)
    }
  }, [open])

  const toggle = async (enabled: boolean) => {
    if (busy) return
    setBusy(true)
    setRequestError(null)
    try {
      setInfo(await api.mcpSetEnabled(enabled))
    } catch (error) {
      setRequestError(errorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const copy = async (target: 'local' | 'network', label: string, text: string) => {
    const generation = ++copyGeneration.current
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    copyTimer.current = null
    setCopied(null)
    setCopyError(null)
    try {
      await navigator.clipboard.writeText(text)
      if (generation !== copyGeneration.current) return
      setCopied(target)
      copyTimer.current = window.setTimeout(() => {
        if (generation === copyGeneration.current) {
          setCopied(null)
          copyTimer.current = null
        }
      }, 2000)
    } catch {
      if (generation === copyGeneration.current) {
        setCopyError(`Could not copy ${label.toLowerCase()}`)
      }
    }
  }

  const updateCli = async (cli: 'codex' | 'claude') => {
    const state = cliStatus?.[cli]
    if (cliBusy || !state?.installed) return
    const action = state.configured ? 'remove' : 'add'
    setCliBusy({ cli, action })
    setCliFeedback(null)
    try {
      const text = await (cli === 'codex'
        ? action === 'add'
          ? api.mcpAddToCodex()
          : api.mcpRemoveFromCodex()
        : action === 'add'
          ? api.mcpAddToClaude()
          : api.mcpRemoveFromClaude())
      if (await refreshCliStatus()) setCliFeedback({ ok: true, text })
    } catch (error) {
      setCliFeedback({ ok: false, text: errorMessage(error) })
    } finally {
      setCliBusy(null)
    }
  }

  const status: McpStatus = requestError ? 'error' : (info?.status ?? 'starting')
  const shownError = requestError ?? info?.error

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        title={`MCP: ${LABEL[status]}`}
        aria-label={`MCP server: ${LABEL[status]}`}
        aria-expanded={open}
        aria-controls="mcp-status-popover"
        onClick={() => {
          setOpen((value) => !value)
        }}
        className="inline-flex h-5 items-center gap-1.5 rounded px-1.5 text-[11px] text-muted transition-colors hover:bg-elevated hover:text-fg focus-visible:outline focus-visible:outline-live"
      >
        <span aria-hidden="true" className={`size-1.5 rounded-full ${DOT[status]}`} />
        MCP
      </button>

      {open && (
        <section
          id="mcp-status-popover"
          role="dialog"
          aria-label="MCP server settings"
          className="absolute bottom-7 right-0 z-40 w-96 select-text rounded border border-edge bg-elevated p-3 text-left shadow-2xl"
        >
          <div className="mb-3 flex items-center justify-between">
            <span className="text-[12px] font-medium text-fg">MCP server</span>
            <span className="inline-flex items-center gap-1.5 text-[11px] text-muted">
              <span aria-hidden="true" className={`size-1.5 rounded-full ${DOT[status]}`} />
              {LABEL[status]}
            </span>
          </div>

          <label className="mb-3 flex items-center justify-between text-[11px] text-fg">
            Enabled
            <input
              type="checkbox"
              checked={info?.enabled ?? false}
              disabled={busy || !info}
              onChange={(event) => void toggle(event.target.checked)}
              className="accent-live"
            />
          </label>

          <div className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Local URL
            </span>
            <div className="relative">
              <code className="block overflow-x-auto rounded bg-panel p-2 pr-9 font-mono text-[10px] text-fg">
                {info?.url ?? 'Loading…'}
              </code>
              <CopyButton
                label="Local URL"
                disabled={!info}
                copied={copied === 'local'}
                onClick={() => info && void copy('local', 'Local URL', info.url)}
              />
            </div>
          </div>

          <div className="mt-3 space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Network URL
            </span>
            <div className="relative">
              <code className="block overflow-x-auto rounded bg-panel p-2 pr-9 font-mono text-[10px] text-fg">
                {info?.networkUrl ?? 'Loading…'}
              </code>
              <CopyButton
                label="Network URL"
                disabled={!info}
                copied={copied === 'network'}
                onClick={() => info && void copy('network', 'Network URL', info.networkUrl)}
              />
            </div>
          </div>

          {copyError && (
            <span role="alert" className="mt-2 block text-[10px] text-error">
              {copyError}
            </span>
          )}

          {shownError && (
            <p role="alert" className="mt-2 break-words text-[10px] text-error">
              {shownError}
            </p>
          )}

          <div className="mt-3 border-t border-edge pt-3">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Register MCP server
            </span>
            <div className="mt-2 flex gap-3">
              <div>
                <button
                  type="button"
                  title={
                    cliStatus?.codex.installed === false
                      ? 'Codex CLI not found on PATH'
                      : cliStatus?.codex.configured
                        ? 'Remove wot_repl from Codex'
                        : 'Add wot_repl to Codex'
                  }
                  disabled={cliBusy !== null || cliStatus?.codex.installed !== true}
                  onClick={() => void updateCli('codex')}
                  className={`h-6 rounded border px-2 text-[10px] disabled:opacity-40 ${cliStatus?.codex.configured ? 'border-error/40 text-error hover:border-error hover:bg-error/10' : 'border-edge text-muted hover:border-live hover:text-fg'}`}
                >
                  {cliBusy?.cli === 'codex'
                    ? cliBusy.action === 'add'
                      ? 'Adding…'
                      : 'Removing…'
                    : cliStatus?.codex.configured
                      ? 'Remove from Codex'
                      : 'Add to Codex'}
                </button>
              </div>
              <div>
                <button
                  type="button"
                  title={
                    cliStatus?.claude.installed === false
                      ? 'Claude CLI not found on PATH'
                      : cliStatus?.claude.configured
                        ? 'Remove wot_repl from Claude Code'
                        : 'Add wot_repl to Claude Code'
                  }
                  disabled={cliBusy !== null || cliStatus?.claude.installed !== true}
                  onClick={() => void updateCli('claude')}
                  className={`h-6 rounded border px-2 text-[10px] disabled:opacity-40 ${cliStatus?.claude.configured ? 'border-error/40 text-error hover:border-error hover:bg-error/10' : 'border-edge text-muted hover:border-live hover:text-fg'}`}
                >
                  {cliBusy?.cli === 'claude'
                    ? cliBusy.action === 'add'
                      ? 'Adding…'
                      : 'Removing…'
                    : cliStatus?.claude.configured
                      ? 'Remove from Claude'
                      : 'Add to Claude'}
                </button>
              </div>
            </div>
            {cliFeedback && (
              <p
                role={cliFeedback.ok ? 'status' : 'alert'}
                className={`mt-2 break-words text-[10px] ${cliFeedback.ok ? 'text-ok' : 'text-error'}`}
              >
                {cliFeedback.text}
              </p>
            )}
          </div>
        </section>
      )}
    </div>
  )
}
